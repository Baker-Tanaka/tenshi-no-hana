# 天使の鼻

## コンセプト

「天使の鼻」は、お酒の熟成中に蒸発する「天使の分け前」を嗅ぎ取る、
蒸留所・熟成庫の見回りローバーを目指すプロジェクトです。
アルコールセンサーのアナログ値をRP2040で読み取り、ROS 2で確認できます。

**Embedded Rust（組み込みRust）でmicro-ROS相当のROS 2連携を実現**します。
RP2040上では `no_std` のRustとEmbassyでデバイスを制御し、
Rust製フレームワーク `micro_xrce_dds_rs` からMicro XRCE-DDSで
micro-ROS Agentへ接続します。ファームウェアにはC版のmicro-ROSランタイムを
組み込まず、生成済みのRustメッセージ型でROS 2と通信します。

> **注意：MQ-3の出力は未校正の生値です。**
> `/alcohol` はADCの読み取り値を0.0〜1.0に正規化したもので、
> アルコール濃度（ppm）や検知判定ではありません。MQ-3の予熱・校正も別途必要です。
> 見回りや自律走行も将来の構想であり、現行実装は以下の手動操作とセンサー表示です。

## 現在できること

Baker link. Dev（RP2040）の2輪ローバーを、ROS 2 Jazzy の
`teleop_twist_keyboard` で操作し、IMU・超音波センサーをRViz2で表示します。
MQ-3の未校正アナログ値はROS 2トピックで配信します。
自律走行・地図・Nav2・odometry は使用しません。

```text
teleop_twist_keyboard -> /cmd_vel -> micro-ROS Agent (TCP 8888)
                                        |
                                      Wi-Fi
                                        |
                              ESP32-C6 / ESP-Hosted 2.12.9
                                        |
                                   SPI / RP2040
                                 /      |      \
                              DRV8835   IMU    HC-SR04   MQ-3
                                         \      |       /
                                      ROS 2 -> RViz2 / topic
```

## 準備とビルド

開発コンテナを使用してください。Rust・ROS 2・Agent・RViz2・teleop の
依存パッケージは [.docker/Dockerfile](.docker/Dockerfile) で導入します。
依存変更後はコンテナを再ビルドします。RViz2 には GUI を表示できる環境が必要です。

```sh
git submodule update --init --recursive
cp wifi_config.json.example wifi_config.json
# ローカルの wifi_config.json に SSID・パスワード・Agent の IPv4:port を設定
cargo build --locked
# リリース用: cargo build --release --locked
```

- `wifi_config.json` は git 管理外です。認証情報を commit しないでください。
- Agent のアドレスはローバーから到達できるホストPCのLAN側IPです。
  Docker内部IPではありません。TCP 8888 の公開・ファイアウォールも確認してください。
- 現行ESP-hostedドライバのパスワード上限は UTF-8 で32バイトです。
- debug の `opt-level = 2` を維持してください。無最適化ではprotobufデコーダが
  RP2040のRAMを超えるスタックを確保します。
- ボードへの書き込みは VS Code の `Baker Link: Debug` を使います。
  ホスト側の Baker Link Env を Run にし、最初は車輪を浮かせてください。

## 起動

### 1. micro-ROS Agent

開発コンテナではAgentが自動起動します。手動で動かす場合のみ、別端末で実行します。
同じポートで二重起動しないでください。

```sh
source /opt/ros/jazzy/setup.bash
source /opt/micro_ros_agent_ws/install/setup.bash
ros2 run micro_ros_agent micro_ros_agent tcp4 --port 8888 -v 4
```

自動起動時のログは `/tmp/micro-ros-agent.log` です。
Windows/WSLで到達できない場合の補助は [scripts/setup_portproxy.ps1](scripts/setup_portproxy.ps1)
を参照してください。

### 2. センサー表示

```sh
source /opt/ros/jazzy/setup.bash
cd /tenshi-no-hana/ros2_ws
colcon build --symlink-install --packages-select tenshi_no_hana_bringup
source install/setup.bash
ros2 launch tenshi_no_hana_bringup teleop.launch.py
```

このlaunchは機体モデル・固定TFとRViz2のみを起動します。
`base_link` 基準で超音波距離とIMUの生加速度ベクトル（重力込み）を表示します。
機体寸法とセンサー位置は仮値なので、必要に応じて
[URDF](ros2_ws/src/tenshi_no_hana_bringup/urdf/tenshi_no_hana.urdf.xacro)を実測値へ変更してください。
IMUは姿勢推定しないため、姿勢表示は無効です。角速度は次で確認できます。

```sh
ros2 topic echo /imu/data --field angular_velocity
```

### 3. キーボード操作

別の対話端末で実行し、その端末にフォーカスして操作します。

```sh
source /opt/ros/jazzy/setup.bash
ros2 run teleop_twist_keyboard teleop_twist_keyboard --ros-args \
  -p stamped:=false -p speed:=0.1 -p turn:=0.5 \
  --remap cmd_vel:=/cmd_vel
```

| キー | 動作 |
| --- | --- |
| `i` | 前進 |
| `u` / `o` | 前進しながら左／右旋回 |
| `k` | 停止 |
| Ctrl+C | 停止して終了 |

- **現在は前進専用**です。片輪でも逆転が必要な指令は両輪停止として扱います。
  後退・その場旋回はできません。旋回速度を上げすぎると前進旋回も拒否されます。
- Jazzy版はキー入力ごとの送信です。移動キーを押し続け、
  OSのキーリピート間隔を500ms未満にしてください。リピート開始前に一度停止する場合があります。
