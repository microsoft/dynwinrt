// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python method signatures, argument wrapping, and return conversion.

use crate::meta::{InterfaceMeta, MethodMeta, ParamDirection};
use crate::types::{TypeIdentity, TypeIdentityKind, TypeMeta};

use super::naming::{PythonProjectionContext, to_snake_case};
use crate::codegen::winrt::python::collections::{CollectionKind, is_mapping_input, type_kind};
use crate::codegen::winrt::python::native_types::{FoundationType, foundation_type};
use crate::codegen::winrt::shared::imports::ireference_inner_type;

pub(crate) fn py_runtime_symbol(
    context: &PythonProjectionContext,
    identity: &TypeIdentity,
    symbol_name: &str,
) -> String {
    format!(
        "_dynwinrt_symbol('{}', '{}')",
        context.implementation_module(identity),
        symbol_name
    )
}

pub(crate) fn py_runtime_type_symbol(
    context: &PythonProjectionContext,
    typ: &TypeMeta,
    symbol_name: &str,
) -> String {
    py_runtime_symbol(context, &context.identity_for_type(typ), symbol_name)
}

pub(crate) fn py_runtime_named_symbol(
    context: &PythonProjectionContext,
    kind: TypeIdentityKind,
    namespace: &str,
    type_name: &str,
    symbol_name: &str,
) -> String {
    py_runtime_symbol(
        context,
        &TypeIdentity::named(kind, namespace, type_name),
        symbol_name,
    )
}

fn py_enum_instance_guard(name: &str) -> String {
    format!("isinstance({name}, __import__('enum').Enum)")
}

fn py_exact_int_guard(name: &str) -> String {
    format!(
        "isinstance({name}, int) and not isinstance({name}, bool) and not {}",
        py_enum_instance_guard(name)
    )
}

fn py_real_number_guard(name: &str) -> String {
    format!(
        "isinstance({name}, (int, float)) and not isinstance({name}, bool) and not {}",
        py_enum_instance_guard(name)
    )
}

pub(crate) fn py_integer_bounds(typ: &TypeMeta) -> Option<(i128, i128)> {
    match typ {
        TypeMeta::I8 => Some((i8::MIN as i128, i8::MAX as i128)),
        TypeMeta::U8 => Some((u8::MIN as i128, u8::MAX as i128)),
        TypeMeta::I16 => Some((i16::MIN as i128, i16::MAX as i128)),
        TypeMeta::U16 => Some((u16::MIN as i128, u16::MAX as i128)),
        TypeMeta::I32 => Some((i32::MIN as i128, i32::MAX as i128)),
        TypeMeta::U32 => Some((u32::MIN as i128, u32::MAX as i128)),
        TypeMeta::I64 => Some((i64::MIN as i128, i64::MAX as i128)),
        TypeMeta::U64 => Some((u64::MIN as i128, u64::MAX as i128)),
        _ => None,
    }
}

/// Return a stable overload-dispatch sort key for a projected Python argument type.
///
/// Python overload dispatch is branch-ordered, so same-arity branches need a
/// canonical specificity order that does not depend on WinMD declaration order.
/// We prefer exact bool/char/string shapes first, then narrower integer ranges
/// (signed before unsigned when widths overlap), and finally float fallbacks.
/// Enums sort after numeric branches because numeric guards reject `Enum`
/// instances, so plain ints still prefer numeric overloads while generated
/// `IntEnum`/`IntFlag` values reach their exact enum branch.
/// `IReference<T>` sorts just after `T` so concrete values prefer the
/// non-nullable overload while `None` still resolves to the nullable branch.
pub(crate) fn py_dispatch_type_sort_key(typ: &TypeMeta) -> (u8, u16, u8, u8, String) {
    if let Some(inner) = ireference_inner_type(typ) {
        let (category, width, detail, _, label) = py_dispatch_type_sort_key(inner);
        let label = if label.is_empty() {
            "IReference".to_string()
        } else {
            format!("IReference<{label}>")
        };
        return (category, width, detail, 1, label);
    }

    match typ {
        TypeMeta::Bool => (0, 0, 0, 0, String::new()),
        TypeMeta::Char16 => (1, 0, 0, 0, String::new()),
        TypeMeta::String => (2, 0, 0, 0, String::new()),
        TypeMeta::I8 => (3, 8, 0, 0, String::new()),
        TypeMeta::U8 => (3, 8, 1, 0, String::new()),
        TypeMeta::I16 => (3, 16, 0, 0, String::new()),
        TypeMeta::U16 => (3, 16, 1, 0, String::new()),
        TypeMeta::I32 => (3, 32, 0, 0, String::new()),
        TypeMeta::U32 => (3, 32, 1, 0, String::new()),
        TypeMeta::I64 => (3, 64, 0, 0, String::new()),
        TypeMeta::U64 => (3, 64, 1, 0, String::new()),
        // Python's native float is a C double, so prefer F64 when both float
        // widths would otherwise accept the same runtime value.
        TypeMeta::F64 => (4, 0, 0, 0, String::new()),
        TypeMeta::F32 => (4, 1, 0, 0, String::new()),
        TypeMeta::Enum { name, .. } => (5, 0, 0, 0, name.clone()),
        TypeMeta::Guid => (6, 0, 0, 0, "Guid".to_string()),
        TypeMeta::Array(_) => (7, 0, 0, 0, format!("{typ:?}")),
        _ => (8, 0, 0, 0, format!("{typ:?}")),
    }
}

