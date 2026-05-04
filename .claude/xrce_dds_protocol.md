# XRCE-DDS ワイヤフォーマット — 真実の所在

> このドキュメントは `external/micro_xrce_dds_rs` を編集するときに必ず参照してください。OMG DDS-XRCE 仕様 PDF と eProsima 実装は乖離しており、**micro-ROS Agent は PDF ではなく eProsima 実装に従います**。

## 真の出所

- リポジトリ: <https://github.com/eProsima/Micro-XRCE-DDS-Client>
- 主要ファイル:
  - `src/c/core/session/submessage_internal.h` — submessage ID
  - `src/c/core/serialization/xrce_header_internal.h` — `SESSION_ID_WITHOUT_CLIENT_KEY = 0x80`
  - `src/c/core/serialization/xrce_types.c` — 全 serializer
  - `src/c/core/session/session_info.c` — `uxr_buffer_create_session`, `uxr_stamp_create_session_header`
  - `src/c/core/session/common_create_entities.c` — CREATE payload size 計算
  - `src/c/core/session/write_access.c` — WRITE_DATA 構築
- 補助: `include/uxr/client/core/type/xrce_types.h` — struct 定義

OMG PDF を信頼してはいけない（`client_timestamp` が定義されているが eProsima は送らない、submessage ID 番号が違う等）。

---

## TCP framing

```
[len: u16 LE][message: len bytes]
```

これだけ。CRC なし、エスケープなし（それらは UART transport のもの）。1 メッセージ = 1 length-prefixed フレーム。

---

## Submessage ID（eProsima 正規）

| ID  | Name            | 用途               |
| --- | --------------- | ------------------ |
| 0   | CREATE_CLIENT   | クライアント登録   |
| 1   | CREATE          | エンティティ生成   |
| 2   | GET_INFO        |                    |
| 3   | DELETE          |                    |
| 4   | STATUS_AGENT    | CREATE_CLIENT 応答 |
| 5   | STATUS          | CREATE/DELETE 応答 |
| 6   | INFO            |                    |
| 7   | WRITE_DATA      | publish            |
| 8   | READ_DATA       | subscribe 要求     |
| 9   | DATA            | subscribe 受信     |
| 10  | ACKNACK         |                    |
| 11  | HEARTBEAT       |                    |
| 12  | RESET           |                    |
| 13  | FRAGMENT        |                    |
| 14  | TIMESTAMP       |                    |
| 15  | TIMESTAMP_REPLY |                    |

### Submessage header（4 byte、4-aligned 配置）

```
[id: u8][flags: u8][len: u16 LE]
```

- `flags & 0x01` = endianness（1 = LE）。LE プラットフォームで送るときは常に立てる
- `flags & 0x02` = LAST_FRAGMENT（FRAGMENT 用）。FORMAT_DATA = 0x00, FORMAT_SAMPLE = 0x02 と兼用
- CREATE では `0x06`（REUSE | REPLACE）も慣習的に立てる（idempotent 再生成のため）

`uxr_buffer_submessage_header` は呼ぶ前に `ucdr_align_to(ub, 4)` する。**メッセージバッファ内で常に 4-byte 境界に配置すること。** メッセージヘッダーは 4 または 8 byte なのでどちらも自動で揃う。

---

## メッセージヘッダー

```
session_id : u8
stream_id  : u8
seq_num    : u16 LE   ← 常に LE。endianness フラグの影響を受けない
client_key : [u8; 4]  ← session_id < 0x80 のときだけ
```

`SESSION_ID_WITHOUT_CLIENT_KEY = 0x80`:

| `session_id` | header size | key を含む？ |
| ------------ | ----------- | ------------ |
| `0x00–0x7F`  | 8 byte      | はい         |
| `0x80–0xFE`  | 4 byte      | いいえ       |

### CREATE_CLIENT メッセージだけ特殊

`uxr_stamp_create_session_header` は header の session_id を `info->id & 0x80` に書き換える:

| `info->id` | header sid | header size | header の key |
| ---------- | ---------- | ----------- | ------------- |
| `0x81`     | `0x80`     | 4 byte      | なし          |
| `0x01`     | `0x00`     | 8 byte      | あり          |

ただし **payload 内の `session_id` は元の `info->id` のまま**。Agent はこれで本来の session_id を学習する。

