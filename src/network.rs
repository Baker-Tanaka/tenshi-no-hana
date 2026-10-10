use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_ros::{
    transport::{
        embassy_net::{
            client_key, Agent, AgentConfig, ConnectFailureKind, Driver, RuntimeConfig, TcpBuffers,
        },
        ConnectionTask,
    },
    Context,
};
use embassy_rp::gpio::{Input, Output};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{
    board::{Wifi, WifiSpi},
    wifi_config::AppConfig,
};

type SpiInterfaceType = SpiInterface<WifiSpi, Input<'static>>;
type EspHostedRunner = EspRunner<'static, SpiInterfaceType, Output<'static>>;
type XrceDriver = Driver<TcpSocket<'static>>;

const WIFI_RETRY_SECS: u64 = 5;
const DHCP_POLL_MS: u64 = 500;
const AGENT_RETRY_SECS: u64 = 5;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
#[embassy_executor::task]
pub(crate) async fn run(spawner: Spawner, context: Context, wifi: Wifi, config: AppConfig) {
    let stack = match start_wifi(spawner, wifi, &config).await {
        Ok(stack) => stack,
        Err(error) => {
            warn!(
                "[wifi] initialization failed: {:?}; sensor/control tasks remain active",
                defmt::Debug2Format(&error)
            );
            return;
        }
    };
    let (connection, driver) = connect_agent(context, stack, &config).await;
    info!("[microros] runtime started");
    if let Err(error) = connection.run_with_driver(driver).await {
        warn!("[embassy_ros] Agent connection stopped: {}", error);
    }
}

async fn start_wifi(
    spawner: Spawner,
    wifi: Wifi,
    config: &AppConfig,
) -> Result<embassy_net::Stack<'static>, embassy_net_esp_hosted_mcu::Error> {
    let interface = SpiInterface::new(wifi.spi, wifi.handshake, wifi.data_ready);
    let (net_device, mut control, esp_runner) =
        embassy_net_esp_hosted_mcu::new(ESP_STATE.init(State::new()), interface, wifi.reset, None)
            .await;
    spawner.spawn(esp_hosted_task(esp_runner).unwrap());

    let (stack, net_runner) = embassy_net::new(
        net_device,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::new()),
        0x1234_5678_9abc_def0u64,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    info!("[wifi] connecting to \"{}\"...", config.wifi_ssid);
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
        .await?;
    loop {
        match control
            .connect(config.wifi_ssid, config.wifi_password)
            .await
        {
            Ok(true) => {
                info!("[wifi] connected");
                break;
            }
            Ok(false) => warn!("[wifi] association failed; retrying"),
            Err(error) => warn!(
                "[wifi] association error: {:?}; retrying",
                defmt::Debug2Format(&error)
            ),
        }
        Timer::after(Duration::from_secs(WIFI_RETRY_SECS)).await;
    }
    while stack.config_v4().is_none() {
        Timer::after(Duration::from_millis(DHCP_POLL_MS)).await;
    }
    info!("[microros] DHCP OK");
    Ok(stack)
}

async fn connect_agent(
    context: Context,
    stack: embassy_net::Stack<'static>,
    config: &AppConfig,
) -> (ConnectionTask, XrceDriver) {
    static TCP_BUFFERS: StaticCell<TcpBuffers> = StaticCell::new();
    let mut socket = TCP_BUFFERS.init(TcpBuffers::new()).socket(stack);
    let agent = Agent::new(context);
    let connection = AgentConfig::new(
        config.agent_endpoint(),
        RuntimeConfig::new(0x81, client_key!()),
    );

    loop {
        match agent.connect(socket, connection).await {
            Ok(connected) => return connected,
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
