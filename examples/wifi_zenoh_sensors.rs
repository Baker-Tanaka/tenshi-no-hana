// wifi_zenoh_sensors.rs — WiFi → Zenoh → ROS2 sensor data publishing
//
// Baker link.dev (RP2040) → SPI0 → XIAO ESP32-C3 (esp-hosted-mcu)
// → WiFi → Zenoh Router → ROS2 sensor topics.
//
// Published topics (6 total):
//   /angel_nose/temperature  (std_msgs/Float32)    — BME280 temperature [°C]
//   /angel_nose/humidity     (std_msgs/Float32)    — BME280 relative humidity [%]
//   /angel_nose/pressure     (std_msgs/Float32)    — BME280 atmospheric pressure [hPa]
//   /angel_nose/ethanol      (std_msgs/Float32)    — MQ-3B analog voltage [V]
//   /angel_nose/range        (sensor_msgs/Range)   — HC-SR04 distance [m]  GP2/GP3
//   /angel_nose/imu          (sensor_msgs/Imu)     — ICM-20602 6-axis IMU  I2C0 0x68
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
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcIrqHandler};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Blocking as I2cBlocking, Config as I2cConfig, I2c};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Instant, Timer};
use embedded_hal::i2c::I2c as I2cTrait;
use embedded_hal_bus::i2c::RefCellDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use panic_probe as _;
use static_cell::StaticCell;
use zenoh_ros2_nostd::prelude::*;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault (flip-link active)");
}

bind_interrupts!(struct Irqs {
    DMA_IRQ_0    => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
    ADC_IRQ_FIFO => AdcIrqHandler;
    // IO_IRQ_BANK0: embassy-rp 0.10 registers this internally; no bind needed.
});

/// Sensor sampling interval.  Edit this value and recompile to change the period.
const SAMPLE_PERIOD_MS: u64 = 1_000;

const TEMP_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/temperature");
const HUMI_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/humidity");
const PRES_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/pressure");
const ETOH_TOPIC: TopicKeyExpr = msg::std_msgs::Float32Type::topic(0, "angel_nose/ethanol");
const RANGE_TOPIC: TopicKeyExpr = msg::sensor_msgs::RangeType::topic(0, "angel_nose/range");
const IMU_TOPIC: TopicKeyExpr = msg::sensor_msgs::ImuType::topic(0, "angel_nose/imu");

static TEMP_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(TEMP_TOPIC);
static HUMI_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(HUMI_TOPIC);
static PRES_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(PRES_TOPIC);
static ETOH_PUB: Publisher<msg::std_msgs::Float32Msg, 8, 4> = Publisher::new(ETOH_TOPIC);
static RANGE_PUB: Publisher<msg::sensor_msgs::RangeMsg, { msg::sensor_msgs::RANGE_CDR_CAP }, 4> =
    Publisher::new(RANGE_TOPIC);
static IMU_PUB: Publisher<msg::sensor_msgs::ImuMsg, { msg::sensor_msgs::IMU_CDR_CAP }, 4> =
    Publisher::new(IMU_TOPIC);

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;

// RefCellDevice<'a, T> holds &'a RefCell<T>; the type parameter T is the inner bus.
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

    // I2C0 shared bus: BME280 @ 0x76 (primary) + ICM-20602 @ 0x68
    let raw_i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let i2c_bus = I2C_BUS.init(RefCell::new(raw_i2c));

    let bme_dev = RefCellDevice::new(i2c_bus);
    let mut bme280: MyBme280 = BME280::new_primary(bme_dev);
    match bme280.init(&mut Delay) {
        Ok(()) => info!("[bme280] initialized"),
        Err(_) => error!("[bme280] init failed — check wiring"),
    }

    // Wake ICM-20602 and verify WHO_AM_I before spawning sensor task
    {
        let mut imu_tmp = RefCellDevice::new(i2c_bus);
        match icm20602_init(&mut imu_tmp).await {
            Ok(()) => info!("[imu] ICM-20602 ready"),
            Err(()) => error!("[imu] ICM-20602 init failed — see probe log above"),
        }
    }
    let imu_dev = RefCellDevice::new(i2c_bus);

    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mq3_ch = Channel::new_pin(p.PIN_26, Pull::None);

    // HC-SR04: GP2 = Trig out, GP3 = Echo in
    let trig = Output::new(p.PIN_2, Level::Low);
    let echo = Input::new(p.PIN_3, Pull::None);

    spawner.spawn(net_task(net_runner).unwrap());
    spawner.spawn(zenoh_task(stack).unwrap());
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

        // Each create_publisher returns a different PublisherHandle type, so check individually.
        let reg_ok = node.create_publisher(&TEMP_PUB).await.is_ok()
            && node.create_publisher(&HUMI_PUB).await.is_ok()
            && node.create_publisher(&PRES_PUB).await.is_ok()
            && node.create_publisher(&ETOH_PUB).await.is_ok()
            && node.create_publisher(&RANGE_PUB).await.is_ok()
            && node.create_publisher(&IMU_PUB).await.is_ok();
        if !reg_ok {
            error!("[zenoh] publisher registration failed");
            reconnect.wait_and_advance().await;
            continue;
        }

        info!("[zenoh] Node ready — 6 publishers registered");
        node.spin_and_backoff(&mut reconnect).await;
        warn!("[zenoh] session ended — reconnecting");
    }
}

