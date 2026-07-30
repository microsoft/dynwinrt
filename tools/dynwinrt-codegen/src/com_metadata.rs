// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use windows_metadata::{HasAttributes, reader};

use crate::types::TypeMeta;

#[derive(Debug, Clone, PartialEq)]
pub enum ParamDirection {
    In,
    Out,
    InOut,
    OutFill,
    OutStringBuffer { count_param_index: usize },
    UnsupportedNativeArray { count_param_index: Option<usize> },
}

impl ParamDirection {
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::In | Self::InOut | Self::UnsupportedNativeArray { .. }
        )
    }

    pub fn is_output(&self) -> bool {
        matches!(
            self,
            Self::Out
                | Self::InOut
                | Self::OutFill
                | Self::OutStringBuffer { .. }
                | Self::UnsupportedNativeArray { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name: String,
    pub typ: TypeMeta,
    pub direction: ParamDirection,
}

#[derive(Debug, Clone, Default)]
pub struct MethodMeta {
    pub name: String,
    pub vtable_index: usize,
    pub params: Vec<ParamMeta>,
    pub return_type: Option<TypeMeta>,
    pub preserve_hresult: bool,
    pub doc: Option<String>,
    pub owned_outputs: Vec<OwnedOutput>,
}

#[derive(Debug, Clone)]
pub struct OwnedOutput {
    pub param_index: usize,
    pub free_with: String,
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceMeta {
    pub name: String,
    pub namespace: String,
    pub iid: String,
    pub methods: Vec<MethodMeta>,
    pub generic_piid: Option<String>,
    pub generic_args: Vec<TypeMeta>,
    pub doc: Option<String>,
    pub deprecated: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComInterfaceMeta {
    pub interface: InterfaceMeta,
    pub base_offset: usize,
    pub is_iunknown_rooted: bool,
    pub base_chain: Vec<String>,
    pub coclass_clsid: Option<String>,
    pub coclass_name: Option<String>,
    pub own_methods_start: usize,
    pub referenced_enums: Vec<ComEnumMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComEnumValue {
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Clone)]
pub struct ComEnumMember {
    pub name: String,
    pub value: ComEnumValue,
}

#[derive(Debug, Clone)]
pub struct ComEnumMeta {
    pub namespace: String,
    pub name: String,
    pub underlying: TypeMeta,
    pub members: Vec<ComEnumMember>,
    pub is_flags: bool,
}

pub fn parse_com_interface(
    winmd_paths: &str,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let index = crate::meta::load_index(winmd_paths)?;
    parse_com_interface_from_index(&index, namespace, name)
}

pub fn parse_com_enum(winmd_paths: &str, namespace: &str, name: &str) -> Option<ComEnumMeta> {
    let index = crate::meta::load_index(winmd_paths)?;
    let def = index.get(namespace, name).next()?;
    parse_com_enum_def(&def)
}

pub fn first_classic_com_interface_in_namespace(
    winmd_paths: &str,
    namespace: &str,
) -> Option<String> {
    let index = crate::meta::load_index(winmd_paths)?;
    let names = index
        .all()
        .filter(|def| {
            def.namespace() == namespace
                && def
                    .flags()
                    .contains(windows_metadata::TypeAttributes::Interface)
        })
        .map(|def| def.name().to_string())
        .collect::<Vec<_>>();
    names.into_iter().find(|name| {
        parse_com_interface_from_index(&index, namespace, name).is_some_and(|interface| {
            interface.is_iunknown_rooted || interface.interface.name.ends_with("Interop")
        })
    })
}

fn parse_com_interface_from_index(
    index: &reader::Index,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let def = index.get(namespace, name).next()?;
    if !def
        .flags()
        .contains(windows_metadata::TypeAttributes::Interface)
    {
        return None;
    }

    let mut base_chain = Vec::new();
    let mut current = (namespace.to_string(), name.to_string());
    let mut root = None;
    for _ in 0..32 {
        let current_def = index.get(&current.0, &current.1).next()?;
        let base = match current_def.interface_impls().next()?.interface(&[]) {
            windows_metadata::Type::Name(name) => (name.namespace, name.name),
            _ => return None,
        };
        match base.1.as_str() {
            "IUnknown" => {
                root = Some((true, 3));
                base_chain.push((
                    "Windows.Win32.System.Com".to_string(),
                    "IUnknown".to_string(),
                    0,
                ));
                break;
            }
            "IInspectable" => {
                root = Some((false, 6));
                base_chain.push((
                    "Windows.Foundation".to_string(),
                    "IInspectable".to_string(),
                    0,
                ));
                break;
            }
            _ => {
                let base_def = index.get(&base.0, &base.1).next()?;
                let count = base_def.methods().count();
                base_chain.push((base.0.clone(), base.1.clone(), count));
                current = base;
            }
        }
    }
    let (is_iunknown_rooted, root_offset) = root?;
    let own_methods_start = root_offset
        + base_chain
            .iter()
            .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
            .map(|(_, _, count)| count)
            .sum::<usize>();

    let mut methods = Vec::new();
    let mut slot = root_offset;
    for (base_namespace, base_name, _) in base_chain
        .iter()
        .rev()
        .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
    {
        let base_def = index.get(base_namespace, base_name).next()?;
        let mut base_methods = parse_methods(index, &base_def, slot);
        slot += base_methods.len();
        methods.append(&mut base_methods);
    }
    if slot != own_methods_start {
        return None;
    }
    methods.extend(parse_methods(index, &def, slot));

    let iid = crate::meta::extract_iid(&def);
    let interface = InterfaceMeta {
        name: name.to_string(),
        namespace: namespace.to_string(),
        iid,
        methods,
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };
    let (coclass_name, coclass_clsid) = find_coclass(index, namespace, name);
    let referenced_enums = collect_referenced_enums(index, &interface);

    Some(ComInterfaceMeta {
        interface,
        base_offset: root_offset,
        is_iunknown_rooted,
        base_chain: base_chain.into_iter().map(|(_, name, _)| name).collect(),
        coclass_clsid,
        coclass_name,
        own_methods_start,
        referenced_enums,
    })
}

fn parse_methods(
    index: &reader::Index,
    def: &reader::TypeDef,
    base_offset: usize,
) -> Vec<MethodMeta> {
    def.methods()
        .enumerate()
        .map(|(index_in_interface, method)| {
            let signature = method.signature(&[]);
            let raw_name = method.name().to_string();
            let name = method
                .find_attribute("OverloadAttribute")
                .and_then(|attribute| {
                    attribute
                        .value()
                        .into_iter()
                        .next()
                        .and_then(|(_, value)| match value {
                            windows_metadata::Value::Utf8(value) => Some(value),
                            _ => None,
                        })
                })
                .unwrap_or(raw_name);
            let mut params = Vec::new();
            let mut owned_outputs = Vec::new();
            for (param_index, (param, typ)) in method
                .params()
                .filter(|param| param.sequence() > 0)
                .zip(signature.types.iter())
                .enumerate()
            {
                let mut direction = classify_direction(
                    param.flags(),
                    matches!(typ, windows_metadata::Type::Array(_)),
                );
                let mapped_type = map_parameter_type(typ, &direction, index);
                if let Some(count_param_index) = native_array_count_param(&param) {
                    if direction.is_output() && !is_string_buffer(&mapped_type) {
                        direction = ParamDirection::UnsupportedNativeArray { count_param_index };
                    }
                }
                let free_with = param
                    .find_attribute("FreeWithAttribute")
                    .and_then(|attribute| {
                        attribute
                            .value()
                            .into_iter()
                            .next()
                            .and_then(|(_, value)| match value {
                                windows_metadata::Value::Utf8(value) => Some(value),
                                _ => None,
                            })
                    })
                    .or_else(|| {
                        known_free_with(def.namespace(), def.name(), method.name(), typ, &direction)
                    });
                if let Some(free_with) = free_with {
                    owned_outputs.push(OwnedOutput {
                        param_index,
                        free_with,
                    });
                }
                params.push(ParamMeta {
                    name: param.name().to_string(),
                    typ: mapped_type,
                    direction,
                });
            }
            mark_caller_owned_string_buffers(&mut params);
            let return_type = (signature.return_type != windows_metadata::Type::Void)
                .then(|| map_return_type(&signature.return_type, index));
            let preserve_hresult = method.has_attribute("CanReturnMultipleSuccessValuesAttribute")
                || is_known_semantic_hresult(def.namespace(), def.name(), method.name());
            MethodMeta {
                name,
                vtable_index: base_offset + index_in_interface,
                params,
                return_type,
                preserve_hresult,
                doc: None,
                owned_outputs,
            }
        })
        .collect()
}

fn known_free_with(
    interface_namespace: &str,
    interface_name: &str,
    method_name: &str,
    typ: &windows_metadata::Type,
    direction: &ParamDirection,
) -> Option<String> {
    let (windows_metadata::Type::PtrMut(inner, depth)
    | windows_metadata::Type::PtrConst(inner, depth)) = typ
    else {
        return None;
    };
    if !matches!(direction, ParamDirection::Out | ParamDirection::InOut) {
        return None;
    }
    if *depth == 1
        && matches!(
            inner.as_ref(),
            windows_metadata::Type::Name(name)
                if name.namespace == "Windows.Win32.Foundation" && name.name == "BSTR"
        )
    {
        return Some("SysFreeString".into());
    }
    let is_known_cotaskmem_wide_string = matches!(
        (interface_namespace, interface_name, method_name),
        ("Windows.Win32.UI.Shell", "IShellItem", "GetDisplayName")
            | ("Windows.Win32.UI.Shell", "IFileDialog", "GetFileName")
            | ("Windows.Win32.System.Com", "IPersistFile", "GetCurFile")
    );
    if *depth == 1
        && is_known_cotaskmem_wide_string
        && matches!(
            inner.as_ref(),
            windows_metadata::Type::Name(name)
                if name.namespace == "Windows.Win32.Foundation" && name.name == "PWSTR"
        )
    {
        return Some("CoTaskMemFree".into());
    }
    // Windows.Win32.winmd omits FreeWith on IShellLink::GetIDList.
    if *depth < 2 {
        return None;
    }
    match inner.as_ref() {
        windows_metadata::Type::Name(name)
            if name.namespace == "Windows.Win32.UI.Shell.Common" && name.name == "ITEMIDLIST" =>
        {
            Some("CoTaskMemFree".into())
        }
        _ => None,
    }
}

fn is_known_semantic_hresult(
    interface_namespace: &str,
    interface_name: &str,
    method_name: &str,
) -> bool {
    matches!(
        (interface_namespace, interface_name, method_name),
        ("Windows.Win32.System.Com", "IPersistFile", "GetCurFile")
    )
}

fn map_parameter_type(
    typ: &windows_metadata::Type,
    direction: &ParamDirection,
    index: &reader::Index,
) -> TypeMeta {
    use windows_metadata::Type;

    match typ {
        Type::PtrMut(inner, depth) | Type::PtrConst(inner, depth) => {
            if matches!(direction, ParamDirection::Out | ParamDirection::InOut) && *depth == 1 {
                map_com_type(inner, index)
            } else {
                TypeMeta::Object
            }
        }
        Type::ConstRef(inner)
            if matches!(direction, ParamDirection::Out | ParamDirection::InOut) =>
        {
            map_com_type(inner, index)
        }
        Type::ConstRef(_) => TypeMeta::Object,
        _ => map_com_type(typ, index),
    }
}

fn map_return_type(typ: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    use windows_metadata::Type;

    match typ {
        Type::PtrMut(_, _) | Type::PtrConst(_, _) | Type::ConstRef(_) => TypeMeta::Object,
        _ => map_com_type(typ, index),
    }
}

fn map_com_type(typ: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    match typ {
        windows_metadata::Type::ISize => native_isize_type(),
        windows_metadata::Type::USize => native_usize_type(),
        windows_metadata::Type::Name(name)
            if is_canonical_hstring_name(&name.namespace, &name.name) =>
        {
            TypeMeta::String
        }
        windows_metadata::Type::Name(name) => {
            if let Some(def) = index.get(&name.namespace, &name.name).next() {
                if let Some(enum_meta) = parse_com_enum_def(&def) {
                    return enum_meta.as_type_meta();
                }
                if let Some(delegate) = parse_com_delegate_def(&def) {
                    return delegate;
                }
            }
            crate::meta::map_winmd_type_with_generics(typ, index, &[])
        }
        _ => crate::meta::map_winmd_type_with_generics(typ, index, &[]),
    }
}

fn is_canonical_hstring_name(namespace: &str, name: &str) -> bool {
    namespace == "Windows.Win32.System.WinRT" && name == "HSTRING"
}

fn parse_com_delegate_def(def: &reader::TypeDef) -> Option<TypeMeta> {
    let extends = def.extends()?;
    if !matches!(
        (extends.namespace(), extends.name()),
        ("System", "Delegate") | ("System", "MulticastDelegate")
    ) {
        return None;
    }
    Some(TypeMeta::Delegate {
        namespace: def.namespace().to_string(),
        name: def.name().to_string(),
        iid: crate::meta::extract_iid(def),
    })
}

impl ComEnumMeta {
    fn as_type_meta(&self) -> TypeMeta {
        TypeMeta::Enum {
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            underlying: Box::new(self.underlying.clone()),
            members: Vec::new(),
            is_flags: self.is_flags,
            doc: None,
            deprecated: None,
        }
    }
}

fn parse_com_enum_def(def: &reader::TypeDef) -> Option<ComEnumMeta> {
    let mut fields = def.fields();
    let underlying = fields
        .find(|field| field.name() == "value__")
        .and_then(|field| map_com_enum_underlying(&field.ty()))?;
    let members = def
        .fields()
        .filter(|field| field.name() != "value__")
        .filter_map(|field| {
            let value = match field.constant()?.value() {
                windows_metadata::Value::I8(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U8(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I16(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U16(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I32(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U32(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I64(value) => ComEnumValue::Signed(value),
                windows_metadata::Value::U64(value) => ComEnumValue::Unsigned(value),
                _ => return None,
            };
            Some(ComEnumMember {
                name: field.name().to_string(),
                value,
            })
        })
        .collect();
    Some(ComEnumMeta {
        namespace: def.namespace().to_string(),
        name: def.name().to_string(),
        underlying,
        members,
        is_flags: def.has_attribute("FlagsAttribute"),
    })
}

fn map_com_enum_underlying(typ: &windows_metadata::Type) -> Option<TypeMeta> {
    match typ {
        windows_metadata::Type::I8 => Some(TypeMeta::I8),
        windows_metadata::Type::U8 => Some(TypeMeta::U8),
        windows_metadata::Type::I16 => Some(TypeMeta::I16),
        windows_metadata::Type::U16 => Some(TypeMeta::U16),
        windows_metadata::Type::I32 => Some(TypeMeta::I32),
        windows_metadata::Type::U32 => Some(TypeMeta::U32),
        windows_metadata::Type::I64 => Some(TypeMeta::I64),
        windows_metadata::Type::U64 => Some(TypeMeta::U64),
        _ => None,
    }
}

pub fn native_isize_type() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "System".into(),
        name: "IntPtr".into(),
        fields: Vec::new(),
    }
}

pub fn native_usize_type() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "System".into(),
        name: "UIntPtr".into(),
        fields: Vec::new(),
    }
}

pub fn is_native_isize(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct {
            namespace,
            name,
            ..
        } if namespace == "System" && name == "IntPtr"
    )
}

pub fn is_native_usize(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct {
            namespace,
            name,
            ..
        } if namespace == "System" && name == "UIntPtr"
    )
}

fn native_array_count_param(param: &reader::MethodParam) -> Option<Option<usize>> {
    let attribute = param.find_attribute("NativeArrayInfoAttribute")?;
    let count = attribute
        .value()
        .into_iter()
        .find(|(name, _)| name == "CountParamIndex")
        .and_then(|(_, value)| match value {
            windows_metadata::Value::I16(value) if value >= 0 => Some(value as usize),
            windows_metadata::Value::U16(value) => Some(value as usize),
            windows_metadata::Value::I32(value) if value >= 0 => Some(value as usize),
            windows_metadata::Value::U32(value) => usize::try_from(value).ok(),
            _ => None,
        });
    Some(count)
}

fn classify_direction(flags: windows_metadata::ParamAttributes, is_array: bool) -> ParamDirection {
    let is_in = flags.contains(windows_metadata::ParamAttributes::In);
    let is_out = flags.contains(windows_metadata::ParamAttributes::Out);
    match (is_in, is_out, is_array) {
        (true, true, _) => ParamDirection::InOut,
        (_, true, true) => ParamDirection::OutFill,
        (_, true, false) => ParamDirection::Out,
        _ => ParamDirection::In,
    }
}

fn find_coclass(
    index: &reader::Index,
    namespace: &str,
    interface_name: &str,
) -> (Option<String>, Option<String>) {
    let Some(stripped) = interface_name.strip_prefix('I') else {
        return (None, None);
    };
    let mut candidates = vec![stripped.to_string()];
    let without_version = stripped
        .trim_end_matches(|character: char| character.is_ascii_digit())
        .to_string();
    if without_version != stripped {
        candidates.push(without_version);
    }
    for candidate in candidates {
        let Some(def) = index.get(namespace, &candidate).next() else {
            continue;
        };
        let is_coclass = matches!(
            def.extends()
                .map(|base| (base.namespace().to_string(), base.name().to_string())),
            Some((namespace, name)) if namespace == "System" && name == "ValueType"
        );
        if is_coclass {
            let clsid = crate::meta::extract_iid(&def);
            if !clsid.is_empty() {
                return (Some(candidate), Some(clsid));
            }
        }
    }
    (None, None)
}

fn collect_referenced_enums(index: &reader::Index, interface: &InterfaceMeta) -> Vec<ComEnumMeta> {
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for method in &interface.methods {
        for typ in method
            .params
            .iter()
            .map(|param| &param.typ)
            .chain(method.return_type.iter())
        {
            if let TypeMeta::Enum {
                namespace, name, ..
            } = typ
            {
                let full_name = format!("{namespace}.{name}");
                if names.insert(full_name)
                    && let Some(enum_meta) = index
                        .get(namespace, name)
                        .next()
                        .and_then(|def| parse_com_enum_def(&def))
                {
                    result.push(enum_meta);
                }
            }
        }
    }
    result
}

fn mark_caller_owned_string_buffers(params: &mut [ParamMeta]) {
    for index in 0..params.len().saturating_sub(1) {
        if params[index].direction == ParamDirection::Out
            && is_string_buffer(&params[index].typ)
            && params[index + 1].direction == ParamDirection::In
            && is_string_buffer_count(&params[index].typ, &params[index + 1])
        {
            params[index].direction = ParamDirection::OutStringBuffer {
                count_param_index: index + 1,
            };
        }
    }
    let count_index = params.iter().find_map(|param| match param.direction {
        ParamDirection::OutStringBuffer { count_param_index } => Some(count_param_index),
        _ => None,
    });
    if let Some(count_index) = count_index {
        for param in params.iter_mut().skip(count_index + 1) {
            let name = param.name.to_ascii_lowercase();
            let is_find_data = name == "pfd"
                || name.contains("finddata")
                || matches!(
                    &param.typ,
                    TypeMeta::Struct { name, .. }
                        if name == "WIN32_FIND_DATAW" || name == "WIN32_FIND_DATAA"
                );
            if is_find_data
                && matches!(param.direction, ParamDirection::Out | ParamDirection::InOut)
            {
                param.direction = ParamDirection::In;
                param.typ = TypeMeta::Object;
            }
        }
    }
}

fn is_string_buffer(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && (name == "PWSTR" || name == "PSTR")
    )
}

fn is_string_buffer_count(buffer_type: &TypeMeta, param: &ParamMeta) -> bool {
    let name = param.name.to_ascii_lowercase();
    let is_wide = matches!(
        buffer_type,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && name == "PWSTR"
    );
    matches!(param.typ, TypeMeta::I32 | TypeMeta::U32)
        && (name.starts_with("cch")
            || (!is_wide && name.starts_with("cb"))
            || matches!(name.as_str(), "len" | "length" | "size" | "max" | "count")
            || name.starts_with("max")
            || name.starts_with("size"))
}

pub fn find_runtime_class_default_iid(
    winmd_paths: &str,
    simple_name: &str,
) -> Option<(String, String, String)> {
    let index = crate::meta::load_index(winmd_paths)?;
    let mut found = None;
    let mut collision = false;
    for def in index.all() {
        if def.name() != simple_name
            || !def
                .flags()
                .contains(windows_metadata::TypeAttributes::WindowsRuntime)
            || def
                .flags()
                .contains(windows_metadata::TypeAttributes::Interface)
        {
            continue;
        }
        for implementation in def.interface_impls() {
            if !implementation.has_attribute("DefaultAttribute") {
                continue;
            }
            let windows_metadata::Type::Name(name) = implementation.interface(&[]) else {
                continue;
            };
            if !name.generics.is_empty() {
                continue;
            }
            let interface = index.get(&name.namespace, &name.name).next()?;
            let iid = crate::meta::extract_iid(&interface);
            if iid.is_empty() {
                continue;
            }
            let candidate = (def.namespace().to_string(), name.name, iid);
            match &found {
                None => found = Some(candidate),
                Some(existing) if existing == &candidate => {}
                Some(_) => collision = true,
            }
            break;
        }
    }
    (!collision).then_some(found).flatten()
}

pub fn discover_newest_windows_winmd() -> Option<String> {
    let base = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata");
    let mut versions = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("10."))
        .collect::<Vec<_>>();
    versions.sort_by_key(|version| {
        version
            .split('.')
            .filter_map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
    });
    versions.into_iter().rev().find_map(|version| {
        let path = base.join(version).join("Windows.winmd");
        path.exists().then(|| path.to_string_lossy().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_out_is_com_only() {
        use windows_metadata::ParamAttributes;

        assert_eq!(
            classify_direction(ParamAttributes::In | ParamAttributes::Out, false),
            ParamDirection::InOut
        );
    }

    #[test]
    fn hstring_mapping_requires_the_canonical_namespace() {
        assert!(is_canonical_hstring_name(
            "Windows.Win32.System.WinRT",
            "HSTRING"
        ));
        assert!(!is_canonical_hstring_name("Contoso.Interop", "HSTRING"));
    }

    #[test]
    fn find_data_after_string_buffer_is_caller_owned_pointer() {
        let mut params = vec![
            ParamMeta {
                name: "pszFile".into(),
                typ: TypeMeta::Struct {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "PWSTR".into(),
                    fields: Vec::new(),
                },
                direction: ParamDirection::Out,
            },
            ParamMeta {
                name: "cch".into(),
                typ: TypeMeta::I32,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "pfd".into(),
                typ: TypeMeta::Object,
                direction: ParamDirection::InOut,
            },
        ];

        mark_caller_owned_string_buffers(&mut params);

        assert_eq!(
            params[0].direction,
            ParamDirection::OutStringBuffer {
                count_param_index: 1
            }
        );
        assert_eq!(params[2].direction, ParamDirection::In);
        assert!(matches!(params[2].typ, TypeMeta::Object));
    }

    #[test]
    fn item_id_list_double_pointer_uses_cotaskmem_ownership() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.UI.Shell.Common",
                "ITEMIDLIST",
            )),
            2,
        );
        assert_eq!(
            known_free_with("", "", "", &typ, &ParamDirection::Out).as_deref(),
            Some("CoTaskMemFree")
        );
    }

    #[test]
    fn bstr_array_does_not_claim_scalar_sysfree_ownership() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.Foundation",
                "BSTR",
            )),
            2,
        );
        assert_eq!(
            known_free_with("", "", "", &typ, &ParamDirection::Out),
            None
        );
    }

    #[test]
    fn documented_shell_wide_string_outputs_use_cotaskmem() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.Foundation",
                "PWSTR",
            )),
            1,
        );
        for (interface, method) in [
            ("IShellItem", "GetDisplayName"),
            ("IFileDialog", "GetFileName"),
        ] {
            assert_eq!(
                known_free_with(
                    "Windows.Win32.UI.Shell",
                    interface,
                    method,
                    &typ,
                    &ParamDirection::Out
                )
                .as_deref(),
                Some("CoTaskMemFree")
            );
        }
        assert_eq!(
            known_free_with(
                "Windows.Win32.System.Com",
                "IPersistFile",
                "GetCurFile",
                &typ,
                &ParamDirection::Out
            )
            .as_deref(),
            Some("CoTaskMemFree")
        );
    }

    #[test]
    fn documented_get_cur_file_hresult_is_semantic() {
        assert!(is_known_semantic_hresult(
            "Windows.Win32.System.Com",
            "IPersistFile",
            "GetCurFile"
        ));
        assert!(!is_known_semantic_hresult(
            "Windows.Win32.System.Com",
            "IPersistFile",
            "Load"
        ));
    }
}
