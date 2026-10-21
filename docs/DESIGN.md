# 天使の鼻 - 現行システム設計

## 対象と責務

現行の完成対象は、RP2040 の走行制御・IMU・超音波測距・open-loop
odometry と、micro-ROS Agent を介した ROS 2 / Nav2 連携である。
エタノール計測、液面推定、地図ベースの自律巡回、Web ダッシュボードは
別段階とする。回路に候補センサーが存在しても、実装済みとは扱わない。

```text
RViz2 / Nav2 (ROS 2 Jazzy)
       |
micro-ROS Agent (TCP :8888)
       |
Wi-Fi / ESP32-C6 (ESP-hosted MCU slave 2.12.9)
       |
SPI / Baker link. Dev (RP2040)
       +-- DRV8835: モーター
       +-- ICM-20602: 加速度・角速度
       +-- HC-SR04: 前方距離
```

| 層 | 責務 |
| --- | --- |
| `external/embassy` | HAL、非同期実行、TCP/IP、ESP-hosted MCU ドライバ。原則変更しない |
| `external/micro_xrce_dds_rs` | Rust の組み込み ROS フレームワーク。XRCE/CDR、ノード、型付き配送、接続状態、時計 |
| 同リポジトリの `codegen/` | ホスト専用の ROS IDL メッセージ生成 |
| `generated/` | ROS インストールなしで Cargo から使える生成済み `no_std` crate |
| `src/board.rs`、`src/drivers/` | ボードリソースの所有権とデバイス操作 |
| `src/nodes/` | ローバー固有の周期処理、topic、モーター制限 |
| `src/launch.rs` | 組み込みノード構成と起動順 |
| `src/network.rs` | ボード固有 Wi-Fi 設定、Agent 接続先、再試行方針 |
| `ros2_ws/src/tenshi_no_hana_bringup` | URDF、RViz2、odom 基準 Nav2 |

フレームワーク修正は submodule 内に保存される。配布時には
`micro_xrce_dds_rs` 側の変更を先に commit / push してから、親リポジトリの
submodule 参照を更新する必要がある。親の commit だけでは submodule 内の
未コミット変更を配布できない。

## メッセージ生成

