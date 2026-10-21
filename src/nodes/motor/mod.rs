use defmt::{info, warn};
use embassy_futures::select::{select3, Either3};
use embassy_ros::{Node, Qos, Subscription, Topic};
use embassy_time::{Duration, Instant, Timer};

use crate::ros_msgs;
use state::{publish_motion_state, CMD_VEL_TIMEOUT_MS};
use tenshi_no_hana::{
    diff_drive::{forward_only_twist_to_wheels, DiffDriveConfig},
    drivers::drv8835::Drv8835,
};

const MAX_MOTOR_DUTY_SCALE: f32 = 0.6;
const WHEEL_SEPARATION_M: f32 = 0.13;
const MAX_WHEEL_SPEED_MPS: f32 = 0.4;

static CMD_VEL: Topic<ros_msgs::geometry_msgs::Twist> = Topic::new(
    "/cmd_vel",
    Qos::command(Duration::from_millis(CMD_VEL_TIMEOUT_MS)),
);

type Motors = Drv8835<'static>;

#[embassy_ros::node(name = "motor")]
pub(crate) async fn node(
    node: Node,
    mut motors: Motors,
    #[subscription(topic = &CMD_VEL)] mut commands: Subscription<ros_msgs::geometry_msgs::Twist>,
) {
    let context = node.context();
    info!("[motor] task start (forward-only; reverse commands stop both wheels)");
    let config = DiffDriveConfig::new(WHEEL_SEPARATION_M, MAX_WHEEL_SPEED_MPS);
    let mut connection = defmt::unwrap!(context.connection().watch());
    let mut epoch = connection.current().await.epoch;
    let mut last_command = Instant::now();
    let mut stopped = true;
    stop(&mut motors);

    loop {
        let deadline = last_command + Duration::from_millis(CMD_VEL_TIMEOUT_MS);
        match select3(
            commands.recv_sample(),
            connection.changed(),
            Timer::at(deadline),
        )
        .await
        {
            Either3::First(Ok(sample)) => {
                let status = context.connection().status();
                if epoch != status.epoch || !status.is_ready() {
                    epoch = status.epoch;
                    stop(&mut motors);
                    stopped = true;
                    warn!("[motor] connection epoch changed, stop");
                    continue;
                }
                let twist = sample.message;
                let linear = twist.linear.x as f32;
                let angular = twist.angular.z as f32;
                if !linear.is_finite() || !angular.is_finite() {
                    warn!("[motor] non-finite command, stop");
                    stop(&mut motors);
                    stopped = true;
                    continue;
                }
                last_command = sample.received_at;
                if linear == 0.0 && angular == 0.0 {
                    stop(&mut motors);
                    stopped = true;
                    continue;
                }
                let Some((left, right)) = forward_only_twist_to_wheels(&config, linear, angular) else {
                    warn!("[motor] reverse wheel requested in forward-only mode, stop");
                    stop(&mut motors);
                    stopped = true;
                    continue;
                };
                let left_limited = forward_only(left * MAX_MOTOR_DUTY_SCALE);
                let right_limited = forward_only(right * MAX_MOTOR_DUTY_SCALE);
                if left_limited == 0.0 && right_limited == 0.0 {
                    stop(&mut motors);
                    stopped = true;
                    continue;
                }
                motors.set_left(left_limited);
                motors.set_right(right_limited);
                let left_speed = left_limited * MAX_WHEEL_SPEED_MPS;
                let right_speed = right_limited * MAX_WHEEL_SPEED_MPS;
                publish_motion_state(
                    0.5 * (left_speed + right_speed),
                    (right_speed - left_speed) / WHEEL_SEPARATION_M,
                );
                stopped = false;
                info!(
                    "[/cmd_vel] lx={} az={} -> fwd-only l={} r={}",
                    linear, angular, left_limited, right_limited
                );
            }
            Either3::First(Err(error)) => {
                warn!("[motor] command rejected: {}, stop", error);
                stop(&mut motors);
                stopped = true;
            }
            Either3::Second(status) => {
                if epoch != status.epoch || !status.is_ready() {
                    stop(&mut motors);
                    stopped = true;
                    warn!("[motor] connection changed, stop");
                }
                epoch = status.epoch;
            }
            Either3::Third(()) => {
                if !stopped {
                    stop(&mut motors);
                    stopped = true;
                    warn!("[motor] cmd_vel timeout, stop");
                }
                // Keep the idle watchdog yielding rather than polling an expired timer.
                last_command = Instant::now();
            }
        }
    }
}

fn stop(motors: &mut Motors) {
    motors.stop();
    publish_motion_state(0.0, 0.0);
}

fn forward_only(value: f32) -> f32 {
    value.max(0.0)
}

pub(crate) mod state;
