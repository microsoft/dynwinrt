// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::super::ir::{
    ComEnumUnderlying, ComPrimitive, ComScalarRepr, ComType, NativePodArchitectureLayout,
    NativePodFieldType, NativePodLayout, NativePodScalar, NativeUnionArchitectureLayout,
    NativeUnionFieldType, NativeUnionLayout, PointerAliasKind, ProjectedComResult,
    ResultConversion, SafeArrayElement, StringEncoding,
};
#[cfg(test)]
use super::super::ir::{NativePodField, NativeUnionField};

pub(super) fn abi_type_js(typ: &ComType) -> String {
    match typ {
        ComType::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => "DynCom.boolType()",
            ComPrimitive::I8 => "DynCom.i8Type()",
            ComPrimitive::U8 => "DynCom.u8Type()",
            ComPrimitive::I16 => "DynCom.i16Type()",
            ComPrimitive::U16 => "DynCom.u16Type()",
            ComPrimitive::I32 => "DynCom.i32Type()",
            ComPrimitive::U32 => "DynCom.u32Type()",
            ComPrimitive::I64 => "DynCom.i64Type()",
            ComPrimitive::U64 => "DynCom.u64Type()",
            ComPrimitive::F32 => "DynCom.f32Type()",
            ComPrimitive::F64 => "DynCom.f64Type()",
            ComPrimitive::Char16 => "DynCom.char16Type()",
        }
        .into(),
        ComType::NativeIsize => "DynCom.isizeType()".into(),
        ComType::NativeUsize => "DynCom.usizeType()".into(),
        ComType::Win32Bool | ComType::HResult => "DynCom.i32Type()".into(),
        ComType::Guid => "DynCom.guidType()".into(),
        ComType::HString => "DynCom.hstringType()".into(),
        ComType::Enum { underlying, .. } => enum_abi_type_js(*underlying).into(),
        ComType::ScalarAlias { underlying, .. } => scalar_abi_type_js(*underlying).into(),
        ComType::RawPointer | ComType::PointerAlias { .. } => "DynCom.pointerType()".into(),
        ComType::Bstr => "DynCom.bstrType()".into(),
        ComType::NativePod { layout } => {
            format!("DynCom.nativeStructType({})", native_pod_layout_js(layout))
        }
        ComType::NativePodPointer { layout } => format!(
            "DynCom.nativeStructPointerType({})",
            native_pod_layout_js(layout)
        ),
        ComType::NativeUnionPointer { layout } => format!(
            "DynCom.nativeUnionPointerType({})",
            native_union_layout_js(layout)
        ),
        ComType::Variant => "DynCom.variantType()".into(),
        ComType::VariantByValue => "DynCom.variantByValueType()".into(),
        ComType::SafeArray { element } => safe_array_abi_type_js(*element, false),
        ComType::PropVariant => "DynCom.propVariantType()".into(),
        ComType::DispatchParams => "DynCom.dispatchParamsType()".into(),
        ComType::ExcepInfo => "DynCom.excepInfoType()".into(),
        ComType::ManagedInterface { iid } => {
            format!("DynCom.interfaceType(WinGuid.parse('{iid}'))")
        }
        ComType::CoTaskMemWideString => "DynCom.coTaskMemWideStringType()".into(),
        ComType::StringArray { .. } => "DynCom.pointerType()".into(),
        ComType::TypedBuffer { element } => abi_type_js(element),
        ComType::OwningArray { element, .. } => abi_type_js(element),
    }
}

pub(super) fn safe_array_abi_type_js(element: SafeArrayElement, nullable: bool) -> String {
    match element {
        SafeArrayElement::Interface { iid } => format!(
            "DynCom.safeArrayType('unknown', WinGuid.parse('{}'){})",
            super::super::project::format_guid(&iid),
            if nullable { ", true" } else { "" }
        ),
        _ => format!(
            "DynCom.safeArrayType('{}'{}{})",
            safe_array_element_name(element),
            if nullable { ", undefined" } else { "" },
            if nullable { ", true" } else { "" }
        ),
    }
}

fn safe_array_element_name(element: SafeArrayElement) -> &'static str {
    match element {
        SafeArrayElement::I8 => "i8",
        SafeArrayElement::U8 => "u8",
        SafeArrayElement::I16 => "i16",
        SafeArrayElement::U16 => "u16",
        SafeArrayElement::I32 => "i32",
        SafeArrayElement::U32 => "u32",
        SafeArrayElement::I64 => "i64",
        SafeArrayElement::U64 => "u64",
        SafeArrayElement::F32 => "f32",
        SafeArrayElement::F64 => "f64",
        SafeArrayElement::Bool => "bool",
        SafeArrayElement::Bstr => "bstr",
        SafeArrayElement::Interface { .. } => {
            unreachable!("interface SAFEARRAY elements use exact-IID rendering")
        }
        SafeArrayElement::Variant => "variant",
    }
}

