// diff_drive.rs — Differential-drive kinematics for a 2WD rover.
//
// Converts a ROS2 geometry_msgs/Twist (linear.x [m/s], angular.z [rad/s])
// into normalised left/right wheel commands in [-1.0, 1.0]. Higher layers
// feed those into a motor driver (e.g. `Drv8835::set_left/set_right`).
//
// `linear.y`, `linear.z`, `angular.x`, `angular.y` are ignored — a 2-wheel
// differential-drive base cannot satisfy them.

/// Static parameters of the differential-drive base.
pub struct DiffDriveConfig {
    /// Distance between the two driven wheel contact points [m].
    pub wheel_separation: f32,
    /// Maximum achievable forward wheel speed at PWM = 100 % [m/s].
    /// Used to scale wheel velocities into the [-1.0, 1.0] PWM range.
    /// Set this to a hand-calibrated value; without encoders it is open-loop.
    pub max_wheel_speed: f32,
}

impl DiffDriveConfig {
    pub const fn new(wheel_separation: f32, max_wheel_speed: f32) -> Self {
        Self {
            wheel_separation,
            max_wheel_speed,
        }
    }
}

/// Convert (linear.x, angular.z) into normalised (left, right) wheel commands.
/// Each wheel command is clamped to [-1.0, 1.0]; saturation is per-wheel and
/// does NOT preserve the linear/angular ratio (acceptable for a slow rover).
pub fn twist_to_wheels(cfg: &DiffDriveConfig, linear_x: f32, angular_z: f32) -> (f32, f32) {
    let half_l = cfg.wheel_separation * 0.5;
    let v_l = linear_x - angular_z * half_l;
    let v_r = linear_x + angular_z * half_l;
    let inv_max = 1.0 / cfg.max_wheel_speed;
    (clamp_unit(v_l * inv_max), clamp_unit(v_r * inv_max))
}

fn clamp_unit(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else if x > 1.0 {
        1.0
    } else if x < -1.0 {
        -1.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "{} ≉ {}", a, b);
    }

    #[test]
    fn straight_forward() {
        let cfg = DiffDriveConfig::new(0.13, 0.4);
        let (l, r) = twist_to_wheels(&cfg, 0.2, 0.0);
        approx(l, 0.5);
        approx(r, 0.5);
    }

    #[test]
    fn pure_rotation() {
        let cfg = DiffDriveConfig::new(0.2, 1.0);
        // angular.z = 1 rad/s, half_L = 0.1 → ±0.1 m/s wheel speeds
        let (l, r) = twist_to_wheels(&cfg, 0.0, 1.0);
        approx(l, -0.1);
        approx(r, 0.1);
    }

    #[test]
    fn saturation() {
        let cfg = DiffDriveConfig::new(0.13, 0.4);
        let (l, r) = twist_to_wheels(&cfg, 10.0, 0.0);
        approx(l, 1.0);
        approx(r, 1.0);
        let (l, r) = twist_to_wheels(&cfg, -10.0, 0.0);
        approx(l, -1.0);
        approx(r, -1.0);
    }
}
