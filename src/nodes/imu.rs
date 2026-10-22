use defmt::{info, warn};
use embassy_ros::{Node, Publisher, Qos, Topic};
use embassy_time::{Duration, Timer};

use crate::board::Imu;
use geometry_msgs::msg::{Quaternion, Vector3};
use sensor_msgs::msg::Imu as ImuMessage;
use tenshi_no_hana::drivers::icm20602;

const FRAME_ID: &str = "imu_link";
const PERIOD_MS: u64 = 20;
const SENSOR_RETRY_SECS: u64 = 1;
const SENSOR_FAILURE_LIMIT: u32 = 25;

static DATA: Topic<ImuMessage, 2> = Topic::new("/imu/data", Qos::sensor_data());

#[embassy_ros::node(name = "imu")]
pub(crate) async fn node(
    node: Node,
    mut device: Imu,
    #[publisher(topic = &DATA)] publisher: Publisher<ImuMessage>,
) {
    let context = node.context();
    info!("[imu] task start");
    let mut address = None;
    let mut read_failures = 0u32;

    loop {
        if address.is_none() {
            match icm20602::init(&mut device).await {
                Ok(detected) => {
                    address = Some(detected);
                    read_failures = 0;
                    info!("[imu] detected at 0x{:02x}", detected);
                }
                Err(()) => {
                    warn!("[imu] detection failed; retrying");
                    Timer::after(Duration::from_secs(SENSOR_RETRY_SECS)).await;
                    continue;
                }
            }
        }

        if let Ok(reading) = icm20602::read(&mut device, address.unwrap()) {
            if read_failures != 0 {
                info!("[imu] reading recovered");
                read_failures = 0;
            }
            let message = to_message(reading, context.clock().now());
            if let Err(error) = publisher.publish(&message) {
                warn!("[imu] publish failed: {}", error);
            }
        } else {
            read_failures = read_failures.saturating_add(1);
            if read_failures == 1 || read_failures.is_multiple_of(100) {
                warn!("[imu] read failed, count={}", read_failures);
            }
            if read_failures >= SENSOR_FAILURE_LIMIT {
                address = None;
                warn!("[imu] sensor lost; restarting detection");
            }
        }
        Timer::after(Duration::from_millis(PERIOD_MS)).await;
    }
}

fn to_message(reading: icm20602::Reading, stamp: embassy_ros::Timestamp) -> ImuMessage {
    let [ax, ay, az] = reading.acceleration;
    let [gx, gy, gz] = reading.angular_velocity;
    let mut orientation_covariance = [0.0f64; 9];
    orientation_covariance[0] = -1.0;

    let mut message = ImuMessage {
        orientation: Quaternion {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        orientation_covariance,
        angular_velocity: Vector3 {
            x: gx,
            y: gy,
            z: gz,
        },
        angular_velocity_covariance: [0.0; 9],
        linear_acceleration: Vector3 {
            x: ax,
            y: ay,
            z: az,
        },
        linear_acceleration_covariance: [0.0; 9],
        ..Default::default()
    };
    message.header.set_stamp(stamp);
    message.header.frame_id.push_str(FRAME_ID).unwrap();
    message
}
