// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python method signatures, argument wrapping, and return conversion.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::TypeMeta;

use super::naming::{to_snake_case, to_snake_case_filename};
use crate::codegen::shared::imports::ireference_inner_type;

pub(crate) fn py_runtime_symbol(type_name: &str, symbol_name: &str) -> String {
    format!(
        "_dynwinrt_symbol('{}', '{}')",
        to_snake_case_filename(type_name),
        symbol_name
    )
}

// ======================================================================
// Python type expression
// ======================================================================

fn py_interface_iid(typ: &TypeMeta) -> Option<String> {
    match typ {
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => {
            Some(format!("WinGUID.parse('{}')", iid))
        }
        TypeMeta::Parameterized { .. } => Some(format!("{}.iid()", py_dynwinrt_type(typ))),
        _ => None,
    }
}

pub(crate) fn py_interface_iid_expr(iface: &InterfaceMeta) -> Option<String> {
    if let Some(ref piid) = iface.generic_piid {
        let args = iface
            .generic_args
            .iter()
            .map(py_dynwinrt_type)
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "DynWinRTType.parameterized(WinGUID.parse('{}'), [{}]).iid()",
            piid, args
        ))
    } else if !iface.iid.is_empty() {
        Some(format!("WinGUID.parse('{}')", iface.iid))
    } else {
        None
    }
}

/// Map a TypeMeta to a `DynWinRTType.*()` Python expression.
pub(crate) fn py_dynwinrt_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "DynWinRTType.bool_type()".to_string(),
        TypeMeta::I8 => "DynWinRTType.i8_type()".to_string(),
        TypeMeta::I16 => "DynWinRTType.i16_type()".to_string(),
        TypeMeta::Char16 => "DynWinRTType.char16()".to_string(),
        TypeMeta::I32 => "DynWinRTType.i32_type()".to_string(),
        TypeMeta::U8 => "DynWinRTType.u8_type()".to_string(),
        TypeMeta::U16 => "DynWinRTType.u16_type()".to_string(),
        TypeMeta::U32 => "DynWinRTType.u32_type()".to_string(),
        TypeMeta::I64 => "DynWinRTType.i64_type()".to_string(),
        TypeMeta::U64 => "DynWinRTType.u64_type()".to_string(),
        TypeMeta::F32 => "DynWinRTType.f32_type()".to_string(),
        TypeMeta::F64 => "DynWinRTType.f64_type()".to_string(),
        TypeMeta::String => "DynWinRTType.hstring()".to_string(),
        TypeMeta::Guid => "DynWinRTType.guid_type()".to_string(),
        TypeMeta::Object => "DynWinRTType.object()".to_string(),
        TypeMeta::Interface { iid, .. } if !iid.is_empty() => {
            format!("DynWinRTType.interface(WinGUID.parse('{}'))", iid)
        }
        TypeMeta::Interface { .. } => "DynWinRTType.object()".to_string(),
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => {
            let full_name = format!("{}.{}", namespace, name);
            if let Some(default_iid) = default_interface.as_deref().and_then(py_interface_iid) {
                format!(
                    "DynWinRTType.runtime_class('{}', {})",
                    full_name, default_iid
                )
            } else {
                "DynWinRTType.object()".to_string()
            }
        }

        TypeMeta::Delegate { .. } => "DynWinRTType.object()".to_string(),
        TypeMeta::AsyncOperation(inner) => {
            format!(
                "DynWinRTType.i_async_operation({})",
                py_dynwinrt_type(inner)
            )
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            format!(
                "DynWinRTType.i_async_operation_with_progress({}, {})",
                py_dynwinrt_type(result),
                py_dynwinrt_type(progress)
            )
        }
        TypeMeta::AsyncAction => "DynWinRTType.i_async_action()".to_string(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!(
                "DynWinRTType.i_async_action_with_progress({})",
                py_dynwinrt_type(progress)
            )
        }
        TypeMeta::Struct { name, .. } if name == "HResult" => "DynWinRTType.hresult()".to_string(),
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => {
            let full_name = format!("{}.{}", namespace, name);
            let field_types: Vec<String> =
                fields.iter().map(|f| py_dynwinrt_type(&f.typ)).collect();
            format!(
                "DynWinRTType.struct_type('{}', [{}])",
                full_name,
                field_types.join(", ")
            )
        }
        TypeMeta::Array(inner) => {
            format!("DynWinRTType.array_type({})", py_dynwinrt_type(inner))
        }
        TypeMeta::Enum {
            namespace,
            name,
            members,
            ..
        } => {
            let full_name = format!("{}.{}", namespace, name);
            if members.is_empty() {
                format!("DynWinRTType.enum_type('{}')", full_name)
            } else {
                let names: Vec<String> = members.iter().map(|m| format!("'{}'", m.name)).collect();
                let values: Vec<String> = members.iter().map(|m| m.value.to_string()).collect();
                format!(
                    "DynWinRTType.enum_type('{}', [{}], [{}])",
                    full_name,
                    names.join(", "),
                    values.join(", ")
                )
            }
        }
        TypeMeta::Parameterized { piid, args, .. } => {
            if piid.is_empty() {
                "DynWinRTType.object()".to_string()
            } else {
                let arg_types: Vec<String> = args.iter().map(|a| py_dynwinrt_type(a)).collect();
                format!(
                    "DynWinRTType.parameterized(WinGUID.parse('{}'), [{}])",
                    piid,
                    arg_types.join(", ")
                )
            }
        }
    }
}