pub(super) fn input_type_dts(typ: &ComType) -> String {
    match typ {
        ComType::PointerAlias {
            name,
            kind: PointerAliasKind::HandleValue,
            ..
        } if name == "HWND" => format!("{name} | Buffer | Uint8Array"),
        _ => type_dts(typ),
    }
}

pub(super) fn type_dts(typ: &ComType) -> String {
    match typ {
        ComType::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => "boolean",
            ComPrimitive::I8
            | ComPrimitive::U8
            | ComPrimitive::I16
            | ComPrimitive::U16
            | ComPrimitive::I32
            | ComPrimitive::U32
            | ComPrimitive::F32
            | ComPrimitive::F64
            | ComPrimitive::Char16 => "number",
            ComPrimitive::I64 | ComPrimitive::U64 => "bigint",
        }
        .into(),
        ComType::NativeIsize | ComType::NativeUsize => "bigint".into(),
        ComType::Win32Bool => "boolean".into(),
        ComType::HResult => "number".into(),
        ComType::Guid => "string".into(),
        ComType::HString => "string".into(),
        ComType::Enum { name, .. } => name.clone(),
        ComType::ScalarAlias { name, .. } => name.clone(),
        ComType::RawPointer => "Buffer | Uint8Array".into(),
        ComType::PointerAlias { name, .. } => name.clone(),
        ComType::NativePod { layout } | ComType::NativePodPointer { layout } => layout.name.clone(),
        ComType::NativeUnionPointer { layout } => layout.name.clone(),
        ComType::Bstr => "string".into(),
        ComType::Variant => "DynComVariant".into(),
        ComType::VariantByValue => "DynComVariant".into(),
        ComType::SafeArray { .. } => "DynComSafeArray".into(),
        ComType::PropVariant => "DynComPropVariant".into(),
        ComType::DispatchParams => "DynComDispatchParams".into(),
        ComType::ExcepInfo => "DynComExcepInfo".into(),
        ComType::ManagedInterface { .. } => "DynWinRtValue".into(),
        ComType::CoTaskMemWideString => "string".into(),
        ComType::StringArray { .. } => "string[]".into(),
        ComType::TypedBuffer { element } => match element.as_ref() {
            ComType::NativePod { layout } => format!("{}Array", layout.name),
            _ => "Buffer | ArrayBufferView".into(),
        },
        ComType::OwningArray { element, interface } => interface.as_ref().map_or_else(
            || format!("{}[]", type_dts(element)),
            |interface| format!("{}[]", interface.name),
        ),
    }
}

pub(super) fn result_type_dts(result: &ProjectedComResult) -> String {
    match &result.conversion {
        ResultConversion::Bstr | ResultConversion::CoTaskMemString(_) => "string".into(),
        ResultConversion::CoTaskMemData
        | ResultConversion::ManagedCom
        | ResultConversion::DynamicIidAdoption => "DynWinRtValue".into(),
        ResultConversion::Value | ResultConversion::BorrowedHandle | ResultConversion::HString => {
            type_dts(&result.typ)
        }
        ResultConversion::Buffer => "Buffer".into(),
        ResultConversion::PlainArray => {
            let ComType::TypedBuffer { element } = &result.typ else {
                unreachable!("plain array result requires a typed-buffer result")
            };
            format!("{}[]", type_dts(element))
        }
        ResultConversion::EnumeratorArray { interface } => {
            let (element, projected_interface) = projected_array_parts(&result.typ);
            interface.as_ref().or(projected_interface).map_or_else(
                || format!("{}[]", type_dts(element)),
                |interface| format!("{}[]", interface.name),
            )
        }
        ResultConversion::OwningArray { interface } => {
            let (element, projected_interface) = projected_array_parts(&result.typ);
            interface.as_ref().or(projected_interface).map_or_else(
                || format!("{}[]", type_dts(element)),
                |interface| format!("{}[]", interface.name),
            )
        }
        ResultConversion::Variant => "DynComVariant".into(),
        ResultConversion::SafeArray => "DynComSafeArray".into(),
        ResultConversion::PropVariant => "DynComPropVariant".into(),
        ResultConversion::ExcepInfo => "DynComExcepInfo".into(),
    }
}

