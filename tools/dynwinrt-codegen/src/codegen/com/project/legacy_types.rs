// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! TypeMeta compatibility mapping used only by diagnostic preflight and
//! synthetic renderer tests.

use crate::com_metadata::{is_native_isize, is_native_usize};
use crate::types::TypeMeta;

use super::super::ir::{
    ComEnumUnderlying, ComPrimitive, ComScalarRepr, ComType, PointerAliasKind, StringEncoding,
    UnsupportedComType,
};

pub(in crate::codegen::com) fn project_type(typ: &TypeMeta) -> Result<ComType, UnsupportedComType> {
    if is_native_isize(typ) {
        return Ok(ComType::NativeIsize);
    }
    if is_native_usize(typ) {
        return Ok(ComType::NativeUsize);
    }
    match typ {
        TypeMeta::Bool => Ok(ComType::Primitive(ComPrimitive::Bool)),
        TypeMeta::I8 => Ok(ComType::Primitive(ComPrimitive::I8)),
        TypeMeta::U8 => Ok(ComType::Primitive(ComPrimitive::U8)),
        TypeMeta::I16 => Ok(ComType::Primitive(ComPrimitive::I16)),
        TypeMeta::U16 => Ok(ComType::Primitive(ComPrimitive::U16)),
        TypeMeta::I32 => Ok(ComType::Primitive(ComPrimitive::I32)),
        TypeMeta::U32 => Ok(ComType::Primitive(ComPrimitive::U32)),
        TypeMeta::I64 => Ok(ComType::Primitive(ComPrimitive::I64)),
        TypeMeta::U64 => Ok(ComType::Primitive(ComPrimitive::U64)),
        TypeMeta::F32 => Ok(ComType::Primitive(ComPrimitive::F32)),
        TypeMeta::F64 => Ok(ComType::Primitive(ComPrimitive::F64)),
        TypeMeta::Char16 => Ok(ComType::Primitive(ComPrimitive::Char16)),
        TypeMeta::String => Ok(ComType::HString),
        TypeMeta::Guid => Ok(ComType::Guid),
        TypeMeta::Object => Ok(ComType::RawPointer),
        TypeMeta::Interface {
            namespace,
            name,
            iid,
        } if iid.is_empty() => Err(UnsupportedComType::UnresolvedInterface {
            namespace: namespace.clone(),
            name: name.clone(),
        }),
        TypeMeta::Interface { iid, .. } => Ok(ComType::ManagedInterface { iid: iid.clone() }),
        TypeMeta::RuntimeClass {
            default_interface: Some(default_interface),
            ..
        } => project_type(default_interface),
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface: None,
        } => Err(UnsupportedComType::UnresolvedRuntimeClass {
            namespace: namespace.clone(),
            name: name.clone(),
        }),
        TypeMeta::Delegate {
            namespace, name, ..
        } => Err(UnsupportedComType::Delegate {
            namespace: namespace.clone(),
            name: name.clone(),
        }),
        TypeMeta::Parameterized {
            namespace, name, ..
        } => Err(UnsupportedComType::ParameterizedInterface {
            namespace: namespace.clone(),
            name: name.clone(),
        }),
        TypeMeta::AsyncAction
        | TypeMeta::AsyncActionWithProgress(_)
        | TypeMeta::AsyncOperation(_)
        | TypeMeta::AsyncOperationWithProgress(_, _) => Err(UnsupportedComType::AsyncInterface),
        TypeMeta::Array(_) => Err(UnsupportedComType::Array),
        TypeMeta::Enum {
            namespace,
            name,
            underlying,
            ..
        } => Ok(ComType::Enum {
            namespace: namespace.clone(),
            name: name.clone(),
            underlying: project_enum_underlying(underlying)?,
        }),
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Win32.Foundation" && name == "BOOL" => Ok(ComType::Win32Bool),
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Win32.Foundation" && name == "HRESULT" => Ok(ComType::HResult),
        TypeMeta::Struct {
            namespace, name, ..
        } if namespace == "Windows.Win32.Foundation" && name == "BSTR" => Ok(ComType::Bstr),
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } if namespace.starts_with("Windows.Win32.")
            && fields.len() == 1
            && fields[0].name == "Value" =>
        {
            if let Some(underlying) = scalar_alias_underlying(name, &fields[0].typ) {
                Ok(ComType::ScalarAlias {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    underlying,
                })
            } else if matches!(fields[0].typ, TypeMeta::Object) {
                classify_pointer_alias(name)
                    .map(|kind| ComType::PointerAlias {
                        namespace: namespace.clone(),
                        name: name.clone(),
                        kind,
                    })
                    .ok_or_else(|| UnsupportedComType::UnknownPointerAlias {
                        namespace: namespace.clone(),
                        name: name.clone(),
                    })
            } else {
                Err(UnsupportedComType::NativeStructLayout {
                    namespace: namespace.clone(),
                    name: name.clone(),
                })
            }
        }
        TypeMeta::Struct {
            namespace, name, ..
        } => Err(UnsupportedComType::NativeStructLayout {
            namespace: namespace.clone(),
            name: name.clone(),
        }),
    }
}

