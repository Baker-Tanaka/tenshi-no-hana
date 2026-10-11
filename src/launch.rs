use embassy_executor::Spawner;
use embassy_ros::{Error, NodeSpawner};

use crate::{
    board::Board,
    network,
    nodes::{imu, motor, odometry, range},
    wifi_config::AppConfig,
};

pub(crate) fn register(
    nodes: &NodeSpawner,
    board: Board,
    network_spawner: Spawner,
) -> Result<(), Error> {
    nodes.register(motor::node, board.motors)?;
    nodes.register(imu::node, board.imu)?;
    nodes.register(range::node, board.range)?;
    nodes.register(odometry::node, ())?;
    network_spawner.spawn(
        network::run(
            network_spawner,
            nodes.context(),
            board.wifi,
            AppConfig::new(),
        )
        .map_err(|_| Error::TaskBusy)?,
    );
    Ok(())
}
