// pwm_led.rs - PWMによるLEDの明るさ制御
//
// PWM（パルス幅変調）を使ってLEDの明るさを徐々に変化させます。
// GPIO22に接続されたLEDをフェードイン・フェードアウトさせます。
//
// 実行方法: cargo run --example pwm_led

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;

use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

#[rp2040_hal::entry]
fn main() -> ! {
    info!("PWM LED fade start!");

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

    // PWMスライスの初期化
    // GPIO22はPWMスライス3のチャンネルA
    let pwm_slices = hal::pwm::Slices::new(pac.PWM, &mut pac.RESETS);
    let mut pwm = pwm_slices.pwm3;
    pwm.set_ph_correct();
    pwm.enable();

    // GPIO22をPWM出力に設定
    let mut channel_a = pwm.channel_a;
    channel_a.output_to(pins.gpio22);

    let max_duty = channel_a.max_duty_cycle();
    info!("Max duty cycle: {}", max_duty);

    loop {
        // フェードイン
        info!("Fade in");
        for duty in (0..=max_duty).step_by(256) {
            channel_a.set_duty_cycle(duty).unwrap();
            timer.delay_ms(5);
        }

        timer.delay_ms(500);

        // フェードアウト
        info!("Fade out");
        for duty in (0..=max_duty).rev().step_by(256) {
            channel_a.set_duty_cycle(duty).unwrap();
            timer.delay_ms(5);
        }

        timer.delay_ms(500);
    }
}