// ======================================================================
// Python type expression
// ======================================================================

pub(crate) fn py_runtime_class_iid_const(typ: &TypeMeta) -> Option<(String, String)> {
    let TypeMeta::RuntimeClass {
        namespace,
        name,
        default_interface,
    } = typ
    else {
        return None;
    };
    let TypeMeta::Interface { iid, .. } = default_interface.as_deref()? else {
        return None;
    };
    if iid.is_empty() {
        return None;
    }
    let qualified = format!("{}_{}", namespace, name)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    Some((format!("IID_ARG_{}", qualified), iid.clone()))
}

pub(crate) fn py_collect_runtime_class_iid_consts(
    typ: &TypeMeta,
    output: &mut Vec<(String, String)>,
) {
    if let Some(value) = py_runtime_class_iid_const(typ) {
        output.push(value);
    }
    match typ {
        TypeMeta::Parameterized { args, .. } => {
            for argument in args {
                py_collect_runtime_class_iid_consts(argument, output);
            }
        }
        TypeMeta::Array(inner)
        | TypeMeta::AsyncActionWithProgress(inner)
        | TypeMeta::AsyncOperation(inner) => {
            py_collect_runtime_class_iid_consts(inner, output);
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            py_collect_runtime_class_iid_consts(result, output);
            py_collect_runtime_class_iid_consts(progress, output);
        }
        _ => {}
    }
}

