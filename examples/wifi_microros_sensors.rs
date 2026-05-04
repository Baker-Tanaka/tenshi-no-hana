// wifi_microros_sensors.rs — WiFi → Micro XRCE-DDS → micro-ROS Agent → ROS2 sensor topics
//
// Baker link. Dev (RP2040) → SPI0 → XIAO ESP32-C3 (esp-hosted-mcu)
// → WiFi → micro-ROS Agent (TCP port 8888) → ROS2 DDS.
//
// Published topics (6 total):
//   /angel_nose/temperature  (std_msgs/Float32)   — BME280 temperature [°C]
//   /angel_nose/humidity     (std_msgs/Float32)   — BME280 relative humidity [%]
//   /angel_nose/pressure     (std_msgs/Float32)   — BME280 atmospheric pressure [hPa]
//   /angel_nose/ethanol      (std_msgs/Float32)   — MQ-3B AOUT voltage [V]
//   /angel_nose/range        (sensor_msgs/Range)  — HC-SR04 distance [m]
//   /angel_nose/imu          (sensor_msgs/Imu)    — ICM-20602 6-axis IMU
//
// Build:
//   cp wifi_config.json.example wifi_config.json  # set ssid/password/agent_addr
//   cargo build --no-default-features --features wifi,sensor --example wifi_microros_sensors
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use wifi_config::AppConfig;

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
use micro_xrce_dds_rs::{
    protocol::{object_id, ENTITY_DATAWRITER},
    ros2::msg::{sensor_msgs, std_msgs},
    XrceSession,
};
use panic_probe as _;
use static_cell::StaticCell;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

bind_interrupts!(struct Irqs {
    DMA_IRQ_0    => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
    ADC_IRQ_FIFO => AdcIrqHandler;
});

const SAMPLE_PERIOD_MS: u64 = 1_000;

// const MQ3B_WARMUP_SAMPLES: u32 = (120_000 / SAMPLE_PERIOD_MS) as u32;
const MQ3B_WARMUP_SAMPLES: u32 = (6_000 / SAMPLE_PERIOD_MS) as u32;

// ObjectId = (idx << 4) | entity_type
const DW_TEMP: u16 = object_id(1, ENTITY_DATAWRITER);
const DW_HUMI: u16 = object_id(2, ENTITY_DATAWRITER);
const DW_PRES: u16 = object_id(3, ENTITY_DATAWRITER);
const DW_ETOH: u16 = object_id(4, ENTITY_DATAWRITER);
const DW_RANGE: u16 = object_id(5, ENTITY_DATAWRITER);
const DW_IMU: u16 = object_id(6, ENTITY_DATAWRITER);

static TEMP_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static HUMI_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static PRES_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();
static ETOH_CH: Channel<CriticalSectionRawMutex, f32, 4> = Channel::new();

struct RangeData {
    range: f32,
}
struct ImuData {
    ax: f64,
    ay: f64,
    az: f64,
    gx: f64,
    gy: f64,
    gz: f64,
}

