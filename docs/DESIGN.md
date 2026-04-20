# 天使の鼻 — 設計書 (DESIGN.md)

> **プロジェクト名**: 天使の鼻 (Angel's Nose)
> **最終更新**: 2026-04-19

---

## 1. プロジェクト概要

ウィスキー蒸留所・熟成庫内を自律巡回する 2WD ローバー。
空気中のエタノール蒸気（天使の分け前）、温湿度・気圧、樽内液面を計測し、
ROS2 経由でリアルタイム監視する。

**コアスタック**: Baker link.dev (RP2040) + Embassy-rs (Rust no\_std) + zenoh-ros2-nostd

---

## 2. システムアーキテクチャ

```text
┌───────────────────────────────────────────────────────────┐
│                Baker link.dev (RP2040)                    │
│                   SPI Master                              │
│                                                           │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │ MQ-3     │  │ BME280   │  │ DRV8835  │  │ HC-SR04  │ │
│  │ (ADC)    │  │ (I2C)    │  │ (PWM)    │  │ (GPIO)   │ │
│  │ GP26     │  │ GP4/GP5  │  │ GP10-11  │  │ GP2/GP3  │ │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘ │
│                                                           │
│  ┌──────────────────────────────────────────────────────┐ │
│  │  SPI0 (GP16-19) + Control (GP13-15)                  │ │
│  │  embassy-net-esp-hosted                               │ │
│  └──────────────────┬───────────────────────────────────┘ │
└─────────────────────┼─────────────────────────────────────┘
                      │ SPI Full-Duplex
                      │ + Handshake / Data Ready / Reset
┌─────────────────────┼─────────────────────────────────────┐
│  XIAO ESP32-C3      │                                     │
│  esp-hosted-mcu     ▼                                     │
│  (SPI Slave)  ── WiFi ──► AP                              │
└───────────────────────────────────────────────────────────┘
                      │
                      │ TCP/IP (WiFi)
                      ▼
┌───────────────────────────────────────────────────────────┐
│  Host PC / Docker                                         │
│                                                           │
│  ┌──────────────┐  ┌────────────────┐  ┌──────────────┐  │
│  │ Zenoh Router │  │ ROS2 Node      │  │ Web          │  │
│  │ :7447        │  │ rmw_zenoh_cpp  │  │ Dashboard    │  │
│  └──────────────┘  └────────────────┘  └──────────────┘  │
└───────────────────────────────────────────────────────────┘
```

---

## 3. ソフトウェアレイヤー

```text
┌─────────────────────────────────────────────────────┐
│  Application (examples/wifi_zenoh_chatter.rs)       │
│  Publisher<StringMsg> / Subscription<StringMsg>     │
├─────────────────────────────────────────────────────┤
│  zenoh-ros2-nostd   (external/zenoh_ros2_nostd)     │
│  ┌─────────────────────────────────────────────┐    │
│  │  sdk/  — NodeBuilder, Node, spin()          │    │
│  │  ros2/ — TopicKeyExpr, Publisher, Sub, QoS  │    │
│  │  session/ — Session, Reconnect              │    │
│  │  transport/ — Zenoh v9, Handshake, Codec    │    │
│  │  cdr/  — CDR LE serialization               │    │
│  └─────────────────────────────────────────────┘    │
├─────────────────────────────────────────────────────┤
│  embassy-net  (TCP/IP stack)                        │
│  └── TcpSocket  — Read + Write → NodeBuilder       │
├─────────────────────────────────────────────────────┤
│  embassy-net-esp-hosted  (WiFi over SPI)            │
│  └── EspHosted → NetDevice                          │
├─────────────────────────────────────────────────────┤
│  embassy-rp  (RP2040 HAL)                           │
│  └── SPI0 (async) + GPIO (Handshake/DR/Reset)       │
├─────────────────────────────────────────────────────┤
│  Hardware: Baker link.dev ↔ XIAO ESP32-C3 (SPI)    │
└─────────────────────────────────────────────────────┘
```

---

## 4. ハードウェア構成

### 4.1 BOM 要約

| 部品            | 品番                     | 価格目安 | 役割                           |
| --------------- | ------------------------ | -------- | ------------------------------ |
| 2WD シャーシ    | FT-DC-002                | ¥1,900   | 車体                           |
| Baker link. Dev | RP2040                   | ¥1,980   | メインMCU                      |
| XIAO ESP32-C3   | Seeed Studio             | ¥800     | WiFi コプロセッサ (esp-hosted) |
| DRV8835         | デュアルモータードライバ | ¥400     | モーター制御                   |
| MQ-3B           | エタノールセンサー       | ¥450     | 天使の分け前検知               |
| BME280          | AE-BME280 モジュール     | ¥1,080   | 温湿度・気圧                   |
| HC-SR04         | 超音波センサー           | ¥300     | 障害物回避                     |

### 4.2 RP2040 (Baker link.dev) ↔ XIAO ESP32-C3 SPI 接続

> WIZ630io と同一 SPI0 バスレイアウト (GP14-19) を再利用

| RP2040 GPIO     | ESP32-C3 GPIO | XIAO Pin | 信号名     | 用途           |
| --------------- | ------------- | -------- | ---------- | -------------- |
| GP16 (SPI0 RX)  | GPIO5         | D3       | SPI MISO   | データ入力     |
| GP17 (GPIO out) | GPIO10        | D10      | SPI CS     | チップセレクト |
| GP18 (SPI0 SCK) | GPIO6         | D4       | SPI CLK    | クロック       |
| GP19 (SPI0 TX)  | GPIO7         | D5       | SPI MOSI   | データ出力     |
| GP15 (GPIO in)  | GPIO3         | D1       | Handshake  | タイミング同期 |
| GP13 (GPIO in)  | GPIO4         | D2       | Data Ready | データ到着通知 |
| GP14 (GPIO out) | GPIO21        | D6       | Reset      | リセット制御   |
| 3V3 OUT         | 3V3           | —        | 電源       | 3.3V 給電      |
| GND             | GND           | —        | GND        | グランド       |

### 4.3 RP2040 その他ピンアサイン

| RP2040 GPIO    | 機能             | 接続先             |
| -------------- | ---------------- | ------------------ |
| GP4 (I2C0 SDA) | I2C データ       | BME280 SDA         |
| GP5 (I2C0 SCL) | I2C クロック     | BME280 SCL         |
| GP26 (ADC0)    | ADC 入力         | MQ-3B アナログ出力 |
| GP10           | PWM (左モーター) | DRV8835 AIN1       |
| GP11           | PWM (右モーター) | DRV8835 BIN1       |
| GP2            | GPIO (Trigger)   | HC-SR04 Trig       |
| GP3            | GPIO (Echo)      | HC-SR04 Echo       |
| GP20           | LED (赤)         | ステータス LED     |
| GP21           | LED (橙)         | ステータス LED     |
| GP22           | LED (緑)         | ステータス LED     |

### 4.4 注意事項

1. **ESP32-C3 の I2C ピン (D4/D5) は SPI で占有** — I2C センサーはすべて RP2040 側に接続
2. **ESP32-C3 の JTAG デバッグ不可** — GPIO4〜7 が SPI + 信号線で使用済み。USB シリアルでデバッグ
3. **GPIO9 (BOOT ボタン) は Data Ready に使用不可** — GPIO4 (D2) に変更済み
4. **ストラッピングピン (GPIO2/8/9) は未使用**

---

## 5. ESP32-C3 esp-hosted-mcu スレーブ準備

ESP32-C3 は **esp-hosted-mcu** ファームウェアを書き込み、SPI スレーブとして動作させる。
RP2040 から `embassy-net-esp-hosted` 経由で WiFi 接続を利用する。

### 5.1 ファームウェアビルド手順

```sh
# esp-hosted-mcu リポジトリをクローン
git clone --recursive https://github.com/espressif/esp-hosted-mcu.git
cd esp-hosted-mcu

# ESP-IDF 環境セットアップ
. ./esp-idf/export.sh

# ターゲットを ESP32-C3 に設定
idf.py set-target esp32c3

# ピン設定のカスタマイズ
idf.py menuconfig
```

### 5.2 menuconfig 設定

**Example Configuration → Bus Config → SPI Full-Duplex Configuration:**

| 設定項目          | 値     |
| ----------------- | ------ |
| SPI MOSI (GPIO)   | **7**  |
| SPI MISO (GPIO)   | **5**  |
| SPI CLK (GPIO)    | **6**  |
| SPI CS (GPIO)     | **10** |
| Handshake (GPIO)  | **3**  |
| Data Ready (GPIO) | **4**  |
| Reset pin (GPIO)  | **21** |

### 5.3 ビルド・フラッシュ

```sh
# ビルド
idf.py build

# XIAO ESP32-C3 を BOOT モードで USB 接続
# (BOOT ボタンを押しながら USB 接続、または BOOT + RESET)
idf.py -p /dev/ttyACM0 flash

# ログ確認 (オプション)
idf.py -p /dev/ttyACM0 monitor
```

フラッシュ後、ESP32-C3 は SPI スレーブとして起動し、RP2040 からの SPI コマンドを待機する。

---

## 6. 開発ロードマップ

### Phase 1: WiFi 通信（最優先）

ESP32-C3 (esp-hosted) 経由で Zenoh Router に TCP 接続し、`/chatter` トピックを pub/sub する。

| ステップ | 内容                                              | 成果物                           |
| -------- | ------------------------------------------------- | -------------------------------- |
| 1-1      | ESP32-C3 に esp-hosted-mcu ファームウェア書き込み | ESP32-C3 SPI スレーブ起動        |
| 1-2      | Cargo.toml に WiFi 依存追加                       | `wifi` feature flag              |
| 1-3      | WiFi 通信サンプル作成                             | `examples/wifi_zenoh_chatter.rs` |
| 1-4      | 動作確認 (Docker + verify_sub.py)                 | `/chatter` メッセージ受信        |

**検証基準**:
- `cargo build --no-default-features --features embassy,wifi --example wifi_zenoh_chatter` 成功
- RTT ログ: DHCP 取得 → TCP 接続 → Zenoh ハンドシェイク成功
- `verify_sub.py` で `/chatter` メッセージ受信

### Phase 2: センサー統合

BME280 (I2C) と MQ-3B (ADC) のセンサーデータを取得し、ROS2 トピックとしてパブリッシュする。

| ステップ | 内容                             | 成果物                             | 状態 |
| -------- | -------------------------------- | ---------------------------------- | ---- |
| 2-1      | BME280 I2C ドライバ統合          | `bme280` crate (sync/blocking I2C) | ✅    |
| 2-2      | MQ-3 ADC 読み取り                | embassy-rp ADC (GP26)              | ✅    |
| 2-3      | スタンドアロンセンサー読み取り   | `examples/sensor_read.rs`          | ✅    |
| 2-4      | センサーデータ ROS2 パブリッシュ | `examples/wifi_zenoh_sensors.rs`   | ✅    |

**ROS2 トピック**:

| トピック                  | 型                 | 内容              |
| ------------------------- | ------------------ | ----------------- |
| `/angel_nose/temperature` | `std_msgs/Float32` | BME280 温度 [°C]  |
| `/angel_nose/humidity`    | `std_msgs/Float32` | BME280 湿度 [%]   |
| `/angel_nose/pressure`    | `std_msgs/Float32` | BME280 気圧 [hPa] |
| `/angel_nose/ethanol`     | `std_msgs/Float32` | MQ-3B 電圧 [V]    |

**ビルドコマンド**:
```sh
# センサー単体テスト
cargo build --no-default-features --features sensor --example sensor_read

# WiFi + センサー + Zenoh パブリッシュ
cargo build --no-default-features --features wifi,sensor --example wifi_zenoh_sensors
```

**検証基準**:
- `cargo check --features sensor --example sensor_read` 成功
- `cargo check --features wifi,sensor --example wifi_zenoh_sensors` 成功
- RTT ログ: BME280 温湿度気圧 + MQ-3B ADC 値の周期的出力
- ROS2 側: `ros2 topic echo /angel_nose/temperature std_msgs/msg/Float32` でデータ受信

### Phase 3: モーター制御

| ステップ | 内容                      | 成果物                    |
| -------- | ------------------------- | ------------------------- |
| 3-1      | DRV8835 PWM 制御          | 2WD 前後進・旋回          |
| 3-2      | `/cmd_vel` サブスクライブ | Twist → モーター PWM 変換 |

### Phase 4: 障害物回避・ナビゲーション

| ステップ | 内容                   | 成果物         |
| -------- | ---------------------- | -------------- |
| 4-1      | HC-SR04 超音波測距     | 前方障害物距離 |
| 4-2      | IMU (6 軸) odom 計算   | 姿勢推定       |
| 4-3      | 簡易障害物回避ロジック | 自律走行       |

### Phase 5: 統合・自律巡回

| ステップ | 内容                   | 成果物                            |
| -------- | ---------------------- | --------------------------------- |
| 5-1      | 複数樽巡回マッピング   | ArUco/AprilTag 位置推定           |
| 5-2      | 液面磁界測定           | 浮遊ネオジム磁石 + ホールセンサー |
| 5-3      | 天使の分け前蒸発量推定 | エタノール + 液面データ統合       |
| 5-4      | Web ダッシュボード     | Zenoh → WebSocket → ブラウザ      |

---

## 7. WiFi 通信の実装パターン

`external/zenoh_ros2_nostd/examples/bakerlink_wiz630io/` をテンプレートとし、
`embassy-net-wiznet` を `embassy-net-esp-hosted` に置換する。

### 7.1 タスク構成

```text
main()
  ├── esp_hosted_task  — SPI 通信ドライバループ
  ├── net_task         — embassy-net パケット I/O ループ
  ├── zenoh_task       — DHCP → TCP → Zenoh → Node::spin()
  └── app_task         — Publisher.send() / Subscription.try_recv()
```

### 7.2 トランスポート抽象化

zenoh-ros2-nostd の `NodeBuilder::build(transport)` は `T: Read + Write` を受け取る。
TCP ソケットをそのまま渡せるため、WiFi/Ethernet の違いはネットワークデバイス層のみ。

```rust
// WiFi (esp-hosted) でも Ethernet (WIZ630io) でも同じ:
let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
socket.connect(router_endpoint).await?;
let mut node = NodeBuilder::new("my_node")
    .zid(cfg.zenoh.session.zid)
    .domain_id(0)
    .build(socket)
    .await?;
node.spin_and_backoff(&mut reconnect).await;
```

---

## 8. ビルド・デプロイ

### 8.1 開発環境

- **Dev Container**: Rust 1.94.1 + thumbv6m-none-eabi
- **デバッガ**: probe-rs (Baker link.dev 内蔵 CMSIS-DAP)
- **ROS2**: Docker (rmw_zenoh_cpp + zenohd)

### 8.2 ビルドコマンド

```sh
# 通常ビルド (HAL + defmt)
cargo build --release

# Embassy サンプル
cargo build --no-default-features --features embassy --example embassy_blinky

# WiFi サンプル (Phase 1)
cargo build --no-default-features --features embassy,wifi --example wifi_zenoh_chatter

# 書き込み (probe-rs)
cargo run --release

# UF2 変換 (USB ブートローダ書き込み用)
elf2uf2-rs target/thumbv6m-none-eabi/release/tenshi-no-hana target/tenshi-no-hana.uf2
```

### 8.3 Docker (ROS2 + Zenoh Router)

```sh
cd <project_root>
docker compose up -d

# 動作確認
python3 verify_sub.py

# または ROS2 CLI
ros2 topic echo /chatter std_msgs/msg/String
```

---

## 9. 関連リンク

- [Baker link.dev](https://www.baker-link.com/)
- [embassy-net-esp-hosted](https://github.com/embassy-rs/embassy/tree/embassy-net-esp-hosted-v0.3.0/embassy-net-esp-hosted)
- [esp-hosted-mcu](https://github.com/espressif/esp-hosted-mcu)
- [zenoh-ros2-nostd](external/zenoh_ros2_nostd/)
- [Seeed Studio XIAO ESP32C3](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)
