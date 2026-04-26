// wifi_zenoh_sensors.rs — WiFi → Zenoh → ROS2 sensor data publishing
//
// Baker link.dev (RP2040) → SPI0 → XIAO ESP32-C3 (esp-hosted-mcu)
// → WiFi → Zenoh Router → ROS2 sensor topics.
//
// Published topics:
//   /angel_nose/temperature  (std_msgs/Float32)  — BME280 temperature [°C]
//   /angel_nose/humidity     (std_msgs/Float32)  — BME280 relative humidity [%]
//   /angel_nose/pressure     (std_msgs/Float32)  — BME280 atmospheric pressure [hPa]
//   /angel_nose/ethanol      (std_msgs/Float32)  — MQ-3B analog voltage [V]
//
// Build:
//   cp wifi_config.json.example wifi_config.json  # edit credentials
//   cargo build --no-default-features --features wifi,sensor --example wifi_zenoh_sensors
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use wifi_config::AppConfig;

use bme280::i2c::BME280;
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
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcIrqHandler};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Blocking as I2cBlocking, Config as I2cConfig, I2c};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use panic_probe as _;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault (flip-link active)");
}
use static_cell::StaticCell;
use zenoh_ros2_nostd::prelude::*;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
    ADC_IRQ_FIFO => AdcIrqHandler;
});

// ── ROS2 topic definitions ───────────────────────────────────────────────────

const TEMP_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/temperature");
const HUMI_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/humidity");
const PRES_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/pressure");
const ETOH_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/ethanol");

static TEMP_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(TEMP_TOPIC);
static HUMI_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(HUMI_TOPIC);
static PRES_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(PRES_TOPIC);
static ETOH_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(ETOH_TOPIC);

// ── Type aliases ─────────────────────────────────────────────────────────────

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;

type MyI2c = I2c<'static, I2C0, I2cBlocking>;
type MyBme280 = BME280<MyI2c>;

// ── Static storage ───────────────────────────────────────────────────────────

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

