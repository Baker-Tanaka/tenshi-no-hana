---
applyTo: "examples/wifi_*.rs,src/wifi_config.rs"
---

# esp-hosted-mcu 学習ノート

## 概要

`embassy-net-esp-hosted-mcu` は RP2040 ↔ XIAO ESP32-C3 間の SPI 通信ドライバー。
FW バリアント: **ESP-Hosted-MCU** (`espressif/esp-hosted-mcu` リポジトリ)。
FG/NG とは別物。混同しないこと。

## ESP32-C3 FW バージョン

- 使用中: **v2.12.6** (ESP-IDF v5.5.4)
- FW プロジェクト名: `network_adapter`
- リポジトリ: `espressif/esp-hosted-mcu` (main ブランチ)
- Rust ドライバー: `external/embassy/embassy-net-esp-hosted-mcu` (WIP, March 2026)

## SPI 設定

| 項目 | 値 |
|------|-----|
| Mode | **3** (CPOL=1, CPHA=1) |
| Polarity | `IdleHigh` |
| Phase | `CaptureOnSecondTransition` |
| Frequency | 10 MHz |
| Handshake (GP15) | `Pull::Down` ← `Pull::Up` は NG |
| DataReady (GP13) | `Pull::Down` |

## ピン割り当て

| RP2040 | Signal | ESP32-C3 |
|--------|--------|----------|
| GP16 | MISO | GPIO5 |
| GP17 | CS | GPIO10 |
| GP18 | SCK | GPIO6 |
| GP19 | MOSI | GPIO7 |
| GP15 | Handshake | GPIO3 |
| GP13 | Data Ready | GPIO4 |
| GP14 | Reset | GPIO21 |

## インターフェースタイプ (ESP-Hosted-MCU Header)

| 値 | 名前 | 説明 |
|----|------|------|
| 0 | ESP_INVALID_IF | 無効 |
| 1 | ESP_STA_IF | Wi-Fi Station フレーム |
| 2 | ESP_AP_IF | SoftAP フレーム |
| 3 | ESP_SERIAL_IF | RPC 制御フレーム |
| 4 | ESP_HCI_IF | Bluetooth HCI |
| **5** | **ESP_PRIV_IF** | **プライベート通信 (スレーブ↔ホスト内部)** |
| 6 | ESP_TEST_IF | スループットテスト |

> **`unknown iftype 5` の警告は正常。** `ESP_PRIV_IF` はドライバーが処理しなくて良い。

## シリアルパケット TLV ヘッダー形式 (SERIAL_MSG_HEADER_LEN = 12)

```
bytes[0..10]:  \x01\x06\x00RPCEvt\x02  (Event) or
               \x01\x06\x00RPCRsp\x02  (Response)
bytes[10..12]: payload_len (u16 LE)
bytes[12..]:   protobuf Rpc message
```

## Protobuf Rpc メッセージ

```protobuf
message Rpc {
    RpcType msg_type = 1;   // 1=Req, 2=Resp, 3=Event
    RpcId   msg_id   = 2;   // e.g. 769=Event_ESPInit, 770=Event_Heartbeat
    uint32  uid      = 3;
    oneof payload { ... }   // field number = RpcId value
}
```

### 重要な RpcId 値

| 値 | 名前 |
|----|------|
| 769 (0x301) | Event_ESPInit |
| 770 (0x302) | Event_Heartbeat |
| 771 | Event_StaConnected |
| 772 | Event_StaDisconnected |

## 初期化シーケンス

1. `runner.run()` 開始 → ESP32 リセット
2. ESP32 起動後、`Event_ESPInit` (iftype=3, RPCEvt, field 769) を送信
3. RP2040 ドライバーが受信 → `init_done()` → `control.init()` の await 解除
4. `set_heartbeat(10)` → 10秒間隔で heartbeat
5. `set_wifi_init_config` → `set_wifi_mode` → `start_wifi`
6. `control.connect(ssid, password)` → `EventStaConnected` 待ち

## heartbeat タイムアウト

- `HEARTBEAT_MAX_GAP = 20秒`
- `events_notifier = None` の場合、タイムアウトで **panic!("heartbeat from esp32 stopped")**
- 初期化中 (`ControlState::Reboot`) はタイムアウトを延長する

## よくあるエラーと原因

### `panicked at 'heartbeat from esp32 stopped'`

原因の優先順位:
1. **`failed to parse event`** が先に出ている場合 → EventEspInit のデコード失敗
   - protobuf のバイト列をログ確認 (`debug!` 有効時: `serial rx:` のログ)
   - `bad tlv` が出る場合 → FW が送るヘッダー形式の不一致
