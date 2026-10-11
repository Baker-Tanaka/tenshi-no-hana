use embassy_executor::Spawner;
use embassy_ros::{NodeSpawner, StartupError};

use crate::{
    board::Board,
    network,
    nodes::{imu, motor, odometry, range},
    wifi_config::AppConfig,
};

pub(crate) fn register(
    nodes: &NodeSpawner,
    board: Board,
) -> Result<crate::board::Wifi, StartupError> {
    nodes.register(motor::node, board.motors)?;
    nodes.register(imu::node, board.imu)?;
    nodes.register(range::node, board.range)?;
    nodes.register(odometry::node, ())?;
    Ok(board.wifi)
}

pub(crate) fn network(spawner: Spawner, context: embassy_ros::Context, wifi: crate::board::Wifi) {
    spawner.spawn(network::run(spawner, context, wifi, AppConfig::new()).unwrap());
}
