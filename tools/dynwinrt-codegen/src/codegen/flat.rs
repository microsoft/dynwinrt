// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Flat-Win32 `[DllImport]` code generation.
//!
//! Reads a `FlatApisMeta` (a container of DllImport static methods on an
//! `Apis` class in `Windows.Win32.winmd`) and emits a natural JS/DTS wrapper
//! that calls into `DynWinRtValue.flatInvoke` under the hood.
//!
//! ## Emission model
//!
//! For each flat method we categorise every parameter into one of three shapes:
//!
//! * **Input scalar / handle / enum / string** — passed by value into the JS
//!   function's argument list.
//! * **Pointer to a small scalar/handle/enum**, with direction `[out]` — the
//!   generator allocates a caller-side `Buffer` internally and projects the
//!   value into the JS return.
//! * **Pointer to a byte buffer / void / opaque struct** — remains in the
//!   argument list as a `Buffer | null` slot so the caller controls allocation
//!   (matches the natural Win32 idiom for `RegQueryValueExW`'s `lpData`).
//!
//! Non-zero LSTATUS/WIN32_ERROR/HRESULT returns are surfaced as a `.status`
//! field on the returned object (or as the sole `number` return when there
//! are no projected out-params). The emitted `.js` never throws on non-zero
//! LSTATUS — the caller decides what to do (mirroring the hand-written
//! `bindings/js/e2e/registry.js` design).

use std::collections::{BTreeSet, HashSet};

use crate::meta::{FlatAbiType, FlatApisMeta, FlatDirection, FlatMethodMeta, FlatParamMeta};
use crate::types::TypeMeta;

/// Rendered flat-Apis output: primary `.js` + `.d.ts` for the class, plus
/// zero or more sibling files (one `.js` + `.d.ts` per referenced enum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatGeneratedOutput {
    pub js: String,
    pub dts: String,
    /// Additional files (filename → content), stable-sorted by filename.
    pub extra_files: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

pub fn generate_flat_apis_files(meta: &FlatApisMeta) -> FlatGeneratedOutput {
    // Fail-loud filter: methods whose return type isn't representable by the
    // current `flatInvoke` ABI MUST be skipped rather than silently emitted as
    // a truncating I32 read. Print a per-skip warning so the operator sees
    // what was omitted and why.
    let (kept, skipped) = partition_supported_methods(&meta.methods);
    for (name, reason) in &skipped {
        eprintln!(
            "warning: dynwinrt-codegen: skipping flat export `{}::{}` — {}",
            meta.class_name, name, reason
        );
    }
    let kept_enum_keys = referenced_enum_keys_for_methods(&kept);
    let referenced_enums = meta
        .referenced_enums
        .iter()
        .filter(|en| match en {
            TypeMeta::Enum {
                namespace, name, ..
            } => kept_enum_keys.contains(&(namespace.clone(), name.clone())),
            _ => false,
        })
        .cloned()
        .collect();
    let filtered_meta = FlatApisMeta {
        methods: kept,
        referenced_enums,
        ..meta.clone()
    };

    let js = render_js(&filtered_meta);
    let dts = render_dts(&filtered_meta);

    // Sibling files: one per referenced enum.
    //
    // Enum sibling files (`Foo.js`, `Foo.d.ts`) key on the simple name only,
    // so two distinct enums that share the same simple name from different
    // namespaces would collide here and produce a wrong-shape enum file
    // (only one variant survives). `parse_flat_apis_from_index` already
    // deduplicates by `(namespace, name)` — but if the caller assembles a
    // `FlatApisMeta` with a genuine simple-name collision across
    // namespaces, we fail loud with a diagnostic rather than emit a
    // corrupt Apis module.
    let mut by_simple_name: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for en in &filtered_meta.referenced_enums {
        if let TypeMeta::Enum { namespace, name, .. } = en {
            by_simple_name.entry(name).or_default().push(namespace);
        }
    }
    for (name, namespaces) in &by_simple_name {
        if namespaces.len() > 1 {
            panic!(
                "flat codegen: multiple distinct enums named `{name}` referenced by \
                 `{}` from namespaces {:?}. Sibling-file emission would collide on the \
                 `{name}` simple name. Split the export or add namespace-qualified \
                 aliasing in the codegen before proceeding.",
                filtered_meta.class_name, namespaces,
            );
        }
    }

    let mut extra_files: Vec<(String, String)> = Vec::new();
    for en in &filtered_meta.referenced_enums {
        if let TypeMeta::Enum { name, .. } = en {
            let (ejs, edts) = render_enum_files(en);
            extra_files.push((format!("{}.js", name), ejs));
            extra_files.push((format!("{}.d.ts", name), edts));
        }
    }
    extra_files.sort_by(|a, b| a.0.cmp(&b.0));

    FlatGeneratedOutput {
        js,
        dts,
        extra_files,
    }
}

/// Split the methods into (kept, skipped). Skipped methods are those the
/// codegen cannot yet emit correctly — silently emitting them would produce
/// wrong-value wrappers (truncation, mis-marshalling), which violates the
/// fail-loud principle applied elsewhere.
fn partition_supported_methods(
    methods: &[FlatMethodMeta],
) -> (Vec<FlatMethodMeta>, Vec<(String, &'static str)>) {
    let mut kept: Vec<FlatMethodMeta> = Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    for m in methods {
        if let Some(reason) = unsupported_return_reason(&m.return_type) {
            skipped.push((m.name.clone(), reason));
            continue;
        }
        if let Some(reason) = m.params.iter().find_map(|p| unsupported_param_reason(&p.abi)) {
            skipped.push((m.name.clone(), reason));
            continue;
        }
        kept.push(m.clone());
    }
    (kept, skipped)
}

fn referenced_enum_keys_for_methods(methods: &[FlatMethodMeta]) -> HashSet<(String, String)> {
    let mut keys = HashSet::new();
    for m in methods {
        collect_referenced_enum_keys(&m.return_type, &mut keys);
        for p in &m.params {
            collect_referenced_enum_keys(&p.abi, &mut keys);
        }
    }
    keys
}

fn collect_referenced_enum_keys(t: &FlatAbiType, keys: &mut HashSet<(String, String)>) {
    match t {
        FlatAbiType::PtrTo(inner) => collect_referenced_enum_keys(inner, keys),
        FlatAbiType::Enum {
            namespace,
            name,
            underlying,
            ..
        } => {
            keys.insert((namespace.clone(), name.clone()));
            collect_referenced_enum_keys(underlying, keys);
        }
        _ => {}
    }
}

/// True when an enum's underlying ABI type cannot be faithfully represented on
/// the current JS enum surface. Enum members are `i32`-backed and project as a
/// `number`-based union, so only 32-bit-or-smaller integer underlyings are
/// representable. A 64-bit (`I64`/`U64`) or float (`F32`/`F64`) underlying would
/// silently emit truncated/wrong member constants and an ABI-mismatched calling
/// convention, so such methods are skipped fail-loud instead.
fn enum_underlying_unrepresentable(t: &FlatAbiType) -> bool {
    matches!(
        t,
        FlatAbiType::Enum { underlying, .. }
            if !matches!(
                **underlying,
                FlatAbiType::I8
                    | FlatAbiType::U8
                    | FlatAbiType::I16
                    | FlatAbiType::U16
                    | FlatAbiType::I32
                    | FlatAbiType::U32
            )
    )
}

/// Returns `Some(reason)` if the given return type has no faithful mapping
/// to the current `flatInvoke` return-kind ABI. `None` means the type is
/// representable and the method can be emitted.
fn unsupported_return_reason(t: &FlatAbiType) -> Option<&'static str> {
    if enum_underlying_unrepresentable(t) {
        return Some(
            "enum return type has a 64-bit/float underlying ABI with no faithful JS \
             enum projection; refusing to emit an unsafe fallback.",
        );
    }
    match t {
        FlatAbiType::Unknown => Some(
            "return type could not be classified; refusing to emit an ABI-unsafe I32 fallback",
        ),
        _ => None,
    }
}

