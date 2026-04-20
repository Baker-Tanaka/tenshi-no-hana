// esp_hosted_spi_test.rs — Verify SPI communication with ESP32-C3 (esp-hosted-mcu)
//
// Diagnostic test for SPI bus between Baker link.dev (RP2040) and XIAO ESP32-C3.
//
// Steps:
//   1. GPIO init
//   2. Pin state diagnostics — manual reset + HS/DR monitoring
//   3. SPI0 + esp-hosted driver start
//   4. Control::init() — IOCTL communication test
//   5. Control::get_status()
//
// Build:
//   cargo build --no-default-features --features wifi --example esp_hosted_spi_test
//
#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, MAX_SPI_BUFFER_SIZE, Runner as EspRunner, SpiInterface, State,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Async, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{Delay, Duration, Timer, with_timeout};
use embedded_hal_bus::spi::ExclusiveDevice;
use panic_probe as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

type MySpi = Spi<'static, SPI0, Async>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;

static ESP_STATE: StaticCell<State> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    defmt::trace!("RTT init");
    Timer::after(Duration::from_millis(500)).await;
    info!("========================================");
    info!("  ESP-Hosted SPI Communication Test");
    info!("  Build: v3 — SPI Mode 3 (CPOL=1/CPHA=1)");
    info!("========================================");

    // ── Step 1: GPIO init ────────────────────────────────────────────────────
    info!("[1/5] GPIO init...");
    info!("  SCK=GP18 MOSI=GP19 MISO=GP16 CS=GP17");
    info!("  HS=GP15  DR=GP13   RST=GP14");

    // HS: Pull::Down (matches ESP32 GPIO_PULLDOWN_ONLY on HS output)
    // DR: Pull::Down (matches ESP32 GPIO_PULLDOWN_ONLY on DR output)
    let handshake = Input::new(p.PIN_15, Pull::Down);
    let data_ready = Input::new(p.PIN_13, Pull::Down);
    let mut reset = Output::new(p.PIN_14, Level::Low); // ESP32 held in reset
    info!("[1/5] OK");

    // ── Step 2: Pin state diagnostics ────────────────────────────────────────
    info!("[2/5] Pin diagnostics (manual reset cycle)...");

    // ESP32 should be in reset (GP14=LOW) — HS/DR should be LOW
    Timer::after_millis(100).await;
    info!(
        "  [reset asserted] HS={} DR={}",
        if handshake.is_high() { "HIGH" } else { "LOW" },
        if data_ready.is_high() { "HIGH" } else { "LOW" },
    );

    // Release reset — ESP32 boots
    reset.set_high();
    info!("  Reset released (GP14=HIGH) — ESP32 booting...");

    // Monitor HS/DR for 3 seconds
    let mut hs_seen_high = false;
    let mut dr_seen_high = false;
    for i in 0..6u8 {
        Timer::after_millis(500).await;
        let hs = handshake.is_high();
        let dr = data_ready.is_high();
        if hs {
            hs_seen_high = true;
        }
        if dr {
            dr_seen_high = true;
        }
        info!(
            "  [t={}ms] HS={} DR={}",
            (i as u16 + 1) * 500,
            if hs { "HIGH" } else { "LOW" },
            if dr { "HIGH" } else { "LOW" },
        );
    }

    if hs_seen_high {
        info!("  >> HS went HIGH — handshake signal OK");
    } else {
        warn!("  >> HS never HIGH — check GP15 <-> ESP32 GPIO3 wiring");
    }
    if dr_seen_high {
        info!("  >> DR went HIGH — data-ready signal OK");
    } else {
        warn!("  >> DR never HIGH — check GP13 <-> ESP32 GPIO4 wiring");
    }

    // Hold ESP32 in reset for the driver to do its own reset cycle
    reset.set_low();
    Timer::after_millis(100).await;
    info!("[2/5] OK — pin diagnostics complete");

    // ── Step 3: SPI0 + driver init ───────────────────────────────────────────
    info!("[3/5] SPI0 init + esp-hosted driver start...");

    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 10_000_000; // 10 MHz
    spi_cfg.polarity = Polarity::IdleHigh; // CPOL=1
    spi_cfg.phase = Phase::CaptureOnSecondTransition; // CPHA=1  (SPI Mode 3, matches ESP32)

    let spi: MySpi = Spi::new(
        p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, p.DMA_CH0, p.DMA_CH1, Irqs, spi_cfg,
    );

    let cs = Output::new(p.PIN_17, Level::High);
    let spi_dev: MySpiDevice = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let spi_iface = SpiInterface::new(spi_dev, handshake, data_ready);

    let esp_state = ESP_STATE.init(State::new());
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;

    spawner.spawn(esp_hosted_task(esp_runner).unwrap());
    let _ = net_device;

    info!("[3/5] OK — driver started, runner resets ESP32 internally");

    // ── Step 4: Control::init() — IOCTL communication test ──────────────────
    info!("[4/5] control.init() (heartbeat + wifi mode + MAC)...");

    match with_timeout(
        Duration::from_secs(15),
        control.init(EspConfig {
            static_rx_buf_num: 10,
            dynamic_rx_buf_num: 32,
            tx_buf_type: BufferType::Dynamic,
            static_tx_buf_num: 0,
            dynamic_tx_buf_num: 32,
            rx_mgmt_buf_type: BufferType::Dynamic,
            rx_mgmt_buf_num: 20,
        }),
    )
    .await
    {
        Ok(Ok(())) => {
            info!("[4/5] OK — control.init() succeeded!");
            info!("  SPI communication confirmed working");
        }
        Ok(Err(e)) => {
            match e {
                embassy_net_esp_hosted_mcu::Error::Failed(code) => {
                    error!("[4/5] FAIL — control.init() error: Failed({})", code)
                }
                embassy_net_esp_hosted_mcu::Error::Timeout => {
                    error!("[4/5] FAIL — control.init() error: Timeout")
                }
                embassy_net_esp_hosted_mcu::Error::Internal => {
                    error!("[4/5] FAIL — control.init() error: Internal")
                }
            }
            loop {
                Timer::after_secs(60).await;
            }
        }
        Err(_) => {
            error!("[4/5] FAIL — control.init() timeout (15s)");
            error!("  Runner could not communicate with ESP32 over SPI.");
            if !hs_seen_high {
                error!("  LIKELY CAUSE: HS never went HIGH in pin diagnostics");
            }
            if !dr_seen_high {
                error!("  LIKELY CAUSE: DR never went HIGH in pin diagnostics");
            }
            if hs_seen_high && dr_seen_high {
                error!("  HS/DR worked in manual test but driver failed.");
                error!("  Possible: embassy-net-esp-hosted v0.3 targets");
                error!("  esp-hosted-fg, not esp-hosted-mcu v2.x.");
                error!("  Try esp-hosted-fg firmware on ESP32-C3.");
            }
            loop {
                Timer::after_secs(60).await;
            }
        }
    }

    // ── Step 5: get_status() — further IOCTL validation ─────────────────────
    info!("[5/5] control.get_status()...");

    match with_timeout(Duration::from_secs(5), control.get_status()).await {
        Ok(Ok(status)) => {
            info!("[5/5] OK — WiFi status:");
            info!(
                "  SSID: \"{=str}\"  RSSI: {} dBm  Ch: {}",
                status.ssid.as_str(),
                status.rssi,
                status.channel
            );
        }
        Ok(Err(e)) => match e {
            embassy_net_esp_hosted_mcu::Error::Failed(code) => {
                warn!("[5/5] get_status() Failed({}) — OK if not connected", code)
            }
            embassy_net_esp_hosted_mcu::Error::Timeout => {
                warn!("[5/5] get_status() Timeout")
            }
            embassy_net_esp_hosted_mcu::Error::Internal => {
                warn!("[5/5] get_status() Internal error")
            }
        },
        Err(_) => {
            warn!("[5/5] get_status() timeout");
        }
    }

    info!("========================================");
    info!("  SPI TEST PASSED");
    info!("  RP2040 <-> ESP32-C3 communication OK");
    info!("========================================");

    loop {
        Timer::after_secs(3600).await;
    }
}

#[embassy_executor::task]
async fn esp_hosted_task(runner: MyEspRunner) {
    static TX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    runner
        .run(
            TX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE]),
            RX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE]),
        )
        .await
}
