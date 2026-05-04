//! Compile-time WiFi and micro-ROS Agent configuration.
//!
//! All settings come from `wifi_config.json` (embedded by `build.rs`).
//! No runtime configuration is needed.
//!
//! # Usage
//!
//! ```ignore
//! let cfg = AppConfig::new();
//! control.connect(cfg.wifi_ssid, cfg.wifi_password).await?;
//! socket.connect(cfg.agent_endpoint()).await?;
//! ```

use embassy_net::{IpEndpoint, Ipv4Address};

const WIFI_SSID_STR: &str = env!("WIFI_SSID");
const WIFI_PASSWORD_STR: &str = env!("WIFI_PASSWORD");
const AGENT_ADDR_STR: &str = env!("MICRO_ROS_AGENT_ADDR");

/// Full compile-time application configuration.
pub struct AppConfig {
    pub wifi_ssid: &'static str,
    pub wifi_password: &'static str,
    pub agent_ip: [u8; 4],
    pub agent_port: u16,
}

impl AppConfig {
    pub fn new() -> Self {
        let (agent_ip, agent_port) = parse_addr(AGENT_ADDR_STR);
        Self {
            wifi_ssid: WIFI_SSID_STR,
            wifi_password: WIFI_PASSWORD_STR,
            agent_ip,
            agent_port,
        }
    }

    /// Build the embassy-net `IpEndpoint` for the micro-ROS Agent.
    pub fn agent_endpoint(&self) -> IpEndpoint {
        IpEndpoint::from((Ipv4Address::from(self.agent_ip), self.agent_port))
    }
}

fn parse_addr(addr: &str) -> ([u8; 4], u16) {
    let colon = addr
        .bytes()
        .rposition(|b| b == b':')
        .expect("MICRO_ROS_AGENT_ADDR: missing ':'. Expected \"a.b.c.d:port\"");
    let (ip_str, port_str) = (&addr[..colon], &addr[colon + 1..]);
    let port = parse_u16(port_str)
        .expect("MICRO_ROS_AGENT_ADDR: invalid port number (1–65535)");
    let mut octets = [0u8; 4];
    let mut count = 0usize;
    let mut tmp = ip_str;
    while let Some(dot) = tmp.find('.') {
        assert!(count < 4, "MICRO_ROS_AGENT_ADDR: too many IP octets");
        octets[count] = parse_u8(&tmp[..dot])
            .expect("MICRO_ROS_AGENT_ADDR: invalid IP octet (0–255)");
        count += 1;
        tmp = &tmp[dot + 1..];
    }
    octets[count] = parse_u8(tmp).expect("MICRO_ROS_AGENT_ADDR: invalid IP octet");
    count += 1;
    assert!(count == 4, "MICRO_ROS_AGENT_ADDR: IP must have 4 octets");
    (octets, port)
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
