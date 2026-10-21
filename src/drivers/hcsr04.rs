#[cfg(feature = "embassy")]
use embassy_rp::gpio::{Input, Output};
#[cfg(feature = "embassy")]
use embassy_time::{with_timeout, Duration, Instant, Timer};

pub const MIN_RANGE_M: f32 = 0.02;
pub const MAX_RANGE_M: f32 = 4.0;
pub const FIELD_OF_VIEW_RAD: f32 = 0.26;

const SPEED_OF_SOUND_MPS: f32 = 343.0;
#[cfg(feature = "embassy")]
const ECHO_TIMEOUT: Duration = Duration::from_millis(30);

pub fn echo_duration_to_metres(duration_us: u64) -> f32 {
    duration_us as f32 * SPEED_OF_SOUND_MPS / 2_000_000.0
}

#[cfg(feature = "embassy")]
pub struct Hcsr04<'d> {
    trigger: Output<'d>,
    echo: Input<'d>,
}

#[cfg(feature = "embassy")]
impl<'d> Hcsr04<'d> {
    pub fn new(trigger: Output<'d>, echo: Input<'d>) -> Self {
        Self { trigger, echo }
    }

    pub async fn measure(&mut self) -> Option<f32> {
        self.trigger.set_low();
        Timer::after_micros(2).await;
        self.trigger.set_high();
        Timer::after_micros(10).await;
        self.trigger.set_low();

        with_timeout(ECHO_TIMEOUT, self.echo.wait_for_rising_edge())
            .await
            .ok()?;
        let started = Instant::now();
        with_timeout(ECHO_TIMEOUT, self.echo.wait_for_falling_edge())
            .await
            .ok()?;

        let distance = echo_duration_to_metres((Instant::now() - started).as_micros());
        (MIN_RANGE_M..=MAX_RANGE_M)
            .contains(&distance)
            .then_some(distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_echo_round_trip_to_metres() {
        assert_eq!(echo_duration_to_metres(0), 0.0);
        assert!((echo_duration_to_metres(1_000) - 0.1715).abs() < 1e-6);
        assert!(echo_duration_to_metres(30_000) > MAX_RANGE_M);
    }
}
