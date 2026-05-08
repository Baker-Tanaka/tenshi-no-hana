// microros_resilient.rs — on-target verification of v0.5 reconnect supervisor.
//
// Demonstrates the disconnect / resume API:
// - publisher_task publishes /angel_nose/heartbeat every second.
// - supervisor_task waits on RUNTIME.wait_for_disconnect(), then sleeps,
//   builds a fresh TCP socket, calls RUNTIME.resume() and spawns a new
//   xrce_exec instance.
// - test_disrupt_task forces a disconnect every 30 s so you can observe the
//   recovery cycle without unplugging WiFi.
//
// Buffer / task slot pool:
//   The example pre-allocates **3 sets** of TCP buffers and a `pool_size = 3`
//   xrce_exec task slot.  That covers the initial connection + 2 reconnects.
//   For long-running deployments increase both numbers.
//
// Build:
//   cargo build --release --no-default-features --features wifi --example microros_resilient
//
// Verify on the host:
//   ros2 topic echo /angel_nose/heartbeat
//   # Stream pauses ~5–10 s every 30 s while the supervisor reconnects, then
//   # resumes seamlessly on the same Publisher handle.
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use cortex_m_rt::exception;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, Stack, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use heapless::String as HString;
use micro_xrce_dds_rs::{client_key, msg, Context, Executor, Publisher, Runtime, RuntimeConfig};
use panic_probe as _;
use portable_atomic::{AtomicUsize, Ordering};
use static_cell::StaticCell;
use wifi_config::AppConfig;

#[exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    defmt::panic!("HardFault: possible stack overflow or bus fault");
}

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

type MySpi = Spi<'static, SPI0, SpiAsync>;
type MySpiDevice = ExclusiveDevice<MySpi, Output<'static>, Delay>;
type MySpiIface = SpiInterface<MySpiDevice, Input<'static>>;
type MyEspRunner = EspRunner<'static, MySpiIface, Output<'static>>;
type MyExecutor = Executor<TcpSocket<'static>>;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static RUNTIME: Runtime = Runtime::new();

// ── 3-slot TCP buffer pool ───────────────────────────────────────────────────
// Each parked Executor task retains its TCP buffers.  We rotate through
// independent slot indexes so a fresh `TcpSocket::new` borrows uncontended
// memory.  `BUF_SLOT_COUNT` must equal the xrce_exec pool_size below.

const BUF_SLOT_COUNT: usize = 3;
static TCP_RX_0: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_TX_0: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_RX_1: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_TX_1: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_RX_2: StaticCell<[u8; 4096]> = StaticCell::new();
static TCP_TX_2: StaticCell<[u8; 4096]> = StaticCell::new();

static NEXT_BUF_SLOT: AtomicUsize = AtomicUsize::new(0);

fn allocate_buf_slot() -> Option<(&'static mut [u8; 4096], &'static mut [u8; 4096])> {
    let idx = NEXT_BUF_SLOT.fetch_add(1, Ordering::Relaxed);
    match idx {
        0 => Some((TCP_RX_0.init([0u8; 4096]), TCP_TX_0.init([0u8; 4096]))),
        1 => Some((TCP_RX_1.init([0u8; 4096]), TCP_TX_1.init([0u8; 4096]))),
        2 => Some((TCP_RX_2.init([0u8; 4096]), TCP_TX_2.init([0u8; 4096]))),
        _ => None, // exhausted — supervisor stops attempting reconnects
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!(
        "[main] microros_resilient start (pool slots={})",
        BUF_SLOT_COUNT as u32
    );

    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 10_000_000;
    spi_cfg.polarity = Polarity::IdleHigh;
    spi_cfg.phase = Phase::CaptureOnSecondTransition;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, p.DMA_CH0, p.DMA_CH1, Irqs, spi_cfg,
    );
    let cs = Output::new(p.PIN_17, Level::High);
    let handshake = Input::new(p.PIN_15, Pull::Down);
    let data_ready = Input::new(p.PIN_13, Pull::Down);
    let reset = Output::new(p.PIN_14, Level::Low);

    let spi_dev = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let spi_iface = SpiInterface::new(spi_dev, handshake, data_ready);
    let esp_state = ESP_STATE.init(State::new());
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;
    spawner.spawn(esp_hosted_task(esp_runner).unwrap());

    let cfg = AppConfig::new();
    info!("[wifi] connecting to \"{}\"...", cfg.wifi_ssid);
    defmt::unwrap!(
        control
            .init(EspConfig {
                static_rx_buf_num: 10,
                dynamic_rx_buf_num: 32,
                tx_buf_type: BufferType::Dynamic,
                static_tx_buf_num: 0,
                dynamic_tx_buf_num: 32,
                rx_mgmt_buf_type: BufferType::Dynamic,
                rx_mgmt_buf_num: 20,
            })
            .await
    );
    let connected = defmt::unwrap!(control.connect(cfg.wifi_ssid, cfg.wifi_password).await);
    defmt::assert!(connected, "WiFi association failed");
    info!("[wifi] connected");

    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        0x1234_5678_9abc_def0u64,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    while stack.config_v4().is_none() {
        Timer::after(Duration::from_millis(500)).await;
    }
    info!("[microros] DHCP OK");

    // First connection.
    let socket = build_and_connect_socket(stack, &cfg).await;
    let (ctx, exec) = defmt::unwrap!(
        RUNTIME
            .start(socket, RuntimeConfig::new(0x81, client_key!()))
            .await
    );
    info!("[microros] runtime started (initial)");

    spawner.spawn(xrce_exec(exec).unwrap());
    spawner.spawn(publisher_node(ctx).unwrap());
    spawner.spawn(supervisor_task(spawner, stack, cfg).unwrap());
    spawner.spawn(test_disrupt_task().unwrap());
}

