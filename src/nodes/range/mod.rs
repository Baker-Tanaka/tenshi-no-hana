use defmt::{info, warn};
use embassy_executor::task;
use embassy_time::{Duration, Timer};
use micro_xrce_dds_rs::{Node, Session};

use crate::ros_msgs;
use tenshi_no_hana::drivers::hcsr04;

const TOPIC: &str = "/ultrasonic/front";
const FRAME_ID: &str = "ultrasonic_link";
const PERIOD_MS: u64 = 100;

pub(crate) type Sensor = hcsr04::Hcsr04<'static>;

#[task]
pub(crate) async fn run(session: Session, mut sensor: Sensor) -> ! {
    info!("[range] task start");
    let node = defmt::unwrap!(session.create_node("range").await);
    let publisher = create_publisher(&node).await;
    let mut message = ros_msgs::sensor_msgs::Range {
        radiation_type: 0,
        field_of_view: hcsr04::FIELD_OF_VIEW_RAD,
        min_range: hcsr04::MIN_RANGE_M,
        max_range: hcsr04::MAX_RANGE_M,
        variance: 0.01,
        ..Default::default()
    };
    message.header.frame_id.push_str(FRAME_ID).unwrap();

    loop {
        message.range = sensor.measure().await.unwrap_or(f32::INFINITY);
        let stamp = session.clock().now();
        message.header.stamp.sec = stamp.seconds;
        message.header.stamp.nanosec = stamp.nanoseconds;
        if let Err(error) = publisher.publish(&message).await {
            warn!("[range] publish failed: {}", error);
        }
        Timer::after(Duration::from_millis(PERIOD_MS)).await;
    }
}

async fn create_publisher(
    node: &Node,
) -> micro_xrce_dds_rs::Publisher<ros_msgs::sensor_msgs::Range> {
    let publisher = defmt::unwrap!(
        node.create_publisher::<ros_msgs::sensor_msgs::Range>(TOPIC)
            .await
    );
    info!("[range] publisher ready");
    publisher
}