---

## CLIENT_Representation（CREATE_CLIENT の payload、16 byte）

```
xrce_cookie         : [u8; 4]  = "XRCE"           = 0x58 0x52 0x43 0x45
xrce_version        : [u8; 2]  = [0x01, 0x00]
xrce_vendor_id      : [u8; 2]  = [0x01, 0x0F]     (eProsima)
client_key          : [u8; 4]                     ← session_id より前
session_id          : u8
optional_properties : u8 (bool, 0 = なし)
[ properties      ]                               ← optional_properties=true のみ
mtu                 : u16 (CDR 2-aligned)
```

OMG PDF にある `client_timestamp` は **送らない**。

### 完全な CREATE_CLIENT バイト列（session_id=0x81、key=BACEA105、mtu=512）

```
TCP framing :  18 00                       len=24 LE
msg hdr     :  80 00 00 00                 sid=0x80, stream=0, seq=0
sub hdr     :  00 01 10 00                 id=0, flags=LE, len=16
payload     :  58 52 43 45                 "XRCE"
               01 00                       version 1.0
               01 0F                       eProsima
               BA CE A1 05                 client_key
               81                          session_id 0x81
               00                          optional_properties=false
               00 02                       mtu=512 LE
```

合計 26 byte（framing 込み）。

### Agent 応答 STATUS_AGENT（19 byte 中身、framing 込み 21 byte）

```
TCP framing :  13 00                       len=19
msg hdr     :  81 00 00 00                 echoed sid=0x81 (元の値)
sub hdr     :  04 01 0B 00                 STATUS_AGENT, LE, len=11
payload     :  00                          result.status (OK)
               00                          result.implementation_status
               58 52 43 45                 cookie
               01 00                       version
               01 0F                       vendor (eProsima)
               00                          optional_properties=false
```

`payload[0] == 0x00` を OK として確認する。

---

## CREATE submessage（エンティティ生成）

### Payload 構造

```
BaseObjectRequest:
  request_id : [u8; 2]   big-endian packed u16
  object_id  : [u8; 2]   big-endian (idx<<4)|kind の 16bit
ObjectVariant:
  kind       : u8        (Object kind と同じ値)
  representation:
    Representation3_Base or RepresentationBinAndXML_Base:
      format : u8        ← 0x02 = AS_XML（micro-ROS は常にこれ）
      [variant by format]:
        XML : ucdr_string  (uint32 len incl null + bytes + '\0')
    （エンティティ種別ごとに後続フィールドが続く）
```

`ObjectId` と `RequestId` は CDR primitive ではなく `ucdr_serialize_array_uint8_t` で書かれる **2 byte raw**。endianness の影響を受けない。実装上は `(idx<<4)|kind` の 16bit を big-endian で並べる（高バイトが先）。

### Object kind

| Hex  | Kind        | Variant 構成                                           |
| ---- | ----------- | ------------------------------------------------------ |
| 0x01 | PARTICIPANT | Representation3_Base + `int16` domain_id               |
| 0x02 | TOPIC       | Representation3_Base + ObjectId participant_id         |
| 0x03 | PUBLISHER   | RepresentationBinAndXML_Base + ObjectId participant_id |
| 0x04 | SUBSCRIBER  | RepresentationBinAndXML_Base + ObjectId participant_id |
| 0x05 | DATAWRITER  | Representation3_Base + ObjectId publisher_id           |
| 0x06 | DATAREADER  | Representation3_Base + ObjectId subscriber_id          |

> **重要**: 既存コードで `ENTITY_DATAWRITER = 0x04` だったが、正しくは **`0x05`**。`0x04` は SUBSCRIBER。

### CDR alignment（payload start を origin=0 とする CDR ストリーム）

```
0  : request_id  (2 byte raw)
2  : object_id   (2 byte raw)
4  : kind        (u8)
5  : format      (u8)
6  : (2 byte pad to align uint32)
8  : xml length  (u32 LE)
12 : xml bytes...
12+L+1 : null terminator '\0'
…   : 後続フィールド（ObjectId 2byte raw、または PARTICIPANT は 2-aligned int16）
```

PARTICIPANT のみ末尾の int16 domain_id 直前に 1 byte の追加 padding が要る場合がある（`(payload_length % 2 != 0)` のとき）。他のエンティティは ObjectId が 2-byte raw なのでアライン不要。