fn py_runtime_class_wrap(name: &str, typ: &TypeMeta) -> String {
    let raw = format!("getattr({}, '_obj', {})", name, name);
    py_runtime_class_iid_const(typ)
        .map(|(iid, _)| format!("{}.cast({})", raw, iid))
        .unwrap_or(raw)
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
            match default_interface.as_deref() {
                Some(default) => format!(
                    "DynWinRTType.runtime_class('{}', {})",
                    full_name,
                    py_dynwinrt_type(default)
                ),
                None => "DynWinRTType.object()".to_string(),
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

    if let Some(wrapped) = py_wrap_collection(name, typ) {
        return wrapped;
    }

    match typ {
        TypeMeta::String => format!("DynWinRTValue.from_hstring({})", name),
        TypeMeta::Bool => format!("DynWinRTValue.from_bool({})", name),
        TypeMeta::I8 => format!("DynWinRTValue.from_i8({})", name),
        TypeMeta::U8 => format!("DynWinRTValue.from_u8({})", name),
        TypeMeta::I16 => format!("DynWinRTValue.from_i16({})", name),
        TypeMeta::U16 => format!("DynWinRTValue.from_u16({})", name),
        TypeMeta::Char16 => format!("DynWinRTValue.from_u16(ord({}))", name),
        TypeMeta::I32 => format!("DynWinRTValue.from_i32({})", name),
        TypeMeta::U32 => format!("DynWinRTValue.from_u32({})", name),
        TypeMeta::I64 => format!("DynWinRTValue.from_i64({})", name),
        TypeMeta::U64 => format!("DynWinRTValue.from_u64({})", name),
        TypeMeta::Enum { .. } => format!(
            "DynWinRTValue.enum_value({}, int({}))",
            py_dynwinrt_type(typ),
            name
        ),
        TypeMeta::F32 => format!("DynWinRTValue.from_f32({})", name),
        TypeMeta::F64 => format!("DynWinRTValue.from_f64({})", name),
        TypeMeta::Guid => format!("DynWinRTValue.from_guid(_dynwinrt_guid({}))", name),
        TypeMeta::RuntimeClass { .. } => py_runtime_class_wrap(name, typ),
        TypeMeta::Object | TypeMeta::Interface { .. } | TypeMeta::Delegate { .. } => {
            format!("getattr({}, '_obj', {})", name, name)
        }
        TypeMeta::Parameterized { .. } => format!("getattr({}, '_obj', {})", name, name),
        TypeMeta::Array(inner) => format!(
            "_dynwinrt_array({}, lambda item: {}, {}, {})",
            name,
            py_wrap_native_value("item", inner),
            py_dynwinrt_type(inner),
            if matches!(inner.as_ref(), TypeMeta::U8) {
                "True"
            } else {
                "False"
            }
        ),
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
    py_wrap_native_value(name, typ)
}

pub(crate) fn py_wrap_native_value(name: &str, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => format!("DynWinRTValue.from_bool({})", name),
        TypeMeta::I8 => format!("DynWinRTValue.from_i8({})", name),
        TypeMeta::U8 => format!("DynWinRTValue.from_u8({})", name),
        TypeMeta::I16 => format!("DynWinRTValue.from_i16({})", name),
        TypeMeta::U16 => format!("DynWinRTValue.from_u16({})", name),
        TypeMeta::Char16 => format!("DynWinRTValue.from_u16(ord({}))", name),
        TypeMeta::I32 => format!("DynWinRTValue.from_i32({})", name),
        TypeMeta::U32 => format!("DynWinRTValue.from_u32({})", name),
        TypeMeta::I64 => format!("DynWinRTValue.from_i64({})", name),
        TypeMeta::U64 => format!("DynWinRTValue.from_u64({})", name),
        TypeMeta::F32 => format!("DynWinRTValue.from_f32({})", name),
        TypeMeta::F64 => format!("DynWinRTValue.from_f64({})", name),
        TypeMeta::String => format!("DynWinRTValue.from_hstring({})", name),
        TypeMeta::Guid => format!("DynWinRTValue.from_guid(_dynwinrt_guid({}))", name),
        TypeMeta::Enum { .. } => format!(
            "DynWinRTValue.enum_value({}, {})",
            py_dynwinrt_type(typ),
            format!("int({name})")
        ),
        TypeMeta::Struct {
            name: struct_name, ..
        } if struct_name == "HResult" => {
            format!("DynWinRTValue.from_i32({})", name)
        }
        TypeMeta::Struct {
            name: struct_name, ..
        } => format!("_pack_{}({}).to_value()", to_snake_case(struct_name), name),
        TypeMeta::RuntimeClass { .. } => py_runtime_class_wrap(name, typ),
        TypeMeta::Object
        | TypeMeta::Interface { .. }
        | TypeMeta::Parameterized { .. }
        | TypeMeta::Delegate { .. } => format!("getattr({}, '_obj', {})", name, name),
        TypeMeta::Array(inner) => format!(
            "_dynwinrt_array({}, lambda item: {}, {}, {})",
            name,
            py_wrap_native_value("item", inner),
            py_dynwinrt_type(inner),
            if matches!(inner.as_ref(), TypeMeta::U8) {
                "True"
            } else {
                "False"
            }
        ),
        _ => panic!("unsupported Python value type: {:?}", typ),
    }
}

fn py_wrap_collection(name: &str, typ: &TypeMeta) -> Option<String> {
    let TypeMeta::Parameterized { args, .. } = typ else {
        return None;
    };
    let kind = type_kind(typ)?;
    if is_mapping_input(kind, args) {
        let (key, value) = if matches!(
            kind,
            CollectionKind::Mapping | CollectionKind::MutableMapping
        ) {
            (args.first()?, args.get(1)?)
        } else {
            match args.first()? {
                TypeMeta::Parameterized {
                    args: pair_args, ..
                } => (pair_args.first()?, pair_args.get(1)?),
                _ => return None,
            }
        };
        return Some(format!(
            "_dynwinrt_map({}, lambda item: {}, lambda item: {}, {}, {})",
            name,
            py_wrap_native_value("item", key),
            py_wrap_native_value("item", value),
            py_dynwinrt_type(key),
            py_dynwinrt_type(value)
        ));
    }
    if matches!(
        kind,
        CollectionKind::Iterable | CollectionKind::Sequence | CollectionKind::MutableSequence
    ) {
        let element = args.first()?;
        return Some(format!(
            "_dynwinrt_vector({}, lambda item: {}, {})",
            name,
            py_wrap_native_value("item", element),
            py_dynwinrt_type(element)
        ));
    }
    None
}

pub(crate) fn py_type_guard(
    name: &str,
    typ: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    if let Some(inner) = ireference_inner_type(typ) {
        return format!(
            "({name} is None or {})",
            py_type_guard(name, inner, context)
        );
    }

    if let TypeMeta::Parameterized { args, .. } = typ
        && let Some(kind) = type_kind(typ)
    {
        if is_mapping_input(kind, args) {
            return format!("isinstance({name}, Mapping)");
        }
        if matches!(
            kind,
            CollectionKind::Iterable
                | CollectionKind::Iterator
                | CollectionKind::Sequence
                | CollectionKind::MutableSequence
        ) {
            return format!(
                "isinstance({name}, Iterable) and not isinstance({name}, (str, bytes, bytearray))"
            );
        }
    }
    if let Some((min, max)) = py_integer_bounds(typ) {
        return format!("{} and {min} <= {name} <= {max}", py_exact_int_guard(name));
    }
    match typ {
        TypeMeta::Bool => format!("isinstance({name}, bool)"),
        TypeMeta::F32 | TypeMeta::F64 => py_real_number_guard(name),
        TypeMeta::Char16 => {
            format!("isinstance({name}, str) and len({name}) == 1 and ord({name}) <= 65535")
        }
        TypeMeta::String => format!("isinstance({name}, str)"),
        TypeMeta::Guid => format!("isinstance({name}, UUID)"),
        TypeMeta::Enum {
            namespace,
            name: type_name,
            ..
        } if context.is_known_type(typ) => format!(
            "isinstance({name}, {})",
            py_runtime_named_symbol(
                context,
                TypeIdentityKind::Enum,
                namespace,
                type_name,
                type_name
            )
        ),
        TypeMeta::Enum { .. } => py_exact_int_guard(name),
        TypeMeta::Array(_) => format!(
            "isinstance({name}, (DynWinRTArray, bytes, bytearray, Sequence)) and not isinstance({name}, str)"
        ),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => {
            format!("isinstance({name}, datetime)")
        }
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => {
            format!("isinstance({name}, timedelta)")
        }
        TypeMeta::Struct { .. } => format!(
            "isinstance({name}, {})",
            context.reference_name_for_type(typ)
        ),
        typ @ TypeMeta::RuntimeClass { .. } if py_runtime_class_iid_const(typ).is_some() => {
            let (iid, _) = py_runtime_class_iid_const(typ).expect("checked above");
            format!("_dynwinrt_can_cast({name}, {iid})")
        }
        TypeMeta::RuntimeClass {
            namespace,
            name: type_name,
            ..
        } if context.is_known_type(typ) => format!(
            "isinstance({name}, {})",
            py_runtime_named_symbol(
                context,
                TypeIdentityKind::Class,
                namespace,
                type_name,
                type_name
            )
        ),
        TypeMeta::Interface {
            namespace,
            name: type_name,
            ..
        } if context.is_known_type(typ) => format!(
            "isinstance({name}, {})",
            py_runtime_named_symbol(
                context,
                TypeIdentityKind::Interface,
                namespace,
                type_name,
                type_name
            )
        ),
        TypeMeta::Object
        | TypeMeta::Delegate { .. }
        | TypeMeta::RuntimeClass { .. }
        | TypeMeta::Interface { .. }
        | TypeMeta::Parameterized { .. } => {
            format!("isinstance(getattr({name}, '_obj', {name}), DynWinRTValue)")
        }
        _ => "True".to_string(),
    }
}

/// Convert a Python return expression, given the raw `.call()` result expression.
pub(crate) fn py_convert_return(
    expr: &str,
    return_type: Option<&TypeMeta>,
    is_async: bool,
    context: &PythonProjectionContext,
) -> String {
    if is_async {
        return py_wrap_async(
            expr,
            return_type.expect("async conversion requires a return type"),
            None,
            context,
        );
    }
    match return_type {
        Some(TypeMeta::String) => format!("{}.to_string()", expr),
        Some(TypeMeta::Guid) => format!("_dynwinrt_uuid({}.to_guid())", expr),
        Some(TypeMeta::I8 | TypeMeta::U8 | TypeMeta::I16 | TypeMeta::U16 | TypeMeta::I32) => {
            format!("{}.to_number()", expr)
        }
        Some(TypeMeta::U32) => format!("{}.to_u32()", expr),
        Some(TypeMeta::Char16) => format!("chr({}.to_number())", expr),
        Some(TypeMeta::I64) => format!("{}.to_i64()", expr),
        Some(TypeMeta::U64) => format!("{}.to_u64()", expr),
        Some(TypeMeta::F32 | TypeMeta::F64) => format!("{}.to_f64()", expr),
        Some(TypeMeta::Bool) => format!("{}.to_bool()", expr),
        Some(typ @ TypeMeta::Enum { name, .. }) if context.is_known_type(typ) => {
            let TypeMeta::Enum { name, .. } = typ else {
                unreachable!()
            };
            format!(
                "_dynwinrt_enum('{}', '{}', {}.to_number())",
                context.implementation_module_for_type(typ),
                name,
                expr
            )
        }

        Some(TypeMeta::Enum { .. }) => format!("{}.to_number()", expr),
        Some(typ @ TypeMeta::Parameterized { .. }) if ireference_inner_type(typ).is_some() => {
            let concrete = context.projected_name_for_type(typ);
            let wrapper = py_runtime_type_symbol(context, typ, &concrete);
            format!(
                "(lambda value: None if value.is_null() else {}(value).value)({})",
                wrapper, expr
            )
        }
        Some(typ @ TypeMeta::RuntimeClass { name, .. }) if context.is_known_type(typ) => {
            let TypeMeta::RuntimeClass { name, .. } = typ else {
                unreachable!()
            };
            let wrapper = py_runtime_type_symbol(context, typ, name);
            format!(
                "(lambda value: None if value.is_null() else {}._from_native(value))({})",
                wrapper, expr
            )
        }
        Some(TypeMeta::Struct { name, .. }) if name == "HResult" => format!("{}.to_number()", expr),
        Some(TypeMeta::Struct { name, .. }) => format!("_unpack_{}({})", to_snake_case(name), expr),
        Some(TypeMeta::Delegate { .. }) => {
            format!(
                "(lambda value: None if value.is_null() else value)({})",
                expr
            )
        }
        Some(typ @ TypeMeta::Interface { .. }) if context.is_known_type(typ) => {
            let projected_name = context.projected_name_for_type(typ);
            let wrapper = py_runtime_type_symbol(context, typ, &projected_name);
            format!(
                "(lambda value: None if value.is_null() else {}(value))({})",
                wrapper, expr
            )
        }
        Some(TypeMeta::Object | TypeMeta::RuntimeClass { .. } | TypeMeta::Interface { .. }) => {
            format!(
                "(lambda value: None if value.is_null() else value)({})",
                expr
            )
        }
        Some(typ @ TypeMeta::Parameterized { .. }) => {
            let concrete = context.projected_name_for_type(typ);
            if context.is_known_type(typ) {
                let wrapper = py_runtime_type_symbol(context, typ, &concrete);
                format!(
                    "(lambda value: None if value.is_null() else {}(value))({})",
                    wrapper, expr
                )
            } else {
                format!(
                    "(lambda value: None if value.is_null() else value)({})",
                    expr
                )
            }
        }
        Some(TypeMeta::Array(inner)) => {
            let arr_expr = format!("{}.as_array()", expr);
            py_convert_array_return(&arr_expr, inner, context)
        }
        _ => expr.to_string(),
    }
}

fn py_value_converter(typ: &TypeMeta, context: &PythonProjectionContext) -> String {
    format!(
        "lambda value: {}",
        py_convert_return("value", Some(typ), false, context)
    )
}

pub(crate) fn py_wrap_async(
    expr: &str,
    async_type: &TypeMeta,
    result_converter: Option<String>,
    context: &PythonProjectionContext,
) -> String {
    py_wrap_async_with_converters(expr, async_type, result_converter, None, context)
}

pub(crate) fn py_wrap_async_with_converters(
    expr: &str,
    async_type: &TypeMeta,
    result_converter: Option<String>,
    progress_converter: Option<String>,
    context: &PythonProjectionContext,
) -> String {
    match async_type {
        TypeMeta::AsyncAction => format!(
            "_dynwinrt_track_projected(_DynWinRTAsync({}, {}), 'WinRTAsync')",
            expr,
            result_converter.unwrap_or_else(|| "lambda _value: None".to_string())
        ),
        TypeMeta::AsyncOperation(result) => format!(
            "_dynwinrt_track_projected(_DynWinRTAsync({}, {}), 'WinRTAsync')",
            expr,
            result_converter.unwrap_or_else(|| py_value_converter(result, context))
        ),
        TypeMeta::AsyncActionWithProgress(progress) => format!(
            "_dynwinrt_track_projected(_DynWinRTAsyncWithProgress({}, {}, {}), 'WinRTAsyncWithProgress')",
            expr,
            result_converter.unwrap_or_else(|| "lambda _value: None".to_string()),
            progress_converter.unwrap_or_else(|| py_value_converter(progress, context))
        ),
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "_dynwinrt_track_projected(_DynWinRTAsyncWithProgress({}, {}, {}), 'WinRTAsyncWithProgress')",
            expr,
            result_converter.unwrap_or_else(|| py_value_converter(result, context)),
            progress_converter.unwrap_or_else(|| py_value_converter(progress, context))
        ),
        _ => panic!("py_wrap_async requires an async type: {:?}", async_type),
    }
}

