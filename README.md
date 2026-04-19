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

| 部品名               | 具体品番・おすすめ                                   | 価格目安（税込） | 購入URL                                                                                                | 備考                                 |
| -------------------- | ---------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------ |
| 2WDロボットシャーシ  | FT-DC-002 / 2WD Mini Smart Robot Mobile Platform Kit | ¥1,900           | [秋月電子](https://akizukidenshi.com/catalog/g/g113651/)                                               | モーター付き、エンコーダ無し版もあり |
| Baker link. Dev      | -                                                    | ¥1,980           | [スイッチサイエンス](https://www.switch-science.com/products/10044)                                    | Embassy-rsでRust no_std完璧対応      |
| モータドライバ       | TB6612FNG モジュール or IC                           | ¥400〜1,395      | [Amazon](https://www.amazon.co.jp/-/en/Module-Bridge-Driver-TB6612FNG-100KHz/dp/B09N9Y9J29) または秋月 | Pico PWM直駆動に最適                 |
| エタノールセンサー   | MQ-3B (またはMQ-3モジュール)                         | ¥450             | [秋月電子](https://akizukidenshi.com/catalog/g/g116269/)                                               | 天使の分け前検知の主役               |
| 温湿度・気圧センサー | BME280 モジュール (AE-BME280)                        | ¥1,080〜1,380    | [秋月電子](https://akizukidenshi.com/catalog/g/g109421/)                                               | 蒸発量補正に必須                     |
| IMU (オプション)     | MPU6050 GY-521                                       | ¥300〜600        | Amazon / 秋月類似品                                                                                    | odom計算・姿勢推定用                 |
| 超音波センサー (×2)  | HC-SR04                                              | ¥300×2 = ¥600    | [秋月電子](https://akizukidenshi.com/catalog/g/g111009/)                                               | 障害物回避                           |
| 電源                 | 18650×2 + ホルダー + DC-DC降圧 or モバイルバッテリー | ¥800〜1,500      | Amazon / 秋月                                                                                          | 長時間稼働                           |
| その他               | ジャンパワイヤ、ネジ、ユニバーサル基板など           | ¥500〜1,000      | 秋月 / Amazon                                                                                          | 配線・固定用                         |


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
- bakerlink.dev  
- X: [@BakerlinkLab](https://x.com/BakerlinkLab)

---

「天使の鼻で、ウィスキーの息吹を嗅ぐ。」🥃✨

このREADMEは自由にカスタマイズしてください。  
配線図・回路図・キャリブレーション方法・ROS2パッケージが完成したら、随時追加していきましょう！

## ESP32-Hosted（ESP32-C3）
`sdkconfig` から、現在のピンアサインを抜き出しました。
### Seeed Studio XIAO ESP32C3 の esp-hosted Slave ピン割り当て

| 信号名         | GPIO  | XIAO ESP32C3 物理ピン | 用途                       |
| -------------- | ----- | --------------------- | -------------------------- |
| **SPI MOSI**   | 7     | **D5**                | 必須                       |
| **SPI MISO**   | 2     | **D0**                | 必須                       |
| **SPI CLK**    | 6     | **D4**                | 必須                       |
| **SPI CS**     | 10    | **D10**               | 必須                       |
| **Handshake**  | 3     | **D1**                | 重要（タイミング同期）     |
| **Data Ready** | **4** | **D2**                | 重要（データ到着通知）     |
| **Reset**      | -1    | （未使用）            | ホスト側で任意のピンを使う |

**重要ポイント**:
- **Data Ready** が **GPIO4（D2）** になっています（デフォルトの9ではありません）。
- **Reset** は `-1`（無効）になっています。Rust側（ホストMCU）からリセット制御したい場合は、後でmenuconfigでGPIOを指定してください。

**Resetピンを有効にする**（おすすめ）
   menuconfigでResetピンを設定しましょう。
   ```powershell
   idf.py menuconfig
   ```
   → Example Configuration → Bus Config → SPI Full-Duplex Configuration → **Reset pin** で任意のGPIO（例: GPIO5 = D3）を指定 → 保存 → 再ビルド・フラッシュ。