2. **SPI 通信が全く届いていない** → 配線確認、Handshake/DataReady の Pull 確認
3. **チェックサムエラー** → `rx: bad checksum` のログが出る

### `unknown iftype 5`

正常。`ESP_PRIV_IF` は無視して良い。

### `failed to parse event: N bytes: [...]`

protobuf デコード失敗。出力されたバイト列で診断:
- `08 03` が先頭近くにあれば field 1 = Event type ✓
- `10 81 06` が続けば field 2 = RpcId 769 = ESPInit ✓
- 全く違うバイト列 → FW バージョンと proto スキーマの不一致

## デバッグログ強化 (適用済み)

`external/embassy/embassy-net-esp-hosted-mcu/src/lib.rs` に以下を適用:
- `handle_rx`: `trace!` → `debug!` (RX バイト列を debug レベルで出力)
- `handle_event`: 失敗時に **`DecodeError` variant 名**・`data.len()` と `HexSlice(data)` を出力
  - `VarIntLimit / UnexpectedEof / Capacity / WrongLen / Utf8 / ZeroField` など
- `event without payload`: `msg_type` / `msg_id` の値も出力
- `serial rx`: `trace!` → `debug!`
- `serial rx: bad tlv`: TLV 先頭バイトを出力

`DEFMT_LOG = "debug"` (.cargo/config.toml) で全て出力される。

## handle_event スクラッチ手書きパーサー (適用済み / 最終形)

**問題**: `Rpc`（micropb生成コード）を使うと **二重にスタック汚染**する：
1. `let mut event = Rpc::default()` → ~1300 bytes をスタックに確保
2. `Rpc::decode` の内部: `Some(Rpc_::Payload::EventEspInit(Default::default()))` で
   `Rpc_::Payload` enum (~1300 bytes) の**一時値がスタックに生成**される
   → `event_buf: Rpc` を static に移しても ② が残るため `VarIntLimit` が継続

**解決**: `Rpc` / `micropb` を `handle_event` から完全除去し、
`pb_read_varint` / `pb_skip_field` の手書き minimal パーサーに置き換え。

```rust
// lib.rs の handle_event: allocations ゼロ、Rpc struct 不使用
fn handle_event(&mut self, data: &[u8]) {
    let mut pos = 0usize;
    let mut msg_id: u32 = 0;
    // pass 1: find field 2 (msg_id)
    while pos < data.len() { /* pb_read_varint / pb_skip_field */ }
    // pass 2: find field msg_id (payload sub-message), extract resp (field 1)
    match msg_id {
        769 => { /* EventEspInit */   self.shared.init_done(); ... }
        770 => { /* EventHeartbeat */ self.reset_heartbeat_deadline(); }
        775 => { /* EventStaConnected */    ... }
        776 => { /* EventStaDisconnected */ ... }
        _   => { debug!("unknown event msg_id={}", msg_id); }
    }
}
```

**RpcId 定数** (proto.rs L55761):
| RpcId | 値 | 意味 |
|-------|----|------|
| EventEspInit | 769 | 初期化完了 |
| EventHeartbeat | 770 | ハートビート |
| EventStaConnected | 775 | STA 接続完了 |
| EventStaDisconnected | 776 | STA 切断 |

**Protobuf 構造**:
- `Rpc { msg_type=field1, msg_id=field2, uid=field3, payload=field<msg_id>(LEN) }`
- payload 内の `resp` = field1 (varint, i32)

## Cargo.toml 設定

```toml
[dependencies]
embassy-net-esp-hosted-mcu = {
    path = "external/embassy/embassy-net-esp-hosted-mcu",
    features = ["defmt"],
    optional = true
}

[patch.crates-io]
embassy-net-driver = { path = "external/embassy/embassy-net-driver" }
embassy-net-driver-channel = { path = "external/embassy/embassy-net-driver-channel" }
embassy-time-driver = { path = "external/embassy/embassy-time-driver" }
# embassy-net-esp-hosted-mcu は path dep なので patch 不要
```

## FG variant との主な違い

| | `embassy-net-esp-hosted` (FG) | `embassy-net-esp-hosted-mcu` (MCU) |
|---|---|---|
| Proto | `CtrlMsg` | `Rpc` |
| `new()` 第4引数 | なし | `events_notifier: Option<&'static Signal<...>>` |
| `runner.run()` | 引数なし | `(tx_buf, rx_buf)` 外部バッファ必要 |
| `control.init()` | 引数なし | `EspConfig` 必須 |
| `control.connect()` | `Result<(), Error>` | `Result<bool, Error>` |
| InterfaceType::Sta | 0 | 1 |
| InterfaceType::Serial | 2 | 3 |
| EventEspInit field# | 301 | 769 |