/// Convert an array return expression to the appropriate Python list.
pub(crate) fn py_convert_array_return(
    arr_expr: &str,
    inner: &TypeMeta,
    context: &PythonProjectionContext,
) -> String {
    match inner {
        TypeMeta::I8 => format!("{}.to_i8_list()", arr_expr),
        TypeMeta::U8 => format!("{}.to_bytes()", arr_expr),
        TypeMeta::I16 => format!("{}.to_i16_list()", arr_expr),
        TypeMeta::U16 | TypeMeta::Char16 => format!("{}.to_u16_list()", arr_expr),
        TypeMeta::I32 => format!("{}.to_i32_list()", arr_expr),
        typ @ TypeMeta::Enum { name, .. } if context.is_known_type(typ) => format!(
            "[_dynwinrt_enum('{}', '{}', value) for value in {}.to_i32_list()]",
            context.implementation_module_for_type(typ),
            name,
            arr_expr
        ),
        TypeMeta::Enum { .. } => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::U32 => format!("{}.to_u32_list()", arr_expr),
        TypeMeta::I64 => format!("{}.to_i64_list()", arr_expr),
        TypeMeta::U64 => format!("{}.to_u64_list()", arr_expr),
        TypeMeta::F32 => format!("{}.to_f32_list()", arr_expr),
        TypeMeta::F64 => format!("{}.to_f64_list()", arr_expr),
        TypeMeta::Bool => format!("[v.to_bool() for v in {}.to_values()]", arr_expr),
        TypeMeta::String => format!("{}.to_string_list()", arr_expr),
        TypeMeta::Guid => format!(
            "[_dynwinrt_uuid(v.to_guid()) for v in {}.to_values()]",
            arr_expr
        ),
        TypeMeta::Struct { name, .. } if name == "HResult" => format!("{}.to_i32_list()", arr_expr),
        TypeMeta::Struct { name, .. } => format!(
            "[_unpack_{}(v) for v in {}.to_values()]",
            to_snake_case(name),
            arr_expr
        ),
        typ @ TypeMeta::RuntimeClass { name, .. } if context.is_known_type(typ) => {
            format!(
                "_dynwinrt_wrap_values('{}', '{}', {}.to_values())",
                context.implementation_module_for_type(typ),
                name,
                arr_expr
            )
        }
        typ @ TypeMeta::Interface { .. } if context.is_known_type(typ) => {
            let projected_name = context.projected_name_for_type(typ);
            format!(
                "_dynwinrt_wrap_values('{}', '{}', {}.to_values())",
                context.implementation_module_for_type(typ),
                projected_name,
                arr_expr
            )
        }
        typ @ TypeMeta::Parameterized { .. } => {
            let concrete = context.projected_name_for_type(typ);
            if context.is_known_type(typ) {
                format!(
                    "_dynwinrt_wrap_values('{}', '{}', {}.to_values())",
                    context.implementation_module_for_type(typ),
                    concrete,
                    arr_expr
                )
            } else {
                format!(
                    "[None if v.is_null() else v for v in {}.to_values()]",
                    arr_expr
                )
            }
        }
        TypeMeta::Object | TypeMeta::Delegate { .. } => format!(
            "[None if v.is_null() else v for v in {}.to_values()]",
            arr_expr
        ),
        _ => format!("{}.to_values()", arr_expr),
    }
}