// ── Entry point ──────────────────────────────────────────────────────────────

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] start");

    // ── SPI0 (esp-hosted) ────────────────────────────────────────────────────
    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 10_000_000;
    spi_cfg.polarity = Polarity::IdleHigh; // CPOL=1
    spi_cfg.phase = Phase::CaptureOnSecondTransition; // CPHA=1 (SPI Mode 3)

    let spi = Spi::new(
        p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, p.DMA_CH0, p.DMA_CH1, Irqs, spi_cfg,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    let handshake = Input::new(p.PIN_15, Pull::Down);
    let data_ready = Input::new(p.PIN_13, Pull::Down);
    let reset = Output::new(p.PIN_14, Level::Low);

    let spi_dev = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let spi_iface = SpiInterface::new(spi_dev, handshake, data_ready);

    // ── ESP-Hosted driver ────────────────────────────────────────────────────
    let esp_state = ESP_STATE.init(State::new());
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;

    spawner.spawn(esp_hosted_task(esp_runner).unwrap());

    // ── WiFi connect ─────────────────────────────────────────────────────────
    let cfg = AppConfig::new();
    info!("[wifi] connecting to \"{}\"...", cfg.wifi_ssid);
    let esp_config = EspConfig {
        static_rx_buf_num: 10,
        dynamic_rx_buf_num: 32,
        tx_buf_type: BufferType::Dynamic,
        static_tx_buf_num: 0,
        dynamic_tx_buf_num: 32,
        rx_mgmt_buf_type: BufferType::Dynamic,
        rx_mgmt_buf_num: 20,
    };
    defmt::unwrap!(control.init(esp_config).await);
    let connected = defmt::unwrap!(control.connect(cfg.wifi_ssid, cfg.wifi_password).await);
    defmt::assert!(connected, "WiFi association failed");
    info!("[wifi] connected");

    // ── Network stack ────────────────────────────────────────────────────────
    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        0x1234_5678_9abc_def0u64,
    );

    // ── I2C0: GP4 (SDA), GP5 (SCL) → BME280 (blocking) ──────────────────────
    let i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let mut bme280: MyBme280 = BME280::new_primary(i2c);
    match bme280.init(&mut Delay) {
        Ok(()) => info!("[bme280] initialized"),
        Err(_) => error!("[bme280] init failed — check wiring"),
    }

    // ── ADC0: GP26 → MQ-3B ───────────────────────────────────────────────────
    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mq3_ch = Channel::new_pin(p.PIN_26, Pull::None);

    // ── Spawn tasks ───────────────────────────────────────────────────────────
    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(zenoh_task(stack).unwrap());
    spawner.spawn(sensor_task(bme280, adc, mq3_ch).unwrap());
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

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
async fn zenoh_task(stack: Stack<'static>) {
    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();

    let cfg = AppConfig::new();
    let mut reconnect = ReconnectPolicy::default_policy();
    let tcp_rx = TCP_RX.init([0u8; 4096]);
    let tcp_tx = TCP_TX.init([0u8; 4096]);
    let router_ep = cfg.zenoh.router_endpoint();

    loop {
        while stack.config_v4().is_none() {
            Timer::after(Duration::from_millis(500)).await;
        }
        info!("[zenoh] DHCP OK");

        let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
        socket.set_timeout(Some(Duration::from_secs(30)));

        match with_timeout(Duration::from_secs(10), socket.connect(router_ep)).await {
            Ok(Ok(())) => info!("[zenoh] TCP connected"),
            _ => {
                warn!("[zenoh] TCP connect failed");
                reconnect.wait_and_advance().await;
                continue;
            }
        }

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

        let pubs = [
            node.register_static_publisher(&TEMP_PUB).await,
            node.register_static_publisher(&HUMI_PUB).await,
            node.register_static_publisher(&PRES_PUB).await,
            node.register_static_publisher(&ETOH_PUB).await,
        ];
        if pubs.iter().any(|r| r.is_err()) {
            error!("[zenoh] publisher registration failed");
            reconnect.wait_and_advance().await;
            continue;
        }

        info!("[zenoh] Node ready — 4 publishers registered");
        node.spin_and_backoff(&mut reconnect).await;
        warn!("[zenoh] session ended — reconnecting");
    }
}

/// Read BME280 + MQ-3 and publish to Zenoh/ROS2 every 5 seconds.
#[embassy_executor::task]
async fn sensor_task(
    mut bme280: MyBme280,
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut mq3_ch: Channel<'static>,
) {
    let mut count: u32 = 0;
    loop {
        count += 1;

        match bme280.measure(&mut Delay) {
            Ok(m) => {
                let pres = m.pressure / 100.0; // Pa → hPa
                info!(
                    "[sensor] #{}: T={} °C  H={} %  P={} hPa",
                    count, m.temperature, m.humidity, pres
                );
                let _ = TEMP_PUB
                    .send(&msg::std_msgs::Float32Msg {
                        data: m.temperature,
                    })
                    .await;
                let _ = HUMI_PUB
                    .send(&msg::std_msgs::Float32Msg { data: m.humidity })
                    .await;
                let _ = PRES_PUB
                    .send(&msg::std_msgs::Float32Msg { data: pres })
                    .await;
            }
            Err(_) => warn!("[sensor] #{}: BME280 read failed", count),
        }

        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let voltage = raw as f32 * 3.3 / 4096.0;
                info!("[sensor] #{}: MQ-3 raw={} V={}", count, raw, voltage);
                let _ = ETOH_PUB
                    .send(&msg::std_msgs::Float32Msg { data: voltage })
                    .await;
            }
            Err(_) => warn!("[sensor] #{}: MQ-3 ADC failed", count),
        }

        Timer::after(Duration::from_secs(5)).await;
    }
}
