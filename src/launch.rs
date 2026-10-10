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
    let imu_data = defmt::unwrap!(imu::prepare(context));
    let front_range = defmt::unwrap!(range::prepare(context));
    let (odom, tf) = defmt::unwrap!(odometry::prepare(context));
    let commands = defmt::unwrap!(motor::prepare(context));

    defmt::unwrap!(context.export(&imu::DATA));
    defmt::unwrap!(context.export(&range::FRONT));
    defmt::unwrap!(context.export(&odometry::ODOM));
    defmt::unwrap!(context.export(&odometry::TF));
    defmt::unwrap!(context.import(&motor::CMD_VEL));

    spawner.spawn(motor::run(context, commands, board.motors).unwrap());
    spawner.spawn(imu::run(context, imu_data, board.imu).unwrap());
    spawner.spawn(range::run(context, front_range, board.range).unwrap());
    spawner.spawn(odometry::run(context, odom, tf).unwrap());
    spawner.spawn(network::run(spawner, context, board.wifi, AppConfig::new()).unwrap());
}
