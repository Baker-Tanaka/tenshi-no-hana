// main.rs — 天使の鼻 (Angel's Nose) rover firmware.
//
// Baker link. Dev (RP2040) + XIAO ESP32-C3 (esp-hosted-mcu, SPI WiFi
// co-processor) rover. Publishes all onboard sensors to ROS2 via a
// micro-ROS Agent over WiFi/TCP, and subscribes to `/cmd_vel` to drive a
// DRV8835 dual H-bridge in PH/EN mode.
//
// ROS2 interface (namespace `/angel_nose/`):
//
//   Subscriptions:
//     /angel_nose/cmd_vel       geometry_msgs/Twist
//
//   Publications:
//     /angel_nose/temperature   std_msgs/Float32   BME280 [°C]
//     /angel_nose/humidity      std_msgs/Float32   BME280 [%]
//     /angel_nose/pressure      std_msgs/Float32   BME280 [hPa]
//     /angel_nose/ethanol       std_msgs/Float32   MQ-3B  [V]
//     /angel_nose/range         sensor_msgs/Range  HC-SR04
//     /angel_nose/imu           sensor_msgs/Imu    ICM-20602
//     /angel_nose/odom          nav_msgs/Odometry  open-loop dead-reckoning
//     /tf                       tf2_msgs/TFMessage odom → base_link
//
// Safety: a 500 ms watchdog stops the motors if no /cmd_vel arrives — same
// default as turtlebot3's `cmd_vel_timeout`.
//
// Verify with stock ROS2 tooling once flashed and the micro-ROS Agent is up
// (see docs/Note.md for `docker compose up -d`):
//
//   ros2 topic list
//   ros2 topic echo /angel_nose/temperature
//   ros2 topic echo /angel_nose/humidity
//   ros2 topic echo /angel_nose/pressure
//   ros2 topic echo /angel_nose/ethanol
//   ros2 topic echo /angel_nose/range
//   ros2 topic echo /angel_nose/imu
//   ros2 topic echo /angel_nose/odom
//   ros2 topic echo /tf
//   ros2 run teleop_twist_keyboard teleop_twist_keyboard \
//     --ros-args -r cmd_vel:=/angel_nose/cmd_vel
//
// Build (default features already select wifi+sensor, see Cargo.toml):
//   cargo build --release
#![no_std]
#![no_main]

mod diff_drive;
mod drv8835;
mod ros_msgs;
mod wifi_config;

use bme280::i2c::BME280;
use core::cell::RefCell;
use cortex_m_rt::exception;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_rp::adc::{
    Adc, Channel as AdcChannel, Config as AdcConfig, InterruptHandler as AdcIrqHandler,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Blocking as I2cBlocking, Config as I2cConfig, I2c};
use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Instant, Timer};
use embedded_hal::i2c::I2c as I2cTrait;
use embedded_hal_bus::i2c::RefCellDevice;
use embedded_hal_bus::spi::ExclusiveDevice;
use micro_xrce_dds_rs::{client_key, subscription_slot, Context, Executor, Runtime, RuntimeConfig};
use panic_probe as _;
use portable_atomic::{AtomicU32, AtomicU64, Ordering};
use static_cell::StaticCell;
use wifi_config::AppConfig;

use diff_drive::{twist_to_wheels, DiffDriveConfig};
use drv8835::{Drv8835, DEFAULT_PWM_TOP};

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
const CMD_VEL_TIMEOUT_MS: u64 = 500;
const ODOM_PUBLISH_PERIOD_MS: u64 = 100;
const MAX_MOTOR_DUTY_SCALE: f32 = 0.6;

/// FT-DC-002 chassis: ~130 mm wheel separation, top speed ~0.4 m/s.
/// Hand-calibrate `MAX_WHEEL_SPEED` once encoders are available.
const WHEEL_SEPARATION_M: f32 = 0.13;
const MAX_WHEEL_SPEED_MPS: f32 = 0.4;

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;
type MyI2cBus = RefCell<I2c<'static, I2C0, I2cBlocking>>;
type MyI2cDev = RefCellDevice<'static, I2c<'static, I2C0, I2cBlocking>>;
type MyBme280 = BME280<MyI2cDev>;
type MyExecutor = Executor<TcpSocket<'static>>;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static I2C_BUS: StaticCell<MyI2cBus> = StaticCell::new();
static RUNTIME: Runtime = Runtime::new();

