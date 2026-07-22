// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic-COM (option A) code generation.
//!
//! This module generates natural TypeScript/JS wrappers for IUnknown-rooted
//! Win32 COM interfaces described in Windows.Win32.winmd. It intentionally
//! keeps a **separate pipeline** from the WinRT projection: classic COM has
//! meaningfully different semantics (IUnknown base offset of 3 vs 6, HRESULT
//! throw-on-failure, `CoCreateInstance` activation, no IReference/async
//! projection) so mixing them into the existing IR would obscure both paths.
//!
//! What we emit today (phase 1):
//! - `<InterfaceName>.js`: registration via `DynWinRtType.registerInterfaceUnknown`
//!   + a natural class with camelCase methods and static `create()` /
//!   `_fromNative()`.
//! - `<InterfaceName>.d.ts`: PascalCase class, camelCase methods, opaque
//!   handle typedefs (HWND etc.) as `bigint | Buffer`, HRESULT returns
//!   projected to `void` (throwing on failure via the runtime).
//! - Per-enum sibling files for each enum referenced by any method parameter.

use std::collections::BTreeSet;

use crate::meta::{ComInterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use crate::types::TypeMeta;

/// A rendered classic-COM output: primary `.js` + `.d.ts` for the interface,
/// plus zero or more sibling files (one `.js` + `.d.ts` per referenced enum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComGeneratedOutput {
    pub js: String,
    pub dts: String,
    /// Additional files (filename → content). Includes each enum's `.js` and
    /// `.d.ts`. Stable-sorted by filename for deterministic output.
    pub extra_files: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// Generate the `.js` + `.d.ts` for a classic-COM interface.
///
/// `winmd_paths` is the semicolon-separated list of `.winmd` files loaded by
/// the generator. Interop `*Interop` interfaces consult these winmds FIRST
/// to resolve the projected WinRT runtime class's default IID; if that fails
/// (e.g. the caller only passed Win32 metadata), the generator falls back to
/// the NEWEST installed `UnionMetadata\<version>\Windows.winmd`. If the target
/// IID still cannot be resolved for a confirmed interop shape, generation
/// **fails loudly** with `Err(...)` — the generator must never emit a NULL
/// riid that would silently break the wrapper at runtime.
pub fn generate_com_interface_files(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<ComGeneratedOutput, String> {
    // Detect whether this is a `*Interop` interface whose every method has the
    // `(HWND, [HSTRING…,] REFIID, out void**)` GetForWindow shape. When so, we
    // emit natural signatures that hide the REFIID + void** — the caller only
    // supplies the natural in-params, and the wrapper returns the projected
    // WinRT object. We also emit a companion runtime-class file that provides
    // the ergonomic `<ClassName>.getForWindow(hwnd)` static surface.
    let interop = detect_interop(meta, winmd_paths)?;

    let js = render_js(meta, interop.as_ref());
    let dts = render_dts(meta, interop.as_ref());

    // Per-enum sibling files (referenced by parameter types).
    let mut extra_files: Vec<(String, String)> = Vec::new();
    for en in &meta.referenced_enums {
        if let TypeMeta::Enum { name, .. } = en {
            let (enum_js, enum_dts) = render_enum_files(en);
            extra_files.push((format!("{}.js", name), enum_js));
            extra_files.push((format!("{}.d.ts", name), enum_dts));
        }
    }

    // Companion projected-class files: only when the interop resolved to a
    // real WinRT runtime class. This emits a natural `<ClassName>.js`/.d.ts
    // with a static `getForWindow(hwnd)` and a `.runtimeClassName` getter,
    // giving the E2E a MEANINGFUL surface to exercise on the returned object.
    if let Some(ref info) = interop {
        if let Some((cjs, cdts)) = render_projected_class_files(meta, info) {
            extra_files.push((format!("{}.js", info.class_name), cjs));
            extra_files.push((format!("{}.d.ts", info.class_name), cdts));
        }
    }

    extra_files.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(ComGeneratedOutput { js, dts, extra_files })
}

// ---------------------------------------------------------------------------
// Interop detection
// ---------------------------------------------------------------------------

/// Metadata for a single method within a `*Interop` interface. Each method
/// is EITHER interop-shaped (`riid + void**` trailing pair to hide) OR plain
/// (no special handling — HWND setter etc.).
#[derive(Debug, Clone)]
struct InteropMethod {
    /// Original method name (PascalCase, e.g. "GetForWindow").
    name: String,
    /// camelCase method name for JS/TS emission.
    camel: String,
    /// Absolute vtable slot.
    vtable_index: usize,
    /// `Some(natural_params)` when the method has the interop shape (last two
    /// ABI params are `(REFIID, out void**)`), i.e. the surface should hide
    /// them. `None` means "plain" — emit like a normal classic-COM method.
    natural_params: Option<Vec<ParamMeta>>,
    /// For plain methods, the underlying `MethodMeta` so we can reuse the
    /// existing emission path.
    plain: Option<MethodMeta>,
    /// Underlying method's docstring, if any.
    _doc: Option<String>,
}

/// Interop-level metadata for the whole interface.
#[derive(Debug, Clone)]
struct InteropInfo {
    /// Every method — some tagged interop-shape, some plain.
    methods: Vec<InteropMethod>,
    /// The projected WinRT runtime-class name (derived from the interop
    /// interface: `ISystemMediaTransportControlsInterop` →
    /// `SystemMediaTransportControls`).
    class_name: String,
    /// Full namespace of the projected runtime class in the WinRT metadata
    /// (e.g. `"Windows.Media"`). Empty when auto-resolution failed.
    class_namespace: String,
    /// Default interface IID of the projected runtime class, used as the
    /// REFIID in the interop call. Empty when auto-resolution failed.
    target_iid: String,
}

/// Recognise an interop method: last two ABI parameters are
/// `(In: REFIID /* Guid* */, Out: Object /* void** */)`, HRESULT return.
///
/// The trailing in-param is treated as a hidden REFIID **only when we're
/// confident it's actually one** — either its metadata type projects to
/// `TypeMeta::Guid` (System.Guid) OR its parameter name (case-insensitive)
/// is exactly `riid` / `iid`. A method whose last in-param is a real
/// application-level Object (a live COM interface pointer) MUST NOT be
/// interpreted as interop-shaped, since dropping that argument would silently
/// break the wrapper. See Fix 3 in the accompanying code-review notes.
fn method_is_interop_shape(m: &MethodMeta) -> Option<Vec<ParamMeta>> {
    // Must return HRESULT
    match &m.return_type {
        Some(t) if is_hresult(t) => {}
        _ => return None,
    }
    // Enforce the exact structural shape in the ORIGINAL parameter order:
    //   [in]... [in REFIID] [out void**]
    // i.e. every param except the last is [in], the last is the sole [out],
    // and the second-to-last [in] is the REFIID. Filtering into direction
    // buckets would have lost this ordering and could misclassify methods
    // where the [out] param appears mid-signature or where the REFIID is
    // not at the tail of the in-list.
    if m.params.len() < 2 {
        return None;
    }
    let last_idx = m.params.len() - 1;
    let out_param = &m.params[last_idx];
    if out_param.direction != ParamDirection::Out {
        return None;
    }
    if !matches!(out_param.typ, TypeMeta::Object) {
        return None;
    }
    // All preceding params must be [in].
    for p in &m.params[..last_idx] {
        if p.direction != ParamDirection::In {
            return None;
        }
    }
    // The last of those [in] params is the REFIID.
    let riid = &m.params[last_idx - 1];
    let is_riid = match &riid.typ {
        TypeMeta::Guid => true,
        TypeMeta::Object => {
            let n = riid.name.to_ascii_lowercase();
            n == "riid" || n == "iid"
        }
        _ => false,
    };
    if !is_riid {
        return None;
    }
    // Natural params: every [in] EXCEPT the trailing REFIID, preserving
    // original order.
    let natural: Vec<ParamMeta> = m.params[..last_idx - 1].iter().cloned().collect();
    Some(natural)
}

/// Best-effort detection: an interface qualifies as an "interop" iff
/// (a) its name ends with `"Interop"`, and
/// (b) at least ONE method matches the interop shape.
///
/// Any interop-shape methods get natural signatures (hide riid + void**);
/// the rest fall back to the normal classic-COM emission.
///
/// Returns:
/// - `Ok(None)` — not an interop interface.
/// - `Ok(Some(info))` — an interop interface with a resolved target IID.
/// - `Err(msg)` — an interop interface was detected but the projected WinRT
///   runtime class's default IID could not be resolved from either the
///   passed winmds or the newest installed Windows SDK. This is a hard
///   failure by design: silently emitting a NULL riid would produce a
///   generated wrapper that fails only at runtime, on a machine the
///   developer may not have.
fn detect_interop(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<Option<InteropInfo>, String> {
    let iface = &meta.interface;
    if !iface.name.ends_with("Interop") {
        return Ok(None);
    }
    if iface.methods.is_empty() {
        return Ok(None);
    }
    let mut methods = Vec::with_capacity(iface.methods.len());
    let mut has_interop_method = false;
    for m in &iface.methods {
        match method_is_interop_shape(m) {
            Some(natural) => {
                has_interop_method = true;
                methods.push(InteropMethod {
                    name: m.name.clone(),
                    camel: camel_case(&m.name),
                    vtable_index: m.vtable_index,
                    natural_params: Some(natural),
                    plain: None,
                    _doc: m.doc.clone(),
                });
            }
            None => {
                methods.push(InteropMethod {
                    name: m.name.clone(),
                    camel: camel_case(&m.name),
                    vtable_index: m.vtable_index,
                    natural_params: None,
                    plain: Some(m.clone()),
                    _doc: m.doc.clone(),
                });
            }
        }
    }
    if !has_interop_method {
        return Ok(None);
    }

    // Derive the WinRT runtime-class simple name from the interop name:
    // strip leading `I` and trailing `Interop`.
    let stripped_i = iface.name.strip_prefix('I').unwrap_or(&iface.name);
    let class_name = stripped_i
        .strip_suffix("Interop")
        .unwrap_or(stripped_i)
        .to_string();

    // Auto-resolve the projected class's default interface IID. Try the winmds
    // the generator was actually given FIRST (portable — respects an integrator
    // who pinned a specific SDK via --ref); if that fails, discover the newest
    // installed Windows SDK winmd. If BOTH fail, we cannot generate a working
    // interop wrapper — fail loudly rather than emit a NULL riid.
    let (class_namespace, target_iid) = match resolve_projected_default_iid(
        winmd_paths,
        &class_name,
    ) {
        Some((ns, _iface_name, iid)) => (ns, iid),
        None => {
            return Err(format!(
                "Classic-COM interop generator: cannot resolve default IID for the projected \
                 WinRT runtime class `{cls}` (derived from `{iface}`). \
                 Neither the winmds passed to the generator ({paths:?}) nor the newest installed \
                 `C:\\Program Files (x86)\\Windows Kits\\10\\UnionMetadata\\<version>\\Windows.winmd` \
                 contains a WinRT runtime class of that name with a resolvable default interface. \
                 Pass the correct Windows.winmd via --ref or install a recent Windows SDK.",
                cls = class_name,
                iface = iface.name,
                paths = winmd_paths,
            ));
        }
    };

    Ok(Some(InteropInfo {
        methods,
        class_name,
        class_namespace,
        target_iid,
    }))
}

/// Auto-resolve the target class + IID for interop projection.
///
/// Consults, in order:
///   1. The winmd paths currently loaded by the generator (`winmd_paths`).
///   2. The NEWEST installed `Windows Kits\10\UnionMetadata\<version>\Windows.winmd`
///      (dynamically discovered — NOT pinned to a specific SDK version).
///
/// Returns `None` when the class cannot be found in either source.
fn resolve_projected_default_iid(
    winmd_paths: &str,
    simple_class_name: &str,
) -> Option<(String, String, String)> {
    // First: try the winmds the generator was given. When integrators pass
    // pinned Windows metadata via --ref/--ref-list this preserves reproducibility.
    if !winmd_paths.is_empty() {
        if let Some(result) =
            crate::meta::find_runtime_class_default_iid(winmd_paths, simple_class_name)
        {
            return Some(result);
        }
    }
    // Fallback: newest installed SDK. This makes the generator portable across
    // machines that have any recent SDK installed, not just `10.0.26100.0`.
    let sdk_winmd = crate::meta::discover_newest_windows_winmd()?;
    // Avoid re-loading if the SDK path was already among the passed winmds.
    if winmd_paths
        .split(';')
        .any(|p| p.eq_ignore_ascii_case(&sdk_winmd))
    {
        return None;
    }
    crate::meta::find_runtime_class_default_iid(&sdk_winmd, simple_class_name)
}


// ---------------------------------------------------------------------------
// .js rendering
// ---------------------------------------------------------------------------

fn render_js(meta: &ComInterfaceMeta, interop: Option<&InteropInfo>) -> String {
    let iface = &meta.interface;
    let iid = &iface.iid;
    let name = &iface.name;

    let mut out = String::new();
    out.push_str("// Generated by dynwinrt-codegen — do not edit\n");

    // Imports (runtime + any referenced enums)
    out.push_str(&format!(
        "import {{ DynWinRtType, DynWinRtMethodSig, DynWinRtValue, WinGuid }} from '{}';\n",
        crate::codegen::project::get_import_name()
    ));
    for en in enum_import_names(meta) {
        out.push_str(&format!("import {{ {} }} from './{}.js';\n", en, en));
    }
    // Interop: import the projected class so we can wrap the returned object.
    if let Some(info) = interop {
        if !info.target_iid.is_empty() {
            out.push_str(&format!(
                "import {{ {cls} }} from './{cls}.js';\n",
                cls = info.class_name
            ));
        }
    }
    out.push('\n');

    out.push_str(&format!(
        "export const IID_{name} = WinGuid.parse('{iid}');\n",
        name = name,
        iid = iid
    ));
    // For interop wrappers with a resolved target IID, also emit the target
    // interface IID as a private constant used by the getForWindow call.
    if let Some(info) = interop {
        if !info.target_iid.is_empty() {
            out.push_str(&format!(
                "const IID_{cls}_default = WinGuid.parse('{iid}');\n",
                cls = info.class_name,
                iid = info.target_iid,
            ));
        }
    }
    out.push('\n');

    // Interface registration (lazy). Base-aware: IUnknown-rooted uses
    // registerInterfaceUnknown (first user slot = 3); IInspectable-rooted
    // uses registerInterface (first user slot = 6).
    let register_fn = if meta.is_iunknown_rooted {
        "registerInterfaceUnknown"
    } else {
        "registerInterface"
    };
    let cache_var = format!("_{name}Cache", name = name);
    let iface_var = format!("_{name}", name = name);
    out.push_str(&format!("let {cache_var};\n", cache_var = cache_var));
    out.push_str(&format!(
        "const {iface_var} = new Proxy({{}}, {{\n    get(_target, prop) {{\n        {cache_var} ??= DynWinRtType.{register_fn}('{name}', IID_{name})\n",
        iface_var = iface_var,
        cache_var = cache_var,
        name = name,
        register_fn = register_fn,
    ));
    for m in &iface.methods {
        out.push_str(&format!(
            "            .addMethod('{}', {})\n",
            m.name,
            build_method_sig_js(m)
        ));
    }
    // Trim trailing newline before closing the block, then close.
    if out.ends_with('\n') {
        out.truncate(out.len() - 1);
    }
    out.push_str(";\n");
    out.push_str(&format!(
        "        const value = {cache_var}[prop];\n        return typeof value === 'function' ? value.bind({cache_var}) : value;\n    }},\n}});\n",
        cache_var = cache_var,
    ));
    out.push('\n');

    // Class body
    out.push_str(&format!("export class {name} {{\n", name = name));
    out.push_str("    _obj;\n");
    out.push_str("    constructor(obj) { this._obj = obj; }\n");
    out.push_str(&format!(
        "    static _fromNative(obj) {{ return new {name}(obj); }}\n",
        name = name
    ));

    if let Some(ref clsid) = meta.coclass_clsid {
        // static create() — classic COM CLSID-based activation.
        out.push_str(&format!(
            "    /** Create a new `{name}` via `CoCreateInstance` on `CLSID_{cc}`. */\n",
            name = name,
            cc = meta.coclass_name.as_deref().unwrap_or("Coclass")
        ));
        out.push_str(&format!(
            "    static create() {{\n        const _obj = DynWinRtValue.coCreateInstance('{clsid}', IID_{name});\n        return new {name}(_obj);\n    }}\n",
            clsid = clsid,
            name = name,
        ));
    } else if let Some(info) = interop {
        if !info.class_namespace.is_empty() {
            // static create() — interop activation: activate the projected
            // WinRT runtime class's factory, then QI to the interop IID.
            let full_class_name = format!("{}.{}", info.class_namespace, info.class_name);
            out.push_str(&format!(
                "    /** Create a new `{name}` by activating the `{full_class_name}` factory and QI'ing to the interop. */\n",
                name = name,
                full_class_name = full_class_name,
            ));
            out.push_str(&format!(
                "    static create() {{\n        const factory = DynWinRtValue.activationFactory('{full_class_name}');\n        const _obj = factory.cast(IID_{name});\n        return new {name}(_obj);\n    }}\n",
                full_class_name = full_class_name,
                name = name,
            ));
        }
    }

    // Emit methods: natural interop shape when available, otherwise pass-through.
    if let Some(info) = interop {
        for im in &info.methods {
            emit_interop_method_js(&mut out, im, &iface_var, info);
        }
    } else {
        for m in &iface.methods {
            emit_method_js(&mut out, m, &iface_var);
        }
    }
    out.push_str("}\n");
    out
}

fn build_method_sig_js(m: &MethodMeta) -> String {
    let mut parts = Vec::new();
    for p in &m.params {
        if p.direction == ParamDirection::In {
            parts.push(format!(".addIn({})", ts_type_expr_js(&p.typ)));
        } else if p.direction == ParamDirection::Out {
            parts.push(format!(".addOut({})", ts_type_expr_js(&p.typ)));
        } else if p.direction == ParamDirection::OutFill {
            parts.push(format!(".addOutFill({})", ts_type_expr_js(&p.typ)));
        }
    }
    // Return type of a classic-COM HRESULT method is NOT part of the sig —
    // the runtime swallows HRESULT and throws on failure. Only non-HRESULT
    // returns are recorded (rare; e.g. IClassFactory::CreateInstance uses
    // HRESULT, so most Win32 methods land here).
    if let Some(ref rt) = m.return_type {
        if !is_hresult(rt) {
            parts.push(format!(".addOut({})", ts_type_expr_js(rt)));
        }
    }
    if parts.is_empty() {
        "new DynWinRtMethodSig()".to_string()
    } else {
        format!("new DynWinRtMethodSig(){}", parts.join(""))
    }
}

fn emit_method_js(out: &mut String, m: &MethodMeta, iface_var: &str) {
    let camel = camel_case(&m.name);
    let in_params: Vec<&ParamMeta> = m
        .params
        .iter()
        .filter(|p| p.direction == ParamDirection::In)
        .collect();
    let out_params: Vec<&ParamMeta> = m
        .params
        .iter()
        .filter(|p| p.direction == ParamDirection::Out)
        .collect();
    let has_outfill = m
        .params
        .iter()
        .any(|p| p.direction == ParamDirection::OutFill);

    let param_list: Vec<String> = in_params
        .iter()
        .enumerate()
        .map(|(i, p)| js_param_name(&p.name, i))
        .collect();

    let args_exprs: Vec<String> = in_params
        .iter()
        .enumerate()
        .map(|(i, p)| wrap_arg_js(&p.typ, &js_param_name(&p.name, i)))
        .collect();

    out.push_str(&format!(
        "    {camel}({params}) {{\n",
        camel = camel,
        params = param_list.join(", ")
    ));
    // Project trailing `[out]` params as JS return values, mirroring how the
    // WinRT codegen already handles out-params (see
    // `codegen/javascript/project/methods.rs` — `is_multi_output` / `invokeAll`).
    // OutFill (caller-allocated buffers, e.g. GetPath(LPWSTR, cchMax)) are
    // NOT projected — see the TODO note below.
    if has_outfill {
        out.push_str("        // TODO: caller-allocated [out, sizeis] buffers are not yet projected as returns.\n");
    }
    match out_params.len() {
        0 => {
            out.push_str(&format!(
                "        {iface_var}.method({slot}).invoke(this._obj, [{args}]);\n",
                iface_var = iface_var,
                slot = m.vtable_index,
                args = args_exprs.join(", ")
            ));
        }
        1 => {
            out.push_str(&format!(
                "        const _out = {iface_var}.method({slot}).invoke(this._obj, [{args}]);\n",
                iface_var = iface_var,
                slot = m.vtable_index,
                args = args_exprs.join(", ")
            ));
            out.push_str(&format!(
                "        return {};\n",
                unwrap_return_js(&out_params[0].typ, "_out")
            ));
        }
        _ => {
            out.push_str(&format!(
                "        const _r = {iface_var}.method({slot}).invokeAll(this._obj, [{args}]);\n",
                iface_var = iface_var,
                slot = m.vtable_index,
                args = args_exprs.join(", ")
            ));
            let items: Vec<String> = out_params
                .iter()
                .enumerate()
                .map(|(i, p)| unwrap_return_js(&p.typ, &format!("_r[{i}]")))
                .collect();
            out.push_str(&format!("        return [{}];\n", items.join(", ")));
        }
    }
    out.push_str("    }\n");
}

/// Unwrap the `DynWinRtValue` result of a method invocation into a natural JS
/// value, according to the `[out]` param's declared type. Mirrors the WinRT
/// codegen's `convert_return` for the primitive/GUID/enum/handle cases;
/// Object/Interface/RuntimeClass currently return the raw `DynWinRtValue`
/// (caller can `.cast(IID)` to bridge to another wrapper).
fn unwrap_return_js(t: &TypeMeta, expr: &str) -> String {
    if is_win32_bool(t) {
        // Win32 BOOL marshals as i32 at the ABI; project as JS boolean.
        return format!("({expr}.toNumber() !== 0)");
    }
    if handle_type_name(t).is_some() {
        // Opaque Win32 handle (HWND, PWSTR, etc.) → raw pointer as bigint.
        // Use `asPointerBigint` (not `toI64`): the runtime may return the
        // handle as a `WinRTValue::Object`/`RawPtr`/`Null` when the handle's
        // inner `Value` field is a `void*`-shaped type, and `toI64` panics
        // on those variants (it falls back to `toNumber`, which explicitly
        // panics for non-numeric WinRTValues). `asPointerBigint` cleanly
        // handles Object/RawPtr/Null and preserves all 64 pointer bits.
        return format!("{expr}.asPointerBigint()");
    }
    match t {
        TypeMeta::Bool => format!("{expr}.toBool()"),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::Char16 => format!("{expr}.toNumber()"),
        TypeMeta::I64 | TypeMeta::U64 => format!("{expr}.toI64()"),
        TypeMeta::F32 | TypeMeta::F64 => format!("{expr}.toF64()"),
        TypeMeta::Guid => format!("{expr}.toGuid().toString()"),
        TypeMeta::Enum { underlying, .. } => unwrap_return_js(underlying, expr),
        TypeMeta::String => format!("{expr}.toString()"),
        // Object / Interface / RuntimeClass / Struct pointer / etc.
        // Return the raw DynWinRtValue and let the caller decide (e.g. cast).
        _ => expr.to_string(),
    }
}

/// `.d.ts` return-type text for a classic-COM plain method whose HRESULT is
/// swallowed. Projects `[out]` params as the natural return type: 0 outs →
/// `void`, 1 out → that type, N outs → a tuple.
fn dts_return_type_for_outs(m: &MethodMeta) -> String {
    let out_params: Vec<&ParamMeta> = m
        .params
        .iter()
        .filter(|p| p.direction == ParamDirection::Out)
        .collect();
    match out_params.len() {
        0 => "void".to_string(),
        1 => ts_type_expr_dts(&out_params[0].typ),
        _ => {
            let items: Vec<String> = out_params
                .iter()
                .map(|p| ts_type_expr_dts(&p.typ))
                .collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Emit an interop method: either natural (hide trailing REFIID + void**) or
/// plain (fall back to the normal classic-COM emission).
fn emit_interop_method_js(out: &mut String, im: &InteropMethod, iface_var: &str, info: &InteropInfo) {
    let Some(natural_params) = &im.natural_params else {
        // Plain method — reuse the existing pass-through emission.
        if let Some(m) = &im.plain {
            emit_method_js(out, m, iface_var);
        }
        return;
    };
    let param_list: Vec<String> = natural_params
        .iter()
        .enumerate()
        .map(|(i, p)| js_param_name(&p.name, i))
        .collect();

    let mut arg_exprs: Vec<String> = natural_params
        .iter()
        .enumerate()
        .map(|(i, p)| wrap_arg_js(&p.typ, &js_param_name(&p.name, i)))
        .collect();

    // The synthesised REFIID pointer. When we have a resolved target IID we
    // pass the cached pointer; otherwise the method is unusable (still emitted
    // for completeness so `.d.ts` doesn't lie about the surface).
    let riid_arg = if !info.target_iid.is_empty() {
        format!("DynWinRtValue.iidPointer(IID_{}_default)", info.class_name)
    } else {
        "DynWinRtValue.pointer(0n)".to_string()
    };
    arg_exprs.push(riid_arg);

    out.push_str(&format!(
        "    {camel}({params}) {{\n",
        camel = im.camel,
        params = param_list.join(", "),
    ));
    if !info.target_iid.is_empty() {
        out.push_str(&format!(
            "        const _out = {iface_var}.method({slot}).invoke(this._obj, [{args}]);\n",
            iface_var = iface_var,
            slot = im.vtable_index,
            args = arg_exprs.join(", "),
        ));
        out.push_str(&format!(
            "        return {cls}._fromNative(_out);\n",
            cls = info.class_name,
        ));
    } else {
        // Fallback: no projection available. Return the raw object.
        out.push_str(&format!(
            "        return {iface_var}.method({slot}).invoke(this._obj, [{args}]);\n",
            iface_var = iface_var,
            slot = im.vtable_index,
            args = arg_exprs.join(", "),
        ));
    }
    out.push_str("    }\n");
}


// ---------------------------------------------------------------------------
// .d.ts rendering
// ---------------------------------------------------------------------------

fn render_dts(meta: &ComInterfaceMeta, interop: Option<&InteropInfo>) -> String {
    let iface = &meta.interface;
    let name = &iface.name;

    let mut out = String::new();
    out.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    // Import Buffer type hint via `import type` from Node ambient — HWND uses `Buffer`.
    // Node.js `Buffer` is a global type; no import needed. We DO import enum types.
    for en in enum_import_names(meta) {
        out.push_str(&format!("import {{ {} }} from './{}.js';\n", en, en));
    }
    // Interop: import the projected class declaration so return types resolve.
    if let Some(info) = interop {
        if !info.target_iid.is_empty() {
            out.push_str(&format!(
                "import {{ {cls} }} from './{cls}.js';\n",
                cls = info.class_name
            ));
        }
    }
    out.push('\n');

    // Emit typedef aliases for handles seen in method parameters.
    let handle_aliases = collect_handle_aliases(meta);
    for h in &handle_aliases {
        out.push_str(&format!(
            "/** Opaque Win32 handle or pointer newtype (e.g. HWND, PWSTR). Accepts either a raw pointer as `bigint` or a `Buffer`. */\nexport type {h} = bigint | Buffer;\n",
            h = h
        ));
    }
    if !handle_aliases.is_empty() {
        out.push('\n');
    }

    out.push_str(&format!("export declare const IID_{name}: unknown;\n\n", name = name));

    out.push_str(&format!("export declare class {name} {{\n", name = name));
    if meta.coclass_clsid.is_some() {
        out.push_str("    /** Create a new instance via the coclass activation path. */\n");
        out.push_str(&format!("    static create(): {name};\n", name = name));
    } else if let Some(info) = interop {
        if !info.class_namespace.is_empty() {
            out.push_str(&format!(
                "    /** Activate the projected WinRT class and QI to the interop. */\n    static create(): {name};\n",
                name = name
            ));
        }
    }
    out.push_str(&format!(
        "    /** Wrap an existing native COM pointer (for QueryInterface bridging). */\n    static _fromNative(obj: unknown): {name};\n",
        name = name
    ));

    if let Some(info) = interop {
        // Interop methods: NATURAL signatures for interop-shape methods (no
        // riid, no void**). Plain methods fall through to the normal
        // classic-COM emission.
        for im in &info.methods {
            match (&im.natural_params, &im.plain) {
                (Some(natural), _) => {
                    let ts_params: Vec<String> = natural
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            format!("{}: {}", js_param_name(&p.name, i), ts_type_expr_dts(&p.typ))
                        })
                        .collect();
                    let ret = if !info.target_iid.is_empty() {
                        info.class_name.clone()
                    } else {
                        "unknown".to_string()
                    };
                    out.push_str(&format!(
                        "    {camel}({params}): {ret};\n",
                        camel = im.camel,
                        params = ts_params.join(", "),
                        ret = ret,
                    ));
                }
                (None, Some(m)) => {
                    let camel = camel_case(&m.name);
                    let in_params: Vec<&ParamMeta> = m
                        .params
                        .iter()
                        .filter(|p| p.direction == ParamDirection::In)
                        .collect();
                    let ts_params: Vec<String> = in_params
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            format!("{}: {}", js_param_name(&p.name, i), ts_type_expr_dts(&p.typ))
                        })
                        .collect();
                    let ret = match &m.return_type {
                        None => "void".to_string(),
                        // HRESULT is swallowed by the runtime (throw on failure).
                        // Project `[out]` params as the natural return instead.
                        Some(t) if is_hresult(t) => dts_return_type_for_outs(m),
                        Some(t) => ts_type_expr_dts(t),
                    };
                    out.push_str(&format!(
                        "    {camel}({params}): {ret};\n",
                        camel = camel,
                        params = ts_params.join(", "),
                        ret = ret,
                    ));
                }
                _ => {}
            }
        }
    } else {
        for m in &iface.methods {
            let camel = camel_case(&m.name);
            let in_params: Vec<&ParamMeta> = m
                .params
                .iter()
                .filter(|p| p.direction == ParamDirection::In)
                .collect();
            let ts_params: Vec<String> = in_params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!("{}: {}", js_param_name(&p.name, i), ts_type_expr_dts(&p.typ))
                })
                .collect();
            let ret = match &m.return_type {
                None => "void".to_string(),
                // HRESULT is swallowed by the runtime (throw on failure).
                // Project `[out]` params as the natural return instead.
                Some(t) if is_hresult(t) => dts_return_type_for_outs(m),
                Some(t) => ts_type_expr_dts(t),
            };
            out.push_str(&format!(
                "    {camel}({params}): {ret};\n",
                camel = camel,
                params = ts_params.join(", "),
                ret = ret,
            ));
        }
    }
    out.push_str("}\n");
    out
}

