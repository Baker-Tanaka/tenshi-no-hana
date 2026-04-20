// sensor_read.rs — BME280 + MQ-3 sensor reading with Embassy
//
// Baker link.dev (RP2040) reads:
//   - BME280 (I2C0: GP4 SDA, GP5 SCL) → temperature, humidity, pressure
//   - MQ-3B  (ADC0: GP26)              → ethanol vapor analog voltage
//
// Build:
//   cargo build --no-default-features --features sensor --example sensor_read
//
// Run (probe-rs):
//   cargo run --no-default-features --features sensor --example sensor_read

#![no_std]
#![no_main]

use bme280::i2c::BME280;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcIrqHandler};
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::Pull;
use embassy_rp::i2c::{Config as I2cConfig, I2c};
use embassy_time::{Delay, Duration, Timer};
use panic_probe as _;

bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => AdcIrqHandler;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    defmt::trace!("RTT init");
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] Sensor read example start");

    // ── I2C0: GP4 (SDA), GP5 (SCL) → BME280 (blocking) ─────────────────────
    let i2c = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let mut bme280 = BME280::new_primary(i2c);

    match bme280.init(&mut Delay) {
        Ok(()) => info!("[bme280] Initialized (addr=0x76)"),
        Err(_) => {
            error!("[bme280] Init failed — check wiring (SDA=GP4, SCL=GP5)");
            loop {
                Timer::after_secs(60).await;
            }
        }
    }

    // ── ADC0: GP26 → MQ-3B ──────────────────────────────────────────────────
    let mut adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mut mq3_ch = Channel::new_pin(p.PIN_26, Pull::None);
    info!("[mq3] ADC channel ready (GP26)");

    // ── Sensor loop (2s interval) ────────────────────────────────────────────
    let mut count: u32 = 0;
    loop {
        count += 1;

        // BME280 measurement (blocking — completes in ~μs)
        match bme280.measure(&mut Delay) {
            Ok(m) => {
                info!(
                    "[bme280] #{}: T={} °C  H={} %  P={} hPa",
                    count,
                    m.temperature,
                    m.humidity,
                    m.pressure / 100.0, // Pa → hPa
                );
            }
            Err(_) => warn!("[bme280] #{}: Read failed", count),
        }

        // MQ-3 ADC measurement
        match adc.read(&mut mq3_ch).await {
            Ok(raw) => {
                let voltage = raw as f32 * 3.3 / 4096.0;
                info!("[mq3] #{}: raw={} voltage={}V", count, raw, voltage);
            }
            Err(_) => warn!("[mq3] #{}: ADC read failed", count),
        }

        Timer::after(Duration::from_secs(2)).await;
    }
}
