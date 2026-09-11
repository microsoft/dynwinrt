// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Flat-export facts, deliberately separate from WinRT and Classic COM models.

use serde::Serialize;
use sha2::{Digest, Sha256};
use windows_metadata::{AsRow, HasAttributes, reader};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeType {
    pub name: String,
    pub kind: String,
    pub pointers: Vec<bool>,
    pub underlying: Option<Box<NativeType>>,
    pub attributes: Vec<Attribute>,
}

impl NativeType {
    pub(crate) fn scalar(&self, name: &str) -> bool {
        self.name == name && self.kind == "scalar" && self.pointers.is_empty()
    }

    pub(crate) fn pointer_to(&self, name: &str) -> bool {
        self.name == name && self.kind == "scalar" && self.pointers == [false]
    }

    pub(crate) fn named(&self, name: &str, pointers: &[bool], underlying: &str) -> bool {
        self.name == name
            && self.pointers == pointers
            && self
                .underlying
                .as_ref()
                .is_some_and(|value| value.scalar(underlying))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Parameter {
    pub name: String,
    pub sequence: u16,
    pub input: bool,
    pub output: bool,
    pub optional: bool,
    pub flags: String,
    pub attributes: Vec<Attribute>,
    pub const_attribute: bool,
    pub reserved: bool,
    pub byte_count_parameter: Option<u16>,
    pub typ: NativeType,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Export {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: String,
    pub architectures: u32,
    pub method_flags: String,
    pub impl_flags: String,
    pub import_flags: String,
    pub signature_flags: String,
    pub signature: String,
    pub attributes: Vec<Attribute>,
    pub return_attributes: Vec<Attribute>,
    pub return_type: NativeType,
    pub parameters: Vec<Parameter>,
    // Provenance is checked independently; it is not a substitute for the
    // pre-contract fingerprint of all the native facts above.
    #[serde(skip)]
    pub metadata_sha256: String,
}

impl Export {
    pub fn fingerprint(&self) -> String {
        sha256(&serde_json::to_vec(self).expect("native facts are serializable"))
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:X}", Sha256::digest(bytes))
}

fn hex_blob(mut blob: reader::Blob<'_>) -> String {
    let mut result = String::new();
    while !blob.is_empty() {
        use std::fmt::Write;
        write!(result, "{:02X}", blob.read_u8()).unwrap();
    }
    result
}

fn attributes<'a>(item: &impl HasAttributes<'a>) -> Vec<Attribute> {
    let mut result = item
        .attributes()
        .map(|attribute| {
            let constructor = attribute.ctor();
            let parent = constructor.parent();
            Attribute {
                name: format!("{}.{}", parent.namespace(), parent.name()),
                value: hex_blob(attribute.blob(2)),
            }
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| (&left.name, &left.value).cmp(&(&right.name, &right.value)));
    result
}

fn native_type(typ: &windows_metadata::Type, index: &reader::Index, depth: usize) -> NativeType {
    use windows_metadata::Type;
    let plain = |name: String, kind: &str| NativeType {
        name,
        kind: kind.into(),
        pointers: Vec::new(),
        underlying: None,
        attributes: Vec::new(),
    };
    if depth > 8 {
        return plain("recursive-or-deep-type".into(), "unsupported");
    }
    match typ {
        Type::PtrMut(inner, count) | Type::PtrConst(inner, count) => {
            let mut result = native_type(inner, index, depth + 1);
            result.pointers.extend(std::iter::repeat_n(
                matches!(typ, Type::PtrConst(_, _)),
                *count,
            ));
            result
        }
        Type::ConstRef(inner) => {
            let mut result = native_type(inner, index, depth + 1);
            result.pointers.push(true);
            result
        }
        Type::Void => plain("void".into(), "scalar"),
        Type::U8 => plain("u8".into(), "scalar"),
        Type::U16 => plain("u16".into(), "scalar"),
        Type::Char => plain("char16".into(), "scalar"),
        Type::U32 => plain("u32".into(), "scalar"),
        Type::U64 => plain("u64".into(), "scalar"),
        Type::I32 => plain("i32".into(), "scalar"),
        Type::ISize => plain("isize".into(), "scalar"),
        Type::USize => plain("usize".into(), "scalar"),
        Type::Name(name) if name.generics.is_empty() => {
            let mut result = plain(format!("{}.{}", name.namespace, name.name), "unsupported");
            let definitions = index.get(&name.namespace, &name.name).collect::<Vec<_>>();
            if let [definition] = definitions.as_slice() {
                result.attributes = attributes(definition);
                let fields = definition
                    .fields()
                    .filter(|field| {
                        !field
                            .flags()
                            .contains(windows_metadata::FieldAttributes::Static)
                    })
                    .collect::<Vec<_>>();
                if let [field] = fields.as_slice() {
                    let is_enum = field.name() == "value__";
                    let is_typedef = definition.has_attribute("NativeTypedefAttribute");
                    if is_enum || (is_typedef && field.name() == "Value") {
                        result.kind = if is_enum { "enum" } else { "typedef" }.into();
                        result.underlying =
                            Some(Box::new(native_type(&field.ty(), index, depth + 1)));
                    }
                }
            }
            result
        }
        unsupported => plain(format!("{unsupported:?}"), "unsupported"),
    }
}

fn read_indexes(winmd_paths: &str) -> Result<Vec<(reader::Index, String)>, String> {
    let paths = winmd_paths
        .split(';')
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err("win32.metadata: no metadata paths supplied".into());
    }
    paths
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("win32.metadata: cannot read {path}: {error}"))?;
            let index = reader::Index::read(path)
                .ok_or_else(|| format!("win32.metadata: invalid metadata {path}"))?;
            Ok((index, sha256(&bytes)))
        })
        .collect()
}

