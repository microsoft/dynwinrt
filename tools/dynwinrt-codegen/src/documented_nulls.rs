// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Windows SDK members whose documented result can be null.
//!
//! WinRT metadata carries no nullability. `api-docs/windows-null-results.txt`
//! lists the doc comment IDs (`M:`/`P:` api-ids) of the Windows SDK methods
//! and properties whose documentation says the result can be null;
//! `scripts/extract-null-results.py` derives it from MicrosoftDocs/winrt-api.
//! A member is looked up by the type its documentation lists it under, its
//! CLR name, and its parameter types.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::meta::MethodMeta;
use crate::types::TypeMeta;

const TABLE: &str = include_str!("../api-docs/windows-null-results.txt");

struct Table {
    members: HashSet<String>,
    owners: HashSet<String>,
}

static DOCUMENTED: LazyLock<Table> = LazyLock::new(|| {
    let members = entries().map(normalize_api_id).collect::<HashSet<_>>();
    let owners = members.iter().filter_map(|id| owner_of(id)).collect();
    Table { members, owners }
});

/// The api-ids listed in the table, verbatim.
pub(crate) fn entries() -> impl Iterator<Item = &'static str> {
    TABLE
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Whether the documentation of `method`, listed under the type named
/// `owner` (`Namespace.Type`), says its result can be null.
pub(crate) fn documents_null_result(owner: &str, method: &MethodMeta) -> bool {
    DOCUMENTED.owners.contains(owner)
        && member_id(owner, method).is_some_and(|id| DOCUMENTED.members.contains(&id))
}

/// The normalized doc comment ID of `method` listed under `owner`. Only
/// methods and property getters produce results.
pub(crate) fn member_id(owner: &str, method: &MethodMeta) -> Option<String> {
    if method.is_property_getter {
        let property = method.raw_name.strip_prefix("get_")?;
        return Some(format!("P:{owner}.{property}"));
    }
    if method.is_property_setter || method.is_event_add || method.is_event_remove {
        return None;
    }
    let parameters = method
        .params
        .iter()
        .map(|parameter| doc_type_name(&parameter.typ))
        .collect::<Vec<_>>()
        .join(",");
    Some(format!("M:{owner}.{}({parameters})", method.raw_name))
}

/// Normalizes an api-id so that metadata can reproduce it: methods always
/// carry a parameter list, and parameter types lose by-reference markers,
/// modifiers and generic arguments.
pub(crate) fn normalize_api_id(api_id: &str) -> String {
    let Some(open) = api_id.find('(') else {
        return if api_id.starts_with("M:") {
            format!("{api_id}()")
        } else {
            api_id.to_string()
        };
    };
    let parameters = split_parameters(api_id[open + 1..].trim_end_matches(')'))
        .into_iter()
        .map(normalize_doc_type)
        .collect::<Vec<_>>()
        .join(",");
    format!("{}({parameters})", &api_id[..open])
}

fn owner_of(id: &str) -> Option<String> {
    let member = id.get(2..)?.split('(').next()?;
    member.rsplit_once('.').map(|(owner, _)| owner.to_string())
}

fn split_parameters(list: &str) -> Vec<&str> {
    let mut parameters = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (index, character) in list.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parameters.push(&list[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !list.is_empty() {
        parameters.push(&list[start..]);
    }
    parameters
}

fn normalize_doc_type(name: &str) -> String {
    let name = name.split('!').next().unwrap_or(name).trim_end_matches('@');
    let mut result = String::with_capacity(name.len());
    let mut depth = 0usize;
    for character in name.chars() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => result.push(character),
            _ => {}
        }
    }
    match result.split_once('`') {
        Some((definition, _)) => definition.to_string(),
        // Some pages spell this struct with its .NET projection.
        None if result == "System.Type" => "Windows.UI.Xaml.Interop.TypeName".to_string(),
        None => result,
    }
}