pub(super) fn project_enum_underlying(
    typ: &TypeMeta,
) -> Result<ComEnumUnderlying, UnsupportedComType> {
    match typ {
        TypeMeta::I8 => Ok(ComEnumUnderlying::I8),
        TypeMeta::U8 => Ok(ComEnumUnderlying::U8),
        TypeMeta::I16 => Ok(ComEnumUnderlying::I16),
        TypeMeta::U16 => Ok(ComEnumUnderlying::U16),
        TypeMeta::I32 => Ok(ComEnumUnderlying::I32),
        TypeMeta::U32 => Ok(ComEnumUnderlying::U32),
        TypeMeta::I64 => Ok(ComEnumUnderlying::I64),
        TypeMeta::U64 => Ok(ComEnumUnderlying::U64),
        _ => Err(UnsupportedComType::Unknown),
    }
}

fn scalar_alias_underlying(name: &str, typ: &TypeMeta) -> Option<ComScalarRepr> {
    match name {
        "LPARAM" | "LRESULT" => return Some(ComScalarRepr::NativeIsize),
        "WPARAM" => return Some(ComScalarRepr::NativeUsize),
        _ => {}
    }
    let primitive = match typ {
        TypeMeta::Bool => ComPrimitive::Bool,
        TypeMeta::I8 => ComPrimitive::I8,
        TypeMeta::U8 => ComPrimitive::U8,
        TypeMeta::I16 => ComPrimitive::I16,
        TypeMeta::U16 => ComPrimitive::U16,
        TypeMeta::I32 => ComPrimitive::I32,
        TypeMeta::U32 => ComPrimitive::U32,
        TypeMeta::I64 => ComPrimitive::I64,
        TypeMeta::U64 => ComPrimitive::U64,
        TypeMeta::F32 => ComPrimitive::F32,
        TypeMeta::F64 => ComPrimitive::F64,
        TypeMeta::Char16 => ComPrimitive::Char16,
        _ => return None,
    };
    Some(ComScalarRepr::Primitive(primitive))
}

fn classify_pointer_alias(name: &str) -> Option<PointerAliasKind> {
    if matches!(
        name,
        "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR" | "PWCHAR" | "PCWCHAR" | "LPWCH" | "LPCWCH"
    ) {
        Some(PointerAliasKind::StringPointer(StringEncoding::Wide))
    } else if matches!(
        name,
        "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR" | "LPCH" | "LPCCH"
    ) {
        Some(PointerAliasKind::StringPointer(StringEncoding::Ansi))
    } else if matches!(
        name,
        "PSID"
            | "PSECURITY_DESCRIPTOR"
            | "MEMORY_MAPPED_VIEW_ADDRESS"
            | "LPPROC_THREAD_ATTRIBUTE_LIST"
            | "PVOID"
            | "PCVOID"
            | "LPVOID"
            | "LPCVOID"
    ) {
        Some(PointerAliasKind::DataPointer)
    } else if is_known_handle_alias(name) {
        Some(PointerAliasKind::HandleValue)
    } else {
        None
    }
}

