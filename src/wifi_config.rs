//! Compile-time WiFi and Zenoh configuration.
//!
//! All settings originate from `wifi_config.json` (embedded by `build.rs`)
//! or are hard-coded device constants.  No runtime configuration is needed.
//!
//! # Usage
//!
//! ```ignore
//! let cfg = AppConfig::new();
//! control.join_wpa2(cfg.wifi_ssid, cfg.wifi_password).await?;
//! socket.connect(cfg.zenoh.router_endpoint()).await?;
//! ```

use embassy_net::{IpEndpoint, Ipv4Address};
use zenoh_ros2_nostd::ros2::ZenohRos2Config;
use zenoh_ros2_nostd::transport::protocol::ZenohId;

// ── Compile-time environment variables (from build.rs / wifi_config.json) ──────

const WIFI_SSID_STR: &str = env!("WIFI_SSID");
const WIFI_PASSWORD_STR: &str = env!("WIFI_PASSWORD");
const ZENOH_ROUTER_ADDR_STR: &str = env!("ZENOH_ROUTER_ADDR");

/// Device Zenoh ID raw bytes — change per device to avoid collisions.
///
/// Use the device MAC address or another unique byte sequence.
const DEVICE_ZID_BYTES: [u8; 8] = [0xBA, 0xCE, 0xA1, 0x05, 0xE0, 0x01, 0x00, 0x01];

// ── Config structs ──────────────────────────────────────────────────────────────

/// Full compile-time application configuration.
pub struct AppConfig {
    /// WiFi SSID to connect to.
    pub wifi_ssid: &'static str,
    /// WiFi password (empty string for open networks).
    pub wifi_password: &'static str,
    /// Zenoh session and router configuration.
    pub zenoh: ZenohConfig,
}

/// Zenoh session and TCP router configuration.
pub struct ZenohConfig {
    /// Router IPv4 address (four octets).
    pub router_ip: [u8; 4],
    /// Router TCP port.
    pub router_port: u16,
    /// Core Zenoh + ROS2 session settings.
    pub session: ZenohRos2Config,
}

impl AppConfig {
    /// Build the configuration by parsing all compile-time env vars.
    ///
    /// Panics at startup if any value in `wifi_config.json` is missing or
    /// malformed.
    pub fn new() -> Self {
        let (router_ip, router_port) = parse_router_addr(ZENOH_ROUTER_ADDR_STR);
        Self {
            wifi_ssid: WIFI_SSID_STR,
            wifi_password: WIFI_PASSWORD_STR,
            zenoh: ZenohConfig {
                router_ip,
                router_port,
                session: ZenohRos2Config::new(ZenohId::from_bytes(&DEVICE_ZID_BYTES)),
            },
        }
    }
}

impl ZenohConfig {
    /// Build the embassy-net `IpEndpoint` for the zenoh router.
    pub fn router_endpoint(&self) -> IpEndpoint {
        IpEndpoint::from((Ipv4Address::from(self.router_ip), self.router_port))
    }
}

// ── Address parsing helpers (no_std, no alloc) ──────────────────────────────────

/// Parse `"a.b.c.d:port"` into `([u8; 4], u16)`.
fn parse_router_addr(addr: &str) -> ([u8; 4], u16) {
    let colon = last_colon(addr)
        .expect("ZENOH_ROUTER_ADDR: missing ':'.  Expected format: \"a.b.c.d:port\"");
    let (ip_str, port_str) = (&addr[..colon], &addr[colon + 1..]);

    let port =
        parse_u16(port_str).expect("ZENOH_ROUTER_ADDR: invalid port number (must be 1–65535)");

    let mut octets = [0u8; 4];
    let mut count = 0usize;
    let mut tmp = ip_str;
    while let Some(dot) = tmp.find('.') {
        assert!(count < 4, "ZENOH_ROUTER_ADDR: too many IP octets");
        octets[count] =
            parse_u8(&tmp[..dot]).expect("ZENOH_ROUTER_ADDR: invalid IP octet (must be 0–255)");
        count += 1;
        tmp = &tmp[dot + 1..];
    }
    assert!(count < 4, "ZENOH_ROUTER_ADDR: too many IP octets");
    octets[count] = parse_u8(tmp).expect("ZENOH_ROUTER_ADDR: invalid IP octet (must be 0–255)");
    count += 1;
    assert!(
        count == 4,
        "ZENOH_ROUTER_ADDR: IP must have exactly 4 octets"
    );

    (octets, port)
}

fn last_colon(s: &str) -> Option<usize> {
    s.bytes().rposition(|b| b == b':')
}

fn parse_u8(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut v: u16 = 0;
    for b in s.bytes() {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        v = v * 10 + (b - b'0') as u16;
        if v > 255 {
            return None;
        }
    }
    Some(v as u8)
}

fn parse_u16(s: &str) -> Option<u16> {
    if s.is_empty() || s.len() > 5 {
        return None;
    }
    let mut v: u32 = 0;
    for b in s.bytes() {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        v = v * 10 + (b - b'0') as u32;
        if v > 65535 {
            return None;
        }
    }
    Some(v as u16)
}
