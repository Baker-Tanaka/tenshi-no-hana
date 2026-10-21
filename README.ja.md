# 天使の鼻 - ROS2 × Baker link. Dev × ローバー

> ⚠️ 本プロジェクトは現在開発途中です。機能、配線、ドキュメントは変更される可能性があります。

**ROS 2 / Nav2 から遠隔操作できる 2WD ローバー。**

## イメージ画像

![プロジェクトプレビュー](docs/img/image.png)

![システム構成](docs/img/tenshi_no_hana_rover_v2.svg)

> English README is available at `README.md`.

## 概要
天使の鼻は、Baker link. Dev Rev.1 (RP2040) 基板を中心に構成された 2輪ローバーです。現行の対象は ROS 2 からの走行制御、IMU・超音波測距、open-loop odometry です。蒸留所巡回、エタノール計測、磁界による液面推定は将来拡張であり、現在のファームウェアには実装していません。

本プロジェクトは以下を組み合わせています。
- **ESP32-C6 の Wi-Fi / ESP-hosted MCU（slave firmware 2.12.9）**
- **Embassy-rs no_std 非同期ランタイム**
- **micro_xrce_dds_rs** と **micro-ROS Agent** による ROS2 互換メッセージング

## 主な特徴
- ROS 2 速度指令と 500 ms のモーター watchdog（現在は前進のみ）
- 6軸 IMU (ICM-20602) の加速度・角速度配信（姿勢融合は行わない）
- HC-SR04 測距と車輪指令速度による open-loop odometry
- ESP32-C6 で Wi-Fi 通信を実現
- 低コストかつ no_std 組み込み設計

## ハードウェア構成
2026年4月時点の日本国内向け概算です。

