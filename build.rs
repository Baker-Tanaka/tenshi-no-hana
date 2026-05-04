use std::fs;

fn main() {
    // Re-run this build script whenever memory.x or the config file changes.
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=wifi_config.json");

    // Tell the linker where to find memory.x
    println!(
        "cargo:rustc-link-search={}",
        std::env::var("CARGO_MANIFEST_DIR").unwrap()
    );

    // WiFi config: only inject env vars when the wifi feature is enabled.
    if std::env::var("CARGO_FEATURE_WIFI").is_ok() {
        let json = fs::read_to_string("wifi_config.json").unwrap_or_else(|_| {
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
}

/// Extract the string value for `key` from a simple flat JSON object.
/// Handles `"key": "value"` patterns; no support for escape sequences beyond `\"`.
fn extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = json.find(needle.as_str())? + needle.len();
    let after_key = json[start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let inner = after_colon.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(&inner[..end])
}
