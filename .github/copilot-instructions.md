# Project Guidelines — 天使の鼻 (tenshi-no-hana)

## Overview

ウィスキー蒸留所巡回用 2WD ローバー。Baker link.dev (RP2040) + Embassy-rs (Rust no_std) をメイン MCU とし、XIAO ESP32-C3 (esp-hosted-mcu) 経由の WiFi で Zenoh Router / ROS2 と通信する。詳細は [docs/DESIGN.md](../docs/DESIGN.md) を参照。

### ターゲット

| Target | Chip | Toolchain Target | 用途 |
|--------|------|-----------------|------|
| Baker link.dev | RP2040 (Cortex-M0+) | `thumbv6m-none-eabi` | メイン制御・センサー・モーター |
| XIAO ESP32-C3 | ESP32-C3 | (esp-hosted-mcu firmware) | WiFi コプロセッサ (SPI スレーブ) |

Toolchain: stable `1.94.1`（`rust-toolchain.toml` 参照）。nightly 不要。

## Architecture

```
src/
 main.rs          # エントリポイント (Embassy multi-task LED demo)
 wifi_config.rs   # WiFi + Zenoh compile-time config (env vars from build.rs)
examples/
 wifi_zenoh_chatter.rs   # WiFi → Zenoh → ROS2 /chatter pub/sub
 wifi_zenoh_sensors.rs   # WiFi → Zenoh → ROS2 センサーデータ (BME280 + MQ-3B)
 embassy_*.rs             # Embassy async 基本サンプル
 *.rs                     # HAL direct サンプル
external/
 embassy/                 # git submodule (oktima fork, upstream-esp-hosted-mcu branch)
   ├── embassy-net-esp-hosted-mcu/  # SpiInterface ベースの WiFi ドライバー (MCU variant)
   ├── embassy-net-driver/          # patched from submodule
   ├── embassy-net-driver-channel/  # patched from submodule
   └── embassy-time-driver/         # patched from submodule
 zenoh_ros2_nostd/        # git submodule — no_std ROS2 通信ライブラリ
docs/
 DESIGN.md                # 設計書・ロードマップ
 schematics/              # 回路図 (SVG)
```

## Communication Stack

```
Application (Publisher / Subscription)
  └── zenoh-ros2-nostd (sdk → ros2 → session → transport → cdr)
        └── embassy-net v0.7.1 (TcpSocket: Read + Write)  ← v0.9 不可 (下記参照)
              └── embassy-net-esp-hosted-mcu v0.1 (WiFi over SPI, SpiInterface)
                    └── embassy-rp SPI0 (async) + GPIO control
                          └── Hardware: RP2040 ↔ ESP32-C3
```

## Code Style

- `#![no_std]` — ヒープ割り当て禁止
- `heapless` コレクション (`Vec<u8, N>`, `String<N>`)
- async: `embassy-executor`, `embassy-time`, `embassy-rp`
- ロギング: `defmt` (`info!`, `warn!`, `error!`) + `defmt-rtt`
- エラーハンドリング: `defmt::unwrap!()` または `.unwrap()` (no_std では panic = probe-rs でキャッチ)
- Embassy タスクパターン: `#[embassy_executor::task]` で独立タスクに分離
- タスクスポーン: `spawner.spawn(task_fn(args).unwrap())` パターン（`task_fn` は `Result<SpawnToken, _>` を返す）

## Language & Communication

- ユーザーへの応答は**日本語**
- コード中のコメント・ドキュメントは**英語** (`///`, `//!`)
- コミットメッセージは**英語**

## Build Commands

```sh
# デフォルト (Embassy LED デモ)
cargo build --release

# Embassy サンプル
cargo build --no-default-features --features embassy --example embassy_blinky

# センサーサンプル
cargo build --no-default-features --features sensor --example sensor_read

# WiFi サンプル  (wifi feature は embassy を含む)
cargo build --no-default-features --features wifi --example wifi_zenoh_chatter

# WiFi + センサーサンプル
cargo build --no-default-features --features wifi,sensor --example wifi_zenoh_sensors

# 書き込み (probe-rs)
cargo run --release

# UF2 変換
elf2uf2-rs target/thumbv6m-none-eabi/release/tenshi-no-hana target/tenshi-no-hana.uf2
```

WiFi サンプルを動かすには `wifi_config.json` が必要（`.gitignore` 済み）。`wifi_config.json.example` を参照。

## Hardware Pin Assignment (RP2040)

### SPI0 → ESP32-C3 (esp-hosted-mcu)
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
- **プロジェクト内 WiFi 実装**: `examples/wifi_zenoh_chatter.rs`（最もシンプルな実装）
- **zenoh-ros2-nostd API**: `external/zenoh_ros2_nostd/src/sdk/` (NodeBuilder, Node, Publisher)

## Conventions