### Submessage flags

慣例: `0x01 (LE) | 0x02 (REUSE) | 0x04 (REPLACE) = 0x07`。Agent はこのビットで重複生成を許可する。

### XML テンプレート（micro-ROS 互換）

```xml
<dds><participant><rtps><name>NODE_NAME</name></rtps></participant></dds>
<dds><topic><name>rt/TOPIC_NAME</name><dataType>std_msgs::msg::dds_::String_</dataType></topic></dds>
<dds><publisher><name>MyPublisher</name></publisher></dds>
<dds><data_writer><topic><kind>NO_KEY</kind><name>rt/TOPIC_NAME</name><dataType>std_msgs::msg::dds_::String_</dataType></topic></data_writer></dds>
```

Topic 名は ROS2 規約により `rt/` プレフィックスを付ける（ROS2 trace のキー無し版）。Type 名は `<package>::msg::dds_::<MsgName>_` （末尾アンダースコア）。

---

## READ_DATA submessage（subscribe 要求）

```
BaseObjectRequest:
  request_id : [u8; 2]
  object_id  : [u8; 2]   DataReader の packed ObjectId
ReadSpecification:
  preferred_stream_id : u8       (BEST_EFFORT=0x01)
  data_format         : u8       (FORMAT_DATA=0x00)
  optional_content_filter_expression : bool (= 0)
  optional_delivery_control          : bool (= 0)
```

> ⚠ **STATUS は返らない**: eProsima Client (`uxr_buffer_request_data`) は
> READ_DATA 送信後に STATUS を待たない。Agent も STATUS を返さず、いきなり
> DATA フレームを送り始める。送信側で STATUS を `wait_status_for` で待つと
> TCP read timeout (`Io`) で死ぬ。**READ_DATA は fire-and-forget**。

## STATUS_Payload（CREATE 応答）

```
related_request:
  request_id : [u8; 2]  ← CREATE で送った request_id をそのまま echo
  object_id  : [u8; 2]  ← CREATE で送った object_id をそのまま echo
result:
  status                 : u8
  implementation_status  : u8
```

合計 6 byte。`request_id` で対応する CREATE を識別する。

実例（CREATE_PARTICIPANT 成功）:
```
msg hdr  : 81 01 00 00         sid=0x81, stream=BEST_EFFORT, seq=0
sub hdr  : 05 01 06 00         STATUS, LE, len=6
payload  : 00 01               related request_id=1
           00 11               related object_id=0x0011 (idx=1 << 4 | PARTICIPANT)
           00 00               status=OK, impl_status=0
```

---

## WRITE_DATA submessage（publish）

```
BaseObjectRequest:
  request_id : [u8; 2]   通常 0x00 0x00（BEST_EFFORT は req_id 不要）
  object_id  : [u8; 2]   DataWriter の packed ObjectId
[CDR field bytes only]   ← ★ encap header は付けない ★
```

> ⚠ **重大な落とし穴**: WRITE_DATA payload には **CDR encapsulation header
> (`00 01 00 00`) を含めてはいけない**。Agent は受け取ったバイト列をそのまま
> Fast-DDS DataWriter に渡し、Fast-DDS が SerializedPayload を組み立てるとき
> に encap header を勝手に付ける。こちらが付けてしまうと **encap が二重になり**、
> 後続 4 byte（本来は string length など）が encap として解釈されて
> deserialize が静かに失敗する（`ros2 topic echo` は無反応、`hz` だけ動く）。
>
> 検証方法: `ros2 topic echo /<topic> <type> --raw` で生バイト列が
> `00 01 00 00 00 01 00 00 ...` のように encap 連続なら二重化している。

Submessage flags: `0x01 (LE) | 0x00 (FORMAT_DATA) = 0x01`。Stream は **BEST_EFFORT (0x01)** を使う。BEST_EFFORT では Agent から STATUS は返らない（fire-and-forget）。

実例（std_msgs/String "hello #40" を DW idx=1 に publish）:
```
msg hdr  : 81 01 04 00         sid, stream=BEST_EFFORT, seq=4
sub hdr  : 07 01 12 00         WRITE_DATA, LE, len=18
payload  : 00 00               request_id (unused)
           00 15               object_id = (1<<4)|5 = 0x0015 (DATAWRITER)
           0a 00 00 00         string length 10 (incl null) ← encap は ここに入れない
           "hello #40\0"
```

