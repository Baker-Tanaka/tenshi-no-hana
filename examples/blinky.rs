// blinky.rs - 最もシンプルなLED点滅の例
//
// 組み込みRustの基本を学ぶための最小限の例です。
// GPIO22に接続されたLEDを1秒間隔で点滅させます。
//
// 実行方法: cargo run --example blinky

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

#[rp2040_hal::entry]
fn main() -> ! {
    info!("Blinky start!");

    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);

    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut timer = rp2040_hal::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);

    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let mut led = pins.gpio22.into_push_pull_output();

    loop {
        info!("LED ON");
        led.set_high().unwrap();
        timer.delay_ms(1000);

        info!("LED OFF");
        led.set_low().unwrap();
        timer.delay_ms(1000);
    }
}