- Feature gates: `embassy` (default), `hal-rt`, `sensor`, `wifi`
- `wifi` feature は `embassy` を前提とする（`--features wifi` のみで OK、`embassy,wifi` は冗長）
- `wifi_config.json` は `.gitignore` に追加済み（認証情報を含むため）
- ESP32-C3 は SPI スレーブ専用。アプリコードは載せない
- RP2040 側の SPI0 ピン (GP14-19) は WIZ630io と同一レイアウト

## Known Pitfalls

### 必須: 依存クレートの最適化
`[profile.dev.package."*"] opt-level = 2` は**削除禁止**。`CtrlMsg::decode()` は 64 個の oneof match arm を持ち、opt-level=0 では ~96 KB のスタックを消費してクラッシュする。Embassy の async poll チェーン全体も同様。

### embassy-net バージョン固定
`embassy-net` は **v0.7.x** に固定すること。v0.9 は `embedded-io-async = "0.7"` を要求するが、`zenoh-ros2-nostd` は `embedded-io-async = "0.6"` を使用しており semver 非互換。`[patch.crates-io]` には `embassy-net-driver` / `embassy-net-driver-channel` / `embassy-time-driver` のみをサブモジュールから patch する。

### SPI の設定
- SPI Mode: **Mode 3** (`Polarity::IdleHigh` + `Phase::CaptureOnSecondTransition`)
- Handshake (GP15) と DataReady (GP13) は **`Pull::Down`** — `Pull::Up` にすると常時 HIGH になり通信不能

### defmt::Format 非実装型
-1 `defmt::Format` を実装していないため variants を手動 match する:
- `embassy_net::tcp::ConnectError`
- `micropb::DecodeError<Infallible>`

### CtrlMsg のスタック使用量
`CtrlMsg` は 1 個あたり ~1376 bytes。Cortex-M0+ で同時に 2 個以上スタックに置かない。decode 前に既存 msg のフィールドをクリアしてから `decode_from_bytes()` を呼ぶ（micropb はマージセマンティクスのため）。

## embassy-net-esp-hosted-mcu API

### esp-hosted-fg と esp-hosted-mcu の違い
| | `embassy-net-esp-hosted` (FG, 旧) | `embassy-net-esp-hosted-mcu` (MCU, 現行) |
|---|---|---|
| crates.io | v0.3 | path dep (サブモジュール) |
| proto | `CtrlMsg` | `Rpc` (RPC形式) |
| `new()` | `(state, iface, reset)` | `(state, iface, reset, events_notifier)` |
| `runner.run()` | 引数なし (内部バッファ) | `(tx_buf, rx_buf)` (外部バッファ必要) |
| `control.init()` | 引数なし | `(EspConfig)` 必須 |
| `control.connect()` | `Result<(), Error>` | `Result<bool, Error>` (`bool` = link up) |
| InterfaceType::Sta | 0 | 1 |
| InterfaceType::Serial | 2 | 3 |

### 使い方テンプレート

```rust
use embassy_net_esp_hosted_mcu::{
    self, BufferType, EspConfig, MAX_SPI_BUFFER_SIZE, NetDriver,
    Runner as EspRunner, SpiInterface, State,
};

// driver 初期化
let esp_state = ESP_STATE.init(State::new());
let (net_device, mut control, esp_runner) =
    embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;
spawner.spawn(esp_hosted_task(esp_runner).unwrap());

// WiFi 接続
let esp_config = EspConfig {
    static_rx_buf_num: 10,
    dynamic_rx_buf_num: 32,
    tx_buf_type: BufferType::Dynamic,
    static_tx_buf_num: 0,
    dynamic_tx_buf_num: 32,
    rx_mgmt_buf_type: BufferType::Dynamic,
    rx_mgmt_buf_num: 20,
};
defmt::unwrap!(control.init(esp_config).await);
let connected = defmt::unwrap!(control.connect(ssid, password).await);
defmt::assert!(connected, "WiFi association failed");

// Runner タスク (StaticCell で 'static バッファを確保)
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
```

### Cargo.toml 設定 (embassy-net-esp-hosted-mcu)
```toml
[dependencies]
embassy-net-esp-hosted-mcu = { path = "external/embassy/embassy-net-esp-hosted-mcu",
                               features = ["defmt"], optional = true }

[patch.crates-io]
# embassy-net-esp-hosted-mcu は path dep なので patch 不要
embassy-net-driver = { path = "external/embassy/embassy-net-driver" }
embassy-net-driver-channel = { path = "external/embassy/embassy-net-driver-channel" }
embassy-time-driver = { path = "external/embassy/embassy-time-driver" }
```

### EspHostedEvents (events_notifier を使う場合)
`new(..., Some(&EVENTS_SIGNAL))` を渡すと heartbeat 失効・切断時に `EspHostedEvents::Deadline` / `Disconnected` を通知できる。通常は `None` でよい（panic で代替）。
