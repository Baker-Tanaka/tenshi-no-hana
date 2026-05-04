// wifi_microros_sensors.rs — RP2040 → WiFi → micro-ROS Agent → ROS2 sensor topics.
//
// Published topics:
//   /angel_nose/temperature  std_msgs/Float32   BME280 [°C]
//   /angel_nose/humidity     std_msgs/Float32   BME280 [%]
//   /angel_nose/pressure     std_msgs/Float32   BME280 [hPa]
//   /angel_nose/ethanol      std_msgs/Float32   MQ-3B AOUT [V]
//   /angel_nose/range        sensor_msgs/Range  HC-SR04 [m]
//   /angel_nose/imu          sensor_msgs/Imu    ICM-20602 6-axis
//
// Build:
//   cargo build --release --no-default-features --features wifi,sensor --example wifi_microros_sensors
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use bme280::i2c::BME280;
use core::cell::RefCell;
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
use embassy_rp::adc::{
    Adc, Channel as AdcChannel, Config as AdcConfig, InterruptHandler as AdcIrqHandler,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Blocking as I2cBlocking, Config as I2cConfig, I2c};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{with_timeout, Delay, Duration, Instant, Timer};
use embedded_hal::i2c::I2c as I2cTrait;
use embedded_hal_bus::i2c::RefCellDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use micro_xrce_dds_rs::{client_key, msg, Session};
use panic_probe as _;
use static_cell::StaticCell;
use wifi_config::AppConfig;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

bind_interrupts!(struct Irqs {
    DMA_IRQ_0    => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
    ADC_IRQ_FIFO => AdcIrqHandler;
});

const SAMPLE_PERIOD_MS: u64 = 1_000;
const MQ3B_WARMUP_SAMPLES: u32 = (6_000 / SAMPLE_PERIOD_MS) as u32;

// ── Inter-task channels (sensor task → microros task) ────────────────────────
static TEMP_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static HUMI_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static PRES_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static ETOH_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static RANGE_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static IMU_CH: Channel<CriticalSectionRawMutex, ImuSample, 4> = Channel::new();

