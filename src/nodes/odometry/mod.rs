use defmt::{info, warn};
use embassy_ros::{Node, Publisher, Qos, Topic};
use embassy_time::{Duration, Instant, Timer};
use tenshi_no_hana::odometry::OpenLoopOdometry;

use crate::{nodes::motor::state::read_motion_state_with_timeout, ros_msgs};

const FRAME_ODOMETRY: &str = "odom";
const FRAME_BASE_LINK: &str = "base_link";
const PUBLISH_PERIOD_MS: u64 = 100;
static ODOM: Topic<ros_msgs::nav_msgs::Odometry> = Topic::new("/odom", Qos::sensor_data());
static TF: Topic<ros_msgs::tf2_msgs::TFMessage> = Topic::new("/tf", Qos::sensor_data());

#[embassy_ros::node(name = "odom")]
pub(crate) async fn node(
    node: Node,
    _resources: (),
    #[publisher(topic = &ODOM)] publisher: Publisher<ros_msgs::nav_msgs::Odometry>,
    #[publisher(topic = &TF)] tf_publisher: Publisher<ros_msgs::tf2_msgs::TFMessage>,
) {
    let context = node.context();
    use ros_msgs::geometry_msgs::TransformStamped;
    use ros_msgs::nav_msgs::Odometry;
    use ros_msgs::std_msgs::Header;
    use ros_msgs::tf2_msgs::TFMessage;

    let mut header = Header::default();
    header.frame_id.push_str(FRAME_ODOMETRY).unwrap();
    let mut odometry_message = Odometry {
        header: header.clone(),
        ..Default::default()
    };
    odometry_message
        .child_frame_id
        .push_str(FRAME_BASE_LINK)
        .unwrap();
    odometry_message.pose.pose.orientation.w = 1.0;

    let mut transform = TransformStamped {
        header,
        ..Default::default()
    };
    transform.child_frame_id.push_str(FRAME_BASE_LINK).unwrap();
    transform.transform.rotation.w = 1.0;

    info!("[odom] task start");
    let mut estimator = OpenLoopOdometry::new();
    let mut last_tick = Instant::now();
    loop {
        let now = Instant::now();
        let dt = (now - last_tick).as_micros() as f32 / 1_000_000.0;
        last_tick = now;
        let (linear, angular) = read_motion_state_with_timeout(now);
        let pose = estimator.update(linear, angular, dt);
        let stamp = context.clock().now();
        let half_yaw = 0.5 * pose.yaw;
        let yaw_sin = libm::sinf(half_yaw);
        let yaw_cos = libm::cosf(half_yaw);
        fill_odometry(
            &mut odometry_message,
            pose,
            linear,
            angular,
            yaw_sin,
            yaw_cos,
        );
        fill_transform(&mut transform, pose, yaw_sin, yaw_cos);
        odometry_message.header.set_stamp(stamp);
        transform.header.set_stamp(stamp);

        if let Err(error) = publisher.publish(&odometry_message) {
            warn!("[odom] /odom publish failed: {}", error);
        }
        let mut tf_message = TFMessage::default();
        if tf_message.transforms.push(transform.clone()).is_err() {
            warn!("[odom] /tf message capacity exceeded");
        } else if let Err(error) = tf_publisher.publish(&tf_message) {
            warn!("[odom] /tf publish failed: {}", error);
        }
        Timer::after(Duration::from_millis(PUBLISH_PERIOD_MS)).await;
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_odometry(
    message: &mut ros_msgs::nav_msgs::Odometry,
    pose: tenshi_no_hana::odometry::Pose2d,
    linear: f32,
    angular: f32,
    yaw_sin: f32,
    yaw_cos: f32,
) {
    message.pose.pose.position.x = pose.x as f64;
    message.pose.pose.position.y = pose.y as f64;
    message.pose.pose.position.z = 0.0;
    message.pose.pose.orientation.x = 0.0;
    message.pose.pose.orientation.y = 0.0;
    message.pose.pose.orientation.z = yaw_sin as f64;
    message.pose.pose.orientation.w = yaw_cos as f64;
    message.twist.twist.linear.x = linear as f64;
    message.twist.twist.angular.z = angular as f64;
    message.pose.covariance[0] = 0.04;
    message.pose.covariance[7] = 0.04;
    message.pose.covariance[35] = 0.20;
    message.twist.covariance[0] = 0.09;
    message.twist.covariance[35] = 0.30;
}

fn fill_transform(
    transform: &mut ros_msgs::geometry_msgs::TransformStamped,
    pose: tenshi_no_hana::odometry::Pose2d,
    yaw_sin: f32,
    yaw_cos: f32,
) {
    transform.transform.translation.x = pose.x as f64;
    transform.transform.translation.y = pose.y as f64;
    transform.transform.translation.z = 0.0;
    transform.transform.rotation.x = 0.0;
    transform.transform.rotation.y = 0.0;
    transform.transform.rotation.z = yaw_sin as f64;
    transform.transform.rotation.w = yaw_cos as f64;
}
