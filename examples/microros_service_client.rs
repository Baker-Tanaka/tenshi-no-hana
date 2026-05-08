// microros_service_client.rs — on-target verification of v0.3 Service support.
//
// Calls `/add_two_ints` (example_interfaces/srv/AddTwoInts) once per second
// with incrementing operands, prints the sum.
//
// ROS2 side setup (run on the host):
//   ros2 run examples_rclpy_minimal_service service     # hosts /add_two_ints
// or any other AddTwoInts server you have running.
//
// Agent side:
//   docker compose up -d  # starts micro-ROS Agent on TCP port 8888
//
// Build:
//   cargo build --release --no-default-features --features wifi --example microros_service_client
//
// Verify:
//   On the host, `ros2 service list | grep add_two_ints` should show the
//   service.  RTT log on the firmware will print successive `sum=N` lines.
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use cortex_m_rt::exception;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi::{Async as SpiAsync, Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::{bind_interrupts, dma, peripherals::*};
use embassy_time::{with_timeout, Delay, Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use micro_xrce_dds_rs::{
    cdr::CdrWriter, cdr_reader::CdrReader, client_key, service_client_slot, Context, Error,
    Executor, Message, Runtime, RuntimeConfig, Service,
};
use panic_probe as _;
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

// ── example_interfaces/srv/AddTwoInts wire types ─────────────────────────────
//
//   int64 a
//   int64 b
//   ---
//   int64 sum

pub struct AddTwoIntsRequest {
    pub a: i64,
    pub b: i64,
}

impl Message for AddTwoIntsRequest {
    const TYPE_NAME: &'static str = "example_interfaces::srv::dds_::AddTwoInts_Request_";
    const MAX_SERIALIZED_SIZE: usize = 16;

    fn serialize(&self, w: &mut CdrWriter) {
        w.i64_val(self.a);
        w.i64_val(self.b);
    }

    fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> {
        let a = r.i64_val()?;
        let b = r.i64_val()?;
        Ok(Self { a, b })
    }
}

pub struct AddTwoIntsResponse {
    pub sum: i64,
}

impl Message for AddTwoIntsResponse {
    const TYPE_NAME: &'static str = "example_interfaces::srv::dds_::AddTwoInts_Response_";
    const MAX_SERIALIZED_SIZE: usize = 8;

    fn serialize(&self, w: &mut CdrWriter) {
        w.i64_val(self.sum);
    }

    fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> {
        Ok(Self { sum: r.i64_val()? })
    }
}

pub struct AddTwoInts;

impl Service for AddTwoInts {
    type Request = AddTwoIntsRequest;
    type Response = AddTwoIntsResponse;
    const SERVICE_NAME: &'static str = "/add_two_ints";
    const REQUEST_TYPE_NAME: &'static str = "example_interfaces::srv::dds_::AddTwoInts_Request_";
    const RESPONSE_TYPE_NAME: &'static str = "example_interfaces::srv::dds_::AddTwoInts_Response_";
}

service_client_slot!(static ADD_HANDLES: AddTwoInts);

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] microros_service_client start");

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

    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();
    let tcp_rx = TCP_RX.init([0u8; 4096]);
    let tcp_tx = TCP_TX.init([0u8; 4096]);

    let agent_ep = cfg.agent_endpoint();
    let mut socket = TcpSocket::new(stack, tcp_rx, tcp_tx);
    socket.set_timeout(Some(Duration::from_secs(30)));
    match with_timeout(Duration::from_secs(10), socket.connect(agent_ep)).await {
        Ok(Ok(())) => info!("[microros] TCP connected"),
        _ => defmt::panic!("[microros] TCP connect timeout"),
    }

    let (ctx, exec) = defmt::unwrap!(
        RUNTIME
            .start(socket, RuntimeConfig::new(0x81, client_key!()))
            .await
    );
    info!("[microros] runtime started");

    spawner.spawn(xrce_exec(exec).unwrap());
    spawner.spawn(caller_node(ctx).unwrap());
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

#[embassy_executor::task]
async fn xrce_exec(exec: MyExecutor) -> ! {
    exec.run().await
}

#[embassy_executor::task]
async fn caller_node(ctx: Context) -> ! {
    let node = defmt::unwrap!(ctx.create_node("tenshi_no_hana_svc_cli").await);
    let client = defmt::unwrap!(node.create_service_client::<AddTwoInts>(&ADD_HANDLES).await);
    info!(
        "[svc-cli] requester ready: oid=0x{:04X}",
        client.requester_oid()
    );

    let mut a: i64 = 1;
    let mut b: i64 = 2;
    loop {
        match with_timeout(
            Duration::from_secs(5),
            client.call(&AddTwoIntsRequest { a, b }),
        )
        .await
        {
            Ok(Ok(resp)) => info!("[svc-cli] call({}, {}) -> sum={}", a, b, resp.sum),
            Ok(Err(e)) => error!("[svc-cli] call err: {}", e),
            Err(_) => warn!("[svc-cli] call timeout (5s)"),
        }
        a = a.wrapping_add(1);
        b = b.wrapping_add(2);
        Timer::after(Duration::from_secs(1)).await;
    }
}
