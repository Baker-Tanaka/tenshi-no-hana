// microros_action_server.rs — on-target verification of v0.4-rc1 ActionServer.
//
// Hosts `/angel_nose/fibonacci` (example_interfaces/action/Fibonacci) on the
// firmware.  On the ROS2 host you can drive it with:
//
//   ros2 action send_goal /angel_nose/fibonacci example_interfaces/action/Fibonacci \
//        '{order: 8}' --feedback
//
// The firmware computes the Fibonacci sequence one element per 200 ms,
// publishing each partial sequence as feedback, then replies via the
// GetResult service with status=SUCCEEDED + the full sequence.
//
// Agent side:
//   docker compose up -d
//
// Build:
//   cargo build --release --no-default-features --features wifi --example microros_action_server
//
// Note: this server runs under the **single-flight** semantics of v0.4-rc1.
// Concurrent goals are not supported; the second `ros2 action send_goal` will
// queue behind the active one.
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
use heapless::Vec as HVec;
use micro_xrce_dds_rs::{
    action::goal_status, cdr::CdrWriter, cdr_reader::CdrReader, client_key, Action, ActionServer,
    ActionServerHandles, Context, Error, Executor, Message, Runtime, RuntimeConfig,
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

// ── Fibonacci wire types — server uses the same definitions as the client ───
//
// example_interfaces/action/Fibonacci.action:
//   int32 order
//   ---
//   int32[] sequence
//   ---
//   int32[] partial_sequence

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
        Ok(Self {
            sequence: r.i32_seq_into::<FIB_MAX_LEN>()?,
        })
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
        Ok(Self {
            partial_sequence: r.i32_seq_into::<FIB_MAX_LEN>()?,
        })
    }
}

pub struct Fibonacci;

impl Action for Fibonacci {
    type Goal = FibonacciGoal;
    type Result = FibonacciResult;
    type Feedback = FibonacciFeedback;

    /// Namespaced under `/angel_nose` so it doesn't collide with the standard
    /// rclpy minimal action server when both run on the same agent.
    const ACTION_NAME: &'static str = "/angel_nose/fibonacci";

    const GOAL_TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Goal_";
    const RESULT_TYPE_NAME: &'static str = "example_interfaces::action::dds_::Fibonacci_Result_";
    const FEEDBACK_TYPE_NAME: &'static str =
        "example_interfaces::action::dds_::Fibonacci_Feedback_";

    const SEND_GOAL_SERVICE_NAME: &'static str = "/angel_nose/fibonacci/_action/send_goal";
    const GET_RESULT_SERVICE_NAME: &'static str = "/angel_nose/fibonacci/_action/get_result";
    const CANCEL_GOAL_SERVICE_NAME: &'static str = "/angel_nose/fibonacci/_action/cancel_goal";
    const FEEDBACK_TOPIC_NAME: &'static str = "/angel_nose/fibonacci/_action/feedback";
    const STATUS_TOPIC_NAME: &'static str = "/angel_nose/fibonacci/_action/status";

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

static FIB_SRV_HANDLES: ActionServerHandles<Fibonacci, 4, 4, 4> = ActionServerHandles::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    Timer::after(Duration::from_millis(500)).await;
    info!("[main] microros_action_server start");

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
    let node = defmt::unwrap!(ctx.create_node("tenshi_no_hana_action_srv").await);

    let server: ActionServer<Fibonacci, 4, 4, 4> = defmt::unwrap!(
        node.create_action_server::<Fibonacci, 4, 4, 4>(&FIB_SRV_HANDLES)
            .await
    );
    info!("[action-srv] ready");
    info!(
        "[action-srv] try: ros2 action send_goal /angel_nose/fibonacci example_interfaces/action/Fibonacci '{{order: 8}}' --feedback"
    );

