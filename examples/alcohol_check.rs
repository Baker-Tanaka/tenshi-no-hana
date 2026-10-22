#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    adc::{Adc, Channel, Config},
    gpio::Pull,
};
use embassy_time::{Duration, Timer};
use panic_probe as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    let mut adc = Adc::new_blocking(peripherals.ADC, Config::default());
    let mut channel = Channel::new_pin(peripherals.PIN_28, Pull::None);

    info!("[mq3] GP28 ADC test start");

    loop {
        match adc.blocking_read(&mut channel) {
            Ok(raw) => {
                let normalized = raw as f32 / 4095.0;
                let millivolts = raw as u32 * 3300 / 4095;
                info!(
                    "[mq3] raw={} normalized={} approx_mV={}",
                    raw, normalized, millivolts
                );
            }
            Err(_) => info!("[mq3] ADC conversion failed"),
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}