/// The doc comment ID name of a parameter type, after normalization.
fn doc_type_name(typ: &TypeMeta) -> String {
    let name = match typ {
        TypeMeta::Bool => "System.Boolean",
        TypeMeta::I8 => "System.SByte",
        TypeMeta::U8 => "System.Byte",
        TypeMeta::I16 => "System.Int16",
        TypeMeta::U16 => "System.UInt16",
        TypeMeta::I32 => "System.Int32",
        TypeMeta::U32 => "System.UInt32",
        TypeMeta::I64 => "System.Int64",
        TypeMeta::U64 => "System.UInt64",
        TypeMeta::F32 => "System.Single",
        TypeMeta::F64 => "System.Double",
        TypeMeta::Char16 => "System.Char",
        TypeMeta::String => "System.String",
        TypeMeta::Guid => "System.Guid",
        TypeMeta::Object => "System.Object",
        TypeMeta::AsyncAction => "Windows.Foundation.IAsyncAction",
        TypeMeta::AsyncActionWithProgress(_) => "Windows.Foundation.IAsyncActionWithProgress",
        TypeMeta::AsyncOperation(_) => "Windows.Foundation.IAsyncOperation",
        TypeMeta::AsyncOperationWithProgress(..) => {
            "Windows.Foundation.IAsyncOperationWithProgress"
        }
        TypeMeta::Array(inner) => return format!("{}[]", doc_type_name(inner)),
        TypeMeta::Interface {
            namespace, name, ..
        }
        | TypeMeta::RuntimeClass {
            namespace, name, ..
        }
        | TypeMeta::Delegate {
            namespace, name, ..
        }
        | TypeMeta::Struct {
            namespace, name, ..
        }
        | TypeMeta::Enum {
            namespace, name, ..
        }
        | TypeMeta::Parameterized {
            namespace, name, ..
        } => {
            let definition = name.split('`').next().unwrap_or(name);
            return format!("{namespace}.{definition}");
        }
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    const WINDOWS_WINMD: &str =
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

    #[test]
    fn api_ids_normalize_to_metadata_reproducible_keys() {
        assert_eq!(
            normalize_api_id("M:Windows.Devices.Sensors.Compass.GetDefault"),
            "M:Windows.Devices.Sensors.Compass.GetDefault()"
        );
        assert_eq!(
            normalize_api_id(
                "M:N.T.Find(Windows.Foundation.Collections.IMap{System.String,Windows.Foundation.Collections.IVector{System.String}},System.Byte[]@,System.Guid@!System.Runtime.CompilerServices.IsConst)"
            ),
            "M:N.T.Find(Windows.Foundation.Collections.IMap,System.Byte[],System.Guid)"
        );
        assert_eq!(normalize_api_id("P:N.T.Value"), "P:N.T.Value");
        assert_eq!(
            normalize_api_id("M:N.T.Get(System.Type)"),
            normalize_api_id("M:N.T.Get(Windows.UI.Xaml.Interop.TypeName)")
        );
        assert_eq!(owner_of("M:N.T.Find()").as_deref(), Some("N.T"));
    }

    #[test]
    fn member_ids_use_clr_names_and_parameter_types() {
        let method = MethodMeta {
            name: "GetDefaultWithAccelerometerReadingType".into(),
            raw_name: "GetDefault".into(),
            params: vec![
                ParamMeta {
                    name: "readingType".into(),
                    typ: TypeMeta::Enum {
                        namespace: "Windows.Devices.Sensors".into(),
                        name: "AccelerometerReadingType".into(),
                        underlying: Box::new(TypeMeta::I32),
                        members: vec![],
                        is_flags: false,
                        doc: None,
                        deprecated: None,
                    },
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "values".into(),
                    typ: TypeMeta::Array(Box::new(TypeMeta::Parameterized {
                        namespace: "Windows.Foundation.Collections".into(),
                        name: "IIterable`1".into(),
                        piid: String::new(),
                        args: vec![TypeMeta::String],
                    })),
                    direction: ParamDirection::Out,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            member_id("Windows.Devices.Sensors.Accelerometer", &method).as_deref(),
            Some(
                "M:Windows.Devices.Sensors.Accelerometer.GetDefault(Windows.Devices.Sensors.AccelerometerReadingType,Windows.Foundation.Collections.IIterable[])"
            )
        );
        let getter = MethodMeta {
            name: "get_Parent".into(),
            raw_name: "get_Parent".into(),
            is_property_getter: true,
            ..Default::default()
        };
        assert_eq!(
            member_id("Windows.UI.Xaml.FrameworkElement", &getter).as_deref(),
            Some("P:Windows.UI.Xaml.FrameworkElement.Parent")
        );
        let setter = MethodMeta {
            name: "put_Parent".into(),
            raw_name: "put_Parent".into(),
            is_property_setter: true,
            ..Default::default()
        };
        assert_eq!(member_id("N.T", &setter), None);
    }

    /// Every entry names a member of the Windows SDK metadata whose parsed
    /// method carries the documented-null fact. This guards the table against
    /// typos and drift, and the key derivation against metadata changes.
    #[test]
    fn every_entry_resolves_to_a_flagged_windows_sdk_member() {
        // Documented, but absent from the 10.0.26100 SDK metadata: newer APIs,
        // and AllJoyn, which the SDK dropped.
        const ABSENT_FROM_TEST_SDK: [&str; 3] = [
            "M:Windows.Devices.AllJoyn.AllJoynServiceInfo.FromIdAsync(System.String)",
            "M:Windows.Gaming.UI.GameMonitor.GetDefault()",
            "M:Windows.System.User.GetUserAgeRangeAsync()",
        ];
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping: Windows.winmd not found");
            return;
        }
        let index = crate::meta::load_index(WINDOWS_WINMD).expect("Windows.winmd index");
        let mut by_owner = BTreeMap::<String, BTreeSet<String>>::new();
        for entry in entries() {
            let id = normalize_api_id(entry);
            let owner = owner_of(&id).expect("owner");
            by_owner.entry(owner).or_default().insert(id);
        }
        let mut unresolved = Vec::new();
        let mut unflagged = Vec::new();
        for (owner, ids) in &by_owner {
            let (namespace, name) = owner.rsplit_once('.').expect("qualified owner");
            let methods = crate::meta::documented_owner_methods(&index, namespace, name);
            for id in ids {
                let matches = methods
                    .iter()
                    .filter(|method| member_id(owner, method).as_deref() == Some(id))
                    .collect::<Vec<_>>();
                if ABSENT_FROM_TEST_SDK.contains(&id.as_str()) {
                    let member = id[2..]
                        .split('(')
                        .next()
                        .unwrap()
                        .rsplit('.')
                        .next()
                        .unwrap();
                    assert!(
                        !methods.iter().any(|method| method.raw_name == member),
                        "{id} is present in the test SDK; drop it from ABSENT_FROM_TEST_SDK"
                    );
                } else if matches.is_empty() {
                    unresolved.push(id.clone());
                } else if !matches.iter().any(|method| method.documented_null_result) {
                    unflagged.push(id.clone());
                }
            }
        }
        assert!(unresolved.is_empty(), "unresolved entries: {unresolved:#?}");
        assert!(unflagged.is_empty(), "entries not applied: {unflagged:#?}");
        assert!(by_owner.values().map(BTreeSet::len).sum::<usize>() > 900);
    }
}