pub(super) fn wrap_arg_js(typ: &ComType, variable: &str) -> String {
    match typ {
        ComType::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => format!("DynCom.boolValue({variable})"),
            ComPrimitive::I8 => format!("DynCom.i8Value({variable})"),
            ComPrimitive::U8 => format!("DynCom.u8Value({variable})"),
            ComPrimitive::I16 => format!("DynCom.i16({variable})"),
            ComPrimitive::U16 => format!("DynCom.u16({variable})"),
            ComPrimitive::I32 => format!("DynCom.i32({variable})"),
            ComPrimitive::U32 => format!("DynCom.u32({variable})"),
            ComPrimitive::I64 => format!("DynCom.i64(BigInt({variable}))"),
            ComPrimitive::U64 => format!("DynCom.u64(BigInt({variable}))"),
            ComPrimitive::F32 => format!("DynCom.f32({variable})"),
            ComPrimitive::F64 => format!("DynCom.f64({variable})"),
            ComPrimitive::Char16 => format!("DynCom.char16({variable})"),
        },
        ComType::NativeIsize => format!("DynCom.isize(BigInt({variable}))"),
        ComType::NativeUsize => format!("DynCom.usize(BigInt({variable}))"),
        ComType::Win32Bool => format!("DynCom.i32({variable} ? 1 : 0)"),
        ComType::HResult => format!("DynCom.i32({variable})"),
        ComType::Guid => format!("DynCom.guid(WinGuid.parse({variable}))"),
        ComType::HString => format!("DynCom.hstring({variable})"),
        ComType::Enum { underlying, .. } => wrap_enum_arg_js(*underlying, variable),
        ComType::ScalarAlias { underlying, .. } => wrap_scalar_arg_js(*underlying, variable),
        ComType::RawPointer => format!("DynCom.safeDataPointer({variable})"),
        ComType::Bstr => format!("DynCom.bstr({variable})"),
        ComType::PointerAlias {
            name,
            kind: PointerAliasKind::HandleValue,
            ..
        } if name == "HWND" => {
            format!("DynCom.pointer(DynCom.handleValue({variable}))")
        }
        ComType::PointerAlias {
            kind: PointerAliasKind::StringPointer(StringEncoding::Wide),
            ..
        } => format!("DynCom.safeWideStringPointer({variable})"),
        ComType::PointerAlias {
            kind: PointerAliasKind::StringPointer(StringEncoding::Ansi),
            ..
        } => format!("DynCom.safeAnsiStringPointer({variable})"),
        ComType::PointerAlias {
            kind: PointerAliasKind::DataPointer,
            ..
        } => format!("DynCom.safeDataPointer({variable})"),
        ComType::PointerAlias {
            kind: PointerAliasKind::HandleValue,
            ..
        } => format!("DynCom.pointer(DynCom.handleValue({variable}))"),
        ComType::NativePod { layout } | ComType::NativePodPointer { layout } => format!(
            "DynCom.nativeStruct({}, {variable})",
            native_pod_layout_js(layout)
        ),
        ComType::NativeUnionPointer { layout } => format!(
            "DynCom.nativeUnion({}, {variable})",
            native_union_layout_js(layout)
        ),
        ComType::Variant => format!("DynCom.variant({variable})"),
        ComType::VariantByValue => format!("DynCom.variant({variable})"),
        ComType::SafeArray { .. } => format!("DynCom.safeArray({variable})"),
        ComType::PropVariant => format!("DynCom.propVariant({variable})"),
        ComType::DispatchParams => format!("DynCom.dispatchParams({variable})"),
        ComType::ExcepInfo => unreachable!("EXCEPINFO is output-only"),
        ComType::ManagedInterface { .. } => variable.to_string(),
        ComType::CoTaskMemWideString => {
            unreachable!("CoTaskMem string elements are output-only")
        }
        ComType::StringArray { encoding, .. } => match encoding {
            StringEncoding::Wide => format!("DynCom.wideStringArray({variable})"),
            StringEncoding::Ansi => format!("DynCom.ansiStringArray({variable})"),
        },
        ComType::TypedBuffer { element } => match element.as_ref() {
            ComType::NativePod { layout } => format!(
                "DynCom.nativeStructBuffer({}, {variable})",
                native_pod_layout_js(layout)
            ),
            _ => format!("DynCom.buffer({variable})"),
        },
        ComType::OwningArray { element, interface } => match element.as_ref() {
            ComType::ManagedInterface { iid } => {
                if interface.is_some() {
                    format!(
                        "DynCom.interfaceArray(WinGuid.parse('{iid}'), {variable}.map(value => value._obj))"
                    )
                } else {
                    format!("DynCom.interfaceArray(WinGuid.parse('{iid}'), {variable})")
                }
            }
            ComType::Bstr => format!("DynCom.bstrArray({variable})"),
            ComType::Variant => format!("DynCom.variantArray({variable})"),
            ComType::CoTaskMemWideString => {
                unreachable!("CoTaskMem string elements are output-only")
            }
            _ => unreachable!("validated owning array element"),
        },
    }
}

