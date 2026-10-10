use std::env;
use std::fs;
use std::path::PathBuf;

use micro_xrce_dds_codegen::{generate, Capacities};

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=wifi_config.json");
    println!("cargo:rerun-if-changed=msgs");
    println!("cargo:rerun-if-changed=external/micro_xrce_dds_rs/codegen");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rustc-link-search={}", manifest_dir.display());

    if env::var("CARGO_FEATURE_WIFI").is_ok() {
        let json = fs::read_to_string(manifest_dir.join("wifi_config.json")).unwrap_or_else(|_| {
            panic!(
                "\n\n\
                 wifi_config.json not found.\n\
                 Copy wifi_config.json.example to wifi_config.json and fill in your credentials.\n\
                 This file is git-ignored and will not be committed.\n"
            )
        });

        let ssid = extract_str(&json, "ssid")
            .unwrap_or_else(|| panic!("wifi_config.json: missing or invalid \"ssid\" field"));
        let password = extract_str(&json, "password")
            .unwrap_or_else(|| panic!("wifi_config.json: missing or invalid \"password\" field"));
        let agent_addr = extract_str(&json, "agent_addr").unwrap_or_else(|| {
            panic!(
                "wifi_config.json: missing \"agent_addr\" field.\n\
                 Example: \"agent_addr\": \"192.168.1.1:8888\""
            )
        });

        println!("cargo:rustc-env=WIFI_SSID={ssid}");
        println!("cargo:rustc-env=WIFI_PASSWORD={password}");
        println!("cargo:rustc-env=MICRO_ROS_AGENT_ADDR={agent_addr}");
    }

    let output_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ros_msgs.rs");
    generate(
        &[manifest_dir.join("msgs")],
        &output_path,
        Capacities::default(),
    )
    .unwrap_or_else(|error| panic!("ROS message generation failed: {error}"));
}

/// Extract a string value from the simple flat JSON configuration.
fn extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = json.find(needle.as_str())? + needle.len();
    let after_key = json[start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let inner = after_colon.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(&inner[..end])
}