fn unsupported_param_reason(t: &FlatAbiType) -> Option<&'static str> {
    // Enum params (by value) OR enum out-params (PtrTo(Enum)) with a 64-bit/float
    // underlying can't be faithfully represented (i32-backed members, number-typed
    // surface), so skip rather than emit ABI-mismatched constants/calling convention.
    if enum_underlying_unrepresentable(t)
        || matches!(t, FlatAbiType::PtrTo(inner) if enum_underlying_unrepresentable(inner))
    {
        return Some(
            "enum parameter has a 64-bit/float underlying ABI that the JS enum surface \
             cannot faithfully represent; refusing to emit an ABI-mismatched wrapper.",
        );
    }
    match t {
        FlatAbiType::Unknown => Some(
            "parameter type could not be classified as a by-value ABI type; \
             refusing to emit a wrapper that would pass a pointer where the callee \
             expects an inline value",
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Per-param classification
// ---------------------------------------------------------------------------

/// How a flat parameter surfaces in the generated JS wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParamSurface {
    /// Value passed by the caller (scalar, handle, enum, string, or opaque pointer).
    Input,
    /// Caller passes an initial value (a scalar); the wrapper allocates a
    /// slot, writes the caller's value, calls the API, and reads the final
    /// value back. Both a param slot and a return-object field appear.
    InOutScalar,
    /// A pure `[out]` pointer to a small scalar. The wrapper allocates the
    /// slot internally and projects the value into the return object.
    OutScalar,
    /// Opaque pointer — remains in the argument list as `Buffer|bigint|null`.
    OpaquePointer,
}

fn classify(p: &FlatParamMeta) -> ParamSurface {
    match &p.abi {
        FlatAbiType::PtrTo(inner) => {
            let is_projectable = is_small_scalarish(inner);
            match (p.direction, is_projectable) {
                (FlatDirection::Out, true) => ParamSurface::OutScalar,
                (FlatDirection::InOut, true) => ParamSurface::InOutScalar,
                _ => ParamSurface::OpaquePointer,
            }
        }
        FlatAbiType::Ptr => ParamSurface::OpaquePointer,
        // A PWSTR/PSTR (LPWSTR/LPSTR) parameter marked `[out]` or
        // `[in,out]` is a caller-allocated output buffer (e.g.
        // `RegEnumKeyW(..., LPWSTR name, ...)`, `RegLoadMUIStringW`), NOT
        // a read-only string input. Surfacing it as `string | null` and
        // marshalling via `_wideStringBuffer` would make these APIs
        // unusable (the caller can't observe what was written into the
        // freshly-allocated internal buffer). Route them through
        // `OpaquePointer` so the caller supplies a Buffer they own,
        // matching the actual Win32 usage pattern.
        FlatAbiType::PWStr | FlatAbiType::PStr
            if matches!(p.direction, FlatDirection::Out | FlatDirection::InOut) =>
        {
            ParamSurface::OpaquePointer
        }
        _ => ParamSurface::Input,
    }
}

fn is_small_scalarish(t: &FlatAbiType) -> bool {
    // NOTE: U8/I8 are intentionally EXCLUDED. Byte-sized pointer params in
    // Win32 are overwhelmingly caller-allocated buffers (e.g.
    // `RegQueryValueExW`'s `lpData: LPBYTE` with a separate `lpcbData: DWORD`
    // size slot). Projecting them as scalar returns would silently promote a
    // 1-byte read to the return object AND hide the buffer semantics.
    matches!(
        t,
        FlatAbiType::I16
            | FlatAbiType::U16
            | FlatAbiType::I32
            | FlatAbiType::U32
            | FlatAbiType::I64
            | FlatAbiType::U64
            | FlatAbiType::Bool32
            | FlatAbiType::Handle { .. }
            | FlatAbiType::Enum { .. }
    )
}

// ---------------------------------------------------------------------------
// Return / status classification
// ---------------------------------------------------------------------------

/// Whether the method's return should project as a Win32 `.status` numeric
/// field. Backed by `FlatMethodMeta::return_is_status`, which is set at
/// parse time by inspecting the raw winmd Type (HRESULT/NTSTATUS/LSTATUS)
/// and the mapped enum name (WIN32_ERROR-family). Deliberately does NOT
/// treat every I32/U32 as a status code — plain integer returns like
/// `GetCurrentProcessId -> u32` or `MulDiv -> i32` are real values and
/// must project as `{ result: number }`, not `{ status: number }`.
fn is_status_return(m: &FlatMethodMeta) -> bool {
    m.return_is_status
}

fn flat_ret_kind_literal(t: &FlatAbiType) -> &'static str {
    // Map return type to the string literal passed to DynWinRtValue.flatInvoke.
    match t {
        FlatAbiType::I32
        | FlatAbiType::I16
        | FlatAbiType::I8
        | FlatAbiType::Bool
        | FlatAbiType::Bool32 => "I32",
        FlatAbiType::U32 | FlatAbiType::U16 | FlatAbiType::U8 | FlatAbiType::Char16 => "U32",
        FlatAbiType::I64 => "I64",
        FlatAbiType::U64 => "U64",
        FlatAbiType::Enum { underlying, .. } => match **underlying {
            FlatAbiType::I32 => "I32",
            FlatAbiType::I8 => "I32",
            FlatAbiType::I16 => "I32",
            _ => "U32",
        },
        FlatAbiType::Void => "Void",
        FlatAbiType::Ptr
        | FlatAbiType::PtrTo(_)
        | FlatAbiType::PWStr
        | FlatAbiType::PStr
        | FlatAbiType::Handle { .. } => "Ptr",
        FlatAbiType::F32 => "F32",
        FlatAbiType::F64 => "F64",
        FlatAbiType::Unknown => {
            debug_assert!(
                false,
                "flat_ret_kind_literal: Unknown return should have been filtered upstream"
            );
            "I32"
        }
    }
}

