#![no_std]
#![no_main]

mod board;
mod launch;
mod network;
mod nodes;
mod ros_msgs;
mod wifi_config;

use cortex_m_rt::exception;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use panic_probe as _;

#[exception]
unsafe fn HardFault(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    defmt::info!("[main] rover start");

    launch::start(spawner, board::init(peripherals)).await;
}
