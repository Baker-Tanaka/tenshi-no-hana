# 天使の鼻 (Angel's Nose) - ROS2 × Pico 2WD巡回ローバー

**ウィスキー樽の「天使の分け前」を嗅ぎながら、樽の中の水位まで監視する天使の鼻型巡回ロボット**

※完成イメージ

![完成イメージ](docs/img/image.png)


（画像：Baker link. Dev + MQ-3 + BME280搭載のコンパクト2WDローバー。倉庫のウィスキー樽の中でエタノール蒸気を嗅ぎ回る姿）

## コンセプト
「天使の分け前（Angel's Share）」から派生した **「天使の鼻（Angel's Nose）」** プロジェクト。  
2輪ローバーが蒸留所・熟成庫内を自律巡回し、空気中のアルコール濃度（エタノール蒸気）、温湿度、気圧を計測。さらに磁界センサーを使って樽内の液面（ウィスキーの減り具合）を非接触で測定します。

**bakerlink.dev + ROS2 + Embassy-rs (Rust)** を活用した、軽量・低コスト・無線対応のスマート監視システムです。

## 主な特徴
- エタノール蒸気（天使の分け前）検知
- 温湿度・気圧センシング（蒸発量補正用）
- 磁界センサーによる樽内液面（水位）測定（浮遊ネオジム磁石方式）
- ROS2（またはEmbassy-rs）による制御・ナビゲーション
- XIAO-ESP32-C3によるWi-Fi/無線通信
- 低コスト（本体6,500〜9,500円程度）

## ハードウェア BOM（最安志向・日本国内中心・2026年4月時点）

