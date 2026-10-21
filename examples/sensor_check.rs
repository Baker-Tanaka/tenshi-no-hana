//! Stand-alone ICM-20602 and HC-SR04 check. No Wi-Fi, ROS or motor output.
//!
//! Keep the board still. Hold a flat object 10-100 cm in front of the
//! HC-SR04 during the range phase.
#![no_std]
#![no_main]

use defmt::{info, warn};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    gpio::{Input, Level, Output, Pull},
    i2c::{Blocking, Config as I2cConfig, I2c},
    peripherals::I2C0,
};
use embassy_time::{Duration, Timer};
use panic_probe as _;
use tenshi_no_hana::drivers::{hcsr04::Hcsr04, icm20602};

const IMU_SAMPLES: usize = 100;
const RANGE_SAMPLES: usize = 20;
const GRAVITY: f64 = 9.80665;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // Hold the ESP32 in reset so Wi-Fi traffic cannot disturb the supply.
    let _esp_reset = Output::new(p.PIN_14, Level::Low);

    let mut imu = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let mut range = Hcsr04::new(
        Output::new(p.PIN_2, Level::Low),
        Input::new(p.PIN_3, Pull::Down),
    );

    Timer::after(Duration::from_millis(500)).await;
    info!("[check] sensor check start");

    loop {
        let imu_ok = check_imu(&mut imu).await;
        let range_ok = check_range(&mut range).await;
        info!(
            "[check] RESULT imu={} range={}",
            if imu_ok { "PASS" } else { "FAIL" },
            if range_ok { "PASS" } else { "FAIL" }
        );
        Timer::after(Duration::from_secs(3)).await;
    }
}

async fn check_imu(i2c: &mut I2c<'static, I2C0, Blocking>) -> bool {
    let Ok(address) = icm20602::init(i2c).await else {
        warn!("[imu] not found at 0x68/0x69 (WHO_AM_I != 0x12)");
        return false;
    };
    info!("[imu] detected at 0x{:02x}", address);

    let mut sum_a = [0.0f64; 3];
    let mut sum_g = [0.0f64; 3];
    let mut min_g = [f64::MAX; 3];
    let mut max_g = [f64::MIN; 3];
    let mut good = 0usize;
    for _ in 0..IMU_SAMPLES {
        if let Ok(r) = icm20602::read(i2c, address) {
            good += 1;
            for i in 0..3 {
                sum_a[i] += r.acceleration[i];
                sum_g[i] += r.angular_velocity[i];
                min_g[i] = min_g[i].min(r.angular_velocity[i]);
                max_g[i] = max_g[i].max(r.angular_velocity[i]);
            }
        }
        Timer::after(Duration::from_millis(20)).await;
    }
    if good < IMU_SAMPLES * 9 / 10 {
        warn!("[imu] only {}/{} reads succeeded", good, IMU_SAMPLES);
        return false;
    }

    let n = good as f64;
    let a = [sum_a[0] / n, sum_a[1] / n, sum_a[2] / n];
    let g = [sum_g[0] / n, sum_g[1] / n, sum_g[2] / n];
    let magnitude = libm::sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
    let noise = (0..3)
        .map(|i| max_g[i] - min_g[i])
        .fold(0.0f64, |acc, v| acc.max(v));
    info!(
        "[imu] accel mean [m/s2] x={} y={} z={} |a|={}",
        a[0], a[1], a[2], magnitude
    );
    info!(
        "[imu] gyro mean [rad/s] x={} y={} z={} max peak-to-peak={}",
        g[0], g[1], g[2], noise
    );

    let gravity_ok = (0.9 * GRAVITY..1.1 * GRAVITY).contains(&magnitude);
    let still_ok = g.iter().all(|v| v.abs() < 0.1) && noise < 0.2;
    if !gravity_ok {
        warn!("[imu] |a| is not close to 9.81 m/s2");
    }
    if !still_ok {
        warn!("[imu] gyro is not still: board moved or gyro offset is large");
    }
    gravity_ok && still_ok
}

async fn check_range(sensor: &mut Hcsr04<'static>) -> bool {
    let mut valid = 0usize;
    let mut min_mm = u32::MAX;
    let mut max_mm = 0u32;
    for _ in 0..RANGE_SAMPLES {
        match sensor.measure().await {
            Some(metres) => {
                let mm = (metres * 1000.0) as u32;
                valid += 1;
                min_mm = min_mm.min(mm);
                max_mm = max_mm.max(mm);
                info!("[range] {} mm", mm);
            }
            None => info!("[range] no echo / out of range"),
        }
        Timer::after(Duration::from_millis(100)).await;
    }
    if valid == 0 {
        warn!("[range] no valid echo: check wiring, 5 V supply, echo divider, or target");
        return false;
    }
    info!(
        "[range] valid {}/{} min={} mm max={} mm",
        valid, RANGE_SAMPLES, min_mm, max_mm
    );
    valid * 4 >= RANGE_SAMPLES * 3
}