| 部品                 | 例                                                                   | 価格目安 | 購入先                                                              | 備考                                       |
| -------------------- | -------------------------------------------------------------------- | -------- | ------------------------------------------------------------------- | ------------------------------------------ |
| 2WD ロボットシャーシ | 2WD Mini Smart Robot Mobile Platform Kit                             | ¥1,900   | [秋月電子](https://akizukidenshi.com/catalog/g/g113651/)            | FA-130 系 3V 定格ギヤモーター×2 付属       |
| Baker link. Dev      | Rev. 1                                                               | ¥1,980   | [スイッチサイエンス](https://www.switch-science.com/products/10044) | メイン MCU、Embassy-rs 対応                |
| モータドライバ       | AE-DRV8835-S（DRV8835 ステッピング&DC モータードライバーモジュール） | ¥400     | [秋月電子](https://akizukidenshi.com/catalog/g/g109848/)            | PH/EN モード（MODE ピンを VCC へ）         |
| IMU                  | ICM-20602 Breakout Board                                             | ¥990     | [スイッチサイエンス](https://www.switch-science.com/products/8695)  | 6軸（加速度+ジャイロ）、I2C addr 0x68      |
| エタノールセンサー   | MQ-3B / MQ-3 モジュール                                              | ¥450     | [秋月電子](https://akizukidenshi.com/catalog/g/g116269/)            | 天使の分け前検知。ヒーターは 5V/150mA      |
| 超音波センサー       | HC-SR04                                                              | ¥300     | [スイッチサイエンス](https://www.switch-science.com/products/8224/) | 障害物回避用（5V デバイス）                |
| 3.3V レギュレーター  | NJU7223F33（3.3V 500mA LDO）                                         | ¥80      | [秋月電子](https://akizukidenshi.com/catalog/g/g100432/)            | 9V → 3.3V レール（ロジック系）             |
| 5V レギュレーター    | BP5293-50（5V 1A DC-DC）                                             | ¥240     | [秋月電子](https://akizukidenshi.com/catalog/g/g111188/)            | 9V → 5V レール（モーター・MQ-3B・HC-SR04） |
| 電池                 | 9V 電池 (006P) + スナップ                                            | ~¥500    | —                                                                   | 両レギュレーターへ給電。電源スイッチ推奨   |
| 分圧抵抗             | 10kΩ + 15kΩ ×2 組                                                    | < ¥20    | —                                                                   | MQ-3B AOUT / HC-SR04 Echo（5V）を 3.0V 化  |

## 配線図

![](docs/schematics/tenshi_no_hana_wiring.svg)

> 図と後半の C3 用ピン表は、過去の XIAO ESP32-C3 設計です。
> 現行実機は ESP32-C6 / ESP-Hosted MCU slave 2.12.9 です。
> 実配線には [確認済みの現行 GPIO 表](docs/DESIGN.md#ピンと電源)を使用してください。

### 電源系統

```
9V 電池 (006P)
  ├── BP5293-50  → 5V レール   : XIAO 5V ピン、MQ-3B VCC（ヒーター）、HC-SR04 VCC、DRV8835 VM
  └── NJU7223F33 → 3.3V レール : Baker link. Dev 3V3、ICM-20602 VCC、DRV8835 VCC（ロジック）
```

> - 書き込み・デバッグで **USB を接続する時は電池を切り離す** こと。
> - BP5293-50 は **入力 8V 以上** が必要。9V 電池の残量低下に注意。
> - NJU7223F33 は (9V−3.3V)×負荷電流 の発熱がある。3.3V 系の負荷を軽く保ち、放熱パターンを確保する。

### 信号レベル（5V → 3.3V）

RP2040 の GPIO は **5V 非耐圧**。以下の 2 信号は **10kΩ + 15kΩ の分圧**（5V → 3.0V）が必須:

- **MQ-3B AOUT → GP26 (ADC0)** — 将来のセンサーノード用。ADC 取得・予熱処理は未実装。AOUT は最大 ~5V。
- **HC-SR04 Echo → GP3** — Echo は 5V パルスを出力。

### モーター駆動

キット付属モーターは **3V 定格** に対し DRV8835 の VM は 5V レールから給電するため、
ファームウェア側で PWM duty を **~60%** に制限して実効電圧を 3V 相当に抑える
（DRV8835 は最大 1.5A/ch。VM 直近にバルクコンデンサ推奨）。

## 無線通信
- **ESP32-C6** を ESP-Hosted MCU slave **2.12.9** として使用
- `external/embassy` サブモジュールに含まれる `embassy-net-esp-hosted` を利用

## ソフトウェア構成
- `micro_xrce_dds_rs` と micro-ROS Agent
- 生成済みの `no_std` 標準 ROS メッセージ crate。ファームウェアに C の RMW ライブラリはリンクしない

## ファームウェアのビルド

```sh
git submodule update --init --recursive
cp wifi_config.json.example wifi_config.json
# wifi_config.json に SSID、パスワード、Agent の IPv4 アドレスとポートを設定する。
cargo build --locked
cargo build --release --locked
```

ツールチェーンは [rust-toolchain.toml](rust-toolchain.toml) で固定します。
`flip-link` が必要です（開発コンテナには導入済み）。
Wi-Fi 設定は commit しないでください。JSON、SSID のサイズ、Agent の接続先が
不正な場合は、代替値を使わずビルドエラーにします。
現行 ESP-hosted fork のパスワード上限は UTF-8 で 32 バイトです。
超過時は接続を無限に再試行せず、ビルド時にエラーにします。
既定 feature は `wifi` で、ファームウェアの binary に必須です。
debug ビルドもアプリ・依存とも `opt-level = 2` を維持してください。
依存だけの最適化ではアプリで実体化される protobuf デコーダの巨大な
スタック確保を防げません。debug シンボルと assertions は有効のままです。

ボードなしで走行計算を検証できます。

```sh
cargo test --lib --no-default-features --target x86_64-unknown-linux-gnu
```

[メッセージ定義](msgs/)やデバイス容量を変更した場合は、ROS Jazzy 開発コンテナで
再生成します。

```sh
bash external/micro_xrce_dds_rs/codegen/regenerate.sh
# 入力・生成器・生成結果の整合を、ファイルを書き換えずに確認する:
bash external/micro_xrce_dds_rs/codegen/regenerate.sh --check
```

ament 連携とテストは[生成器の説明](external/micro_xrce_dds_rs/codegen/README.md)を
参照してください。入力と生成結果を一緒にバージョン管理します。
submodule 内の変更は、そのリポジトリで先に commit / push してから親の参照を
更新する必要があります。親だけ commit しても submodule 内の未コミット変更は
配布されません。

## ファームウェア構成

組み込み側では、実行時にノードを探して起動する ROS 2 launch ファイルではなく、
起動時にタスク構成を固定する `launch` モジュールを使います。
[`src/main.rs`](src/main.rs) は Embassy のエントリーポイントに限定し、
RP2040 と ROS context の初期化、`NodeSpawner` への spawner の受け渡し、
ボードリソースの組み立て、[`src/launch.rs`](src/launch.rs) の
呼び出しだけを担当します。ノードを追加・削除するときは `main.rs` ではなく、
`launch.rs` のノード起動一覧を変更します。

| モジュール | 責務 |
| --- | --- |
| [`board.rs`](src/board.rs) | 配線、割り込み、ハードウェアリソースの生成 |
| [`drivers/`](src/drivers/) | DRV8835・HC-SR04・ICM-20602 の操作。ROS entity は扱わない |
| [`network.rs`](src/network.rs) | ESP-hosted・network runner、Wi-Fi/DHCP/TCP 接続、XRCE driver 起動 |
| [`launch.rs`](src/launch.rs) | アプリのタスク構成と起動順序 |
| [`nodes/imu/`](src/nodes/imu/) | `/imu/data` |
| [`nodes/range/`](src/nodes/range/) | `/ultrasonic/front` |
| [`nodes/motor/`](src/nodes/motor/) | `/cmd_vel`、watchdog、実現速度の共有 |
| [`nodes/odometry/`](src/nodes/odometry/) | open-loop `/odom` と `/tf` |

各ノードの処理、topic、ノード固有設定は専用フォルダにまとめます。
各ノードは `#[embassy_ros::node(name = "...")]` を付けた普通の async 関数として
記述し、型付き引数に Publisher / Subscription を宣言します。
setup/run の tuple 配線や別の起動関数は不要です。
launch は `nodes.register(imu::node, board.imu)?` のように起動対象を登録します
（odometry のリソースは `()`）。task の確保と spawn はフレームワークが担当します。
topic と生成される task はノード内部に隠し、endpoint 作成と ROS 接続は自動化します。
Topic の内部配送は共有しますが、DDS endpoint は各ノードに所属します。
Embassy の所有権に従ってデバイスを一度だけ移譲します。
Publisher / Subscription を組み立ててノードへ渡す責務は持ちません。
ノード側でローカル用とネットワーク用の Publisher を使い分ける必要はありません。
IMU タスクは I2C バスを直接所有するため、共有バス用のセルは不要です。
motor と odometry 間では、速度と更新時刻を一つの短い critical section で
読み書きし、異なるサンプルの値が混ざらないようにします。

ローカルのセンサー取得、モーター制御、odometry は Wi-Fi / Agent 接続前に
起動します。モーターはフレームワークの世代付き接続状態を監視し、
切断・古い世代の指令・期限切れの指令で停止します。
500 ms の watchdog は受信キューから取り出した時刻ではなく、
デバイスが指令を受理した単調増加時刻を基準にします。
遠隔 `/cmd_vel` 操作には Wi-Fi と
micro-ROS Agent が必要です。オフライン時もセンサー取得と状態推定は続きますが、
自律走行機能を追加するものではありません。

公開フレームワークは
[`embassy-ros`](external/micro_xrce_dds_rs/embassy_ros/README.md) です。
`embassy_ros::init()` → `Context` → `Node` → 型付き Topic の endpoint という
インターフェイスにしました。静的ストレージで容量を固定し、Embassy の
channel 型は通常の利用 API に出しません。
[`micro_xrce_dds_rs`](external/micro_xrce_dds_rs/) は低レベル backend として維持します。
entity 準備、routing、時刻同期はフレームワークが担当し、
ノードは取得時点で `context.clock().now()` を使います。初回同期前は従来どおり
起動後の単調増加時刻を使用しますが、これは Unix 時刻ではありません。
接続リトライ方針、センサー設定、モーター制限、ノード構成はアプリ側に残します。
通信切断はログに出し、アプリが Agent への再接続を試行します。
Wi-Fi 切断時は再接続し、ESP-hosted の heartbeat は通知を受けて扱います
（ドライバ既定の timeout 時 panic は使いません）。
ハードウェアの初期化には応答可能な slave が必要です。

共通の TCP 接続・XRCE handshake は、任意機能
`embassy_ros::transport::embassy_net::Agent` を使います。アプリ側は接続先、静的
`TcpBuffers`、リトライ方針を指定します。失敗時は socket が返るため再利用でき、
起動中の接続・handshake 失敗はログを出して再試行します。
ボード固有の Wi-Fi 初期化は `network.rs` に残します。

ROS メッセージはホスト専用の
[生成器](external/micro_xrce_dds_rs/codegen/README.md)で公式 ROS IDL
adapter / parser を使って生成します。
[ros2_rust](https://github.com/ros2-rust/ros2_rust/blob/main/docs/message-generation.md)
に従い、生成器と実行時サポートを分離します。
[生成済みパッケージ crate](generated/)をバージョン管理し、デバイス側は
`no_std` 型と CDR を使い、C の RMW には依存しません。
無制限フィールドのデバイス内容量は文字列 32 バイト、
sequence は `/tf` の一つの transform に合わせて一要素です。
定義・容量変更時は再生成が必要ですが、通常の Cargo build に ROS / Python は不要です。
アプリの build script は Wi-Fi 設定の検証・埋め込みとリンカ検索パスのみを扱います。
生成された Header には `set_stamp` を提供します。
公開 Publisher は非同期通信を待たない keep-last
方式で、成功はデバイス内の受理を意味します。Agent 配送 ACK やイベントの
確実な配送を保証する API ではありません。

`main.rs` は一度だけ `init()` し、`launch.rs` はモーターから順にノード定義を
登録します。フレームワークはネットワーク I/O なしで endpoint と ROS 方向の
準備を同期的に済ませてから task を起動します。登録失敗はノード名・topic・
失敗フェーズ付きで報告し、後続登録・構成確定・ネットワーク接続を禁止します。
main は登録後に `nodes.finish()` で明示的に構成を確定してから network を起動します。
全ノードの同時有効化や起動済み task のロールバックは保証しません。
ノードごとの bridge 処理はなく、
フレームワークの connection タスクが既存の ROS interface を接続します。
ネットワーク接続の再試行が続いてもローカル制御は動き続けます。
既存の ROS topic 名とモーター制限は維持しています。

この `launch.rs` は組み込みファームウェア内のノード構成を表すもので、
Nav2 や RViz などホスト側 ROS 2 プロセスを起動する `ros2 launch` とは別です。

## 設置・運用の流れ
## ROS 2 インターフェース

| 方向 | トピック            | 型                    | 用途                    |
| ---- | ------------------- | --------------------- | ----------------------- |
| 購読 | `/cmd_vel`          | `geometry_msgs/Twist` | 速度指令                |
| 配信 | `/imu/data`         | `sensor_msgs/Imu`     | 加速度・角速度          |
| 配信 | `/ultrasonic/front` | `sensor_msgs/Range`   | HC-SR04 前方障害物距離  |
| 配信 | `/odom`             | `nav_msgs/Odometry`   | open-loop 姿勢・速度    |
| 配信 | `/tf`               | `tf2_msgs/TFMessage`  | `odom` から `base_link` |

メッセージ時刻は micro-ROS Agent と同期します。姿勢推定は行わず、車輪エンコーダがないためオドメトリは open-loop です。高精度な Nav2 運用には外部自己位置推定を併用してください。

## キーボード操作とセンサー可視化（手動操作）

通常ファームウェアを使用してください。`sensor_check` / `motor_check` の
単体試験ファームウェアには ROS 通信機能がありません。
Wi-Fi と micro-ROS Agent（TCP 8888）の接続が必要です。
現時点で通信の継続安定性は未確認です。

コンテナを再ビルドするか、ROS ホストで依存パッケージを追加します。

```sh
sudo apt-get update
sudo apt-get install ros-jazzy-teleop-twist-keyboard ros-jazzy-rviz-imu-plugin
source /opt/ros/jazzy/setup.bash
cd /tenshi-no-hana/ros2_ws
colcon build --symlink-install --packages-select tenshi_no_hana_bringup
source install/setup.bash
ros2 launch tenshi_no_hana_bringup teleop.launch.py
```

RViz2 が表示できる GUI 環境で実行してください。`base_link` 基準で機体、
超音波距離、IMU の生加速度ベクトル（重力込み）を表示します。
姿勢推定がないため IMU の姿勢表示は無効です。角速度の数値は
`ros2 topic echo /imu/data --field angular_velocity` で確認できます。

別の対話端末で、セットアップを読み込んでキーボード操作を起動します。

```sh
source /opt/ros/jazzy/setup.bash
source /tenshi-no-hana/ros2_ws/install/setup.bash
ros2 run teleop_twist_keyboard teleop_twist_keyboard --ros-args \
  -p stamped:=false -p speed:=0.1 -p turn:=0.5 \
  --remap cmd_vel:=/cmd_vel
```

- 最初は車輪を浮かせ、端末にフォーカスして `i`（前進）、`u` / `o`
  （前進しながら旋回）、`k`（停止）を確認してください。
- Jazzy 版はキー入力ごとに指令を送ります。移動キーを押し続け、
  OS のキーリピート間隔を 500 ms 未満にしてください。入力・通信が途絶えると
  本体の 500 ms watchdog が停止します。キーリピート開始前に一度停止する場合があります。
- 現在は前進専用です。後退・その場旋回・片輪逆転が必要な指令は
  両輪停止として扱います。速度や旋回速度を上げると `u` / `o` も拒否され得ます。
- Nav2 の `bringup.launch.py` や他の `/cmd_vel` 配信元を同時に起動しないでください。
- 最後に書き込んだものが自動で動く `motor_check` の場合は、先に
  通常ファームウェアを書き込んでください。

## RViz2・Nav2 の起動

`.docker/Dockerfile` の変更を反映するため開発コンテナを再ビルドし、ROS 2 ホストパッケージをビルドします。

```sh
cd ros2_ws
colcon build --symlink-install
source install/setup.bash
```

ローバーと micro-ROS Agent の接続後、地図を使わない odom 基準モードを起動します。

```sh
ros2 launch tenshi_no_hana_bringup bringup.launch.py
```

このモードは短距離・低速の相対目標用です。最初は車輪を浮かせて RViz2 の **2D Goal Pose** を試し、床上では 0.25〜0.5 m の目標から確認してください。Nav2 のキャンセルまたは速度出力停止後、ファームウェアの watchdog が 500 ms 以内にモーターを停止します。

HC-SR04 の検出範囲は機体前方の狭い範囲だけです。近距離障害物の検出には使えますが、自己位置推定、SLAM、全周衝突防止で LiDAR を代替できません。また、現在の odometry はエンコーダを持たず指令速度を積分するため、位置誤差が累積します。costmap を実運用する前に[ロボット記述](ros2_ws/src/tenshi_no_hana_bringup/urdf/tenshi_no_hana.urdf.xacro)の仮寸法を実測値へ変更してください。
[docs/DESIGN.md](docs/DESIGN.md) をハードウェアとシステム設計の正本として参照してください。

## 今後の拡張
- MQ-3B の取得・校正と磁界による液面測定
- 車輪エンコーダまたは外部自己位置推定の追加
- 自律マッピングと複数ローバー対応

## 旧設計の ESP32-Hosted (ESP32-C3) ピン割り当て

以下は旧設計の記録であり、現行 ESP32-C6 の設定ではありません。
[現行 GPIO 表](docs/DESIGN.md#ピンと電源)を参照してください。
以下は設計上のピン配置です。通信実績のある ESP-hosted slave のバージョンと
`sdkconfig` は未記録のため、slave を再ビルドする前に実機と照合してください。

> 参考: [Seeed Studio XIAO ESP32C3 Getting Started](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)

### XIAO ESP32C3 GPIO ↔ ピン対応

| XIAO ピン | GPIO   | 標準機能  | 備考                               |
| --------- | ------ | --------- | ---------------------------------- |
| D0        | GPIO2  | ADC       | ⚠️ ストラッピングピン               |
| D1        | GPIO3  | ADC       |                                    |
| D2        | GPIO4  | ADC       | MTMS (JTAG)                        |
| D3        | GPIO5  | ADC       | MTDI (JTAG)                        |
| D4        | GPIO6  | SDA (I2C) | FSPICLK, MTCK (JTAG)               |
| D5        | GPIO7  | SCL (I2C) | FSPID, MTDO (JTAG)                 |
| D6        | GPIO21 | UART TX   |                                    |
| D7        | GPIO20 | UART RX   |                                    |
| D8        | GPIO8  | SPI SCK   | ⚠️ ストラッピングピン               |
| D9        | GPIO9  | SPI MISO  | ⚠️ ストラッピングピン / BOOT ボタン |
| D10       | GPIO10 | SPI MOSI  | FSPICS0                            |

### 推奨 esp-hosted Slave 配線

> GPIO2/8/9 のストラッピングピンは避ける設計です。

| 信号       | GPIO   | XIAO ピン | 用途           |
| ---------- | ------ | --------- | -------------- |
| SPI MOSI   | GPIO7  | D5        | 必須           |
| SPI MISO   | GPIO5  | D3        | 必須           |
| SPI CLK    | GPIO6  | D4        | 必須           |
| SPI CS     | GPIO10 | D10       | 必須           |
| Handshake  | GPIO3  | D1        | タイミング同期 |
| Data Ready | GPIO4  | D2        | データ到着通知 |
| Reset      | GPIO21 | D6        | 推奨           |

#### 変更点

| 信号     | 旧 GPIO     | 新 GPIO      | 理由                     |
| -------- | ----------- | ------------ | ------------------------ |
| SPI MISO | GPIO2（D0） | GPIO5（D3）  | ストラップピンの回避     |
| Reset    | なし        | GPIO21（D6） | ホストリセット制御を追加 |

#### 空きピン

| XIAO ピン | GPIO   | 備考                   |
| --------- | ------ | ---------------------- |
| D0        | GPIO2  | 予備だがストラップピン |
| D7        | GPIO20 | デバッグ用 UART RX     |
| D8        | GPIO8  | 予備だがストラップピン |
| D9        | GPIO9  | BOOT ボタンに注意      |

### ESP-IDF menuconfig 設定

```powershell
idf.py menuconfig
```

**Example Configuration → Bus Config → SPI Full-Duplex Configuration** で次を設定します:

| 設定項目          | 値  |
| ----------------- | --- |
| SPI MOSI (GPIO)   | 7   |
| SPI MISO (GPIO)   | 5   |
| SPI CLK (GPIO)    | 6   |
| SPI CS (GPIO)     | 10  |
| Handshake (GPIO)  | 3   |
| Data Ready (GPIO) | 4   |
| Reset pin (GPIO)  | 21  |

保存後にビルドします。

```bash
# esp-hosted-mcu の slave サンプル作成
idf.py create-project-from-example "espressif/esp_hosted:slave"
cd slave
idf.py set-target esp32c3
idf.py menuconfig
# MISO=5, MOSI=7, CLK=6, CS=10, HS=3, DR=4
# Reset GPIO=21
idf.py build flash
```

## サブモジュールセットアップ

RP2040 側の依存はサブモジュールで管理します。

```bash
git submodule update --init --recursive external/embassy external/micro_xrce_dds_rs
```
