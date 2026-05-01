# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

天使の鼻 (Angel's Nose) — ウィスキー蒸留所巡回用 2WD ローバー。Baker link. Dev (RP2040, Cortex-M0+) を主 MCU とし、XIAO ESP32-C3 を SPI 接続の WiFi コプロセッサ (esp-hosted-mcu) として使用。Embassy-rs (Rust `no_std`) で実装し、Zenoh 経由で ROS2 と通信する。

| Target          | Chip                | Toolchain Target                              |
| --------------- | ------------------- | --------------------------------------------- |
| Baker link. Dev | RP2040 (Cortex-M0+) | `thumbv6m-none-eabi`                          |
| XIAO ESP32-C3   | ESP32-C3            | esp-hosted-mcu ファームウェア（SPI スレーブ） |

Toolchain: stable `1.94.1`（`rust-toolchain.toml`）。nightly 不要。

## Build Commands

```sh
# デフォルト (Embassy LED デモ)
cargo build --release

# センサーサンプル
cargo build --no-default-features --features sensor --example sensor_read

# WiFi サンプル (wifi feature は embassy を含む)
cargo build --no-default-features --features wifi --example wifi_zenoh_chatter

# WiFi + センサー
cargo build --no-default-features --features wifi,sensor --example wifi_zenoh_sensors

# 書き込み (probe-rs / Baker link. Dev 内蔵 CMSIS-DAP)
cargo run --release

# UF2 変換 (USB ブートローダ書き込み)
elf2uf2-rs target/thumbv6m-none-eabi/release/tenshi-no-hana target/tenshi-no-hana.uf2

# Docker (ROS2 + Zenoh Router)
docker compose up -d
```

WiFi ビルドには `wifi_config.json` が必要（`.gitignore` 済み）。`wifi_config.json.example` を参照して作成する。

## Architecture

```
src/
  main.rs          # Embassy multi-task LED demo (エントリポイント)
  wifi_config.rs   # AppConfig: wifi_config.json → build.rs → env vars → compile-time定数
examples/
  wifi_zenoh_chatter.rs   # WiFi → Zenoh → ROS2 /chatter pub/sub (最もシンプルな WiFi 実装)
  wifi_zenoh_sensors.rs   # WiFi → Zenoh → ROS2 センサーデータ (BME280 + MQ-3B)
  sensor_read.rs           # BME280 + MQ-3B スタンドアロン読み取り
  esp_hosted_spi_test.rs   # SPI 通信テスト
external/
  embassy/                 # git submodule (oktima fork, upstream-esp-hosted-mcu branch)
    embassy-net-esp-hosted-mcu/  # WiFi ドライバー (MCU variant, SpiInterface ベース)
    embassy-net-driver/          # [patch.crates-io] でオーバーライド
    embassy-net-driver-channel/  # [patch.crates-io] でオーバーライド
    embassy-time-driver/         # [patch.crates-io] でオーバーライド
  zenoh_ros2_nostd/        # git submodule — no_std ROS2 通信ライブラリ
```

### Communication Stack

```
Application (Publisher / Subscription)
  └── zenoh-ros2-nostd (NodeBuilder → Node → spin())
        └── embassy-net v0.7.x (TcpSocket: Read + Write)
              └── embassy-net-esp-hosted-mcu (WiFi over SPI)
                    └── embassy-rp SPI0 (async) + GPIO (Handshake/DR/Reset)
                          └── RP2040 ↔ ESP32-C3 (SPI Full-Duplex)
```

### Task Structure (WiFi examples)

```
main()
  ├── esp_hosted_task  — SPI 通信ドライバループ (TX/RX buf は StaticCell で確保)
  ├── net_task         — embassy-net パケット I/O ループ
  ├── zenoh_task       — DHCP → TCP → Zenoh → Node::spin()
  └── app_task         — Publisher.send() / Subscription.try_recv()
```

### Reference Implementation

新しい WiFi 実装を書くときのテンプレート:
- `external/zenoh_ros2_nostd/examples/bakerlink_wiz630io/` (WIZ630io 版、構造が同じ)
- `examples/wifi_zenoh_chatter.rs` (ESP-hosted 版、最もシンプル)

## Code Style

- `#![no_std]` — ヒープ割り当て禁止
- コレクション: `heapless::Vec<u8, N>`, `heapless::String<N>`
- async: `embassy-executor`, `embassy-time`, `embassy-rp`
- ロギング: `defmt` (`info!`, `warn!`, `error!`) + RTT 転送
- タスク定義: `#[embassy_executor::task]`、スポーン: `spawner.spawn(task_fn(args).unwrap())`
- エラー: `defmt::unwrap!()` または `.unwrap()`（no_std では panic → probe-rs がキャッチ）
- コード中のコメント・ドキュメントは**英語**、ユーザーへの応答は**日本語**、コミットメッセージは**英語**

## Feature Flags

| Feature             | 内容                                                      |
| ------------------- | --------------------------------------------------------- |
| `embassy` (default) | Embassy executor/time/rp/sync                             |
| `hal-rt`            | rp2040-hal runtime                                        |
| `sensor`            | embassy + BME280                                          |
| `wifi`              | embassy + embassy-net + esp-hosted-mcu + zenoh-ros2-nostd |

`--features wifi` のみで embassy も有効になる（`embassy,wifi` は冗長）。

## Known Pitfalls

### `[profile.dev.package."*"] opt-level = 2` は削除禁止
`CtrlMsg::decode()` が 64 個の oneof arm を持ち、最適化なしでは ~96 KB のスタックを消費してクラッシュする。Embassy async poll チェーン全体も同様。

### embassy-net は v0.7.x に固定
v0.9 は `embedded-io-async = "0.7"` を要求するが、`zenoh-ros2-nostd` は `"0.6"` を使用。`[patch.crates-io]` には `embassy-net-driver` / `embassy-net-driver-channel` / `embassy-time-driver` のみを submodule から patch する（`embassy-net-esp-hosted-mcu` は path dep なので patch 不要）。

### SPI 設定
- SPI Mode: **Mode 3** (`Polarity::IdleHigh` + `Phase::CaptureOnSecondTransition`)
- Handshake (GP15) と DataReady (GP13) は **`Pull::Down`** — `Pull::Up` にすると常時 HIGH になり通信不能

### defmt::Format 非実装型
以下は手動 match が必要:
- `embassy_net::tcp::ConnectError`
- `micropb::DecodeError<Infallible>`

### CtrlMsg スタック使用量
`CtrlMsg` 1 個あたり ~1376 bytes。Cortex-M0+ で同時に 2 個以上スタックに置かない。`decode_from_bytes()` 前に既存フィールドをクリアすること（micropb はマージセマンティクス）。

## Hardware Pin Assignment (RP2040)

### SPI0 → ESP32-C3 (esp-hosted-mcu)
| RP2040          | Signal     | ESP32-C3 GPIO | XIAO Pin |
| --------------- | ---------- | ------------- | -------- |
| GP16 (SPI0 RX)  | MISO       | GPIO5         | D3       |
| GP17 (GPIO out) | CS         | GPIO10        | D10      |
| GP18 (SPI0 SCK) | SCK        | GPIO6         | D4       |
| GP19 (SPI0 TX)  | MOSI       | GPIO7         | D5       |
| GP15 (GPIO in)  | Handshake  | GPIO3         | D1       |
| GP13 (GPIO in)  | Data Ready | GPIO4         | D2       |
| GP14 (GPIO out) | Reset      | GPIO21        | D6       |

### Sensors & Actuators
| RP2040         | Function       | Device             |
| -------------- | -------------- | ------------------ |
| GP4/GP5        | I2C0 SDA/SCL   | BME280             |
| GP26           | ADC0           | MQ-3B (エタノール) |
| GP10/GP11      | PWM            | DRV8835 (モーター) |
| GP2/GP3        | GPIO Trig/Echo | HC-SR04 (超音波)   |
| GP20/GP21/GP22 | GPIO           | ステータス LED     |

## Supplementary Instruction Files

- [examples/CLAUDE.md](examples/CLAUDE.md) — esp-hosted-mcu 詳細ノート（WiFi examples / `src/wifi_config.rs` 編集時に自動適用）
- [.claude/serialization.md](.claude/serialization.md) — `no_std` シリアライゼーション選択ガイド（`@.claude/serialization.md` で参照可能）

## Memory Usage Analysis

RP2040 メモリ制約: **FLASH 2048KB** (BOOT2 256B を除く実質 ~2047.75KB), **RAM 264KB**

### Cargo alias（短縮形）

```sh
cargo size-default   # default feature (Embassy LED デモ)
cargo size-wifi      # wifi_zenoh_chatter example
cargo size-sensor    # sensor_read example
cargo size-all       # wifi_zenoh_sensors example (最大構成)
cargo nm-top         # ROM 使用量上位シンボル一覧
```

### フルコマンド（feature/example を変えたい場合）

```sh
# Berkeley format: text / data / bss / dec / hex の列を表示
cargo size --release -- -B
cargo size --no-default-features --features wifi --example wifi_zenoh_chatter --release -- -B

# sysv format: セクション別詳細
cargo size --release -- -A

# シンボル別サイズ上位 30 件（ROM 肥大化の原因調査）
cargo nm --release -- --print-size --size-sort --radix=d | grep ' [tT] ' | tail -30

# セクションヘッダー確認
cargo objdump --release -- --section-headers
```

### 出力の読み方

| 列     | 意味                    | 対応メモリ                              |
| ------ | ----------------------- | --------------------------------------- |
| `text` | コード + read-only data | FLASH                                   |
| `data` | 初期値あり変数          | FLASH (初期値格納) + RAM (実行時コピー) |
| `bss`  | ゼロ初期化変数          | RAM のみ                                |

**ROM 使用量 = `text + data`** （上限 ~2,096,896 B）  
**RAM 使用量 = `data + bss`** （上限 270,336 B = 264 KB）

> flip-link 使用時は `.bss`/`.data` が RAM 末尾に配置され、スタックは下位アドレスから伸長する。
> RAM がオーバーフローしてもスタック破壊より先にリンクエラーになる。

## Submodule Setup

```sh
git submodule update --init --recursive external/embassy external/zenoh_ros2_nostd
```

## esp-hosted-mcu API Quick Reference

```rust
use embassy_net_esp_hosted_mcu::{
    BufferType, EspConfig, MAX_SPI_BUFFER_SIZE, Runner as EspRunner, SpiInterface, State,
};

// 初期化
let (net_device, mut control, esp_runner) =
    embassy_net_esp_hosted_mcu::new(esp_state, spi_iface, reset, None).await;

// Runner タスク (StaticCell で 'static バッファを確保)
#[embassy_executor::task]
async fn esp_hosted_task(runner: MyEspRunner) {
    static TX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; MAX_SPI_BUFFER_SIZE]> = StaticCell::new();
    runner.run(TX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE]), RX_BUF.init([0u8; MAX_SPI_BUFFER_SIZE])).await
}

// WiFi 接続
control.init(EspConfig { static_rx_buf_num: 10, dynamic_rx_buf_num: 32, tx_buf_type: BufferType::Dynamic,
    static_tx_buf_num: 0, dynamic_tx_buf_num: 32, rx_mgmt_buf_type: BufferType::Dynamic, rx_mgmt_buf_num: 20 }).await;
let connected = control.connect(ssid, password).await; // Result<bool, Error>
```

esp-hosted-**fg** (旧 crates.io v0.3) と esp-hosted-**mcu** (現行、path dep) は API が異なる。`runner.run()` に外部バッファが必要、`control.init()` に `EspConfig` が必須、`InterfaceType::Sta = 1`（fg では 0）。
