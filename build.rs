use std::env;
use std::fs;
use std::net::SocketAddrV4;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
struct WifiConfig {
    ssid: String,
    password: String,
    agent_addr: String,
}

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=wifi_config.json");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-link-search={}", manifest_dir.display());

    if env::var("CARGO_FEATURE_WIFI").is_ok() {
        let json =
            fs::read_to_string(manifest_dir.join("wifi_config.json")).unwrap_or_else(|error| {
                panic!(
                    "\n\n\
                 wifi_config.json could not be read.\n\
                 Copy wifi_config.json.example to wifi_config.json and fill in your credentials.\n\
                 This file is git-ignored and will not be committed.\n\
                 Read error: {error}\n"
                )
            });

        let config: WifiConfig = serde_json::from_str(&json).unwrap_or_else(|error| {
            panic!("wifi_config.json: invalid configuration: {error}");
        });
        assert!(
            !config.ssid.is_empty() && config.ssid.len() <= 32,
            "wifi_config.json: ssid must be 1-32 UTF-8 bytes"
        );
        assert!(
            config.password.len() <= 32,
            "wifi_config.json: this ESP-hosted driver supports passwords of at most 32 UTF-8 bytes"
        );
        for (name, value) in [
            ("ssid", &config.ssid),
            ("password", &config.password),
            ("agent_addr", &config.agent_addr),
        ] {
            assert!(
                !value.contains(['\r', '\n', '\0']),
                "wifi_config.json: {name} must not contain CR, LF or NUL"
            );
        }
        let endpoint: SocketAddrV4 = config.agent_addr.parse().unwrap_or_else(|_| {
            panic!("wifi_config.json: agent_addr must be an IPv4 address and port (a.b.c.d:port)")
        });
        assert!(
            endpoint.port() != 0,
            "wifi_config.json: agent_addr port must be 1-65535"
        );
        println!("cargo:rustc-env=WIFI_SSID={}", config.ssid);
        println!("cargo:rustc-env=WIFI_PASSWORD={}", config.password);
        println!("cargo:rustc-env=MICRO_ROS_AGENT_ADDR={endpoint}");
    }
}