#[derive(Clone, Copy)]
struct ImuSample {
    ax: f64,
    ay: f64,
    az: f64,
    gx: f64,
    gy: f64,
    gz: f64,
}

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;
type MyI2cBus = RefCell<I2c<'static, I2C0, I2cBlocking>>;
type MyI2cDev = RefCellDevice<'static, I2c<'static, I2C0, I2cBlocking>>;
type MyBme280 = BME280<MyI2cDev>;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static I2C_BUS: StaticCell<MyI2cBus> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] start");

    // SPI0 → ESP32-C3 (Mode 3)
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

    // I2C0: BME280 @ 0x76, ICM-20602 @ 0x68
    let raw_i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let i2c_bus = I2C_BUS.init(RefCell::new(raw_i2c));

    let bme_dev = RefCellDevice::new(i2c_bus);
    let mut bme280: MyBme280 = BME280::new_primary(bme_dev);
    match bme280.init(&mut Delay) {
        Ok(()) => info!("[bme280] initialized"),
        Err(_) => error!("[bme280] init failed — check wiring"),
    }
    {
        let mut imu_tmp = RefCellDevice::new(i2c_bus);
        match icm20602_init(&mut imu_tmp).await {
            Ok(()) => info!("[imu] ICM-20602 ready"),
            Err(()) => error!("[imu] ICM-20602 init failed"),
        }
    }
    let imu_dev = RefCellDevice::new(i2c_bus);

    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mq3_ch = AdcChannel::new_pin(p.PIN_26, Pull::None);
    let trig = Output::new(p.PIN_2, Level::Low);
    let echo = Input::new(p.PIN_3, Pull::None);

    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(microros_task(stack).unwrap());
    spawner.spawn(sensor_task(bme280, adc, mq3_ch, trig, echo, imu_dev).unwrap());
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
        info!("[microros] DHCP OK");

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
        let pub_temp = defmt::unwrap!(
            session
                .create_publisher::<msg::std_msgs::Float32>(&node, "/angel_nose/temperature")
                .await
        );
        let pub_humi = defmt::unwrap!(
            session
                .create_publisher::<msg::std_msgs::Float32>(&node, "/angel_nose/humidity")
                .await
        );
        let pub_pres = defmt::unwrap!(
            session
                .create_publisher::<msg::std_msgs::Float32>(&node, "/angel_nose/pressure")
                .await
        );
        let pub_etoh = defmt::unwrap!(
            session
                .create_publisher::<msg::std_msgs::Float32>(&node, "/angel_nose/ethanol")
                .await
        );
        let pub_range = defmt::unwrap!(
            session
                .create_publisher::<msg::sensor_msgs::Range>(&node, "/angel_nose/range")
                .await
        );
        let pub_imu = defmt::unwrap!(
            session
                .create_publisher::<msg::sensor_msgs::Imu>(&node, "/angel_nose/imu")
                .await
        );
        info!("[microros] all publishers ready");

        loop {
            let mut any = false;
            if let Ok(v) = TEMP_CH.try_receive() {
                if let Err(e) = session.publish(&pub_temp, &msg::std_msgs::Float32(v)).await {
                    error!("[microros] temp: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = HUMI_CH.try_receive() {
                if let Err(e) = session.publish(&pub_humi, &msg::std_msgs::Float32(v)).await {
                    error!("[microros] humi: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = PRES_CH.try_receive() {
                if let Err(e) = session.publish(&pub_pres, &msg::std_msgs::Float32(v)).await {
                    error!("[microros] pres: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = ETOH_CH.try_receive() {
                if let Err(e) = session.publish(&pub_etoh, &msg::std_msgs::Float32(v)).await {
                    error!("[microros] etoh: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(range_m) = RANGE_CH.try_receive() {
                let m = msg::sensor_msgs::Range {
                    radiation_type: msg::sensor_msgs::RANGE_ULTRASOUND,
                    field_of_view: 0.2618,
                    min_range: 0.02,
                    max_range: 4.0,
                    range: range_m,
                    variance: 0.0,
                };
                if let Err(e) = session.publish(&pub_range, &m).await {
                    error!("[microros] range: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(s) = IMU_CH.try_receive() {
                let m = msg::sensor_msgs::Imu {
                    orientation: [0.0, 0.0, 0.0, 1.0],
                    orientation_covariance: [-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    angular_velocity: [s.gx, s.gy, s.gz],
                    angular_velocity_covariance: [0.0; 9],
                    linear_acceleration: [s.ax, s.ay, s.az],
                    linear_acceleration_covariance: [0.0; 9],
                };
                if let Err(e) = session.publish(&pub_imu, &m).await {
                    error!("[microros] imu: {}", e);
                    break;
                }
                any = true;
            }
            if !any {
                Timer::after(Duration::from_millis(10)).await;
            }
        }
        warn!("[microros] connection lost, reconnecting");
        Timer::after(Duration::from_secs(2)).await;
    }
}

#[embassy_executor::task]
async fn sensor_task(
    mut bme280: MyBme280,
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut mq3_ch: AdcChannel<'static>,
    mut trig: Output<'static>,
    mut echo: Input<'static>,
    mut imu_dev: MyI2cDev,
) {
    let mut count: u32 = 0;
    loop {
        count += 1;
        if let Ok(m) = bme280.measure(&mut Delay) {
            info!(
                "[sensor] #{}: T={} H={} P={}",
                count,
                m.temperature,
                m.humidity,
                m.pressure / 100.0
            );
            let _ = TEMP_CH.try_send(m.temperature);
            let _ = HUMI_CH.try_send(m.humidity);
            let _ = PRES_CH.try_send(m.pressure / 100.0);
        }
        if let Ok(raw) = adc.read(&mut mq3_ch).await {
            let v = raw as f32 * 5.5 / 4096.0;
            if count > MQ3B_WARMUP_SAMPLES {
                let _ = ETOH_CH.try_send(v);
            }
        }
        if let Some(d) = hcsr04_measure(&mut trig, &mut echo).await {
            let _ = RANGE_CH.try_send(d);
        }
        if let Ok((ax, ay, az, gx, gy, gz)) = icm20602_read(&mut imu_dev) {
            let _ = IMU_CH.try_send(ImuSample {
                ax,
                ay,
                az,
                gx,
                gy,
                gz,
            });
        }
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

async fn hcsr04_measure(trig: &mut Output<'_>, echo: &mut Input<'_>) -> Option<f32> {
    trig.set_high();
    Timer::after(Duration::from_micros(10)).await;
    trig.set_low();
    if with_timeout(Duration::from_millis(30), echo.wait_for_high())
        .await
        .is_err()
    {
        return None;
    }
    let t1 = Instant::now();
    if with_timeout(Duration::from_millis(40), echo.wait_for_low())
        .await
        .is_err()
    {
        return None;
    }
    Some((Instant::now() - t1).as_micros() as f32 / 5800.0)
}

async fn icm20602_init(i2c: &mut impl I2cTrait) -> Result<(), ()> {
    let mut id = [0u8; 1];
    i2c.write_read(0x68, &[0x75], &mut id).map_err(|_| ())?;
    info!("[imu] WHO_AM_I=0x{:02X}", id[0]);
    i2c.write(0x68, &[0x6B, 0x01]).map_err(|_| ())?;
    Timer::after(Duration::from_millis(100)).await;
    Ok(())
}

fn icm20602_read(i2c: &mut impl I2cTrait) -> Result<(f64, f64, f64, f64, f64, f64), ()> {
    let mut buf = [0u8; 14];
    i2c.write_read(0x68, &[0x3B], &mut buf).map_err(|_| ())?;
    const A_SCALE: f64 = 2.0 * 9.80665 / 32768.0;
    const G_SCALE: f64 = 250.0 * core::f64::consts::PI / 180.0 / 32768.0;
    Ok((
        i16::from_be_bytes([buf[0], buf[1]]) as f64 * A_SCALE,
        i16::from_be_bytes([buf[2], buf[3]]) as f64 * A_SCALE,
        i16::from_be_bytes([buf[4], buf[5]]) as f64 * A_SCALE,
        i16::from_be_bytes([buf[8], buf[9]]) as f64 * G_SCALE,
        i16::from_be_bytes([buf[10], buf[11]]) as f64 * G_SCALE,
        i16::from_be_bytes([buf[12], buf[13]]) as f64 * G_SCALE,
    ))
}
