//! Stand-alone DRV8835 motor check. No Wi-Fi, ROS or sensors.
//!
//! The wheels must be raised. Nothing moves for START_DELAY_S after boot so
//! power can be connected first. The sequence then repeats.
#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    gpio::{Level, Output},
    pwm::{Config as PwmConfig, Pwm},
};
use embassy_time::{Duration, Timer};
use panic_probe as _;
use tenshi_no_hana::drivers::drv8835::{Drv8835, DEFAULT_PWM_TOP};

const START_DELAY_S: u64 = 15;
// Below the 0.6 duty limit used by the rover firmware.
const DUTY: f32 = 0.5;
const RUN_MS: u64 = 1500;
const PAUSE_MS: u64 = 1500;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // Hold the ESP32 in reset so Wi-Fi traffic cannot disturb the supply.
    let _esp_reset = Output::new(p.PIN_14, Level::Low);

    let mut pwm_config = PwmConfig::default();
    pwm_config.top = DEFAULT_PWM_TOP;
    pwm_config.compare_b = 0;
    let mut motors = Drv8835::new(
        Output::new(p.PIN_10, Level::Low),
        Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_config.clone()),
        pwm_config.clone(),
        Output::new(p.PIN_12, Level::Low),
        Pwm::new_output_b(p.PWM_SLICE4, p.PIN_9, pwm_config.clone()),
        pwm_config,
    );

    info!(
        "[motor] check starts in {} s. Raise the wheels and connect motor power now",
        START_DELAY_S
    );
    Timer::after(Duration::from_secs(START_DELAY_S)).await;

    const STEPS: [(&str, f32, f32); 3] = [
        ("LEFT forward", DUTY, 0.0),
        ("RIGHT forward", 0.0, DUTY),
        ("BOTH forward", DUTY, DUTY),
    ];

    loop {
        for (label, left, right) in STEPS {
            info!("[motor] step {}", label);
            motors.set_left(left);
            motors.set_right(right);
            Timer::after(Duration::from_millis(RUN_MS)).await;
            motors.stop();
            info!("[motor] stopped");
            Timer::after(Duration::from_millis(PAUSE_MS)).await;
        }
        info!("[motor] cycle done; stopped");
        Timer::after(Duration::from_secs(5)).await;
    }
}
