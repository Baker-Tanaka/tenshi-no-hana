// drv8835.rs — Texas Instruments DRV8835 dual H-bridge driver, PH/EN mode.
//
// In PH/EN mode (MODE pin tied HIGH), each motor uses two pins:
//   xPHASE  : direction (HIGH = forward, LOW = reverse)
//   xENABLE : PWM speed (0..top duty)
//
// Mapping for 天使の鼻:
//   Left  motor : APHASE=GP10 (GPIO out), AENABLE=GP11 (PWM5_B)
//   Right motor : BPHASE=GP12 (GPIO out), BENABLE=GP9  (PWM4_B)
//
// Both PWM enable pins are channel-B of their respective slices.

use embassy_rp::gpio::Output;
use embassy_rp::pwm::{Config as PwmConfig, Pwm};

/// Default PWM TOP. With sysclk=125 MHz and divider=1, top=4999 → ~25 kHz,
/// which sits above the audible range and well within the DRV8835 limit
/// (250 kHz). Duty resolution = 1/5000 ≈ 0.02 %.
pub const DEFAULT_PWM_TOP: u16 = 4999;

/// DRV8835 in Phase/Enable mode driving two DC motors.
pub struct Drv8835<'a> {
    a_phase: Output<'a>,
    b_phase: Output<'a>,
    a_pwm: Pwm<'a>,
    b_pwm: Pwm<'a>,
    a_cfg: PwmConfig,
    b_cfg: PwmConfig,
    top: u16,
}

impl<'a> Drv8835<'a> {
    /// Construct a driver with the given direction GPIOs and PWM peripherals.
    /// Both `a_pwm` / `b_pwm` must have been created with `Pwm::new_output_b`
    /// so that channel-B of each slice drives the ENABLE line.
    ///
    /// `pwm_top` is the PWM counter wrap value used by both slices. The two
    /// configs are configured to match (same `top`, same `divider`).
    pub fn new(
        a_phase: Output<'a>,
        a_pwm: Pwm<'a>,
        a_cfg: PwmConfig,
        b_phase: Output<'a>,
        b_pwm: Pwm<'a>,
        b_cfg: PwmConfig,
    ) -> Self {
        let top = a_cfg.top;
        let mut me = Self {
            a_phase,
            b_phase,
            a_pwm,
            b_pwm,
            a_cfg,
            b_cfg,
            top,
        };
        me.stop();
        me
    }

    /// Set left motor speed in the range [-1.0, 1.0].
    /// Sign chooses direction (positive = forward), magnitude sets PWM duty.
    pub fn set_left(&mut self, speed: f32) {
        let (forward, duty) = split_speed(speed, self.top);
        self.a_phase
            .set_level(if forward { embassy_rp::gpio::Level::High } else { embassy_rp::gpio::Level::Low });
        self.a_cfg.compare_b = duty;
        self.a_pwm.set_config(&self.a_cfg);
    }

    /// Set right motor speed in the range [-1.0, 1.0].
    pub fn set_right(&mut self, speed: f32) {
        let (forward, duty) = split_speed(speed, self.top);
        self.b_phase
            .set_level(if forward { embassy_rp::gpio::Level::High } else { embassy_rp::gpio::Level::Low });
        self.b_cfg.compare_b = duty;
        self.b_pwm.set_config(&self.b_cfg);
    }

    /// Coast both motors to a stop (PWM duty = 0). PHASE pins are left as-is;
    /// since ENABLE is 0 the H-bridge outputs are high-Z regardless.
    pub fn stop(&mut self) {
        self.a_cfg.compare_b = 0;
        self.b_cfg.compare_b = 0;
        self.a_pwm.set_config(&self.a_cfg);
        self.b_pwm.set_config(&self.b_cfg);
    }
}

fn split_speed(speed: f32, top: u16) -> (bool, u16) {
    let forward = speed >= 0.0;
    let mag = if speed.is_nan() { 0.0 } else { speed.abs() };
    let mag = if mag > 1.0 { 1.0 } else { mag };
    let duty = (mag * top as f32) as u16;
    (forward, duty)
}
