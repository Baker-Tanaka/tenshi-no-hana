// adc_read.rs - ADCによるアナログ値の読み取り
//
// ADC（アナログ-デジタル変換器）を使ってアナログセンサーの値を読み取ります。
// GPIO26（ADC0）に接続されたセンサーの値と、内蔵温度センサーの値を読み取ります。
//
// 実行方法: cargo run --example adc_read

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;

use embedded_hal::delay::DelayNs;

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

#[rp2040_hal::entry]
fn main() -> ! {
    info!("ADC read start!");

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

    // ADCの初期化
    let mut adc = hal::Adc::new(pac.ADC, &mut pac.RESETS);

    // GPIO26をADC入力（ADC0チャンネル）として設定
    let mut adc_pin_0 = hal::adc::AdcPin::new(pins.gpio26.into_floating_input()).unwrap();

    // 内蔵温度センサーを有効化
    let mut temperature_sensor = adc.take_temp_sensor().unwrap();

    loop {
        // ADC0チャンネルの値を読み取り（12ビット: 0-4095）
        let adc_value: u16 = adc.read(&mut adc_pin_0).unwrap();
        // 電圧に変換（基準電圧3.3V、12ビット分解能）
        let voltage_mv = (adc_value as u32) * 3300 / 4096;
        info!("ADC0: raw={}, voltage={} mV", adc_value, voltage_mv);

        // 内蔵温度センサーの値を読み取り
        let temp_value: u16 = adc.read(&mut temperature_sensor).unwrap();
        // 温度に変換（データシートの式に基づく: T = 27 - (V - 0.706) / 0.001721）
        // 整数演算で近似: temp_mC = 27000 - (voltage_uV - 706000) * 1000 / 1721
        let temp_voltage_uv = (temp_value as i32) * 3_300_000 / 4096;
        let temp_mc = 27_000 - (temp_voltage_uv - 706_000) * 1000 / 1721;
        info!(
            "Temperature: {}.{} deg C (raw={})",
            temp_mc / 1000,
            (temp_mc % 1000) / 100,
            temp_value
        );

        timer.delay_ms(1000);
    }
}
