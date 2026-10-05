use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{DhcpConfig, Runner as NetRunner, StackResources};
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, NetDriver, Runner as EspRunner, SpiInterface, State,
    MAX_SPI_BUFFER_SIZE,
};
use embassy_rp::gpio::{Input, Output};
use embassy_time::{with_timeout, Duration, Timer};
use micro_xrce_dds_rs::{client_key, Driver, Runtime, RuntimeConfig, Session};
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
const AGENT_CONNECT_TIMEOUT_SECS: u64 = 10;
const AGENT_RETRY_SECS: u64 = 5;

static ESP_STATE: StaticCell<State> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static XRCE_RUNTIME: Runtime = Runtime::new();

pub(crate) async fn start(spawner: Spawner, wifi: Wifi, config: &AppConfig) -> Session {
    let stack = start_wifi(spawner, wifi, config).await;
    let (session, driver) = connect_agent(stack, config).await;
    info!("[microros] runtime started");
    spawner.spawn(xrce_driver_task(driver).unwrap());

    session
}

async fn start_wifi(
    spawner: Spawner,
    wifi: Wifi,
    config: &AppConfig,
) -> embassy_net::Stack<'static> {
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
    loop {
        if defmt::unwrap!(
            control
                .connect(config.wifi_ssid, config.wifi_password)
                .await
        ) {
            info!("[wifi] connected");
            break;
        }
        warn!("[wifi] association failed; retrying");
        Timer::after(Duration::from_secs(WIFI_RETRY_SECS)).await;
    }
    while stack.config_v4().is_none() {
        Timer::after(Duration::from_millis(DHCP_POLL_MS)).await;
    }
    info!("[microros] DHCP OK");
    stack
}

async fn connect_agent(
    stack: embassy_net::Stack<'static>,
    config: &AppConfig,
) -> (Session, XrceDriver) {
    static TCP_RX: StaticCell<[u8; 4096]> = StaticCell::new();
    static TCP_TX: StaticCell<[u8; 4096]> = StaticCell::new();
    let mut socket = TcpSocket::new(stack, TCP_RX.init([0; 4096]), TCP_TX.init([0; 4096]));
    socket.set_timeout(Some(Duration::from_secs(30)));
    let endpoint = config.agent_endpoint();

    loop {
        match with_timeout(
            Duration::from_secs(AGENT_CONNECT_TIMEOUT_SECS),
            socket.connect(endpoint),
        )
        .await
        {
            Ok(Ok(())) => {
                info!("[microros] TCP connected");
                break;
            }
            Ok(Err(error)) => warn!(
                "[microros] TCP connect failed: {:?}; retrying",
                defmt::Debug2Format(&error)
            ),
            Err(_) => warn!("[microros] TCP connect timed out; retrying"),
        }
        socket.abort();
        Timer::after(Duration::from_secs(AGENT_RETRY_SECS)).await;
    }

    defmt::unwrap!(
        XRCE_RUNTIME
            .start(socket, RuntimeConfig::new(0x81, client_key!()))
            .await
    )
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

#[embassy_executor::task]
async fn xrce_driver_task(driver: XrceDriver) {
    if let Err(error) = driver.run().await {
        warn!("[microros] XRCE driver stopped: {}", error);
    }
}