/// Emit the companion `<ClassName>.js` + `.d.ts` for the projected WinRT
/// runtime class. Provides:
/// - a `static getForWindow(hwnd)` that opens the interop and calls it,
///   returning a natural `<ClassName>` wrapper;
/// - an internal constructor that stores the live COM object;
/// - a `runtimeClassName` getter (via IInspectable::GetRuntimeClassName) —
///   the E2E's proof that the returned object is a live WinRT instance.
fn render_projected_class_files(
    meta: &ComInterfaceMeta,
    info: &InteropInfo,
) -> Option<(String, String)> {
    if info.target_iid.is_empty() || info.class_namespace.is_empty() {
        return None;
    }
    // The interop wrapper file is named after the interface (e.g.
    // `IDataTransferManagerInterop.js`). We import from it.
    let interop_module = &meta.interface.name;
    let full_class_name = format!("{}.{}", info.class_namespace, info.class_name);

    // Pick the primary interop method to expose as the `static getForWindow`.
    // Prefer one whose PascalCase name equals "GetForWindow"; otherwise take
    // the first interop-shape method.
    let primary = info
        .methods
        .iter()
        .find(|im| im.name == "GetForWindow" && im.natural_params.is_some())
        .or_else(|| info.methods.iter().find(|im| im.natural_params.is_some()))?;
    let primary_natural = primary.natural_params.as_ref()?;

    // The IInspectable IID is a fixed WinRT constant.
    const IID_IINSPECTABLE: &str = "af86e2e0-b12d-4c6a-9c5a-d7aa65101e90";

    // -- .js --
    let mut js = String::new();
    js.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    js.push_str(&format!(
        "import {{ DynWinRtType, DynWinRtMethodSig, WinGuid }} from '{}';\n",
        crate::codegen::project::get_import_name()
    ));
    js.push_str(&format!(
        "import {{ {interop} }} from './{interop}.js';\n",
        interop = interop_module,
    ));
    js.push('\n');

    js.push_str(&format!(
        "const IID_IInspectable = WinGuid.parse('{iid}');\n\n",
        iid = IID_IINSPECTABLE
    ));
    // IInspectable registration (lazy) — used to reach GetRuntimeClassName.
    // IInspectable is the base itself; its methods live at absolute vtable
    // slots 3, 4, 5 (right after IUnknown). Register with the +3 base so that
    // `.method(4)` resolves to `GetRuntimeClassName` at the real absolute slot.
    js.push_str("let _IInspectableCache;\n");
    js.push_str("const _IInspectable = new Proxy({}, {\n    get(_target, prop) {\n");
    js.push_str("        _IInspectableCache ??= DynWinRtType.registerInterfaceUnknown('IInspectable_projected', IID_IInspectable)\n");
    js.push_str("            .addMethod('GetIids', new DynWinRtMethodSig().addOut(DynWinRtType.pointer()).addOut(DynWinRtType.pointer()))\n");
    js.push_str("            .addMethod('GetRuntimeClassName', new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))\n");
    js.push_str("            .addMethod('GetTrustLevel', new DynWinRtMethodSig().addOut(DynWinRtType.i32Type()));\n");
    js.push_str("        const value = _IInspectableCache[prop];\n");
    js.push_str("        return typeof value === 'function' ? value.bind(_IInspectableCache) : value;\n");
    js.push_str("    },\n});\n\n");

    js.push_str(&format!("export class {cls} {{\n", cls = info.class_name));
    js.push_str("    _obj;\n");
    js.push_str("    constructor(obj) { this._obj = obj; }\n");
    js.push_str(&format!(
        "    static _fromNative(obj) {{ return new {cls}(obj); }}\n",
        cls = info.class_name,
    ));

    // Static getForWindow(hwnd) — the high-level natural surface.
    let param_list: Vec<String> = primary_natural
        .iter()
        .enumerate()
        .map(|(i, p)| js_param_name(&p.name, i))
        .collect();
    js.push_str(&format!(
        "    /** Get a `{cls}` for the given HWND via the {interop} interop. */\n",
        cls = info.class_name,
        interop = interop_module,
    ));
    js.push_str(&format!(
        "    static {camel}({params}) {{\n",
        camel = primary.camel,
        params = param_list.join(", "),
    ));
    js.push_str(&format!(
        "        const interop = {interop}.create();\n",
        interop = interop_module,
    ));
    // Call interop.<camelMethod>(...naturalArgs) — this returns a
    // `<ClassName>` already wrapped via `_fromNative`.
    js.push_str(&format!(
        "        return interop.{camel}({params});\n",
        camel = primary.camel,
        params = param_list.join(", "),
    ));
    js.push_str("    }\n");

    // runtimeClassName getter — IInspectable slot 4 (absolute vtable index).
    js.push_str("    /** IInspectable::GetRuntimeClassName — the projected class name. */\n");
    js.push_str("    get runtimeClassName() {\n");
    js.push_str("        return _IInspectable.method(4).getString(this._obj);\n");
    js.push_str("    }\n");

    js.push_str("}\n");

    // -- .d.ts --
    let mut dts = String::new();
    dts.push_str("// Generated by dynwinrt-codegen — do not edit\n\n");
    // Handle typedef for HWND (needed for the static getForWindow signature).
    let handle_aliases = collect_handle_aliases(meta);
    for h in &handle_aliases {
        dts.push_str(&format!(
            "/** Opaque Win32 handle or pointer newtype (e.g. HWND, PWSTR). Accepts either a raw pointer as `bigint` or a `Buffer`. */\nexport type {h} = bigint | Buffer;\n",
            h = h
        ));
    }
    if !handle_aliases.is_empty() {
        dts.push('\n');
    }
    dts.push_str(&format!("export declare class {cls} {{\n", cls = info.class_name));
    dts.push_str(&format!(
        "    /** Wrap an existing native COM pointer (for QueryInterface bridging). */\n    static _fromNative(obj: unknown): {cls};\n",
        cls = info.class_name,
    ));
    let ts_params: Vec<String> = primary_natural
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}: {}", js_param_name(&p.name, i), ts_type_expr_dts(&p.typ)))
        .collect();
    dts.push_str(&format!(
        "    /** Get a `{cls}` for the given HWND (projected from `{full_class_name}`). */\n",
        cls = info.class_name,
        full_class_name = full_class_name,
    ));
    dts.push_str(&format!(
        "    static {camel}({params}): {cls};\n",
        camel = primary.camel,
        params = ts_params.join(", "),
        cls = info.class_name,
    ));
    dts.push_str("    /** IInspectable::GetRuntimeClassName — the projected class name. */\n");
    dts.push_str("    get runtimeClassName(): string;\n");
    dts.push_str("}\n");

    Some((js, dts))
}


