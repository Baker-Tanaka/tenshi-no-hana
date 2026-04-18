// i2c_scan.rs - I2Cバススキャナー
//
// I2Cバス上のデバイスをスキャンし、接続されているデバイスの
// アドレスを一覧表示します。デバッグ・動作確認に便利です。
//
// ピン配置:
//   - GPIO4: I2C0 SDA（データ）
//   - GPIO5: I2C0 SCL（クロック）
//
// 実行方法: cargo run --example i2c_scan

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;

use embedded_hal::i2c::I2c;

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

#[rp2040_hal::entry]
fn main() -> ! {
    info!("I2C scan start!");

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

    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // I2C0の初期化（400kHz）
    // I2Cピンはプルアップが必要
    let sda_pin = pins.gpio4.into_function::<hal::gpio::FunctionI2C>();
    let scl_pin = pins.gpio5.into_function::<hal::gpio::FunctionI2C>();
    let sda_pin = sda_pin.into_pull_type::<hal::gpio::PullUp>();
    let scl_pin = scl_pin.into_pull_type::<hal::gpio::PullUp>();

    let mut i2c = hal::I2C::i2c0(
        pac.I2C0,
        sda_pin,
        scl_pin,
        fugit::RateExtU32::kHz(400),
        &mut pac.RESETS,
        &clocks.system_clock,
    );

    info!("Scanning I2C bus...");
    info!("     0  1  2  3  4  5  6  7  8  9  A  B  C  D  E  F");

    let mut device_count = 0u32;

    // 7ビットアドレス空間（0x00-0x7F）をスキャン
    // 0x00-0x07 と 0x78-0x7F は予約アドレス
    for addr in 0x08u8..0x78u8 {
        if addr % 16 == 0 {
            info!("{:02x}:", addr);
        }

        // アドレスに対して空の読み取りを試みる
        let mut buf = [0u8; 1];
        if i2c.read(addr, &mut buf).is_ok() {
            info!("  Found device at address: 0x{:02x}", addr);
            device_count += 1;
        }
    }

    info!("Scan complete! Found {} device(s)", device_count);

    loop {
        cortex_m::asm::wfi();
    }
}