/// Build a `DynWinRTMethodSig().add_in(...)...` Python expression.
pub(crate) fn py_build_method_sig(method: &MethodMeta) -> String {
    let mut parts = Vec::new();

    for param in &method.params {
        if param.direction == ParamDirection::In {
            parts.push(format!(".add_in({})", py_dynwinrt_type(&param.typ)));
        }
    }
    for param in &method.params {
        if param.direction == ParamDirection::Out {
            parts.push(format!(".add_out({})", py_dynwinrt_type(&param.typ)));
        } else if param.direction == ParamDirection::OutFill {
            parts.push(format!(".add_out_fill({})", py_dynwinrt_type(&param.typ)));
        }
    }
    if let Some(ref return_type) = method.return_type {
        parts.push(format!(".add_out({})", py_dynwinrt_type(return_type)));
    }

    if parts.is_empty() {
        "DynWinRTMethodSig()".to_string()
    } else {
        format!("DynWinRTMethodSig(){}", parts.join(""))
    }
}

/// Wrap a Python variable name into a DynWinRTValue expression.
pub(crate) fn py_wrap_arg(name: &str, typ: &TypeMeta) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        let value_type = py_dynwinrt_type(inner);
        let wrapped = py_wrap_reference_value("value", inner);
        return format!(
            "_dynwinrt_box_reference({}, {}, lambda value: {})",
            name, value_type, wrapped
        );
    }

    match typ {
        TypeMeta::String => format!("DynWinRTValue.from_hstring({})", name),
        TypeMeta::Bool => format!("DynWinRTValue.from_bool({})", name),
        TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::Enum { .. }
        | TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16 => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::I64 | TypeMeta::U64 => format!("DynWinRTValue.from_i64({})", name),
        TypeMeta::F32 => format!("DynWinRTValue.from_f32({})", name),
        TypeMeta::F64 => format!("DynWinRTValue.from_f64({})", name),
        TypeMeta::Guid => format!("DynWinRTValue.from_guid({})", name),
        TypeMeta::RuntimeClass { .. }
        | TypeMeta::Object
        | TypeMeta::Interface { .. }
        | TypeMeta::Parameterized { .. }
        | TypeMeta::Delegate { .. } => {
            format!("getattr({}, '_obj', {})", name, name)
        }
        TypeMeta::Array(_) => format!("{}.to_value()", name),
        TypeMeta::Struct {
            name: struct_name, ..
        } if struct_name == "HResult" => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::Struct {
            name: struct_name, ..
        } => {
            format!("_pack_{}({}).to_value()", to_snake_case(struct_name), name)
        }
        _ => name.to_string(),
    }
}