[ros2_rust の message generation](https://github.com/ros2-rust/ros2_rust/blob/main/docs/message-generation.md)
にならい、ホスト側生成器と Rust 実行時サポートを分離する。

```text
ROS .msg
   -> 公式 rosidl_adapter
   -> IDL
   -> 公式 rosidl_parser
   -> パッケージごとの no_std Rust crate
   -> Cargo / XRCE-DDS
```

ament / colcon 側で生成し、生成結果もバージョン管理する。
通常のファームウェア `cargo build` には ROS、Python、C の RMW / typesupport
ライブラリを要求しない。生成手順・対応型・制約の正本は
[生成器の説明](../external/micro_xrce_dds_rs/codegen/README.md)とする。

ROS のメッセージ定義と同じ DDS 型名・CDR ワイヤー表現を用い、デバイス内の
文字列・sequence は固定容量にする。本ローバーでは無制限フィールドの
既定上限を文字列 32 バイト、sequence 1 要素とする。これは ROS 型そのものの
変更ではなく、デバイス側の受理上限である。上限超過や不正な CDR を黙って
切り詰めて成功扱いにしてはいけない。

新しいメッセージや容量変更では生成器を再実行し、生成結果の差分と
シリアライズ互換性を検証する。IDL 生成はファームウェア build script の
責務ではない。`build.rs` はリンカ検索パスと Wi-Fi 設定の検証・埋め込みを行う。

## ノードと ROS インターフェース

| ノード | 方向 | topic | ROS 型 | 周期 |
| --- | --- | --- | --- | --- |
| motor | 購読 | `/cmd_vel` | `geometry_msgs/Twist` | 受信時 |
| imu | 配信 | `/imu/data` | `sensor_msgs/Imu` | 約 20 ms |
| range | 配信 | `/ultrasonic/front` | `sensor_msgs/Range` | 測定後 100 ms 待機 |
| odom | 配信 | `/odom` | `nav_msgs/Odometry` | 約 100 ms |
| odom | 配信 | `/tf` | `tf2_msgs/TFMessage` | 約 100 ms |

- ローカルのタスクは Wi-Fi / Agent 接続を待たずに動作する。
- IMU は加速度・角速度のみ。姿勢推定は行わず、orientation covariance の
  先頭を -1 とする。
- range は測定失敗・範囲外を正の無限大として配信する。
- odometry は車輪エンコーダではなく、duty 制限後の指令速度を積分する。
  IMU との融合は行わない。
- `odom -> base_link` はファームウェア、固定センサー TF はホストの
  `robot_state_publisher` が担当する。
- Publisher の成功はデバイス内の受理であり、Agent の配送 ACK ではない。
  センサーデータは最新値を優先し、ネットワーク遅延で計測タスクを止めない。
- 時刻同期前の timestamp は起動後の単調増加時刻であり、Unix 時刻ではない。

## モーター制御の契約

- DRV8835 は PH/EN モード。MODE を VCC へ接続する。
- 車輪間隔は 0.13 m、100% duty 換算の最大車輪速度は 0.4 m/s。
  実機校正前の値であり、実測値に合わせる必要がある。
- duty の絶対値を 0.6 以下に制限する。
- 非有限値、切断、接続世代変更、期限切れ指令では停止する。
- watchdog は 500 ms。キューから取り出した時刻ではなく、デバイスが
  指令を受理した時刻を基準とする。
- 再接続時に旧接続の指令を再生しない。新しい接続で新たな有効指令が
  届くまで停止する。
- 通信障害後もローカル計測は継続する。ただし MCU 自体の故障を
  ソフトウェア watchdog の保証範囲に含めない。

## ピンと電源

### RP2040 と ESP32-C6（現行実機）

| RP2040 | ESP32-C6 GPIO | 信号 |
| --- | --- | --- |
| GP16 | GPIO21 | SPI MISO |
| GP17 | GPIO18 | SPI CS |
| GP18 | GPIO22 | SPI CLK |
| GP19 | GPIO23 | SPI MOSI |
| GP15 | GPIO1 | Handshake |
| GP13 | GPIO2 | Data Ready |
| GP14 | GPIO16 | Slave Reset（EN ではない） |

SPI は 10 MHz、CPOL=1、CPHA=1。起動ログで ESP32-C6、
ESP-Hosted MCU slave 2.12.9 と上記 GPIO 設定を確認し、実配線も確認済み。
SPI 初期化、ESPInit、Wi-Fi 接続、DHCP アドレス取得まで実機で確認した。
過去の XIAO ESP32-C3 用の表や回路図を現行実機へ流用しないこと。
slave の更新前にバージョン・設定・実配線を保存する。

### センサーとモーター

| RP2040 | 接続先 |
| --- | --- |
| GP4 / GP5 | ICM-20602 SDA / SCL、アドレス 0x68 または 0x69 |
| GP2 / GP3 | HC-SR04 Trigger / Echo |
| GP10 / GP11 | 左モーター PHASE / PWM5_B ENABLE |
| GP12 / GP9 | 右モーター PHASE / PWM4_B ENABLE |
| GP26 | 将来の MQ-3B ADC。現行コードでは未使用 |

9 V 電池から BP5293-50 で 5 V、NJU7223F33 で 3.3 V を生成する設計。
USB 接続時には電池を切り離す。RP2040 は 5 V 非耐圧なので HC-SR04 Echo
および将来の MQ-3B AOUT は 10 kΩ / 15 kΩ 分圧を通す。
実配線・電源条件は [日本語 README](../README.ja.md) と回路図も確認する。

## ホスト側ナビゲーション

ROS 2 Jazzy。Nav2 の global frame は `odom`、base frame は `base_link`。
地図・AMCL・SLAM は使用しない。HC-SR04 を RangeSensorLayer に入力し、
rolling costmap と DWB controller で短距離・低速の相対目標を扱う。

車輪エンコーダがないので位置誤差は累積する。前方の狭い範囲だけを見る
超音波センサーは全周衝突回避や自己位置推定を保証しない。
URDF の寸法・costmap footprint は仮値なので、実運用前に実測する。

## 検証と完了基準

1. ROS のない通常の Cargo 環境でも、生成済みメッセージを使って
   `cargo build` と `cargo build --release` が完了する。
2. 生成器のテストで公式 IDL 入力、固定容量、生成の再現性、
   CDR シリアライズの境界条件を確認する。
3. フレームワークのホストテストで配送、指令鮮度、切断・再接続、
   フレーム分割受信、時計を検証する。
4. ROS 2 パッケージを colcon で build / test し、URDF と topic / frame 契約を検証する。
5. 実機では車輪を浮かせ、Wi-Fi / DHCP / Agent 接続、全 topic、
   指令停止後の watchdog、Agent 再起動後の復旧を確認する。
   コンパイル成功やデバッグセッションの存在だけで実機動作済みとしない。

実機の書き込み・リセットは明示的な確認後に行う。
