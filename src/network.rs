use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::{
    join::join,
    select::{select, Either},
};
use embassy_net::{DhcpConfig, Runner as NetRunner, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, Control, EspConfig, EspHostedEvents, NetDriver, Runner as EspRunner,
    SpiInterface, State, MAX_SPI_BUFFER_SIZE,
};
use embassy_ros::{
    transport::embassy_net::{
        client_key, Agent, AgentConfig, ConnectFailureKind, RuntimeConfig, TcpBuffers,
    },
    Context,
};
use embassy_rp::gpio::{Input, Output};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    board::{Wifi, WifiSpi},
    wifi_config::AppConfig,
};

type SpiInterfaceType = SpiInterface<WifiSpi, Input<'static>>;
type EspHostedRunner = EspRunner<'static, SpiInterfaceType, Output<'static>>;

const WIFI_RETRY_SECS: u64 = 5;
const AGENT_RETRY_SECS: u64 = 5;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static WIFI_EVENTS: StaticCell<Signal<NoopRawMutex, EspHostedEvents>> = StaticCell::new();
#[embassy_executor::task]
pub(crate) async fn run(spawner: Spawner, context: Context, wifi: Wifi, config: AppConfig) {
    let events = WIFI_EVENTS.init(Signal::new());
    let (stack, control) = start_wifi(spawner, wifi, events).await;
    join(
        maintain_wifi(control, stack, events, &config),
        maintain_agent(context, stack, &config),
    )
    .await;
}

async fn start_wifi(
    spawner: Spawner,
    wifi: Wifi,
    events: &'static Signal<NoopRawMutex, EspHostedEvents>,
) -> (embassy_net::Stack<'static>, Control<'static>) {
    let interface = SpiInterface::new(wifi.spi, wifi.handshake, wifi.data_ready);
    let (net_device, control, esp_runner) = embassy_net_esp_hosted_mcu::new(
        ESP_STATE.init(State::new()),
        interface,
        wifi.reset,
        Some(events),
    )
    .await;
    spawner.spawn(esp_hosted_task(esp_runner).unwrap());

    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        0x1234_5678_9abc_def0u64,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    (stack, control)
}

async fn maintain_wifi(
    mut control: Control<'static>,
    stack: embassy_net::Stack<'static>,
    events: &'static Signal<NoopRawMutex, EspHostedEvents>,
    config: &AppConfig,
) {
    let mut initialized = false;
    loop {
        if !initialized {
            info!("[wifi] waiting for ESP-hosted initialization");
            match control
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
            {
                Ok(()) => initialized = true,
                Err(error) => {
                    warn!(
                        "[wifi] initialization failed: {:?}",
                        defmt::Debug2Format(&error)
                    );
                    if let Err(error) = control.reset_device().await {
                        warn!("[wifi] reset failed: {:?}", defmt::Debug2Format(&error));
                    }
                    Timer::after(Duration::from_secs(WIFI_RETRY_SECS)).await;
                    continue;
                }
            }
        }
        events.reset();
        info!("[wifi] connecting to \"{}\"...", config.wifi_ssid);
        match control
            .connect(config.wifi_ssid, config.wifi_password)
            .await
        {
            Ok(true) => {
                info!("[wifi] connected");
                // The control event precedes the network runner's link-state update.
                stack.wait_link_up().await;
                match select(events.wait(), stack.wait_link_down()).await {
                    Either::First(EspHostedEvents::Deadline) => {
                        warn!("[wifi] ESP heartbeat stopped; resetting");
                        if let Err(error) = control.reset_device().await {
                            warn!("[wifi] reset failed: {:?}", defmt::Debug2Format(&error));
                        }
                        initialized = false;
                    }
                    _ => warn!("[wifi] link lost; reconnecting"),
                }
            }
            Ok(false) => warn!("[wifi] association failed; retrying"),
            Err(error) => warn!(
                "[wifi] association error: {:?}; retrying",
                defmt::Debug2Format(&error)
            ),
        }
        Timer::after(Duration::from_secs(WIFI_RETRY_SECS)).await;
    }
}

async fn maintain_agent(context: Context, stack: embassy_net::Stack<'static>, config: &AppConfig) {
    static TCP_BUFFERS: StaticCell<TcpBuffers> = StaticCell::new();
    let mut socket = TCP_BUFFERS.init(TcpBuffers::new()).socket(stack);
    let agent = Agent::new(context);
    let connection_config = AgentConfig::new(
        config.agent_endpoint(),
        RuntimeConfig::new(0x81, client_key!()),
    );

    loop {
        stack.wait_link_up().await;
        stack.wait_config_up().await;
        info!("[microros] DHCP OK; connecting to Agent");
        match agent.connect(socket, connection_config).await {
            Ok((connection, mut driver)) => {
                info!("[microros] runtime started");
                let network_lost = select(stack.wait_link_down(), stack.wait_config_down());
                match select(network_lost, connection.run_with_driver(&mut driver)).await {
                    Either::First(_) => warn!("[microros] network lost; reconnecting"),
                    Either::Second(Err(error)) => {
                        warn!(
                            "[embassy_ros] Agent connection stopped: {}; retrying",
                            error
                        )
                    }
                    Either::Second(Ok(())) => {
                        warn!("[embassy_ros] Agent connection ended; retrying")
                    }
                }
                socket = driver.into_inner();
                socket.abort();
            }
            Err(failure) => {
                if matches!(
                    failure.kind,
                    ConnectFailureKind::InvalidConfiguration
                        | ConnectFailureKind::Handshake(
                            micro_xrce_dds_rs::Error::InvalidInput
                                | micro_xrce_dds_rs::Error::AlreadyStarted
                        )
                ) {
                    defmt::panic!(
                        "[microros] invalid Agent configuration: {:?}",
                        defmt::Debug2Format(&failure.kind)
                    );
                }
                warn!(
                    "[microros] Agent connection failed: {:?}; retrying",
                    defmt::Debug2Format(&failure.kind)
                );
                socket = failure.socket;
                socket.abort();
            }
        }
        Timer::after(Duration::from_secs(AGENT_RETRY_SECS)).await;
    }
}

#[embassy_executor::task]
async fn esp_hosted_task(runner: EspHostedRunner) {
    static TX_BUFFER: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    static RX_BUFFER: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    runner
        .run(
            TX_BUFFER.init([0; MAX_SPI_BUFFER_SIZE]),
            RX_BUFFER.init([0; MAX_SPI_BUFFER_SIZE]),
        )
        .await
}

#[embassy_executor::task]
async fn net_task(mut runner: NetRunner<'static, NetDriver<'static>>) {
    runner.run().await
}
