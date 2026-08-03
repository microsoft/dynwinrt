// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::super::ir::{
    ComEnumUnderlying, ComPrimitive, ComScalarRepr, ComType, PointerAliasKind, ProjectedComResult,
    ResultConversion, StringEncoding,
};

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
        ComType::RawPointer | ComType::PointerAlias { .. } | ComType::Bstr => {
            "DynCom.pointerType()".into()
        }
        ComType::ManagedInterface { iid } => {
            format!("DynCom.interfaceType(WinGuid.parse('{iid}'))")
        }
    }
}

pub(super) fn input_type_dts(typ: &ComType) -> String {
    match typ {
        ComType::PointerAlias {
            name,
            kind: PointerAliasKind::HandleValue,
        } if name == "HWND" => format!("{name} | Buffer | Uint8Array"),
        ComType::PointerAlias {
            name,
            kind: PointerAliasKind::DataPointer,
        } => format!("{name} | Buffer | Uint8Array"),
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
        ComType::RawPointer => "bigint | Buffer".into(),
        ComType::PointerAlias { name, .. } => name.clone(),
        ComType::Bstr => "BSTR".into(),
        ComType::ManagedInterface { .. } => "DynWinRtValue".into(),
    }
}

pub(super) fn result_type_dts(result: &ProjectedComResult) -> String {
    match result.conversion {
        ResultConversion::Bstr | ResultConversion::CoTaskMemString(_) => "string".into(),
        ResultConversion::CoTaskMemData
        | ResultConversion::ManagedCom
        | ResultConversion::DynamicIidAdoption => "DynWinRtValue".into(),
        ResultConversion::Value | ResultConversion::HString => type_dts(&result.typ),
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
        ComType::RawPointer | ComType::Bstr => format!("DynCom.pointer({variable})"),
        ComType::PointerAlias {
            name,
            kind: PointerAliasKind::HandleValue,
        } if name == "HWND" => {
            format!("DynCom.pointer(DynCom.handleValue({variable}))")
        }
        ComType::PointerAlias {
            kind: PointerAliasKind::StringPointer(StringEncoding::Wide),
            ..
        } => format!("DynCom.wideStringPointer({variable})"),
        ComType::PointerAlias {
            kind: PointerAliasKind::StringPointer(StringEncoding::Ansi),
            ..
        } => format!("DynCom.ansiStringPointer({variable})"),
        ComType::PointerAlias { .. } => format!("DynCom.pointer({variable})"),
        ComType::ManagedInterface { .. } => variable.to_string(),
    }
}

pub(super) fn unwrap_result_js(result: &ProjectedComResult, expression: &str) -> String {
    match result.conversion {
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
        ResultConversion::Value => unwrap_value_js(&result.typ, expression),
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
        ComType::RawPointer | ComType::PointerAlias { .. } | ComType::Bstr => {
            format!("DynCom.asPointerBigint({expression})")
        }
        ComType::ManagedInterface { .. } => expression.to_string(),
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
                name: "E".into(),
                underlying: ComEnumUnderlying::U32,
            },
            ComType::ScalarAlias {
                name: "COLORREF".into(),
                underlying: ComScalarRepr::Primitive(ComPrimitive::U32),
            },
            ComType::RawPointer,
            ComType::PointerAlias {
                name: "HWND".into(),
                kind: PointerAliasKind::HandleValue,
            },
            ComType::Bstr,
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
