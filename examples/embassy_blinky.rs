// embassy_blinky.rs - Embassyを使った非同期LED点滅
//
// Embassy非同期フレームワークを使ったLED点滅の基本例です。
// async/awaitにより、ブロッキングなしでタイマーを待機できます。
//
// 実行方法: cargo run --no-default-features --features embassy --example embassy_blinky

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_time::Timer;
use panic_probe as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Embassy blinky start!");

    let p = embassy_rp::init(Default::default());

    // GPIO22をプッシュプル出力に設定
    let mut led = Output::new(p.PIN_22, Level::Low);

    loop {
        info!("LED ON");
        led.set_high();
        Timer::after_millis(1000).await;

        info!("LED OFF");
        led.set_low();
        Timer::after_millis(1000).await;
    }
}
