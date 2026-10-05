use defmt::{info, warn};
use embassy_executor::task;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use micro_xrce_dds_rs::{Node, Session};
use tenshi_no_hana::odometry::OpenLoopOdometry;

use crate::{nodes::motor::state::read_motion_state_with_timeout, ros_msgs};

const TOPIC_ODOMETRY: &str = "/odom";
const TOPIC_TF: &str = "/tf";
const FRAME_ODOMETRY: &str = "odom";
const FRAME_BASE_LINK: &str = "base_link";
const PUBLISH_PERIOD_MS: u64 = 100;
const ENTITY_START_DELAY_SECS: u64 = 5;
const PUBLISH_TIMEOUT_MS: u64 = 250;

#[task]
pub(crate) async fn run(session: Session) -> ! {
    use ros_msgs::geometry_msgs::TransformStamped;
    use ros_msgs::nav_msgs::Odometry;
    use ros_msgs::std_msgs::Header;
    use ros_msgs::tf2_msgs::TFMessage;

    info!("[odom] task start");
    // Let the sensor nodes create their entities before the larger odometry types.
    Timer::after(Duration::from_secs(ENTITY_START_DELAY_SECS)).await;
    let node = defmt::unwrap!(session.create_node("odom").await);
    let (publisher, tf_publisher) = create_publishers(&node).await;

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

    let mut estimator = OpenLoopOdometry::new();
    let mut last_tick = Instant::now();
    let mut iteration = 0u32;
    loop {
        iteration += 1;
        let now = Instant::now();
        let dt = (now - last_tick).as_micros() as f32 / 1_000_000.0;
        last_tick = now;

        let (linear, angular) = read_motion_state_with_timeout(now);
        let pose = estimator.update(linear, angular, dt);
        let half_yaw = 0.5 * pose.yaw;
        let yaw_sin = libm::sinf(half_yaw);
        let yaw_cos = libm::cosf(half_yaw);
        let stamp = session.clock().now();
        let (sec, nanosec) = (stamp.seconds, stamp.nanoseconds);

        fill_odometry(
            &mut odometry_message,
            pose,
            linear,
            angular,
            yaw_sin,
            yaw_cos,
            sec,
            nanosec,
        );
        if with_timeout(
            Duration::from_millis(PUBLISH_TIMEOUT_MS),
            publisher.publish(&odometry_message),
        )
        .await
        .map(|result| result.is_err())
        .unwrap_or(true)
        {
            warn!("[odom] /odom publish failed iter={}", iteration);
        } else if iteration <= 3 || iteration.is_multiple_of(10) {
            info!("[odom] /odom published iter={}", iteration);
        }

        fill_transform(&mut transform, pose, yaw_sin, yaw_cos, sec, nanosec);
        let mut tf_message = TFMessage::default();
        if tf_message.transforms.push(transform.clone()).is_err() {
            warn!("[odom] /tf message capacity exceeded");
        } else if with_timeout(
            Duration::from_millis(PUBLISH_TIMEOUT_MS),
            tf_publisher.publish(&tf_message),
        )
        .await
        .map(|result| result.is_err())
        .unwrap_or(true)
        {
            warn!("[odom] /tf publish failed iter={}", iteration);
        }

        Timer::after(Duration::from_millis(PUBLISH_PERIOD_MS)).await;
    }
}

async fn create_publishers(
    node: &Node,
) -> (
    micro_xrce_dds_rs::Publisher<ros_msgs::nav_msgs::Odometry>,
    micro_xrce_dds_rs::Publisher<ros_msgs::tf2_msgs::TFMessage>,
) {
    let odometry = defmt::unwrap!(
        node.create_publisher::<ros_msgs::nav_msgs::Odometry>(TOPIC_ODOMETRY)
            .await
    );
    let tf = defmt::unwrap!(
        node.create_publisher::<ros_msgs::tf2_msgs::TFMessage>(TOPIC_TF)
            .await
    );
    info!("[odom] /odom and /tf publishers ready");
    (odometry, tf)
}

#[allow(clippy::too_many_arguments)]
fn fill_odometry(
    message: &mut ros_msgs::nav_msgs::Odometry,
    pose: tenshi_no_hana::odometry::Pose2d,
    linear: f32,
    angular: f32,
    yaw_sin: f32,
    yaw_cos: f32,
    sec: i32,
    nanosec: u32,
) {
    message.header.stamp.sec = sec;
    message.header.stamp.nanosec = nanosec;
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
    sec: i32,
    nanosec: u32,
) {
    transform.header.stamp.sec = sec;
    transform.header.stamp.nanosec = nanosec;
    transform.transform.translation.x = pose.x as f64;
    transform.transform.translation.y = pose.y as f64;
    transform.transform.translation.z = 0.0;
    transform.transform.rotation.x = 0.0;
    transform.transform.rotation.y = 0.0;
    transform.transform.rotation.z = yaw_sin as f64;
    transform.transform.rotation.w = yaw_cos as f64;
}