// Shared motion state from `motor_node` to `odom_node`.
// Values are the realized base velocity after wheel saturation and duty limit.
static MOTION_LINEAR_BITS: AtomicU32 = AtomicU32::new(0);
static MOTION_ANGULAR_BITS: AtomicU32 = AtomicU32::new(0);
static MOTION_LAST_UPDATE_US: AtomicU64 = AtomicU64::new(0);

subscription_slot!(static SUB_CMDVEL: ros_msgs::geometry_msgs::Twist);

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] rover start");

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

    // DRV8835 (PH/EN mode, MODE pin tied HIGH externally)
    //   APHASE=GP10  AENABLE=GP11 (PWM5_B)
    //   BPHASE=GP12  BENABLE=GP9  (PWM4_B)
    let mut pwm_cfg = PwmConfig::default();
    pwm_cfg.top = DEFAULT_PWM_TOP;
    pwm_cfg.compare_b = 0;
    let a_phase = Output::new(p.PIN_10, Level::Low);
    let a_pwm = Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_cfg.clone());
    let b_phase = Output::new(p.PIN_12, Level::Low);
    let b_pwm = Pwm::new_output_b(p.PWM_SLICE4, p.PIN_9, pwm_cfg.clone());
    let motors = Drv8835::new(a_phase, a_pwm, pwm_cfg.clone(), b_phase, b_pwm, pwm_cfg);
    info!("[motor] DRV8835 ready");

    spawner.spawn(net_task(net_runner).unwrap());

    // Wait for DHCP.
    while stack.config_v4().is_none() {
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("[microros] DHCP OK");

    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();
    let tcp_rx = TCP_RX.init([0u8; 4096]);
    let tcp_tx = TCP_TX.init([0u8; 4096]);

    let agent_ep = cfg.agent_endpoint();
    let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
    socket.set_timeout(Some(Duration::from_secs(30)));
    if with_timeout(Duration::from_secs(10), socket.connect(agent_ep))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
    {
        info!("[microros] TCP connected");
    } else {
        defmt::panic!("[microros] TCP connect timeout");
    }

    let (ctx, exec) = defmt::unwrap!(
        RUNTIME
            .start(socket, RuntimeConfig::new(0x81, client_key!()))
            .await
    );
    info!("[microros] runtime started");

    spawner.spawn(xrce_exec(exec).unwrap());
    spawner.spawn(env_node(ctx, bme280).unwrap());
    spawner.spawn(ethanol_node(ctx, adc, mq3_ch).unwrap());
    spawner.spawn(range_node(ctx, trig, echo).unwrap());
    spawner.spawn(imu_node(ctx, imu_dev).unwrap());
    spawner.spawn(motor_node(ctx, motors).unwrap());
    spawner.spawn(odom_node(ctx).unwrap());
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
async fn xrce_exec(exec: MyExecutor) -> ! {
    exec.run().await
}

#[embassy_executor::task]
async fn env_node(ctx: Context, mut bme280: MyBme280) -> ! {
    info!("[env] task start");
    let node = defmt::unwrap!(ctx.create_node("env").await);
    info!("[env] node created");
    let pub_temp = defmt::unwrap!(
        node.create_publisher::<ros_msgs::std_msgs::Float32>("/angel_nose/temperature")
            .await
    );
    let pub_humi = defmt::unwrap!(
        node.create_publisher::<ros_msgs::std_msgs::Float32>("/angel_nose/humidity")
            .await
    );
    let pub_pres = defmt::unwrap!(
        node.create_publisher::<ros_msgs::std_msgs::Float32>("/angel_nose/pressure")
            .await
    );
    info!("[env] publishers ready");

    loop {
        if let Ok(m) = bme280.measure(&mut Delay) {
            match pub_temp
                .publish(&ros_msgs::std_msgs::Float32 {
                    data: m.temperature,
                })
                .await
            {
                Ok(()) => info!("[env] temp pub ok {}", m.temperature),
                Err(_) => warn!("[env] temp pub FAILED"),
            }
            pub_humi
                .publish(&ros_msgs::std_msgs::Float32 { data: m.humidity })
                .await
                .ok();
            pub_pres
                .publish(&ros_msgs::std_msgs::Float32 {
                    data: m.pressure / 100.0,
                })
                .await
                .ok();
        } else {
            warn!("[env] BME280 measure FAILED");
        }
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

#[embassy_executor::task]
async fn ethanol_node(
    ctx: Context,
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut mq3_ch: AdcChannel<'static>,
) -> ! {
    info!("[ethanol] task start");
    let node = defmt::unwrap!(ctx.create_node("ethanol").await);
    info!("[ethanol] node created");
    let pub_etoh = defmt::unwrap!(
        node.create_publisher::<ros_msgs::std_msgs::Float32>("/angel_nose/ethanol")
            .await
    );
    info!("[ethanol] publisher ready");

    let mut count: u32 = 0;
    loop {
        count += 1;
        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let v = raw as f32 * 5.5 / 4096.0;
                if count > MQ3B_WARMUP_SAMPLES {
                    match pub_etoh
                        .publish(&ros_msgs::std_msgs::Float32 { data: v })
                        .await
                    {
                        Ok(()) => info!("[ethanol] pub ok v={}", v),
                        Err(_) => warn!("[ethanol] pub FAILED v={}", v),
                    }
                } else {
                    info!(
                        "[ethanol] warmup {}/{} raw={}",
                        count, MQ3B_WARMUP_SAMPLES, raw
                    );
                }
            }
            Err(_) => warn!("[ethanol] ADC read FAILED count={}", count),
        }
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

#[embassy_executor::task]
async fn range_node(ctx: Context, mut trig: Output<'static>, mut echo: Input<'static>) -> ! {
    info!("[range] task start");
    let node = defmt::unwrap!(ctx.create_node("range").await);
    info!("[range] node created");
    let pub_range = defmt::unwrap!(
        node.create_publisher::<ros_msgs::sensor_msgs::Range>("/angel_nose/range")
            .await
    );
    info!("[range] publisher ready");

    loop {
        if let Some(d) = hcsr04_measure(&mut trig, &mut echo).await {
            let m = ros_msgs::sensor_msgs::Range {
                radiation_type: ros_msgs::sensor_msgs::RANGE_ULTRASOUND,
                field_of_view: 0.2618,
                min_range: 0.02,
                max_range: 4.0,
                range: d,
                variance: 0.0,
                ..Default::default()
            };
            pub_range.publish(&m).await.ok();
        }
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

#[embassy_executor::task]
async fn imu_node(ctx: Context, mut imu_dev: MyI2cDev) -> ! {
    info!("[imu] task start");
    let node = defmt::unwrap!(ctx.create_node("imu").await);
    info!("[imu] node created");
    let pub_imu = defmt::unwrap!(
        node.create_publisher::<ros_msgs::sensor_msgs::Imu>("/angel_nose/imu")
            .await
    );
    info!("[imu] publisher ready");

    loop {
        if let Ok((ax, ay, az, gx, gy, gz)) = icm20602_read(&mut imu_dev) {
            let mut ori_cov = [0.0f64; 9];
            ori_cov[0] = -1.0; // orientation not estimated
            let m = ros_msgs::sensor_msgs::Imu {
                orientation: ros_msgs::geometry_msgs::Quaternion {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                orientation_covariance: ori_cov,
                angular_velocity: ros_msgs::geometry_msgs::Vector3 {
                    x: gx,
                    y: gy,
                    z: gz,
                },
                angular_velocity_covariance: [0.0; 9],
                linear_acceleration: ros_msgs::geometry_msgs::Vector3 {
                    x: ax,
                    y: ay,
                    z: az,
                },
                linear_acceleration_covariance: [0.0; 9],
                ..Default::default()
            };
            pub_imu.publish(&m).await.ok();
        }
        Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS)).await;
    }
}

/// /cmd_vel subscriber + DRV8835 driver + 500 ms watchdog.
///
/// Loop semantics:
///   * a Twist arrives within timeout → apply diff-drive kinematics & PWM
///   * no Twist within timeout        → stop motors, log once until next msg
#[embassy_executor::task]
async fn motor_node(ctx: Context, mut motors: Drv8835<'static>) -> ! {
    info!("[motor] task start");
    let node = defmt::unwrap!(ctx.create_node("motor").await);
    info!("[motor] node created");
    defmt::unwrap!(
        node.create_subscription("/angel_nose/cmd_vel", &SUB_CMDVEL)
            .await
    );
    info!("[motor] subscribed /angel_nose/cmd_vel");

    let cfg = DiffDriveConfig::new(WHEEL_SEPARATION_M, MAX_WHEEL_SPEED_MPS);
    let mut stopped = true;
    motors.stop();
    publish_motion_state(0.0, 0.0);

    loop {
        match with_timeout(Duration::from_millis(CMD_VEL_TIMEOUT_MS), SUB_CMDVEL.recv()).await {
            Ok(twist) => {
                let lx = twist.linear.x as f32;
                let az = twist.angular.z as f32;
                let (l, r) = twist_to_wheels(&cfg, lx, az);

                // Limit duty to protect 3V-rated motors when VM=5V.
                let l_limited = l * MAX_MOTOR_DUTY_SCALE;
                let r_limited = r * MAX_MOTOR_DUTY_SCALE;
                motors.set_left(l_limited);
                motors.set_right(r_limited);

                // Convert limited wheel commands back to realized base twist.
                let v_l = l_limited * MAX_WHEEL_SPEED_MPS;
                let v_r = r_limited * MAX_WHEEL_SPEED_MPS;
                let lin_realized = 0.5 * (v_l + v_r);
                let ang_realized = (v_r - v_l) / WHEEL_SEPARATION_M;
                publish_motion_state(lin_realized, ang_realized);

                stopped = false;
                info!(
                    "[/cmd_vel] lx={} az={} -> l={} r={} (limited l={} r={})",
                    lx, az, l, r, l_limited, r_limited
                );
            }
            Err(_) => {
                if !stopped {
                    motors.stop();
                    publish_motion_state(0.0, 0.0);
                    stopped = true;
                    warn!("[motor] cmd_vel timeout, stop");
                }
            }
        }
    }
}

/// Periodically publish `nav_msgs/Odometry` and a `tf2_msgs/TFMessage`
/// containing the canonical `odom → base_link` transform.
///
/// This task uses the build.rs-generated message types from `src/ros_msgs.rs`
/// (parsed from `msgs/`). For now it publishes an open-loop dead-reckoning
/// pose at 10 Hz integrated from the realized `/cmd_vel` so `rviz2` and
/// `ros2 topic echo /tf` see a live frame.
#[embassy_executor::task]
async fn odom_node(ctx: Context) -> ! {
    use ros_msgs::geometry_msgs::TransformStamped;
    use ros_msgs::nav_msgs::Odometry;
    use ros_msgs::std_msgs::Header;
    use ros_msgs::tf2_msgs::TFMessage;

    info!("[odom] task start");
    // Give sensor nodes a head start on entity creation. Odometry's deeply
    // nested types take longer for the agent to register, and odom's 10 Hz
    // publish loop saturates tx_channel for any task still creating.
    Timer::after(Duration::from_secs(5)).await;
    info!("[odom] starting create after delay");
    let node = defmt::unwrap!(ctx.create_node("odom").await);
    info!("[odom] node created");
    let pub_odom = defmt::unwrap!(node.create_publisher::<Odometry>("/angel_nose/odom").await);
    info!("[odom] /odom publisher ready");
    let pub_tf = defmt::unwrap!(node.create_publisher::<TFMessage>("/tf").await);
    info!("[odom] /tf publisher ready — ALL ODOM READY");

    let mut header = Header::default();
    let _ = header.frame_id.push_str("odom");

    let mut odom = Odometry::default();
    odom.header = header.clone();
    let _ = odom.child_frame_id.push_str("base_link");
    // Identity orientation (quaternion w=1).
    odom.pose.pose.orientation.w = 1.0;

    let mut tf_stamped = TransformStamped::default();
    tf_stamped.header = header.clone();
    let _ = tf_stamped.child_frame_id.push_str("base_link");
    tf_stamped.transform.rotation.w = 1.0;

    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut yaw = 0.0f32;

    let start = Instant::now();
    let mut last_tick = Instant::now();
    let mut iter: u32 = 0;
    loop {
        iter += 1;
        let now = Instant::now();
        let dt = (now - last_tick).as_micros() as f32 / 1_000_000.0;
        last_tick = now;

        let (lin, ang) = read_motion_state_with_timeout(now);
        x += lin * dt * cos_approx(yaw);
        y += lin * dt * sin_approx(yaw);
        yaw += ang * dt;

        let half = 0.5 * yaw;
        let qz = sin_approx(half);
        let qw = cos_approx(half);

        let elapsed = (Instant::now() - start).as_micros();
        let sec = (elapsed / 1_000_000) as i32;
        let nanosec = ((elapsed % 1_000_000) * 1_000) as u32;

        odom.header.stamp.sec = sec;
        odom.header.stamp.nanosec = nanosec;
        odom.pose.pose.position.x = x as f64;
        odom.pose.pose.position.y = y as f64;
        odom.pose.pose.position.z = 0.0;
        odom.pose.pose.orientation.x = 0.0;
        odom.pose.pose.orientation.y = 0.0;
        odom.pose.pose.orientation.z = qz as f64;
        odom.pose.pose.orientation.w = qw as f64;
        odom.twist.twist.linear.x = lin as f64;
        odom.twist.twist.angular.z = ang as f64;

        // Modest covariance for open-loop dead-reckoning.
        odom.pose.covariance[0] = 0.04;
        odom.pose.covariance[7] = 0.04;
        odom.pose.covariance[35] = 0.20;
        odom.twist.covariance[0] = 0.09;
        odom.twist.covariance[35] = 0.30;

        match pub_odom.publish(&odom).await {
            Ok(()) => {
                if iter <= 3 || iter % 10 == 0 {
                    info!("[odom] /odom pub ok iter={}", iter);
                }
            }
            Err(_) => warn!("[odom] /odom pub FAILED iter={}", iter),
        }

        tf_stamped.header.stamp.sec = sec;
        tf_stamped.header.stamp.nanosec = nanosec;
        tf_stamped.transform.translation.x = x as f64;
        tf_stamped.transform.translation.y = y as f64;
        tf_stamped.transform.translation.z = 0.0;
        tf_stamped.transform.rotation.x = 0.0;
        tf_stamped.transform.rotation.y = 0.0;
        tf_stamped.transform.rotation.z = qz as f64;
        tf_stamped.transform.rotation.w = qw as f64;
        let mut tf_msg = TFMessage::default();
        let _ = tf_msg.transforms.push(tf_stamped.clone());
        pub_tf.publish(&tf_msg).await.ok();

        Timer::after(Duration::from_millis(ODOM_PUBLISH_PERIOD_MS)).await;
    }
}

fn publish_motion_state(linear: f32, angular: f32) {
    MOTION_LINEAR_BITS.store(linear.to_bits(), Ordering::Relaxed);
    MOTION_ANGULAR_BITS.store(angular.to_bits(), Ordering::Relaxed);
    MOTION_LAST_UPDATE_US.store(Instant::now().as_micros(), Ordering::Relaxed);
}

fn read_motion_state_with_timeout(now: Instant) -> (f32, f32) {
    let last = MOTION_LAST_UPDATE_US.load(Ordering::Relaxed);
    if now.as_micros().saturating_sub(last) > CMD_VEL_TIMEOUT_MS * 1_000 {
        return (0.0, 0.0);
    }
    (
        f32::from_bits(MOTION_LINEAR_BITS.load(Ordering::Relaxed)),
        f32::from_bits(MOTION_ANGULAR_BITS.load(Ordering::Relaxed)),
    )
}

fn wrap_pi(mut x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    const TAU: f32 = 2.0 * core::f32::consts::PI;
    while x > PI {
        x -= TAU;
    }
    while x < -PI {
        x += TAU;
    }
    x
}

fn sin_approx(x: f32) -> f32 {
    // 5th-order polynomial around 0 after wrapping to [-pi, pi].
    let x = wrap_pi(x);
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 + (x2 * x2) / 120.0)
}

fn cos_approx(x: f32) -> f32 {
    let x = wrap_pi(x);
    let x2 = x * x;
    1.0 - x2 / 2.0 + (x2 * x2) / 24.0
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