pub(super) fn unwrap_result_js(result: &ProjectedComResult, expression: &str) -> String {
    match &result.conversion {
        ResultConversion::Bstr => format!("DynCom.takeBstr({expression})"),
        ResultConversion::CoTaskMemString(StringEncoding::Wide) => {
            format!("DynCom.takeCoTaskMemWideString({expression})")
        }
        ResultConversion::CoTaskMemString(StringEncoding::Ansi) => {
            format!("DynCom.takeCoTaskMemAnsiString({expression})")
        }
        ResultConversion::CoTaskMemData => {
            format!("DynCom.adoptCoTaskMemPointer({expression})")
        }
        ResultConversion::ManagedCom | ResultConversion::DynamicIidAdoption => {
            expression.to_string()
        }
        ResultConversion::HString => format!("{expression}.toString()"),
        ResultConversion::Value | ResultConversion::BorrowedHandle => {
            unwrap_value_js(&result.typ, expression)
        }
        ResultConversion::Buffer => format!("DynCom.takeBuffer({expression})"),
        ResultConversion::PlainArray => {
            let ComType::TypedBuffer { element } = &result.typ else {
                unreachable!("plain array result requires a typed-buffer result")
            };
            unwrap_array_result_js(element, None, expression)
        }
        ResultConversion::EnumeratorArray { interface } => {
            let (element, projected_interface) = projected_array_parts(&result.typ);
            unwrap_array_result_js(
                element,
                interface.as_ref().or(projected_interface),
                expression,
            )
        }
        ResultConversion::OwningArray { interface } => {
            let (element, projected_interface) = projected_array_parts(&result.typ);
            unwrap_array_result_js(
                element,
                interface.as_ref().or(projected_interface),
                expression,
            )
        }
        ResultConversion::Variant => format!("DynCom.takeVariant({expression})"),
        ResultConversion::SafeArray => format!("DynCom.takeSafeArray({expression})"),
        ResultConversion::PropVariant => format!("DynCom.takePropVariant({expression})"),
        ResultConversion::ExcepInfo => format!("DynCom.takeExcepInfo({expression})"),
    }
}

fn unwrap_value_js(typ: &ComType, expression: &str) -> String {
    match typ {
        ComType::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => format!("DynCom.toBool({expression})"),
            ComPrimitive::I8
            | ComPrimitive::U8
            | ComPrimitive::I16
            | ComPrimitive::U16
            | ComPrimitive::I32
            | ComPrimitive::Char16 => format!("DynCom.toNumber({expression})"),
            ComPrimitive::U32 => format!("DynCom.toU32({expression})"),
            ComPrimitive::I64 => format!("DynCom.toI64Bigint({expression})"),
            ComPrimitive::U64 => format!("DynCom.toU64Bigint({expression})"),
            ComPrimitive::F32 | ComPrimitive::F64 => {
                format!("DynCom.toF64({expression})")
            }
        },
        ComType::NativeIsize => format!("DynCom.toIsizeBigint({expression})"),
        ComType::NativeUsize => format!("DynCom.toUsizeBigint({expression})"),
        ComType::Win32Bool => format!("(DynCom.toNumber({expression}) !== 0)"),
        ComType::HResult => format!("DynCom.toNumber({expression})"),
        ComType::Guid => format!("DynCom.toGuidString({expression})"),
        ComType::HString => format!("{expression}.toString()"),
        ComType::Enum { underlying, .. } => unwrap_enum_js(*underlying, expression),
        ComType::ScalarAlias { underlying, .. } => unwrap_scalar_js(*underlying, expression),
        ComType::RawPointer | ComType::PointerAlias { .. } => {
            format!("DynCom.asPointerBigint({expression})")
        }
        ComType::Bstr => unreachable!("BSTR outputs require the BSTR result conversion"),
        ComType::NativePod { layout } | ComType::NativePodPointer { layout } => format!(
            "DynCom.nativeStructBytes({}, {expression})",
            native_pod_layout_js(layout)
        ),
        ComType::NativeUnionPointer { .. } => {
            unreachable!("native union outputs require an active-field contract")
        }
        ComType::Variant => format!("DynCom.takeVariant({expression})"),
        ComType::VariantByValue => unreachable!("by-value VARIANT is input-only"),
        ComType::SafeArray { .. } => format!("DynCom.takeSafeArray({expression})"),
        ComType::PropVariant => format!("DynCom.takePropVariant({expression})"),
        ComType::DispatchParams => unreachable!("DISPPARAMS is input-only"),
        ComType::ExcepInfo => format!("DynCom.takeExcepInfo({expression})"),
        ComType::ManagedInterface { .. } => expression.to_string(),
        ComType::CoTaskMemWideString => {
            unreachable!("CoTaskMem string elements are array-only")
        }
        ComType::StringArray { .. } => {
            unreachable!("string arrays are input-only")
        }
        ComType::TypedBuffer { .. } => format!("DynCom.takeBuffer({expression})"),
        ComType::OwningArray { .. } => {
            unreachable!("owning arrays require an explicit array result conversion")
        }
    }
}