fn flat_ret_decode_expr(t: &FlatAbiType, ret_kind: &str) -> String {
    match (t, ret_kind) {
        (FlatAbiType::Bool | FlatAbiType::Bool32, _) => "(_ret.toNumber() !== 0)".to_string(),
        (_, "Ptr") => "_ret.asPointerBigint()".to_string(),
        (_, "I64") => "_ret.toI64BigInt()".to_string(),
        (_, "U64") => "_ret.toU64BigInt()".to_string(),
        (_, "F32" | "F64") => "_ret.toF64()".to_string(),
        (_, "Void") => "undefined".to_string(),
        _ => "_ret.toNumber()".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------

fn camel_case(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() && chars[i].is_ascii_uppercase() {
        i += 1;
    }
    if i == 0 {
        return s.to_string();
    }
    if i == chars.len() {
        return s.to_ascii_lowercase();
    }
    if i == 1 {
        let mut out = String::with_capacity(s.len());
        out.push(chars[0].to_ascii_lowercase());
        for c in &chars[1..] {
            out.push(*c);
        }
        return out;
    }
    // Multi-char uppercase followed by lowercase: last uppercase begins the next word.
    let mut out = String::with_capacity(s.len());
    for c in &chars[..i - 1] {
        out.push(c.to_ascii_lowercase());
    }
    for c in &chars[i - 1..] {
        out.push(*c);
    }
    out
}

fn js_param_name(raw: &str, idx: usize) -> String {
    let base = if raw.is_empty() {
        format!("arg{}", idx)
    } else {
        raw.to_string()
    };
    let stripped = strip_hungarian(&base);
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars();
    if let Some(first) = chars.next() {
        out.push(first.to_ascii_lowercase());
    }
    for c in chars {
        out.push(c);
    }
    // Reserved-word guard.
    match out.as_str() {
        "class" | "return" | "function" | "default" | "this" | "new" | "delete" | "let" | "const"
        | "var" | "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "break"
        | "continue" | "true" | "false" | "null" | "undefined" | "in" | "of" | "typeof"
        | "instanceof" | "throw" | "try" | "catch" | "finally" | "yield" | "async" | "await"
        | "with" | "void" | "public" | "private" | "protected" | "package" | "static" | "import"
        | "export" | "extends" | "super" | "arguments" | "status" | "result" => format!("{}_", out),
        _ => out,
    }
}

/// Compute per-method JS parameter names, deduplicating collisions. Two
/// different Win32 params can strip to the same identifier (e.g.
/// `RegLoadMUIStringA` has both `pOutBuf` and `OutBuf` which both reduce to
/// `outBuf`). Duplicate parameter names are a fatal SyntaxError in strict
/// mode, so we suffix collisions with `_2`, `_3`, ... in encounter order.
fn js_param_names_for_method(m: &FlatMethodMeta) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(m.params.len());
    for (i, p) in m.params.iter().enumerate() {
        let base = js_param_name(&p.name, i);
        let name = match seen.get(&base).copied() {
            Some(n) => {
                let renamed = format!("{}_{}", base, n + 1);
                seen.insert(base.clone(), n + 1);
                renamed
            }
            None => {
                seen.insert(base.clone(), 1);
                base
            }
        };
        out.push(name);
    }
    out
}

fn strip_hungarian(s: &str) -> &str {
    let prefixes = [
        "lpwsz", "pwsz", "lpsz", "psz", "pwstr", "pcwstr", "lp", "pp", "ppv", "hwnd", "dw", "sz",
        "cb", "cx", "cy", "cw", "ch", "cn", "cc", "np", "ph", "pd", "pf", "pv",
    ];
    for p in prefixes {
        if let Some(rest) = s.strip_prefix(p) {
            if rest
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
            {
                return rest;
            }
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Type surface
// ---------------------------------------------------------------------------

fn dts_type_of(t: &FlatAbiType) -> String {
    match t {
        FlatAbiType::Void => "void".into(),
        FlatAbiType::Bool | FlatAbiType::Bool32 => "boolean".into(),
        FlatAbiType::I8
        | FlatAbiType::U8
        | FlatAbiType::I16
        | FlatAbiType::U16
        | FlatAbiType::I32
        | FlatAbiType::U32
        | FlatAbiType::Char16 => "number".into(),
        FlatAbiType::I64 | FlatAbiType::U64 => "bigint".into(),
        FlatAbiType::F32 | FlatAbiType::F64 => "number".into(),
        FlatAbiType::PWStr | FlatAbiType::PStr => "string | null".into(),
        FlatAbiType::Handle { name, .. } => name.clone(),
        FlatAbiType::Enum { name, .. } => name.clone(),
        FlatAbiType::Ptr => "bigint | Buffer | null".into(),
        FlatAbiType::PtrTo(_) => "bigint | Buffer | null".into(),
        // Opaque type we couldn't classify from metadata. At runtime it is
        // marshalled as `DynWinRtValue.pointer(var)` (the same shape as
        // `Ptr`), so the .d.ts input type must match the runtime contract:
        // a pointer-like BigInt/Buffer, not a permissive `unknown`. Using
        // `unknown` here silently accepts arbitrary JS values that would
        // then crash inside `DynWinRtValue.pointer(...)` with a type error.
        FlatAbiType::Unknown => "bigint | Buffer | null".into(),
    }
}

/// Return-position type for the flat wrapper.
///
/// Distinct from [`dts_type_of`] because the runtime read side
/// (`render_method_js` around `_ret.asPointerBigint()` / `_ret.toNumber()`)
/// produces different JS values than the input-side types [`dts_type_of`]
/// accepts. Concretely: any `retKind === "Ptr"` (per
/// [`flat_ret_kind_literal`] — `Ptr`, `PtrTo(_)`, `PWStr`, `PStr`,
/// `Handle{..}`) is unconditionally converted via `asPointerBigint()`,
/// which returns a plain `bigint` (`0n` for null). Typing the `.d.ts`
/// `result` as `bigint | Buffer | null` or `string | null` (as
/// [`dts_type_of`] does for input params) would misdescribe the runtime.
/// All other kinds match [`dts_type_of`]: booleans → `boolean`, small
/// integers → `number` (from `_ret.toNumber()`), enums → their alias.
fn dts_return_type_of(t: &FlatAbiType) -> String {
    match t {
        FlatAbiType::Void => "void".into(),
        FlatAbiType::Ptr
        | FlatAbiType::PtrTo(_)
        | FlatAbiType::PWStr
        | FlatAbiType::PStr
        | FlatAbiType::Handle { .. } => "bigint".into(),
        _ => dts_type_of(t),
    }
}

// ---------------------------------------------------------------------------
// .js rendering
// ---------------------------------------------------------------------------

fn render_js(meta: &FlatApisMeta) -> String {
    let mut out = String::new();
    out.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    out.push_str("// Flat-Win32 [DllImport] wrappers for ");
    out.push_str(&meta.namespace);
    out.push_str(".");
    out.push_str(&meta.class_name);
    out.push_str("\n");
    out.push_str("//\n// Each exported function is a natural JS wrapper around\n");
    out.push_str("// DynWinRtValue.flatInvoke(dll, entry, retKind, args). Pointer-to-scalar\n");
    out.push_str("// [out]/[in,out] params are projected as return-object fields; opaque\n");
    out.push_str("// pointer params (Buffer|bigint|null) stay in the argument list.\n\n");
    // Honor `--import-name`: the CLI stores the runtime package name (or a
    // relative path when generating against a local build) in a process-wide
    // slot managed by javascript::project. Falls back to '@microsoft/dynwinrt'.
    let runtime_import = crate::codegen::javascript::project::get_import_name();
    out.push_str(&format!(
        "import {{ DynWinRtValue }} from '{runtime_import}';\n\n"
    ));

    // A small runtime helper for wide- and narrow-string marshalling.
    // Emitted inline so the generated file has no cross-file runtime
    // dependencies beyond `dynwinrt`.
    let mut methods_js = String::new();
    for m in &meta.methods {
        render_method_js(&mut methods_js, m);
        methods_js.push('\n');
    }

    out.push_str(WIDE_STRING_HELPER);
    out.push_str(NARROW_STRING_HELPER);
    // The handle-slot helper is only needed when a method writes a `bigint |
    // number` handle into an in/out 64-bit slot; emit it only if referenced.
    if methods_js.contains("_handleU64(") {
        out.push_str(HANDLE_SLOT_HELPER);
    }
    out.push_str("\n");
    out.push_str(&methods_js);

    // Aggregate exports as a frozen object, mirroring the classic-COM
    // `export class` shape but for a module-namespace of functions.
    out.push_str("export const Apis = Object.freeze({\n");
    for m in &meta.methods {
        let camel = camel_case(&m.name);
        out.push_str(&format!("    {camel},\n"));
    }
    out.push_str("});\n");

    // Also emit named DLL/entry constants for advanced callers.
    out.push_str("\n// Raw metadata for each export (dll, entry point).\n");
    out.push_str("export const FLAT_EXPORTS = Object.freeze({\n");
    for m in &meta.methods {
        let camel = camel_case(&m.name);
        out.push_str(&format!(
            "    {camel}: {{ dll: '{}', entry: '{}' }},\n",
            m.dll, m.entry_point
        ));
    }
    out.push_str("});\n");

    out
}

const WIDE_STRING_HELPER: &str = "\
// Build a NUL-terminated UTF-16LE Buffer for LPCWSTR args. Rejects embedded
// U+0000 up front — Win32 wide-string APIs would silently truncate at the
// first NUL, which is a source of validation-bypass bugs.
function _wideStringBuffer(str) {
    if (str === null || str === undefined) return null;
    if (typeof str !== 'string') {
        throw new TypeError(`expected string, got ${typeof str}`);
    }
    if (str.indexOf('\\u0000') !== -1) {
        throw new RangeError('string contains embedded NUL (U+0000)');
    }
    const buf = Buffer.alloc((str.length + 1) * 2);
    buf.write(str, 'utf16le');
    return buf;
}
";

const NARROW_STRING_HELPER: &str = "\
// Build a NUL-terminated UTF-8 Buffer for LPCSTR/PSTR args. Distinct from
// the wide-string helper because ANSI/UTF-8 Win32 A-suffixed exports
// (e.g. `RegOpenKeyExA`) take a single-byte `char*`, not `wchar_t*` —
// writing UTF-16LE bytes into them corrupts parameters and can smash the
// callee's stack. On modern Windows (10 1903+) with the app manifested
// for UTF-8 ACP, or on OS versions that natively accept UTF-8 for A-APIs,
// this is the correct encoding. This typed wrapper always UTF-8-encodes the
// string; a caller needing a different/legacy ANSI code page must bypass the
// generated wrapper and call `DynWinRtValue.flatInvoke` directly with a
// pre-encoded Buffer (this helper only accepts a JS string).
// Rejects embedded U+0000 for the same truncation-safety reason as the
// wide-string helper.
function _narrowStringBuffer(str) {
    if (str === null || str === undefined) return null;
    if (typeof str !== 'string') {
        throw new TypeError(`expected string, got ${typeof str}`);
    }
    if (str.indexOf('\\u0000') !== -1) {
        throw new RangeError('string contains embedded NUL (U+0000)');
    }
    const byteLen = Buffer.byteLength(str, 'utf8');
    const buf = Buffer.alloc(byteLen + 1);
    buf.write(str, 'utf8');
    return buf;
}
";

const HANDLE_SLOT_HELPER: &str = "\
// Coerce a handle (bigint | number) to a BigInt for a 64-bit in/out slot.
// A bigint carries full 64-bit handle bits; a number must be a non-negative
// safe integer (a number above 2^53-1 has already lost bits, so it is
// rejected rather than silently writing a wrong handle).
function _handleU64(x) {
    if (typeof x === 'bigint') return x;
    if (typeof x === 'number') {
        if (!Number.isSafeInteger(x) || x < 0) {
            throw new RangeError('handle number must be a non-negative safe integer (use a bigint for a full 64-bit handle)');
        }
        return BigInt(x);
    }
    throw new TypeError(`expected a bigint or number handle, got ${typeof x}`);
}
";

fn render_method_js(out: &mut String, m: &FlatMethodMeta) {
    let camel = camel_case(&m.name);
    let ret_kind = flat_ret_kind_literal(&m.return_type);

    // Classify params
    let classified: Vec<(usize, ParamSurface)> = m
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| (i, classify(p)))
        .collect();

    // Compute JS parameter names ONCE with collision-avoidance so downstream
    // sites (argument list, JSDoc, slot names, arg wrappers, result object)
    // all agree — a duplicate JS identifier would be a fatal SyntaxError.
    let jnames: Vec<String> = js_param_names_for_method(m);

    // Names for arg list (all except OutScalar).
    let mut param_names: Vec<String> = Vec::new();
    for (i, s) in &classified {
        if *s != ParamSurface::OutScalar {
            param_names.push(jnames[*i].clone());
        }
    }

    // Emit function
    out.push_str("/**\n");
    out.push_str(&format!(" * {} — {} export.\n", m.name, m.dll));
    out.push_str(" *\n");
    for (i, p) in m.params.iter().enumerate() {
        let kind = match &classified[i].1 {
            ParamSurface::Input => "in",
            ParamSurface::InOutScalar => "in,out",
            ParamSurface::OutScalar => "out",
            ParamSurface::OpaquePointer => "in/out pointer",
        };
        out.push_str(&format!(
            " * @param {}  [{}] {}\n",
            jnames[i],
            kind,
            describe_abi(&p.abi)
        ));
    }
    out.push_str(&format!(
        " * @returns {}\n",
        describe_return_shape(m, &classified, &jnames)
    ));
    out.push_str(" */\n");

    out.push_str(&format!(
        "export function {camel}({}) {{\n",
        param_names.join(", ")
    ));

    // Emit slot allocations for OutScalar / InOutScalar params.
    for (i, s) in &classified {
        let p = &m.params[*i];
        let jname = &jnames[*i];
        let slot = format!("_{jname}Slot");
        match s {
            ParamSurface::OutScalar => {
                let (alloc, _read) = scalar_slot_alloc_and_read(&pointee(&p.abi));
                out.push_str(&format!("    const {slot} = {alloc};\n"));
            }
            ParamSurface::InOutScalar => {
                let inner = pointee(&p.abi);
                let (alloc, _read) = scalar_slot_alloc_and_read(&inner);
                let writer = scalar_slot_write(&inner, jname);
                out.push_str(&format!("    const {slot} = {alloc};\n"));
                let write_line = writer.replace.replace("{slot}", &slot);
                out.push_str(&format!("    {write_line};\n"));
            }
            _ => {}
        }
    }

    // Emit keep-alive locals for wide/narrow string buffers so the
    // freshly-allocated Buffer stays reachable from a JS local through
    // the flatInvoke call. `DynWinRtValue.pointer(Buffer)` extracts the
    // Buffer's `as_ptr()` but does NOT retain the Buffer itself, so the
    // temporary `_wideStringBuffer(x)` / `_narrowStringBuffer(x)` value
    // would become unreachable the moment `pointer(...)` returned and
    // could be reclaimed by GC before the callee runs — passing a
    // dangling pointer to the flat Win32 export. A named `const` in the
    // function's stack frame keeps the Buffer alive across the invoke
    // call (JS engines must consider identifiers reachable through
    // the enclosing scope until they leave scope), which is the same
    // pattern used for the out/in-out `_*Slot` Buffers above.
    let mut string_keepalive: Vec<(usize, String, &'static str)> = Vec::new();
    for (i, s) in &classified {
        if *s != ParamSurface::Input {
            continue;
        }
        let p = &m.params[*i];
        let jname = &jnames[*i];
        match &p.abi {
            FlatAbiType::PWStr => {
                let local = format!("_{jname}Buf");
                out.push_str(&format!(
                    "    const {local} = _wideStringBuffer({jname});\n"
                ));
                string_keepalive.push((*i, local, "wide"));
            }
            FlatAbiType::PStr => {
                let local = format!("_{jname}Buf");
                out.push_str(&format!(
                    "    const {local} = _narrowStringBuffer({jname});\n"
                ));
                string_keepalive.push((*i, local, "narrow"));
            }
            _ => {}
        }
    }

    // Build the flatInvoke args array.
    let mut arg_exprs: Vec<String> = Vec::with_capacity(m.params.len());
    for (i, s) in &classified {
        let p = &m.params[*i];
        let jname = &jnames[*i];
        let expr = match s {
            ParamSurface::OutScalar | ParamSurface::InOutScalar => {
                let slot = format!("_{jname}Slot");
                format!("DynWinRtValue.pointer({slot})")
            }
            ParamSurface::OpaquePointer => {
                // Caller-supplied Buffer / bigint / null — pass through
                // untouched. Skip `wrap_arg_js`, which would incorrectly
                // apply the string-input transformation (`_wideStringBuffer`
                // et al.) to a PWStr/PStr param that the caller wants to
                // treat as a raw byte buffer.
                format!("DynWinRtValue.pointer({jname})")
            }
            ParamSurface::Input => {
                // If this is a string param with a keep-alive local,
                // pass the local directly to pointer() — do NOT recreate
                // a fresh temp Buffer inline.
                if let Some((_, local, _)) = string_keepalive.iter().find(|(idx, _, _)| idx == i) {
                    format!("DynWinRtValue.pointer({local})")
                } else {
                    wrap_arg_js(&p.abi, jname)
                }
            }
        };
        arg_exprs.push(expr);
    }

    let args_line = arg_exprs.join(", ");
    out.push_str(&format!(
        "    const _ret = DynWinRtValue.flatInvoke('{}', '{}', '{}', [{}]);\n",
        m.dll, m.entry_point, ret_kind, args_line,
    ));
    let ret_val = flat_ret_decode_expr(&m.return_type, ret_kind);

    // Compose the return.
    let has_projected_out = classified
        .iter()
        .any(|(_, s)| matches!(s, ParamSurface::OutScalar | ParamSurface::InOutScalar));
    if !has_projected_out {
        // Simple return: status/return value.
        if matches!(m.return_type, FlatAbiType::Void) {
            out.push_str("    return undefined;\n");
        } else if is_status_return(m) {
            out.push_str(&format!("    return {{ status: {ret_val} }};\n"));
        } else {
            out.push_str(&format!("    return {{ result: {ret_val} }};\n"));
        }
    } else {
        // Build result object.
        out.push_str("    return {\n");
        if is_status_return(m) {
            out.push_str(&format!("        status: {ret_val},\n"));
        } else if !matches!(m.return_type, FlatAbiType::Void) {
            out.push_str(&format!("        result: {ret_val},\n"));
        }
        for (i, s) in &classified {
            if !matches!(s, ParamSurface::OutScalar | ParamSurface::InOutScalar) {
                continue;
            }
            let p = &m.params[*i];
            let jname = &jnames[*i];
            let slot = format!("_{jname}Slot");
            let (_alloc, read) = scalar_slot_alloc_and_read(&pointee(&p.abi));
            let read_expr = read.replace("{slot}", &slot);
            out.push_str(&format!("        {jname}: {read_expr},\n"));
        }
        out.push_str("    };\n");
    }

    out.push_str("}\n");
}

fn pointee(t: &FlatAbiType) -> FlatAbiType {
    match t {
        FlatAbiType::PtrTo(inner) => (**inner).clone(),
        _ => FlatAbiType::U32,
    }
}

struct WriteExpr {
    replace: String,
}

impl WriteExpr {
    fn new(s: &str) -> Self {
        Self {
            replace: s.to_string(),
        }
    }
}

/// Returns (alloc-expression, read-expression) for a caller-side Buffer slot
/// backing a scalar out or inout parameter. The read expression contains the
/// literal placeholder `{slot}` to substitute with the slot variable name.
fn scalar_slot_alloc_and_read(t: &FlatAbiType) -> (String, String) {
    match t {
        FlatAbiType::I8 => (
            "Buffer.alloc(1)".into(),
            "{slot}.readInt8(0)".into(),
        ),
        FlatAbiType::U8 => ("Buffer.alloc(1)".into(), "{slot}.readUInt8(0)".into()),
        FlatAbiType::I16 => ("Buffer.alloc(2)".into(), "{slot}.readInt16LE(0)".into()),
        FlatAbiType::U16 | FlatAbiType::Char16 => {
            ("Buffer.alloc(2)".into(), "{slot}.readUInt16LE(0)".into())
        }
        FlatAbiType::Bool | FlatAbiType::Bool32 => (
            "Buffer.alloc(4)".into(),
            "({slot}.readInt32LE(0) !== 0)".into(),
        ),
        FlatAbiType::I32 => {
            ("Buffer.alloc(4)".into(), "{slot}.readInt32LE(0)".into())
        }
        FlatAbiType::U32 => ("Buffer.alloc(4)".into(), "{slot}.readUInt32LE(0)".into()),
        FlatAbiType::I64 => ("Buffer.alloc(8)".into(), "{slot}.readBigInt64LE(0)".into()),
        FlatAbiType::U64 | FlatAbiType::Handle { .. } => (
            // Handles are pointer-sized on x64; use 8-byte BigUInt64 for both
            // storage and read-back.
            "Buffer.alloc(8)".into(),
            "{slot}.readBigUInt64LE(0)".into(),
        ),
        FlatAbiType::Enum { underlying, .. } => match **underlying {
            FlatAbiType::U32 => (
                "Buffer.alloc(4)".into(),
                "({slot}.readUInt32LE(0) | 0)".into(),
            ),
            _ => scalar_slot_alloc_and_read(underlying),
        },
        _ => (
            // Fallback: 4-byte slot as an u32 (matches most Win32 DWORDs).
            "Buffer.alloc(4)".into(),
            "{slot}.readUInt32LE(0)".into(),
        ),
    }
}

/// Write-expression for an inout scalar slot. Returns a `WriteExpr` where
/// `.replace` contains `{slot}` to substitute with the slot variable name.
fn scalar_slot_write(t: &FlatAbiType, value_var: &str) -> WriteExpr {
    match t {
        FlatAbiType::I8 => WriteExpr::new(&format!("{{slot}}.writeInt8({value_var}, 0)")),
        FlatAbiType::U8 => WriteExpr::new(&format!("{{slot}}.writeUInt8({value_var}, 0)")),
        FlatAbiType::I16 => WriteExpr::new(&format!("{{slot}}.writeInt16LE({value_var}, 0)")),
        FlatAbiType::U16 | FlatAbiType::Char16 => {
            WriteExpr::new(&format!("{{slot}}.writeUInt16LE({value_var}, 0)"))
        }
        FlatAbiType::I32 | FlatAbiType::Bool32 => {
            WriteExpr::new(&format!("{{slot}}.writeInt32LE({value_var}, 0)"))
        }
        FlatAbiType::U32 => WriteExpr::new(&format!("{{slot}}.writeUInt32LE({value_var}, 0)")),
        FlatAbiType::I64 => WriteExpr::new(&format!(
            "{{slot}}.writeBigInt64LE(BigInt({value_var}), 0)"
        )),
        FlatAbiType::U64 => WriteExpr::new(&format!(
            "{{slot}}.writeBigUInt64LE(BigInt({value_var}), 0)"
        )),
        // Handle in-out slots accept both bigint and number (Buffer is
        // intentionally NOT a valid Handle input — see the handle typedef
        // in the .d.ts — because `DynWinRtValue.pointer(Buffer)` uses the
        // buffer's own address, not the bytes it contains). Route through
        // `_handleU64`, which carries a bigint losslessly and rejects a
        // number that is not a non-negative safe integer (a number above
        // 2^53-1 has already lost bits, so `BigInt(x)` would write a wrong
        // handle silently).
        FlatAbiType::Handle { .. } => WriteExpr::new(&format!(
            "{{slot}}.writeBigUInt64LE(_handleU64({value_var}), 0)"
        )),
        FlatAbiType::Enum { underlying, .. } => match **underlying {
            FlatAbiType::U32 => WriteExpr::new(&format!(
                "{{slot}}.writeUInt32LE(({value_var}) >>> 0, 0)"
            )),
            _ => scalar_slot_write(underlying, value_var),
        },
        _ => WriteExpr::new(&format!("{{slot}}.writeUInt32LE({value_var}, 0)")),
    }
}

fn wrap_arg_js(t: &FlatAbiType, var: &str) -> String {
    match t {
        FlatAbiType::Bool => format!("DynWinRtValue.i32({var} ? 1 : 0)"),
        FlatAbiType::Bool32 => format!("DynWinRtValue.i32({var} ? 1 : 0)"),
        FlatAbiType::I8 => format!("DynWinRtValue.i32({var})"),
        FlatAbiType::U8 => format!("DynWinRtValue.u32({var})"),
        FlatAbiType::I16 => format!("DynWinRtValue.i32({var})"),
        FlatAbiType::U16 | FlatAbiType::Char16 => format!("DynWinRtValue.u32({var})"),
        FlatAbiType::I32 => format!("DynWinRtValue.i32({var})"),
        FlatAbiType::U32 => format!("DynWinRtValue.u32({var})"),
        FlatAbiType::I64 => format!("DynWinRtValue.i64(BigInt({var}))"),
        FlatAbiType::U64 => format!("DynWinRtValue.u64(BigInt({var}))"),
        // Emit correctly-typed float wrappers so the value round-trips as
        // an IEEE-754 float, not a mis-marshalled pointer. If the Rust
        // `flat_invoke` path doesn't yet accept F32/F64 args, this will
        // throw a clear "unsupported arg kind" — fail loud, not silently
        // wrong. Never emit `pointer(<float>)` here.
        FlatAbiType::F32 => format!("DynWinRtValue.f32({var})"),
        FlatAbiType::F64 => format!("DynWinRtValue.f64({var})"),
        FlatAbiType::PWStr => {
            format!("DynWinRtValue.pointer(_wideStringBuffer({var}))")
        }
        FlatAbiType::PStr => {
            format!("DynWinRtValue.pointer(_narrowStringBuffer({var}))")
        }
        // Handles: type is `bigint | number` (see the handle typedef in
        // the .d.ts). Pass the value straight through to `pointer`, which
        // accepts `bigint | number`: a bigint carries full 64-bit handle
        // bits losslessly, and a JS number is validated as a safe integer
        // (unsafe values are rejected, not silently truncated). Do NOT wrap
        // in `BigInt(x)` — for a number above Number.MAX_SAFE_INTEGER the
        // bits are already lost before BigInt sees them, and wrapping also
        // bypasses `pointer`'s safe-integer validation.
        FlatAbiType::Handle { .. } => format!("DynWinRtValue.pointer({var})"),
        FlatAbiType::Ptr | FlatAbiType::PtrTo(_) => format!("DynWinRtValue.pointer({var})"),
        FlatAbiType::Enum { underlying, .. } => match **underlying {
            FlatAbiType::U32 => format!("DynWinRtValue.u32(({var}) >>> 0)"),
            _ => wrap_arg_js(underlying, var),
        },
        FlatAbiType::Void | FlatAbiType::Unknown => {
            format!("DynWinRtValue.pointer({var})")
        }
    }
}

fn describe_abi(t: &FlatAbiType) -> String {
    match t {
        FlatAbiType::Handle { name, .. } => format!("{name} handle"),
        FlatAbiType::PWStr => "LPCWSTR string".into(),
        FlatAbiType::PStr => "LPCSTR string".into(),
        FlatAbiType::Enum { name, .. } => format!("{name} enum"),
        FlatAbiType::Ptr => "opaque pointer".into(),
        FlatAbiType::PtrTo(inner) => format!("pointer to {}", describe_abi(inner)),
        other => format!("{other:?}"),
    }
}

fn describe_return_shape(
    m: &FlatMethodMeta,
    classified: &[(usize, ParamSurface)],
    jnames: &[String],
) -> String {
    let outs: Vec<(usize, &FlatParamMeta)> = classified
        .iter()
        .filter(|(_, s)| matches!(s, ParamSurface::OutScalar | ParamSurface::InOutScalar))
        .map(|(i, _)| (*i, &m.params[*i]))
        .collect();
    if outs.is_empty() {
        if matches!(m.return_type, FlatAbiType::Void) {
            "undefined".into()
        } else if is_status_return(m) {
            "{ status: number }".into()
        } else {
            "{ result: <return> }".into()
        }
    } else {
        let mut parts: Vec<String> = Vec::new();
        if is_status_return(m) {
            parts.push("status: number".into());
        } else if !matches!(m.return_type, FlatAbiType::Void) {
            parts.push("result: <return>".into());
        }
        // Use the SANITIZED JS identifiers (jnames) — not raw winmd param
        // names — because the emitter uses these same identifiers as the
        // return-object field names (see the `return { <jname>: ... }` emit
        // site). Documenting `p.name` would show Hungarian-prefixed / raw
        // names that don't actually exist on the returned object.
        for (i, _p) in outs {
            parts.push(format!("{}: <out>", jnames[i]));
        }
        format!("{{ {} }}", parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// .d.ts rendering
// ---------------------------------------------------------------------------

fn render_dts(meta: &FlatApisMeta) -> String {
    let mut out = String::new();
    out.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    out.push_str("// Flat-Win32 [DllImport] wrappers for ");
    out.push_str(&meta.namespace);
    out.push_str(".");
    out.push_str(&meta.class_name);
    out.push_str("\n\n");

    // Import referenced enums as type-only imports.
    let mut enum_imports: BTreeSet<String> = BTreeSet::new();
    for e in &meta.referenced_enums {
        if let TypeMeta::Enum { name, .. } = e {
            enum_imports.insert(name.clone());
        }
    }
    for name in &enum_imports {
        out.push_str(&format!("import {{ {name} }} from './{name}.js';\n"));
    }
    if !enum_imports.is_empty() {
        out.push('\n');
    }

    // Emit handle typedef aliases.
    let handle_aliases = collect_handle_aliases(meta);
    for h in &handle_aliases {
        out.push_str(&format!(
            "/** Opaque Win32 handle. Pass either a raw pointer value as a `bigint` (safe for full 64-bit handle values) or a `number` (only for handles that fit in a JS safe integer, e.g. HWND with small window IDs). Do NOT pass a `Buffer` — `DynWinRtValue.pointer(Buffer)` uses the buffer's own address, not the bytes it contains, so a Buffer of pointer bits would be misinterpreted as a pointer TO a `{h}`. */\nexport type {h} = bigint | number;\n"
        ));
    }
    if !handle_aliases.is_empty() {
        out.push('\n');
    }

    for m in &meta.methods {
        render_method_dts(&mut out, m);
        out.push('\n');
    }

    // Aggregate object type.
    out.push_str("export declare const Apis: {\n");
    for m in &meta.methods {
        let camel = camel_case(&m.name);
        out.push_str(&format!("    {camel}: typeof {camel};\n"));
    }
    out.push_str("};\n\n");
    out.push_str("export declare const FLAT_EXPORTS: Readonly<Record<string, { dll: string; entry: string }>>;\n");

    out
}

fn render_method_dts(out: &mut String, m: &FlatMethodMeta) {
    let camel = camel_case(&m.name);
    let classified: Vec<(usize, ParamSurface)> = m
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| (i, classify(p)))
        .collect();

    // Match .js name-generation exactly (including collision suffixes).
    let jnames: Vec<String> = js_param_names_for_method(m);

    // Argument list (Input, InOutScalar, OpaquePointer).
    let mut params: Vec<String> = Vec::new();
    for (i, s) in &classified {
        let p = &m.params[*i];
        let jname = &jnames[*i];
        let ts_ty = match s {
            ParamSurface::Input => dts_type_of(&p.abi),
            ParamSurface::InOutScalar => dts_type_of(&pointee(&p.abi)),
            ParamSurface::OutScalar => continue,
            ParamSurface::OpaquePointer => "bigint | Buffer | Uint8Array | null".into(),
        };
        params.push(format!("{jname}: {ts_ty}"));
    }

    // Return type. Collect (index, param) so we can look up the deduped name.
    let out_indices: Vec<usize> = classified
        .iter()
        .filter(|(_, s)| matches!(s, ParamSurface::OutScalar | ParamSurface::InOutScalar))
        .map(|(i, _)| *i)
        .collect();

    let ret_ty = if out_indices.is_empty() {
        if matches!(m.return_type, FlatAbiType::Void) {
            "void".to_string()
        } else if is_status_return(m) {
            "{ readonly status: number }".to_string()
        } else {
            format!(
                "{{ readonly result: {} }}",
                dts_return_type_of(&m.return_type)
            )
        }
    } else {
        let mut fields: Vec<String> = Vec::new();
        if is_status_return(m) {
            fields.push("readonly status: number".into());
        } else if !matches!(m.return_type, FlatAbiType::Void) {
            fields.push(format!(
                "readonly result: {}",
                dts_return_type_of(&m.return_type)
            ));
        }
        for i in &out_indices {
            let p = &m.params[*i];
            let jname = &jnames[*i];
            let ty = dts_return_type_of(&pointee(&p.abi));
            fields.push(format!("readonly {jname}: {ty}"));
        }
        format!("{{ {} }}", fields.join("; "))
    };

    out.push_str(&format!(
        "/** {name} — {dll} export. */\nexport declare function {camel}({params}): {ret_ty};\n",
        name = m.name,
        dll = m.dll,
        camel = camel,
        params = params.join(", "),
        ret_ty = ret_ty,
    ));
}

// ---------------------------------------------------------------------------
// Handles + enums helpers
// ---------------------------------------------------------------------------

fn collect_handle_aliases(meta: &FlatApisMeta) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for m in &meta.methods {
        for p in &m.params {
            walk_abi_for_handles(&p.abi, &mut set);
        }
        walk_abi_for_handles(&m.return_type, &mut set);
    }
    set.into_iter().collect()
}

fn walk_abi_for_handles(t: &FlatAbiType, set: &mut BTreeSet<String>) {
    match t {
        FlatAbiType::Handle { name, .. } => {
            set.insert(name.clone());
        }
        FlatAbiType::PtrTo(inner) => walk_abi_for_handles(inner, set),
        FlatAbiType::Enum { .. }
        | FlatAbiType::Bool
        | FlatAbiType::Bool32
        | FlatAbiType::I8
        | FlatAbiType::U8
        | FlatAbiType::I16
        | FlatAbiType::U16
        | FlatAbiType::I32
        | FlatAbiType::U32
        | FlatAbiType::I64
        | FlatAbiType::U64
        | FlatAbiType::F32
        | FlatAbiType::F64
        | FlatAbiType::Char16
        | FlatAbiType::PWStr
        | FlatAbiType::PStr
        | FlatAbiType::Ptr
        | FlatAbiType::Void
        | FlatAbiType::Unknown => {}
    }
}

fn render_enum_files(en: &TypeMeta) -> (String, String) {
    let (name, members) = match en {
        TypeMeta::Enum { name, members, .. } => (name.as_str(), members),
        _ => unreachable!(),
    };
    let mut js = String::new();
    js.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    js.push_str(&format!("export const {name} = Object.freeze({{\n"));
    for m in members {
        js.push_str(&format!("    {}: {},\n", m.name, m.value));
    }
    js.push_str("});\n");

    // Emit .d.ts as a const object + companion type, matching the JS
    // `Object.freeze({...})` runtime shape and the convention used by
    // the WinRT/classic-COM enum emitters
    // (tools/dynwinrt-codegen/src/codegen/javascript/render/declarations.rs).
    // Deliberately avoids `export declare const enum` so that consumers
    // with TypeScript `isolatedModules` (Vite, esbuild, Next.js, etc.)
    // don't hit the "const enums are not usable when isolatedModules is
    // enabled" error, and so the emitted type mirrors what actually
    // exists at runtime.
    let mut dts = String::new();
    dts.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    dts.push_str(&format!(
        "export type {name} = (typeof {name})[keyof typeof {name}];\n"
    ));
    dts.push_str(&format!("export declare const {name}: {{\n"));
    for m in members {
        dts.push_str(&format!("    readonly {}: {};\n", m.name, m.value));
    }
    dts.push_str("};\n");
    (js, dts)
}

// ---------------------------------------------------------------------------
// Unit tests (no winmd — pure logic)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_flat() {
        assert_eq!(camel_case("RegOpenKeyExW"), "regOpenKeyExW");
        assert_eq!(camel_case("MulDiv"), "mulDiv");
        assert_eq!(camel_case("GetLastError"), "getLastError");
        assert_eq!(camel_case("URL"), "url");
    }

    #[test]
    fn js_param_name_reserves_return_object_keys_and_js_keywords() {
        // `status` and `result` are the return-object field names for a flat
        // wrapper; a parameter/out-field that strips to either would collide
        // with (and overwrite) the actual return value, so both are reserved.
        assert_eq!(js_param_name("status", 0), "status_");
        assert_eq!(js_param_name("result", 0), "result_");
        // JS keywords are reserved too.
        assert_eq!(js_param_name("class", 0), "class_");
        assert_eq!(js_param_name("return", 0), "return_");
        // Ordinary names are unchanged.
        assert_eq!(js_param_name("hKey", 0), "hKey");
    }

    #[test]
    fn handle_inout_slot_write_validates_via_helper() {
        // A handle in/out slot (.d.ts type `bigint | number`) must route the
        // value through `_handleU64`, which rejects lossy numbers above 2^53-1
        // rather than silently writing wrong handle bits via `BigInt(x)`.
        let h = scalar_slot_write(
            &FlatAbiType::Handle {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HANDLE".into(),
            },
            "hFile",
        );
        assert_eq!(h.replace, "{slot}.writeBigUInt64LE(_handleU64(hFile), 0)");
        // A `bigint`-typed U64 slot has no number ambiguity, so it keeps the
        // direct BigInt() coercion.
        let u = scalar_slot_write(&FlatAbiType::U64, "count");
        assert_eq!(u.replace, "{slot}.writeBigUInt64LE(BigInt(count), 0)");
    }

    #[test]
    fn handle_arg_passes_through_without_lossy_bigint_wrap() {
        // Regression (commit 16d293f): a handle ARG must be passed straight to
        // pointer() — which accepts bigint|number and validates safe integers —
        // NOT wrapped in BigInt(x). BigInt(number) for a value above 2^53-1 has
        // already lost bits and bypasses pointer()'s validation.
        let arg = wrap_arg_js(
            &FlatAbiType::Handle {
                namespace: "Windows.Win32.System.Registry".into(),
                name: "HKEY".into(),
            },
            "hKey",
        );
        assert_eq!(arg, "DynWinRtValue.pointer(hKey)");
        assert!(
            !arg.contains("BigInt("),
            "handle arg must not wrap in BigInt(): {arg}"
        );
    }

    #[test]
    fn dts_type_of_scalars_and_handles() {
        assert_eq!(dts_type_of(&FlatAbiType::Bool), "boolean");
        assert_eq!(dts_type_of(&FlatAbiType::Bool32), "boolean");
        assert_eq!(dts_type_of(&FlatAbiType::U32), "number");
        assert_eq!(dts_type_of(&FlatAbiType::I64), "bigint");
        assert_eq!(dts_type_of(&FlatAbiType::PWStr), "string | null");
        assert_eq!(
            dts_type_of(&FlatAbiType::Handle {
                namespace: "Windows.Win32.System.Registry".into(),
                name: "HKEY".into()
            }),
            "HKEY"
        );
    }

    #[test]
    fn classify_out_hkey_projects_as_return() {
        let p = FlatParamMeta {
            name: "phkResult".into(),
            direction: FlatDirection::Out,
            abi: FlatAbiType::PtrTo(Box::new(FlatAbiType::Handle {
                namespace: "Windows.Win32.System.Registry".into(),
                name: "HKEY".into(),
            })),
        };
        assert_eq!(classify(&p), ParamSurface::OutScalar);
    }

    #[test]
    fn classify_out_byte_buffer_stays_opaque() {
        // Byte-sized pointer params in Win32 are almost always caller-allocated
        // buffers with a separate size argument (e.g. RegQueryValueExW's
        // lpData/lpcbData). We deliberately keep them as OpaquePointer so the
        // caller passes a Buffer|null. The `is_small_scalarish` helper
        // excludes U8/I8 for this reason.
        let p = FlatParamMeta {
            name: "lpData".into(),
            direction: FlatDirection::Out,
            abi: FlatAbiType::PtrTo(Box::new(FlatAbiType::U8)),
        };
        assert_eq!(classify(&p), ParamSurface::OpaquePointer);
    }

    #[test]
    fn status_return_reads_flag_not_type() {
        // Since the flag is populated at parse time from raw winmd type
        // info, the unit test just verifies the accessor reads what's
        // stored — the parse-time classification is covered by snapshot
        // tests against real Win32 metadata (see registry_apis snapshot).
        fn method(return_type: FlatAbiType, return_is_status: bool) -> FlatMethodMeta {
            FlatMethodMeta {
                name: "F".into(),
                dll: "x.dll".into(),
                entry_point: "F".into(),
                return_type,
                params: vec![],
                return_is_status,
            }
        }
        assert!(is_status_return(&method(FlatAbiType::I32, true)));
        assert!(!is_status_return(&method(FlatAbiType::I32, false)));
        assert!(is_status_return(&method(
            FlatAbiType::Enum {
                namespace: "Windows.Win32.Foundation".into(),
                name: "WIN32_ERROR".into(),
                underlying: Box::new(FlatAbiType::U32),
                members: vec![],
            },
            true,
        )));
    }

    #[test]
    fn generate_end_to_end_snapshot_shape_for_synthetic_method() {
        // Synthesise a minimal Apis with one method to keep this fast and
        // hermetic (no winmd required).
        let m = FlatMethodMeta {
            name: "MulDiv".into(),
            dll: "kernel32.dll".into(),
            entry_point: "MulDiv".into(),
            return_type: FlatAbiType::I32,
            params: vec![
                FlatParamMeta {
                    name: "nNumber".into(),
                    abi: FlatAbiType::I32,
                    direction: FlatDirection::In,
                },
                FlatParamMeta {
                    name: "nNumerator".into(),
                    abi: FlatAbiType::I32,
                    direction: FlatDirection::In,
                },
                FlatParamMeta {
                    name: "nDenominator".into(),
                    abi: FlatAbiType::I32,
                    direction: FlatDirection::In,
                },
            ],
            return_is_status: false,
        };
        let apis = FlatApisMeta {
            namespace: "Test".into(),
            class_name: "Apis".into(),
            methods: vec![m],
            referenced_enums: vec![],
        };
        let out = generate_flat_apis_files(&apis);
        assert!(out.js.contains("export function mulDiv"));
        assert!(out.js.contains("flatInvoke('kernel32.dll', 'MulDiv', 'I32'"));
        assert!(out.dts.contains("mulDiv"));
    }
}