static RANGE_CH: Channel<CriticalSectionRawMutex, RangeData, 4> = Channel::new();
static IMU_CH: Channel<CriticalSectionRawMutex, ImuData, 4> = Channel::new();

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
            Err(()) => error!("[imu] ICM-20602 init failed — see probe log above"),
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

    // XRCE-DDS session parameters
    const SESSION_ID: u8 = 0x81;
    const CLIENT_KEY: [u8; 4] = [0xBA, 0xCE, 0xA1, 0x05];

    loop {
        while stack.config_v4().is_none() {
            Timer::after(Duration::from_millis(500)).await;
        }
        info!("[microros] DHCP OK");

        let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
        socket.set_timeout(Some(Duration::from_secs(30)));

        match with_timeout(Duration::from_secs(10), socket.connect(agent_ep)).await {
            Ok(Ok(())) => info!("[microros] TCP connected to agent"),
            _ => {
                warn!("[microros] TCP connect failed — retry in 3s");
                Timer::after(Duration::from_secs(3)).await;
                continue;
            }
        }

        // Establish XRCE-DDS session (15s timeout — agent must send STATUS_AGENT)
        let mut session = match with_timeout(
            Duration::from_secs(15),
            XrceSession::connect(socket, SESSION_ID, CLIENT_KEY),
        )
        .await
        {
            Ok(Ok(s)) => {
                info!("[microros] XRCE-DDS session established");
                s
            }
            Ok(Err(e)) => {
                error!("[microros] session connect failed: {}", e);
                Timer::after(Duration::from_secs(3)).await;
                continue;
            }
            Err(_) => {
                error!("[microros] session connect timeout — no STATUS_AGENT in 15s");
                Timer::after(Duration::from_secs(3)).await;
                continue;
            }
        };

        // Create DDS entities
        if let Err(e) = create_entities(&mut session).await {
            error!("[microros] entity creation failed: {}", e);
            Timer::after(Duration::from_secs(3)).await;
            continue;
        }
        info!("[microros] all DDS entities ready — publishing");

        let dw_temp = micro_xrce_dds_rs::DataWriterId(DW_TEMP);
        let dw_humi = micro_xrce_dds_rs::DataWriterId(DW_HUMI);
        let dw_pres = micro_xrce_dds_rs::DataWriterId(DW_PRES);
        let dw_etoh = micro_xrce_dds_rs::DataWriterId(DW_ETOH);
        let dw_range = micro_xrce_dds_rs::DataWriterId(DW_RANGE);
        let dw_imu = micro_xrce_dds_rs::DataWriterId(DW_IMU);

        // Publish loop: drain inter-task channels and write to agent
        loop {
            let mut any = false;

            if let Ok(v) = TEMP_CH.try_receive() {
                let mut buf = [0u8; 8];
                let payload = std_msgs::serialize_float32(&mut buf, v);
                if let Err(e) = session.write_data(dw_temp, payload).await {
                    error!("[microros] write_data temp: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = HUMI_CH.try_receive() {
                let mut buf = [0u8; 8];
                let payload = std_msgs::serialize_float32(&mut buf, v);
                if let Err(e) = session.write_data(dw_humi, payload).await {
                    error!("[microros] write_data humi: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = PRES_CH.try_receive() {
                let mut buf = [0u8; 8];
                let payload = std_msgs::serialize_float32(&mut buf, v);
                if let Err(e) = session.write_data(dw_pres, payload).await {
                    error!("[microros] write_data pres: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(v) = ETOH_CH.try_receive() {
                let mut buf = [0u8; 8];
                let payload = std_msgs::serialize_float32(&mut buf, v);
                if let Err(e) = session.write_data(dw_etoh, payload).await {
                    error!("[microros] write_data etoh: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(rd) = RANGE_CH.try_receive() {
                let mut buf = [0u8; 48];
                let payload = sensor_msgs::serialize_range(
                    &mut buf,
                    sensor_msgs::RANGE_ULTRASOUND,
                    0.2618_f32,
                    0.02_f32,
                    4.0_f32,
                    rd.range,
                    0.0_f32,
                );
                if let Err(e) = session.write_data(dw_range, payload).await {
                    error!("[microros] write_data range: {}", e);
                    break;
                }
                any = true;
            }
            if let Ok(imu) = IMU_CH.try_receive() {
                let mut buf = [0u8; 320];
                let payload = sensor_msgs::serialize_imu(
                    &mut buf,
                    &[0.0, 0.0, 0.0, 1.0],
                    &[-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    &[imu.gx, imu.gy, imu.gz],
                    &[0.0; 9],
                    &[imu.ax, imu.ay, imu.az],
                    &[0.0; 9],
                );
                if let Err(e) = session.write_data(dw_imu, payload).await {
                    error!("[microros] write_data imu: {}", e);
                    break;
                }
                any = true;
            }

            if !any {
                Timer::after(Duration::from_millis(10)).await;
            }
        }

        warn!("[microros] connection lost — reconnecting");
        Timer::after(Duration::from_secs(2)).await;
    }
}

/// Create XRCE-DDS Participant, 6 Topics, 1 Publisher, and 6 DataWriters.
async fn create_entities<T: embedded_io_async::Read + embedded_io_async::Write>(
    s: &mut XrceSession<T>,
) -> Result<(), micro_xrce_dds_rs::XrceError> {
    const PARTICIPANT_IDX: u16 = 1;
    const PUBLISHER_IDX: u16 = 1;

    s.create_participant(PARTICIPANT_IDX, "tenshi_no_hana").await?;
    info!("[microros] Participant OK");

    // Topics (topic_idx matches the DataWriter index for clarity)
    s.create_topic(1, PARTICIPANT_IDX, "rt/angel_nose/temperature", std_msgs::FLOAT32_TYPE).await?;
    s.create_topic(2, PARTICIPANT_IDX, "rt/angel_nose/humidity",    std_msgs::FLOAT32_TYPE).await?;
    s.create_topic(3, PARTICIPANT_IDX, "rt/angel_nose/pressure",    std_msgs::FLOAT32_TYPE).await?;
    s.create_topic(4, PARTICIPANT_IDX, "rt/angel_nose/ethanol",     std_msgs::FLOAT32_TYPE).await?;
    s.create_topic(5, PARTICIPANT_IDX, "rt/angel_nose/range",       sensor_msgs::RANGE_TYPE).await?;
    s.create_topic(6, PARTICIPANT_IDX, "rt/angel_nose/imu",         sensor_msgs::IMU_TYPE).await?;
    info!("[microros] Topics OK");

    s.create_publisher(PUBLISHER_IDX, PARTICIPANT_IDX).await?;
    info!("[microros] Publisher OK");

    s.create_datawriter(1, PUBLISHER_IDX, "rt/angel_nose/temperature", std_msgs::FLOAT32_TYPE).await?;
    s.create_datawriter(2, PUBLISHER_IDX, "rt/angel_nose/humidity",    std_msgs::FLOAT32_TYPE).await?;
    s.create_datawriter(3, PUBLISHER_IDX, "rt/angel_nose/pressure",    std_msgs::FLOAT32_TYPE).await?;
    s.create_datawriter(4, PUBLISHER_IDX, "rt/angel_nose/ethanol",     std_msgs::FLOAT32_TYPE).await?;
    s.create_datawriter(5, PUBLISHER_IDX, "rt/angel_nose/range",       sensor_msgs::RANGE_TYPE).await?;
    s.create_datawriter(6, PUBLISHER_IDX, "rt/angel_nose/imu",         sensor_msgs::IMU_TYPE).await?;
    info!("[microros] DataWriters OK");

    Ok(())
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

        // BME280
        match bme280.measure(&mut Delay) {
            Ok(m) => {
                let pres = m.pressure / 100.0;
                info!(
                    "[sensor] #{}: T={} H={} P={}",
                    count, m.temperature, m.humidity, pres
                );
                let _ = TEMP_CH.try_send(m.temperature);
                let _ = HUMI_CH.try_send(m.humidity);
                let _ = PRES_CH.try_send(pres);
            }
            Err(_) => warn!("[sensor] #{}: BME280 read failed", count),
        }

        // MQ-3B
        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let voltage = raw as f32 * 5.5 / 4096.0;
                if count <= MQ3B_WARMUP_SAMPLES {
                    info!("[sensor] #{}: MQ-3 warming up raw={}", count, raw);
                } else {
                    info!("[sensor] #{}: MQ-3 AOUT={}V", count, voltage);
                    let _ = ETOH_CH.try_send(voltage);
                }
            }
            Err(_) => warn!("[sensor] #{}: MQ-3 ADC failed", count),
        }

        // HC-SR04
        match hcsr04_measure(&mut trig, &mut echo).await {
            Some(dist_m) => {
                info!("[sensor] #{}: HC-SR04 {}m", count, dist_m);
                let _ = RANGE_CH.try_send(RangeData { range: dist_m });
            }
            None => warn!("[sensor] #{}: HC-SR04 timeout", count),
        }

        // ICM-20602
        match icm20602_read(&mut imu_dev) {
            Ok((ax, ay, az, gx, gy, gz)) => {
                info!(
                    "[sensor] #{}: IMU accel=({} {} {}) gyro=({} {} {})",
                    count, ax as f32, ay as f32, az as f32, gx as f32, gy as f32, gz as f32
                );
                let _ = IMU_CH.try_send(ImuData {
                    ax,
                    ay,
                    az,
                    gx,
                    gy,
                    gz,
                });
            }
            Err(()) => {
                warn!("[sensor] #{}: ICM-20602 read failed — re-waking", count);
                let _ = icm20602_init(&mut imu_dev).await;
            }
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
    match i2c.write_read(0x68, &[0x75], &mut id) {
        Ok(()) => info!("[imu] probe: WHO_AM_I=0x{:02X}", id[0]),
        Err(_) => {
            error!("[imu] probe: no response at 0x68");
            match i2c.write_read(0x69, &[0x75], &mut id) {
                Ok(()) => warn!("[imu] device at 0x69 WHO_AM_I=0x{:02X}", id[0]),
                Err(_) => error!("[imu] no response at 0x68 or 0x69"),
            }
            return Err(());
        }
    }
    match i2c.write(0x68, &[0x6B, 0x01]) {
        Ok(()) => {}
        Err(_) => {
            error!("[imu] write PWR_MGMT_1 NACK");
            return Err(());
        }
    }
    Timer::after(Duration::from_millis(100)).await;
    match i2c.write_read(0x68, &[0x75], &mut id) {
        Ok(()) => {}
        Err(_) => {
            error!("[imu] WHO_AM_I read after wake failed");
            return Err(());
        }
    }
    match id[0] {
        0x12 => info!("[imu] ICM-20602 confirmed"),
        0x11 => warn!("[imu] WHO_AM_I=0x11 (ICM-20600, compatible)"),
        0x68 => warn!("[imu] WHO_AM_I=0x68 (MPU-6050, compatible)"),
        0x70 => warn!("[imu] WHO_AM_I=0x70 (MPU-6500, compatible)"),
        other => {
            error!("[imu] WHO_AM_I=0x{:02X} unexpected", other);
            return Err(());
        }
    }
    Ok(())
}

fn icm20602_read(i2c: &mut impl I2cTrait) -> Result<(f64, f64, f64, f64, f64, f64), ()> {
    let mut buf = [0u8; 14];
    i2c.write_read(0x68, &[0x3B], &mut buf).map_err(|_| ())?;

    const A_SCALE: f64 = 2.0 * 9.80665 / 32768.0;
    const G_SCALE: f64 = 250.0 * core::f64::consts::PI / 180.0 / 32768.0;

    let ax = i16::from_be_bytes([buf[0], buf[1]]) as f64 * A_SCALE;
    let ay = i16::from_be_bytes([buf[2], buf[3]]) as f64 * A_SCALE;
    let az = i16::from_be_bytes([buf[4], buf[5]]) as f64 * A_SCALE;
    let gx = i16::from_be_bytes([buf[8], buf[9]]) as f64 * G_SCALE;
    let gy = i16::from_be_bytes([buf[10], buf[11]]) as f64 * G_SCALE;
    let gz = i16::from_be_bytes([buf[12], buf[13]]) as f64 * G_SCALE;

    Ok((ax, ay, az, gx, gy, gz))
}