pub(crate) fn has_exports(paths: &str, namespace: &str, container: &str) -> Result<bool, String> {
    Ok(read_indexes(paths)?.iter().any(|(index, _)| {
        index.get(namespace, container).any(|definition| {
            definition
                .methods()
                .any(|method| method.impl_map().is_some())
        })
    }))
}

pub fn read_exports(paths: &str, namespace: &str) -> Result<Vec<Export>, String> {
    let mut result = Vec::new();
    for (index, metadata_sha256) in read_indexes(paths)? {
        for definition in index.get(namespace, "Apis") {
            for method in definition.methods() {
                let Some(import) = method.impl_map() else {
                    continue;
                };
                let signature = method.signature(&[]);
                let parameters = method
                    .params()
                    .filter(|p| p.sequence() > 0)
                    .collect::<Vec<_>>();
                if parameters.len() != signature.types.len()
                    || parameters
                        .iter()
                        .enumerate()
                        .any(|(i, p)| usize::from(p.sequence()) != i + 1)
                {
                    return Err(format!(
                        "win32.metadata: incomplete parameter metadata: {}",
                        method.name()
                    ));
                }
                let architectures = method
                    .find_attribute("SupportedArchitectureAttribute")
                    .map(|attribute| match attribute.value().first() {
                        Some((_, windows_metadata::Value::I32(value))) => *value as u32,
                        Some((_, windows_metadata::Value::U32(value))) => *value,
                        _ => 0,
                    })
                    .unwrap_or(7);
                result.push(Export {
                    namespace: namespace.into(),
                    container: "Apis".into(),
                    name: method.name().into(),
                    dll: import.import_scope().name().into(),
                    entry_point: import.import_name().into(),
                    calling_convention: method.calling_convention().into(),
                    architectures,
                    method_flags: format!("{:?}", method.flags()),
                    impl_flags: format!("{:?}", method.impl_flags()),
                    import_flags: format!("{:?}", import.flags()),
                    signature_flags: format!("{:?}", signature.flags),
                    signature: hex_blob(method.blob(4)),
                    attributes: attributes(&method),
                    return_attributes: method
                        .params()
                        .find(|p| p.sequence() == 0)
                        .map(|p| attributes(&p))
                        .unwrap_or_default(),
                    return_type: native_type(&signature.return_type, &index, 0),
                    parameters: parameters
                        .iter()
                        .zip(&signature.types)
                        .map(|(p, typ)| Parameter {
                            name: p.name().into(),
                            sequence: p.sequence(),
                            input: p.flags().contains(windows_metadata::ParamAttributes::In),
                            output: p.flags().contains(windows_metadata::ParamAttributes::Out),
                            optional: p
                                .flags()
                                .contains(windows_metadata::ParamAttributes::Optional),
                            flags: format!("{:?}", p.flags()),
                            attributes: attributes(p),
                            const_attribute: p.has_attribute("ConstAttribute"),
                            reserved: p.has_attribute("ReservedAttribute"),
                            byte_count_parameter: p.find_attribute("MemorySizeAttribute").and_then(
                                |attribute| {
                                    attribute.value().into_iter().find_map(|(name, value)| {
                                        if name != "BytesParamIndex" {
                                            return None;
                                        }
                                        match value {
                                            windows_metadata::Value::I16(value) => {
                                                u16::try_from(value).ok()
                                            }
                                            windows_metadata::Value::U16(value) => Some(value),
                                            _ => None,
                                        }
                                    })
                                },
                            ),
                            typ: native_type(typ, &index, 0),
                        })
                        .collect(),
                    metadata_sha256: metadata_sha256.clone(),
                });
            }
        }
    }
    result.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_metadata_pinned_facts() {
        let Ok(paths) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let expected = [
            (
                "GetTickCount",
                "199E5B9170C2A222CF04949C7CBC37E1F336676AE98BAFAEE331CB55E011610D",
            ),
            (
                "GetTickCount64",
                "93EDCB91DF08DAA0AA42697B520295D6CA1FF2906EEA2CD9B896FF3C11CB35AC",
            ),
            (
                "RegOpenKeyExW",
                "FC5467309E37CCBB048F8468AA199A5313B44A1E23DB5603C4B93BD28C4AA3E1",
            ),
            (
                "RegQueryValueExW",
                "A5C297C41C6C5946003B1EA0F6858B8C640FDA4481FBC4091A0D84975703E048",
            ),
            (
                "RegCloseKey",
                "3344E3CE53F9FDF5B94746614FE9808B4F907AA6C5B0D662F4440C3AD4B16BCE",
            ),
        ];
        let mut found = 0;
        for namespace in ["SystemInformation", "Registry"] {
            for export in
                read_exports(&paths, &format!("Windows.Win32.System.{namespace}")).unwrap()
            {
                if let Some((_, fingerprint)) =
                    expected.iter().find(|(name, _)| *name == export.name)
                {
                    found += 1;
                    assert_eq!(export.fingerprint(), *fingerprint);
                    assert_eq!(export.calling_convention, "system");
                    assert_eq!(export.architectures, 7);
                    assert_eq!(
                        export.metadata_sha256,
                        "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D"
                    );
                    if export.name == "RegQueryValueExW" {
                        assert!(export.parameters[1].const_attribute);
                        assert!(export.parameters[2].reserved);
                        assert_eq!(export.parameters[4].byte_count_parameter, Some(5));
                        assert!(export.parameters[5].input && export.parameters[5].output);
                    }
                }
            }
        }
        assert_eq!(found, expected.len());
    }
}
