use embassy_executor::Spawner;

use crate::{
    board::Board,
    network,
    nodes::{imu, motor, odometry, range, time_sync},
    wifi_config::AppConfig,
};

pub(crate) async fn start(spawner: Spawner, board: Board) {
    let config = AppConfig::new();
    let session = network::start(spawner, board.wifi, &config).await;

    let (imu_device, imu_address) = imu::initialize(board.imu).await;
    time_sync::synchronize(session).await;

    spawner.spawn(imu::run(session, imu_device, imu_address).unwrap());
    spawner.spawn(range::run(session, board.range).unwrap());
    spawner.spawn(motor::run(session, board.motors).unwrap());
    spawner.spawn(odometry::run(session).unwrap());
    spawner.spawn(time_sync::run(session).unwrap());
}
