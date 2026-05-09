// build.rs — combines two responsibilities:
//   1. Inject Wi-Fi / micro-ROS Agent config from `wifi_config.json` as
//      `cargo:rustc-env` variables (only when the `wifi` feature is enabled).
//   2. Parse vendored ROS2 `.msg` files under `msgs/` with `roslibrust_codegen`
//      and emit no_std-friendly Rust types into `$OUT_DIR/ros_msgs.rs`.
//      `src/ros_msgs.rs` `include!`s the result.

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use roslibrust_codegen::{
    find_and_parse_ros_messages, ArrayType, ConstantInfo, FieldInfo, FieldType,
};

// `roslibrust_codegen` exposes its `ParsedMessageFile` type through a private
// module (the inner fields are pub but the type itself is unreachable as a
// name). We copy the parts we need into a local struct so we can store and
// pass the data through our codegen helpers.
struct Msg {
    name: String,
    package: String,
    fields: Vec<FieldInfo>,
    constants: Vec<ConstantInfo>,
}

const DEFAULT_STRING_CAP: usize = 32;
const DEFAULT_SEQ_CAP: usize = 8;

fn main() {
    // ── Wi-Fi config injection ──────────────────────────────────────────────
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=wifi_config.json");

    println!(
        "cargo:rustc-link-search={}",
        env::var("CARGO_MANIFEST_DIR").unwrap()
    );

    if env::var("CARGO_FEATURE_WIFI").is_ok() {
        let json = fs::read_to_string("wifi_config.json").unwrap_or_else(|_| {
            panic!(
                "\n\n\
                 wifi_config.json not found.\n\
                 Copy wifi_config.json.example to wifi_config.json and fill in your credentials.\n\
                 This file is git-ignored and will not be committed.\n"
            )
        });

        let ssid = extract_str(&json, "ssid")
            .unwrap_or_else(|| panic!("wifi_config.json: missing or invalid \"ssid\" field"));
        let password = extract_str(&json, "password")
            .unwrap_or_else(|| panic!("wifi_config.json: missing or invalid \"password\" field"));
        let agent_addr = extract_str(&json, "agent_addr").unwrap_or_else(|| {
            panic!(
                "wifi_config.json: missing \"agent_addr\" field.\n\
                 Example: \"agent_addr\": \"192.168.1.1:8888\""
            )
        });

        println!("cargo:rustc-env=WIFI_SSID={ssid}");
        println!("cargo:rustc-env=WIFI_PASSWORD={password}");
        println!("cargo:rustc-env=MICRO_ROS_AGENT_ADDR={agent_addr}");
    }

    // ── ROS2 .msg → Rust codegen ────────────────────────────────────────────
    generate_ros_msgs();
}

/// Extract the string value for `key` from a simple flat JSON object.
/// Handles `"key": "value"` patterns; no support for escape sequences beyond `\"`.
fn extract_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let start = json.find(needle.as_str())? + needle.len();
    let after_key = json[start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let inner = after_colon.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(&inner[..end])
}

// ─────────────────────────────────────────────────────────────────────────────
// ROS2 message codegen
//
// Pipeline:
//   1. `roslibrust_codegen::find_and_parse_ros_messages` walks `msgs/` and
//      returns parsed AST (`ParsedMessageFile { fields, constants, ... }`).
//   2. We DO NOT call its built-in token emitter (which produces std types
//      using `String`/`Vec`). We walk the AST ourselves and emit no_std code:
//        - `pub struct Foo { ... }` with primitives, fixed arrays, and
//          `heapless::String<N>` / `heapless::Vec<T, N>` for variable-length
//          fields (CDR `string`, `T[]`, `T[<=N]`).
//        - `impl Foo { fn serialize_inline(&self, w: &mut CdrWriter); ... }`
//          for embedding in outer structs.
//        - `impl micro_xrce_dds_rs::Message for Foo { ... }`.
//   3. Output → `$OUT_DIR/ros_msgs.rs`. `src/ros_msgs.rs` `include!`s it.
//
// Heapless capacities used when a .msg field is unbounded:
//   * `string`         → `heapless::String<32>`
//   * `string[]`       → `heapless::Vec<heapless::String<32>, 8>`
//   * `<prim>[]`       → `heapless::Vec<<prim>, 8>`
//   * `<struct>[]`     → `heapless::Vec<<struct>, 8>`
// Override per-field with bounded ROS2 syntax in the .msg file:
//   `string<=64 frame_id`  or  `geometry_msgs/TransformStamped[<=4] transforms`.
// ─────────────────────────────────────────────────────────────────────────────

