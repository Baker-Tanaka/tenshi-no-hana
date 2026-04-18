// multicore.rs - デュアルコアの使用例
//
// RP2040のデュアルコア（Core0 + Core1）を活用する例です。
// Core0でGPIO22のLEDを点滅させ、Core1でGPIO20のLEDを点滅させます。
// FIFOを使ってコア間の通信も行います。
//
// 実行方法: cargo run --example multicore

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use hal::multicore::{Multicore, Stack};

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

// Core1用のスタック領域
static CORE1_STACK: Stack<4096> = Stack::new();

// Core1で実行される関数
fn core1_task() {
    // Core1のペリフェラルを取得
    let pac = unsafe { pac::Peripherals::steal() };
    let sio = hal::Sio::new(pac.SIO);

    // Core1のSIO FIFOを取得
    let mut fifo = sio.fifo;

    // FIFOからGPIO20のピン情報を受け取るまで待機
    let _ = fifo.read_blocking();
    info!("Core1: Received start signal!");

    // Core1では直接レジスタ操作でGPIO20を制御
    // （ピンの所有権はCore0側で設定済み）
    let sio_regs = unsafe { &*pac::SIO::ptr() };

    loop {
        // GPIO20をトグル
        sio_regs
            .gpio_out_xor()
            .write(|w| unsafe { w.bits(1 << 20) });
        // 簡易ディレイ
        cortex_m::asm::delay(6_000_000);

        info!("Core1: LED toggled");
    }
}

#[rp2040_hal::entry]
fn main() -> ! {
    info!("Multicore example start!");

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

    let mut sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    // Core0: GPIO22（Green LED）を制御
    let mut green_led = pins.gpio22.into_push_pull_output();

    // GPIO20（Red LED）をCore1用に出力設定
    let _red_led = pins.gpio20.into_push_pull_output();

    // Core1を起動
    let mut mc = Multicore::new(&mut pac.PSM, &mut pac.PPB, &mut sio.fifo);
    let cores = mc.cores();
    let core1 = &mut cores[1];
    core1
        .spawn(CORE1_STACK.take().unwrap(), core1_task)
        .unwrap();

    // Core1に開始シグナルを送信
    let fifo = unsafe { &(*pac::SIO::ptr()) };
    fifo.fifo_wr().write(|w| unsafe { w.bits(1) });

    info!("Core0: Core1 started!");

    // Core0: GPIO22のLEDを独自のタイミングで点滅
    loop {
        info!("Core0: Green LED ON");
        green_led.set_high().unwrap();
        timer.delay_ms(500);

        info!("Core0: Green LED OFF");
        green_led.set_low().unwrap();
        timer.delay_ms(500);
    }
}
