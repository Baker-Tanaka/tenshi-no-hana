use defmt::{info, warn};
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use micro_xrce_dds_rs::{Node, Session};

use crate::{board::Imu, ros_msgs};
use tenshi_no_hana::drivers::icm20602;

const TOPIC: &str = "/imu/data";
const FRAME_ID: &str = "imu_link";
const PERIOD_MS: u64 = 20;
const SENSOR_RETRY_SECS: u64 = 1;
const SENSOR_FAILURE_LIMIT: u32 = 25;

pub(crate) async fn initialize(mut device: Imu) -> (Imu, Option<u8>) {
    let address = detect_sensor(&mut device).await;
    (device, address)
}

async fn detect_sensor(device: &mut Imu) -> Option<u8> {
    match icm20602::init(device).await {
        Ok(address) => {
            info!("[imu] ICM-20602 ready at 0x{:02x}", address);
            Some(address)
        }
        Err(()) => {
            defmt::error!("[imu] ICM-20602 not found at 0x68 or 0x69");
            None
        }
    }
}

#[task]
pub(crate) async fn run(session: Session, mut device: Imu, mut address: Option<u8>) -> ! {
    info!("[imu] task start");
    let node = defmt::unwrap!(session.create_node("imu").await);
    let publisher = create_publisher(&node).await;
    let mut read_failures = 0u32;
    let mut publish_failures = 0u32;
    let mut published = 0u32;

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

            let message = to_message(reading, session.clock().now());
            match publisher.publish(&message).await {
                Ok(()) => {
                    published = published.saturating_add(1);
                    if publish_failures != 0 {
                        info!("[imu] publishing recovered");
                        publish_failures = 0;
                    }
                    if published == 1 {
                        info!("[imu] first sample published");
                    }
                }
                Err(error) => {
                    publish_failures = publish_failures.saturating_add(1);
                    if publish_failures == 1 || publish_failures.is_multiple_of(100) {
                        warn!(
                            "[imu] publish failed: {}, count={}",
                            error, publish_failures
                        );
                    }
                }
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

async fn create_publisher(node: &Node) -> micro_xrce_dds_rs::Publisher<ros_msgs::sensor_msgs::Imu> {
    let publisher = defmt::unwrap!(
        node.create_publisher::<ros_msgs::sensor_msgs::Imu>(TOPIC)
            .await
    );
    info!("[imu] publisher ready");
    publisher
}

fn to_message(
    reading: icm20602::Reading,
    stamp: micro_xrce_dds_rs::Timestamp,
) -> ros_msgs::sensor_msgs::Imu {
    let [ax, ay, az] = reading.acceleration;
    let [gx, gy, gz] = reading.angular_velocity;
    let mut orientation_covariance = [0.0f64; 9];
    orientation_covariance[0] = -1.0;

    let mut message = ros_msgs::sensor_msgs::Imu {
        orientation: ros_msgs::geometry_msgs::Quaternion {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        orientation_covariance,
        angular_velocity: ros_msgs::geometry_msgs::Vector3 {
            x: gx,
            y: gy,
            z: gz,
        },
        angular_velocity_covariance: [0.0; 9],
        linear_acceleration: ros_msgs::geometry_msgs::Vector3 {
            x: ax,
            y: ay,
            z: az,
        },
        linear_acceleration_covariance: [0.0; 9],
        ..Default::default()
    };
    message.header.stamp.sec = stamp.seconds;
    message.header.stamp.nanosec = stamp.nanoseconds;
    message.header.frame_id.push_str(FRAME_ID).unwrap();
    message
}