async fn build_and_connect_socket<'a>(
    stack: Stack<'static>,
    cfg: &AppConfig,
) -> TcpSocket<'static> {
    let (rx, tx) = match allocate_buf_slot() {
        Some(p) => p,
        None => defmt::panic!("[buf-pool] exhausted — increase BUF_SLOT_COUNT"),
    };
    let mut socket = TcpSocket::new(stack, rx.as_mut_slice(), tx.as_mut_slice());
    socket.set_timeout(Some(Duration::from_secs(30)));
    let agent_ep = cfg.agent_endpoint();
    match with_timeout(Duration::from_secs(10), socket.connect(agent_ep)).await {
        Ok(Ok(())) => info!("[microros] TCP connected"),
        _ => defmt::panic!("[microros] TCP connect timeout"),
    }
    socket
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

#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, NetDriver<'static>>) {
    runner.run().await
}

/// Pool-size matches BUF_SLOT_COUNT — one task slot per buffer pair.
#[embassy_executor::task(pool_size = 3)]
async fn xrce_exec(exec: MyExecutor) -> ! {
    exec.run().await
}

#[embassy_executor::task]
async fn publisher_node(ctx: Context) -> ! {
    let node = defmt::unwrap!(ctx.create_node("tenshi_no_hana_resilient").await);
    let pub_hb: Publisher<msg::std_msgs::String> = defmt::unwrap!(
        node.create_publisher::<msg::std_msgs::String>("/angel_nose/heartbeat")
            .await
    );
    info!("[pub] publishing /angel_nose/heartbeat");

    let mut count: u32 = 0;
    loop {
        count += 1;
        let mut buf: HString<32> = HString::new();
        let _ = core::fmt::write(&mut buf, format_args!("hb #{}", count));
        match pub_hb.publish(&msg::std_msgs::String(buf.as_str())).await {
            Ok(()) => info!("[pub] sent: {}", buf.as_str()),
            Err(e) => {
                warn!(
                    "[pub] publish failed: {} — paused while supervisor reconnects",
                    e
                );
                // While disconnected, the next publish() will short-circuit
                // and return Disconnected immediately.  Sleep so we don't
                // spin the log.
                Timer::after(Duration::from_secs(1)).await;
            }
        }
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
async fn supervisor_task(spawner: Spawner, stack: Stack<'static>, cfg: AppConfig) -> ! {
    loop {
        RUNTIME.wait_for_disconnect().await;
        warn!("[supervisor] disconnect detected — backing off 5 s");
        Timer::after(Duration::from_secs(5)).await;

        loop {
            info!("[supervisor] attempting resume...");
            let socket = build_and_connect_socket(stack, &cfg).await;
            match RUNTIME
                .resume(socket, RuntimeConfig::new(0x81, client_key!()))
                .await
            {
                Ok(exec) => {
                    // The auto-generated `xrce_exec(exec)` returns
                    // `Result<SpawnToken<_>, SpawnError>`; allocation of a
                    // task slot from the pool happens here.  Match on it so
                    // pool exhaustion is handled gracefully (instead of the
                    // implicit `.unwrap()` panic the simple examples use).
                    match xrce_exec(exec) {
                        Ok(token) => {
                            spawner.spawn(token);
                            info!("[supervisor] runtime resumed; spawned new xrce_exec");
                            break;
                        }
                        Err(_) => {
                            error!(
                                "[supervisor] xrce_exec pool exhausted ({} parked) — supervisor stopping",
                                BUF_SLOT_COUNT as u32
                            );
                            core::future::pending::<()>().await;
                        }
                    }
                }
                Err(e) => {
                    error!("[supervisor] resume failed: {} — retrying in 5 s", e);
                    Timer::after(Duration::from_secs(5)).await;
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn test_disrupt_task() -> ! {
    // For demo: trigger a disconnect every 30 seconds so the supervisor
    // exercise is observable without unplugging anything.  Comment out for
    // production — leave only the supervisor + publisher.
    loop {
        Timer::after(Duration::from_secs(30)).await;
        warn!("[test] forcing disconnect for supervisor demo");
        RUNTIME.force_disconnect();
    }
}
