# no_std シリアライゼーション — 組み込み Rust (Cortex-M) ガイド

## 対象環境

- `#![no_std]`
- ヒープなし (`alloc` 不使用)
- スタック予算: **1–2 KB 総計** (ISR・OS 共有)
- MCU: Cortex-M0/M0+/M3/M4/M33
- メッセージ形状: 主に **フラット (非ネスト)** な struct

---

## フォーマット選択 (スタック安全性順)

1. **手書き固定長バイナリ** — スタック予算が厳しい場合は常にこれを優先
2. **`postcard`** — Rust ネイティブプロジェクトのデフォルト
3. **`minicbor`** — CBOR 相互運用が必要な場合
4. **`nanopb` (C via FFI)** — protobuf ワイヤ互換が必要な場合
5. **`micropb`** — フラットメッセージのみ。ネスト型には使わない
6. **`prost` / `protobuf`** — ❌ 使用禁止。`alloc` が必要

---

## スタック使用量目安 (デコード、フラットメッセージ)

| フォーマット | スタック概算 (decode) | 備考 |
|---|---|---|
| 手書き固定長 | 16–64 B | 開発者が完全制御 |
| `postcard` | 48–128 B | enum/エラーパスを含む上限 |
| `minicbor` | 64–128 B | シンプルな struct に `#[derive(Decode)]` |
| `nanopb` (C) | 64–256 B | メッセージ struct 込み |
| `micropb` (Rust, flat) | 200–512 B | フラットのみ |
| `micropb` (Rust, nested) | 512 B–1 KB+ | ネスト禁止 |
| `prost` / `protobuf` | N/A (no_alloc) | `alloc` 必須。使用禁止 |

> ⚠️ 値は目安。実機で `cargo size`、スタックペイント、またはデバッガで必ず検証する。

---

## コード規則

### ✅ DO

```rust
// 直接固定長デコード — 明示的なバイトスライス
fn decode_my_msg(buf: &[u8]) -> Result<MyMsg, DecodeError> {
    if buf.len() < 6 { return Err(DecodeError::TooShort); }
    Ok(MyMsg {
        id:  u16::from_le_bytes(buf[0..2].try_into().unwrap()),
        val: u32::from_le_bytes(buf[2..6].try_into().unwrap()),
    })
}
```

```rust
// postcard: コピーを避けるため借用型を使う
#[derive(serde::Deserialize)]
struct MyMsg<'a> {
    id: u16,
    payload: &'a [u8],  // ゼロコピー借用
}
let msg: MyMsg = postcard::from_bytes(buf)?;
```

```rust
// minicbor: スタック安全のため手動 Decode impl より #[derive] を優先
#[derive(minicbor::Decode)]
struct MyMsg {
    #[n(0)] id: u16,
    #[n(1)] val: u32,
}
```

### ❌ DO NOT

```rust
// NG: ホットループ内の trait impl でスタック上に一時バッファを確保
impl PbRead for MyReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, _> {
        let tmp: [u8; 256] = [0u8; 256]; // ← 呼び出しフレームごとに 256 B 消費
    }
}
```

```rust
// NG: no_std + no_alloc 環境で prost を使用
use prost::Message; // ❌ alloc なしではコンパイル不可
```

```rust
// NG: micropb のメッセージ型を 2 レベル以上ネスト
// 各ネストで decode_message() フレームが積まれる (~200–300 B/レベル)
message Outer {
    oneof payload {
        Inner inner = 1;       // +1 フレーム
        // DeepInner deep      // ← ❌ 3 レベル以上は避ける
    }
}
```

---

## micropb 使用時の警告条件

以下の場合はコメントで警告を挿入する:
- **ネストした `oneof`** (2 レベル以上) を使用
- `PbRead` 実装でスタック上に **固定サイズ配列** を確保
- `Result<_, PbError>` が `?` で **3 呼び出しレベル以上** 伝播

```rust
// ⚠️ STACK WARNING: This nested oneof may consume 512 B–1 KB of stack.
// Verify with stack analysis before deploying on Cortex-M with <2 KB stack.
```

---

## nanopb FFI ガイドライン

```rust
// メッセージ struct は常にスタックに確保 (nanopb の設計意図)
let mut msg: ffi::MyMessage = unsafe { core::mem::zeroed() };
let mut stream = unsafe {
    ffi::pb_istream_from_buffer(buf.as_ptr(), buf.len())
};
let ok = unsafe {
    ffi::pb_decode(&mut stream, ffi::MyMessage_fields, &mut msg as *mut _ as *mut _)
};
```

- メッセージ struct をヒープに確保しない — nanopb のスタックモデルを破壊する
- `pb_decode` はトップレベルでは非再帰。スタック安全のためコールバック変種より優先

---

## Rust + protobuf + 小スタックが難しい理由

1. **protobuf はヒープを前提**: `oneof`、`repeated`、可変長フィールドは `Vec`/`Box` に自然にマップされる
2. **Rust モノモルフィゼーション**: ジェネリックデコード関数は型ごとに展開され、C の `void*` 消去と異なりフレームサイズが変動する
3. **`serde` Visitor チェーン**: `Deserialize` は複数のトレートメソッド呼び出しを経由し、全最適化レベルでインライン化が保証されない
4. **推奨**: `no_std` + `no_alloc` で protobuf ワイヤ互換が必要なら、成熟した純 Rust 代替が出るまで `nanopb` FFI を優先

---

## コミット前チェックリスト

- [ ] デコードパスでスタックに `[u8; N]` 配列を確保している場合、N > 64 なら要注意
- [ ] ネストメッセージ型を使用している場合、ネスト深さを確認。micropb で 2 超なら警告
- [ ] `prost` または `alloc` 依存クレートをインポートしていないか
- [ ] スタック使用量を実測で確認したか (スタックペイント / `defmt` / デバッガ)
- [ ] postcard + `serde` derive 使用時、所有型より `&[u8]` / `&str` 借用型を優先したか

---

## 関連クレート (no_std + no_alloc 検証済み)

| クレート | バージョン | no_alloc | ワイヤ形式 |
|---|---|---|---|
| `postcard` | 1.x | ✅ | postcard (独自) |
| `minicbor` | 0.20+ | ✅ | CBOR (RFC 7049) |
| `micropb` | 0.2+ | ✅ (制限あり) | protobuf |
| `serde` | 1.x | ✅ (`derive` 付き) | フォーマット非依存 |
| `nanopb` | 0.4.x (C) | ✅ | protobuf |

> 組み込みプロジェクトでは `Cargo.toml` でクレートバージョンを必ずピン留めする。
