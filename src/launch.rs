use embassy_executor::Spawner;
use embassy_ros::{ContextStorage, InitOptions};

use crate::{
    board::Board,
    network,
    nodes::{imu, motor, odometry, range},
    wifi_config::AppConfig,
};

static ROS: ContextStorage = ContextStorage::new();

pub(crate) async fn start(spawner: Spawner, board: Board) {
    let context = defmt::unwrap!(embassy_ros::init(&ROS, InitOptions::default()));
    defmt::unwrap!(motor::start(spawner, context, board.motors));
    defmt::unwrap!(imu::start(spawner, context, board.imu));
    defmt::unwrap!(range::start(spawner, context, board.range));
    defmt::unwrap!(odometry::start(spawner, context));
    spawner.spawn(network::run(spawner, context, board.wifi, AppConfig::new()).unwrap());
}
