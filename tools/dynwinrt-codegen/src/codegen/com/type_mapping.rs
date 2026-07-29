// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;

use crate::com_metadata::{
    ComInterfaceMeta, MethodMeta, ParamDirection, ParamMeta, is_native_isize, is_native_usize,
};
use crate::types::TypeMeta;

use super::naming::js_param_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StringEncoding {
    Wide,
    Ansi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HandleAliasKind {
    HandleValue,
    DataPointer,
    StringPointer,
}

pub(super) fn validate_com_abi(meta: &ComInterfaceMeta) -> Result<(), String> {
    for method in &meta.interface.methods {
        for param in &method.params {
            if let ParamDirection::UnsupportedNativeArray { count_param_index } = param.direction {
                let count = count_param_index
                    .map(|index| format!("parameter index {index}"))
                    .unwrap_or_else(|| "metadata-defined size".into());
                return Err(format!(
                    "{}.{}: caller-sized native buffers are not supported (`{}` uses {count})",
                    meta.interface.name, method.name, param.name
                ));
            }
            if matches!(param.typ, TypeMeta::Struct { .. })
                && !is_win32_bool(&param.typ)
                && !is_hresult(&param.typ)
                && !is_native_isize(&param.typ)
                && !is_native_usize(&param.typ)
                && handle_type_name(&param.typ).is_none()
            {
                return Err(format!(
                    "{}.{}: struct parameter `{}` requires native layout projection",
                    meta.interface.name, method.name, param.name
                ));
            }
            if param.direction == ParamDirection::OutFill {
                return Err(format!(
                    "{}.{}: caller-allocated array outputs are not supported",
                    meta.interface.name, method.name
                ));
            }
            if param.direction == ParamDirection::InOut && !supports_in_out(&param.typ) {
                return Err(format!(
                    "{}.{}: unsupported [in, out] parameter `{}` of type {:?}",
                    meta.interface.name, method.name, param.name, param.typ
                ));
            }
        }
        if let Some(return_type) = method
            .return_type
            .as_ref()
            .filter(|return_type| !is_hresult(return_type))
        {
            if !supports_direct_return(return_type) {
                return Err(format!(
                    "{}.{}: unsupported direct native return type {:?}",
                    meta.interface.name, method.name, return_type
                ));
            }
        }
    }
    Ok(())
}

fn supports_in_out(t: &TypeMeta) -> bool {
    is_native_isize(t)
        || is_native_usize(t)
        || is_win32_bool(t)
        || is_hresult(t)
        || handle_type_name(t).is_some()
        || matches!(
            t,
            TypeMeta::Bool
                | TypeMeta::I8
                | TypeMeta::U8
                | TypeMeta::I16
                | TypeMeta::U16
                | TypeMeta::I32
                | TypeMeta::U32
                | TypeMeta::I64
                | TypeMeta::U64
                | TypeMeta::F32
                | TypeMeta::F64
                | TypeMeta::Char16
                | TypeMeta::Enum { .. }
        )
}

fn supports_direct_return(t: &TypeMeta) -> bool {
    supports_in_out(t)
}

pub(super) fn unwrap_return_js(t: &TypeMeta, expr: &str) -> String {
    if is_native_isize(t) {
        return format!("DynCom.toIsizeBigint({expr})");
    }
    if is_native_usize(t) {
        return format!("DynCom.toUsizeBigint({expr})");
    }
    match string_buffer_encoding(t) {
        Some(StringEncoding::Wide) => {
            return format!("DynCom.takeCoTaskMemWideString({expr})");
        }
        Some(StringEncoding::Ansi) => {
            return format!("DynCom.takeCoTaskMemAnsiString({expr})");
        }
        None => {}
    }
    if is_win32_bool(t) {
        return format!("(DynCom.toNumber({expr}) !== 0)");
    }
    if handle_type_name(t).is_some() {
        return format!("DynCom.asPointerBigint({expr})");
    }
    match t {
        TypeMeta::Bool => format!("DynCom.toBool({expr})"),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::Char16 => format!("DynCom.toNumber({expr})"),
        TypeMeta::U32 => format!("DynCom.toU32({expr})"),
        TypeMeta::I64 => format!("DynCom.toI64Bigint({expr})"),
        TypeMeta::U64 => format!("DynCom.toU64Bigint({expr})"),
        TypeMeta::F32 | TypeMeta::F64 => format!("DynCom.toF64({expr})"),
        TypeMeta::Guid => format!("DynCom.toGuidString({expr})"),
        TypeMeta::Enum { underlying, .. } => unwrap_return_js(underlying, expr),
        TypeMeta::String => format!("{expr}.toString()"),
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => expr.to_string(),
        _ => expr.to_string(),
    }
}

#[derive(Clone, Copy)]
pub(super) struct MethodResult<'a> {
    pub(super) typ: &'a TypeMeta,
    pub(super) param_index: Option<usize>,
}

