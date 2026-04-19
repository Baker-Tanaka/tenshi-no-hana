# Project Guidelines — 天使の鼻 (tenshi-no-hana)

## Overview

ウィスキー蒸留所巡回用 2WD ローバー。Baker link.dev (RP2040) + Embassy-rs (Rust no\_std) をメイン MCU とし、XIAO ESP32-C3 (esp-hosted-mcu) 経由の WiFi で Zenoh Router / ROS2 と通信する。

### ターゲット

| Target | Chip | Toolchain Target | 用途 |
|--------|------|-----------------|------|
| Baker link.dev | RP2040 (Cortex-M0+) | `thumbv6m-none-eabi` | メイン制御・センサー・モーター |
| XIAO ESP32-C3 | ESP32-C3 | (esp-hosted-mcu firmware) | WiFi コプロセッサ (SPI スレーブ) |

## Architecture

```
src/
├── main.rs          # エントリポイント (HAL direct / traffic light demo)
├── wifi_config.rs   # WiFi + Zenoh compile-time config
examples/
├── wifi_zenoh_chatter.rs   # WiFi → Zenoh → ROS2 /chatter pub/sub
├── embassy_*.rs             # Embassy async 基本サンプル
├── *.rs                     # HAL direct サンプル
external/
└── zenoh_ros2_nostd/        # no_std ROS2 通信ライブラリ
docs/
├── DESIGN.md                # 設計書・ロードマップ
└── schematics/              # 回路図 (SVG)
```

## Communication Stack

```
Application (Publisher / Subscription)
  └── zenoh-ros2-nostd (sdk → ros2 → session → transport → cdr)
        └── embassy-net (TcpSocket: Read + Write)
              └── embassy-net-esp-hosted (WiFi over SPI)
                    └── embassy-rp SPI0 (async) + GPIO control
                          └── Hardware: RP2040 ↔ ESP32-C3
```

## Code Style

- `#![no_std]` — ヒープ割り当て禁止
- `heapless` コレクション (`Vec<u8, N>`, `String<N>`)
- async: `embassy-executor`, `embassy-time`, `embassy-rp`
- ロギング: `defmt` (`info!`, `warn!`, `error!`) + `defmt-rtt`
- エラーハンドリング: `defmt::unwrap!()` または `.unwrap()` (no\_std では panic = probe-rs でキャッチ)
- Embassy タスクパターン: `#[embassy_executor::task]` で独立タスクに分離

## Language & Communication

- ユーザーへの応答は**日本語**
- コード中のコメント・ドキュメントは**英語** (`///`, `//!`)
- コミットメッセージは**英語**

## Build Commands

```sh
# 通常ビルド
cargo build --release

# Embassy サンプル
cargo build --no-default-features --features embassy --example embassy_blinky

# センサーサンプル
cargo build --no-default-features --features sensor --example sensor_read

# WiFi サンプル
cargo build --no-default-features --features embassy,wifi --example wifi_zenoh_chatter

# WiFi + センサーサンプル
cargo build --no-default-features --features wifi,sensor --example wifi_zenoh_sensors

# 書き込み (probe-rs)
cargo run --release

# UF2 変換
elf2uf2-rs target/thumbv6m-none-eabi/release/tenshi-no-hana target/tenshi-no-hana.uf2
```

## Hardware Pin Assignment (RP2040)

### SPI0 → ESP32-C3 (esp-hosted)
| RP2040 | Signal | ESP32-C3 |
|--------|--------|----------|
| GP16 | MISO | GPIO5 (D3) |
| GP17 | CS | GPIO10 (D10) |
| GP18 | SCK | GPIO6 (D4) |
| GP19 | MOSI | GPIO7 (D5) |
| GP15 | Handshake | GPIO3 (D1) |
| GP13 | Data Ready | GPIO4 (D2) |
| GP14 | Reset | GPIO21 (D6) |

### Sensors & Actuators
| RP2040 | Function | Device |
|--------|----------|--------|
| GP4/GP5 | I2C0 SDA/SCL | BME280 |
| GP26 | ADC0 | MQ-3B |
| GP10/GP11 | PWM | DRV8835 |
| GP2/GP3 | GPIO Trig/Echo | HC-SR04 |
| GP20-22 | LED | Status LEDs |

## Reference Implementations

- **WiFi テンプレート**: `external/zenoh_ros2_nostd/examples/bakerlink_wiz630io/`
  - `src/main.rs` — タスク構成パターン
  - `src/config.rs` — AppConfig / ZenohConfig
  - `build.rs` — config.json → env 注入
- **ESP32-C3 WiFi 参考**: `external/zenoh_ros2_nostd/examples/esp32c3_wifi/`
- **zenoh-ros2-nostd API**: `external/zenoh_ros2_nostd/src/sdk/` (NodeBuilder, Node, Publisher)

## Conventions

- Feature gates: `hal-rt` (default), `embassy`, `wifi`
- `wifi` feature は `embassy` を前提とする
- `wifi_config.json` は `.gitignore` に追加済み（認証情報を含むため）
- ESP32-C3 は SPI スレーブ専用。アプリコードは載せない
- RP2040 側の SPI0 ピン (GP14-19) は WIZ630io と同一レイアウト