- 指令の受信から500ms経過、通信切断、再接続、非有限値ではモーターを停止します。
  duty上限は60%です。これはソフトウェアの監視であり、MCU停止に対する
  ハードウェアwatchdogではありません。
- 他の `/cmd_vel` 配信元を同時に動かさないでください。
  異常時に電源を切れる状態で、初回は車輪を浮かせて確認してください。

## 配線

### ESP32-C6

ESP-Hosted MCU slave **2.12.9**、SPI mode 3、10MHz。
GPIO番号はESP側ファームウェアの設定と合わせます。

| RP2040 | ESP32-C6 GPIO | 信号 |
| --- | --- | --- |
| GP18 | 22 | CLK |
| GP19 | 23 | MOSI |
| GP16 | 21 | MISO |
| GP17 | 18 | CS |
| GP15 | 1 | Handshake |
| GP13 | 2 | Data Ready |
| GP14 | 16 | Slave Reset（ENではない） |

### センサー・モーター

| RP2040 | 接続先 |
| --- | --- |
| GP4 / GP5 | ICM-20602 SDA / SCL（0x68または0x69、自動検出） |
| GP2 / GP3 | HC-SR04 Trigger / Echo |
| GP28 | MQ-3 AO（22 kΩをAO側、10 kΩをGND側とする分圧の接点） |
| GP10 / GP11 | 左DRV8835 PHASE / ENABLE（PWM） |
| GP12 / GP9 | 右DRV8835 PHASE / ENABLE（PWM） |

GNDを共通にし、RP2040のGPIO/ADC信号は3.3V以下にしてください。MQ-3モジュールを5V給電する場合、
AOとGNDの間に22 kΩと10 kΩを直列に接続し、その接点をGP28へ接続します
（AO—22 kΩ—GP28—10 kΩ—GND）。AOが5VのときGP28は約1.56Vです。
未分圧のAOをGP28へ接続しないでください。HC-SR04は5V給電で、
Echoは10kΩ（上側）/15kΩ（GND側）の分圧を通します。RP2040は5V非耐圧です。
DRV8835のコードはPH/EN（MODE=VCC）を前提としますが、現行実機では
後進が未確認のため前進専用です。

モーターは3V定格、VMは5Vを想定しています。USBと電池は電源を分離・逆流防止
できていると確認できない限り同時に接続しないでください。
[cad/](cad/) は旧設計の資料として残しています。ESP32-C3などの旧部品・配線を
含むため、現行の接続先は上の表を優先してください。

## 構成とテスト

| 場所 | 内容 |
| --- | --- |
| [src/main.rs](src/main.rs) | 初期化とノードの登録 |
| [src/board.rs](src/board.rs) | ピンと周辺回路 |
| [src/network.rs](src/network.rs) | Wi-Fi・DHCP・Agent接続と再試行 |
| [src/nodes/](src/nodes/) | motor・imu・range・alcohol |
| [examples/alcohol_check.rs](examples/alcohol_check.rs) | Wi-Fi/ROSを使わずGP28のMQ-3 ADC値だけをRTT出力するテスト |
| [src/drivers/](src/drivers/) | デバイス操作 |
| [src/diff_drive.rs](src/diff_drive.rs) | 前進専用の左右輪指令計算 |
| [ros2_ws/src/tenshi_no_hana_bringup/](ros2_ws/src/tenshi_no_hana_bringup/) | 手動操作用launch・RViz・URDF |
| [msgs/](msgs/) / [generated/](generated/) | ROSメッセージ入力 / 生成済みno_std Rust crate |
| [external/](external/) | EmbassyとRust micro-ROSフレームワーク |

| ROS topic | 方向 | 型 |
| --- | --- | --- |
| `/cmd_vel` | 購読 | `geometry_msgs/Twist` |
| `/imu/data` | 配信 | `sensor_msgs/Imu` |
| `/ultrasonic/front` | 配信 | `sensor_msgs/Range` |
| `/alcohol` | 配信 | `std_msgs/Float32`（ADC raw / 4095） |

IMUは加速度と角速度のみ、超音波は測定失敗・範囲外を正の無限大で配信します。
`/odom` と動的 `/tf` の配信はありません。センサーの固定TFはホストで生成します。
センサータスクはWi-Fi接続前から動作し、時刻はAgent同期前のみ起動後時刻です。
実機でキーボード操作、IMUの重力加速度、超音波の連続測距を確認済みです。
長時間の通信安定性と測定精度は別途検証が必要です。

```sh
# ボード不要の走行計算・距離換算テスト
cargo test --locked --lib --no-default-features --target x86_64-unknown-linux-gnu

# ROS構成テスト（先にROS環境をsource）
cd /tenshi-no-hana/ros2_ws
colcon test --packages-select tenshi_no_hana_bringup
colcon test-result --verbose
```

通常のCargoビルドにROSやPythonは不要です。メッセージ定義を変更した場合だけ、
ROS Jazzy環境で再生成し、入力と出力を一緒に管理します。
生成・互換性試験用の標準メッセージ一式は維持し、アプリは必要なcrateだけに依存します。

```sh
bash external/micro_xrce_dds_rs/codegen/regenerate.sh
bash external/micro_xrce_dds_rs/codegen/regenerate.sh --check
```

生成器の詳細は [codegen/README.md](external/micro_xrce_dds_rs/codegen/README.md) を参照してください。
submodule内の変更を配布する際は、それぞれのリポジトリへ先にcommit/pushし、
親リポジトリの参照を更新してください。
