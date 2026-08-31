# 天使の鼻 — 設計書 (DESIGN.md)

> **プロジェクト名**: 天使の鼻 (Angel's Nose)
> **最終更新**: 2026-04-19

---

## 1. プロジェクト概要

ウィスキー蒸留所・熟成庫内を自律巡回する 2WD ローバー。
空気中のエタノール蒸気（天使の分け前）と樽内液面を計測し、
ROS2 経由でリアルタイム監視する。

**コアスタック**: Baker link. Dev Rev.1 (RP2040) + Embassy-rs (Rust no\_std) + micro_xrce_dds_rs / micro-ROS Agent

---

## 2. システムアーキテクチャ

```text
┌───────────────────────────────────────────────────────────┐
│                Baker link. Dev (RP2040)                    │
│                   SPI Master                              │
│                                                           │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐ │
│  │ MQ-3B    │  │ ICM-20602│  │ DRV8835  │  │ HC-SR04  │ │
│  │ (ADC)    │  │ (I2C)    │  │ (PH/EN)  │  │ (GPIO)   │ │
│  │ GP26     │  │ GP4/GP5  │  │ GP9-12   │  │ GP2/GP3  │ │
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
│  │ micro-ROS    │  │ ROS2 Node      │  │ Web          │  │
│  │ Agent :8888  │  │ rmw_fastrtps_cpp │  │ Dashboard    │  │
│  └──────────────┘  └────────────────┘  └──────────────┘  │
└───────────────────────────────────────────────────────────┘
```

---

## 2.1 ROS2 メッセージ・コードジェネレーション

`build.rs` が起動時に `msgs/` 以下の vendored `.msg` ファイルを
[`roslibrust_codegen`](https://crates.io/crates/roslibrust_codegen) (v0.20) で
パースし、`$OUT_DIR/ros_msgs.rs` に no_std 互換の Rust 型を生成する。
`src/ros_msgs.rs` がそれを `include!` し、example からは
`#[path = "../src/ros_msgs.rs"]` で参照する。

```text
msgs/                          # vendored .msg + package.xml
  ├── builtin_interfaces/      # Time, Duration
  ├── std_msgs/                # Header
  ├── geometry_msgs/           # Vector3, Point, Quaternion, Pose, Twist,
  │                            # PoseWithCovariance, TwistWithCovariance,
  │                            # Transform, TransformStamped
  ├── sensor_msgs/             # JointState
  ├── nav_msgs/                # Odometry
  └── tf2_msgs/                # TFMessage
       │
       ▼ build.rs
$OUT_DIR/ros_msgs.rs            # pub mod nav_msgs { pub struct Odometry { ... }
                                #   impl Message for Odometry { ... } }
       │
       ▼ include!()
src/ros_msgs.rs
       │
       ▼ #[path]
examples/wifi_microros_rover.rs # use ros_msgs::nav_msgs::Odometry;
```

設計方針:
- **`roslibrust_codegen` の AST のみ利用**。標準の token emitter は std 型 (`String`, `Vec`) を出すので、AST (`ParsedMessageFile { fields, constants }`) を独自 emitter で歩いて no_std 出力に変換する。
- 可変長フィールドは `heapless::String<32>` / `heapless::Vec<T, 8>` (デフォルト)。`.msg` 側で `string<=64 frame_id` / `T[<=4]` と書けば個別オーバーライド可能。
- 各メッセージ型は `Clone + Debug + Default` と `micro_xrce_dds_rs::Message` (DDS type name + serialize/deserialize) を実装。サブメッセージとして埋め込み用の `serialize_inline` / `deserialize_inline` も emit。
- `MAX_SERIALIZED_SIZE` は build.rs が依存グラフをトポロジカル順に解決して bottom-up 計算。
- 既存手書きの `external/micro_xrce_dds_rs/src/ros2/msg/*.rs` (Float32, Twist, Range, Imu) はそのまま維持し、生成型と共存。

新メッセージを追加するには `msgs/<package>/msg/Foo.msg` を置くだけ。`build.rs` の `cargo:rerun-if-changed=msgs` が自動で再生成をトリガする。

`FRAME_BUF_SIZE` (`external/micro_xrce_dds_rs/src/rt/inner.rs`) は `nav_msgs/Odometry` (~770 B) を 1 フレームで送るため 512 → **1024** にバンプ済み。

## 3. ソフトウェアレイヤー

```text
┌─────────────────────────────────────────────────────┐
│  Application (examples/wifi_microros_sensors.rs)    │
│  Publisher<StringMsg> / Subscription<StringMsg>     │
├─────────────────────────────────────────────────────┤
│  micro_xrce_dds_rs   (external/micro_xrce_dds_rs)   │
│  ┌─────────────────────────────────────────────┐    │
│  │  sdk/  — NodeBuilder, Node, spin()          │    │
│  │  ros2/ — TopicKeyExpr, Publisher, Sub, QoS  │    │
│  │  session/ — Session, Reconnect              │    │
│  │  transport/ — XRCE-DDS, TCP, fragmentation   │    │
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
│  Hardware: Baker link. Dev ↔ XIAO ESP32-C3 (SPI)    │
└─────────────────────────────────────────────────────┘
```

---

## 4. ハードウェア構成

### 4.1 BOM 要約

| 部品            | 品番                                     | 価格目安 | 役割                            |
| --------------- | ---------------------------------------- | -------- | ------------------------------- |
| 2WD シャーシ    | 2WD Mini Smart Robot Mobile Platform Kit | ¥1,900   | 車体（FA-130 系 3V モーター×2） |
| Baker link. Dev | Rev. 1 (RP2040)                          | ¥1,980   | メイン MCU                      |
| XIAO ESP32-C3   | Seeed Studio                             | ¥800     | WiFi コプロセッサ (esp-hosted)  |
| DRV8835         | AE-DRV8835-S モジュール                  | ¥400     | モーター制御（PH/EN モード）    |
| ICM-20602       | Breakout Board                           | ¥990     | 6軸 IMU（odom / 姿勢推定）      |
| MQ-3B           | エタノールセンサー                       | ¥450     | 天使の分け前検知                |
| HC-SR04         | 超音波センサー                           | ¥300     | 障害物回避                      |
| NJU7223F33      | 3.3V 500mA LDO                           | ¥80      | 9V → 3.3V レール                |
| BP5293-50       | 5V 1A 降圧 DC-DC                         | ¥240     | 9V → 5V レール                  |
| 9V 電池 (006P)  | + スナップ / スイッチ                    | ~¥500    | 主電源                          |

### 4.2 RP2040 (Baker link. Dev) ↔ XIAO ESP32-C3 SPI 接続

| RP2040 GPIO     | ESP32-C3 GPIO | XIAO Pin | 信号名     | 用途                                                |
| --------------- | ------------- | -------- | ---------- | --------------------------------------------------- |
| GP16 (SPI0 RX)  | GPIO5         | D3       | SPI MISO   | データ入力                                          |
| GP17 (GPIO out) | GPIO10        | D10      | SPI CS     | チップセレクト                                      |
| GP18 (SPI0 SCK) | GPIO6         | D4       | SPI CLK    | クロック                                            |
| GP19 (SPI0 TX)  | GPIO7         | D5       | SPI MOSI   | データ出力                                          |
| GP15 (GPIO in)  | GPIO3         | D1       | Handshake  | タイミング同期                                      |
| GP13 (GPIO in)  | GPIO4         | D2       | Data Ready | データ到着通知                                      |
| GP14 (GPIO out) | GPIO21        | D6       | Reset      | リセット制御                                        |
| —               | 5V            | —        | 電源       | 5V レール (BP5293-50) から給電。内蔵 LDO で 3.3V 化 |
| GND             | GND           | —        | GND        | グランド                                            |

### 4.3 RP2040 その他ピンアサイン

| RP2040 GPIO    | 機能                | 接続先                           |
| -------------- | ------------------- | -------------------------------- |
| GP4 (I2C0 SDA) | I2C データ          | ICM-20602 SDA                    |
| GP5 (I2C0 SCL) | I2C クロック        | ICM-20602 SCL                    |
| GP26 (ADC0)    | ADC 入力            | MQ-3B AOUT（10k/15k 分圧経由）   |
| GP10           | GPIO Out (左 PHASE) | DRV8835 AIN1 (APHASE)            |
| GP11 (PWM5_B)  | PWM (左 ENABLE)     | DRV8835 AIN2 (AENABLE)           |
| GP12           | GPIO Out (右 PHASE) | DRV8835 BIN1 (BPHASE)            |
| GP9 (PWM4_B)   | PWM (右 ENABLE)     | DRV8835 BIN2 (BENABLE)           |
| GP2            | GPIO (Trigger)      | HC-SR04 Trig                     |
| GP3            | GPIO (Echo)         | HC-SR04 Echo（10k/15k 分圧経由） |
| GP20           | LED (赤)            | ステータス LED                   |
| GP21           | LED (橙)            | ステータス LED                   |
| GP22           | LED (緑)            | ステータス LED                   |

> DRV8835 は **Phase/Enable モード** (MODE ピンを VCC にプル) で使用。
> `xPHASE` で方向、`xENABLE` の PWM duty で速度を決める最小ピン構成。

### 4.4 電源設計

```text
9V 電池 (006P)
  ├── BP5293-50 （降圧 DC-DC、IN 8〜24V → 5V / 1A）
  │     └─ 5V レール: XIAO ESP32-C3 5V ピン、MQ-3B VCC（ヒーター 150mA）、
  │                  HC-SR04 VCC、DRV8835 VM（モーター電源）
  └── NJU7223F33（LDO、IN ≤10V → 3.3V / 500mA）
        └─ 3.3V レール: Baker link. Dev 3V3、ICM-20602 VCC、DRV8835 VCC（ロジック）
```

| 項目           | 内容                                                                                                                    |
| -------------- | ----------------------------------------------------------------------------------------------------------------------- |
| XIAO の給電    | 5V ピンに入力し内蔵 LDO で 3.3V 化。WiFi ピーク電流（~350mA）を 5V レール側に逃がし、NJU7223F33 の負荷を軽く保つ        |
| モーター駆動   | キット付属モーターは 3V 定格。VM=5V のため PWM duty を ~60% に制限して実効 3V 相当で駆動。VM 直近にバルクコンデンサ推奨 |
| BP5293-50      | 入力 8V 未満で出力低下。9V 電池の残量低下に注意                                                                         |
| NJU7223F33     | (9V−3.3V)×負荷電流の発熱。放熱パターン確保                                                                              |
| USB 併用禁止   | 書き込み・デバッグで USB 接続中は電池を切り離す                                                                         |
| 5V → 3.3V 信号 | MQ-3B AOUT / HC-SR04 Echo は 10kΩ+15kΩ 分圧で 3.0V 化（RP2040 は 5V 非耐圧）                                            |

### 4.5 注意事項

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

ESP32-C3 (esp-hosted) 経由で micro-ROS Agent に TCP 接続し、ROS2 トピックを pub/sub する。

| ステップ | 内容                                              | 成果物                              |
| -------- | ------------------------------------------------- | ----------------------------------- |
| 1-1      | ESP32-C3 に esp-hosted-mcu ファームウェア書き込み | ESP32-C3 SPI スレーブ起動           |
| 1-2      | Cargo.toml に WiFi 依存追加                       | `wifi` feature flag                 |
| 1-3      | WiFi 通信サンプル作成                             | `examples/wifi_microros_sensors.rs` |
| 1-4      | 動作確認 (Docker + verify_sub.py)                 | `/chatter` メッセージ受信           |

**検証基準**:
- `cargo build --no-default-features --features wifi --example wifi_microros_sensors` 成功
- RTT ログ: DHCP 取得 → TCP 接続 → micro-ROS XRCE-DDS セッション成功
- `verify_sub.py` で `/chatter` メッセージ受信

### Phase 2: センサー統合

MQ-3B (ADC) のセンサーデータを取得し、ROS2 トピックとしてパブリッシュする。

> **2026-07 設計変更**: BME280（温湿度・気圧）はハードウェア構成から削除。
> I2C0 (GP4/GP5) には ICM-20602 (IMU) を接続する。既存の BME280 コードパスは
> examples に残っているが、回路上は未接続。

| ステップ | 内容                             | 成果物                              | 状態 |
| -------- | -------------------------------- | ----------------------------------- | ---- |
| 2-1      | MQ-3 ADC 読み取り                | embassy-rp ADC (GP26)               | ✅    |
| 2-2      | スタンドアロンセンサー読み取り   | `examples/sensor_read.rs`           | ✅    |
| 2-3      | センサーデータ ROS2 パブリッシュ | `examples/wifi_microros_sensors.rs` | ✅    |
| 2-4      | ICM-20602 (I2C) ドライバ統合     | IMU データ取得                      | ⏳    |

**ROS2 トピック**:

| トピック              | 型                 | 内容           |
| --------------------- | ------------------ | -------------- |
| `/angel_nose/ethanol` | `std_msgs/Float32` | MQ-3B 電圧 [V] |

**ビルドコマンド**:
```sh
# センサー単体テスト
cargo build --no-default-features --features sensor --example sensor_read

# WiFi + センサー + micro-ROS パブリッシュ
cargo build --no-default-features --features wifi,sensor --example wifi_microros_sensors
```

**検証基準**:
- `cargo check --features sensor --example sensor_read` 成功
- `cargo check --features wifi,sensor --example wifi_microros_sensors` 成功
- RTT ログ: MQ-3B ADC 値の周期的出力
- ROS2 側: `ros2 topic echo /angel_nose/ethanol std_msgs/msg/Float32` でデータ受信

### Phase 3: モーター制御

| ステップ | 内容                         | 成果物                                    | 状態 |
| -------- | ---------------------------- | ----------------------------------------- | ---- |
| 3-1      | DRV8835 PWM 制御             | 2WD 前後進・旋回 (`src/drv8835.rs`)       | ✅    |
| 3-2      | `/cmd_vel` サブスクライブ    | Twist → モーター PWM 変換 + 500 ms WD     | ✅    |
| 3-3      | ROS2 .msg コードジェネ       | `build.rs` + `roslibrust_codegen`         | ✅    |
| 3-4      | `/odom` + `/tf` パブリッシュ | `nav_msgs/Odometry`, `tf2_msgs/TFMessage` | ✅    |

### Phase 4: 障害物回避・ナビゲーション

| ステップ | 内容                  | 成果物                                    | 状態 |
| -------- | --------------------- | ----------------------------------------- | ---- |
| 4-1      | HC-SR04 超音波測距    | `/ultrasonic/front` (`sensor_msgs/Range`) | ✅    |
| 4-2      | open-loop odometry    | `/odom`, `odom` → `base_link`             | ✅    |
| 4-3      | RViz2・odom 基準 Nav2 | 2WD URDF、rolling costmap、DWB            | ✅    |

HC-SR04 は前方の近距離障害物専用であり、自己位置推定や SLAM には使用しない。
車輪エンコーダがないため、odom 基準 Nav2 は短距離・低速試験に限定する。
既知地図での巡回は現行構成の対象外であり、実装する場合は 2D LiDAR などの外部測位センサーと localization 構成を追加する。

### Phase 5: 統合・自律巡回

| ステップ | 内容                   | 成果物                            |
| -------- | ---------------------- | --------------------------------- |
| 5-1      | 複数樽巡回マッピング   | ArUco/AprilTag 位置推定           |
| 5-2      | 液面磁界測定           | 浮遊ネオジム磁石 + ホールセンサー |
| 5-3      | 天使の分け前蒸発量推定 | エタノール + 液面データ統合       |
| 5-4      | Web ダッシュボード     | ROS2 → WebSocket → ブラウザ       |

---

## 7. WiFi 通信の実装パターン

`examples/wifi_microros_sensors.rs` をテンプレートとし、`embassy-net-wiznet` を `embassy-net-esp-hosted` に置換する。

### 7.1 タスク構成

```text
main()
  ├── esp_hosted_task  — SPI 通信ドライバループ
  ├── net_task         — embassy-net パケット I/O ループ
  ├── microros_task    — DHCP → TCP → XRCE-DDS / micro-ROS → Node::spin()
  └── app_task         — Publisher.send() / Subscription.try_recv()
```

### 7.2 トランスポート抽象化

micro_xrce_dds_rs の `XrceSession::connect()` は `T: Read + Write` を受け取る。
TCP ソケットをそのまま渡せるため、WiFi/Ethernet の違いはネットワークデバイス層のみ。

```rust
let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
socket.connect(agent_endpoint).await?;
let session = XrceSession::connect(socket, SESSION_ID, CLIENT_KEY).await?;
```

---

## 8. ビルド・デプロイ

### 8.1 開発環境

- **Dev Container**: Rust 1.94.1 + thumbv6m-none-eabi
- **デバッガ**: probe-rs (Baker link. Dev 内蔵 CMSIS-DAP)
- **ROS2**: Docker (micro-ROS Agent)

### 8.2 ビルドコマンド

```sh
# 通常ビルド (HAL + defmt)
cargo build --release

# Embassy サンプル
cargo build --no-default-features --features embassy --example embassy_blinky

# WiFi サンプル (Phase 1)
cargo build --no-default-features --features wifi --example wifi_microros_sensors

# 書き込み (probe-rs)
cargo run --release

# UF2 変換 (USB ブートローダ書き込み用)
elf2uf2-rs target/thumbv6m-none-eabi/release/tenshi-no-hana target/tenshi-no-hana.uf2
```

### 8.3 Docker (micro-ROS Agent)

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

- [Baker link. Dev](https://www.baker-link.com/)
- [embassy-net-esp-hosted](https://github.com/embassy-rs/embassy/tree/embassy-net-esp-hosted-v0.3.0/embassy-net-esp-hosted)
- [esp-hosted-mcu](https://github.com/espressif/esp-hosted-mcu)
- [Seeed Studio XIAO ESP32C3](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)

