// wifi_zenoh_chatter.rs — WiFi → Zenoh → ROS2 /chatter pub/sub
//
// Baker link.dev (RP2040) → SPI0 → XIAO ESP32-C3 (esp-hosted-mcu)
// → WiFi → Zenoh Router → ROS2 /chatter topic.
//
// Based on: external/zenoh_ros2_nostd/examples/bakerlink_wiz630io/
//
// Build:
//   cp wifi_config.json.example wifi_config.json  # edit credentials
//   cargo build --no-default-features --features embassy,wifi --example wifi_zenoh_chatter
//
// Run (probe-rs):
//   cargo run --no-default-features --features embassy,wifi --example wifi_zenoh_chatter

#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use wifi_config::AppConfig;

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, Stack, StackResources};
use embassy_net_esp_hosted::{self, NetDriver, Runner as EspRunner, SpiInterface, State};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Async, Config as SpiConfig, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Duration, Timer};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use heapless::String;
use panic_probe as _;
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;
use zenoh_ros2_nostd::cdr::cdr_cap_for_string;
use zenoh_ros2_nostd::prelude::*;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

// ── ROS2 topic definition ────────────────────────────────────────────────────

const CHATTER_TOPIC: TopicKeyExpr = msg::std_msgs::String::topic(0, "chatter");
const CDR_BUF_CAP: usize = cdr_cap_for_string(128);

#[derive(Serialize, Deserialize, Debug)]
struct StringMsg {
    data: String<128>,
}

static CHATTER_PUB: Publisher<StringMsg, CDR_BUF_CAP, 4> = Publisher::new(CHATTER_TOPIC);
static CHATTER_SUB: Subscription<StringMsg, CDR_BUF_CAP, 4> = Subscription::new();

// ── Type aliases ─────────────────────────────────────────────────────────────

type MySpi = Spi<'static, SPI0, Async>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, NoDelay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;

// ── Static storage ───────────────────────────────────────────────────────────

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

// ── Entry point ──────────────────────────────────────────────────────────────

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    defmt::trace!("RTT init");
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] start — WiFi Zenoh chatter");

    // ── SPI0 setup (Mode 0 for esp-hosted) ───────────────────────────────────
    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 10_000_000; // 10 MHz

    let spi: MySpi = Spi::new(
        p.SPI0, p.PIN_18, // SCK
        p.PIN_19, // MOSI
        p.PIN_16, // MISO
        p.DMA_CH0, p.DMA_CH1, Irqs, spi_cfg,
    );

    let cs = Output::new(p.PIN_17, Level::High);
    let handshake = Input::new(p.PIN_15, Pull::None);
    let data_ready = Input::new(p.PIN_13, Pull::None);
    let reset = Output::new(p.PIN_14, Level::Low);

    let spi_dev: MySpiDevice = ExclusiveDevice::new_no_delay(spi, cs).unwrap();
    let spi_iface = SpiInterface::new(spi_dev, handshake, data_ready);

    // ── ESP-Hosted driver init ───────────────────────────────────────────────
    let esp_state = ESP_STATE.init(State::new());
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted::new(esp_state, spi_iface, reset).await;

    // ── WiFi connect ─────────────────────────────────────────────────────────
    let cfg = AppConfig::new();
    info!("[wifi] Connecting to \"{}\"...", cfg.wifi_ssid);
    control.init().await.expect("esp init");
    control
        .connect(cfg.wifi_ssid, cfg.wifi_password)
        .await
        .expect("wifi connect");
    info!("[wifi] Connected.");

    // ── Network stack ────────────────────────────────────────────────────────
    let seed = 0x1234_5678_9abc_def0u64; // TODO: use RP2040 ROSC for entropy
    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );

    // ── Spawn tasks ──────────────────────────────────────────────────────────
    spawner.spawn(esp_hosted_task(esp_runner).unwrap());
    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(zenoh_task(stack).unwrap());
    spawner.spawn(app_task().unwrap());
}

// ── Tasks ────────────────────────────────────────────────────────────────────

/// Drives the ESP-Hosted SPI communication loop.
#[embassy_executor::task]
async fn esp_hosted_task(runner: MyEspRunner) {
    runner.run().await
}

/// Drives the embassy-net packet I/O loop.
#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, NetDriver<'static>>) {
    runner.run().await
}

