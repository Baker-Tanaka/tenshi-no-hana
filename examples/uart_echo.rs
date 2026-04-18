// uart_echo.rs - UARTシリアル通信（エコーバック）
//
// UARTを使ったシリアル通信の基本的な例です。
// 受信したデータをそのまま送り返す（エコーバック）します。
//
// ピン配置:
//   - GPIO0: UART0 TX（送信）
//   - GPIO1: UART0 RX（受信）
//
// 実行方法: cargo run --example uart_echo

#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use panic_probe as _;
use rp2040_hal as hal;

use hal::pac;
use hal::Clock;

use hal::uart::{DataBits, StopBits, UartConfig, UartPeripheral};

#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

const XTAL_FREQ_HZ: u32 = 12_000_000u32;

#[rp2040_hal::entry]
fn main() -> ! {
    info!("UART echo start!");

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

    // UARTピンの設定
    let uart_pins = (
        pins.gpio0.into_function::<hal::gpio::FunctionUart>(),
        pins.gpio1.into_function::<hal::gpio::FunctionUart>(),
    );

    // UART0を115200bps, 8N1で初期化
    let uart = UartPeripheral::new(pac.UART0, uart_pins, &mut pac.RESETS)
        .enable(
            UartConfig::new(
                fugit::RateExtU32::Hz(115200),
                DataBits::Eight,
                None,
                StopBits::One,
            ),
            clocks.peripheral_clock.freq(),
        )
        .unwrap();

    // ウェルカムメッセージ送信
    uart.write_full_blocking(b"Hello from RP2040! Type something...\r\n");
    info!("UART initialized at 115200 baud");

    let mut buf = [0u8; 1];
    loop {
        // 1バイト受信を試みる
        if uart.read_raw(&mut buf).is_ok() {
            let b = buf[0];
            // 受信データをdefmtでログ出力
            info!("Received: 0x{:02x}", b);

            // エコーバック
            uart.write_full_blocking(&[b]);

            // 改行の場合はCR+LFを送信
            if b == b'\r' {
                uart.write_full_blocking(b"\n");
            }
        }
    }
}
