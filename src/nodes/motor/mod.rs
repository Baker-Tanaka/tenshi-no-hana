use defmt::{info, warn};
use embassy_executor::task;
use embassy_time::{with_timeout, Duration};
use micro_xrce_dds_rs::{subscription_slot, Session};

use crate::ros_msgs;
use state::{publish_motion_state, CMD_VEL_TIMEOUT_MS};
use tenshi_no_hana::{
    diff_drive::{twist_to_wheels, DiffDriveConfig},
    drivers::drv8835::Drv8835,
};

const TOPIC: &str = "/cmd_vel";
const MAX_MOTOR_DUTY_SCALE: f32 = 0.6;
const WHEEL_SEPARATION_M: f32 = 0.13;
const MAX_WHEEL_SPEED_MPS: f32 = 0.4;

subscription_slot!(static SUB_CMD_VEL: ros_msgs::geometry_msgs::Twist);

pub(crate) type Motors = Drv8835<'static>;

#[task]
pub(crate) async fn run(session: Session, mut motors: Motors) -> ! {
    info!("[motor] task start");
    let node = defmt::unwrap!(session.create_node("motor").await);
    defmt::unwrap!(node.create_subscription(TOPIC, &SUB_CMD_VEL).await);
    info!("[motor] subscribed /cmd_vel");

    let config = DiffDriveConfig::new(WHEEL_SEPARATION_M, MAX_WHEEL_SPEED_MPS);
    let mut stopped = true;
    motors.stop();
    publish_motion_state(0.0, 0.0);

    loop {
        match with_timeout(
            Duration::from_millis(CMD_VEL_TIMEOUT_MS),
            SUB_CMD_VEL.recv(),
        )
        .await
        {
            Ok(twist) => {
                let linear_x = twist.linear.x as f32;
                let angular_z = twist.angular.z as f32;
                if linear_x == 0.0 && angular_z == 0.0 {
                    motors.stop();
                    publish_motion_state(0.0, 0.0);
                    if !stopped {
                        info!("[/cmd_vel] zero command, stop");
                    }
                    stopped = true;
                    continue;
                }

                let (left, right) = twist_to_wheels(&config, linear_x, angular_z);
                let left_limited = left * MAX_MOTOR_DUTY_SCALE;
                let right_limited = right * MAX_MOTOR_DUTY_SCALE;
                motors.set_left(left_limited);
                motors.set_right(right_limited);

                let left_speed = left_limited * MAX_WHEEL_SPEED_MPS;
                let right_speed = right_limited * MAX_WHEEL_SPEED_MPS;
                let realized_linear = 0.5 * (left_speed + right_speed);
                let realized_angular = (right_speed - left_speed) / WHEEL_SEPARATION_M;
                publish_motion_state(realized_linear, realized_angular);

                stopped = false;
                info!(
                    "[/cmd_vel] lx={} az={} -> l={} r={} (limited l={} r={})",
                    linear_x, angular_z, left, right, left_limited, right_limited
                );
            }
            Err(_) if !stopped => {
                motors.stop();
                publish_motion_state(0.0, 0.0);
                stopped = true;
                warn!("[motor] cmd_vel timeout, stop");
            }
            Err(_) => {}
        }
    }
}
pub(crate) mod state;