fn projected_array_parts(
    typ: &ComType,
) -> (&ComType, Option<&super::super::ir::ProjectedInterfaceRef>) {
    match typ {
        ComType::TypedBuffer { element } => (element, None),
        ComType::OwningArray { element, interface } => (element, interface.as_ref()),
        _ => unreachable!("array result requires a projected array type"),
    }
}

fn unwrap_array_result_js(
    element: &ComType,
    interface: Option<&super::super::ir::ProjectedInterfaceRef>,
    expression: &str,
) -> String {
    if let Some(interface) = interface {
        return format!(
            "Array.from(DynCom.takeComArray({expression}), value => {{ try {{ return {}._fromNative(value); }} finally {{ value.release(); }} }})",
            interface.name
        );
    }
    match element {
        ComType::Bstr => format!("Array.from(DynCom.takeBstrArray({expression}))"),
        ComType::Variant => format!("Array.from(DynCom.takeVariantArray({expression}))"),
        ComType::CoTaskMemWideString => {
            format!("Array.from(DynCom.takeCoTaskMemWideStringArray({expression}))")
        }
        ComType::Guid => format!("Array.from(DynCom.takeGuidArray({expression}))"),
        ComType::NativePod { layout } => format!(
            "Array.from(DynCom.takeNativeStructArray({expression}, {}))",
            native_pod_layout_js(layout)
        ),
        ComType::ManagedInterface { .. } => {
            format!("Array.from(DynCom.takeComArray({expression}))")
        }
        element => format!(
            "Array.from(DynCom.{}({expression}))",
            scalar_array_take_method(element)
        ),
    }
}

fn scalar_array_take_method(typ: &ComType) -> &'static str {
    match typ {
        ComType::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => "takeBoolArray",
            ComPrimitive::I8 => "takeI8Array",
            ComPrimitive::U8 => "takeU8Array",
            ComPrimitive::I16 => "takeI16Array",
            ComPrimitive::U16 | ComPrimitive::Char16 => "takeU16Array",
            ComPrimitive::I32 => "takeI32Array",
            ComPrimitive::U32 => "takeU32Array",
            ComPrimitive::I64 => "takeI64Array",
            ComPrimitive::U64 => "takeU64Array",
            ComPrimitive::F32 => "takeF32Array",
            ComPrimitive::F64 => "takeF64Array",
        },
        ComType::NativeIsize => "takeIsizeArray",
        ComType::NativeUsize => "takeUsizeArray",
        ComType::Win32Bool => "takeWin32BoolArray",
        ComType::HResult => "takeI32Array",
        ComType::Enum { underlying, .. } => match underlying {
            ComEnumUnderlying::I8 => "takeI8Array",
            ComEnumUnderlying::U8 => "takeU8Array",
            ComEnumUnderlying::I16 => "takeI16Array",
            ComEnumUnderlying::U16 => "takeU16Array",
            ComEnumUnderlying::I32 => "takeI32Array",
            ComEnumUnderlying::U32 => "takeU32Array",
            ComEnumUnderlying::I64 => "takeI64Array",
            ComEnumUnderlying::U64 => "takeU64Array",
        },
        ComType::ScalarAlias { underlying, .. } => match underlying {
            ComScalarRepr::Primitive(primitive) => {
                scalar_array_take_method(&ComType::Primitive(*primitive))
            }
            ComScalarRepr::NativeIsize => "takeIsizeArray",
            ComScalarRepr::NativeUsize => "takeUsizeArray",
        },
        _ => unreachable!("validated plain array output element"),
    }
}

pub(super) fn native_pod_layout_js(layout: &NativePodLayout) -> String {
    format!("_nativeLayout_{}", layout.name)
}

pub(super) fn native_pod_descriptor_js(layout: &NativePodLayout) -> String {
    let descriptor = format!(
        "{{\"name\":\"{}.{}\",\"x86\":{},\"x64\":{},\"arm64\":{}}}",
        layout.namespace,
        layout.name,
        native_pod_architecture_json(&layout.x86),
        native_pod_architecture_json(&layout.x64),
        native_pod_architecture_json(&layout.arm64),
    );
    format!(
        "'{}'",
        descriptor.replace('\\', "\\\\").replace('\'', "\\'")
    )
}

pub(super) fn native_union_layout_js(layout: &NativeUnionLayout) -> String {
    format!("_nativeUnionLayout_{}", layout.name)
}

