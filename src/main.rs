#![no_std]
#![no_main]

mod board;
mod network;
mod nodes;
mod wifi_config;

use cortex_m_rt::exception;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_ros::{ContextStorage, InitOptions, NodeSpawner};
use embassy_time::{Duration, Timer};
use panic_probe as _;

static ROS: ContextStorage = ContextStorage::new();

#[exception]
unsafe fn HardFault(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    defmt::info!("[main] rover start");

    let board = board::init(peripherals);
    let context = defmt::unwrap!(embassy_ros::init(&ROS, InitOptions::default()));
    let nodes = NodeSpawner::new(spawner, context);
    defmt::unwrap!(nodes.register(nodes::motor::node, board.motors));
    defmt::unwrap!(nodes.register(nodes::imu::node, board.imu));
    defmt::unwrap!(nodes.register(nodes::range::node, board.range));
    defmt::unwrap!(nodes.register(nodes::alcohol::node, board.alcohol));
    let context = defmt::unwrap!(nodes.finish());
    spawner
        .spawn(network::run(spawner, context, board.wifi, wifi_config::AppConfig::new()).unwrap());
}
