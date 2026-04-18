// embassy_button.rs - Embassyを使った非同期ボタン入力
//
// Embassyの非同期GPIO割り込みを使ってボタン入力を処理する例です。
// ポーリングの代わりにasync/awaitでエッジ検出を待機するため、
// CPU使用率を最小限に抑えられます。
//
// ピン配置:
//   - GPIO22: Green LED
//   - GPIO20: Red LED
//   - GPIO23: Button（プルアップ、押すとLOW）
//
// 実行方法: cargo run --no-default-features --features embassy --example embassy_button

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_time::Timer;
use panic_probe as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Embassy button example start!");

    let p = embassy_rp::init(Default::default());

    let mut green_led = Output::new(p.PIN_22, Level::Low);
    let mut red_led = Output::new(p.PIN_20, Level::High);
    let mut button = Input::new(p.PIN_23, Pull::Up);

    info!("Waiting for button press...");

    loop {
        // ボタンが押される（立ち下がりエッジ）まで非同期で待機
        button.wait_for_falling_edge().await;
        info!("Button pressed!");

        // 赤LED OFF → 緑LED ON
        red_led.set_low();
        green_led.set_high();
        Timer::after_millis(2000).await;

        // 緑LED OFF
        green_led.set_low();

        // 赤LED ON に戻す
        red_led.set_high();

        // デバウンス用の短い待機
        Timer::after_millis(200).await;

        info!("Ready for next press");
    }
}