fn py_wrap_reference_value(name: &str, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => format!("DynWinRTValue.from_bool({})", name),
        TypeMeta::I8 => format!("DynWinRTValue.from_i8({})", name),
        TypeMeta::U8 => format!("DynWinRTValue.from_u8({})", name),
        TypeMeta::I16 => format!("DynWinRTValue.from_i16({})", name),
        TypeMeta::U16 | TypeMeta::Char16 => format!("DynWinRTValue.from_u16({})", name),
        TypeMeta::I32 => format!("DynWinRTValue.from_i32({})", name),
        TypeMeta::U32 => format!("DynWinRTValue.from_u32({})", name),
        TypeMeta::I64 => format!("DynWinRTValue.from_i64({})", name),
        TypeMeta::U64 => format!("DynWinRTValue.from_u64({})", name),
        TypeMeta::F32 => format!("DynWinRTValue.from_f32({})", name),
        TypeMeta::F64 => format!("DynWinRTValue.from_f64({})", name),
        TypeMeta::String => format!("DynWinRTValue.from_hstring({})", name),
        TypeMeta::Guid => format!("DynWinRTValue.from_guid(WinGUID.parse({}))", name),
        TypeMeta::Enum { .. } => format!(
            "DynWinRTValue.enum_value({}, {})",
            py_dynwinrt_type(typ),
            name
        ),
        TypeMeta::Struct {
            name: struct_name, ..
        } if struct_name == "HResult" => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::Struct {
            name: struct_name, ..
        } => format!("_pack_{}({}).to_value()", to_snake_case(struct_name), name),
        _ => panic!("unsupported IReference inner type: {:?}", typ),
    }
}