/// Zenoh session lifecycle:
/// 1. Wait for DHCP
/// 2. TCP connect to Zenoh router
/// 3. Build Node → register pub/sub → spin
/// 4. On disconnect: exponential backoff → retry
#[embassy_executor::task]
async fn zenoh_task(stack: Stack<'static>) {
    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();

    let cfg = AppConfig::new();
    let mut reconnect = ReconnectPolicy::default_policy();
    let tcp_rx = TCP_RX.init([0u8; 4096]);
    let tcp_tx = TCP_TX.init([0u8; 4096]);

    let router_ep = cfg.zenoh.router_endpoint();
    info!(
        "[net] router target = {}.{}.{}.{}:{}",
        cfg.zenoh.router_ip[0],
        cfg.zenoh.router_ip[1],
        cfg.zenoh.router_ip[2],
        cfg.zenoh.router_ip[3],
        cfg.zenoh.router_port,
    );

    loop {
        // Wait for DHCP
        while stack.config_v4().is_none() {
            Timer::after(Duration::from_millis(500)).await;
        }
        if let Some(ip_cfg) = stack.config_v4() {
            let addr = ip_cfg.address.address().octets();
            let gw = ip_cfg.gateway.map(|g| g.octets()).unwrap_or([0, 0, 0, 0]);
            info!(
                "[net] DHCP OK — IP {}.{}.{}.{} GW {}.{}.{}.{}",
                addr[0], addr[1], addr[2], addr[3], gw[0], gw[1], gw[2], gw[3],
            );
        }

        // TCP connect
        let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
        socket.set_timeout(Some(Duration::from_secs(30)));

        match with_timeout(Duration::from_secs(10), socket.connect(router_ep)).await {
            Ok(Ok(())) => info!("[zenoh] TCP connected"),
            Ok(Err(e)) => {
                match e {
                    embassy_net::tcp::ConnectError::InvalidState => {
                        error!("[zenoh] connect failed: InvalidState")
                    }
                    embassy_net::tcp::ConnectError::ConnectionReset => {
                        error!("[zenoh] connect failed: ConnectionReset")
                    }
                    embassy_net::tcp::ConnectError::TimedOut => {
                        error!("[zenoh] connect failed: TimedOut")
                    }
                    embassy_net::tcp::ConnectError::NoRoute => {
                        error!("[zenoh] connect failed: NoRoute")
                    }
                }
                reconnect.wait_and_advance().await;
                continue;
            }
            Err(_) => {
                warn!("[zenoh] TCP connect timeout");
                reconnect.wait_and_advance().await;
                continue;
            }
        }

        // Zenoh handshake + Node setup
        let mut node = match NodeBuilder::new("tenshi_no_hana")
            .zid(cfg.zenoh.session.zid)
            .domain_id(cfg.zenoh.session.domain_id)
            .build(socket)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                error!("[zenoh] handshake failed: {}", e);
                reconnect.wait_and_advance().await;
                continue;
            }
        };

        // Register publisher
        if let Err(e) = node.register_static_publisher(&CHATTER_PUB).await {
            error!("[zenoh] publisher reg failed: {}", e);
            reconnect.wait_and_advance().await;
            continue;
        }

        // Subscribe
        CHATTER_SUB.clear();
        if let Err(e) = node
            .subscribe_with_dispatch(CHATTER_TOPIC, &CHATTER_SUB)
            .await
        {
            error!("[zenoh] subscribe failed: {}", e);
            reconnect.wait_and_advance().await;
            continue;
        }

        info!("[zenoh] Node ready");
        node.spin_and_backoff(&mut reconnect).await;
        warn!("[zenoh] Session ended — reconnecting...");
    }
}

/// Application logic: publish counter message every 5s, echo received messages.
#[embassy_executor::task]
async fn app_task() {
    let mut counter: u32 = 0;
    loop {
        // Publish
        let mut data: String<128> = String::new();
        let _ = core::fmt::write(
            &mut data,
            core::format_args!("Hello from tenshi-no-hana! count={}", counter),
        );
        counter += 1;

        if let Err(e) = CHATTER_PUB.send(&StringMsg { data }).await {
            error!("[app] publish error: {}", e);
        }

        // Receive
        while let Some(result) = CHATTER_SUB.try_recv() {
            match result {
                Ok(m) => info!("[app] /chatter: {=str}", m.data.as_str()),
                Err(e) => warn!("[app] deserialize error: {}", e),
            }
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}
