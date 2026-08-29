//! Open-loop differential-drive odometry.
//!
//! The current board has no wheel encoder inputs, so this estimator integrates
//! the velocity actually requested from the motor driver. It is useful for
//! short-range operation, but it must be replaced or fused with measured wheel
//! motion before precision navigation is attempted.

const PI: f32 = core::f32::consts::PI;
const TAU: f32 = 2.0 * PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose2d {
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpenLoopOdometry {
    pose: Pose2d,
}

impl OpenLoopOdometry {
    pub const fn new() -> Self {
        Self {
            pose: Pose2d {
                x: 0.0,
                y: 0.0,
                yaw: 0.0,
            },
        }
    }

    pub const fn pose(&self) -> Pose2d {
        self.pose
    }

    pub fn reset(&mut self) {
        self.pose = Pose2d::default();
    }

    pub fn update(&mut self, linear_x: f32, angular_z: f32, dt_seconds: f32) -> Pose2d {
        if !linear_x.is_finite() || !angular_z.is_finite() || !dt_seconds.is_finite() {
            return self.pose;
        }

        let dt = if dt_seconds < 0.0 { 0.0 } else { dt_seconds };
        self.pose.x += linear_x * dt * libm::cosf(self.pose.yaw);
        self.pose.y += linear_x * dt * libm::sinf(self.pose.yaw);
        self.pose.yaw = wrap_pi(self.pose.yaw + angular_z * dt);
        self.pose
    }
}

pub const fn wrap_pi(mut angle: f32) -> f32 {
    while angle > PI {
        angle -= TAU;
    }
    while angle < -PI {
        angle += TAU;
    }
    angle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }

    #[test]
    fn integrates_straight_motion() {
        let mut odom = OpenLoopOdometry::new();
        odom.update(0.5, 0.0, 2.0);
        let pose = odom.pose();
        approx(pose.x, 1.0);
        approx(pose.y, 0.0);
        approx(pose.yaw, 0.0);
    }

    #[test]
    fn integrates_rotation_and_wraps_yaw() {
        let mut odom = OpenLoopOdometry::new();
        odom.update(0.0, 4.0, 1.0);
        approx(odom.pose().yaw, 4.0 - TAU);
    }

    #[test]
    fn rejects_non_finite_input_and_negative_dt() {
        let mut odom = OpenLoopOdometry::new();
        odom.update(f32::NAN, 1.0, 1.0);
        odom.update(1.0, 0.0, -1.0);
        assert_eq!(odom.pose(), Pose2d::default());
    }
}