pub(super) fn method_results(m: &MethodMeta) -> Vec<MethodResult<'_>> {
    let mut result = Vec::new();
    if let Some(typ) = m.return_type.as_ref().filter(|typ| !is_hresult(typ)) {
        result.push(MethodResult {
            typ,
            param_index: None,
        });
    }
    result.extend(
        m.params
            .iter()
            .enumerate()
            .filter(|(_, param)| {
                matches!(param.direction, ParamDirection::Out | ParamDirection::InOut)
            })
            .map(|(param_index, param)| MethodResult {
                typ: &param.typ,
                param_index: Some(param_index),
            }),
    );
    result
}

pub(super) fn dts_return_type(m: &MethodMeta) -> String {
    if string_buffer_pattern(m).is_some() {
        let outputs = method_results(m);
        if outputs.is_empty() {
            return "string".to_string();
        }
        return format!(
            "[string, {}]",
            outputs
                .iter()
                .map(|result| ts_result_type(m, *result))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let result_types = method_results(m);
    match result_types.len() {
        0 => "void".to_string(),
        1 => ts_result_type(m, result_types[0]),
        _ => format!(
            "[{}]",
            result_types
                .iter()
                .map(|result| ts_result_type(m, *result))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn ts_result_type(method: &MethodMeta, result: MethodResult<'_>) -> String {
    if is_sys_free_string_owned(method, result) || string_buffer_encoding(result.typ).is_some() {
        "string".into()
    } else if is_cotaskmem_owned(method, result) {
        "DynWinRtValue".into()
    } else {
        ts_type_expr_dts(result.typ)
    }
}

pub(super) fn is_cotaskmem_owned(method: &MethodMeta, result: MethodResult<'_>) -> bool {
    let Some(param_index) = result.param_index else {
        return false;
    };
    method
        .owned_outputs
        .iter()
        .any(|owned| owned.param_index == param_index && owned.free_with.contains("CoTaskMemFree"))
}

pub(super) fn is_sys_free_string_owned(method: &MethodMeta, result: MethodResult<'_>) -> bool {
    let Some(param_index) = result.param_index else {
        return false;
    };
    is_bstr(result.typ)
        && method.owned_outputs.iter().any(|owned| {
            owned.param_index == param_index && owned.free_with.contains("SysFreeString")
        })
}

pub(super) fn dts_params_for_method(m: &MethodMeta) -> Vec<String> {
    let string_buffer = string_buffer_pattern(m);
    m.params
        .iter()
        .enumerate()
        .filter(|(_, param)| param.direction.is_input())
        .enumerate()
        .map(|(surface_index, (param_index, param))| {
            let mut name = js_param_name(&param.name, surface_index);
            if let Some((_, count_index, _)) = string_buffer {
                if param_index >= count_index && string_buffer_param_is_optional(m, param_index) {
                    name.push('?');
                }
            }
            format!("{}: {}", name, ts_input_type_expr_dts(&param.typ))
        })
        .collect()
}

pub(super) fn collect_handle_aliases(meta: &ComInterfaceMeta) -> Vec<(String, HandleAliasKind)> {
    let mut aliases = BTreeMap::new();
    for method in &meta.interface.methods {
        for param in &method.params {
            if let Some((alias, kind)) = handle_alias(&param.typ) {
                aliases.insert(alias, kind);
            }
        }
        if let Some((alias, kind)) = method.return_type.as_ref().and_then(handle_alias) {
            aliases.insert(alias, kind);
        }
    }
    aliases.into_iter().collect()
}

pub(super) fn enum_import_names(meta: &ComInterfaceMeta) -> Vec<String> {
    meta.referenced_enums
        .iter()
        .map(|enum_meta| enum_meta.name.clone())
        .collect()
}

pub(super) fn uses_winrt_bridge_value(meta: &ComInterfaceMeta) -> bool {
    for method in &meta.interface.methods {
        for typ in method
            .params
            .iter()
            .map(|param| &param.typ)
            .chain(method.return_type.iter())
        {
            if let TypeMeta::Interface { iid, .. } = typ {
                if !iid.is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

pub(super) fn has_string_buffer_method(meta: &ComInterfaceMeta) -> bool {
    meta.interface
        .methods
        .iter()
        .any(|method| string_buffer_pattern(method).is_some())
}

pub(super) fn string_buffer_pattern(method: &MethodMeta) -> Option<(usize, usize, StringEncoding)> {
    for (index, param) in method.params.iter().enumerate() {
        let ParamDirection::OutStringBuffer { count_param_index } = param.direction else {
            continue;
        };
        let encoding = string_buffer_encoding(&param.typ)?;
        if method
            .params
            .get(count_param_index)
            .is_some_and(|count| count.direction == ParamDirection::In)
        {
            return Some((index, count_param_index, encoding));
        }
    }
    None
}

pub(super) fn string_buffer_param_is_optional(method: &MethodMeta, param_index: usize) -> bool {
    let Some((_, count_index, _)) = string_buffer_pattern(method) else {
        return false;
    };
    let Some(param) = method.params.get(param_index) else {
        return false;
    };
    let is_optional_shape = param_index == count_index
        || (param_index > count_index && is_optional_find_data_out_after_string_count(param));
    if !is_optional_shape {
        return false;
    }
    method
        .params
        .iter()
        .enumerate()
        .skip(param_index + 1)
        .filter(|(_, param)| param.direction.is_input())
        .all(|(_, param)| is_optional_find_data_out_after_string_count(param))
}

fn string_buffer_encoding(t: &TypeMeta) -> Option<StringEncoding> {
    match t {
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Win32.Foundation" && name == "PWSTR" => {
            Some(StringEncoding::Wide)
        }
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Win32.Foundation" && name == "PSTR" => {
            Some(StringEncoding::Ansi)
        }
        _ => None,
    }
}

pub(super) fn is_optional_find_data_out_after_string_count(param: &ParamMeta) -> bool {
    if !matches!(param.direction, ParamDirection::In | ParamDirection::Out) {
        return false;
    }
    let name = param.name.to_ascii_lowercase();
    if name == "pfd" || name.contains("finddata") || name.contains("find_data") {
        return true;
    }
    matches!(
        &param.typ,
        TypeMeta::Struct { name, .. } if name == "WIN32_FIND_DATAW" || name == "WIN32_FIND_DATAA"
    )
}

pub(super) fn ts_type_expr_dts(t: &TypeMeta) -> String {
    if is_native_isize(t) || is_native_usize(t) {
        return "bigint".into();
    }
    if is_win32_bool(t) {
        return "boolean".into();
    }
    if is_hresult(t) {
        return "number".into();
    }
    if let Some(handle) = handle_type_name(t) {
        return handle;
    }
    match t {
        TypeMeta::Bool => "boolean".into(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::F32
        | TypeMeta::F64
        | TypeMeta::Char16 => "number".into(),
        TypeMeta::I64 | TypeMeta::U64 => "bigint".into(),
        TypeMeta::String => "string".into(),
        TypeMeta::Guid => "string".into(),
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => "DynWinRtValue".into(),
        TypeMeta::Enum { name, .. } | TypeMeta::Struct { name, .. } => name.clone(),
        _ => "bigint | Buffer".into(),
    }
}

pub(super) fn ts_input_type_expr_dts(t: &TypeMeta) -> String {
    if accepts_handle_value_buffer(t)
        || matches!(handle_alias(t), Some((_, HandleAliasKind::DataPointer)))
    {
        return format!("{} | Buffer | Uint8Array", handle_type_name(t).unwrap());
    }
    ts_type_expr_dts(t)
}

pub(super) fn ts_type_expr_js(t: &TypeMeta) -> String {
    if is_native_isize(t) {
        return "DynCom.isizeType()".into();
    }
    if is_native_usize(t) {
        return "DynCom.usizeType()".into();
    }
    if is_win32_bool(t) || is_hresult(t) {
        return "DynCom.i32Type()".into();
    }
    if handle_type_name(t).is_some() {
        return "DynCom.pointerType()".into();
    }
    if let TypeMeta::Interface { iid, .. } = t {
        if !iid.is_empty() {
            return format!("DynCom.interfaceType(WinGuid.parse('{iid}'))");
        }
    }
    match t {
        TypeMeta::Bool => "DynCom.boolType()".into(),
        TypeMeta::I8 => "DynCom.i8Type()".into(),
        TypeMeta::U8 => "DynCom.u8Type()".into(),
        TypeMeta::I16 => "DynCom.i16Type()".into(),
        TypeMeta::U16 => "DynCom.u16Type()".into(),
        TypeMeta::I32 => "DynCom.i32Type()".into(),
        TypeMeta::U32 => "DynCom.u32Type()".into(),
        TypeMeta::I64 => "DynCom.i64Type()".into(),
        TypeMeta::U64 => "DynCom.u64Type()".into(),
        TypeMeta::F32 => "DynCom.f32Type()".into(),
        TypeMeta::F64 => "DynCom.f64Type()".into(),
        TypeMeta::Char16 => "DynCom.char16Type()".into(),
        TypeMeta::Guid => "DynCom.guidType()".into(),
        TypeMeta::Enum { underlying, .. } => ts_type_expr_js(underlying),
        _ => "DynCom.pointerType()".into(),
    }
}

pub(super) fn wrap_arg_js(t: &TypeMeta, var: &str) -> String {
    if is_native_isize(t) {
        return format!("DynCom.isize(BigInt({var}))");
    }
    if is_native_usize(t) {
        return format!("DynCom.usize(BigInt({var}))");
    }
    if is_win32_bool(t) {
        return format!("DynCom.i32({var} ? 1 : 0)");
    }
    if is_hresult(t) {
        return format!("DynCom.i32({var})");
    }
    if let Some((_, kind)) = handle_alias(t) {
        return match kind {
            HandleAliasKind::HandleValue if accepts_handle_value_buffer(t) => {
                format!("DynCom.pointer(DynCom.handleValue({var}))")
            }
            HandleAliasKind::HandleValue => format!("DynCom.pointer({var})"),
            HandleAliasKind::DataPointer | HandleAliasKind::StringPointer => {
                format!("DynCom.pointer({var})")
            }
        };
    }
    if let TypeMeta::Interface { iid, .. } = t {
        if !iid.is_empty() {
            return var.to_string();
        }
    }
    match t {
        TypeMeta::Bool => format!("DynCom.boolValue({var})"),
        TypeMeta::I8 => format!("DynCom.i8Value({var})"),
        TypeMeta::U8 => format!("DynCom.u8Value({var})"),
        TypeMeta::I16 => format!("DynCom.i16({var})"),
        TypeMeta::U16 => format!("DynCom.u16({var})"),
        TypeMeta::I32 => format!("DynCom.i32({var})"),
        TypeMeta::U32 => format!("DynCom.u32({var})"),
        TypeMeta::I64 => format!("DynCom.i64(BigInt({var}))"),
        TypeMeta::U64 => format!("DynCom.u64(BigInt({var}))"),
        TypeMeta::F32 => format!("DynCom.f32({var})"),
        TypeMeta::F64 => format!("DynCom.f64({var})"),
        TypeMeta::Char16 => format!("DynCom.char16({var})"),
        TypeMeta::Guid => format!("DynCom.guid(WinGuid.parse({var}))"),
        TypeMeta::Enum { underlying, .. } => wrap_arg_js(underlying, var),
        _ => format!("DynCom.pointer({var})"),
    }
}

pub(super) fn handle_type_name(t: &TypeMeta) -> Option<String> {
    handle_alias(t).map(|(name, _)| name)
}

#[cfg(test)]
pub(super) fn handle_alias_kind(t: &TypeMeta) -> Option<HandleAliasKind> {
    handle_alias(t).map(|(_, kind)| kind)
}

fn handle_alias(t: &TypeMeta) -> Option<(String, HandleAliasKind)> {
    if is_win32_bool(t) {
        return None;
    }
    match t {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } if is_win32_handle_namespace(namespace)
            && !is_hresult_by_name(namespace, name)
            && fields.len() == 1
            && fields[0].name == "Value"
            && matches!(
                fields[0].typ,
                TypeMeta::Object | TypeMeta::U64 | TypeMeta::I64 | TypeMeta::U32 | TypeMeta::I32
            ) =>
        {
            Some((name.clone(), classify_handle_alias(namespace, name)))
        }
        _ => None,
    }
}

fn classify_handle_alias(_namespace: &str, name: &str) -> HandleAliasKind {
    if is_string_pointer_alias_name(name) {
        HandleAliasKind::StringPointer
    } else if is_data_pointer_alias_name(name) {
        HandleAliasKind::DataPointer
    } else {
        HandleAliasKind::HandleValue
    }
}

fn accepts_handle_value_buffer(t: &TypeMeta) -> bool {
    matches!(
        handle_alias(t),
        Some((name, HandleAliasKind::HandleValue)) if name == "HWND"
    )
}

fn is_data_pointer_alias_name(name: &str) -> bool {
    matches!(
        name,
        "PSID"
            | "PSECURITY_DESCRIPTOR"
            | "MEMORY_MAPPED_VIEW_ADDRESS"
            | "LPPROC_THREAD_ATTRIBUTE_LIST"
    )
}

fn is_string_pointer_alias_name(name: &str) -> bool {
    // Classic COM handle typedefs lose pointer-pointee detail by the time they
    // reach TypeMeta (`Value: *mut u16` and `Value: *mut c_void` both become
    // `Value: Object`). Keep the known Win32 NUL-terminated character-pointer
    // aliases as Buffer-capable pointer parameters; all other handle-shaped
    // structs are handle values and must not accept Buffer-of-bits inputs.
    matches!(
        name,
        "PWSTR"
            | "PCWSTR"
            | "PSTR"
            | "PCSTR"
            | "LPWSTR"
            | "LPCWSTR"
            | "LPSTR"
            | "LPCSTR"
            | "PWCHAR"
            | "PCWCHAR"
            | "LPWCH"
            | "LPCWCH"
            | "LPCH"
            | "LPCCH"
    )
}

fn is_win32_handle_namespace(namespace: &str) -> bool {
    namespace.starts_with("Windows.Win32.")
}

pub(super) fn is_hresult(t: &TypeMeta) -> bool {
    matches!(
        t,
        TypeMeta::Struct { namespace, name, .. }
            if is_hresult_by_name(namespace, name)
    )
}

fn is_hresult_by_name(namespace: &str, name: &str) -> bool {
    namespace == "Windows.Win32.Foundation" && name == "HRESULT"
}

pub(super) fn is_win32_bool(t: &TypeMeta) -> bool {
    matches!(
        t,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && name == "BOOL"
    )
}

pub(super) fn is_bstr(t: &TypeMeta) -> bool {
    matches!(
        t,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && name == "BSTR"
    )
}