pub(super) fn native_union_descriptor_js(layout: &NativeUnionLayout) -> String {
    let descriptor = format!(
        "{{\"name\":\"{}.{}\",\"x86\":{},\"x64\":{},\"arm64\":{}}}",
        layout.namespace,
        layout.name,
        native_union_architecture_json(&layout.x86),
        native_union_architecture_json(&layout.x64),
        native_union_architecture_json(&layout.arm64),
    );
    format!(
        "'{}'",
        descriptor.replace('\\', "\\\\").replace('\'', "\\'")
    )
}

fn native_pod_architecture_json(layout: &NativePodArchitectureLayout) -> String {
    let fields = layout
        .fields
        .iter()
        .map(|field| {
            format!(
                "{{\"name\":\"{}\",\"offset\":{},\"count\":{},\"type\":{}}}",
                field.name,
                field.offset,
                field.count,
                native_pod_field_type_json(&field.typ),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"size\":{},\"alignment\":{},\"fields\":[{}]}}",
        layout.size, layout.alignment, fields
    )
}

fn native_pod_field_type_json(typ: &NativePodFieldType) -> String {
    match typ {
        NativePodFieldType::Scalar(scalar) => {
            format!("{{\"kind\":\"{}\"}}", native_pod_scalar_name(*scalar))
        }
        NativePodFieldType::Guid => "{\"kind\":\"guid\"}".into(),
        NativePodFieldType::Pointer => "{\"kind\":\"pointer\"}".into(),
        NativePodFieldType::Struct { name, layout } => format!(
            "{{\"kind\":\"struct\",\"name\":\"{name}\",\"layout\":{}}}",
            native_pod_architecture_json(layout)
        ),
    }
}

fn native_union_architecture_json(layout: &NativeUnionArchitectureLayout) -> String {
    let fields = layout
        .fields
        .iter()
        .map(|field| {
            format!(
                "{{\"name\":\"{}\",\"count\":{},\"type\":{}}}",
                field.name,
                field.count,
                native_union_field_type_json(&field.typ),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"size\":{},\"alignment\":{},\"fields\":[{}]}}",
        layout.size, layout.alignment, fields
    )
}

fn native_union_field_type_json(typ: &NativeUnionFieldType) -> String {
    match typ {
        NativeUnionFieldType::Scalar(scalar) => {
            format!("{{\"kind\":\"{}\"}}", native_pod_scalar_name(*scalar))
        }
        NativeUnionFieldType::Guid => "{\"kind\":\"guid\"}".into(),
        NativeUnionFieldType::Pointer => "{\"kind\":\"pointer\"}".into(),
        NativeUnionFieldType::Struct { name, layout } => format!(
            "{{\"kind\":\"struct\",\"name\":\"{name}\",\"layout\":{}}}",
            native_pod_architecture_json(layout)
        ),
    }
}

fn native_pod_scalar_name(scalar: NativePodScalar) -> &'static str {
    match scalar {
        NativePodScalar::I8 => "i8",
        NativePodScalar::U8 => "u8",
        NativePodScalar::I16 => "i16",
        NativePodScalar::U16 => "u16",
        NativePodScalar::I32 => "i32",
        NativePodScalar::U32 => "u32",
        NativePodScalar::I64 => "i64",
        NativePodScalar::U64 => "u64",
        NativePodScalar::F32 => "f32",
        NativePodScalar::F64 => "f64",
        NativePodScalar::NativeIsize => "isize",
        NativePodScalar::NativeUsize => "usize",
    }
}

fn enum_abi_type_js(underlying: ComEnumUnderlying) -> &'static str {
    match underlying {
        ComEnumUnderlying::I8 => "DynCom.i8Type()",
        ComEnumUnderlying::U8 => "DynCom.u8Type()",
        ComEnumUnderlying::I16 => "DynCom.i16Type()",
        ComEnumUnderlying::U16 => "DynCom.u16Type()",
        ComEnumUnderlying::I32 => "DynCom.i32Type()",
        ComEnumUnderlying::U32 => "DynCom.u32Type()",
        ComEnumUnderlying::I64 => "DynCom.i64Type()",
        ComEnumUnderlying::U64 => "DynCom.u64Type()",
    }
}

pub(super) fn scalar_type_dts(underlying: ComScalarRepr) -> &'static str {
    match underlying {
        ComScalarRepr::Primitive(ComPrimitive::Bool) => "boolean",
        ComScalarRepr::Primitive(ComPrimitive::I64 | ComPrimitive::U64)
        | ComScalarRepr::NativeIsize
        | ComScalarRepr::NativeUsize => "bigint",
        ComScalarRepr::Primitive(_) => "number",
    }
}

fn scalar_abi_type_js(underlying: ComScalarRepr) -> &'static str {
    match underlying {
        ComScalarRepr::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => "DynCom.boolType()",
            ComPrimitive::I8 => "DynCom.i8Type()",
            ComPrimitive::U8 => "DynCom.u8Type()",
            ComPrimitive::I16 => "DynCom.i16Type()",
            ComPrimitive::U16 => "DynCom.u16Type()",
            ComPrimitive::I32 => "DynCom.i32Type()",
            ComPrimitive::U32 => "DynCom.u32Type()",
            ComPrimitive::I64 => "DynCom.i64Type()",
            ComPrimitive::U64 => "DynCom.u64Type()",
            ComPrimitive::F32 => "DynCom.f32Type()",
            ComPrimitive::F64 => "DynCom.f64Type()",
            ComPrimitive::Char16 => "DynCom.char16Type()",
        },
        ComScalarRepr::NativeIsize => "DynCom.isizeType()",
        ComScalarRepr::NativeUsize => "DynCom.usizeType()",
    }
}

