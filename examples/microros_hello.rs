// microros_hello.rs — minimal WiFi → micro-ROS Agent hello-world.
//
// Publishes std_msgs/String on /angel_nose/hello once per second.
//
// Build:
//   cargo build --release --no-default-features --features wifi --example microros_hello
//
// Verify on ROS2 side (inside the ros2-node container):
//   ros2 topic echo /angel_nose/hello
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use cortex_m_rt::exception;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, Stack, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use heapless::String as HString;
use micro_xrce_dds_rs::{client_key, msg, Session};
use panic_probe as _;
use static_cell::StaticCell;
use wifi_config::AppConfig;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] microros_hello start");

    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 10_000_000;
    spi_cfg.polarity = Polarity::IdleHigh;
    spi_cfg.phase = Phase::CaptureOnSecondTransition;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, p.DMA_CH0, p.DMA_CH1, Irqs, spi_cfg,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    let handshake = Input::new(p.PIN_15, Pull::Down);
    let data_ready = Input::new(p.PIN_13, Pull::Down);
    let reset = Output::new(p.PIN_14, Level::Low);

    let spi_dev = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let spi_iface = SpiInterface::new(spi_dev, handshake, data_ready);
    let esp_state = ESP_STATE.init(State::new());
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;
    spawner.spawn(esp_hosted_task(esp_runner).unwrap());

    let cfg = AppConfig::new();
    info!("[wifi] connecting to \"{}\"...", cfg.wifi_ssid);
    defmt::unwrap!(
        control
            .init(EspConfig {
                static_rx_buf_num: 10,
                dynamic_rx_buf_num: 32,
                tx_buf_type: BufferType::Dynamic,
                static_tx_buf_num: 0,
                dynamic_tx_buf_num: 32,
                rx_mgmt_buf_type: BufferType::Dynamic,
                rx_mgmt_buf_num: 20,
            })
            .await
    );
    let connected = defmt::unwrap!(control.connect(cfg.wifi_ssid, cfg.wifi_password).await);
    defmt::assert!(connected, "WiFi association failed");
    info!("[wifi] connected");

    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        0x1234_5678_9abc_def0u64,
    );
    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(microros_task(stack).unwrap());
}

#[embassy_executor::task]
async fn esp_hosted_task(runner: MyEspRunner) {
    static TX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    runner
        .run(
            TX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE]),
            RX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE]),
        )
        .await
}

#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, NetDriver<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn microros_task(stack: Stack<'static>) {
    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();
    let cfg = AppConfig::new();
    let agent_ep = cfg.agent_endpoint();
    let tcp_rx = TCP_RX.init([0u8; 4096]);
    let tcp_tx = TCP_TX.init([0u8; 4096]);

    loop {
        while stack.config_v4().is_none() {
            Timer::after(Duration::from_millis(500)).await;
        }
        info!(
            "[microros] DHCP OK, connecting to agent {}.{}.{}.{}:{}",
            cfg.agent_ip[0], cfg.agent_ip[1], cfg.agent_ip[2], cfg.agent_ip[3], cfg.agent_port
        );

        let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
        socket.set_timeout(Some(Duration::from_secs(30)));

        if with_timeout(Duration::from_secs(10), socket.connect(agent_ep))
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false)
        {
            info!("[microros] TCP connected");
        } else {
            warn!("[microros] TCP connect failed, retry in 3s");
            Timer::after(Duration::from_secs(3)).await;
            continue;
        }

        let mut session = match Session::connect(socket, 0x81, client_key!()).await {
            Ok(s) => {
                info!("[microros] session OK");
                s
            }
            Err(e) => {
                error!("[microros] session connect failed: {}", e);
                Timer::after(Duration::from_secs(3)).await;
                continue;
            }
        };

        let node = defmt::unwrap!(session.create_node("tenshi_no_hana").await);
        let pub_hello = defmt::unwrap!(
            session
                .create_publisher::<msg::std_msgs::String>(&node, "/angel_nose/hello")
                .await
        );
        info!("[microros] publishing /angel_nose/hello");

        let mut count: u32 = 0;
        loop {
            count += 1;
            let mut buf: HString<32> = HString::new();
            let _ = core::fmt::write(&mut buf, format_args!("hello #{}", count));

            if let Err(e) = session
                .publish(&pub_hello, &msg::std_msgs::String(buf.as_str()))
                .await
            {
                error!("[microros] publish failed: {}", e);
                break;
            }
            info!("[microros] sent: {}", buf.as_str());
            Timer::after(Duration::from_secs(1)).await;
        }

        warn!("[microros] connection lost, reconnecting");
        Timer::after(Duration::from_secs(2)).await;
    }
}