fn generate_ros_msgs() {
    println!("cargo:rerun-if-changed=msgs");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let msgs_root = manifest_dir.join("msgs");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ros_msgs.rs");

    if !msgs_root.exists() {
        fs::write(&out_path, "// no msgs/ directory\n").unwrap();
        return;
    }

    let (raw_msgs, _services, _actions) = find_and_parse_ros_messages(&[msgs_root])
        .expect("failed to parse vendored .msg files");

    // Copy out of `roslibrust_codegen`'s private-typed return into local owned
    // structs so we can name them in helpers.
    let parsed_msgs: Vec<Msg> = raw_msgs
        .into_iter()
        .map(|m| Msg {
            name: m.name,
            package: m.package,
            fields: m.fields,
            constants: m.constants,
        })
        .collect();

    let mut by_pkg: BTreeMap<String, Vec<&Msg>> = BTreeMap::new();
    for m in &parsed_msgs {
        by_pkg.entry(m.package.clone()).or_default().push(m);
    }
    for v in by_pkg.values_mut() {
        v.sort_by(|a, b| a.name.cmp(&b.name));
    }

    // Compute MAX_SERIALIZED_SIZE bottom-up. Iterate to fixpoint.
    let mut sizes: HashMap<String, usize> = HashMap::new();
    loop {
        let before = sizes.len();
        for m in &parsed_msgs {
            let fqn = format!("{}/{}", m.package, m.name);
            if sizes.contains_key(&fqn) {
                continue;
            }
            if let Some(s) = try_compute_size(m, &sizes) {
                sizes.insert(fqn, s);
            }
        }
        if sizes.len() == before {
            break;
        }
    }
    if sizes.len() != parsed_msgs.len() {
        let missing: Vec<_> = parsed_msgs
            .iter()
            .filter(|m| !sizes.contains_key(&format!("{}/{}", m.package, m.name)))
            .map(|m| format!("{}/{}", m.package, m.name))
            .collect();
        panic!("could not size messages (cyclic or unknown deps): {missing:?}");
    }

    let mut out = String::new();
    out.push_str("// Auto-generated by build.rs from msgs/. Do not edit by hand.\n\n");
    out.push_str("use heapless;\n");
    out.push_str("use micro_xrce_dds_rs::cdr::CdrWriter;\n");
    out.push_str("use micro_xrce_dds_rs::cdr_reader::CdrReader;\n");
    out.push_str("use micro_xrce_dds_rs::error::Error;\n");
    out.push_str("use micro_xrce_dds_rs::message::Message;\n\n");

    for (pkg, msgs) in &by_pkg {
        writeln!(&mut out, "pub mod {pkg} {{").unwrap();
        out.push_str("    #[allow(unused_imports)]\n");
        out.push_str("    use super::{CdrReader, CdrWriter, Error, Message, heapless};\n\n");
        for m in msgs {
            emit_constants(&mut out, m);
            emit_struct(&mut out, m);
            emit_inline_impl(&mut out, m);
            emit_message_impl(&mut out, m, &sizes);
        }
        out.push_str("}\n\n");
    }

    fs::write(&out_path, out).unwrap();
}

// ── helpers: type translation ────────────────────────────────────────────────

fn rust_type(ft: &FieldType, current_pkg: &str) -> String {
    let inner = rust_type_scalar(ft, current_pkg);
    match ft.array_info {
        ArrayType::NotArray => inner,
        ArrayType::FixedLength(n) => format!("[{inner}; {n}]"),
        ArrayType::Bounded(n) => format!("heapless::Vec<{inner}, {n}>"),
        ArrayType::Unbounded => format!("heapless::Vec<{inner}, {DEFAULT_SEQ_CAP}>"),
    }
}

fn rust_type_scalar(ft: &FieldType, current_pkg: &str) -> String {
    match ft.field_type.as_str() {
        "bool" => "bool".into(),
        "byte" | "uint8" => "u8".into(),
        "char" | "int8" => "i8".into(),
        "uint16" => "u16".into(),
        "int16" => "i16".into(),
        "uint32" => "u32".into(),
        "int32" => "i32".into(),
        "uint64" => "u64".into(),
        "int64" => "i64".into(),
        "float32" => "f32".into(),
        "float64" => "f64".into(),
        "string" | "wstring" => {
            let cap = ft.string_capacity.unwrap_or(DEFAULT_STRING_CAP);
            format!("heapless::String<{cap}>")
        }
        // Struct reference. `package_name` is None when the field is in the
        // same package as its containing message.
        _ => {
            let pkg = ft.package_name.as_deref().unwrap_or(current_pkg);
            if pkg == current_pkg {
                ft.field_type.clone()
            } else {
                format!("super::{pkg}::{}", ft.field_type)
            }
        }
    }
}