    // Drain incoming CancelGoal requests in a side task.  This routes
    // matching cancels to the active goal's cancel flag (read by
    // `accepted.is_cancel_requested()` below) and replies ERROR_NONE; non-
    // matching cancels get ERROR_UNKNOWN_GOAL_ID.  Safe because we are
    // running on the same Embassy executor that the original `main`
    // spawned us on.
    let inner_spawner = unsafe { Spawner::for_current_executor() }.await;
    inner_spawner.spawn(cancel_drain(server).unwrap());

    loop {
        let accepted = match server.accept_next_goal().await {
            Ok(g) => g,
            Err(e) => {
                error!("[action-srv] accept_next_goal failed: {}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };

        let order = accepted.goal.order.max(0).min(FIB_MAX_LEN as i32);
        info!(
            "[action-srv] accepted goal_id_first8={:?} order={}",
            &accepted.goal_id.0[..8],
            order
        );

        // Tell observers we've started executing.  rclpy/rclcpp don't
        // strictly require this — they look at the GetResult reply — but
        // publishing it makes `ros2 action list -t` and similar tooling work.
        if let Err(e) = server
            .publish_status_for_active(accepted.goal_id, goal_status::STATUS_EXECUTING)
            .await
        {
            warn!("[action-srv] publish_status(EXECUTING) failed: {}", e);
        }

        // Compute the Fibonacci sequence step-by-step, publishing feedback as
        // we go.  Use a fresh local heapless::Vec each iteration so we move
        // ownership into the FeedbackMessage.  Poll for cancellation between
        // steps and break out cleanly if requested.
        let mut so_far: HVec<i32, FIB_MAX_LEN> = HVec::new();
        let mut canceled = false;
        for i in 0..(order as usize) {
            if accepted.is_cancel_requested() {
                warn!(
                    "[action-srv] cancel requested mid-execution at step {}",
                    i as u32
                );
                canceled = true;
                break;
            }
            let next = match i {
                0 => 0i32,
                1 => 1i32,
                _ => so_far[i - 1].wrapping_add(so_far[i - 2]),
            };
            if so_far.push(next).is_err() {
                break;
            }
            let mut fb_seq: HVec<i32, FIB_MAX_LEN> = HVec::new();
            for v in so_far.iter() {
                fb_seq.push(*v).ok();
            }
            if let Err(e) = accepted
                .publish_feedback(FibonacciFeedback {
                    partial_sequence: fb_seq,
                })
                .await
            {
                warn!("[action-srv] publish_feedback failed: {}", e);
            }
            Timer::after(Duration::from_millis(200)).await;
        }

        let active_goal_id = accepted.goal_id;
        let result = FibonacciResult { sequence: so_far };

        if canceled {
            info!(
                "[action-srv] computed partial len={} — replying CANCELED",
                result.sequence.len() as u32
            );
            // Status publish first so subscribers see the lifecycle, then the
            // GetResult reply with status=CANCELED.
            if let Err(e) = server
                .publish_status_for_active(active_goal_id, goal_status::STATUS_CANCELED)
                .await
            {
                warn!("[action-srv] publish_status(CANCELED) failed: {}", e);
            }
            if let Err(e) = accepted.canceled(result).await {
                error!("[action-srv] canceled() failed: {}", e);
            }
        } else {
            info!(
                "[action-srv] computed result len={} — replying SUCCEEDED",
                result.sequence.len() as u32
            );
            if let Err(e) = server
                .publish_status_for_active(active_goal_id, goal_status::STATUS_SUCCEEDED)
                .await
            {
                warn!("[action-srv] publish_status(SUCCEEDED) failed: {}", e);
            }
            if let Err(e) = accepted.succeed(result).await {
                error!("[action-srv] succeed failed: {}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn cancel_drain(server: ActionServer<Fibonacci, 4, 4, 4>) -> ! {
    // Active-goal-aware cancel routing — sets the cancel flag on the
    // currently-executing goal so the work loop can observe it via
    // `accepted.is_cancel_requested()`.
    server.serve_cancels_for_active().await
}
