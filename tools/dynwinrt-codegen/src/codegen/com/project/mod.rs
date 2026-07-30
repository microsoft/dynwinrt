// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod interop;
pub(super) mod types;

use crate::com_metadata::{ComEnumValue, ComInterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use crate::types::TypeMeta;

use super::ir::{
    ActivationPlan, ComParamDirection, ComReturnConvention, ComType, ProjectedComEnum,
    ProjectedComEnumMember, ProjectedComInterface, ProjectedComMethod, ProjectedComMethodKind,
    ProjectedComParam, ProjectedComResult, ProjectedEnumValue, ResultConversion, ResultSource,
    StringBufferPlan, StringEncoding, UnsupportedComType,
};
use super::javascript::naming::camel_case;
use interop::resolve_projected_default_iid;
use types::{is_scalar_in_out, is_supported_direct_return, project_enum_underlying, project_type};

pub(super) fn project_com_interface(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<ProjectedComInterface, String> {
    let interop_target = detect_interop_target(meta, winmd_paths)?;
    let methods = meta
        .interface
        .methods
        .iter()
        .map(|method| project_method(meta, method, interop_target.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    let activation = if let Some((class_name, class_namespace, target_iid)) = interop_target {
        ActivationPlan::WinRtFactory {
            class_name,
            class_namespace,
            target_iid,
        }
    } else if let Some(clsid) = &meta.coclass_clsid {
        ActivationPlan::Coclass {
            clsid: clsid.clone(),
            coclass_name: meta
                .coclass_name
                .clone()
                .unwrap_or_else(|| "Coclass".into()),
        }
    } else {
        ActivationPlan::None
    };
    let referenced_enums = meta
        .referenced_enums
        .iter()
        .map(|en| {
            Ok(ProjectedComEnum {
                name: en.name.clone(),
                underlying: project_enum_underlying(&en.underlying).map_err(|unsupported| {
                    unsupported_error(unsupported, "enum underlying type")
                })?,
                members: en
                    .members
                    .iter()
                    .map(|member| ProjectedComEnumMember {
                        name: member.name.clone(),
                        value: match member.value {
                            ComEnumValue::Signed(value) => ProjectedEnumValue::Signed(value),
                            ComEnumValue::Unsigned(value) => ProjectedEnumValue::Unsigned(value),
                        },
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ProjectedComInterface {
        name: meta.interface.name.clone(),
        namespace: meta.interface.namespace.clone(),
        iid: meta.interface.iid.clone(),
        is_iunknown_rooted: meta.is_iunknown_rooted,
        methods,
        activation,
        referenced_enums,
    })
}

fn project_method(
    interface: &ComInterfaceMeta,
    method: &MethodMeta,
    interop_target: Option<&(String, String, String)>,
) -> Result<ProjectedComMethod, String> {
    let context = || format!("{}.{}", interface.interface.name, method.name);
    let dynamic_natural_count = dynamic_iid_natural_param_count(method);
    if dynamic_natural_count.is_some() && method.preserve_hresult {
        return Err(format!(
            "{}: semantic HRESULT dynamic-IID methods are not supported",
            context()
        ));
    }
    let kind = match (interop_target, method.name.as_str(), dynamic_natural_count) {
        (Some((_, _, target_iid)), "GetForWindow", Some(natural_param_count)) => {
            ProjectedComMethodKind::SynthesizedGetForWindow {
                natural_param_count,
                target_iid: target_iid.clone(),
            }
        }
        (_, _, Some(natural_param_count)) => ProjectedComMethodKind::CallerSuppliedDynamicIid {
            natural_param_count,
        },
        _ => ProjectedComMethodKind::Normal,
    };

    let mut params = Vec::with_capacity(method.params.len());
    for (index, param) in method.params.iter().enumerate() {
        match param.direction {
            ParamDirection::UnsupportedNativeArray { count_param_index } => {
                let count = count_param_index
                    .map(|index| format!("parameter index {index}"))
                    .unwrap_or_else(|| "metadata-defined size".into());
                return Err(format!(
                    "{}: caller-sized native buffers are not supported (`{}` uses {count})",
                    context(),
                    param.name
                ));
            }
            ParamDirection::OutFill => {
                return Err(format!(
                    "{}: caller-allocated array outputs are not supported",
                    context()
                ));
            }
            _ => {}
        }
        let typ = project_type(&param.typ).map_err(|unsupported| {
            unsupported_error(
                unsupported,
                &format!("{} parameter `{}`", context(), param.name),
            )
        })?;
        let cleanup = cleanup_for_param(method, index, &context())?;
        if cleanup.is_some() && dynamic_natural_count.is_some() && index == method.params.len() - 1
        {
            return Err(format!(
                "{}: dynamic-IID interface output cannot declare an allocator cleanup contract",
                context()
            ));
        }
        if cleanup.is_some() && param.direction == ParamDirection::InOut {
            return Err(format!(
                "{}: allocator ownership transfer for [in, out] parameter `{}` is not supported",
                context(),
                param.name
            ));
        }
        if param.direction == ParamDirection::InOut && !is_scalar_in_out(&typ) {
            return Err(format!(
                "{}: unsupported [in, out] parameter `{}` of type {:?}",
                context(),
                param.name,
                param.typ
            ));
        }
        if param.direction == ParamDirection::Out
            && typ == ComType::RawPointer
            && dynamic_natural_count.is_none()
            && cleanup != Some(CleanupKind::CoTaskMemFree)
            && cleanup.is_none()
        {
            return Err(unsupported_error(
                UnsupportedComType::UnknownOwnership {
                    type_name: "untyped pointer output".into(),
                },
                &context(),
            ));
        }
        if param.direction == ParamDirection::Out
            && typ == ComType::Bstr
            && cleanup != Some(CleanupKind::SysFreeString)
            && cleanup.is_none()
        {
            return Err(unsupported_error(
                UnsupportedComType::UnknownOwnership {
                    type_name: "BSTR output".into(),
                },
                &context(),
            ));
        }
        if matches!(param.direction, ParamDirection::Out | ParamDirection::InOut)
            && matches!(
                typ,
                ComType::PointerAlias {
                    kind: super::ir::PointerAliasKind::StringPointer,
                    ..
                }
            )
            && cleanup != Some(CleanupKind::CoTaskMemFree)
            && cleanup.is_none()
        {
            return Err(unsupported_error(
                UnsupportedComType::UnknownOwnership {
                    type_name: format!("string pointer output `{}`", param.name),
                },
                &context(),
            ));
        }
        params.push(ProjectedComParam {
            name: param.name.clone(),
            typ,
            direction: match param.direction {
                ParamDirection::In => ComParamDirection::In,
                ParamDirection::Out => ComParamDirection::Out,
                ParamDirection::InOut => ComParamDirection::InOut,
                ParamDirection::OutStringBuffer { .. } => ComParamDirection::OutStringBuffer,
                ParamDirection::OutFill | ParamDirection::UnsupportedNativeArray { .. } => {
                    unreachable!("unsupported directions returned above")
                }
            },
        });
    }
    validate_owned_outputs(method, &params, &context())?;

    let return_convention = match &method.return_type {
        None => ComReturnConvention::Void,
        Some(typ) => {
            let projected = project_type(typ).map_err(|unsupported| match unsupported {
                UnsupportedComType::NativeStructLayout { .. } | UnsupportedComType::Unknown => {
                    unsupported_error(
                        UnsupportedComType::UnsupportedDirectReturn {
                            type_name: format!("{typ:?}"),
                        },
                        &context(),
                    )
                }
                unsupported => {
                    unsupported_error(unsupported, &format!("{} return value", context()))
                }
            })?;
            if projected == ComType::HResult {
                if method.preserve_hresult {
                    ComReturnConvention::SemanticHResult
                } else {
                    ComReturnConvention::HResult
                }
            } else if is_supported_direct_return(&projected) {
                ComReturnConvention::Direct(projected)
            } else {
                return Err(unsupported_error(
                    UnsupportedComType::UnsupportedDirectReturn {
                        type_name: format!("{typ:?}"),
                    },
                    &context(),
                ));
            }
        }
    };
    if method.preserve_hresult && !matches!(return_convention, ComReturnConvention::SemanticHResult)
    {
        return Err(format!(
            "{}: semantic HRESULT metadata requires an HRESULT return",
            context()
        ));
    }

    let string_buffer = project_string_buffer(method)?;
    if params
        .iter()
        .any(|param| param.direction == ComParamDirection::OutStringBuffer)
        && string_buffer.is_none()
    {
        return Err(format!(
            "{}: unsupported string-buffer encoding or count relationship",
            context()
        ));
    }
    let mut results = Vec::new();
    if let ComReturnConvention::SemanticHResult | ComReturnConvention::Direct(_) =
        &return_convention
    {
        let typ = match &return_convention {
            ComReturnConvention::SemanticHResult => ComType::HResult,
            ComReturnConvention::Direct(typ) => typ.clone(),
            ComReturnConvention::HResult | ComReturnConvention::Void => unreachable!(),
        };
        results.push(ProjectedComResult {
            conversion: result_conversion(&typ, method, None, &kind),
            typ,
            source: ResultSource::DirectReturn,
        });
    }
    for (index, param) in params.iter().enumerate() {
        if matches!(
            param.direction,
            ComParamDirection::Out | ComParamDirection::InOut
        ) {
            results.push(ProjectedComResult {
                typ: param.typ.clone(),
                source: ResultSource::Param(index),
                conversion: result_conversion(&param.typ, method, Some(index), &kind),
            });
        }
    }

    Ok(ProjectedComMethod {
        name: method.name.clone(),
        camel_name: camel_case(&method.name),
        vtable_index: method.vtable_index,
        params,
        return_convention,
        results,
        string_buffer,
        kind,
        doc: method.doc.clone(),
    })
}

fn result_conversion(
    typ: &ComType,
    method: &MethodMeta,
    param_index: Option<usize>,
    kind: &ProjectedComMethodKind,
) -> ResultConversion {
    if matches!(
        kind,
        ProjectedComMethodKind::CallerSuppliedDynamicIid { .. }
            | ProjectedComMethodKind::SynthesizedGetForWindow { .. }
    ) && param_index == Some(method.params.len() - 1)
    {
        return ResultConversion::DynamicIidAdoption;
    }
    if let Some(index) = param_index {
        let cleanup = cleanup_for_param(method, index, &method.name)
            .expect("cleanup contract was validated during projection");
        if cleanup == Some(CleanupKind::SysFreeString) && matches!(typ, ComType::Bstr) {
            return ResultConversion::Bstr;
        }
        if cleanup == Some(CleanupKind::CoTaskMemFree) {
            return match typ {
                ComType::PointerAlias { name, .. } if name == "PWSTR" => {
                    ResultConversion::CoTaskMemString(StringEncoding::Wide)
                }
                ComType::PointerAlias { name, .. } if name == "PSTR" => {
                    ResultConversion::CoTaskMemString(StringEncoding::Ansi)
                }
                _ => ResultConversion::CoTaskMemData,
            };
        }
    }
    match typ {
        ComType::ManagedInterface { .. } => ResultConversion::ManagedCom,
        ComType::HString => ResultConversion::HString,
        _ => ResultConversion::Value,
    }
}

fn validate_owned_outputs(
    method: &MethodMeta,
    params: &[ProjectedComParam],
    context: &str,
) -> Result<(), String> {
    for (index, param) in params.iter().enumerate() {
        let Some(cleanup) = cleanup_for_param(method, index, context)? else {
            continue;
        };
        if !matches!(
            param.direction,
            ComParamDirection::Out | ComParamDirection::InOut
        ) {
            return Err(format!(
                "{context}: ownership metadata applies to non-output parameter `{}`",
                param.name
            ));
        }
        if cleanup == CleanupKind::SysFreeString {
            if param.direction != ComParamDirection::Out || param.typ != ComType::Bstr {
                return Err(format!(
                    "{context}: SysFreeString ownership requires a scalar Out BSTR"
                ));
            }
        } else if cleanup == CleanupKind::CoTaskMemFree {
            if param.direction != ComParamDirection::Out
                || !matches!(
                    param.typ,
                    ComType::RawPointer
                        | ComType::PointerAlias {
                            kind: super::ir::PointerAliasKind::DataPointer
                                | super::ir::PointerAliasKind::StringPointer,
                            ..
                        }
                )
            {
                return Err(format!(
                    "{context}: CoTaskMemFree ownership requires an Out data or string pointer"
                ));
            }
        }
    }
    for owned in &method.owned_outputs {
        if owned.param_index >= params.len() {
            return Err(format!(
                "{context}: ownership metadata references missing parameter index {}",
                owned.param_index
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupKind {
    SysFreeString,
    CoTaskMemFree,
}

fn cleanup_for_param(
    method: &MethodMeta,
    param_index: usize,
    context: &str,
) -> Result<Option<CleanupKind>, String> {
    let contracts = method
        .owned_outputs
        .iter()
        .filter(|owned| owned.param_index == param_index)
        .map(|owned| owned.free_with.as_str())
        .collect::<Vec<_>>();
    match contracts.as_slice() {
        [] => Ok(None),
        [contract] => parse_cleanup_contract(contract)
            .map(Some)
            .ok_or_else(|| format!("{context}: unsupported output cleanup contract `{contract}`")),
        _ => Err(format!(
            "{context}: output parameter index {param_index} has multiple cleanup contracts"
        )),
    }
}

fn parse_cleanup_contract(contract: &str) -> Option<CleanupKind> {
    if contract.is_empty()
        || contract != contract.trim()
        || contract
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, ',' | ';'))
    {
        return None;
    }
    let identifier = contract
        .rsplit_once(|character: char| matches!(character, '.' | '!' | ':'))
        .map_or(contract, |(_, identifier)| identifier);
    if identifier.is_empty() {
        return None;
    }
    match identifier {
        "SysFreeString" => Some(CleanupKind::SysFreeString),
        "CoTaskMemFree" => Some(CleanupKind::CoTaskMemFree),
        _ => None,
    }
}

fn project_string_buffer(method: &MethodMeta) -> Result<Option<StringBufferPlan>, String> {
    let buffers = method
        .params
        .iter()
        .enumerate()
        .filter_map(|(buffer_param_index, param)| {
            let ParamDirection::OutStringBuffer { count_param_index } = param.direction else {
                return None;
            };
            Some((buffer_param_index, count_param_index, param))
        })
        .collect::<Vec<_>>();
    if buffers.len() > 1 {
        return Err(format!(
            "{}: multiple caller-owned string buffers are not supported",
            method.name
        ));
    }
    let plan =
        buffers
            .into_iter()
            .next()
            .and_then(|(buffer_param_index, count_param_index, param)| {
                let encoding = match pointer_alias_name(&param.typ) {
                    Some("PWSTR") => StringEncoding::Wide,
                    Some("PSTR") => StringEncoding::Ansi,
                    _ => return None,
                };
                method
                    .params
                    .get(count_param_index)
                    .filter(|count| count.direction == ParamDirection::In)?;
                let optional_param_indices = (count_param_index..method.params.len())
                    .filter(|index| string_buffer_param_is_optional(method, *index))
                    .collect();
                Some(StringBufferPlan {
                    buffer_param_index,
                    count_param_index,
                    encoding,
                    optional_param_indices,
                })
            });
    if plan
        .as_ref()
        .is_some_and(|plan| plan.encoding == StringEncoding::Ansi)
    {
        return Err(format!(
            "{}: caller-owned ANSI output buffers are not yet decoded safely",
            method.name
        ));
    }
    Ok(plan)
}

fn string_buffer_param_is_optional(method: &MethodMeta, param_index: usize) -> bool {
    let Some((_, count_index)) =
        method
            .params
            .iter()
            .enumerate()
            .find_map(|(buffer_index, param)| match param.direction {
                ParamDirection::OutStringBuffer { count_param_index } => {
                    Some((buffer_index, count_param_index))
                }
                _ => None,
            })
    else {
        return false;
    };
    let Some(param) = method.params.get(param_index) else {
        return false;
    };
    let optional_shape =
        param_index == count_index || (param_index > count_index && is_optional_find_data(param));
    optional_shape
        && method
            .params
            .iter()
            .skip(param_index + 1)
            .filter(|param| param.direction.is_input())
            .all(is_optional_find_data)
}

fn is_optional_find_data(param: &ParamMeta) -> bool {
    if !matches!(param.direction, ParamDirection::In | ParamDirection::Out) {
        return false;
    }
    let name = param.name.to_ascii_lowercase();
    name == "pfd"
        || name.contains("finddata")
        || name.contains("find_data")
        || matches!(
            &param.typ,
            TypeMeta::Struct { name, .. }
                if name == "WIN32_FIND_DATAW" || name == "WIN32_FIND_DATAA"
        )
}

fn pointer_alias_name(typ: &TypeMeta) -> Option<&str> {
    match typ {
        TypeMeta::Struct { name, .. } => Some(name),
        _ => None,
    }
}

fn dynamic_iid_natural_param_count(method: &MethodMeta) -> Option<usize> {
    if !method
        .return_type
        .as_ref()
        .is_some_and(|typ| matches!(project_type(typ), Ok(ComType::HResult)))
        || method.params.len() < 2
    {
        return None;
    }
    let output = method.params.last()?;
    if output.direction != ParamDirection::Out || output.typ != TypeMeta::Object {
        return None;
    }
    let iid = &method.params[method.params.len() - 2];
    let iid_name = iid.name.to_ascii_lowercase();
    if iid.direction != ParamDirection::In
        || iid.typ != TypeMeta::Object
        || !matches!(iid_name.as_str(), "iid" | "riid")
        || method.params[..method.params.len() - 2]
            .iter()
            .any(|param| param.direction != ParamDirection::In)
    {
        return None;
    }
    Some(method.params.len() - 2)
}

fn detect_interop_target(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<Option<(String, String, String)>, String> {
    if !meta.interface.name.ends_with("Interop")
        || !meta.interface.methods.iter().any(|method| {
            method.name == "GetForWindow" && dynamic_iid_natural_param_count(method).is_some()
        })
    {
        return Ok(None);
    }
    let stripped_i = meta
        .interface
        .name
        .strip_prefix('I')
        .unwrap_or(&meta.interface.name);
    let class_name = stripped_i
        .strip_suffix("Interop")
        .unwrap_or(stripped_i)
        .to_string();
    let Some((namespace, _, iid)) = resolve_projected_default_iid(winmd_paths, &class_name) else {
        return Err(format!(
            "Classic-COM interop generator: cannot resolve default IID for the projected \
             WinRT runtime class `{class_name}` (derived from `{}`). \
             Neither the winmds passed to the generator ({winmd_paths:?}) nor the newest installed \
             `C:\\Program Files (x86)\\Windows Kits\\10\\UnionMetadata\\<version>\\Windows.winmd` \
             contains a WinRT runtime class of that name with a resolvable default interface. \
             Pass the correct Windows.winmd via --ref or install a recent Windows SDK.",
            meta.interface.name
        ));
    };
    Ok(Some((class_name, namespace, iid)))
}

fn unsupported_error(unsupported: UnsupportedComType, context: &str) -> String {
    match unsupported {
        UnsupportedComType::Array => format!(
            "{context}: native arrays require an explicit count and element-ownership projection; \
             raw-pointer fallback is not allowed"
        ),
        UnsupportedComType::ParameterizedInterface { namespace, name } => format!(
            "{context}: parameterized interface `{namespace}.{name}` requires a computed closed IID \
             and managed ownership projection; raw-pointer fallback is not allowed"
        ),
        UnsupportedComType::AsyncInterface => format!(
            "{context}: async interface requires a computed closed IID and managed ownership \
             projection; raw-pointer fallback is not allowed"
        ),
        UnsupportedComType::Delegate { namespace, name } => format!(
            "{context}: delegate `{namespace}.{name}` requires a managed callback projection; \
             raw-pointer fallback is not allowed"
        ),
        UnsupportedComType::NativeStructLayout { namespace, name } => {
            format!("{context}: struct `{namespace}.{name}` requires native layout projection")
        }
        UnsupportedComType::UnknownPointerAlias { namespace, name } => format!(
            "{context}: pointer-shaped typedef `{namespace}.{name}` has no explicit semantic \
             classification; raw-pointer fallback is not allowed"
        ),
        UnsupportedComType::UnresolvedInterface { namespace, name } => format!(
            "{context}: interface `{namespace}.{name}` has no resolvable IID; \
             pass the metadata that defines it via --ref instead of projecting it as a raw pointer"
        ),
        UnsupportedComType::UnresolvedRuntimeClass { namespace, name } => format!(
            "{context}: runtime class `{namespace}.{name}` has no resolvable default interface; \
             pass the metadata that defines it via --ref"
        ),
        UnsupportedComType::UnknownOwnership { type_name } => {
            format!("{context}: {type_name} has no ownership projection")
        }
        UnsupportedComType::UnsupportedDirectReturn { type_name } => {
            format!("{context}: unsupported direct native return type {type_name}")
        }
        UnsupportedComType::Unknown => {
            format!("{context}: unsupported Classic-COM type")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com_metadata::{InterfaceMeta, MethodMeta, ParamMeta};

    fn interface(method: MethodMeta) -> ComInterfaceMeta {
        ComInterfaceMeta {
            interface: InterfaceMeta {
                name: "ITest".into(),
                namespace: "Tests".into(),
                iid: "00000000-0000-0000-0000-000000000001".into(),
                methods: vec![method],
                ..Default::default()
            },
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: vec!["IUnknown".into()],
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
        }
    }

    #[test]
    fn unsupported_type_fails_during_projection() {
        let method = MethodMeta {
            name: "Bad".into(),
            params: vec![ParamMeta {
                name: "values".into(),
                typ: TypeMeta::Array(Box::new(TypeMeta::I32)),
                direction: ParamDirection::In,
            }],
            ..Default::default()
        };
        assert!(
            project_com_interface(&interface(method), "")
                .unwrap_err()
                .contains("raw-pointer fallback is not allowed")
        );
    }

    #[test]
    fn by_value_guid_is_not_dynamic_iid() {
        let method = MethodMeta {
            name: "Get".into(),
            params: vec![
                ParamMeta {
                    name: "riid".into(),
                    typ: TypeMeta::Guid,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "result".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(TypeMeta::Struct {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HRESULT".into(),
                fields: Vec::new(),
            }),
            ..Default::default()
        };
        assert!(
            project_com_interface(&interface(method), "")
                .unwrap_err()
                .contains("untyped pointer output")
        );
    }

    #[test]
    fn owned_output_without_a_known_cleanup_fails_before_rendering() {
        let method = MethodMeta {
            name: "GetName".into(),
            params: vec![ParamMeta {
                name: "name".into(),
                typ: TypeMeta::Struct {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "BSTR".into(),
                    fields: Vec::new(),
                },
                direction: ParamDirection::Out,
            }],
            return_type: Some(TypeMeta::Struct {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HRESULT".into(),
                fields: Vec::new(),
            }),
            ..Default::default()
        };
        assert!(
            project_com_interface(&interface(method), "")
                .unwrap_err()
                .contains("BSTR output has no ownership projection")
        );
    }
}
