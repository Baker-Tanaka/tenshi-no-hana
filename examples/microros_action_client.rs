// microros_action_client.rs — on-target verification of v0.4-rc1 Action support.
//
// Builds an ActionClient<Fibonacci> over the WiFi → micro-ROS Agent path,
// sends a goal { order: 8 }, prints feedback as it streams in, and prints the
// final result.
//
// ROS2 side setup (run on the host with rclpy installed):
//   ros2 launch ... # not needed
//   ros2 run examples_rclpy_minimal_action_server example
//   # The standard rclpy minimal action server hosts /fibonacci.
//   # Result of fib(8) = [0, 1, 1, 2, 3, 5, 8, 13, 21]
//
// Agent side:
//   docker compose up -d  # starts micro-ROS Agent on TCP port 8888
//
// Build:
//   cargo build --release --no-default-features --features wifi --example microros_action_client
//
// Flash via DAP-server / probe-rs.
#![no_std]
#![no_main]

#[path = "../src/wifi_config.rs"]
mod wifi_config;

use cortex_m_rt::exception;
use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
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
use heapless::Vec as HVec;
use micro_xrce_dds_rs::{
    cdr::CdrWriter, cdr_reader::CdrReader, client_key, subscription_slot, Action,
    ActionClientHandles, Context, Error, Executor, FeedbackMessage, Message, Runtime,
    RuntimeConfig,
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

// ── Fibonacci action — ros2_interfaces compatible wire types ─────────────────

const FIB_MAX_LEN: usize = 64;

pub struct FibonacciGoal {
    pub order: i32,
}

impl Message for FibonacciGoal {
    const TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Goal_";
    const MAX_SERIALIZED_SIZE: usize = 4;

    fn serialize(&self, w: &mut CdrWriter) {
        w.i32_val(self.order);
    }

    fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            order: r.i32_val()?,
        })
    }
}

pub struct FibonacciResult {
    pub sequence: HVec<i32, FIB_MAX_LEN>,
}

impl Message for FibonacciResult {
    const TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Result_";
    const MAX_SERIALIZED_SIZE: usize = 4 + 4 * FIB_MAX_LEN;

    fn serialize(&self, w: &mut CdrWriter) {
        w.i32_seq(self.sequence.as_slice());
    }

    fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> {
        let seq: HVec<i32, FIB_MAX_LEN> = r.i32_seq_into::<FIB_MAX_LEN>()?;
        Ok(Self { sequence: seq })
    }
}

pub struct FibonacciFeedback {
    pub partial_sequence: HVec<i32, FIB_MAX_LEN>,
}

impl Message for FibonacciFeedback {
    const TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Feedback_";
    const MAX_SERIALIZED_SIZE: usize = 4 + 4 * FIB_MAX_LEN;

    fn serialize(&self, w: &mut CdrWriter) {
        w.i32_seq(self.partial_sequence.as_slice());
    }

    fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> {
        let seq: HVec<i32, FIB_MAX_LEN> = r.i32_seq_into::<FIB_MAX_LEN>()?;
        Ok(Self {
            partial_sequence: seq,
        })
    }
}

pub struct Fibonacci;

impl Action for Fibonacci {
    type Goal = FibonacciGoal;
    type Result = FibonacciResult;
    type Feedback = FibonacciFeedback;

    const ACTION_NAME: &'static str = "/fibonacci";

    const GOAL_TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Goal_";
    const RESULT_TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Result_";
    const FEEDBACK_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_Feedback_";

    const SEND_GOAL_SERVICE_NAME: &'static str = "/fibonacci/_action/send_goal";
    const GET_RESULT_SERVICE_NAME: &'static str = "/fibonacci/_action/get_result";
    const CANCEL_GOAL_SERVICE_NAME: &'static str = "/fibonacci/_action/cancel_goal";
    const FEEDBACK_TOPIC_NAME: &'static str = "/fibonacci/_action/feedback";
    const STATUS_TOPIC_NAME: &'static str = "/fibonacci/_action/status";

    const SEND_GOAL_REQUEST_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_SendGoal_Request_";
    const SEND_GOAL_RESPONSE_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_SendGoal_Response_";
    const GET_RESULT_REQUEST_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_GetResult_Request_";
    const GET_RESULT_RESPONSE_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_GetResult_Response_";
    const FEEDBACK_MESSAGE_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_FeedbackMessage_";
}

// ── Statics for the action client ────────────────────────────────────────────

static FIB_HANDLES: ActionClientHandles<Fibonacci> = ActionClientHandles::new();
subscription_slot!(static FIB_FEEDBACK: FeedbackMessage<Fibonacci>, depth = 4);

// ── main / tasks ─────────────────────────────────────────────────────────────

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] microros_action_client start");

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
    spawner.spawn(action_node(ctx).unwrap());
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
async fn action_node(ctx: Context) -> ! {
    let node = defmt::unwrap!(ctx.create_node("tenshi_no_hana_action_cli").await);

    let client = defmt::unwrap!(
        node.create_action_client::<Fibonacci, 4>(&FIB_HANDLES, &FIB_FEEDBACK)
            .await
    );
    info!(
        "[action] client ready: requester_oid=0x{:04X}",
        client.send_goal_requester_oid()
    );

    let order: i32 = 8;
    info!("[action] sending goal: order={}", order);

    // Retry loop: keep trying until the agent accepts the goal.
    let handle = loop {
        match client.send_goal(FibonacciGoal { order }).await {
            Ok(h) => {
                info!(
                    "[action] goal accepted: goal_id_first8={:?} stamp.sec={}",
                    &h.goal_id.0[..8],
                    h.stamp.sec
                );
                break h;
            }
            Err(e) => {
                error!("[action] send_goal failed: {} — retrying in 5s", e);
                Timer::after(Duration::from_secs(5)).await;
            }
        }
    };

    // GoalHandle is Copy: feed one copy to await_result (which consumes it)
    // and keep `handle` for next_feedback in the select loop.
    let result_fut = handle.await_result();
    let mut result_fut = core::pin::pin!(result_fut);
    loop {
        match select(handle.next_feedback(), result_fut.as_mut()).await {
            Either::First(fb) => {
                info!(
                    "[feedback] partial_len={}",
                    fb.partial_sequence.len() as u32
                );
                for (i, v) in fb.partial_sequence.iter().enumerate() {
                    info!("    fb[{}] {}", i as u32, *v);
                }
            }
            Either::Second(result) => {
                match result {
                    Ok(r) => {
                        info!("[action] result: len={}", r.sequence.len() as u32);
                        for (i, v) in r.sequence.iter().enumerate() {
                            info!("  [{}] {}", i as u32, *v);
                        }
                    }
                    Err(e) => error!("[action] await_result failed: {}", e),
                }
                break;
            }
        }
    }

    info!("[action] done — parking");
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