/// Build Python args list expression for method call.
pub(crate) fn py_build_args_expr(in_params: &[&crate::meta::ParamMeta]) -> String {
    in_params
        .iter()
        .map(|p| py_wrap_arg(&to_snake_case(&p.name), &p.typ))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Convert a Python return expression, given the raw `.call()` result expression.
pub(crate) fn py_convert_return(
    expr: &str,
    return_type: Option<&TypeMeta>,
    is_async: bool,
    known_types: &HashSet<String>,
) -> String {
    if is_async {
        let inner_type = match return_type {
            Some(TypeMeta::AsyncOperation(inner)) => Some(inner.as_ref()),
            Some(TypeMeta::AsyncOperationWithProgress(inner, _)) => Some(inner.as_ref()),
            _ => None,
        };
        let waited = format!("{}.wait()", expr);
        return py_convert_return(&waited, inner_type, false, known_types);
    }
    match return_type {
        Some(TypeMeta::String) | Some(TypeMeta::Guid) => format!("{}.to_string()", expr),
        Some(
            TypeMeta::I8
            | TypeMeta::U8
            | TypeMeta::I16
            | TypeMeta::U16
            | TypeMeta::Char16
            | TypeMeta::I32
            | TypeMeta::U32,
        ) => format!("{}.to_number()", expr),
        Some(TypeMeta::I64 | TypeMeta::U64) => format!("{}.to_i64()", expr),
        Some(TypeMeta::F32 | TypeMeta::F64) => format!("{}.to_f64()", expr),
        Some(TypeMeta::Bool) => format!("{}.to_bool()", expr),
        Some(TypeMeta::Enum { name, .. }) if known_types.contains(name) => {
            format!(
                "_dynwinrt_enum('{}', '{}', {}.to_number())",
                to_snake_case_filename(name),
                name,
                expr
            )
        }
        Some(TypeMeta::Enum { .. }) => format!("{}.to_number()", expr),
        Some(typ @ TypeMeta::Parameterized { name, args, .. })
            if ireference_inner_type(typ).is_some() =>
        {
            let concrete = crate::meta::make_parameterized_name(name, args);
            let wrapper = py_runtime_symbol(&concrete, &concrete);
            format!(
                "(lambda value: None if value.is_null() else {}(value).value)({})",
                wrapper, expr
            )
        }
        Some(TypeMeta::RuntimeClass { name, .. }) if known_types.contains(name) => {
            format!("{}({})", py_runtime_symbol(name, name), expr)
        }
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => format!("{}.to_number()", expr),
        Some(TypeMeta::Struct { name, .. }) => format!("_unpack_{}({})", to_snake_case(name), expr),
        Some(TypeMeta::Delegate { .. }) => expr.to_string(),
        Some(TypeMeta::Interface { name, .. }) if known_types.contains(name) => {
            format!("{}({})", py_runtime_symbol(name, name), expr)
        }
        Some(TypeMeta::Parameterized { name, args, .. }) => {
            let concrete = crate::meta::make_parameterized_name(name, args);
            if known_types.contains(&concrete) {
                format!("{}({})", py_runtime_symbol(&concrete, &concrete), expr)
            } else {
                expr.to_string()
            }
        }
        Some(TypeMeta::Array(inner)) => {
            let arr_expr = format!("{}.as_array()", expr);
            py_convert_array_return(&arr_expr, inner, known_types)
        }
        _ => expr.to_string(),
    }
}

/// Convert an array return expression to the appropriate Python list.
pub(crate) fn py_convert_array_return(
    arr_expr: &str,
    inner: &TypeMeta,
    known_types: &HashSet<String>,
) -> String {
    match inner {
        TypeMeta::I8 => format!("{}.to_i8_list()", arr_expr),
        TypeMeta::U8 => format!("{}.to_u8_list()", arr_expr),
        TypeMeta::I16 => format!("{}.to_i16_list()", arr_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("{}.to_u16_list()", arr_expr),
        TypeMeta::I32 | TypeMeta::Enum { .. } => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::U32 => format!("{}.to_u32_list()", arr_expr),
        TypeMeta::I64 => format!("{}.to_i64_list()", arr_expr),
        TypeMeta::U64 => format!("{}.to_u64_list()", arr_expr),
        TypeMeta::F32 => format!("{}.to_f32_list()", arr_expr),
        TypeMeta::F64 => format!("{}.to_f64_list()", arr_expr),
        TypeMeta::Bool => format!("[v.to_bool() for v in {}.to_values()]", arr_expr),
        TypeMeta::String => format!("{}.to_string_list()", arr_expr),
        TypeMeta::Guid => format!("[v.to_string() for v in {}.to_values()]", arr_expr),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::Struct { name, .. } => format!(
            "[_unpack_{}(v) for v in {}.to_values()]",
            to_snake_case(name),
            arr_expr
        ),
        TypeMeta::RuntimeClass { name, .. } if known_types.contains(name) => {
            format!(
                "_dynwinrt_wrap_values('{}', '{}', {}.to_values())",
                to_snake_case_filename(name),
                name,
                arr_expr
            )
        }
        TypeMeta::Interface { name, .. } if known_types.contains(name) => {
            format!(
                "_dynwinrt_wrap_values('{}', '{}', {}.to_values())",
                to_snake_case_filename(name),
                name,
                arr_expr
            )
        }
        _ => format!("{}.to_values()", arr_expr),
    }
}

/// Generate a Python `_IFoo = DynWinRTType.register_interface(...)` block.
pub(crate) fn py_generate_interface_registration(iface: &InterfaceMeta, var_name: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} = DynWinRTType.register_interface(\n",
        var_name
    ));
    out.push_str(&format!("    \"{}\", IID_{}) \\\n", iface.name, iface.name));
    for (i, method) in iface.methods.iter().enumerate() {
        let trailing = if i + 1 < iface.methods.len() {
            " \\"
        } else {
            ""
        };
        out.push_str(&format!(
            "    .add_method(\"{}\", {}){}\n",
            method.name,
            py_build_method_sig(method),
            trailing
        ));
    }
    out
}
