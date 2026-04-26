# GitHub Copilot Instructions: no_std Serialization in Embedded Rust (Cortex-M)

## Context

These instructions apply when writing or reviewing **Rust code for `no_std` + `no_alloc` embedded targets** (Cortex-M class MCUs, stack size ~1–2 KB).

---

## Target Constraints

- `#![no_std]`
- No heap (`alloc` crate not available)
- Stack budget: **1–2 KB total** (shared with ISR, OS, etc.)
- MCU class: Cortex-M0/M0+/M3/M4/M33
- Message shape: primarily **flat (non-nested)** structs

---

## Serialization Format Selection

### Decision Priority (stack-safety order)

1. **Hand-written fixed-length binary** — Always prefer for tight stack budgets
2. **`postcard`** — Good default for Rust-native projects
3. **`minicbor`** — When CBOR interop is required
4. **`nanopb` (C via FFI)** — When protobuf wire compatibility is required
5. **`micropb`** — Only for flat messages; avoid with nested types
6. **`prost` / `protobuf`** — ❌ Do NOT use; requires `alloc`

---

## Stack Usage Reference (Decode, Flat Message)

| Format | Estimated Stack (decode) | Notes |
|---|---|---|
| Hand-written fixed-length | 16–64 B | Full developer control |
| `postcard` | 48–128 B | Upper bound with enum/error paths |
| `minicbor` | 64–128 B | `#[derive(Decode)]` on simple structs |
| `nanopb` (C) | 64–256 B | Includes message struct on stack |
| `micropb` (Rust, flat) | 200–512 B | Flat messages only |
| `micropb` (Rust, nested) | 512 B–1 KB+ | Avoid nesting |
| `prost` / `protobuf` | N/A (no_alloc) | Requires `alloc`; do not use |

> ⚠️ Values are estimates. Always verify with `cargo size`, stack painting, or a debugger on real hardware.

---

## Rules for Code Generation

### ✅ DO

```rust
// Prefer direct fixed-length decode with explicit byte slicing
fn decode_my_msg(buf: &[u8]) -> Result<MyMsg, DecodeError> {
    if buf.len() < 6 { return Err(DecodeError::TooShort); }
    Ok(MyMsg {
        id:  u16::from_le_bytes(buf[0..2].try_into().unwrap()),
        val: u32::from_le_bytes(buf[2..6].try_into().unwrap()),
    })
}
```

```rust
// postcard: use borrowed types to avoid copies
#[derive(serde::Deserialize)]
struct MyMsg<'a> {
    id: u16,
    payload: &'a [u8],  // zero-copy borrow
}

let msg: MyMsg = postcard::from_bytes(buf)?;
```

```rust
// minicbor: prefer #[derive] over manual Decode impl for stack safety
#[derive(minicbor::Decode)]
struct MyMsg {
    #[n(0)] id: u16,
    #[n(1)] val: u32,
}
```

### ❌ DO NOT

```rust
// DON'T: allocate temp buffers on stack inside trait impls called in hot loops
impl PbRead for MyReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, _> {
        let tmp: [u8; 256] = [0u8; 256]; // ← Burns 256 B per call frame
        // ...
    }
}
```

```rust
// DON'T: use prost in no_std + no_alloc context
// prost requires Vec, String, Box — incompatible with no_alloc
use prost::Message; // ❌ Will not compile without alloc
```

```rust
// DON'T: nest micropb message types more than 1–2 levels deep
// Each nesting level adds a decode_message() frame (~200–300 B each)
message Outer {
    oneof payload {
        Inner inner = 1;       // +1 frame
            // DeepInner deep  // ← ❌ Avoid 3+ levels
    }
}
```

---

## micropb-Specific Warnings

Generate a warning comment when:
- `micropb` is used with **nested `oneof`** (≥ 2 levels deep)
- A `PbRead` implementation allocates a **fixed-size array on the stack**
- `Result<_, PbError>` is propagated with `?` through **more than 3 call levels**

Example warning to insert:

```rust
// ⚠️ STACK WARNING: This nested oneof may consume 512 B–1 KB of stack.
// Verify with stack analysis before deploying on Cortex-M with <2 KB stack.
```

---

## nanopb FFI Guidelines

When generating `unsafe` FFI bindings to nanopb:

```rust
// Always stack-allocate the message struct (nanopb design intent)
let mut msg: ffi::MyMessage = unsafe { core::mem::zeroed() }; // matches MyMessage_init_zero
let mut stream = unsafe {
    ffi::pb_istream_from_buffer(buf.as_ptr(), buf.len())
};
let ok = unsafe {
    ffi::pb_decode(&mut stream, ffi::MyMessage_fields, &mut msg as *mut _ as *mut _)
};
```

- Do NOT heap-allocate the message struct — defeats nanopb's stack model
- `pb_decode` is **non-recursive at the top level**; prefer it over callback-based variants for stack safety

---

## Why nanopb Is Stack-Efficient (Reference)

nanopb processes message fields with an **iterative loop**, not deep recursion:

```c
// Conceptual nanopb internals
while (next_tag(stream, &tag)) {
    pb_field_iter_find(&iter, tag);
    decode_field(stream, &iter);  // Non-recursive for primitive types
}
```

Sub-messages cause **one additional `pb_decode` call per nesting level** — predictable and bounded.

---

## Why Rust + protobuf-compat + Tiny Stack Is Hard

Copilot should acknowledge this tradeoff explicitly when suggesting libraries:

1. **protobuf assumes heap**: `oneof`, `repeated`, variable-length fields map naturally to `Vec`/`Box`.
2. **Rust monomorphization**: Generic decode functions expand per-type, growing per-frame stack differently than C's `void*`/function-pointer erasure.
3. **`serde` Visitor chain**: `Deserialize` dispatches through multiple trait method calls; inlining is not guaranteed on all optimization levels.
4. **Practical recommendation**: For protobuf wire-format compatibility in `no_std` + `no_alloc`, prefer `nanopb` via FFI over any pure-Rust solution until a mature alternative exists.

---

## Checklist Before Committing Serialization Code

- [ ] Does the decode path allocate any `[u8; N]` arrays on the stack? If N > 64, flag it.
- [ ] Are nested message types used? Count nesting depth. Warn if > 2 for micropb.
- [ ] Is `prost` or any `alloc`-dependent crate imported? Reject in `no_alloc` targets.
- [ ] Has stack usage been verified with measurement (stack painting / `defmt` / debugger)?
- [ ] Is the `serde` derive used with postcard? Prefer borrowed `&[u8]` / `&str` over owned types.

---

## Related Crates (Verified no_std + no_alloc Compatible)

| Crate | Version (reference) | no_alloc | Wire format |
|---|---|---|---|
| `postcard` | 1.x | ✅ | postcard (custom) |
| `minicbor` | 0.20+ | ✅ | CBOR (RFC 7049) |
| `micropb` | 0.2+ | ✅ (with limits) | protobuf |
| `serde` | 1.x | ✅ (with `derive`) | format-agnostic |
| `nanopb` | 0.4.x (C) | ✅ | protobuf |

> Always pin crate versions in `Cargo.toml` for embedded projects.