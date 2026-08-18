// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Raw metadata facts for flat Win32 `[DllImport]` exports.
//!
//! This module preserves native facts only. Language projection and ABI
//! support decisions belong to `codegen::win32`.

use std::collections::HashSet;

use windows_metadata::{HasAttributes, reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawScalar {
    Bool8,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Char16,
    Bool32,
    NativeIsize,
    NativeUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawConstness {
    Const,
    Mutable,
    Unspecified,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDirection {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStringEncoding {
    Utf16,
    Ansi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCallingConvention {
    System,
    Cdecl,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStatusSemantics {
    None,
    ZeroIsSuccess,
    SignedNonNegativeIsSuccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawArchitectures {
    pub x86: bool,
    pub x64: bool,
    pub arm64: bool,
}

impl RawArchitectures {
    fn all() -> Self {
        Self {
            x86: true,
            x64: true,
            arm64: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEnumMember {
    pub name: String,
    pub value: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawNamedKind {
    Enum {
        underlying: RawScalar,
        members: Vec<RawEnumMember>,
        is_flags: bool,
    },
    Handle {
        cleanup: Option<String>,
    },
    StringPointer {
        encoding: RawStringEncoding,
    },
    DataPointer,
    FunctionPointer,
    Guid,
    ComInterface {
        iid: String,
    },
    NativeStruct {
        layout: Box<RawNativeLayoutSet>,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLayoutKind {
    Sequential,
    Union,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPacking {
    Default,
    Explicit(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeField {
    pub name: String,
    pub typ: RawType,
    pub fixed_count: Option<usize>,
    pub bitfield: bool,
    pub flexible_array: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeLayout {
    pub architectures: RawArchitectures,
    pub kind: RawLayoutKind,
    pub packing: RawPacking,
    pub declared_size: Option<usize>,
    pub forced_alignment: Option<usize>,
    pub fields: Vec<RawNativeField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeLayoutSet {
    pub recursive: bool,
    pub variants: Vec<RawNativeLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBaseType {
    Void,
    Scalar(RawScalar),
    Named {
        namespace: String,
        name: String,
        kind: RawNamedKind,
    },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawType {
    pub base: RawBaseType,
    pub pointer_depth: u8,
    pub constness: RawConstness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBufferSize {
    ElementCountParam(usize),
    ByteCountParam(usize),
    Constant(usize),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBuffer {
    pub element: RawType,
    pub size: RawBufferSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawParameter {
    pub name: String,
    pub typ: RawType,
    pub direction: RawDirection,
    pub nullable: bool,
    pub reserved: bool,
    pub null_null_terminated: bool,
    pub buffer: Option<RawBuffer>,
    pub free_with: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFunction {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub return_type: RawType,
    pub parameters: Vec<RawParameter>,
    pub return_status: RawStatusSemantics,
    pub return_free_with: Option<String>,
    pub supports_last_error: bool,
    pub calling_convention: RawCallingConvention,
    pub architectures: RawArchitectures,
    pub variadic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawApis {
    pub namespace: String,
    pub class_name: String,
    pub functions: Vec<RawFunction>,
}

pub fn parse_apis(winmd_paths: &str, namespace: &str, class_name: &str) -> Option<RawApis> {
    let index = crate::meta::load_index(winmd_paths)?;
    let definition = index.get(namespace, class_name).next()?;
    let functions = definition
        .methods()
        .filter_map(|method| parse_function(&index, namespace, class_name, &method))
        .collect::<Vec<_>>();
    (!functions.is_empty()).then(|| RawApis {
        namespace: namespace.to_string(),
        class_name: class_name.to_string(),
        functions,
    })
}

pub fn parse_all_functions(winmd_paths: &str) -> Option<Vec<RawFunction>> {
    let index = crate::meta::load_index(winmd_paths)?;
    let mut functions = Vec::new();
    for definition in index.all() {
        let namespace = definition.namespace().to_string();
        let container = definition.name().to_string();
        functions.extend(
            definition
                .methods()
                .filter_map(|method| parse_function(&index, &namespace, &container, &method)),
        );
    }
    Some(functions)
}

fn parse_function(
    index: &reader::Index,
    namespace: &str,
    container: &str,
    method: &reader::MethodDef,
) -> Option<RawFunction> {
    let import = method.impl_map()?;
    if matches!(method.name(), ".ctor" | ".cctor") {
        return None;
    }
    let signature = method.signature(&[]);
    let (return_definition, definitions) = match params_by_sequence(method, signature.types.len()) {
        Ok(definitions) => definitions,
        Err(reason) => {
            return Some(invalid_function(
                namespace,
                container,
                method.name(),
                import.import_scope().name(),
                import.import_name(),
                &reason,
            ));
        }
    };

    let return_type = map_type(index, &signature.return_type);
    let mut parameters = Vec::with_capacity(definitions.len());
    for (position, (definition, typ)) in definitions.iter().zip(&signature.types).enumerate() {
        let mapped = map_type(index, typ);
        let (name, direction, nullable, reserved, null_null_terminated, buffer, free_with) =
            match definition {
                Some(definition) => {
                    let flags = definition.flags();
                    (
                        definition.name().to_string(),
                        param_direction(flags),
                        flags.contains(windows_metadata::ParamAttributes::Optional)
                            || definition.has_attribute("OptionalAttribute"),
                        definition.has_attribute("ReservedAttribute"),
                        definition.has_attribute("NullNullTerminatedAttribute"),
                        buffer_size(definition).map(|size| RawBuffer {
                            element: buffer_element(&mapped),
                            size,
                        }),
                        free_with(definition),
                    )
                }
                None => (
                    format!("arg{position}"),
                    RawDirection::In,
                    false,
                    false,
                    false,
                    None,
                    None,
                ),
            };
        parameters.push(RawParameter {
            name,
            typ: mapped,
            direction,
            nullable,
            reserved,
            null_null_terminated,
            buffer,
            free_with,
        });
    }
    apply_known_buffer_contracts(namespace, method.name(), &mut parameters);

    let flags = import.flags();
    Some(RawFunction {
        namespace: namespace.to_string(),
        container: container.to_string(),
        name: method.name().to_string(),
        dll: import.import_scope().name().to_string(),
        entry_point: import.import_name().to_string(),
        return_status: status_semantics(&signature.return_type, &return_type),
        return_free_with: return_definition.as_ref().and_then(free_with),
        return_type,
        parameters,
        supports_last_error: flags.contains(windows_metadata::PInvokeAttributes::SupportsLastError),
        calling_convention: calling_convention(method),
        architectures: architectures(method),
        variadic: signature
            .flags
            .contains(windows_metadata::MethodCallAttributes::VARARG),
    })
}

fn invalid_function(
    namespace: &str,
    container: &str,
    name: &str,
    dll: &str,
    entry_point: &str,
    reason: &str,
) -> RawFunction {
    RawFunction {
        namespace: namespace.to_string(),
        container: container.to_string(),
        name: name.to_string(),
        dll: dll.to_string(),
        entry_point: entry_point.to_string(),
        return_type: RawType {
            base: RawBaseType::Unknown(reason.to_string()),
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        },
        parameters: Vec::new(),
        return_status: RawStatusSemantics::None,
        return_free_with: None,
        supports_last_error: false,
        calling_convention: RawCallingConvention::Unsupported,
        architectures: RawArchitectures {
            x86: false,
            x64: false,
            arm64: false,
        },
        variadic: false,
    }
}

fn params_by_sequence<'a>(
    method: &'a reader::MethodDef<'a>,
    parameter_count: usize,
) -> Result<
    (
        Option<reader::MethodParam<'a>>,
        Vec<Option<reader::MethodParam<'a>>>,
    ),
    String,
> {
    let mut parameters = vec![None; parameter_count];
    let mut return_parameter = None;
    for parameter in method.params() {
        let sequence = parameter.sequence();
        if sequence == 0 {
            if return_parameter.replace(parameter).is_some() {
                return Err("duplicate return parameter sequence 0".into());
            }
            continue;
        }
        let position = sequence as usize - 1;
        let Some(slot) = parameters.get_mut(position) else {
            return Err(format!(
                "parameter sequence {sequence} exceeds signature arity {parameter_count}"
            ));
        };
        if slot.replace(parameter).is_some() {
            return Err(format!("duplicate parameter sequence {sequence}"));
        }
    }
    Ok((return_parameter, parameters))
}

fn param_direction(flags: windows_metadata::ParamAttributes) -> RawDirection {
    match (
        flags.contains(windows_metadata::ParamAttributes::In),
        flags.contains(windows_metadata::ParamAttributes::Out),
    ) {
        (true, true) => RawDirection::InOut,
        (_, true) => RawDirection::Out,
        _ => RawDirection::In,
    }
}

fn map_type(index: &reader::Index, typ: &windows_metadata::Type) -> RawType {
    use windows_metadata::Type;

    match typ {
        Type::Void => raw_base(RawBaseType::Void),
        Type::Bool => raw_scalar(RawScalar::Bool8),
        Type::Char => raw_scalar(RawScalar::Char16),
        Type::I8 => raw_scalar(RawScalar::I8),
        Type::U8 => raw_scalar(RawScalar::U8),
        Type::I16 => raw_scalar(RawScalar::I16),
        Type::U16 => raw_scalar(RawScalar::U16),
        Type::I32 => raw_scalar(RawScalar::I32),
        Type::U32 => raw_scalar(RawScalar::U32),
        Type::I64 => raw_scalar(RawScalar::I64),
        Type::U64 => raw_scalar(RawScalar::U64),
        Type::F32 => raw_scalar(RawScalar::F32),
        Type::F64 => raw_scalar(RawScalar::F64),
        Type::ISize => raw_scalar(RawScalar::NativeIsize),
        Type::USize => raw_scalar(RawScalar::NativeUsize),
        Type::PtrMut(inner, depth) => map_pointer(index, inner, *depth, RawConstness::Mutable),
        Type::PtrConst(inner, depth) => map_pointer(index, inner, *depth, RawConstness::Const),
        Type::Name(name) => map_named(index, &name.namespace, &name.name, &mut Vec::new()),
        other => RawType {
            base: RawBaseType::Unknown(format!("{other:?}")),
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        },
    }
}

fn raw_base(base: RawBaseType) -> RawType {
    RawType {
        base,
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    }
}

fn raw_scalar(scalar: RawScalar) -> RawType {
    raw_base(RawBaseType::Scalar(scalar))
}

fn map_pointer(
    index: &reader::Index,
    inner: &windows_metadata::Type,
    depth: usize,
    outer_constness: RawConstness,
) -> RawType {
    let mut mapped = map_type(index, inner);
    let Ok(depth) = u8::try_from(depth) else {
        mapped.base = RawBaseType::Unknown("pointer depth exceeds u8".into());
        return mapped;
    };
    mapped.pointer_depth = match mapped.pointer_depth.checked_add(depth) {
        Some(depth) => depth,
        None => {
            mapped.base = RawBaseType::Unknown("pointer depth overflow".into());
            return mapped;
        }
    };
    mapped.constness = match (mapped.constness, outer_constness) {
        (RawConstness::Unspecified, value) => value,
        (value, RawConstness::Unspecified) => value,
        (left, right) if left == right => left,
        _ => RawConstness::Mixed,
    };
    mapped
}

fn map_named(
    index: &reader::Index,
    namespace: &str,
    name: &str,
    layout_stack: &mut Vec<(String, String)>,
) -> RawType {
    if (namespace == "System" && name == "Guid")
        || (namespace == "Windows.Win32.Foundation" && name == "GUID")
    {
        return named(namespace, name, RawNamedKind::Guid);
    }
    if namespace == "Windows.Win32.Foundation" {
        match name {
            "BOOL" => return raw_scalar(RawScalar::Bool32),
            "BOOLEAN" => return raw_scalar(RawScalar::U8),
            "HRESULT" | "NTSTATUS" | "LSTATUS" => return raw_scalar(RawScalar::I32),
            "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR" => {
                return named_string(
                    namespace,
                    name,
                    RawStringEncoding::Utf16,
                    matches!(name, "PCWSTR" | "LPCWSTR"),
                );
            }
            "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR" => {
                return named_string(
                    namespace,
                    name,
                    RawStringEncoding::Ansi,
                    matches!(name, "PCSTR" | "LPCSTR"),
                );
            }
            "FARPROC" | "PROC" | "NEARPROC" => {
                return named(namespace, name, RawNamedKind::FunctionPointer);
            }
            "BSTR" => {
                return named(namespace, name, RawNamedKind::Unknown);
            }
            _ => {}
        }
    }

    if is_data_pointer_alias(name) {
        return named(namespace, name, RawNamedKind::DataPointer);
    }
    if let Some(cleanup) = handle_cleanup(index, namespace, name) {
        return named(
            namespace,
            name,
            RawNamedKind::Handle {
                cleanup: Some(cleanup),
            },
        );
    }
    if is_handle_alias(name) {
        return named(
            namespace,
            name,
            RawNamedKind::Handle {
                cleanup: handle_cleanup(index, namespace, name),
            },
        );
    }

    let Some(definition) = index.get(namespace, name).next() else {
        return named(namespace, name, RawNamedKind::Unknown);
    };
    let iid = crate::meta::extract_iid(&definition);
    if definition.extends().is_none() && !iid.is_empty() {
        return named(namespace, name, RawNamedKind::ComInterface { iid });
    }
    let Some(extends) = definition.extends() else {
        return named(namespace, name, RawNamedKind::Unknown);
    };
    if extends.namespace() == "System" && extends.name() == "Enum" {
        let (underlying, members) = parse_enum(&definition);
        return named(
            namespace,
            name,
            RawNamedKind::Enum {
                underlying,
                members,
                is_flags: definition.has_attribute("FlagsAttribute"),
            },
        );
    }
    if extends.namespace() == "System" && matches!(extends.name(), "Delegate" | "MulticastDelegate")
    {
        return named(namespace, name, RawNamedKind::FunctionPointer);
    }
    if extends.namespace() == "System" && extends.name() == "ValueType" {
        if definition.has_attribute("NativeTypedefAttribute") {
            let fields = definition.fields().collect::<Vec<_>>();
            if fields.len() == 1 && fields[0].name() == "Value" {
                return map_transparent_typedef(index, namespace, name, &fields[0].ty());
            }
        }
        return named(
            namespace,
            name,
            RawNamedKind::NativeStruct {
                layout: Box::new(parse_native_layouts(index, namespace, name, layout_stack)),
            },
        );
    }

    fn parse_native_layouts(
        index: &reader::Index,
        namespace: &str,
        name: &str,
        layout_stack: &mut Vec<(String, String)>,
    ) -> RawNativeLayoutSet {
        let key = (namespace.to_string(), name.to_string());
        if layout_stack.contains(&key) {
            return RawNativeLayoutSet {
                recursive: true,
                variants: Vec::new(),
            };
        }
        layout_stack.push(key);
        let variants = index
            .get(namespace, name)
            .filter(|definition| {
                definition
                    .extends()
                    .is_some_and(|base| base.namespace() == "System" && base.name() == "ValueType")
            })
            .map(|definition| {
                let flags = definition.flags();
                let kind = if flags.contains(windows_metadata::TypeAttributes::SequentialLayout) {
                    RawLayoutKind::Sequential
                } else if flags.contains(windows_metadata::TypeAttributes::ExplicitLayout) {
                    RawLayoutKind::Union
                } else {
                    RawLayoutKind::Unknown
                };
                let (packing, declared_size) =
                    definition
                        .class_layout()
                        .map_or((RawPacking::Default, None), |layout| {
                            (
                                if layout.packing_size() == 0 {
                                    RawPacking::Default
                                } else {
                                    RawPacking::Explicit(layout.packing_size())
                                },
                                (layout.class_size() != 0).then_some(layout.class_size() as usize),
                            )
                        });
                let fields = definition
                    .fields()
                    .filter(|field| field.name() != "value__" && field.constant().is_none())
                    .map(|field| {
                        let field_type = field.ty();
                        let (typ, fixed_count) = match field_type {
                            windows_metadata::Type::ArrayFixed(ref element, count) => (
                                map_layout_field_type(
                                    index,
                                    element,
                                    Some(definition),
                                    layout_stack,
                                ),
                                Some(count),
                            ),
                            _ => (
                                map_layout_field_type(
                                    index,
                                    &field_type,
                                    Some(definition),
                                    layout_stack,
                                ),
                                None,
                            ),
                        };
                        let typ = known_native_field_type(&definition, field.name(), typ);
                        RawNativeField {
                            name: field.name().to_string(),
                            typ,
                            fixed_count,
                            bitfield: field.has_attribute("NativeBitfieldAttribute"),
                            flexible_array: field.has_attribute("FlexibleArrayAttribute"),
                        }
                    })
                    .collect();
                RawNativeLayout {
                    architectures: architectures(&definition),
                    kind,
                    packing,
                    declared_size,
                    forced_alignment: definition.find_attribute("AlignmentAttribute").and_then(
                        |attribute| match attribute.value().first() {
                            Some((_, windows_metadata::Value::I32(value))) if *value > 0 => {
                                Some(*value as usize)
                            }
                            _ => None,
                        },
                    ),
                    fields,
                }
            })
            .collect();
        layout_stack.pop();
        RawNativeLayoutSet {
            recursive: false,
            variants,
        }
    }

    fn known_native_field_type(
        enclosing: &reader::TypeDef,
        field: &str,
        mapped: RawType,
    ) -> RawType {
        match (enclosing.namespace(), enclosing.name(), field) {
            ("Windows.Win32.Security", "SECURITY_ATTRIBUTES", "lpSecurityDescriptor")
                if mapped.pointer_depth == 1 && matches!(mapped.base, RawBaseType::Void) =>
            {
                named(
                    enclosing.namespace(),
                    "SECURITY_ATTRIBUTES.lpSecurityDescriptor",
                    RawNamedKind::DataPointer,
                )
            }
            ("Windows.Win32.System.Threading", "PROCESS_INFORMATION", "hProcess" | "hThread") => {
                named(
                    enclosing.namespace(),
                    &format!("PROCESS_INFORMATION.{field}"),
                    RawNamedKind::Handle {
                        cleanup: Some("CloseHandle".into()),
                    },
                )
            }
            (
                "Windows.Win32.System.Threading",
                "STARTUPINFOA" | "STARTUPINFOW",
                "lpReserved" | "lpDesktop" | "lpTitle" | "lpReserved2",
            ) if mapped.pointer_depth > 0
                || matches!(
                    mapped.base,
                    RawBaseType::Named {
                        kind: RawNamedKind::StringPointer { .. } | RawNamedKind::DataPointer,
                        ..
                    }
                ) =>
            {
                named(
                    enclosing.namespace(),
                    &format!("{}.{field}", enclosing.name()),
                    RawNamedKind::DataPointer,
                )
            }
            (
                "Windows.Win32.System.Threading",
                "STARTUPINFOA" | "STARTUPINFOW",
                "hStdInput" | "hStdOutput" | "hStdError",
            ) => named(
                enclosing.namespace(),
                &format!("{}.{field}", enclosing.name()),
                RawNamedKind::Handle { cleanup: None },
            ),
            _ => mapped,
        }
    }

    fn map_layout_field_type<'a>(
        index: &'a reader::Index,
        typ: &windows_metadata::Type,
        enclosing: Option<reader::TypeDef<'a>>,
        layout_stack: &mut Vec<(String, String)>,
    ) -> RawType {
        match typ {
            windows_metadata::Type::Name(name) => {
                if let Some(enclosing) = enclosing
                    && let Some(nested) = index
                        .nested(enclosing)
                        .find(|nested| nested.name() == name.name)
                {
                    return map_nested_layout(index, enclosing, nested, layout_stack);
                }
                if let Some(enclosing) = enclosing
                    && let Some(known) =
                        known_anonymous_layout(index, enclosing, &name.name, layout_stack)
                {
                    return known;
                }
                map_named(index, &name.namespace, &name.name, layout_stack)
            }
            windows_metadata::Type::PtrMut(inner, depth) => {
                let mut mapped = map_layout_field_type(index, inner, enclosing, layout_stack);
                mapped.pointer_depth = mapped
                    .pointer_depth
                    .saturating_add(u8::try_from(*depth).unwrap_or(u8::MAX));
                mapped.constness = RawConstness::Mutable;
                mapped
            }
            windows_metadata::Type::PtrConst(inner, depth) => {
                let mut mapped = map_layout_field_type(index, inner, enclosing, layout_stack);
                mapped.pointer_depth = mapped
                    .pointer_depth
                    .saturating_add(u8::try_from(*depth).unwrap_or(u8::MAX));
                mapped.constness = RawConstness::Const;
                mapped
            }
            windows_metadata::Type::Void => raw_base(RawBaseType::Void),
            windows_metadata::Type::Bool => raw_scalar(RawScalar::Bool8),
            windows_metadata::Type::Char => raw_scalar(RawScalar::Char16),
            windows_metadata::Type::I8 => raw_scalar(RawScalar::I8),
            windows_metadata::Type::U8 => raw_scalar(RawScalar::U8),
            windows_metadata::Type::I16 => raw_scalar(RawScalar::I16),
            windows_metadata::Type::U16 => raw_scalar(RawScalar::U16),
            windows_metadata::Type::I32 => raw_scalar(RawScalar::I32),
            windows_metadata::Type::U32 => raw_scalar(RawScalar::U32),
            windows_metadata::Type::I64 => raw_scalar(RawScalar::I64),
            windows_metadata::Type::U64 => raw_scalar(RawScalar::U64),
            windows_metadata::Type::F32 => raw_scalar(RawScalar::F32),
            windows_metadata::Type::F64 => raw_scalar(RawScalar::F64),
            windows_metadata::Type::ISize => raw_scalar(RawScalar::NativeIsize),
            windows_metadata::Type::USize => raw_scalar(RawScalar::NativeUsize),
            other => RawType {
                base: RawBaseType::Unknown(format!("{other:?}")),
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
        }
    }

    fn known_anonymous_layout(
        index: &reader::Index,
        enclosing: reader::TypeDef,
        name: &str,
        layout_stack: &mut Vec<(String, String)>,
    ) -> Option<RawType> {
        if name != "_Anonymous_e__Union" {
            return None;
        }
        let namespace = enclosing.namespace();
        let identity = format!("{}+{name}", enclosing.name());
        let field = |name: &str, typ: RawType| RawNativeField {
            name: name.into(),
            typ,
            fixed_count: None,
            bitfield: false,
            flexible_array: false,
        };
        let fields = match (namespace, enclosing.name()) {
            ("Windows.Win32.System.SystemInformation", "SYSTEM_INFO") => {
                let words = named(
                    namespace,
                    "SYSTEM_INFO+_Anonymous_e__Union+_Anonymous_e__Struct",
                    RawNamedKind::NativeStruct {
                        layout: Box::new(RawNativeLayoutSet {
                            recursive: false,
                            variants: vec![RawNativeLayout {
                                architectures: architectures(&enclosing),
                                kind: RawLayoutKind::Sequential,
                                packing: RawPacking::Default,
                                declared_size: None,
                                forced_alignment: None,
                                fields: vec![
                                    field(
                                        "wProcessorArchitecture",
                                        map_named(
                                            index,
                                            namespace,
                                            "PROCESSOR_ARCHITECTURE",
                                            layout_stack,
                                        ),
                                    ),
                                    field("wReserved", raw_scalar(RawScalar::U16)),
                                ],
                            }],
                        }),
                    },
                );
                vec![
                    field("dwOemId", raw_scalar(RawScalar::U32)),
                    field("Anonymous", words),
                ]
            }
            ("Windows.Win32.UI.Input.KeyboardAndMouse", "INPUT") => vec![
                field(
                    "mi",
                    map_named(index, namespace, "MOUSEINPUT", layout_stack),
                ),
                field(
                    "ki",
                    map_named(index, namespace, "KEYBDINPUT", layout_stack),
                ),
                field(
                    "hi",
                    map_named(index, namespace, "HARDWAREINPUT", layout_stack),
                ),
            ],
            _ => return None,
        };
        Some(named(
            namespace,
            &identity,
            RawNamedKind::NativeStruct {
                layout: Box::new(RawNativeLayoutSet {
                    recursive: false,
                    variants: vec![RawNativeLayout {
                        architectures: architectures(&enclosing),
                        kind: RawLayoutKind::Union,
                        packing: RawPacking::Default,
                        declared_size: None,
                        forced_alignment: None,
                        fields,
                    }],
                }),
            },
        ))
    }

    fn map_nested_layout<'a>(
        index: &'a reader::Index,
        enclosing: reader::TypeDef<'a>,
        nested: reader::TypeDef<'a>,
        layout_stack: &mut Vec<(String, String)>,
    ) -> RawType {
        let namespace = enclosing.namespace();
        let identity = format!("{}+{}", enclosing.name(), nested.name());
        let key = (namespace.to_string(), identity.clone());
        if layout_stack.contains(&key) {
            return named(
                namespace,
                &identity,
                RawNamedKind::NativeStruct {
                    layout: Box::new(RawNativeLayoutSet {
                        recursive: true,
                        variants: Vec::new(),
                    }),
                },
            );
        }
        layout_stack.push(key);
        let flags = nested.flags();
        let kind = if flags.contains(windows_metadata::TypeAttributes::SequentialLayout) {
            RawLayoutKind::Sequential
        } else if flags.contains(windows_metadata::TypeAttributes::ExplicitLayout) {
            RawLayoutKind::Union
        } else {
            RawLayoutKind::Unknown
        };
        let (packing, declared_size) =
            nested
                .class_layout()
                .map_or((RawPacking::Default, None), |layout| {
                    (
                        if layout.packing_size() == 0 {
                            RawPacking::Default
                        } else {
                            RawPacking::Explicit(layout.packing_size())
                        },
                        (layout.class_size() != 0).then_some(layout.class_size() as usize),
                    )
                });
        let fields = nested
            .fields()
            .filter(|field| field.name() != "value__" && field.constant().is_none())
            .map(|field| {
                let field_type = field.ty();
                let (typ, fixed_count) = match field_type {
                    windows_metadata::Type::ArrayFixed(ref element, count) => (
                        map_layout_field_type(index, element, Some(nested), layout_stack),
                        Some(count),
                    ),
                    _ => (
                        map_layout_field_type(index, &field_type, Some(nested), layout_stack),
                        None,
                    ),
                };
                RawNativeField {
                    name: field.name().to_string(),
                    typ,
                    fixed_count,
                    bitfield: field.has_attribute("NativeBitfieldAttribute"),
                    flexible_array: field.has_attribute("FlexibleArrayAttribute"),
                }
            })
            .collect();
        let layout = RawNativeLayout {
            architectures: architectures(&nested),
            kind,
            packing,
            declared_size,
            forced_alignment: nested
                .find_attribute("AlignmentAttribute")
                .and_then(|attribute| match attribute.value().first() {
                    Some((_, windows_metadata::Value::I32(value))) if *value > 0 => {
                        Some(*value as usize)
                    }
                    _ => None,
                }),
            fields,
        };
        layout_stack.pop();
        named(
            namespace,
            &identity,
            RawNamedKind::NativeStruct {
                layout: Box::new(RawNativeLayoutSet {
                    recursive: false,
                    variants: vec![layout],
                }),
            },
        )
    }
    named(namespace, name, RawNamedKind::Unknown)
}

fn map_transparent_typedef(
    index: &reader::Index,
    namespace: &str,
    name: &str,
    typ: &windows_metadata::Type,
) -> RawType {
    let mapped = map_type(index, typ);
    match mapped.base {
        RawBaseType::Scalar(scalar) if mapped.pointer_depth == 0 => raw_scalar(scalar),
        RawBaseType::Named {
            kind: RawNamedKind::StringPointer { encoding },
            ..
        } => {
            let mut value = named(namespace, name, RawNamedKind::StringPointer { encoding });
            value.constness = mapped.constness;
            value
        }
        _ => named(namespace, name, RawNamedKind::Unknown),
    }
}

fn named(namespace: &str, name: &str, kind: RawNamedKind) -> RawType {
    RawType {
        base: RawBaseType::Named {
            namespace: namespace.to_string(),
            name: name.to_string(),
            kind,
        },
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    }
}

fn named_string(
    namespace: &str,
    name: &str,
    encoding: RawStringEncoding,
    is_const: bool,
) -> RawType {
    let mut value = named(namespace, name, RawNamedKind::StringPointer { encoding });
    value.constness = if is_const {
        RawConstness::Const
    } else {
        RawConstness::Mutable
    };
    value
}

fn parse_enum(definition: &reader::TypeDef) -> (RawScalar, Vec<RawEnumMember>) {
    let mut underlying = RawScalar::I32;
    let mut members = Vec::new();
    for field in definition.fields() {
        if field.name() == "value__" {
            underlying = match field.ty() {
                windows_metadata::Type::I8 => RawScalar::I8,
                windows_metadata::Type::U8 => RawScalar::U8,
                windows_metadata::Type::I16 => RawScalar::I16,
                windows_metadata::Type::U16 => RawScalar::U16,
                windows_metadata::Type::I32 => RawScalar::I32,
                windows_metadata::Type::U32 => RawScalar::U32,
                windows_metadata::Type::I64 => RawScalar::I64,
                windows_metadata::Type::U64 => RawScalar::U64,
                _ => RawScalar::I32,
            };
            continue;
        }
        let Some(constant) = field.constant() else {
            continue;
        };
        let value = match constant.value() {
            windows_metadata::Value::I8(value) => value as i128,
            windows_metadata::Value::U8(value) => value as i128,
            windows_metadata::Value::I16(value) => value as i128,
            windows_metadata::Value::U16(value) => value as i128,
            windows_metadata::Value::I32(value) => value as i128,
            windows_metadata::Value::U32(value) => value as i128,
            windows_metadata::Value::I64(value) => value as i128,
            windows_metadata::Value::U64(value) => value as i128,
            _ => continue,
        };
        members.push(RawEnumMember {
            name: field.name().to_string(),
            value,
        });
    }
    (underlying, members)
}

fn buffer_element(typ: &RawType) -> RawType {
    let mut element = typ.clone();
    if element.pointer_depth > 0 {
        element.pointer_depth -= 1;
    } else if matches!(
        element.base,
        RawBaseType::Named {
            kind: RawNamedKind::DataPointer | RawNamedKind::StringPointer { .. },
            ..
        }
    ) {
        element.base = match &element.base {
            RawBaseType::Named {
                kind:
                    RawNamedKind::StringPointer {
                        encoding: RawStringEncoding::Utf16,
                    },
                ..
            } => RawBaseType::Scalar(RawScalar::Char16),
            _ => RawBaseType::Scalar(RawScalar::U8),
        };
    }
    element
}

fn buffer_size(parameter: &reader::MethodParam) -> Option<RawBufferSize> {
    if let Some(attribute) = parameter.find_attribute("NativeArrayInfoAttribute") {
        let values = attribute.value();
        if let Some(index) = attribute_usize(&values, "CountParamIndex") {
            return Some(RawBufferSize::ElementCountParam(index));
        }
        if let Some(count) = attribute_usize(&values, "CountConst") {
            return Some(RawBufferSize::Constant(count));
        }
        return Some(RawBufferSize::Unknown);
    }
    if let Some(attribute) = parameter.find_attribute("MemorySizeAttribute") {
        return Some(
            attribute_usize(&attribute.value(), "BytesParamIndex")
                .map(RawBufferSize::ByteCountParam)
                .unwrap_or(RawBufferSize::Unknown),
        );
    }
    None
}

fn apply_known_buffer_contracts(namespace: &str, function: &str, parameters: &mut [RawParameter]) {
    if namespace == "Windows.Win32.Globalization"
        && matches!(function, "LCMapStringW" | "LCMapStringEx")
        && parameters
            .get(4)
            .is_some_and(|parameter| parameter.name == "lpDestStr")
    {
        parameters[4].buffer = None;
        return;
    }
    let contract = match (namespace, function) {
        ("Windows.Win32.Globalization", "LCMapStringA") => {
            Some((4, "lpDestStr", 5, "cchDest", true))
        }
        ("Windows.Win32.Globalization", "FoldStringA" | "FoldStringW") => {
            Some((3, "lpDestStr", 4, "cchDest", true))
        }
        (
            "Windows.Win32.Globalization",
            "GetLocaleInfoA" | "GetLocaleInfoW" | "GetLocaleInfoEx",
        ) => Some((2, "lpLCData", 3, "cchData", true)),
        (
            "Windows.Win32.System.Threading",
            "QueryFullProcessImageNameA" | "QueryFullProcessImageNameW",
        ) => Some((2, "lpExeName", 3, "lpdwSize", false)),
        _ => None,
    };
    let Some((buffer_index, buffer_name, count_index, count_name, nullable)) = contract else {
        return;
    };
    if buffer_index >= count_index || count_index >= parameters.len() {
        return;
    }
    let (before_count, from_count) = parameters.split_at_mut(count_index);
    let buffer = &mut before_count[buffer_index];
    let count = &from_count[0];
    if buffer.name != buffer_name || count.name != count_name || buffer.buffer.is_some() {
        return;
    }
    buffer.buffer = Some(RawBuffer {
        element: buffer_element(&buffer.typ),
        size: RawBufferSize::ElementCountParam(count_index),
    });
    buffer.nullable = nullable;
}

fn attribute_usize(values: &[(String, windows_metadata::Value)], name: &str) -> Option<usize> {
    values
        .iter()
        .find(|(candidate, _)| candidate == name)
        .and_then(|(_, value)| match value {
            windows_metadata::Value::I16(value) if *value >= 0 => Some(*value as usize),
            windows_metadata::Value::U16(value) => Some(*value as usize),
            windows_metadata::Value::I32(value) if *value >= 0 => Some(*value as usize),
            windows_metadata::Value::U32(value) => usize::try_from(*value).ok(),
            _ => None,
        })
}

fn free_with(parameter: &reader::MethodParam) -> Option<String> {
    parameter
        .find_attribute("FreeWithAttribute")
        .and_then(|attribute| first_attribute_string(&attribute.value()))
}

fn handle_cleanup(index: &reader::Index, namespace: &str, name: &str) -> Option<String> {
    let metadata = index
        .get(namespace, name)
        .next()
        .and_then(|definition| definition.find_attribute("RAIIFreeAttribute"))
        .and_then(|attribute| first_attribute_string(&attribute.value()));
    metadata.or_else(|| {
        match name {
            "HKEY" => Some("RegCloseKey"),
            "HANDLE" | "SC_HANDLE" => Some("CloseHandle"),
            "HLOCAL" => Some("LocalFree"),
            _ => None,
        }
        .map(str::to_string)
    })
}

fn first_attribute_string(values: &[(String, windows_metadata::Value)]) -> Option<String> {
    values.iter().find_map(|(_, value)| match value {
        windows_metadata::Value::Utf8(value) | windows_metadata::Value::Utf16(value) => {
            Some(value.to_string())
        }
        _ => None,
    })
}

fn architectures<'a, T: HasAttributes<'a>>(item: &T) -> RawArchitectures {
    let Some(attribute) = item.find_attribute("SupportedArchitectureAttribute") else {
        return RawArchitectures::all();
    };
    let Some(bits) = attribute
        .value()
        .first()
        .and_then(|(_, value)| match value {
            windows_metadata::Value::I32(value) => Some(*value as u32),
            windows_metadata::Value::U32(value) => Some(*value),
            windows_metadata::Value::AttributeEnum(_, value) => Some(*value as u32),
            _ => None,
        })
    else {
        return RawArchitectures {
            x86: false,
            x64: false,
            arm64: false,
        };
    };
    RawArchitectures {
        x86: bits == 0 || bits & 0x1 != 0,
        x64: bits == 0 || bits & 0x2 != 0,
        arm64: bits == 0 || bits & 0x4 != 0,
    }
}

fn calling_convention(method: &reader::MethodDef) -> RawCallingConvention {
    let Some(flags) = method.impl_map().map(|mapping| mapping.flags()) else {
        return RawCallingConvention::Unsupported;
    };
    let Some(bits) = parse_pinvoke_attribute_bits(&format!("{flags:?}")) else {
        return RawCallingConvention::Unsupported;
    };
    calling_convention_bits(bits)
}

fn parse_pinvoke_attribute_bits(value: &str) -> Option<u16> {
    value
        .strip_prefix("PInvokeAttributes(")?
        .strip_suffix(')')?
        .parse()
        .ok()
}

fn calling_convention_bits(bits: u16) -> RawCallingConvention {
    match bits & 0x0700 {
        0x0100 => RawCallingConvention::System,
        0x0200 => RawCallingConvention::Cdecl,
        _ => RawCallingConvention::Unsupported,
    }
}

fn status_semantics(raw: &windows_metadata::Type, mapped: &RawType) -> RawStatusSemantics {
    if let windows_metadata::Type::Name(name) = raw {
        if matches!(
            name.name.as_ref(),
            "HRESULT" | "NTSTATUS" | "SECURITY_STATUS"
        ) {
            return RawStatusSemantics::SignedNonNegativeIsSuccess;
        }
        if matches!(
            name.name.as_ref(),
            "LSTATUS" | "WIN32_ERROR" | "CONFIGRET" | "RPC_STATUS" | "NET_API_STATUS" | "MMRESULT"
        ) {
            return RawStatusSemantics::ZeroIsSuccess;
        }
    }
    if matches!(
        &mapped.base,
        RawBaseType::Named {
            kind: RawNamedKind::Enum { .. },
            name,
            ..
        } if name == "WIN32_ERROR"
    ) {
        RawStatusSemantics::ZeroIsSuccess
    } else {
        RawStatusSemantics::None
    }
}

fn is_data_pointer_alias(name: &str) -> bool {
    matches!(
        name,
        "PSID"
            | "PSECURITY_DESCRIPTOR"
            | "MEMORY_MAPPED_VIEW_ADDRESS"
            | "LPPROC_THREAD_ATTRIBUTE_LIST"
            | "PVOID"
            | "PCVOID"
            | "LPVOID"
            | "LPCVOID"
    )
}

fn is_handle_alias(name: &str) -> bool {
    const HANDLES: &[&str] = &[
        "HANDLE",
        "HWND",
        "HACCEL",
        "HBITMAP",
        "HBRUSH",
        "HCURSOR",
        "HDC",
        "HDESK",
        "HDWP",
        "HENHMETAFILE",
        "HFONT",
        "HGDIOBJ",
        "HGLOBAL",
        "HHOOK",
        "HICON",
        "HIMAGELIST",
        "HINSTANCE",
        "HKEY",
        "HKL",
        "HLOCAL",
        "HMENU",
        "HMETAFILE",
        "HMODULE",
        "HMONITOR",
        "HPALETTE",
        "HPEN",
        "HRAWINPUT",
        "HRGN",
        "HRSRC",
        "HTHEME",
        "HWINSTA",
        "SC_HANDLE",
        "SERVICE_STATUS_HANDLE",
        "DPI_AWARENESS_CONTEXT",
    ];
    HANDLES.contains(&name)
}

pub fn distinct_containers(functions: &[RawFunction]) -> usize {
    functions
        .iter()
        .map(|function| (&function.namespace, &function.container))
        .collect::<HashSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_winmd() -> Option<String> {
        std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).is_file())
    }

    #[test]
    fn pinvoke_convention_mask_is_decoded_exactly() {
        assert_eq!(
            calling_convention_bits(0x0100),
            RawCallingConvention::System
        );
        assert_eq!(calling_convention_bits(0x0200), RawCallingConvention::Cdecl);
        for bits in [0x0000, 0x0300, 0x0400, 0x0500] {
            assert_eq!(
                calling_convention_bits(bits),
                RawCallingConvention::Unsupported
            );
        }
        assert_eq!(
            parse_pinvoke_attribute_bits("PInvokeAttributes(768)"),
            Some(0x0300)
        );
        assert_eq!(parse_pinvoke_attribute_bits("changed-format"), None);
    }

    #[test]
    fn registry_metadata_preserves_handle_cleanup_and_buffer_relation() {
        let Some(winmd) = configured_winmd() else {
            return;
        };
        let apis = parse_apis(&winmd, "Windows.Win32.System.Registry", "Apis").unwrap();
        let open = apis
            .functions
            .iter()
            .find(|function| function.name == "RegOpenKeyExW")
            .unwrap();
        let output = open.parameters.last().unwrap();
        assert_eq!(output.direction, RawDirection::Out);
        assert!(matches!(
            &output.typ.base,
            RawBaseType::Named {
                kind: RawNamedKind::Handle { cleanup: Some(cleanup) },
                ..
            } if cleanup == "RegCloseKey"
        ));
        assert_eq!(output.typ.pointer_depth, 1);

        let query = apis
            .functions
            .iter()
            .find(|function| function.name == "RegQueryValueExW")
            .unwrap();
        assert!(matches!(
            query.parameters[4]
                .buffer
                .as_ref()
                .map(|buffer| &buffer.size),
            Some(RawBufferSize::ByteCountParam(5))
        ));
    }

    #[test]
    fn known_string_buffer_contracts_fill_metadata_gaps_exactly() {
        let Some(winmd) = configured_winmd() else {
            return;
        };
        let apis = parse_apis(&winmd, "Windows.Win32.Globalization", "Apis").unwrap();
        let function = apis
            .functions
            .iter()
            .find(|function| function.name == "LCMapStringA")
            .unwrap();
        let buffer = &function.parameters[4];
        assert_eq!(buffer.name, "lpDestStr");
        assert!(buffer.nullable);
        assert!(matches!(
            buffer.buffer.as_ref().map(|buffer| &buffer.size),
            Some(RawBufferSize::ElementCountParam(5))
        ));
        for name in ["LCMapStringW", "LCMapStringEx"] {
            let function = apis
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap();
            assert!(function.parameters[4].buffer.is_none());
        }
    }

    #[test]
    fn nested_anonymous_layouts_are_resolved_from_their_enclosing_type() {
        let Some(winmd) = configured_winmd() else {
            return;
        };
        let apis = parse_apis(&winmd, "Windows.Win32.System.SystemInformation", "Apis").unwrap();
        let system_info = apis
            .functions
            .iter()
            .find(|function| function.name == "GetSystemInfo")
            .expect("GetSystemInfo metadata");
        let system_info = &system_info.parameters[0].typ;
        let RawBaseType::Named {
            kind: RawNamedKind::NativeStruct { layout },
            ..
        } = &system_info.base
        else {
            panic!("SYSTEM_INFO must retain native layout");
        };
        let variant = layout.variants.first().expect("SYSTEM_INFO layout");
        assert!(
            variant.fields.iter().any(|field| {
                matches!(
                    &field.typ.base,
                    RawBaseType::Named {
                        name,
                        kind: RawNamedKind::NativeStruct { layout },
                        ..
                    } if name.contains("_Anonymous_e__Union")
                        && layout.variants.first().is_some_and(|nested| nested.kind == RawLayoutKind::Union)
                )
            }),
            "{:#?}",
            variant.fields
        );
    }

    #[test]
    fn hfile_preserves_its_signed_i32_abi() {
        let Some(winmd) = configured_winmd() else {
            return;
        };
        let functions = parse_all_functions(&winmd).unwrap();
        let function = functions
            .iter()
            .find(|function| function.name == "_lopen")
            .expect("configured metadata contains _lopen");
        assert!(matches!(
            function.return_type,
            RawType {
                base: RawBaseType::Scalar(RawScalar::I32),
                pointer_depth: 0,
                ..
            }
        ));

        let config = functions
            .iter()
            .find(|function| function.name == "CM_Disable_DevNode")
            .expect("configured metadata contains CM_Disable_DevNode");
        assert_eq!(config.return_status, RawStatusSemantics::ZeroIsSuccess);

        let ldap = functions
            .iter()
            .find(|function| function.name == "LdapGetLastError")
            .expect("configured metadata contains LdapGetLastError");
        assert_eq!(ldap.calling_convention, RawCallingConvention::Cdecl);
        assert!(!ldap.variadic);
    }
}
