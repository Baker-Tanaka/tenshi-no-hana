// hw_test.rs — Standalone bring-up test for every sensor/actuator EXCEPT
// the XIAO ESP32-C3 WiFi link (no SPI, no esp-hosted-mcu, no micro-ROS).
//
// Use this to verify wiring/soldering of:
//   - BME280   (I2C0 @0x76)        → temperature, humidity, pressure
//   - MQ-3B    (ADC0, GP26)        → ethanol vapor analog voltage
//   - HC-SR04  (GP2 Trig/GP3 Echo) → ultrasonic distance
//   - ICM-20602 (I2C0 @0x68)       → accelerometer + gyro
//   - DRV8835  (GP9-12, PWM)       → left/right motor direction & speed
//
// All logs go over RTT (defmt). No WiFi/network/micro-ROS code runs, so a
// broken or unpowered XIAO ESP32-C3 has zero effect on this test.
//
// Build & flash:
//   cargo build --no-default-features --features wifi,sensor --example hw_test
#![no_std]
#![no_main]

#[path = "../src/drv8835.rs"]
mod drv8835;

use bme280::i2c::BME280;
use core::cell::RefCell;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::adc::{
    Adc, Channel as AdcChannel, Config as AdcConfig, InterruptHandler as AdcIrqHandler,
};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Config as I2cConfig, I2c};
use embassy_rp::pwm::Config as PwmConfig;
use embassy_rp::pwm::Pwm;
use embassy_time::{with_timeout, Delay, Duration, Instant, Timer};
use embedded_hal::i2c::I2c as I2cTrait;
use embedded_hal_bus::i2c::RefCellDevice;
use panic_probe as _;
use static_cell::StaticCell;

use drv8835::{Drv8835, DEFAULT_PWM_TOP};

bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => AdcIrqHandler;
});

type MyI2cBus = RefCell<I2c<'static, embassy_rp::peripherals::I2C0, embassy_rp::i2c::Blocking>>;

static I2C_BUS: StaticCell<MyI2cBus> = StaticCell::new();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    Timer::after(Duration::from_millis(500)).await;
    info!("========================================");
    info!(" hw_test — non-WiFi hardware bring-up");
    info!("========================================");

    // ── I2C0: BME280 @0x76, ICM-20602 @0x68 (shared bus via RefCell) ────────
    let raw_i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let i2c_bus = I2C_BUS.init(RefCell::new(raw_i2c));

    let bme_dev = RefCellDevice::new(i2c_bus);
    let mut bme280 = BME280::new_primary(bme_dev);
    match bme280.init(&mut Delay) {
        Ok(()) => info!("[bme280] initialized"),
        Err(_) => error!("[bme280] init FAILED — check SDA=GP4/SCL=GP5 wiring"),
    }

    {
        let mut imu_tmp = RefCellDevice::new(i2c_bus);
        match icm20602_init(&mut imu_tmp).await {
            Ok(()) => info!("[imu] ICM-20602 ready"),
            Err(()) => error!("[imu] ICM-20602 init FAILED — check I2C wiring / addr 0x68"),
        }
    }
    let mut imu_dev = RefCellDevice::new(i2c_bus);

    // ── ADC0: MQ-3B (GP26) ───────────────────────────────────────────────────
    let mut adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mut mq3_ch = AdcChannel::new_pin(p.PIN_26, Pull::None);
    info!("[mq3] ADC channel ready (GP26)");

    // ── HC-SR04: Trig=GP2, Echo=GP3 ──────────────────────────────────────────
    let mut trig = Output::new(p.PIN_2, Level::Low);
    let mut echo = Input::new(p.PIN_3, Pull::None);
    info!("[range] HC-SR04 ready (Trig=GP2, Echo=GP3)");

    // ── DRV8835: APHASE=GP10 AENABLE=GP11(PWM5B), BPHASE=GP12 BENABLE=GP9(PWM4B) ─
    let mut pwm_cfg = PwmConfig::default();
    pwm_cfg.top = DEFAULT_PWM_TOP;
    pwm_cfg.compare_b = 0;
    let a_phase = Output::new(p.PIN_10, Level::Low);
    let a_pwm = Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_cfg.clone());
    let b_phase = Output::new(p.PIN_12, Level::Low);
    let b_pwm = Pwm::new_output_b(p.PWM_SLICE4, p.PIN_9, pwm_cfg.clone());
    let mut motors = Drv8835::new(a_phase, a_pwm, pwm_cfg.clone(), b_phase, b_pwm, pwm_cfg);
    info!("[motor] DRV8835 ready");

    let mut count: u32 = 0;
    loop {
        count += 1;
        info!("---- cycle #{} ----", count);

        // BME280
        match bme280.measure(&mut Delay) {
            Ok(m) => info!(
                "[bme280] T={} degC  H={} %  P={} hPa",
                m.temperature,
                m.humidity,
                m.pressure / 100.0
            ),
            Err(_) => warn!("[bme280] measure FAILED"),
        }

        // MQ-3B
        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let v = raw as f32 * 3.3 / 4096.0;
                info!("[mq3] raw={} voltage={}V", raw, v);
            }
            Err(_) => warn!("[mq3] ADC read FAILED"),
        }

        // HC-SR04
        match hcsr04_measure(&mut trig, &mut echo).await {
            Some(d) => info!("[range] distance={} m", d),
            None => warn!("[range] echo timeout (out of range or wiring issue)"),
        }

        // ICM-20602
        match icm20602_read(&mut imu_dev) {
            Ok((ax, ay, az, gx, gy, gz)) => info!(
                "[imu] accel=({}, {}, {}) m/s^2  gyro=({}, {}, {}) rad/s",
                ax, ay, az, gx, gy, gz
            ),
            Err(()) => warn!("[imu] read FAILED"),
        }

        // DRV8835 — short direction/speed sweep every cycle
        info!("[motor] forward 40%");
        motors.set_left(0.4);
        motors.set_right(0.4);
        Timer::after(Duration::from_millis(800)).await;

        info!("[motor] spin CCW");
        motors.set_left(-0.4);
        motors.set_right(0.4);
        Timer::after(Duration::from_millis(800)).await;

        info!("[motor] reverse 40%");
        motors.set_left(-0.4);
        motors.set_right(-0.4);
        Timer::after(Duration::from_millis(800)).await;

        info!("[motor] stop");
        motors.stop();

        Timer::after(Duration::from_secs(2)).await;
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
