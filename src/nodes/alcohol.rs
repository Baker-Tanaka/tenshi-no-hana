use defmt::{info, warn};
use embassy_ros::{Node, Publisher, Qos, Topic};
use embassy_time::{Duration, Timer};
use std_msgs::msg::Float32;

use crate::board::AlcoholSensor;

const PERIOD_MS: u64 = 100;

static ALCOHOL: Topic<Float32, 2> = Topic::new("/alcohol", Qos::sensor_data());

#[embassy_ros::node(name = "alcohol")]
pub(crate) async fn node(
    _node: Node,
    mut sensor: AlcoholSensor,
    #[publisher(topic = &ALCOHOL)] publisher: Publisher<Float32>,
) {
    info!("[alcohol] MQ-3 ADC task start");

    loop {
        match sensor.adc.blocking_read(&mut sensor.channel) {
            Ok(raw) => {
                let message = Float32 {
                    data: raw as f32 / 4095.0,
                };
                if let Err(error) = publisher.publish(&message) {
                    warn!("[alcohol] publish failed: {}", error);
                }
            }
            Err(_) => warn!("[alcohol] ADC conversion failed"),
        }
        Timer::after(Duration::from_millis(PERIOD_MS)).await;
    }
}