fn wrap_scalar_arg_js(underlying: ComScalarRepr, variable: &str) -> String {
    match underlying {
        ComScalarRepr::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => format!("DynCom.boolValue({variable})"),
            ComPrimitive::I8 => format!("DynCom.i8Value({variable})"),
            ComPrimitive::U8 => format!("DynCom.u8Value({variable})"),
            ComPrimitive::I16 => format!("DynCom.i16({variable})"),
            ComPrimitive::U16 => format!("DynCom.u16({variable})"),
            ComPrimitive::I32 => format!("DynCom.i32({variable})"),
            ComPrimitive::U32 => format!("DynCom.u32({variable})"),
            ComPrimitive::I64 => format!("DynCom.i64(BigInt({variable}))"),
            ComPrimitive::U64 => format!("DynCom.u64(BigInt({variable}))"),
            ComPrimitive::F32 => format!("DynCom.f32({variable})"),
            ComPrimitive::F64 => format!("DynCom.f64({variable})"),
            ComPrimitive::Char16 => format!("DynCom.char16({variable})"),
        },
        ComScalarRepr::NativeIsize => format!("DynCom.isize(BigInt({variable}))"),
        ComScalarRepr::NativeUsize => format!("DynCom.usize(BigInt({variable}))"),
    }
}

fn unwrap_scalar_js(underlying: ComScalarRepr, expression: &str) -> String {
    match underlying {
        ComScalarRepr::Primitive(primitive) => match primitive {
            ComPrimitive::Bool => format!("DynCom.toBool({expression})"),
            ComPrimitive::I8
            | ComPrimitive::U8
            | ComPrimitive::I16
            | ComPrimitive::U16
            | ComPrimitive::I32
            | ComPrimitive::Char16 => format!("DynCom.toNumber({expression})"),
            ComPrimitive::U32 => format!("DynCom.toU32({expression})"),
            ComPrimitive::I64 => format!("DynCom.toI64Bigint({expression})"),
            ComPrimitive::U64 => format!("DynCom.toU64Bigint({expression})"),
            ComPrimitive::F32 | ComPrimitive::F64 => {
                format!("DynCom.toF64({expression})")
            }
        },
        ComScalarRepr::NativeIsize => format!("DynCom.toIsizeBigint({expression})"),
        ComScalarRepr::NativeUsize => format!("DynCom.toUsizeBigint({expression})"),
    }
}

fn wrap_enum_arg_js(underlying: ComEnumUnderlying, variable: &str) -> String {
    match underlying {
        ComEnumUnderlying::I8 => format!("DynCom.i8Value({variable})"),
        ComEnumUnderlying::U8 => format!("DynCom.u8Value({variable})"),
        ComEnumUnderlying::I16 => format!("DynCom.i16({variable})"),
        ComEnumUnderlying::U16 => format!("DynCom.u16({variable})"),
        ComEnumUnderlying::I32 => format!("DynCom.i32({variable})"),
        ComEnumUnderlying::U32 => format!("DynCom.u32({variable})"),
        ComEnumUnderlying::I64 => format!("DynCom.i64(BigInt({variable}))"),
        ComEnumUnderlying::U64 => format!("DynCom.u64(BigInt({variable}))"),
    }
}