ROS2 側 (`ros2 topic echo --raw`) が見るバイト列（Fast-DDS が encap を付与した結果）:
```
00 01 00 00      Fast-DDS が付ける CDR_LE encap
0a 00 00 00      string length
"hello #40\0"
00               4-byte alignment pad
```

---

## 実証済みエンドツーエンドシーケンス（2026-05-04）

WSL 内の `tenshi-no-hana-container` から `micro_ros_agent:8888` に対して以下を実行し、すべて status=OK を確認:

1. CREATE_CLIENT (24B) → STATUS_AGENT (19B) OK
2. CREATE_PARTICIPANT idx=1 → STATUS req=1 oid=0x0011 OK
3. CREATE_TOPIC idx=1 with `rt/angel_nose/hello` → STATUS req=2 oid=0x0012 OK
4. CREATE_PUBLISHER idx=1 → STATUS req=3 oid=0x0013 OK
5. CREATE_DATAWRITER idx=1 → STATUS req=4 oid=0x0015 OK
6. WRITE_DATA on dw idx=1 → 応答なし（BEST_EFFORT 仕様通り）

再現スクリプトは `python3` 標準ライブラリだけで書ける（参考: コミット履歴の Python repro）。

---

## デバッグ tips

### Agent ログを最大化

`.docker/compose.yaml` の agent コマンドに `-v 5` を付ける（既に適用済み）。コンテナログで全パケットの 16 進ダンプが見える:

```sh
docker logs -f micro_ros_agent
```

### よくある詰まりポイント

| 症状                                                                      | 原因                                                                                                                                                            |
| ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TCP 接続成功 → CREATE_CLIENT 送信 → 沈黙                                  | submessage ID か CLIENT_Representation のフォーマット違反                                                                                                       |
| STATUS_AGENT は来るが CREATE で沈黙                                       | CREATE のキャスト不足（CDR alignment）または kind 値違い                                                                                                        |
| CREATE は通るが ROS2 で見えない                                           | DataType 名（`std_msgs::msg::dds_::Foo_`）か topic 名前                                                                                                         |
| WRITE_DATA 送信後すぐ TCP 切断                                            | stream_id 違い（BEST_EFFORT は 0x01–0x7F）                                                                                                                      |
| 1 つ目の app は OK、別の app に切り替えると `STATUS_ERR_DDS_ERROR (0x80)` | 同じ `client_key` で異なる topic/型を idx 衝突した状態で REPLACE しようとしている。`docker restart micro_ros_agent` するか、各 app 用に別の `client_key` を使う |
| `STATUS_OK_MATCHED (0x01)` が CREATE で連発                               | 前回 session のエンティティが残存。続く DR/DW が壊れる予兆。同上。                                                                                              |

> SDK は `parse_status_payload` で `STATUS_OK_MATCHED` を受けたとき必ず warn ログ
> （`obj_id=0x... — stale entity reused...`）を出します。次に来る DR/DW が
> `AgentRejected(0x80)` で死んだら、まずは agent restart を疑ってください。

### Client key の運用

各 firmware/example が同じ `client_key` だと、agent が前回 run のエンティティを
再利用しようとして上記の MATCHED 連発 → DR/DW で `0x80` という事故が起きます。

SDK が以下を提供しています:

```rust
// Cargo がアプリ名を勝手に渡してくれる版（推奨、各 example で別キー）
let key: [u8; 4] = micro_xrce_dds_rs::client_key!();

// 明示版（手動でユニーク文字列を指定）
let key: [u8; 4] = micro_xrce_dds_rs::client_key!("my_custom_id");

// const fn 直叩き（compile-time FNV-1a）
const KEY: [u8; 4] = micro_xrce_dds_rs::client_key_from_app_id("my_app");
```

`client_key!()` (引数なし) は `concat!(CARGO_PKG_NAME, "/", CARGO_BIN_NAME)` の
FNV-1a 32-bit hash を返すので、example/binary ごとに自動でユニークになります。

### Wireshark

`tcp.port == 8888` でキャプチャ。RTPS dissector を `Decode As → RTPS` で当てる。
