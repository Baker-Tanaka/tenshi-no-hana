// motor_test.rs — Standalone DRV8835 wiring / direction bring-up.
//
// Drives both motors through a fixed sequence: forward → spin-in-place →
// reverse → stop, repeated. WiFi and micro-ROS are not involved so this
// example builds with the default `embassy` feature alone.
//
// Build & flash:
//   cargo run --release --example motor_test
//
// Pinout (DRV8835 PH/EN mode, MODE pin tied HIGH):
//   GP10 → APHASE (left direction)
//   GP11 → AENABLE (left PWM, slice 5 ch B)
//   GP12 → BPHASE (right direction)
//   GP9  → BENABLE (right PWM, slice 4 ch B)
#![no_std]
#![no_main]

#[path = "../src/drv8835.rs"]
mod drv8835;

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::pwm::{Config as PwmConfig, Pwm};
use embassy_time::{Duration, Timer};
use panic_probe as _;

use drv8835::{Drv8835, DEFAULT_PWM_TOP};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("[motor_test] start");

    let mut pwm_cfg = PwmConfig::default();
    pwm_cfg.top = DEFAULT_PWM_TOP;
    pwm_cfg.compare_b = 0;

    let a_phase = Output::new(p.PIN_10, Level::Low);
    let a_pwm = Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_cfg.clone());
    let b_phase = Output::new(p.PIN_12, Level::Low);
    let b_pwm = Pwm::new_output_b(p.PWM_SLICE4, p.PIN_9, pwm_cfg.clone());

    let mut motors = Drv8835::new(a_phase, a_pwm, pwm_cfg.clone(), b_phase, b_pwm, pwm_cfg);
    info!("[motor_test] DRV8835 ready");

    loop {
        info!("[motor_test] forward 50%");
        motors.set_left(0.5);
        motors.set_right(0.5);
        Timer::after(Duration::from_secs(1)).await;

        info!("[motor_test] spin CCW (left back, right fwd)");
        motors.set_left(-0.5);
        motors.set_right(0.5);
        Timer::after(Duration::from_secs(1)).await;

        info!("[motor_test] reverse 50%");
        motors.set_left(-0.5);
        motors.set_right(-0.5);
        Timer::after(Duration::from_secs(1)).await;

        info!("[motor_test] stop");
        motors.stop();
        Timer::after(Duration::from_secs(2)).await;
    }
}