fn is_known_handle_alias(name: &str) -> bool {
    matches!(
        name,
        "HANDLE"
            | "HWND"
            | "HACCEL"
            | "HBITMAP"
            | "HBRUSH"
            | "HCURSOR"
            | "HDC"
            | "HDESK"
            | "HDWP"
            | "HENHMETAFILE"
            | "HFILE"
            | "HFONT"
            | "HGDIOBJ"
            | "HGLOBAL"
            | "HHOOK"
            | "HICON"
            | "HIMAGELIST"
            | "HINSTANCE"
            | "HKEY"
            | "HKL"
            | "HLOCAL"
            | "HMENU"
            | "HMETAFILE"
            | "HMODULE"
            | "HMONITOR"
            | "HPALETTE"
            | "HPEN"
            | "HRAWINPUT"
            | "HRGN"
            | "HRSRC"
            | "HTHEME"
            | "HWINSTA"
            | "SC_HANDLE"
            | "SERVICE_STATUS_HANDLE"
            | "DPI_AWARENESS_CONTEXT"
    )
}

pub(super) fn is_scalar_in_out(typ: &ComType) -> bool {
    matches!(
        typ,
        ComType::Primitive(_)
            | ComType::NativeIsize
            | ComType::NativeUsize
            | ComType::Win32Bool
            | ComType::HResult
            | ComType::Enum { .. }
            | ComType::ScalarAlias { .. }
            | ComType::PointerAlias { .. }
    )
}

pub(super) fn is_supported_direct_return(typ: &ComType) -> bool {
    is_scalar_in_out(typ)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_type_category_projects_without_fallback() {
        let types = [
            TypeMeta::Bool,
            TypeMeta::I8,
            TypeMeta::U8,
            TypeMeta::I16,
            TypeMeta::U16,
            TypeMeta::I32,
            TypeMeta::U32,
            TypeMeta::I64,
            TypeMeta::U64,
            TypeMeta::F32,
            TypeMeta::F64,
            TypeMeta::Char16,
            TypeMeta::String,
            TypeMeta::Guid,
            TypeMeta::Object,
        ];
        for typ in types {
            assert!(project_type(&typ).is_ok(), "{typ:?}");
        }
    }

    #[test]
    fn reference_like_types_never_degrade_to_raw_pointer() {
        let parameterized = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IVector".into(),
            piid: String::new(),
            args: vec![TypeMeta::I32],
        };
        assert!(matches!(
            project_type(&parameterized),
            Err(UnsupportedComType::ParameterizedInterface { .. })
        ));
        assert!(matches!(
            project_type(&TypeMeta::AsyncAction),
            Err(UnsupportedComType::AsyncInterface)
        ));
    }

    #[test]
    fn transparent_scalar_typedefs_preserve_scalar_abi() {
        let colorref = TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "COLORREF".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::U32,
            }],
        };
        assert_eq!(
            project_type(&colorref),
            Ok(ComType::ScalarAlias {
                namespace: "Windows.Win32.Foundation".into(),
                name: "COLORREF".into(),
                underlying: ComScalarRepr::Primitive(ComPrimitive::U32),
            })
        );

        let lparam = TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "LPARAM".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::Object,
            }],
        };
        assert_eq!(
            project_type(&lparam),
            Ok(ComType::ScalarAlias {
                namespace: "Windows.Win32.Foundation".into(),
                name: "LPARAM".into(),
                underlying: ComScalarRepr::NativeIsize,
            })
        );
    }

    #[test]
    fn unknown_pointer_shaped_typedef_fails_closed() {
        let unknown = TypeMeta::Struct {
            namespace: "Windows.Win32.Foundation".into(),
            name: "MYSTERY_POINTER".into(),
            fields: vec![crate::types::FieldMeta {
                name: "Value".into(),
                typ: TypeMeta::Object,
            }],
        };
        assert!(matches!(
            project_type(&unknown),
            Err(UnsupportedComType::UnknownPointerAlias { .. })
        ));
    }
}