| 部品名               | 具体品番・おすすめ                                   | 価格目安（税込） | 購入URL                                                             | 備考                                 |
| -------------------- | ---------------------------------------------------- | ---------------- | ------------------------------------------------------------------- | ------------------------------------ |
| 2WDロボットシャーシ  | FT-DC-002 / 2WD Mini Smart Robot Mobile Platform Kit | ¥1,900           | [秋月電子](https://akizukidenshi.com/catalog/g/g113651/)            | モーター付き、エンコーダ無し版もあり |
| Baker link. Dev      | -                                                    | ¥1,980           | [スイッチサイエンス](https://www.switch-science.com/products/10044) | Embassy-rsでRust no_std完璧対応      |
| モータドライバ       | デュアルモータードライバDRV8835                      | ¥400〜1,395      | [秋月電子](https://akizukidenshi.com/catalog/g/g109848/)            | PWM直駆動に最適                      |
| エタノールセンサー   | MQ-3B (またはMQ-3モジュール)                         | ¥450             | [秋月電子](https://akizukidenshi.com/catalog/g/g116269/)            | 天使の分け前検知の主役               |
| 温湿度・気圧センサー | BME280 モジュール (AE-BME280)                        | ¥1,080〜1,380    | [秋月電子](https://akizukidenshi.com/catalog/g/g109421/)            | 蒸発量補正に必須                     |
| IMU (オプション)     | 6軸IMUセンサーモジュール                             | ¥990             | [秋月電子](https://akizukidenshi.com/catalog/g/g130950/)            | odom計算・姿勢推定用                 |
| 超音波センサー       | HC-SR04                                              | ¥300             | [秋月電子](https://akizukidenshi.com/catalog/g/g111009/)            | 障害物回避                           |


## 無線通信
- **XIAO-ESP32-C3** を使用（ESP-Hosted-MCU）
- [embassy-net-esp-hosted](https://github.com/embassy-rs/embassy/tree/embassy-net-esp-hosted-v0.3.0/embassy-net-esp-hosted) でRust/Embassy環境から快適にWi-Fi接続可能

## ソフトウェア構成
- **zenoh-ros2-nostd**
- カスタムメッセージ：`angel_nose_msgs`（エタノール濃度、液面高さ、環境データ）

## 設置・測定方法
1. 樽の横に固定距離で横付け（ArUcoマーカー or AprilTag推奨）
2. 浮遊コルク＋ネオジム磁石で液面を磁場強度として検知（ホールセンサー or 3軸磁力計使用）
3. MQ-3で周囲エタノール蒸気濃度を「鼻」で嗅ぐ

## 今後の拡張予定
- 複数樽自動巡回マッピング
- 液面データから天使の分け前蒸発量の推定
- Webダッシュボード（蒸留所監視用）
- 完全Rust no_std実装

---

**関連リンク**：  
- X: [@BakerlinkLab](https://x.com/BakerlinkLab)

---

「天使の鼻で、ウィスキーの息吹を嗅ぐ。」🥃✨
配線図・回路図・キャリブレーション方法・ROS2パッケージが完成したら、随時追加していきます！

## ESP32-Hosted（ESP32-C3）

`sdkconfig` から、現在のピンアサインを抜き出しました。

> **参考**: [Seeed Studio XIAO ESP32C3 Getting Started](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)

### XIAO ESP32C3 GPIO ↔ 物理ピン対応表

| XIAO ピン | GPIO   | デフォルト機能 | 備考                                  |
| --------- | ------ | -------------- | ------------------------------------- |
| **D0**    | GPIO2  | ADC            | ⚠️ **ストラッピングピン**              |
| **D1**    | GPIO3  | ADC            |                                       |
| **D2**    | GPIO4  | ADC            | MTMS (JTAG)                           |
| **D3**    | GPIO5  | ADC            | MTDI (JTAG)                           |
| **D4**    | GPIO6  | **SDA (I2C)**  | FSPICLK, MTCK (JTAG)                  |
| **D5**    | GPIO7  | **SCL (I2C)**  | FSPID, MTDO (JTAG)                    |
| **D6**    | GPIO21 | TX (UART)      |                                       |
| **D7**    | GPIO20 | RX (UART)      |                                       |
| **D8**    | GPIO8  | SPI SCK        | ⚠️ **ストラッピングピン**              |
| **D9**    | GPIO9  | SPI MISO       | ⚠️ **ストラッピングピン / BOOTボタン** |
| **D10**   | GPIO10 | SPI MOSI       | FSPICS0                               |

### Seeed Studio XIAO ESP32C3 の esp-hosted Slave ピン割り当て（推奨構成）

> ストラッピングピン（GPIO2/8/9）を **すべて回避** した安全な構成です。

| 信号名         | GPIO   | XIAO 物理ピン | 接続方式        | 用途                   |
| -------------- | ------ | ------------- | --------------- | ---------------------- |
| **SPI MOSI**   | GPIO7  | **D5**        | FSPID（専用）   | 必須                   |
| **SPI MISO**   | GPIO5  | **D3**        | GPIO Matrix     | 必須                   |
| **SPI CLK**    | GPIO6  | **D4**        | FSPICLK（専用） | 必須                   |
| **SPI CS**     | GPIO10 | **D10**       | FSPICS0（専用） | 必須                   |
| **Handshake**  | GPIO3  | **D1**        | GPIO            | 重要（タイミング同期） |
| **Data Ready** | GPIO4  | **D2**        | GPIO            | 重要（データ到着通知） |
| **Reset**      | GPIO21 | **D6**        | GPIO            | 推奨（リセット制御）   |

#### 旧構成からの変更点

| 信号名   | 旧 GPIO      | 新 GPIO      | 変更理由                                    |
| -------- | ------------ | ------------ | ------------------------------------------- |
| SPI MISO | GPIO2（D0）⚠️ | GPIO5（D3）  | GPIO2 はストラッピングピン → 起動不安定回避 |
| Reset    | -1（無効）   | GPIO21（D6） | ホスト側からのリセット制御を有効化          |

#### 空きピン

| XIAO ピン | GPIO   | 状態                                 |
| --------- | ------ | ------------------------------------ |
| **D0**    | GPIO2  | 空き（⚠️ ストラッピング：未使用推奨） |
| **D7**    | GPIO20 | 空き（UART RX — デバッグ用に確保）   |
| **D8**    | GPIO8  | 空き（⚠️ ストラッピング：未使用推奨） |
| **D9**    | GPIO9  | 空き（⚠️ BOOTボタン：未使用推奨）     |

### 注意事項

1. **I2C デフォルトピンは SPI で占有**
   - D4（GPIO6）= SDA、D5（GPIO7）= SCL は XIAO 標準の I2C ピンですが、SPI CLK / MOSI で使用しています。
   - I2C センサー（BME280 等）はすべて **ホスト側（Baker link. Dev）の I2C** に接続してください。ESP32-C3 側に I2C デバイスは接続しません。

2. **JTAG デバッグ不可**
   - GPIO4〜7（D2〜D5）は JTAG ピン（MTMS/MTDI/MTCK/MTDO）ですが、SPI + 信号線で使い切っています。ESP32-C3 の JTAG デバッグはこの構成では使用できません。USB シリアル経由でのデバッグを使用してください。

3. **Data Ready のデフォルト GPIO9 は使用不可**
   - XIAO ESP32C3 では GPIO9 が **BOOT ボタン** に接続されているため、Data Ready は GPIO4（D2）に変更しています。

### menuconfig での設定手順

```powershell
idf.py menuconfig
```

→ **Example Configuration → Bus Config → SPI Full-Duplex Configuration** で以下を設定：

| 設定項目          | 値     |
| ----------------- | ------ |
| SPI MOSI (GPIO)   | **7**  |
| SPI MISO (GPIO)   | **5**  |
| SPI CLK (GPIO)    | **6**  |
| SPI CS (GPIO)     | **10** |
| Handshake (GPIO)  | **3**  |
| Data Ready (GPIO) | **4**  |
| Reset pin (GPIO)  | **21** |

→ 保存 → 再ビルド・フラッシュ。