fn collect_handle_aliases(meta: &ComInterfaceMeta) -> Vec<String> {
    let mut set = BTreeSet::new();
    for m in &meta.interface.methods {
        for p in &m.params {
            if let Some(h) = handle_type_name(&p.typ) {
                set.insert(h);
            }
        }
    }
    set.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Enum sibling files
// ---------------------------------------------------------------------------

fn render_enum_files(en: &TypeMeta) -> (String, String) {
    let (name, members) = match en {
        TypeMeta::Enum { name, members, .. } => (name.as_str(), members),
        _ => unreachable!(),
    };

    // .js: a frozen object.
    let mut js = String::new();
    js.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    js.push_str(&format!("export const {name} = Object.freeze({{\n", name = name));
    for m in members {
        js.push_str(&format!("    {}: {},\n", m.name, m.value));
    }
    js.push_str("});\n");

    // .d.ts: emit a const object + companion type — matches the JS `Object.freeze({...})`
    // runtime shape and mirrors the WinRT enum generator (see
    // `codegen::javascript::render::declarations::render_enum_dts`). Using `const enum`
    // breaks under TS `isolatedModules`, so we intentionally avoid it.
    let mut dts = String::new();
    dts.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    dts.push_str(&format!(
        "export type {name} = (typeof {name})[keyof typeof {name}];\n",
        name = name
    ));
    dts.push_str(&format!("export declare const {name}: {{\n", name = name));
    for m in members {
        dts.push_str(&format!("    readonly {}: {};\n", m.name, m.value));
    }
    dts.push_str("};\n");

    (js, dts)
}

fn enum_import_names(meta: &ComInterfaceMeta) -> Vec<String> {
    meta.referenced_enums
        .iter()
        .filter_map(|e| match e {
            TypeMeta::Enum { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Type mapping helpers
// ---------------------------------------------------------------------------

/// TS type expression for the `.d.ts` surface.
fn ts_type_expr_dts(t: &TypeMeta) -> String {
    // Win32 BOOL is a struct with a single `Value: I32` field — the same shape
    // as an opaque handle. Special-case it to the natural boolean surface so
    // callers can just pass `true`/`false` rather than a bigint.
    if is_win32_bool(t) {
        return "boolean".into();
    }
    if let Some(h) = handle_type_name(t) {
        return h;
    }
    match t {
        TypeMeta::Bool => "boolean".into(),
        TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::I32 | TypeMeta::U32
            | TypeMeta::F32 | TypeMeta::F64 | TypeMeta::Char16 => "number".into(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint".into(),
        TypeMeta::String => "string".into(),
        TypeMeta::Guid => "string".into(),
        TypeMeta::Enum { name, .. } => name.clone(),
        TypeMeta::Struct { name, .. } => name.clone(),
        // Pointer-to-struct or unknown — opaque bigint|Buffer at the surface.
        _ => "bigint | Buffer".into(),
    }
}

/// Runtime type expression for `DynWinRtMethodSig` calls in `.js`.
fn ts_type_expr_js(t: &TypeMeta) -> String {
    // Win32 BOOL marshals as a 32-bit int at the ABI (Win32 BOOL is `int`),
    // NOT an opaque pointer. Mirrors how enums map to their underlying i32.
    if is_win32_bool(t) {
        return "DynWinRtType.i32Type()".into();
    }
    if handle_type_name(t).is_some() {
        return "DynWinRtType.pointer()".into();
    }
    match t {
        TypeMeta::Bool => "DynWinRtType.boolType()".into(),
        TypeMeta::I8 => "DynWinRtType.i8Type()".into(),
        TypeMeta::U8 => "DynWinRtType.u8Type()".into(),
        TypeMeta::I16 => "DynWinRtType.i16Type()".into(),
        TypeMeta::U16 => "DynWinRtType.u16Type()".into(),
        TypeMeta::I32 => "DynWinRtType.i32Type()".into(),
        TypeMeta::U32 => "DynWinRtType.u32Type()".into(),
        TypeMeta::I64 => "DynWinRtType.i64Type()".into(),
        TypeMeta::U64 => "DynWinRtType.u64Type()".into(),
        TypeMeta::F32 => "DynWinRtType.f32Type()".into(),
        TypeMeta::F64 => "DynWinRtType.f64Type()".into(),
        TypeMeta::Char16 => "DynWinRtType.char16()".into(),
        TypeMeta::String => "DynWinRtType.pointer()".into(), // PCWSTR/PWSTR → opaque
        TypeMeta::Guid => "DynWinRtType.guidType()".into(),
        TypeMeta::Enum { underlying, .. } => ts_type_expr_js(underlying),
        _ => "DynWinRtType.pointer()".into(),
    }
}

fn wrap_arg_js(t: &TypeMeta, var: &str) -> String {
    // Win32 BOOL: accept `boolean`/`number`/`bigint` on the surface and
    // narrow to an i32 (0/1) at the ABI. Truthy → 1, falsy → 0. Non-nullish
    // numerics are preserved so callers passing `1`/`0` still work.
    if is_win32_bool(t) {
        return format!("DynWinRtValue.i32({var} ? 1 : 0)", var = var);
    }
    if handle_type_name(t).is_some() {
        return format!("DynWinRtValue.pointer({var})", var = var);
    }
    match t {
        TypeMeta::Bool => format!("DynWinRtValue.boolValue({var})", var = var),
        TypeMeta::I8 => format!("DynWinRtValue.i8Value({var})", var = var),
        TypeMeta::U8 => format!("DynWinRtValue.u8Value({var})", var = var),
        TypeMeta::I16 => format!("DynWinRtValue.i16Value({var})", var = var),
        TypeMeta::U16 => format!("DynWinRtValue.u16Value({var})", var = var),
        TypeMeta::I32 => format!("DynWinRtValue.i32({var})", var = var),
        TypeMeta::U32 => format!("DynWinRtValue.u32({var})", var = var),
        TypeMeta::I64 => format!("DynWinRtValue.i64(BigInt({var}))", var = var),
        TypeMeta::U64 => format!("DynWinRtValue.u64(BigInt({var}))", var = var),
        TypeMeta::F32 => format!("DynWinRtValue.f32({var})", var = var),
        TypeMeta::F64 => format!("DynWinRtValue.f64({var})", var = var),
        TypeMeta::Char16 => format!("DynWinRtValue.char16({var})", var = var),
        TypeMeta::String => format!("DynWinRtValue.pointer({var})", var = var),
        TypeMeta::Guid => format!("DynWinRtValue.guid(WinGuid.parse({var}))", var = var),
        TypeMeta::Enum { underlying, .. } => wrap_arg_js(underlying, var),
        _ => format!("DynWinRtValue.pointer({var})", var = var),
    }
}

/// Returns `Some("HWND")` etc. when the given type is a Win32 opaque handle
/// (a struct in `Windows.Win32.Foundation` or similar handle-namespace with a
/// single pointer-shaped `Value` field). Also returns handle names for
/// PWSTR/PCWSTR/HRESULT-family types encountered as parameters (except
/// HRESULT itself which is treated as `void`).
fn handle_type_name(t: &TypeMeta) -> Option<String> {
    // BOOL is NOT a handle even though it shape-matches (`{ Value: I32 }`).
    // The natural surface is `boolean` (see `is_win32_bool`).
    if is_win32_bool(t) {
        return None;
    }
    match t {
        TypeMeta::Struct { namespace, name, fields } => {
            if !is_win32_handle_namespace(namespace) {
                return None;
            }
            if is_hresult_by_name(namespace, name) {
                return None; // HRESULT is not a "handle" — never surface it as one
            }
            // Handle heuristic: exactly one field named `Value`, of pointer/int type.
            if fields.len() == 1
                && fields[0].name == "Value"
                && matches!(
                    fields[0].typ,
                    TypeMeta::Object
                        | TypeMeta::U64
                        | TypeMeta::I64
                        | TypeMeta::U32
                        | TypeMeta::I32
                )
            {
                return Some(name.clone());
            }
            None
        }
        _ => None,
    }
}

fn is_win32_handle_namespace(ns: &str) -> bool {
    ns.starts_with("Windows.Win32.")
}

fn is_hresult(t: &TypeMeta) -> bool {
    matches!(
        t,
        TypeMeta::Struct { namespace, name, .. }
            if is_hresult_by_name(namespace, name)
    )
}

fn is_hresult_by_name(ns: &str, name: &str) -> bool {
    ns == "Windows.Win32.Foundation" && name == "HRESULT"
}

/// Recognise the Win32 `BOOL` struct (`Windows.Win32.Foundation.BOOL`) — a
/// `{ Value: I32 }` struct whose natural surface is a JS `boolean` but whose
/// ABI is a 32-bit int. Kept as a distinct helper so the surface remains
/// obvious and greppable.
fn is_win32_bool(t: &TypeMeta) -> bool {
    matches!(
        t,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && name == "BOOL"
    )
}

// ---------------------------------------------------------------------------
// Naming helpers
// ---------------------------------------------------------------------------

fn camel_case(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    // Count the leading uppercase run.
    let mut run = 0usize;
    while run < chars.len() && chars[run].is_ascii_uppercase() {
        run += 1;
    }
    let mut result = String::with_capacity(name.len());
    if run == 0 {
        // Already starts lowercase — return unchanged.
        return name.to_string();
    }
    if run == chars.len() {
        // Fully uppercase (e.g. "URL") — lowercase everything.
        for c in &chars {
            result.push(c.to_ascii_lowercase());
        }
        return result;
    }
    if run == 1 {
        // Simple case: lowercase first char, keep the rest.
        result.push(chars[0].to_ascii_lowercase());
        for c in &chars[1..] {
            result.push(*c);
        }
        return result;
    }
    // Multi-char uppercase run followed by lowercase: last uppercase char is
    // the start of the next word. E.g. "IOHandle" -> "ioHandle".
    for c in &chars[..run - 1] {
        result.push(c.to_ascii_lowercase());
    }
    for c in &chars[run - 1..] {
        result.push(*c);
    }
    result
}

fn js_param_name(raw: &str, index: usize) -> String {
    let base = if raw.is_empty() {
        format!("arg{}", index)
    } else {
        raw.to_string()
    };
    // Camelize (strip common Hungarian prefixes lightly for prettier surface):
    // dwFoo -> foo, pFoo -> foo, lpszFoo -> foo, cbFoo -> foo, iFoo -> foo, hFoo -> foo, hwndFoo -> foo.
    let stripped = strip_hungarian(&base);
    let mut out = String::with_capacity(stripped.len());
    let mut chars = stripped.chars();
    if let Some(first) = chars.next() {
        out.push(first.to_ascii_lowercase());
    }
    for c in chars {
        out.push(c);
    }
    // Guard against JS reserved words.
    match out.as_str() {
        "class" | "return" | "function" | "default" | "this" | "new" | "delete"
        | "let" | "const" | "var" | "if" | "else" | "for" | "while" | "do" | "switch"
        | "case" | "break" | "continue" | "true" | "false" | "null" | "undefined"
        | "in" | "of" | "typeof" | "instanceof" | "throw" | "try" | "catch" | "finally"
        | "yield" | "async" | "await" | "with" | "void" | "public" | "private" | "protected"
        | "package" | "static" | "import" | "export" | "extends" | "super" | "arguments" => {
            format!("{}_", out)
        }
        _ => out,
    }
}

fn strip_hungarian(s: &str) -> &str {
    // Only strip common **multi-character** Hungarian prefixes, and only when
    // followed by an uppercase letter (word boundary). Single-letter prefixes
    // like `h`, `p`, `i` cause too many false positives on real method-param
    // names (e.g. `hwnd` starts with `h` but isn't Hungarian; `pButton` is).
    let prefixes = [
        "lpwsz", "pwsz", "lpsz", "psz", "lpsz", "pwstr", "pcwstr",
        "hwnd", "dw", "sz", "cb", "cx", "cy", "cw", "ch", "cn", "cc",
        "lp", "np", "ph", "pd", "pf", "pv", "ppv", "pp", "wsz",
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
// Unit tests (fast, no winmd — pure logic)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_basic() {
        assert_eq!(camel_case("HrInit"), "hrInit");
        assert_eq!(camel_case("SetProgressValue"), "setProgressValue");
        assert_eq!(camel_case("AddTab"), "addTab");
        assert_eq!(camel_case("URL"), "url");
        assert_eq!(camel_case("IOHandle"), "ioHandle");
    }

    #[test]
    fn strip_hungarian_only_at_word_boundary() {
        assert_eq!(strip_hungarian("dwReserved"), "Reserved");
        assert_eq!(strip_hungarian("hwndTab"), "Tab");
        // "hwnd" alone must NOT be stripped (no uppercase follow-up).
        assert_eq!(strip_hungarian("hwnd"), "hwnd");
    }

    #[test]
    fn handle_type_name_recognizes_hwnd_shape() {
        let hwnd = TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "HWND".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::Object,
            }],
        };
        assert_eq!(handle_type_name(&hwnd).as_deref(), Some("HWND"));
    }

    #[test]
    fn hresult_is_not_a_handle() {
        let hr = TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "HRESULT".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        };
        assert!(handle_type_name(&hr).is_none());
        assert!(is_hresult(&hr));
    }

    #[test]
    fn non_win32_struct_is_not_a_handle() {
        let rect = TypeMeta::Struct {
            namespace: "Windows.Foundation".into(),
            name: "Rect".into(),
            fields: vec![
                crate::types::FieldMeta { name: "X".into(), typ: TypeMeta::F32 },
                crate::types::FieldMeta { name: "Y".into(), typ: TypeMeta::F32 },
                crate::types::FieldMeta { name: "Width".into(), typ: TypeMeta::F32 },
                crate::types::FieldMeta { name: "Height".into(), typ: TypeMeta::F32 },
            ],
        };
        assert!(handle_type_name(&rect).is_none());
    }

    // ---- Fix 2 (BOOL → boolean/i32) ----

    fn win32_bool_struct() -> TypeMeta {
        TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "BOOL".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        }
    }

    #[test]
    fn win32_bool_is_not_a_handle() {
        let b = win32_bool_struct();
        // Sanity: it's the exact shape of a handle (single Value: I32) — the
        // special-case must WIN over the generic handle heuristic.
        assert!(handle_type_name(&b).is_none(),
            "BOOL must not be emitted as an opaque handle typedef");
    }

    #[test]
    fn win32_bool_projects_as_boolean_and_i32() {
        let b = win32_bool_struct();
        // .d.ts surface: boolean (not `BOOL` or `bigint | Buffer`)
        assert_eq!(ts_type_expr_dts(&b), "boolean");
        // .js registration: i32 type (not pointer)
        assert_eq!(ts_type_expr_js(&b), "DynWinRtType.i32Type()");
        // .js argument marshalling: truthy→1, falsy→0 as an i32 (not pointer)
        assert_eq!(
            wrap_arg_js(&b, "fFullscreen"),
            "DynWinRtValue.i32(fFullscreen ? 1 : 0)"
        );
    }

    // ---- Fix 3 (REFIID-guarded interop heuristic) ----

    /// Helper: construct a MethodMeta with HRESULT return type.
    fn make_hresult() -> TypeMeta {
        TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "HRESULT".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::I32,
            }],
        }
    }

    #[test]
    fn interop_shape_accepts_riid_named_object_trailing_in() {
        // Real Windows.Win32 shape: `HRESULT GetForWindow(HWND appWindow, REFIID riid, out void** ppv)`.
        // REFIID typically projects to TypeMeta::Object with name "riid".
        let m = MethodMeta {
            name: "GetForWindow".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "appWindow".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "riid".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "ppv".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let natural = method_is_interop_shape(&m).expect(
            "REFIID-shaped trailing in-param named `riid` must be recognised as interop",
        );
        // Natural in-params = every in EXCEPT the trailing REFIID.
        assert_eq!(natural.len(), 1);
        assert_eq!(natural[0].name, "appWindow");
    }

    #[test]
    fn interop_shape_accepts_guid_typed_trailing_in() {
        // Some winmds project REFIID as TypeMeta::Guid rather than Object.
        let m = MethodMeta {
            name: "GetSomething".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "target".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    // Deliberately NOT named "riid" — the type alone is sufficient.
                    name: "interfaceId".into(),
                    typ: TypeMeta::Guid,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "out".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let natural = method_is_interop_shape(&m)
            .expect("System.Guid-typed trailing in-param must be recognised as interop");
        assert_eq!(natural.len(), 1);
        assert_eq!(natural[0].name, "target");
    }

    /// FIX 3 REGRESSION: a method returning HRESULT with an [out] Object and a
    /// trailing In-Object whose name is NOT `riid`/`iid` (e.g. a real application
    /// COM interface pointer like `original`) must NOT be mis-classified as
    /// interop-shape. Otherwise the codegen would silently drop the caller's
    /// meaningful argument.
    #[test]
    fn interop_shape_rejects_non_refiid_trailing_object() {
        let m = MethodMeta {
            name: "CloneWithOriginal".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "context".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    // NOT `riid`/`iid`, NOT Guid — a real COM pointer in-param.
                    name: "original".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "cloned".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        assert!(
            method_is_interop_shape(&m).is_none(),
            "trailing in-param `original` is a real Object argument, NOT a REFIID — \
             it must not be dropped by the interop heuristic"
        );
    }

    #[test]
    fn interop_shape_rejects_iid_named_non_object_param() {
        // A parameter named `riid` but typed as a plain I32 is not a REFIID —
        // reject rather than silently drop.
        let m = MethodMeta {
            name: "Weird".into(),
            vtable_index: 3,
            params: vec![
                ParamMeta {
                    name: "hwnd".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "riid".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "out".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        assert!(
            method_is_interop_shape(&m).is_none(),
            "an I32 named `riid` is not a REFIID — must be rejected"
        );
    }

    // ---- Fix 1 (winmd-derived interop IID, fail-loud on unresolved) ----

    /// Build a fully synthetic ComInterfaceMeta for an `IFooInterop`-style
    /// interface whose derived projected class name (`Foo`) does NOT exist
    /// anywhere reachable. The generator must FAIL LOUDLY rather than emit
    /// a NULL riid.
    #[test]
    fn interop_generation_fails_when_target_iid_unresolvable() {
        use crate::meta::{ComInterfaceMeta, InterfaceMeta};

        let iface = InterfaceMeta {
            name: "IThisRuntimeClassDoesNotExist_DynWinrtInterop".into(),
            namespace: "Windows.Win32.System.WinRT".into(),
            iid: "00000000-0000-0000-0000-000000000000".into(),
            methods: vec![MethodMeta {
                name: "GetForWindow".into(),
                vtable_index: 3,
                params: vec![
                    ParamMeta {
                        name: "appWindow".into(),
                        typ: TypeMeta::Object,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "riid".into(),
                        typ: TypeMeta::Object,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "ppv".into(),
                        typ: TypeMeta::Object,
                        direction: ParamDirection::Out,
                    },
                ],
                return_type: Some(make_hresult()),
                ..Default::default()
            }],
            generic_piid: None,
            generic_args: Vec::new(),
            doc: None,
            deprecated: None,
        };
        let com = ComInterfaceMeta {
            interface: iface,
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: vec!["IUnknown".into()],
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
        };
        // Pass empty winmd_paths — even with the newest-SDK fallback, the
        // synthetic class name won't be found anywhere.
        let result = generate_com_interface_files(&com, "");
        assert!(
            result.is_err(),
            "generator must fail loudly when the projected runtime-class IID \
             cannot be resolved; got Ok(_)"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("ThisRuntimeClassDoesNotExist_Dynwinrt")
                || err.contains("ThisRuntimeClassDoesNotExist_DynWinrt"),
            "error must name the class it failed to resolve: {}",
            err
        );
        assert!(
            !err.is_empty(),
            "error message must be non-empty (fail-loud contract)"
        );
    }

    #[test]
    fn non_interop_iunknown_interface_still_generates_without_winmd_lookup() {
        // A vanilla IUnknown-rooted interface with no coclass and no
        // interop shape must succeed even when we pass empty winmd paths.
        use crate::meta::{ComInterfaceMeta, InterfaceMeta};
        let iface = InterfaceMeta {
            name: "IMyPlainClassicCom".into(),
            namespace: "Windows.Win32.System.Com".into(),
            iid: "11111111-2222-3333-4444-555555555555".into(),
            methods: vec![MethodMeta {
                name: "DoStuff".into(),
                vtable_index: 3,
                params: vec![],
                return_type: Some(make_hresult()),
                ..Default::default()
            }],
            generic_piid: None,
            generic_args: Vec::new(),
            doc: None,
            deprecated: None,
        };
        let com = ComInterfaceMeta {
            interface: iface,
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: vec!["IUnknown".into()],
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
        };
        let out = generate_com_interface_files(&com, "")
            .expect("plain classic-COM codegen must succeed with no winmds");
        assert!(out.js.contains("registerInterfaceUnknown"));
        assert!(out.js.contains("method(3)"));
    }

    // ---- Fix 4 (classic-COM plain `[out]` param → return-value projection) ----

    fn plain_iface_with_method(m: MethodMeta) -> crate::meta::ComInterfaceMeta {
        use crate::meta::{ComInterfaceMeta, InterfaceMeta};
        let iface = InterfaceMeta {
            name: "IHasOut".into(),
            namespace: "Windows.Win32.System.Com".into(),
            iid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            methods: vec![m],
            generic_piid: None,
            generic_args: Vec::new(),
            doc: None,
            deprecated: None,
        };
        ComInterfaceMeta {
            interface: iface,
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: vec!["IUnknown".into()],
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
        }
    }

    #[test]
    fn plain_method_single_out_scalar_projects_as_return() {
        // Model: `HRESULT GetShowCmd([out] int* pcmd)` — the classic single-out
        // int shape. The out-int must become the method's return value.
        let m = MethodMeta {
            name: "GetShowCmd".into(),
            vtable_index: 8,
            params: vec![ParamMeta {
                name: "pcmd".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::Out,
            }],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        // .js: must capture `_out` and return it as a JS number.
        assert!(
            js.contains("const _out = _IHasOut.method(8).invoke(this._obj, [])"),
            ".js must capture invoke() result into _out:\n{}",
            js
        );
        assert!(
            js.contains("return _out.toNumber();"),
            ".js must unwrap the I32 out as _out.toNumber():\n{}",
            js
        );
        // .d.ts: return type must be `number`, not `void`.
        assert!(
            dts.contains("getShowCmd(): number;"),
            ".d.ts must project single-out I32 as `number`:\n{}",
            dts
        );
    }

    #[test]
    fn plain_method_single_out_guid_projects_as_string() {
        // Model: `HRESULT GetClassID([out] GUID* pClassID)` (IPersist shape).
        let m = MethodMeta {
            name: "GetClassID".into(),
            vtable_index: 3,
            params: vec![ParamMeta {
                name: "pClassID".into(),
                typ: TypeMeta::Guid,
                direction: ParamDirection::Out,
            }],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        assert!(
            js.contains("const _out = _IHasOut.method(3).invoke(this._obj, [])"),
            ".js must capture invoke() result into _out:\n{}",
            js
        );
        assert!(
            js.contains("return _out.toGuid().toString();"),
            ".js must unwrap GUID out via .toGuid().toString():\n{}",
            js
        );
        assert!(
            dts.contains("getClassID(): string;"),
            ".d.ts must project single-out GUID as `string`:\n{}",
            dts
        );
    }

    #[test]
    fn plain_method_single_out_enum_projects_as_underlying() {
        // Model: `HRESULT GetKind([out] MyKind* pk)` where MyKind is an I32
        // enum. Underlying-scalar unwrap → `.toNumber()`; .d.ts uses the enum
        // type name.
        let m = MethodMeta {
            name: "GetKind".into(),
            vtable_index: 5,
            params: vec![ParamMeta {
                name: "pk".into(),
                typ: TypeMeta::Enum {
                    namespace: "Windows.Win32.System.Com".into(),
                    name: "MyKind".into(),
                    underlying: Box::new(TypeMeta::I32),
                    members: Vec::new(),
                    doc: None,
                    deprecated: None,
                },
                direction: ParamDirection::Out,
            }],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        assert!(
            js.contains("return _out.toNumber();"),
            ".js must unwrap enum out via underlying scalar (.toNumber()):\n{}",
            js
        );
        assert!(
            dts.contains("getKind(): MyKind;"),
            ".d.ts must project enum out under the enum's declared name:\n{}",
            dts
        );
    }

    #[test]
    fn plain_method_multi_out_uses_invoke_all_and_tuple_return() {
        // Model: `HRESULT Q([out] uint32_t* a, [out] BOOL* found)` — two
        // trailing out params must flip to `.invokeAll()` and a tuple return.
        let m = MethodMeta {
            name: "Q".into(),
            vtable_index: 6,
            params: vec![
                ParamMeta {
                    name: "a".into(),
                    typ: TypeMeta::U32,
                    direction: ParamDirection::Out,
                },
                ParamMeta {
                    name: "found".into(),
                    typ: TypeMeta::Bool,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        assert!(
            js.contains("const _r = _IHasOut.method(6).invokeAll(this._obj, [])"),
            ".js multi-out must use .invokeAll():\n{}",
            js
        );
        assert!(
            js.contains("return [_r[0].toNumber(), _r[1].toBool()];"),
            ".js multi-out must return a tuple with each out unwrapped:\n{}",
            js
        );
        assert!(
            dts.contains("q(): [number, boolean];"),
            ".d.ts multi-out must project a tuple type:\n{}",
            dts
        );
    }

    #[test]
    fn plain_method_zero_out_still_discards_result() {
        // No out params: existing behavior — invoke and discard.
        let m = MethodMeta {
            name: "DoIt".into(),
            vtable_index: 4,
            params: vec![ParamMeta {
                name: "arg".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            }],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        assert!(
            !js.contains("const _out ="),
            ".js zero-out must not capture invoke() result:\n{}",
            js
        );
        assert!(
            !js.contains("invokeAll"),
            ".js zero-out must not use .invokeAll():\n{}",
            js
        );
        assert!(
            js.contains("_IHasOut.method(4).invoke(this._obj,"),
            ".js zero-out must call plain .invoke():\n{}",
            js
        );
        assert!(
            dts.contains("doIt(arg: number): void;"),
            ".d.ts zero-out must still be `void`:\n{}",
            dts
        );
    }

    #[test]
    fn plain_method_outfill_stays_void_with_todo() {
        // Caller-allocated `[out, sizeis]` buffers are NOT yet projected —
        // emit a `TODO` comment and keep the surface as `void` so we don't
        // half-break anything.
        let m = MethodMeta {
            name: "GetPath".into(),
            vtable_index: 2,
            params: vec![
                ParamMeta {
                    name: "pszFile".into(),
                    typ: TypeMeta::String, // PWSTR buffer, caller-allocated
                    direction: ParamDirection::OutFill,
                },
                ParamMeta {
                    name: "cch".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                },
            ],
            return_type: Some(make_hresult()),
            ..Default::default()
        };
        let com = plain_iface_with_method(m);
        let js = render_js(&com, None);
        let dts = render_dts(&com, None);
        assert!(
            js.contains("TODO: caller-allocated [out, sizeis] buffers"),
            ".js OutFill must include a TODO comment:\n{}",
            js
        );
        assert!(
            !js.contains("return _out") && !js.contains("return _r") && !js.contains("return [") ,
            ".js OutFill must not return anything (avoid half-broken projection):\n{}",
            js
        );
        assert!(
            dts.contains("getPath(cch: number): void;"),
            ".d.ts OutFill must stay `void`:\n{}",
            dts
        );
    }
}
