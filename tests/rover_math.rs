#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;
use tenshi_no_hana::diff_drive::{twist_to_wheels, DiffDriveConfig};
use tenshi_no_hana::hcsr04::echo_duration_to_metres;
use tenshi_no_hana::odometry::OpenLoopOdometry;

fn approx(actual: f32, expected: f32) {
    defmt::assert!((actual - expected).abs() < 1e-5);
}

#[defmt_test::tests]
mod tests {
    use super::*;

    #[test]
    fn converts_twist_to_saturated_wheel_commands() {
        let config = DiffDriveConfig::new(0.13, 0.4);
        let (left, right) = twist_to_wheels(&config, 0.2, 0.0);
        approx(left, 0.5);
        approx(right, 0.5);

        let (left, right) = twist_to_wheels(&config, 10.0, 0.0);
        approx(left, 1.0);
        approx(right, 1.0);
    }

    #[test]
    fn integrates_open_loop_odometry() {
        let mut odometry = OpenLoopOdometry::new();
        let pose = odometry.update(0.5, 0.0, 2.0);
        approx(pose.x, 1.0);
        approx(pose.y, 0.0);
        approx(pose.yaw, 0.0);
    }

    #[test]
    fn converts_echo_duration_to_distance() {
        approx(echo_duration_to_metres(1_000), 0.1715);
    }
}