/// Generate a Python `_IFoo = DynWinRTType.register_interface(...)` block.
pub(crate) fn py_generate_interface_registration(
    iface: &InterfaceMeta,
    var_name: &str,
    symbol_name: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} = DynWinRTType.register_interface(\n",
        var_name
    ));
    out.push_str(&format!("    \"{}\", IID_{symbol_name}) \\\n", iface.name));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enum_type(name: &str, is_flags: bool) -> TypeMeta {
        TypeMeta::Enum {
            namespace: "Contoso".into(),
            name: name.into(),
            underlying: Box::new(TypeMeta::I32),
            members: Vec::new(),
            is_flags,
            doc: None,
            deprecated: None,
        }
    }

    fn geometry_type() -> TypeMeta {
        TypeMeta::RuntimeClass {
            namespace: "Microsoft.UI.Xaml.Media".into(),
            name: "Geometry".into(),
            default_interface: Some(Box::new(TypeMeta::Interface {
                namespace: "Microsoft.UI.Xaml.Media".into(),
                name: "IGeometry".into(),
                iid: "dc102dcc-3be2-5414-8599-94b6e76ef39b".into(),
            })),
        }
    }

    #[test]
    fn runtime_class_inputs_cast_to_the_expected_interface() {
        let geometry = geometry_type();
        assert_eq!(
            py_type_guard("value", &geometry, &PythonProjectionContext::default()),
            "_dynwinrt_can_cast(value, IID_ARG_Microsoft_UI_Xaml_Media_Geometry)"
        );
        assert_eq!(
            py_wrap_native_value("value", &geometry),
            "getattr(value, '_obj', value).cast(IID_ARG_Microsoft_UI_Xaml_Media_Geometry)"
        );
        let mut constants = Vec::new();
        py_collect_runtime_class_iid_consts(&TypeMeta::Array(Box::new(geometry)), &mut constants);
        assert_eq!(
            constants,
            vec![(
                "IID_ARG_Microsoft_UI_Xaml_Media_Geometry".into(),
                "dc102dcc-3be2-5414-8599-94b6e76ef39b".into(),
            )]
        );
    }

    #[test]
    fn python_numeric_overload_integer_guards_use_exact_ranges() {
        let context = PythonProjectionContext::default();
        assert_eq!(
            py_type_guard("value", &TypeMeta::I8, &context),
            "isinstance(value, int) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum) and -128 <= value <= 127"
        );
        assert_eq!(
            py_type_guard("value", &TypeMeta::U8, &context),
            "isinstance(value, int) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum) and 0 <= value <= 255"
        );
        assert_eq!(
            py_type_guard("value", &TypeMeta::U64, &context),
            format!(
                "isinstance(value, int) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum) and 0 <= value <= {}",
                u64::MAX
            )
        );
    }

    #[test]
    fn python_numeric_overload_float_guards_reject_enum_instances() {
        let context = PythonProjectionContext::default();
        assert_eq!(
            py_type_guard("value", &TypeMeta::F64, &context),
            "isinstance(value, (int, float)) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum)"
        );
    }

    #[test]
    fn python_known_enum_guards_are_precise_and_unknown_enums_fail_closed() {
        let mode = enum_type("Mode", false);
        let context = PythonProjectionContext::packaged([mode.type_identity()]).unwrap();
        assert_eq!(
            py_type_guard("value", &mode, &context),
            "isinstance(value, _dynwinrt_symbol('contoso__mode', 'Mode'))"
        );
        assert_eq!(
            py_type_guard(
                "value",
                &enum_type("Mode", false),
                &PythonProjectionContext::default()
            ),
            "isinstance(value, int) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum)"
        );
        assert_eq!(
            py_type_guard(
                "value",
                &enum_type("Options", true),
                &PythonProjectionContext::default()
            ),
            "isinstance(value, int) and not isinstance(value, bool) and not isinstance(value, __import__('enum').Enum)"
        );
    }
}
