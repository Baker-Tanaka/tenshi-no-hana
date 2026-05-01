# 天使の鼻 - ROS2 × Baker link. Dev × ローバー

> ⚠️ 本プロジェクトは現在開発途中です。機能、配線、ドキュメントは変更される可能性があります。

**ウィスキー樽の蒸発成分、環境データ、液面を同時に監視する巡回ロボットプロジェクト。**

![プロジェクトプレビュー](docs/img/image.png)

![システム構成](docs/img/tenshi_no_hana_rover_v2.svg)

> English README is available at `README.md`.

## 概要
天使の鼻は、Baker link. Dev(RP2040) 基板を中心に構成された 2輪ローバーです。蒸留所や熟成庫を巡回し、エタノール蒸気濃度、温度、湿度、気圧を測定し、磁界センサーで樽内の液面の変化を推定します。

本プロジェクトは以下を組み合わせています。
- **Baker link. Dev (RP2040)**
- **XIAO-ESP32-C3 の Wi-Fi / ESP-hosted MCU**
- **Embassy-rs no_std 非同期ランタイム**
- **zenoh-ros2-nostd** による ROS2 互換メッセージング

## 主な特徴
- エタノール蒸気を検知し「天使の分け前」を間接的に監視
- 温湿度・気圧で環境補正を行う
- 浮遊磁石＋磁界検出で樽内液面を非接触測定
- XIAO-ESP32-C3 で Wi-Fi 通信を実現
- 低コストかつ no_std 組み込み設計

## ハードウェア構成
2026年4月時点の日本国内向け概算です。

| 部品                 | 例                                                   | 価格目安    | 購入先                                                              | 備考                               |
| -------------------- | ---------------------------------------------------- | ----------- | ------------------------------------------------------------------- | ---------------------------------- |
| 2WD ロボットシャーシ | FT-DC-002 / 2WD Mini Smart Robot Mobile Platform Kit | ¥1,900      | [秋月電子](https://akizukidenshi.com/catalog/g/g113651/)            | モーター付き、エンコーダ無版もあり |
| Baker link. Dev      | -                                                    | ¥1,980      | [スイッチサイエンス](https://www.switch-science.com/products/10044) | Embassy-rs 対応                    |
| モータドライバ       | DRV8835 デュアルモータドライバ                       | ¥400〜1,395 | [秋月電子](https://akizukidenshi.com/catalog/g/g109848/)            | PWM 直接駆動に最適                 |
| エタノールセンサー   | MQ-3B / MQ-3 モジュール                              | ¥450        | [秋月電子](https://akizukidenshi.com/catalog/g/g116269/)            | 天使の分け前検知に使用             |
| 環境センサー         | BME280 モジュール                                    | ¥1,650      | [スイッチサイエンス](https://www.switch-science.com/products/2236)  | 蒸発補正に必須                     |
| オプション IMU       | 6軸 IMU センサーモジュール                           | ¥990        | [スイッチサイエンス](https://www.switch-science.com/products/8695)  | 走行 odom / 姿勢推定用             |
| 超音波センサー       | -                                                    | ¥300        | [スイッチサイエンス](https://www.switch-science.com/products/8224/) | 障害物回避用                       |

```
GPIOs: CLK:6 MOSI:7 MISO:5 CS:10 HS:3 DR:4
```

## 配線図

![](docs/schematics/baker_link_esp32c3_spi.svg)

## 無線通信
- **XIAO-ESP32-C3** を ESP-Hosted MCU として使用
- `external/embassy` サブモジュールに含まれる `embassy-net-esp-hosted` を利用

## ソフトウェア構成
- `zenoh-ros2-nostd`
- カスタムメッセージ `angel_nose_msgs`（エタノール濃度、液面高さ、環境データ）

## 設置・運用の流れ
1. 樽横に一定距離でローバーを設置し、姿勢を固定する
2. 浮遊コルク＋ネオジム磁石で磁界変化から液面を推定する
3. MQ-3 センサーで周囲のエタノール蒸気を検知する

## 今後の拡張
- 複数樽を自動巡回するマッピング
- 液面データから蒸発量を推定する解析
- 蒸留所向け Web ダッシュボード
- 完全 Rust no_std 実装の完成

## ESP32-Hosted (ESP32-C3) ピン割り当て
現在の `sdkconfig` に基づくピン配置です。

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
git submodule update --init --recursive external/embassy external/zenoh_ros2_nostd
```