/// Conservative max serialized byte count for a primitive type. Includes
/// worst-case alignment padding before the value.
fn primitive_max_size(t: &str) -> Option<usize> {
    Some(match t {
        "bool" | "byte" | "uint8" | "char" | "int8" => 1,
        "uint16" | "int16" => 2 + 1,
        "uint32" | "int32" | "float32" => 4 + 3,
        "uint64" | "int64" | "float64" => 8 + 7,
        _ => return None,
    })
}

fn try_compute_size(m: &Msg, sizes: &HashMap<String, usize>) -> Option<usize> {
    let mut total: usize = 0;
    for fi in &m.fields {
        total += try_field_size(fi, &m.package, sizes)?;
    }
    Some(total + 8)
}

fn try_field_size(
    fi: &FieldInfo,
    current_pkg: &str,
    sizes: &HashMap<String, usize>,
) -> Option<usize> {
    let scalar = scalar_size(&fi.field_type, current_pkg, sizes)?;
    Some(match fi.field_type.array_info {
        ArrayType::NotArray => scalar,
        ArrayType::FixedLength(n) => scalar * n + 8,
        ArrayType::Bounded(n) => 4 + scalar * n + 8,
        ArrayType::Unbounded => 4 + scalar * DEFAULT_SEQ_CAP + 8,
    })
}

fn scalar_size(
    ft: &FieldType,
    current_pkg: &str,
    sizes: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(p) = primitive_max_size(&ft.field_type) {
        return Some(p);
    }
    if ft.field_type == "string" || ft.field_type == "wstring" {
        let cap = ft.string_capacity.unwrap_or(DEFAULT_STRING_CAP);
        return Some(4 + cap + 1 + 3);
    }
    let pkg = ft.package_name.as_deref().unwrap_or(current_pkg);
    sizes.get(&format!("{pkg}/{}", ft.field_type)).copied()
}

// ── emitters ────────────────────────────────────────────────────────────────

fn emit_constants(out: &mut String, m: &Msg) {
    if m.constants.is_empty() {
        return;
    }
    for c in &m.constants {
        let ty = match c.constant_type.as_str() {
            "bool" => "bool",
            "byte" | "uint8" => "u8",
            "char" | "int8" => "i8",
            "uint16" => "u16",
            "int16" => "i16",
            "uint32" => "u32",
            "int32" => "i32",
            "uint64" => "u64",
            "int64" => "i64",
            "float32" => "f32",
            "float64" => "f64",
            // Skip string constants (would need lifetime gymnastics).
            _ => continue,
        };
        writeln!(
            out,
            "    pub const {}_{}: {ty} = {};",
            m.name.to_uppercase(),
            c.constant_name,
            c.constant_value
        )
        .unwrap();
    }
    out.push('\n');
}

