use defmt::{info, warn};
use embassy_executor::{task, Spawner};
use embassy_ros::{Context, Error, Publisher, Qos, Topic};
use embassy_time::{Duration, Timer};
use tenshi_no_hana::drivers::hcsr04;

use crate::ros_msgs;

const FRAME_ID: &str = "ultrasonic_link";
const PERIOD_MS: u64 = 100;

static FRONT: Topic<ros_msgs::sensor_msgs::Range, 2> =
    Topic::new("/ultrasonic/front", Qos::sensor_data());

type Sensor = hcsr04::Hcsr04<'static>;

pub(crate) fn start(spawner: Spawner, context: Context, sensor: Sensor) -> Result<(), Error> {
    let node = context.create_node("range")?;
    let publisher = node.create_publisher(&FRONT)?;
    context.export(&FRONT)?;
    spawner.spawn(run(context, publisher, sensor).unwrap());
    Ok(())
}

#[task]
async fn run(
    context: Context,
    publisher: Publisher<ros_msgs::sensor_msgs::Range>,
    mut sensor: Sensor,
) -> ! {
    info!("[range] task start");
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
        message.header.set_stamp(context.clock().now());
        if let Err(error) = publisher.publish(&message) {
            warn!("[range] publish failed: {}", error);
        }
        Timer::after(Duration::from_millis(PERIOD_MS)).await;
    }
}