fn unwrap_enum_js(underlying: ComEnumUnderlying, expression: &str) -> String {
    match underlying {
        ComEnumUnderlying::I8
        | ComEnumUnderlying::U8
        | ComEnumUnderlying::I16
        | ComEnumUnderlying::U16
        | ComEnumUnderlying::I32 => format!("DynCom.toNumber({expression})"),
        ComEnumUnderlying::U32 => format!("DynCom.toU32({expression})"),
        ComEnumUnderlying::I64 => format!("DynCom.toI64Bigint({expression})"),
        ComEnumUnderlying::U64 => format!("DynCom.toU64Bigint({expression})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_layout() -> NativePodLayout {
        let architecture = NativePodArchitectureLayout {
            size: 8,
            alignment: 4,
            fields: vec![
                NativePodField {
                    name: "value".into(),
                    offset: 0,
                    count: 1,
                    typ: NativePodFieldType::Scalar(NativePodScalar::U32),
                },
                NativePodField {
                    name: "parts".into(),
                    offset: 4,
                    count: 2,
                    typ: NativePodFieldType::Scalar(NativePodScalar::U16),
                },
            ],
        };
        NativePodLayout {
            namespace: "Test".into(),
            name: "POD".into(),
            x86: architecture.clone(),
            x64: architecture.clone(),
            arm64: architecture,
        }
    }

    fn union_layout() -> NativeUnionLayout {
        let architecture = NativeUnionArchitectureLayout {
            size: 8,
            alignment: 8,
            fields: vec![
                NativeUnionField {
                    name: "integer".into(),
                    count: 1,
                    typ: NativeUnionFieldType::Scalar(NativePodScalar::U64),
                },
                NativeUnionField {
                    name: "pointer".into(),
                    count: 1,
                    typ: NativeUnionFieldType::Pointer,
                },
            ],
        };
        NativeUnionLayout {
            namespace: "Test".into(),
            name: "UNION".into(),
            x86: architecture.clone(),
            x64: architecture.clone(),
            arm64: architecture,
        }
    }

    #[test]
    fn native_pod_rendering_uses_only_validated_runtime_primitives() {
        let typ = ComType::NativePod {
            layout: pod_layout(),
        };
        assert!(abi_type_js(&typ).starts_with("DynCom.nativeStructType("));
        assert_eq!(type_dts(&typ), "POD");
        assert!(wrap_arg_js(&typ, "value").starts_with("DynCom.nativeStruct("));
        assert!(unwrap_value_js(&typ, "result").starts_with("DynCom.nativeStructBytes("));
        let descriptor = native_pod_descriptor_js(match &typ {
            ComType::NativePod { layout } => layout,
            _ => unreachable!(),
        });
        assert!(descriptor.contains("\"x86\":{\"size\":8"));
        assert!(descriptor.contains("\"count\":2"));

        let pointer = ComType::NativePodPointer {
            layout: pod_layout(),
        };
        assert!(abi_type_js(&pointer).starts_with("DynCom.nativeStructPointerType("));
        assert!(wrap_arg_js(&pointer, "value").starts_with("DynCom.nativeStruct("));
    }

    #[test]
    fn mappings_cover_every_supported_com_type() {
        let types = vec![
            ComType::Primitive(ComPrimitive::Bool),
            ComType::Primitive(ComPrimitive::I8),
            ComType::Primitive(ComPrimitive::U8),
            ComType::Primitive(ComPrimitive::I16),
            ComType::Primitive(ComPrimitive::U16),
            ComType::Primitive(ComPrimitive::I32),
            ComType::Primitive(ComPrimitive::U32),
            ComType::Primitive(ComPrimitive::I64),
            ComType::Primitive(ComPrimitive::U64),
            ComType::Primitive(ComPrimitive::F32),
            ComType::Primitive(ComPrimitive::F64),
            ComType::Primitive(ComPrimitive::Char16),
            ComType::NativeIsize,
            ComType::NativeUsize,
            ComType::Win32Bool,
            ComType::HResult,
            ComType::Guid,
            ComType::HString,
            ComType::Enum {
                namespace: "Tests".into(),
                name: "E".into(),
                underlying: ComEnumUnderlying::U32,
            },
            ComType::ScalarAlias {
                namespace: "Tests".into(),
                name: "COLORREF".into(),
                underlying: ComScalarRepr::Primitive(ComPrimitive::U32),
            },
            ComType::RawPointer,
            ComType::PointerAlias {
                namespace: "Tests".into(),
                name: "HWND".into(),
                kind: PointerAliasKind::HandleValue,
            },
            ComType::Bstr,
            ComType::NativeUnionPointer {
                layout: union_layout(),
            },
            ComType::Variant,
            ComType::SafeArray {
                element: SafeArrayElement::Bstr,
            },
            ComType::PropVariant,
            ComType::ManagedInterface {
                iid: "00000000-0000-0000-0000-000000000000".into(),
            },
        ];
        for typ in types {
            assert!(!abi_type_js(&typ).is_empty());
            assert!(!type_dts(&typ).is_empty());
            assert!(!wrap_arg_js(&typ, "value").is_empty());
        }
    }
}