fn emit_struct(out: &mut String, m: &Msg) {
    writeln!(out, "    #[derive(Clone, Debug)]").unwrap();
    writeln!(out, "    pub struct {} {{", m.name).unwrap();
    for fi in &m.fields {
        let ty = rust_type(&fi.field_type, &m.package);
        writeln!(out, "        pub {}: {ty},", fi.field_name).unwrap();
    }
    out.push_str("    }\n\n");

    // Manual Default impl — covers fixed arrays of any size N (the derive
    // only works for N <= 32) and gives sane zero/identity defaults.
    writeln!(out, "    impl Default for {} {{", m.name).unwrap();
    out.push_str("        fn default() -> Self {\n");
    out.push_str("            Self {\n");
    for fi in &m.fields {
        let init = default_expr(&fi.field_type, &m.package);
        writeln!(out, "                {}: {init},", fi.field_name).unwrap();
    }
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

fn default_expr(ft: &FieldType, current_pkg: &str) -> String {
    match ft.array_info {
        ArrayType::NotArray => default_scalar(ft, current_pkg),
        ArrayType::FixedLength(n) => {
            let scalar = default_scalar(ft, current_pkg);
            format!("[{scalar}; {n}]")
        }
        ArrayType::Bounded(_) | ArrayType::Unbounded => "heapless::Vec::new()".into(),
    }
}

fn default_scalar(ft: &FieldType, current_pkg: &str) -> String {
    match ft.field_type.as_str() {
        "bool" => "false".into(),
        "byte" | "uint8" | "char" | "int8" | "uint16" | "int16" | "uint32" | "int32"
        | "uint64" | "int64" => "0".into(),
        "float32" => "0.0_f32".into(),
        "float64" => "0.0_f64".into(),
        "string" | "wstring" => "heapless::String::new()".into(),
        _ => {
            let ty = rust_type_scalar(ft, current_pkg);
            format!("{ty}::default()")
        }
    }
}

fn emit_inline_impl(out: &mut String, m: &Msg) {
    writeln!(out, "    impl {} {{", m.name).unwrap();
    out.push_str("        pub fn serialize_inline(&self, w: &mut CdrWriter) {\n");
    if m.fields.is_empty() {
        out.push_str("            let _ = w;\n");
    }
    for fi in &m.fields {
        emit_serialize_field(out, fi, &m.package, "self.");
    }
    out.push_str("        }\n\n");
    out.push_str("        pub fn deserialize_inline(r: &mut CdrReader<'_>) -> Result<Self, Error> {\n");
    if m.fields.is_empty() {
        out.push_str("            let _ = r;\n");
    }
    out.push_str("            Ok(Self {\n");
    for fi in &m.fields {
        let expr = deserialize_field_expr(fi, &m.package);
        writeln!(out, "                {}: {expr},", fi.field_name).unwrap();
    }
    out.push_str("            })\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

fn emit_message_impl(out: &mut String, m: &Msg, sizes: &HashMap<String, usize>) {
    let fqn = format!("{}/{}", m.package, m.name);
    let size = sizes.get(&fqn).copied().unwrap_or(1024);
    let type_name = format!("{}::msg::dds_::{}_", m.package, m.name);
    writeln!(out, "    impl Message for {} {{", m.name).unwrap();
    writeln!(out, "        const TYPE_NAME: &'static str = \"{type_name}\";").unwrap();
    writeln!(out, "        const MAX_SERIALIZED_SIZE: usize = {size};").unwrap();
    out.push_str("        fn serialize(&self, w: &mut CdrWriter) { self.serialize_inline(w); }\n");
    out.push_str("        fn deserialize(r: &mut CdrReader<'_>) -> Result<Self, Error> { Self::deserialize_inline(r) }\n");
    out.push_str("    }\n\n");
}

// ── per-field serialize / deserialize ───────────────────────────────────────

fn emit_serialize_field(
    out: &mut String,
    fi: &FieldInfo,
    current_pkg: &str,
    prefix: &str,
) {
    let access = format!("{prefix}{}", fi.field_name);
    match fi.field_type.array_info {
        ArrayType::NotArray => emit_serialize_scalar(out, &fi.field_type, current_pkg, &access),
        ArrayType::FixedLength(_) => {
            writeln!(out, "            for v in &{access} {{").unwrap();
            emit_serialize_scalar_loop(out, &fi.field_type, current_pkg, "v");
            out.push_str("            }\n");
        }
        ArrayType::Bounded(_) | ArrayType::Unbounded => {
            writeln!(out, "            w.u32_val({access}.len() as u32);").unwrap();
            writeln!(out, "            for v in &{access} {{").unwrap();
            emit_serialize_scalar_loop(out, &fi.field_type, current_pkg, "v");
            out.push_str("            }\n");
        }
    }
}

fn emit_serialize_scalar(out: &mut String, ft: &FieldType, _current_pkg: &str, access: &str) {
    match ft.field_type.as_str() {
        "bool" => writeln!(out, "            w.bool_val({access});").unwrap(),
        "byte" | "uint8" => writeln!(out, "            w.u8_val({access});").unwrap(),
        "uint32" => writeln!(out, "            w.u32_val({access});").unwrap(),
        "int32" => writeln!(out, "            w.i32_val({access});").unwrap(),
        "uint64" => writeln!(out, "            w.u64_val({access});").unwrap(),
        "int64" => writeln!(out, "            w.i64_val({access});").unwrap(),
        "float32" => writeln!(out, "            w.f32_val({access});").unwrap(),
        "float64" => writeln!(out, "            w.f64_val({access});").unwrap(),
        "string" | "wstring" => {
            writeln!(out, "            w.string({access}.as_str());").unwrap();
        }
        "char" | "int8" => writeln!(out, "            w.u8_val({access} as u8);").unwrap(),
        "int16" | "uint16" => {
            writeln!(out, "            w.align_to(2);").unwrap();
            writeln!(out, "            w.bytes_raw(&{access}.to_le_bytes());").unwrap();
        }
        // Struct reference — call inline serializer.
        _ => writeln!(out, "            {access}.serialize_inline(w);").unwrap(),
    }
}

/// Variant used inside `for v in &arr { ... }`, where `v: &T`.
fn emit_serialize_scalar_loop(out: &mut String, ft: &FieldType, _current_pkg: &str, var: &str) {
    match ft.field_type.as_str() {
        "bool" => writeln!(out, "                w.bool_val(*{var});").unwrap(),
        "byte" | "uint8" => writeln!(out, "                w.u8_val(*{var});").unwrap(),
        "uint32" => writeln!(out, "                w.u32_val(*{var});").unwrap(),
        "int32" => writeln!(out, "                w.i32_val(*{var});").unwrap(),
        "uint64" => writeln!(out, "                w.u64_val(*{var});").unwrap(),
        "int64" => writeln!(out, "                w.i64_val(*{var});").unwrap(),
        "float32" => writeln!(out, "                w.f32_val(*{var});").unwrap(),
        "float64" => writeln!(out, "                w.f64_val(*{var});").unwrap(),
        "string" | "wstring" => {
            writeln!(out, "                w.string({var}.as_str());").unwrap();
        }
        "char" | "int8" => writeln!(out, "                w.u8_val(*{var} as u8);").unwrap(),
        "int16" | "uint16" => {
            writeln!(out, "                w.align_to(2);").unwrap();
            writeln!(out, "                w.bytes_raw(&{var}.to_le_bytes());").unwrap();
        }
        _ => writeln!(out, "                {var}.serialize_inline(w);").unwrap(),
    }
}

fn deserialize_field_expr(fi: &FieldInfo, current_pkg: &str) -> String {
    match fi.field_type.array_info {
        ArrayType::NotArray => deserialize_scalar_expr(&fi.field_type, current_pkg),
        ArrayType::FixedLength(n) => {
            let inner = deserialize_scalar_expr(&fi.field_type, current_pkg);
            let ty = rust_type_scalar(&fi.field_type, current_pkg);
            let default = default_scalar(&fi.field_type, current_pkg);
            format!(
                "{{ let mut a: [{ty}; {n}] = [{default}; {n}]; for slot in a.iter_mut() {{ *slot = {inner}; }} a }}"
            )
        }
        ArrayType::Bounded(_) | ArrayType::Unbounded => {
            let cap = match fi.field_type.array_info {
                ArrayType::Bounded(n) => n,
                _ => DEFAULT_SEQ_CAP,
            };
            let inner = deserialize_scalar_expr(&fi.field_type, current_pkg);
            let ty = rust_type_scalar(&fi.field_type, current_pkg);
            format!(
                "{{ let len = r.u32_val()? as usize; if len > {cap} {{ return Err(Error::Deserialization); }} let mut v: heapless::Vec<{ty}, {cap}> = heapless::Vec::new(); for _ in 0..len {{ let _ = v.push({inner}); }} v }}"
            )
        }
    }
}

fn deserialize_scalar_expr(ft: &FieldType, current_pkg: &str) -> String {
    match ft.field_type.as_str() {
        "bool" => "r.bool_val()?".into(),
        "byte" | "uint8" => "r.u8_val()?".into(),
        "uint32" => "r.u32_val()?".into(),
        "int32" => "r.i32_val()?".into(),
        "uint64" => "r.u64_val()?".into(),
        "int64" => "r.i64_val()?".into(),
        "float32" => "r.f32_val()?".into(),
        "float64" => "r.f64_val()?".into(),
        "string" | "wstring" => {
            let cap = ft.string_capacity.unwrap_or(DEFAULT_STRING_CAP);
            format!(
                "{{ let bytes = r.string_bytes()?; let s = core::str::from_utf8(bytes).map_err(|_| Error::Deserialization)?; let mut out: heapless::String<{cap}> = heapless::String::new(); out.push_str(s).map_err(|_| Error::Deserialization)?; out }}"
            )
        }
        "char" | "int8" => "(r.u8_val()? as i8)".into(),
        "int16" => {
            "{ r.align_to(2); let b: [u8; 2] = r.bytes_array()?; i16::from_le_bytes(b) }".into()
        }
        "uint16" => {
            "{ r.align_to(2); let b: [u8; 2] = r.bytes_array()?; u16::from_le_bytes(b) }".into()
        }
        _ => {
            let ty = rust_type_scalar(ft, current_pkg);
            format!("{ty}::deserialize_inline(r)?")
        }
    }
}