#[embassy_executor::task]
async fn sensor_task(
    mut bme280: MyBme280,
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut mq3_ch: Channel<'static>,
    mut trig: Output<'static>,
    mut echo: Input<'static>,
    mut imu_dev: MyI2cDev,
) {
    let mut count: u32 = 0;
    loop {
        count += 1;

        // BME280: temperature, humidity, pressure
        match bme280.measure(&mut Delay) {
            Ok(m) => {
                let pres = m.pressure / 100.0; // Pa → hPa
                info!(
                    "[sensor] #{}: T={} °C  H={} %  P={} hPa",
                    count, m.temperature, m.humidity, pres
                );
                let _ = TEMP_PUB.try_send(&msg::std_msgs::Float32Msg {
                    data: m.temperature,
                });
                let _ = HUMI_PUB.try_send(&msg::std_msgs::Float32Msg { data: m.humidity });
                let _ = PRES_PUB.try_send(&msg::std_msgs::Float32Msg { data: pres });
            }
            Err(_) => warn!("[sensor] #{}: BME280 read failed", count),
        }

        // MQ-3B: ethanol analog voltage
        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let voltage = raw as f32 * 3.3 / 4096.0;
                info!("[sensor] #{}: MQ-3 raw={} V={}", count, raw, voltage);
                let _ = ETOH_PUB.try_send(&msg::std_msgs::Float32Msg { data: voltage });
            }
            Err(_) => warn!("[sensor] #{}: MQ-3 ADC failed", count),
        }

        // HC-SR04: ultrasonic distance
        match hcsr04_measure(&mut trig, &mut echo).await {
            Some(dist_m) => {
                info!("[sensor] #{}: HC-SR04 dist={} m", count, dist_m);
                if RANGE_PUB
                    .try_send(&msg::sensor_msgs::RangeMsg {
                        header: msg::sensor_msgs::Header::zero(),
                        radiation_type: msg::sensor_msgs::RANGE_ULTRASOUND,
                        field_of_view: 0.2618_f32, // HC-SR04 ~15° half-angle
                        min_range: 0.02_f32,
                        max_range: 4.0_f32,
                        range: dist_m,
                        variance: 0.0_f32,
                    })
                    .is_err()
                {
                    warn!("[sensor] #{}: range msg dropped (queue full)", count);
                }
            }
            None => warn!("[sensor] #{}: HC-SR04 echo timeout", count),
        }

        // ICM-20602: 6-axis IMU (accelerometer + gyroscope)
        match icm20602_read(&mut imu_dev) {
            Ok((ax, ay, az, gx, gy, gz)) => {
                info!(
                    "[sensor] #{}: IMU accel=({} {} {}) gyro=({} {} {})",
                    count, ax as f32, ay as f32, az as f32, gx as f32, gy as f32, gz as f32
                );
                if IMU_PUB
                    .try_send(&msg::sensor_msgs::ImuMsg {
                        header: msg::sensor_msgs::Header::zero(),
                        orientation: msg::sensor_msgs::Quaternion::IDENTITY,
                        // orientation_covariance[0]=-1 signals unknown (REP-145)
                        orientation_covariance: [-1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        angular_velocity: msg::sensor_msgs::Vector3d {
                            x: gx,
                            y: gy,
                            z: gz,
                        },
                        angular_velocity_covariance: [0.0; 9],
                        linear_acceleration: msg::sensor_msgs::Vector3d {
                            x: ax,
                            y: ay,
                            z: az,
                        },
                        linear_acceleration_covariance: [0.0; 9],
                    })
                    .is_err()
                {
                    warn!("[sensor] #{}: imu msg dropped (queue full)", count);
                }
            }
            Err(()) => {
                warn!("[sensor] #{}: ICM-20602 read failed — re-waking", count);
                // Re-issue wake command; next iteration will retry the read.
                let _ = icm20602_init(&mut imu_dev).await;
            }
        }

        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

async fn hcsr04_measure(trig: &mut Output<'_>, echo: &mut Input<'_>) -> Option<f32> {
    // 10 µs trigger pulse
    trig.set_high();
    Timer::after(Duration::from_micros(10)).await;
    trig.set_low();

    // Wait for echo rising edge (timeout 30 ms — no obstacle at >5 m)
    if with_timeout(Duration::from_millis(30), echo.wait_for_high())
        .await
        .is_err()
    {
        return None;
    }
    let t1 = Instant::now();

    // Wait for echo falling edge (timeout 40 ms ≈ 7 m max)
    if with_timeout(Duration::from_millis(40), echo.wait_for_low())
        .await
        .is_err()
    {
        return None;
    }
    let pulse_us = (Instant::now() - t1).as_micros();

    // distance [m] = pulse_width_µs / 58 / 100
    Some(pulse_us as f32 / 5800.0)
}

/// Wake up ICM-20602 (clear SLEEP in PWR_MGMT_1) and verify WHO_AM_I.
///
/// Requires 100 ms after the wake write before the chip is ready.
async fn icm20602_init(i2c: &mut impl I2cTrait) -> Result<(), ()> {
    // Step 1: Probe — read WHO_AM_I without any write first.
    // This tells us whether the device responds at all before we try to change its state.
    let mut id = [0u8; 1];
    match i2c.write_read(0x68, &[0x75], &mut id) {
        Ok(()) => info!("[imu] probe: 0x68 WHO_AM_I=0x{:02X}", id[0]),
        Err(_) => {
            error!("[imu] probe: no response at 0x68 (device absent or bus error)");
            // Try alternate address (AD0 pin HIGH → 0x69)
            match i2c.write_read(0x69, &[0x75], &mut id) {
                Ok(()) => warn!(
                    "[imu] probe: device at 0x69 (AD0=HIGH) WHO_AM_I=0x{:02X} — update addr",
                    id[0]
                ),
                Err(_) => error!("[imu] probe: no response at 0x68 or 0x69"),
            }
            return Err(());
        }
    }

    // Step 2: Clear SLEEP bit (PWR_MGMT_1 = 0x01, CLKSEL=auto-PLL)
    match i2c.write(0x68, &[0x6B, 0x01]) {
        Ok(()) => {}
        Err(_) => {
            error!("[imu] write PWR_MGMT_1 NACK (device read-only? wrong address?)");
            return Err(());
        }
    }
    // ICM-20602 datasheet: needs ≥100 ms after exiting Sleep Mode
    Timer::after(Duration::from_millis(100)).await;

    // Step 3: Confirm WHO_AM_I after wake
    match i2c.write_read(0x68, &[0x75], &mut id) {
        Ok(()) => {}
        Err(_) => {
            error!("[imu] WHO_AM_I read after wake failed");
            return Err(());
        }
    }
    // Accept ICM-20602 (0x12) and closely compatible chips
    match id[0] {
        0x12 => info!("[imu] ICM-20602 confirmed (WHO_AM_I=0x12)"),
        0x11 => warn!("[imu] WHO_AM_I=0x11 (ICM-20600, compatible)"),
        0x68 => warn!("[imu] WHO_AM_I=0x68 (MPU-6050, compatible)"),
        0x70 => warn!("[imu] WHO_AM_I=0x70 (MPU-6500, compatible)"),
        other => {
            error!("[imu] WHO_AM_I=0x{:02X} (unexpected value)", other);
            return Err(());
        }
    }
    Ok(())
}

/// Burst-read accelerometer and gyroscope from ICM-20602.
///
/// Returns `(ax, ay, az, gx, gy, gz)` in m/s² and rad/s.
/// Uses default register config: ±2 g accel, ±250 °/s gyro.
fn icm20602_read(i2c: &mut impl I2cTrait) -> Result<(f64, f64, f64, f64, f64, f64), ()> {
    // Read 14 bytes: ACCEL_XOUT_H (0x3B) … GYRO_ZOUT_L (0x48)
    // Layout: ax_H ax_L  ay_H ay_L  az_H az_L  temp_H temp_L  gx_H gx_L  gy_H gy_L  gz_H gz_L
    let mut buf = [0u8; 14];
    i2c.write_read(0x68, &[0x3B], &mut buf).map_err(|_| ())?;

    // ACCEL_CONFIG = 0 → ±2 g  → LSB scale = 2 * 9.80665 / 32768
    // GYRO_CONFIG  = 0 → ±250 °/s → LSB scale = 250 * π / 180 / 32768
    const A_SCALE: f64 = 2.0 * 9.80665 / 32768.0;
    const G_SCALE: f64 = 250.0 * core::f64::consts::PI / 180.0 / 32768.0;

    let ax = i16::from_be_bytes([buf[0], buf[1]]) as f64 * A_SCALE;
    let ay = i16::from_be_bytes([buf[2], buf[3]]) as f64 * A_SCALE;
    let az = i16::from_be_bytes([buf[4], buf[5]]) as f64 * A_SCALE;
    // buf[6..8] = TEMP_OUT — skipped
    let gx = i16::from_be_bytes([buf[8], buf[9]]) as f64 * G_SCALE;
    let gy = i16::from_be_bytes([buf[10], buf[11]]) as f64 * G_SCALE;
    let gz = i16::from_be_bytes([buf[12], buf[13]]) as f64 * G_SCALE;

    Ok((ax, ay, az, gx, gy, gz))
}
