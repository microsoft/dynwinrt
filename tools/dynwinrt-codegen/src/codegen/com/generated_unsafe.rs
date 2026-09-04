// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::com_metadata::{
    ComInterfaceMeta, RawComMethod, RawComType, RawExactInterfaceOutputCallContract,
    RawNativeLayoutSet, RawNativeType, RawParamDirection,
};

use super::capability::{
    MetadataFileIdentity, MetadataSetIdentity, MethodCapability, RawClassification,
    classify_interface_methods, metadata_set_identity_for_paths, parameter_manual_reasons,
    parameter_pointee_layouts, raw_aggregate_descriptor,
};
use super::javascript::naming::camel_case;

pub const UNSAFE_SUPPORT_SCHEMA_VERSION: u32 = 11;

#[derive(Debug, Clone)]
pub struct UnsafeGeneratedOutput {
    pub class_name: String,
    pub module_path: String,
    pub js: Option<String>,
    pub dts: Option<String>,
    pub support: UnsafeInterfaceSupport,
    pub metadata_complete_methods: usize,
    pub manual_methods: usize,
    pub blocked_methods: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage2Coverage {
    pub x64_manual_interfaces: usize,
    pub x64_manual_interfaces_with_executable_method: usize,
    pub x64_manual_interfaces_with_executable_manual_method: usize,
    pub executable_manual_methods: usize,
    pub remaining_manual_methods: usize,
    pub runtime_blocked_methods: usize,
}

pub fn measure_stage2_coverage(winmd_paths: &str) -> Result<Stage2Coverage, String> {
    let metadata = metadata_set_identity_for_paths(winmd_paths)?;
    let mut interfaces = crate::com_metadata::parse_all_com_interfaces(winmd_paths)
        .ok_or_else(|| format!("Failed to load Classic COM metadata from {winmd_paths}"))?
        .into_iter()
        .filter(|interface| {
            (interface.is_iunknown_rooted || interface.interface.name.ends_with("Interop"))
                && !(interface.interface.namespace == "Windows.Win32.UI.Controls.RichEdit"
                    && interface.interface.name == "ITextHost2")
        })
        .collect::<Vec<_>>();
    interfaces.sort_by(|left, right| {
        format!("{}.{}", left.interface.namespace, left.interface.name).cmp(&format!(
            "{}.{}",
            right.interface.namespace, right.interface.name
        ))
    });
    let mut coverage = Stage2Coverage {
        x64_manual_interfaces: 0,
        x64_manual_interfaces_with_executable_method: 0,
        x64_manual_interfaces_with_executable_manual_method: 0,
        executable_manual_methods: 0,
        remaining_manual_methods: 0,
        runtime_blocked_methods: 0,
    };
    for interface in &interfaces {
        if super::generate_com_interface_files(interface, winmd_paths).is_ok() {
            continue;
        }
        let methods = classify_interface_methods(interface)?;
        let x64_manual = methods.iter().all(|method| {
            method.targets["x64"].classification != RawClassification::RawRuntimeBlocked
        }) && methods.iter().any(|method| {
            method.targets["x64"].classification == RawClassification::RawManualContract
        });
        if x64_manual {
            coverage.x64_manual_interfaces += 1;
        }
        let output = generate_unsafe_interface_files_with_metadata(interface, &metadata)?;
        coverage.executable_manual_methods += output.manual_methods;
        coverage.runtime_blocked_methods += output.blocked_methods;
        if x64_manual && output.js.is_some() {
            coverage.x64_manual_interfaces_with_executable_method += 1;
        }
        if x64_manual && output.manual_methods > 0 {
            coverage.x64_manual_interfaces_with_executable_manual_method += 1;
        }
    }
    Ok(coverage)
}

pub fn windows_relative_path_key(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains(':')
        || path.contains('\0')
        || !path.is_ascii()
    {
        return Err(format!("Unsafe Windows relative path is invalid: `{path}`"));
    }
    let normalized = path.replace('\\', "/");
    let mut key = Vec::new();
    for segment in normalized.split('/') {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || is_windows_reserved_name(segment)
        {
            return Err(format!(
                "Unsafe Windows path segment is invalid or reserved: `{segment}` in `{path}`"
            ));
        }
        key.push(segment.to_ascii_lowercase());
    }
    Ok(key.join("/"))
}

fn is_windows_reserved_name(segment: &str) -> bool {
    let base = segment
        .split_once('.')
        .map_or(segment, |(base, _)| base)
        .to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || matches!(
        base.strip_prefix("COM"),
        Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
    ) || matches!(
        base.strip_prefix("LPT"),
        Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
    )
}

fn unsafe_module_path(namespace: &str, class_name: &str) -> Result<String, String> {
    super::canonical_module_path(namespace, class_name)
}

pub fn validate_unsafe_supports(supports: &[UnsafeInterfaceSupport]) -> Result<(), String> {
    let mut identities = BTreeMap::<String, &str>::new();
    for support in supports {
        if support.schema_version != UNSAFE_SUPPORT_SCHEMA_VERSION {
            return Err(format!(
                "Unsupported generated unsafe interface schema {} for {}",
                support.schema_version, support.interface_name
            ));
        }
        let (namespace, name) = support.interface_name.rsplit_once('.').ok_or_else(|| {
            format!(
                "Generated unsafe interface identity is not qualified: `{}`",
                support.interface_name
            )
        })?;
        let expected_class = format!("{name}Unsafe");
        if support.unsafe_class != expected_class {
            return Err(format!(
                "Generated unsafe class `{}` does not match interface `{}`",
                support.unsafe_class, support.interface_name
            ));
        }
        let expected_path = unsafe_module_path(namespace, &expected_class)?;
        if support.module_path != expected_path {
            return Err(format!(
                "Retained unsafe modulePath `{}` does not match derived path `{expected_path}` for {}",
                support.module_path, support.interface_name
            ));
        }
        let key = windows_relative_path_key(&support.module_path)?;
        if let Some(existing) = identities.insert(key.clone(), &support.interface_name) {
            return Err(format!(
                "Unsafe interface identities `{existing}` and `{}` collide on case-insensitive Windows path key `{key}`",
                support.interface_name
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeInterfaceSupport {
    pub schema_version: u32,
    pub metadata: UnsafeMetadataSupport,
    pub interface_name: String,
    pub interface_iid: String,
    pub root: String,
    pub base_iids: Vec<String>,
    pub unsafe_class: String,
    pub module_path: String,
    pub methods: Vec<UnsafeMethodSupport>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeMetadataSupport {
    pub set_sha256: String,
    pub files: Vec<MetadataFileIdentity>,
    pub defining_file: Option<MetadataFileIdentity>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeMethodSupport {
    pub name: String,
    pub projected_name: String,
    pub declaring_iid: String,
    pub absolute_slot: usize,
    pub signature_fingerprint: String,
    pub status: String,
    pub reasons: Vec<String>,
    pub strategy_requirements: Vec<UnsafeStrategyRequirement>,
    #[serde(default)]
    pub exact_interface_outputs: Vec<UnsafeExactInterfaceOutput>,
    #[serde(default)]
    pub exact_interface_output_call: Option<UnsafeExactInterfaceOutputCall>,
    pub targets: BTreeMap<String, super::capability::TargetCapability>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeExactInterfaceOutput {
    pub entry_id: String,
    pub family_id: String,
    pub contract_kind: String,
    pub parameter_index: usize,
    pub parameter_name: String,
    pub interface_iid: String,
    pub argument_optional: bool,
    pub nullable_on_success: bool,
    pub reason: String,
    pub citation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeExactInterfaceOutputCall {
    pub entry_id: String,
    pub family_id: String,
    pub contract_kind: String,
    pub source_fingerprint: String,
    pub flags_param_index: usize,
    pub context_param_index: usize,
    pub synchronous_output_param_index: usize,
    pub semisynchronous_output_param_index: usize,
    pub synchronous_flags: i32,
    pub semisynchronous_flag_value: i32,
    pub flags_option_name: String,
    pub synchronous_output_option_name: String,
    pub semisynchronous_output_option_name: String,
    pub reason: String,
    pub citation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeStrategyRequirement {
    pub parameter_index: Option<usize>,
    pub parameter_name: Option<String>,
    pub strategy: String,
    pub reasons: Vec<String>,
    pub direction: Option<String>,
    pub nullable: Option<bool>,
    pub pointee_layouts: Option<BTreeMap<String, Option<UnsafeNativeLayout>>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsafeNativeLayout {
    pub size: usize,
    pub alignment: usize,
}

pub fn generate_unsafe_interface_files(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<UnsafeGeneratedOutput, String> {
    let metadata = metadata_set_identity_for_paths(winmd_paths)?;
    generate_unsafe_interface_files_with_metadata(meta, &metadata)
}

pub fn generate_unsafe_interface_files_with_metadata(
    meta: &ComInterfaceMeta,
    metadata: &MetadataSetIdentity,
) -> Result<UnsafeGeneratedOutput, String> {
    let raw_methods = meta.raw_methods.as_ref().ok_or_else(|| {
        format!(
            "{}.{} has no complete raw method metadata",
            meta.interface.namespace, meta.interface.name
        )
    })?;
    let capabilities = classify_interface_methods(meta)?;
    let class_name = format!("{}Unsafe", meta.interface.name);
    let module_path = unsafe_module_path(&meta.interface.namespace, &class_name)?;
    let mut support_methods = Vec::with_capacity(capabilities.len());
    let mut executable = Vec::new();
    let mut metadata_complete_methods = 0usize;
    let mut manual_methods = 0usize;
    let mut blocked_methods = 0usize;
    for (method, capability) in raw_methods.iter().zip(capabilities) {
        let status = overall_status(&capability);
        match status {
            RawClassification::RawMetadataComplete => {
                metadata_complete_methods += 1;
                executable.push((method, capability.clone()));
            }
            RawClassification::RawManualContract => {
                manual_methods += 1;
                executable.push((method, capability.clone()));
            }
            RawClassification::RawRuntimeBlocked => blocked_methods += 1,
        }
        let reasons = capability
            .targets
            .values()
            .flat_map(|target| {
                target
                    .blocker_reasons
                    .iter()
                    .chain(&target.manual_contract_reasons)
            })
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let strategy_requirements = if status == RawClassification::RawManualContract {
            manual_strategy_requirements(method, &reasons)?
        } else {
            Vec::new()
        };
        support_methods.push(UnsafeMethodSupport {
            name: capability.name.clone(),
            projected_name: camel_case(&capability.projected_name),
            declaring_iid: capability.declaring_iid.clone(),
            absolute_slot: capability.absolute_slot,
            signature_fingerprint: capability.signature_fingerprint.clone(),
            status: status.key().into(),
            strategy_requirements,
            exact_interface_outputs: method
                .params
                .iter()
                .enumerate()
                .filter_map(|(parameter_index, parameter)| {
                    let contract = parameter.exact_interface_output.as_ref()?;
                    let crate::com_metadata::RawEvidence::ExactRegistry {
                        entry_id,
                        family_id,
                        contract_kind,
                        reason,
                        citation,
                    } = &contract.evidence
                    else {
                        return None;
                    };
                    Some(UnsafeExactInterfaceOutput {
                        entry_id: entry_id.clone(),
                        family_id: family_id.id().into(),
                        contract_kind: contract_kind.key().into(),
                        parameter_index,
                        parameter_name: parameter.name.clone(),
                        interface_iid: contract.interface_iid.clone(),
                        argument_optional: contract.argument_optional,
                        nullable_on_success: contract.nullable_on_success,
                        reason: reason.clone(),
                        citation: citation.clone(),
                    })
                })
                .collect(),
            exact_interface_output_call: method.exact_interface_output_call.as_ref().and_then(
                |contract| {
                    let crate::com_metadata::RawEvidence::ExactRegistry {
                        entry_id,
                        family_id,
                        contract_kind,
                        reason,
                        citation,
                    } = &contract.evidence
                    else {
                        return None;
                    };
                    Some(UnsafeExactInterfaceOutputCall {
                        entry_id: entry_id.clone(),
                        family_id: family_id.id().into(),
                        contract_kind: contract_kind.key().into(),
                        source_fingerprint: contract.source_fingerprint.clone(),
                        flags_param_index: contract.flags_param_index,
                        context_param_index: contract.context_param_index,
                        synchronous_output_param_index: contract.synchronous_output_param_index,
                        semisynchronous_output_param_index: contract
                            .semisynchronous_output_param_index,
                        synchronous_flags: contract.synchronous_flags,
                        semisynchronous_flag_value: contract.semisynchronous_flag_value,
                        flags_option_name: contract.flags_option_name.clone(),
                        synchronous_output_option_name: contract
                            .synchronous_output_option_name
                            .clone(),
                        semisynchronous_output_option_name: contract
                            .semisynchronous_output_option_name
                            .clone(),
                        reason: reason.clone(),
                        citation: citation.clone(),
                    })
                },
            ),
            reasons,
            targets: capability.targets,
        });
    }
    let support = UnsafeInterfaceSupport {
        schema_version: UNSAFE_SUPPORT_SCHEMA_VERSION,
        metadata: UnsafeMetadataSupport {
            set_sha256: metadata.set_sha256.clone(),
            files: metadata.files.clone(),
            defining_file: metadata.defining_file(&meta.interface.namespace, &meta.interface.name),
        },
        interface_name: format!("{}.{}", meta.interface.namespace, meta.interface.name),
        interface_iid: meta.interface.iid.clone(),
        root: interface_root(meta).into(),
        base_iids: meta.base_iids.clone(),
        unsafe_class: class_name.clone(),
        module_path: module_path.clone(),
        methods: support_methods,
    };
    if executable.is_empty() {
        return Ok(UnsafeGeneratedOutput {
            class_name,
            module_path,
            js: None,
            dts: None,
            support,
            metadata_complete_methods,
            manual_methods,
            blocked_methods,
        });
    }
    let (js, dts) = render_unsafe_class(meta, &class_name, &support, &executable)?;
    Ok(UnsafeGeneratedOutput {
        class_name,
        module_path,
        js: Some(js),
        dts: Some(dts),
        support,
        metadata_complete_methods,
        manual_methods,
        blocked_methods,
    })
}

fn interface_root(meta: &ComInterfaceMeta) -> &'static str {
    if meta.is_iunknown_rooted && meta.base_offset == 3 {
        "IUnknown"
    } else if !meta.is_iunknown_rooted
        && meta.interface.name.ends_with("Interop")
        && meta.base_offset == 6
    {
        "IInspectable"
    } else {
        "Unknown"
    }
}

#[cfg(test)]
fn generate_unsafe_interface_files_with_identity(
    meta: &ComInterfaceMeta,
    metadata: MetadataSetIdentity,
) -> Result<UnsafeGeneratedOutput, String> {
    generate_unsafe_interface_files_with_metadata(meta, &metadata)
}

fn overall_status(method: &MethodCapability) -> RawClassification {
    if method
        .targets
        .values()
        .any(|target| target.classification == RawClassification::RawRuntimeBlocked)
    {
        RawClassification::RawRuntimeBlocked
    } else if method
        .targets
        .values()
        .any(|target| target.classification == RawClassification::RawManualContract)
    {
        RawClassification::RawManualContract
    } else {
        RawClassification::RawMetadataComplete
    }
}

fn manual_strategy_requirements(
    method: &RawComMethod,
    reasons: &[String],
) -> Result<Vec<UnsafeStrategyRequirement>, String> {
    let mut requirements = Vec::new();
    let mut assigned = BTreeSet::new();
    for (index, param) in method.params.iter().enumerate() {
        let exact_reasons = parameter_manual_reasons(method, index)?;
        let has = |reason: &str| exact_reasons.iter().any(|value| value == reason);
        let pointee_reasons = exact_reasons
            .iter()
            .filter(|reason| {
                reason.as_str() == "external_pointee_storage"
                    || reason.as_str() == "opaque_pointee_contract"
                    || reason.as_str() == "nested_pointer_lifetime"
                    || reason.starts_with("pointee_")
            })
            .cloned()
            .collect::<Vec<_>>();
        let category = super::model::metadata::census_raw_base_category(&param.typ);
        let output = matches!(
            param.direction,
            RawParamDirection::Out | RawParamDirection::InOut
        );
        let abi_pointer_depth = if category == "ComInterface" {
            param.typ.pointer_depth.saturating_add(1)
        } else {
            param.typ.pointer_depth
        };
        let mut parameter_reasons = Vec::new();
        let strategy = if output && has("missing_handle_ownership") && category == "Handle" {
            parameter_reasons.push("missing_handle_ownership".into());
            "UnsafeHandleOutput"
        } else if output
            && param.direction == RawParamDirection::InOut
            && category == "ComInterface"
            && abi_pointer_depth == 2
            && has("missing_interface_replacement_contract")
        {
            parameter_reasons.push("missing_interface_replacement_contract".into());
            "UnsafeInterfaceReplacement"
        } else if output
            && abi_pointer_depth == 2
            && (has("missing_output_ownership") || has("missing_allocator"))
        {
            for reason in ["missing_output_ownership", "missing_allocator"] {
                if has(reason) {
                    parameter_reasons.push(reason.into());
                }
            }
            parameter_reasons.extend(pointee_reasons.iter().cloned());
            "UnsafePointerOutput"
        } else if has("missing_count_relation")
            && (param.typ.pointer_depth > 0
                || matches!(
                    &param.typ.native_type,
                    RawNativeType::Array(_) | RawNativeType::FixedArray { .. }
                )
                || matches!(
                    category,
                    "Pointer" | "DataPointer" | "StringPointer" | "CountedBuffer"
                ))
        {
            parameter_reasons.push("missing_count_relation".into());
            "UnsafeCountedBuffer"
        } else if !pointee_reasons.is_empty()
            && (param.typ.pointer_depth > 0
                || matches!(category, "NativeStruct" | "NativeUnion" | "DataPointer"))
        {
            parameter_reasons.extend(pointee_reasons.iter().cloned());
            "UnsafePointee"
        } else if !exact_reasons.is_empty() && output {
            parameter_reasons.extend(exact_reasons.iter().cloned());
            "UnsafeRawCall"
        } else if has("variant_safearray_element_contract")
            && matches!(category, "Variant" | "SafeArray")
        {
            parameter_reasons.push("variant_safearray_element_contract".into());
            "UnsafeRawCall"
        } else {
            continue;
        };
        for reason in &parameter_reasons {
            assigned.insert(reason.clone());
        }
        parameter_reasons.sort();
        parameter_reasons.dedup();
        requirements.push(UnsafeStrategyRequirement {
            parameter_index: Some(index),
            parameter_name: Some(param.name.clone()),
            strategy: strategy.into(),
            reasons: parameter_reasons,
            direction: Some(
                match param.direction {
                    RawParamDirection::In => "in",
                    RawParamDirection::Out => "out",
                    RawParamDirection::InOut => "inout",
                }
                .into(),
            ),
            nullable: Some(param.optional),
            pointee_layouts: (strategy == "UnsafePointee").then(|| {
                parameter_pointee_layouts(param)
                    .into_iter()
                    .map(|(target, layout)| {
                        (
                            target,
                            layout.map(|layout| UnsafeNativeLayout {
                                size: layout.size,
                                alignment: layout.alignment,
                            }),
                        )
                    })
                    .collect()
            }),
        });
    }
    let remaining = reasons
        .iter()
        .filter(|reason| !assigned.contains(*reason))
        .cloned()
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        requirements.push(UnsafeStrategyRequirement {
            parameter_index: None,
            parameter_name: None,
            strategy: "UnsafeRawCall".into(),
            reasons: remaining,
            direction: None,
            nullable: None,
            pointee_layouts: None,
        });
    }
    Ok(requirements)
}

struct LayoutRegistry {
    entries: BTreeMap<String, (String, bool, String)>,
}

impl LayoutRegistry {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    fn register(
        &mut self,
        namespace: &str,
        name: &str,
        layout: &RawNativeLayoutSet,
    ) -> Result<String, String> {
        let key = format!("{namespace}.{name}");
        if let Some((variable, _, _)) = self.entries.get(&key) {
            return Ok(variable.clone());
        }
        let (is_union, descriptor) = raw_aggregate_descriptor(namespace, name, layout)?;
        let variable = format!("_layout{}", self.entries.len());
        self.entries
            .insert(key, (variable.clone(), is_union, descriptor));
        Ok(variable)
    }
}

struct RenderedMethod {
    registration: String,
    js: String,
    dts: String,
}

fn render_unsafe_class(
    meta: &ComInterfaceMeta,
    class_name: &str,
    support: &UnsafeInterfaceSupport,
    methods: &[(&RawComMethod, MethodCapability)],
) -> Result<(String, String), String> {
    let raw_runtime_import = serde_json::to_string(
        &super::javascript::render::com_raw_runtime_import_name_for_depth(
            super::canonical_namespace_depth(&meta.interface.namespace) + 1,
        ),
    )
    .map_err(|error| error.to_string())?;
    let mut layouts = LayoutRegistry::new();
    let has_manual = methods
        .iter()
        .any(|(_, capability)| overall_status(capability) == RawClassification::RawManualContract);
    let uses_strategy_runtime = has_manual
        || methods.iter().any(|(method, _)| {
            method.exact_interface_output_call.is_some()
                || method
                    .params
                    .iter()
                    .any(|param| param.exact_interface_output.is_some())
        });
    let runtime_path = format!(
        "{}runtime.js",
        "../".repeat(meta.interface.namespace.split('.').count())
    );
    let rendered = methods
        .iter()
        .map(|(method, capability)| render_method(method, capability, &mut layouts))
        .collect::<Result<Vec<_>, _>>()?;
    let mut js = String::new();
    js.push_str("// Generated by dynwinrt-codegen — do not edit\n'use strict';\n");
    js.push_str(&format!("const {{ DynCom, DynComMethodSig, DynComUnsafe, DynComRaw, DynComRawMemory, DynComRawOwnedComPointer, DynComRawPointer, DynComRawStructLayout, DynComRawUnionLayout, WinGuid }} = require({raw_runtime_import});\n\n"));
    if uses_strategy_runtime {
        js.push_str(&format!(
            "const {{ __prepareStrategy, __prepareWritableStorage, __prepareExactWritableSpan, __strategyArgument, __assertRawContract, __validateStrategySpans, __activateStrategies, __markDispatchEntered, __finishStrategy, __failStrategy, __releaseExtracted, __attachUnsafeOutputs }} = require('{}');\n\n",
            runtime_path
        ));
    }
    js.push_str(&format!(
        "const IID = WinGuid.parse('{}');\n",
        meta.interface.iid
    ));
    for (variable, is_union, descriptor) in layouts.entries.values() {
        js.push_str(&format!(
            "const {variable} = {}.fromDescriptor({});\n",
            if *is_union {
                "DynComRawUnionLayout"
            } else {
                "DynComRawStructLayout"
            },
            serde_json::to_string(descriptor).map_err(|error| error.to_string())?
        ));
    }
    js.push_str(&format!(
        "const _support = Object.freeze({});\n",
        serde_json::to_string(support).map_err(|error| error.to_string())?
    ));
    js.push_str(&format!(
        "const _interface = DynComUnsafe.{}('{}.{}', IID)\n",
        if meta.is_iunknown_rooted {
            "registerIUnknownInterface"
        } else {
            "registerIInspectableInterface"
        },
        meta.interface.namespace,
        meta.interface.name
    ));
    for method in &rendered {
        js.push_str(&method.registration);
    }
    js.push_str(";\nconst _token = Symbol('generated unsafe COM wrapper');\n");
    js.push_str("function _rawPointer(value, name) {\n    if (value instanceof DynComRawMemory) return value.pointer().toValue();\n    if (value instanceof DynComRawPointer) return value.toValue();\n    throw new TypeError(`${name} must be DynComRawMemory or DynComRawPointer`);\n}\n\n");
    js.push_str("function _prepareOwnedInterfaceOutput(value, name, optional) {\n    if (value === null && optional) return DynComRawPointer.null().toValue();\n    if (!(value instanceof DynComRawMemory)) throw new TypeError(`${name} must be empty DynComRawMemory${optional ? ' or null' : ''}`);\n    if (value.released) throw new Error(`${name} output slot has been released`);\n    const width = BigInt(DynComRaw.pointerSize());\n    if (value.size < width) throw new RangeError(`${name} output slot is smaller than pointer width`);\n    const pointer = value.pointer();\n    if (value.alignment < width || pointer.address % width !== 0n) throw new RangeError(`${name} output slot is not pointer-aligned`);\n    if (!value.readPointer(0).isNull) throw new Error(`${name} output slot must start null`);\n    return pointer.toValue();\n}\n\n");
    js.push_str("function _takeOwnedInterfaceOutput(value, name, iid, nullable) {\n    if (value === null) return null;\n    const pointer = value.readPointer(0);\n    if (pointer.isNull) {\n        if (nullable) return null;\n        throw new Error(`${name} required interface output was null`);\n    }\n    value.writePointer(0, DynComRawPointer.null());\n    return DynComRawOwnedComPointer.assumeTransferred(pointer, iid);\n}\n\n");
    js.push_str("function _cleanupOwnedInterfaceOutput(value) {\n    if (value === null) return;\n    const pointer = value.readPointer(0);\n    if (pointer.isNull) return;\n    value.writePointer(0, DynComRawPointer.null());\n    DynComRawOwnedComPointer.assumeTransferred(pointer).release();\n}\n\n");
    js.push_str(&format!("class {class_name} {{\n"));
    js.push_str("    constructor(token, obj) {\n        if (token !== _token) throw new TypeError('Use the static from() factory');\n        this._obj = obj;\n    }\n");
    js.push_str("    static from(value) {\n        const native = value && value.nativeValue ? value.nativeValue : value;\n        const cast = native.cast(IID);\n        DynCom.bindComObject(cast);\n        return new this(_token, cast);\n    }\n");
    js.push_str("    static iid = IID;\n    static support = _support;\n");
    js.push_str("    get nativeValue() { return this._obj; }\n");
    js.push_str("    release() { this._obj.release(); }\n");
    for method in &rendered {
        js.push_str(&method.js);
    }
    js.push_str("}\n");
    js.push_str(&format!("exports.{class_name} = {class_name};\n"));

    let mut dts = String::new();
    dts.push_str("// Generated by dynwinrt-codegen — do not edit\n");
    dts.push_str(&format!("import type {{ DynComRawMemory, DynComRawOwnedComPointer, DynComRawPointer, DynWinRtValue, WinGuid }} from {raw_runtime_import};\n\n"));
    if has_manual {
        dts.push_str(&format!(
            "import type {{ UnsafePointee, UnsafePointerOutput, UnsafeHandleOutput, UnsafeInterfaceReplacement, UnsafeInterfaceReplacementResult, UnsafeCountedBuffer, UnsafeRawCall }} from '{}';\n\n",
            runtime_path
        ));
    }
    dts.push_str("export type UnsafeMethodStatus = 'raw_metadata_complete' | 'raw_manual_contract' | 'raw_runtime_blocked';\n");
    dts.push_str("export type UnsafeCleanupAvailability = 'none_required' | 'standard_supported' | 'known_external' | 'unknown';\n");
    dts.push_str("export interface UnsafeMethodLifecycle { readonly cleanup: UnsafeCleanupAvailability; readonly requires_external_pointer_or_callback: boolean; readonly requires_external_acquisition: boolean; readonly requires_current_apartment: boolean; }\n");
    dts.push_str("export interface UnsafeMethodTargetSupport { readonly classification: UnsafeMethodStatus; readonly first_blocker_reason: string | null; readonly blocker_reasons: readonly string[]; readonly manual_contract_reasons: readonly string[]; readonly lifecycle: UnsafeMethodLifecycle; }\n");
    dts.push_str("export interface UnsafeNativeLayoutRequirement { readonly size: number; readonly alignment: number; }\n");
    dts.push_str("export interface UnsafeStrategyRequirement { readonly parameterIndex: number | null; readonly parameterName: string | null; readonly strategy: 'UnsafePointee' | 'UnsafePointerOutput' | 'UnsafeHandleOutput' | 'UnsafeInterfaceReplacement' | 'UnsafeCountedBuffer' | 'UnsafeRawCall'; readonly reasons: readonly string[]; readonly direction: 'in' | 'out' | 'inout' | null; readonly nullable: boolean | null; readonly pointeeLayouts: Readonly<Record<'x64' | 'i686' | 'arm64', UnsafeNativeLayoutRequirement | null>> | null; }\n");
    dts.push_str("export interface UnsafeExactInterfaceOutput { readonly entryId: string; readonly familyId: string; readonly contractKind: string; readonly parameterIndex: number; readonly parameterName: string; readonly interfaceIid: string; readonly argumentOptional: boolean; readonly nullableOnSuccess: boolean; readonly reason: string; readonly citation: string; }\n");
    dts.push_str("export interface UnsafeExactInterfaceOutputCall { readonly entryId: string; readonly familyId: string; readonly contractKind: string; readonly sourceFingerprint: string; readonly flagsParamIndex: number; readonly contextParamIndex: number; readonly synchronousOutputParamIndex: number; readonly semisynchronousOutputParamIndex: number; readonly synchronousFlags: number; readonly semisynchronousFlagValue: number; readonly flagsOptionName: string; readonly synchronousOutputOptionName: string; readonly semisynchronousOutputOptionName: string; readonly reason: string; readonly citation: string; }\n");
    dts.push_str("export interface UnsafeMethodSupport { readonly name: string; readonly projectedName: string; readonly declaringIid: string; readonly absoluteSlot: number; readonly signatureFingerprint: string; readonly status: UnsafeMethodStatus; readonly reasons: readonly string[]; readonly strategyRequirements: readonly UnsafeStrategyRequirement[]; readonly exactInterfaceOutputs: readonly UnsafeExactInterfaceOutput[]; readonly exactInterfaceOutputCall: UnsafeExactInterfaceOutputCall | null; readonly targets: Readonly<Record<'x64' | 'i686' | 'arm64', UnsafeMethodTargetSupport>>; }\n");
    dts.push_str("export interface UnsafeMetadataFile { readonly file: string; readonly package: string; readonly version: string; readonly sha256: string; }\n");
    dts.push_str("export interface UnsafeMetadataSupport { readonly setSha256: string; readonly files: readonly UnsafeMetadataFile[]; readonly definingFile: UnsafeMetadataFile | null; }\n");
    dts.push_str(&format!("export interface UnsafeInterfaceSupport {{ readonly schemaVersion: {UNSAFE_SUPPORT_SCHEMA_VERSION}; readonly metadata: UnsafeMetadataSupport; readonly interfaceName: string; readonly interfaceIid: string; readonly root: 'IUnknown' | 'IInspectable' | 'Unknown'; readonly baseIids: readonly string[]; readonly unsafeClass: string; readonly modulePath: string; readonly methods: readonly UnsafeMethodSupport[]; }}\n\n"));
    dts.push_str(&format!("export declare class {class_name} {{\n"));
    dts.push_str("    private constructor();\n");
    dts.push_str(&format!(
        "    static from(value: DynWinRtValue | {{ readonly nativeValue: DynWinRtValue }}): {class_name};\n"
    ));
    dts.push_str(
        "    static readonly iid: WinGuid;\n    static readonly support: UnsafeInterfaceSupport;\n",
    );
    dts.push_str("    readonly nativeValue: DynWinRtValue;\n    release(): void;\n");
    for method in &rendered {
        dts.push_str(&method.dts);
    }
    dts.push_str("}\n");
    Ok((js, dts))
}

fn render_method(
    method: &RawComMethod,
    capability: &MethodCapability,
    layouts: &mut LayoutRegistry,
) -> Result<RenderedMethod, String> {
    let status = overall_status(capability);
    let mut signature = "new DynComMethodSig()".to_string();
    let mut js_args = Vec::new();
    let mut dts_args = Vec::new();
    let mut exact_interface_outputs = Vec::new();
    for (index, param) in method.params.iter().enumerate() {
        let typ = render_type(&param.typ, layouts)?;
        let exact_null_context = method
            .exact_interface_output_call
            .as_ref()
            .is_some_and(|contract| contract.context_param_index == index);
        signature.push_str(&format!(
            ".{}({})",
            if param.optional || exact_null_context {
                "addNullableIn"
            } else {
                "addIn"
            },
            if exact_null_context {
                "DynCom.pointerType()"
            } else {
                typ.runtime_type.as_str()
            }
        ));
        if let Some(contract) = &param.exact_interface_output {
            exact_interface_outputs.push((index, param, contract));
            js_args.push(format!(
                "_prepareOwnedInterfaceOutput({}, '{}', {})",
                safe_identifier(&param.name),
                param.name,
                contract.argument_optional
            ));
            dts_args.push(format!(
                "{}: DynComRawMemory{}",
                safe_identifier(&param.name),
                if contract.argument_optional {
                    " | null"
                } else {
                    ""
                }
            ));
        } else {
            js_args.push(render_argument(&param.name, &param.typ, &typ));
            dts_args.push(format!(
                "{}: {}",
                safe_identifier(&param.name),
                typ.dts_input
            ));
        }
    }
    let return_kind = return_kind(&method.return_type);
    match return_kind {
        ReturnKind::HResult => {
            if method.semantic_hresult.is_some() {
                signature.push_str(".preserveHresult()");
            }
        }
        ReturnKind::Void => signature.push_str(".returnsVoid()"),
        ReturnKind::Value => {
            let typ = render_type(&method.return_type, layouts)?;
            signature.push_str(&format!(".returns({})", typ.runtime_type));
        }
    }
    let registration = format!(
        "    .addMethodAt({}, '{}', {})\n",
        capability.absolute_slot,
        camel_case(&capability.projected_name),
        signature
    );
    if status == RawClassification::RawManualContract {
        return render_manual_method(method, capability, layouts, registration);
    }
    let name = safe_identifier(&camel_case(&capability.projected_name));
    let args = method
        .params
        .iter()
        .map(|param| safe_identifier(&param.name))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(contract) = &method.exact_interface_output_call {
        return render_exact_interface_output_call_method(
            method,
            capability,
            contract,
            layouts,
            registration,
        );
    }
    if !exact_interface_outputs.is_empty() {
        let mut js = format!(
            "    /** @unsafe Exact owned COM output slots; each requested slot must start null. */\n    {name}({args}) {{\n        const _nativeArgs = [{}];\n        const _owners = [];\n        try {{\n            _interface.method({}).invokeAll(this._obj, _nativeArgs);\n",
            js_args.join(", "),
            capability.absolute_slot,
        );
        let mut result_names = Vec::new();
        let mut result_types = Vec::new();
        for (index, param, contract) in &exact_interface_outputs {
            let result = format!("_ownedOutput{index}");
            js.push_str(&format!(
                "            const {result} = _takeOwnedInterfaceOutput({}, '{}', WinGuid.parse({}), {});\n            if ({result}) _owners.push({result});\n",
                safe_identifier(&param.name),
                param.name,
                serde_json::to_string(&contract.interface_iid)
                    .map_err(|error| error.to_string())?,
                contract.nullable_on_success,
            ));
            result_names.push(result);
            result_types.push(format!(
                "DynComRawOwnedComPointer{}",
                if contract.nullable_on_success || contract.argument_optional {
                    " | null"
                } else {
                    ""
                }
            ));
        }
        match result_names.as_slice() {
            [result] => js.push_str(&format!("            return {result};\n")),
            results => js.push_str(&format!(
                "            return Object.freeze([{}]);\n",
                results.join(", ")
            )),
        }
        js.push_str("        } catch (_error) {\n            const _cleanupErrors = [];\n");
        for (_, param, _) in exact_interface_outputs.iter().rev() {
            js.push_str(&format!(
                "            try {{ _cleanupOwnedInterfaceOutput({}); }} catch (_cleanup) {{ _cleanupErrors.push(_cleanup); }}\n",
                safe_identifier(&param.name)
            ));
        }
        js.push_str("            for (let _index = _owners.length - 1; _index >= 0; _index--) {\n                try { _owners[_index].release(); } catch (_cleanup) { _cleanupErrors.push(_cleanup); }\n            }\n            if (_cleanupErrors.length) throw new AggregateError([_error, ..._cleanupErrors], 'Exact interface output cleanup failed', { cause: _error });\n            throw _error;\n        }\n    }\n");
        let dts_return = match result_types.as_slice() {
            [result] => result.clone(),
            results => format!("readonly [{}]", results.join(", ")),
        };
        let dts = format!(
            "    /** @unsafe Exact owned COM output slots; each requested slot must start null. */\n    {name}({}): {dts_return};\n",
            dts_args.join(", ")
        );
        return Ok(RenderedMethod {
            registration,
            js,
            dts,
        });
    }
    let mut js =
        format!("    /** @unsafe Metadata-complete outbound ABI. */\n    {name}({args}) {{\n");
    js.push_str(&format!(
        "        const _out = _interface.method({}).invokeAll(this._obj, [{}]);\n",
        capability.absolute_slot,
        js_args.join(", ")
    ));
    let returns_value = return_kind == ReturnKind::Value || method.semantic_hresult.is_some();
    if returns_value {
        let conversion = render_return_conversion(&method.return_type, "_out[0]", layouts)?;
        js.push_str(&format!("        return {conversion};\n"));
    }
    js.push_str("    }\n");
    let dts_return = if returns_value {
        render_type(&method.return_type, layouts)?.dts_output
    } else {
        "void".into()
    };
    let dts = format!(
        "    /** @unsafe Metadata-complete outbound ABI. */\n    {name}({}): {dts_return};\n",
        dts_args.join(", ")
    );
    Ok(RenderedMethod {
        registration,
        js,
        dts,
    })
}

fn render_exact_interface_output_call_method(
    method: &RawComMethod,
    capability: &MethodCapability,
    contract: &RawExactInterfaceOutputCallContract,
    layouts: &mut LayoutRegistry,
    registration: String,
) -> Result<RenderedMethod, String> {
    let flags = method
        .params
        .get(contract.flags_param_index)
        .ok_or_else(|| "Exact interface output flags parameter is out of range".to_string())?;
    let context = method
        .params
        .get(contract.context_param_index)
        .ok_or_else(|| "Exact interface output context parameter is out of range".to_string())?;
    let synchronous = method
        .params
        .get(contract.synchronous_output_param_index)
        .ok_or_else(|| "Exact synchronous output parameter is out of range".to_string())?;
    let semisynchronous = method
        .params
        .get(contract.semisynchronous_output_param_index)
        .ok_or_else(|| "Exact semisynchronous output parameter is out of range".to_string())?;
    let synchronous_contract = synchronous
        .exact_interface_output
        .as_ref()
        .ok_or_else(|| "Exact synchronous output has no ownership contract".to_string())?;
    let semisynchronous_contract = semisynchronous
        .exact_interface_output
        .as_ref()
        .ok_or_else(|| "Exact semisynchronous output has no ownership contract".to_string())?;
    if !synchronous_contract.argument_optional
        || !semisynchronous_contract.argument_optional
        || synchronous_contract.nullable_on_success
        || semisynchronous_contract.nullable_on_success
    {
        return Err("Exact mode-selected interface outputs must be optional arguments with non-null requested results".into());
    }

    let mut public_args = Vec::new();
    let mut dts_args = Vec::new();
    let mut native_args = Vec::new();
    for (index, param) in method.params.iter().enumerate() {
        if contract.public_input_param_indices.contains(&index) {
            let rendered = render_type(&param.typ, layouts)?;
            let name = safe_identifier(&param.name);
            public_args.push(name.clone());
            dts_args.push(format!("{name}: {}", rendered.dts_input));
            native_args.push(render_argument(&param.name, &param.typ, &rendered));
        } else if index == contract.flags_param_index {
            let rendered = render_type(&param.typ, layouts)?;
            native_args.push(render_argument(
                &contract.flags_option_name,
                &param.typ,
                &rendered,
            ));
        } else if index == contract.context_param_index {
            native_args.push("DynComRawPointer.null().toValue()".into());
        } else if index == contract.synchronous_output_param_index {
            native_args.push("__strategyArgument(_synchronousOutputRecord)".into());
        } else if index == contract.semisynchronous_output_param_index {
            native_args.push("__strategyArgument(_semisynchronousOutputRecord)".into());
        } else {
            return Err(format!(
                "Exact interface output call does not describe parameter {index} ({})",
                param.name
            ));
        }
    }
    if flags.direction != RawParamDirection::In || context.direction != RawParamDirection::In {
        return Err("Exact interface output flags and context must be input parameters".into());
    }

    let name = safe_identifier(&camel_case(&capability.projected_name));
    let flags_name = safe_identifier(&contract.flags_option_name);
    let synchronous_name = safe_identifier(&contract.synchronous_output_option_name);
    let semisynchronous_name = safe_identifier(&contract.semisynchronous_output_option_name);
    let method_args = if public_args.is_empty() {
        "options".into()
    } else {
        format!("{}, options", public_args.join(", "))
    };
    let js = format!(
        "    /** @unsafe Exact OpenNamespace mode contract; pCtx is always native null. */\n    {name}({method_args}) {{\n        if (!options || Object.getPrototypeOf(options) !== Object.prototype) throw new TypeError('options must be a plain object');\n        if (options.pCtx !== undefined && options.pCtx !== null) throw new TypeError('pCtx is reserved and must be native null');\n        const {flags_name} = options[{}];\n        if (!Number.isInteger({flags_name}) || {flags_name} < -2147483648 || {flags_name} > 2147483647) throw new RangeError('{} must be an exact signed 32-bit integer');\n        const {synchronous_name} = options[{}] ?? null;\n        const {semisynchronous_name} = options[{}] ?? null;\n        if (({synchronous_name} === null) === ({semisynchronous_name} === null)) throw new TypeError('Exactly one OpenNamespace output must be supplied');\n        const _isSynchronous = {flags_name} === {};\n        const _isSemisynchronous = {flags_name} === {};\n        if ({synchronous_name} !== null && !_isSynchronous) throw new TypeError('workingNamespace requires synchronous lFlags');\n        if ({semisynchronous_name} !== null && !_isSemisynchronous) throw new TypeError('result requires exact WBEM_FLAG_RETURN_IMMEDIATELY lFlags');\n        const _synchronousOutputRecord = __prepareExactWritableSpan({synchronous_name}, '{}', true);\n        const _semisynchronousOutputRecord = __prepareExactWritableSpan({semisynchronous_name}, '{}', true);\n        const _prepared = [_synchronousOutputRecord, _semisynchronousOutputRecord];\n        __validateStrategySpans(_prepared);\n        const _nativeArgs = [{}];\n        const _owners = [];\n        try {{\n            _interface.method({}).invokeAll(this._obj, _nativeArgs);\n            const _owner = {synchronous_name} !== null\n                ? _takeOwnedInterfaceOutput({synchronous_name}, '{}', WinGuid.parse({}), false)\n                : _takeOwnedInterfaceOutput({semisynchronous_name}, '{}', WinGuid.parse({}), false);\n            _owners.push(_owner);\n            return _owner;\n        }} catch (_error) {{\n            const _cleanupErrors = [];\n            try {{ _cleanupOwnedInterfaceOutput({semisynchronous_name}); }} catch (_cleanup) {{ _cleanupErrors.push(_cleanup); }}\n            try {{ _cleanupOwnedInterfaceOutput({synchronous_name}); }} catch (_cleanup) {{ _cleanupErrors.push(_cleanup); }}\n            for (let _index = _owners.length - 1; _index >= 0; _index--) {{\n                try {{ _owners[_index].release(); }} catch (_cleanup) {{ _cleanupErrors.push(_cleanup); }}\n            }}\n            if (_cleanupErrors.length) throw new AggregateError([_error, ..._cleanupErrors], 'Exact interface output cleanup failed', {{ cause: _error }});\n            throw _error;\n        }}\n    }}\n",
        serde_json::to_string(&contract.flags_option_name).map_err(|error| error.to_string())?,
        contract.flags_option_name,
        serde_json::to_string(&contract.synchronous_output_option_name)
            .map_err(|error| error.to_string())?,
        serde_json::to_string(&contract.semisynchronous_output_option_name)
            .map_err(|error| error.to_string())?,
        contract.synchronous_flags,
        contract.semisynchronous_flag_value,
        synchronous.name,
        semisynchronous.name,
        native_args.join(", "),
        capability.absolute_slot,
        synchronous.name,
        serde_json::to_string(&synchronous_contract.interface_iid)
            .map_err(|error| error.to_string())?,
        semisynchronous.name,
        serde_json::to_string(&semisynchronous_contract.interface_iid)
            .map_err(|error| error.to_string())?,
    );
    let public_dts = if dts_args.is_empty() {
        String::new()
    } else {
        format!("{}, ", dts_args.join(", "))
    };
    let dts = format!(
        "    /** @unsafe Synchronous mode; lFlags must be exactly {} and pCtx is always native null. */\n    {name}({public_dts}options: {{ readonly {}: {}; readonly {}: DynComRawMemory; readonly {}?: null }}): DynComRawOwnedComPointer;\n    /** @unsafe Semisynchronous mode; lFlags must equal WBEM_FLAG_RETURN_IMMEDIATELY ({}), and pCtx is always native null. */\n    {name}({public_dts}options: {{ readonly {}: {}; readonly {}?: null; readonly {}: DynComRawMemory }}): DynComRawOwnedComPointer;\n",
        contract.synchronous_flags,
        contract.flags_option_name,
        contract.synchronous_flags,
        contract.synchronous_output_option_name,
        contract.semisynchronous_output_option_name,
        contract.semisynchronous_flag_value,
        contract.flags_option_name,
        contract.semisynchronous_flag_value,
        contract.synchronous_output_option_name,
        contract.semisynchronous_output_option_name,
    );
    Ok(RenderedMethod {
        registration,
        js,
        dts,
    })
}

fn render_manual_method(
    method: &RawComMethod,
    capability: &MethodCapability,
    layouts: &mut LayoutRegistry,
    registration: String,
) -> Result<RenderedMethod, String> {
    let reasons = capability
        .targets
        .values()
        .flat_map(|target| target.manual_contract_reasons.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let requirements = manual_strategy_requirements(method, &reasons)?;
    let by_parameter = requirements
        .iter()
        .filter_map(|requirement| {
            requirement
                .parameter_index
                .map(|index| (index, requirement))
        })
        .collect::<BTreeMap<_, _>>();
    let method_contract = requirements
        .iter()
        .find(|requirement| requirement.parameter_index.is_none());
    let mut public_args = Vec::new();
    let mut dts_args = Vec::new();
    let mut native_args = Vec::new();
    let mut preparation = String::new();
    let mut result_strategies = Vec::<(String, String)>::new();
    let mut generic_types = Vec::new();
    let mut prepared_indexes = BTreeSet::new();
    for (index, param) in method.params.iter().enumerate() {
        let name = safe_identifier(&param.name);
        public_args.push(name.clone());
        if let Some(requirement) = by_parameter.get(&index) {
            let strategy = requirement.strategy.as_str();
            let generic_result = matches!(strategy, "UnsafePointerOutput" | "UnsafeHandleOutput")
                .then(|| {
                    let generic = format!("T{}", generic_types.len());
                    generic_types.push(generic.clone());
                    generic
                });
            dts_args.push(if let Some(generic) = &generic_result {
                format!("{name}: {strategy}<{generic}>")
            } else {
                format!("{name}: {strategy}")
            });
            let contract = serde_json::to_string(&serde_json::json!({
                "direction": requirement.direction,
                "nullable": requirement.nullable,
                "pointeeLayouts": requirement.pointee_layouts,
            }))
            .map_err(|error| error.to_string())?;
            preparation.push_str(&format!(
                "            _manualRecord{index} = __prepareStrategy({name}, '{strategy}', '{}', {contract});\n            _prepared.push(_manualRecord{index});\n",
                param.name
            ));
            prepared_indexes.insert(index);
            let expression = match strategy {
                "UnsafePointerOutput" | "UnsafeHandleOutput" => {
                    result_strategies.push((
                        format!("_manualRecord{index}"),
                        generic_result.expect("output strategy generic was created"),
                    ));
                    format!("__strategyArgument(_manualRecord{index})")
                }
                "UnsafeInterfaceReplacement" => {
                    result_strategies.push((
                        format!("_manualRecord{index}"),
                        "UnsafeInterfaceReplacementResult".into(),
                    ));
                    format!("__strategyArgument(_manualRecord{index})")
                }
                "UnsafePointee" | "UnsafeCountedBuffer" | "UnsafeRawCall" => {
                    format!("__strategyArgument(_manualRecord{index})")
                }
                _ => {
                    return Err(format!(
                        "Unsupported generated unsafe strategy `{strategy}`"
                    ));
                }
            };
            native_args.push(expression);
        } else {
            let typ = render_type(&param.typ, layouts)?;
            let writable_pointer = matches!(
                param.direction,
                RawParamDirection::Out | RawParamDirection::InOut
            ) && matches!(typ.conversion, ValueConversion::Pointer);
            dts_args.push(format!(
                "{name}: {}",
                if writable_pointer {
                    "DynComRawMemory"
                } else {
                    typ.dts_input.as_str()
                }
            ));
            if writable_pointer {
                let contract = rendered_pointee_contract(param)?;
                preparation.push_str(&format!(
                    "            _manualRecord{index} = __prepareWritableStorage({name}, '{}', {contract});\n            _prepared.push(_manualRecord{index});\n",
                    param.name
                ));
                prepared_indexes.insert(index);
                native_args.push(format!("__strategyArgument(_manualRecord{index})"));
            } else {
                native_args.push(render_argument(&param.name, &param.typ, &typ));
            }
        }
    }
    if method_contract.is_some() {
        public_args.push("unsafeContract".into());
        dts_args.push("unsafeContract: UnsafeRawCall".into());
        preparation.push_str("            __assertRawContract(unsafeContract);\n");
    }
    let reason_text = reasons.join(", ");
    let requirement_text = requirements
        .iter()
        .map(|requirement| {
            format!(
                "{}:{}",
                requirement.parameter_name.as_deref().unwrap_or("method"),
                requirement.strategy
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let record_declarations = prepared_indexes
        .iter()
        .map(|index| format!("_manualRecord{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let record_declarations = if record_declarations.is_empty() {
        String::new()
    } else {
        format!("        let {record_declarations};\n")
    };
    let name = safe_identifier(&camel_case(&capability.projected_name));
    let mut js = format!(
        "    /** @unsafe Manual outbound ABI. Missing facts: {reason_text}. Required strategies: {requirement_text}. */\n    {name}({}) {{\n        const _prepared = [];\n        const _dispatch = DynComRaw.__createDispatchState();\n{record_declarations}        let _out;\n        try {{\n",
        public_args.join(", ")
    );
    js.push_str(&preparation);
    js.push_str(&format!(
        "            const _method = _interface.method({});\n            const _nativeArgs = [{}];\n            __validateStrategySpans(_prepared);\n            __activateStrategies(_prepared);\n                            _out = DynComRaw.__invokeAllTracked(_method, this._obj, _nativeArgs, _dispatch);\n            __markDispatchEntered(_prepared, _dispatch);\n        }} catch (_error) {{\n            __markDispatchEntered(_prepared, _dispatch);\n            const _cleanupErrors = [];\n            const _unsafeOutputs = [];\n            for (let _index = _prepared.length - 1; _index >= 0; _index--) {{\n                try {{ const _unsafeOutput = __failStrategy(_prepared[_index]); if (_unsafeOutput) _unsafeOutputs.push(_unsafeOutput); }} catch (_cleanup) {{ _cleanupErrors.push(_cleanup); }}\n            }}\n            const _primary = __attachUnsafeOutputs(_error, _unsafeOutputs);\n            if (_cleanupErrors.length) throw new AggregateError([_primary, ..._cleanupErrors], 'Native call and unsafe output cleanup both failed', {{ cause: _primary }});\n            throw _primary;\n        }}\n",
        capability.absolute_slot,
        native_args.join(", ")
    ));
    let return_kind = return_kind(&method.return_type);
    let returns_value = return_kind == ReturnKind::Value || method.semantic_hresult.is_some();
    let mut result_values = Vec::new();
    let mut result_types = Vec::new();
    js.push_str("        const _extracted = [];\n        try {\n");
    if returns_value {
        let conversion = render_return_conversion(&method.return_type, "_out[0]", layouts)?;
        js.push_str(&format!(
            "            const _directResult = {conversion};\n            _extracted.push(_directResult);\n"
        ));
        result_values.push("_directResult".into());
        result_types.push(render_type(&method.return_type, layouts)?.dts_output);
    }

    fn rendered_pointee_contract(
        param: &crate::com_metadata::RawComParam,
    ) -> Result<String, String> {
        let layouts = parameter_pointee_layouts(param)
            .into_iter()
            .map(|(target, layout)| {
                (
                    target,
                    layout.map(|layout| UnsafeNativeLayout {
                        size: layout.size,
                        alignment: layout.alignment,
                    }),
                )
            })
            .collect::<BTreeMap<_, _>>();
        serde_json::to_string(&serde_json::json!({
            "direction": match param.direction {
                RawParamDirection::In => "in",
                RawParamDirection::Out => "out",
                RawParamDirection::InOut => "inout",
            },
            "nullable": param.optional,
            "pointeeLayouts": layouts,
        }))
        .map_err(|error| error.to_string())
    }
    for (index, (record, result_type)) in result_strategies.iter().enumerate() {
        js.push_str(&format!(
            "            const _manualResult{index} = __finishStrategy({record});\n            _extracted.push(_manualResult{index});\n"
        ));
        result_values.push(format!("_manualResult{index}"));
        result_types.push(result_type.clone());
    }
    match result_values.as_slice() {
        [] => {}
        [value] => js.push_str(&format!("            return {value};\n")),
        values => js.push_str(&format!(
            "            return Object.freeze([{}]);\n",
            values.join(", ")
        )),
    }
    js.push_str(
        "        } catch (_finishError) {\n            const _cleanupErrors = [];\n            const _unsafeOutputs = [];\n            for (let _index = _prepared.length - 1; _index >= 0; _index--) {\n                try { const _unsafeOutput = __failStrategy(_prepared[_index]); if (_unsafeOutput) _unsafeOutputs.push(_unsafeOutput); } catch (_cleanup) { _cleanupErrors.push(_cleanup); }\n            }\n            for (let _index = _extracted.length - 1; _index >= 0; _index--) {\n                try { const _unsafeOutput = __releaseExtracted(_extracted[_index]); if (_unsafeOutput) _unsafeOutputs.push(_unsafeOutput); } catch (_cleanup) { _cleanupErrors.push(_cleanup); }\n            }\n            const _primary = __attachUnsafeOutputs(_finishError, _unsafeOutputs);\n            if (_cleanupErrors.length) throw new AggregateError([_primary, ..._cleanupErrors], 'Unsafe result extraction and cleanup both failed', { cause: _primary });\n            throw _primary;\n        }\n",
    );
    js.push_str("    }\n");
    let dts_return = match result_types.as_slice() {
        [] => "void".into(),
        [result] => result.clone(),
        results => format!("readonly [{}]", results.join(", ")),
    };
    let generics = if generic_types.is_empty() {
        String::new()
    } else {
        format!("<{}>", generic_types.join(", "))
    };
    let dts = format!(
        "    /** @unsafe Manual outbound ABI. Missing facts: {reason_text}. Required strategies: {requirement_text}. */\n    {name}{generics}({}): {dts_return};\n",
        dts_args.join(", ")
    );
    Ok(RenderedMethod {
        registration,
        js,
        dts,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReturnKind {
    HResult,
    Void,
    Value,
}

fn return_kind(typ: &RawComType) -> ReturnKind {
    if typ.pointer_depth == 0 && matches!(typ.native_type, RawNativeType::Void) {
        ReturnKind::Void
    } else if typ.pointer_depth == 0
        && matches!(
            &typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.Foundation" && name == "HRESULT"
        )
    {
        ReturnKind::HResult
    } else {
        ReturnKind::Value
    }
}

struct RenderedType {
    runtime_type: String,
    dts_input: String,
    dts_output: String,
    conversion: ValueConversion,
}

enum ValueConversion {
    Pointer,
    PointerBits,
    ComInterface,
    Bool,
    Number,
    NativeIsize,
    NativeUsize,
    SignedBigInt,
    UnsignedBigInt,
    Guid,
    Bstr,
    HString,
    RawValue,
}

fn render_type(typ: &RawComType, layouts: &mut LayoutRegistry) -> Result<RenderedType, String> {
    if typ.pointer_depth > 0 {
        return Ok(RenderedType {
            runtime_type: "DynCom.pointerType()".into(),
            dts_input: "DynComRawMemory | DynComRawPointer".into(),
            dts_output: "DynComRawPointer".into(),
            conversion: ValueConversion::Pointer,
        });
    }
    let scalar = |runtime: &str, dts: &str, conversion| RenderedType {
        runtime_type: format!("DynCom.{runtime}Type()"),
        dts_input: dts.into(),
        dts_output: dts.into(),
        conversion,
    };
    let category = super::model::metadata::census_raw_base_category(typ);
    Ok(match &typ.native_type {
        RawNativeType::Bool => scalar("bool", "boolean", ValueConversion::Bool),
        RawNativeType::I8 => scalar("i8", "number", ValueConversion::Number),
        RawNativeType::U8 => scalar("u8", "number", ValueConversion::Number),
        RawNativeType::I16 => scalar("i16", "number", ValueConversion::Number),
        RawNativeType::U16 | RawNativeType::Char16 => {
            scalar("u16", "number", ValueConversion::Number)
        }
        RawNativeType::I32 => scalar("i32", "number", ValueConversion::Number),
        RawNativeType::U32 => scalar("u32", "number", ValueConversion::Number),
        RawNativeType::I64 => scalar("i64", "bigint", ValueConversion::SignedBigInt),
        RawNativeType::U64 => scalar("u64", "bigint", ValueConversion::UnsignedBigInt),
        RawNativeType::F32 => scalar("f32", "number", ValueConversion::Number),
        RawNativeType::F64 => scalar("f64", "number", ValueConversion::Number),
        RawNativeType::ISize => scalar("isize", "bigint", ValueConversion::NativeIsize),
        RawNativeType::USize => scalar("usize", "bigint", ValueConversion::NativeUsize),
        RawNativeType::Named { .. } if category == "Guid" => RenderedType {
            runtime_type: "DynCom.guidType()".into(),
            dts_input: "WinGuid".into(),
            dts_output: "string".into(),
            conversion: ValueConversion::Guid,
        },
        RawNativeType::Named { .. } if category == "Bstr" => RenderedType {
            runtime_type: "DynCom.bstrType()".into(),
            dts_input: "string".into(),
            dts_output: "string".into(),
            conversion: ValueConversion::Bstr,
        },
        RawNativeType::Named { .. } | RawNativeType::String if category == "HString" => {
            RenderedType {
                runtime_type: "DynCom.hstringType()".into(),
                dts_input: "string".into(),
                dts_output: "string".into(),
                conversion: ValueConversion::HString,
            }
        }
        RawNativeType::Named { iid: Some(iid), .. } if category == "ComInterface" => RenderedType {
            runtime_type: format!(
                "DynCom.interfaceType(WinGuid.parse({}))",
                serde_json::to_string(iid).map_err(|error| error.to_string())?
            ),
            dts_input: "DynWinRtValue | { readonly nativeValue: DynWinRtValue }".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::ComInterface,
        },
        RawNativeType::Named { .. } if category == "Handle" => RenderedType {
            runtime_type: "DynCom.pointerType()".into(),
            dts_input: "bigint".into(),
            dts_output: "bigint".into(),
            conversion: ValueConversion::PointerBits,
        },
        RawNativeType::Named { .. }
            if matches!(
                category,
                "Pointer"
                    | "DataPointer"
                    | "StringPointer"
                    | "CountedBuffer"
                    | "SafeArray"
                    | "FunctionPointer"
            ) =>
        {
            RenderedType {
                runtime_type: "DynCom.pointerType()".into(),
                dts_input: "DynComRawMemory | DynComRawPointer".into(),
                dts_output: "DynComRawPointer".into(),
                conversion: ValueConversion::Pointer,
            }
        }
        RawNativeType::Named { .. } if category == "Scalar" || category == "Enum" => {
            let underlying = typ
                .underlying
                .as_deref()
                .ok_or_else(|| format!("Named {category} has no underlying ABI"))?;
            render_type(underlying, layouts)?
        }
        RawNativeType::Named {
            namespace,
            name,
            layout: Some(layout),
            ..
        } if matches!(category, "NativeStruct" | "NativeUnion") => {
            let variable = layouts.register(namespace, name, layout)?;
            RenderedType {
                runtime_type: format!("{variable}.byValueType()"),
                dts_input: "DynWinRtValue".into(),
                dts_output: "DynWinRtValue".into(),
                conversion: ValueConversion::RawValue,
            }
        }
        RawNativeType::Named { .. } if category == "Variant" => RenderedType {
            runtime_type: "DynCom.variantByValueType()".into(),
            dts_input: "DynWinRtValue".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::RawValue,
        },
        RawNativeType::Named { .. } if category == "PropVariant" => RenderedType {
            runtime_type: "DynCom.propVariantType()".into(),
            dts_input: "DynWinRtValue".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::RawValue,
        },
        RawNativeType::Named { .. } if category == "DispatchParams" => RenderedType {
            runtime_type: "DynCom.dispatchParamsType()".into(),
            dts_input: "DynWinRtValue".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::RawValue,
        },
        RawNativeType::Named { .. } if category == "ExcepInfo" => RenderedType {
            runtime_type: "DynCom.excepInfoType()".into(),
            dts_input: "DynWinRtValue".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::RawValue,
        },
        RawNativeType::Named { .. } if category == "StatStg" => RenderedType {
            runtime_type: "DynCom.statStgType()".into(),
            dts_input: "DynWinRtValue".into(),
            dts_output: "DynWinRtValue".into(),
            conversion: ValueConversion::RawValue,
        },
        RawNativeType::Object | RawNativeType::Array(_) | RawNativeType::FixedArray { .. } => {
            RenderedType {
                runtime_type: "DynCom.pointerType()".into(),
                dts_input: "DynComRawMemory | DynComRawPointer".into(),
                dts_output: "DynComRawPointer".into(),
                conversion: ValueConversion::Pointer,
            }
        }
        RawNativeType::Void
        | RawNativeType::String
        | RawNativeType::Unknown(_)
        | RawNativeType::Named { .. } => {
            return Err(format!(
                "Unsupported metadata-complete generated type ({category}): {:?}",
                typ.native_type,
            ));
        }
    })
}

fn render_argument(name: &str, typ: &RawComType, rendered: &RenderedType) -> String {
    let name = safe_identifier(name);
    match rendered.conversion {
        ValueConversion::Pointer => format!("_rawPointer({name}, '{name}')"),
        ValueConversion::PointerBits => format!("DynCom.pointer({name})"),
        ValueConversion::ComInterface => {
            format!("({name} && {name}.nativeValue ? {name}.nativeValue : {name})")
        }
        ValueConversion::Bool => format!("DynCom.boolValue({name})"),
        ValueConversion::Number => match underlying_native_type(typ) {
            RawNativeType::I8 => format!("DynCom.i8Value({name})"),
            RawNativeType::U8 | RawNativeType::Bool => format!("DynCom.u8Value({name})"),
            RawNativeType::I16 => format!("DynCom.i16({name})"),
            RawNativeType::U16 | RawNativeType::Char16 => format!("DynCom.u16({name})"),
            RawNativeType::U32 => format!("DynCom.u32({name})"),
            RawNativeType::F32 => format!("DynCom.f32({name})"),
            RawNativeType::F64 => format!("DynCom.f64({name})"),
            _ => format!("DynCom.i32({name})"),
        },
        ValueConversion::NativeIsize => format!("DynCom.isize({name})"),
        ValueConversion::NativeUsize => format!("DynCom.usize({name})"),
        ValueConversion::SignedBigInt => format!("DynCom.i64({name})"),
        ValueConversion::UnsignedBigInt => format!("DynCom.u64({name})"),
        ValueConversion::Guid => format!("DynCom.guid({name})"),
        ValueConversion::Bstr => format!("DynCom.bstr({name})"),
        ValueConversion::HString => format!("DynCom.hstring({name})"),
        ValueConversion::RawValue => name,
    }
}

fn underlying_native_type(typ: &RawComType) -> &RawNativeType {
    typ.underlying
        .as_deref()
        .map(underlying_native_type)
        .unwrap_or(&typ.native_type)
}

fn render_return_conversion(
    typ: &RawComType,
    expression: &str,
    layouts: &mut LayoutRegistry,
) -> Result<String, String> {
    let rendered = render_type(typ, layouts)?;
    Ok(match rendered.conversion {
        ValueConversion::Pointer => {
            format!("DynComRawPointer.fromAddress(DynCom.asPointerBigint({expression}))")
        }
        ValueConversion::PointerBits => format!("DynCom.asPointerBigint({expression})"),
        ValueConversion::ComInterface => expression.into(),
        ValueConversion::Bool => format!("DynCom.toBool({expression})"),
        ValueConversion::Number => format!("DynCom.toNumber({expression})"),
        ValueConversion::NativeIsize => format!("DynCom.toIsizeBigint({expression})"),
        ValueConversion::NativeUsize => format!("DynCom.toUsizeBigint({expression})"),
        ValueConversion::SignedBigInt => format!("DynCom.toI64Bigint({expression})"),
        ValueConversion::UnsignedBigInt => format!("DynCom.toU64Bigint({expression})"),
        ValueConversion::Guid => format!("DynCom.toGuidString({expression})"),
        ValueConversion::Bstr => format!("DynCom.takeBstr({expression})"),
        ValueConversion::HString => format!("{expression}.toString()"),
        ValueConversion::RawValue => expression.into(),
    })
}

fn safe_identifier(name: &str) -> String {
    let mut value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() || value.as_bytes()[0].is_ascii_digit() {
        value.insert(0, '_');
    }
    if matches!(
        value.as_str(),
        "class" | "function" | "return" | "default" | "new" | "delete" | "this"
    ) {
        value.push('_');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com_metadata::{
        ComInterfaceMeta, InterfaceMeta, MethodMeta, RawComParam, RawConstness, RawLayoutKind,
        RawNamedKind, RawNativeField, RawNativeLayout, RawPacking, RawParamDirection,
    };

    fn raw(native_type: RawNativeType, pointer_depth: usize) -> RawComType {
        RawComType {
            native_type,
            underlying: None,
            pointer_depth,
            constness: RawConstness::Unspecified,
        }
    }

    fn method(
        name: &str,
        slot: usize,
        params: Vec<RawComParam>,
        return_type: RawComType,
    ) -> RawComMethod {
        RawComMethod {
            declaring_namespace: "Tests".into(),
            declaring_interface: "ITest".into(),
            declaring_iid: "00000000-0000-0000-c000-000000000046".into(),
            metadata_name: name.into(),
            projected_name: name.into(),
            vtable_index: slot,
            params,
            return_type,
            semantic_hresult: None,
            enumerator_next: None,
            exact_contract: None,
            interface_replacement_contracts: Vec::new(),
            exact_interface_output_call: None,
            safe_array_contract_error: None,
        }
    }

    fn param(name: &str, typ: RawComType, direction: RawParamDirection) -> RawComParam {
        RawComParam {
            name: name.into(),
            typ,
            direction,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        }
    }

    fn interface(raw_methods: Vec<RawComMethod>) -> ComInterfaceMeta {
        let methods = raw_methods
            .iter()
            .map(|method| MethodMeta {
                name: method.metadata_name.clone(),
                vtable_index: method.vtable_index,
                ..MethodMeta::default()
            })
            .collect();
        ComInterfaceMeta {
            interface: InterfaceMeta {
                namespace: "Tests".into(),
                name: "ITest".into(),
                iid: "00000000-0000-0000-c000-000000000046".into(),
                methods,
                ..InterfaceMeta::default()
            },
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: Vec::new(),
            base_iids: Vec::new(),
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
            raw_referenced_enums: Some(Vec::new()),
            raw_methods: Some(raw_methods),
        }
    }

    fn identity() -> MetadataSetIdentity {
        MetadataSetIdentity {
            set_sha256: "00".into(),
            files: vec![MetadataFileIdentity {
                file: "test.winmd".into(),
                package: "test-metadata".into(),
                version: "1".into(),
                sha256: "00".into(),
            }],
            definition_files: BTreeMap::new(),
        }
    }

    #[test]
    fn partial_companion_emits_only_metadata_complete_methods() {
        let complete = method("GetURL", 3, Vec::new(), raw(RawNativeType::I32, 0));
        let manual = method(
            "Allocate",
            4,
            vec![param(
                "value",
                raw(RawNativeType::Void, 2),
                RawParamDirection::Out,
            )],
            raw(RawNativeType::I32, 0),
        );
        let blocked = method(
            "UseRecord",
            5,
            vec![param(
                "record",
                raw(
                    RawNativeType::Named {
                        namespace: "Tests".into(),
                        name: "Incomplete".into(),
                        kind: RawNamedKind::Struct,
                        iid: None,
                        layout: None,
                    },
                    0,
                ),
                RawParamDirection::In,
            )],
            raw(RawNativeType::I32, 0),
        );
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![complete, manual, blocked]),
            identity(),
        )
        .expect("partial unsafe companion");

        assert_eq!(output.metadata_complete_methods, 1);
        assert_eq!(output.manual_methods, 1);
        assert_eq!(output.blocked_methods, 1);
        let js = output.js.as_deref().expect("executable JavaScript");
        let dts = output.dts.as_deref().expect("executable declarations");
        assert!(js.contains("getURL()"));
        assert!(js.contains("allocate(value)"));
        assert!(js.contains("__prepareStrategy(value, 'UnsafePointerOutput'"));
        assert!(js.contains("__failStrategy(_prepared[_index])"));
        assert!(js.contains("__finishStrategy(_manualRecord0)"));
        assert!(
            dts.contains("allocate<T0>(value: UnsafePointerOutput<T0>): readonly [number, T0]")
        );
        assert!(!js.contains("useRecord("));
        assert!(!js.contains("implementation"));
        assert!(dts.contains("private constructor()"));
        assert_eq!(
            output
                .support
                .methods
                .iter()
                .map(|method| method.status.as_str())
                .collect::<Vec<_>>(),
            vec![
                "raw_metadata_complete",
                "raw_manual_contract",
                "raw_runtime_blocked"
            ]
        );

        let repeated = generate_unsafe_interface_files_with_identity(
            &interface(vec![
                method("GetURL", 3, Vec::new(), raw(RawNativeType::I32, 0)),
                method(
                    "Allocate",
                    4,
                    vec![param(
                        "value",
                        raw(RawNativeType::Void, 2),
                        RawParamDirection::Out,
                    )],
                    raw(RawNativeType::I32, 0),
                ),
                method(
                    "UseRecord",
                    5,
                    vec![param(
                        "record",
                        raw(
                            RawNativeType::Named {
                                namespace: "Tests".into(),
                                name: "Incomplete".into(),
                                kind: RawNamedKind::Struct,
                                iid: None,
                                layout: None,
                            },
                            0,
                        ),
                        RawParamDirection::In,
                    )],
                    raw(RawNativeType::I32, 0),
                ),
            ]),
            identity(),
        )
        .expect("deterministic companion");
        assert_eq!(output.js, repeated.js);
        assert_eq!(output.dts, repeated.dts);
        assert_eq!(
            serde_json::to_vec(&output.support).unwrap(),
            serde_json::to_vec(&repeated.support).unwrap()
        );
    }

    #[test]
    fn generated_unsafe_files_honor_custom_runtime_imports() {
        let previous = crate::codegen::project::get_import_name();
        crate::codegen::project::set_import_name("../dist/com.js");
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "GetValue",
                3,
                Vec::new(),
                raw(RawNativeType::I32, 0),
            )]),
            identity(),
        )
        .unwrap();
        let runtime = render_unsafe_runtime_files();
        crate::codegen::project::set_import_name(&previous);

        for content in [output.js.unwrap(), output.dts.unwrap()] {
            assert!(content.contains("\"../../../dist/com-unsafe-raw.js\""));
            assert!(!content.contains("@microsoft/dynwinrt/com/unsafe/raw"));
        }
        for (_, content) in runtime {
            assert!(content.contains("\"../../dist/com-unsafe-raw.js\""));
            assert!(!content.contains("@microsoft/dynwinrt/com/unsafe/raw"));
        }
    }

    #[test]
    fn bstr_and_raw_pointer_inputs_use_exact_runtime_conversions() {
        let bstr = raw(
            RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: "BSTR".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            0,
        );
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "OpenNamespace",
                3,
                vec![
                    param("name", bstr.clone(), RawParamDirection::In),
                    param(
                        "context",
                        raw(
                            RawNativeType::Named {
                                namespace: "Tests".into(),
                                name: "IContext".into(),
                                kind: RawNamedKind::Interface,
                                iid: Some("00000002-0000-0000-c000-000000000046".into()),
                                layout: None,
                            },
                            0,
                        ),
                        RawParamDirection::In,
                    ),
                    param(
                        "output",
                        raw(
                            RawNativeType::Named {
                                namespace: "Tests".into(),
                                name: "IOutput".into(),
                                kind: RawNamedKind::Interface,
                                iid: Some("00000001-0000-0000-c000-000000000046".into()),
                                layout: None,
                            },
                            2,
                        ),
                        RawParamDirection::Out,
                    ),
                ],
                raw(RawNativeType::Void, 0),
            )]),
            identity(),
        )
        .expect("BSTR/raw pointer companion");

        let js = output.js.as_deref().expect("executable JavaScript");
        let dts = output.dts.as_deref().expect("executable declarations");
        assert!(js.contains("openNamespace(name, context, output)"));
        assert!(js.contains("DynCom.bstrType()"));
        assert!(js.contains("DynCom.bstr(name)"));
        assert!(js.contains(
            "DynCom.interfaceType(WinGuid.parse(\"00000002-0000-0000-c000-000000000046\"))"
        ));
        assert!(js.contains("(context && context.nativeValue ? context.nativeValue : context)"));
        assert!(js.contains("_rawPointer(output, 'output')"));
        assert!(!js.contains("DynComRawStructLayout.fromDescriptor"));
        assert!(dts.contains(
            "openNamespace(name: string, context: DynWinRtValue | { readonly nativeValue: DynWinRtValue }, output: DynComRawMemory | DynComRawPointer): void"
        ));
        assert_eq!(
            render_return_conversion(&bstr, "result", &mut LayoutRegistry::new()).unwrap(),
            "DynCom.takeBstr(result)"
        );
    }

    #[test]
    fn complete_aggregate_uses_architecture_selected_raw_layout() {
        let record = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "RECORD".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(RawNativeLayoutSet {
                    recursive: false,
                    variants: vec![RawNativeLayout {
                        architectures: 7,
                        kind: RawLayoutKind::Sequential,
                        packing: RawPacking::Default,
                        declared_size: Some(4),
                        fields: vec![RawNativeField {
                            name: "value".into(),
                            typ: raw(RawNativeType::U32, 0),
                            explicit_offset: Some(0),
                            fixed_count: None,
                            bitfield: false,
                            flexible_array: false,
                        }],
                        is_union: false,
                    }],
                })),
            },
            0,
        );
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "UseRecord",
                3,
                vec![param("record", record, RawParamDirection::In)],
                raw(RawNativeType::Void, 0),
            )]),
            identity(),
        )
        .expect("aggregate companion");
        let js = output.js.as_deref().expect("aggregate JavaScript");

        assert!(js.contains("DynComRawStructLayout.fromDescriptor"));
        assert!(js.contains("\\\"kind\\\":\\\"u32\\\""));
        assert!(js.contains("\\\"offset\\\":0"));
        assert!(js.contains("_layout0.byValueType()"));
        assert!(
            output
                .dts
                .as_deref()
                .expect("aggregate declarations")
                .contains("useRecord(record: DynWinRtValue): void")
        );
    }

    #[test]
    fn native_width_scalars_use_target_width_bigint_conversions() {
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![
                method(
                    "RoundTripSigned",
                    3,
                    vec![param(
                        "value",
                        raw(RawNativeType::ISize, 0),
                        RawParamDirection::In,
                    )],
                    raw(RawNativeType::ISize, 0),
                ),
                method(
                    "RoundTripUnsigned",
                    4,
                    vec![param(
                        "value",
                        raw(RawNativeType::USize, 0),
                        RawParamDirection::In,
                    )],
                    raw(RawNativeType::USize, 0),
                ),
                method(
                    "WriteSigned",
                    5,
                    vec![param(
                        "value",
                        raw(RawNativeType::ISize, 1),
                        RawParamDirection::Out,
                    )],
                    raw(RawNativeType::Void, 0),
                ),
            ]),
            identity(),
        )
        .expect("native-width companion");
        let js = output.js.as_deref().expect("native-width JavaScript");
        let dts = output.dts.as_deref().expect("native-width declarations");

        assert!(js.contains(
            "new DynComMethodSig().addIn(DynCom.isizeType()).returns(DynCom.isizeType())"
        ));
        assert!(js.contains(
            "new DynComMethodSig().addIn(DynCom.usizeType()).returns(DynCom.usizeType())"
        ));
        assert!(js.contains("[DynCom.isize(value)]"));
        assert!(js.contains("[DynCom.usize(value)]"));
        assert!(js.contains("return DynCom.toIsizeBigint(_out[0])"));
        assert!(js.contains("return DynCom.toUsizeBigint(_out[0])"));
        assert!(js.contains("new DynComMethodSig().addIn(DynCom.pointerType()).returnsVoid()"));
        assert!(js.contains("_rawPointer(value, 'value')"));
        assert!(dts.contains("roundTripSigned(value: bigint): bigint"));
        assert!(dts.contains("roundTripUnsigned(value: bigint): bigint"));
        assert!(dts.contains("writeSigned(value: DynComRawMemory | DynComRawPointer): void"));
        assert!(!js.contains("DynCom.toI64Bigint(_out[0])"));
        assert!(!js.contains("DynCom.toU64Bigint(_out[0])"));
    }

    #[test]
    fn direct_void_pointer_return_is_not_lowered_as_void() {
        let hresult_pointer = raw(
            RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HRESULT".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            1,
        );
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![
                method(
                    "GetBufferPointer",
                    3,
                    Vec::new(),
                    raw(RawNativeType::Void, 1),
                ),
                method("GetStatusPointer", 4, Vec::new(), hresult_pointer),
            ]),
            identity(),
        )
        .expect("direct void pointer companion");
        let js = output.js.as_deref().unwrap();
        let dts = output.dts.as_deref().unwrap();

        assert!(js.contains("returns(DynCom.pointerType())"));
        assert!(js.contains("DynComRawPointer.fromAddress(DynCom.asPointerBigint(_out[0]))"));
        assert!(!js.contains("returnsVoid()"));
        assert!(dts.contains("getBufferPointer(unsafeContract: UnsafeRawCall): DynComRawPointer"));
        assert!(dts.contains("getStatusPointer(unsafeContract: UnsafeRawCall): DynComRawPointer"));
    }

    #[test]
    fn manual_only_method_produces_an_executable_strategy_companion() {
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "Allocate",
                3,
                vec![param(
                    "value",
                    raw(RawNativeType::Void, 2),
                    RawParamDirection::Out,
                )],
                raw(RawNativeType::I32, 0),
            )]),
            identity(),
        )
        .expect("manual unsafe projection");

        assert!(
            output
                .js
                .as_deref()
                .unwrap()
                .contains("UnsafePointerOutput")
        );
        assert!(
            output
                .dts
                .as_deref()
                .unwrap()
                .contains("allocate<T0>(value: UnsafePointerOutput<T0>): readonly [number, T0]",)
        );
        assert_eq!(output.metadata_complete_methods, 0);
        assert_eq!(output.manual_methods, 1);
        assert_eq!(output.support.methods[0].status, "raw_manual_contract");
        assert_eq!(
            output.support.methods[0].strategy_requirements[0].strategy,
            "UnsafePointerOutput"
        );
    }

    #[test]
    fn invalid_interface_identity_or_root_cannot_render_a_companion() {
        for iid in ["", "00000000-0000-0000-0000-000000000000"] {
            let mut meta = interface(vec![method(
                "GetValue",
                3,
                Vec::new(),
                raw(RawNativeType::I32, 0),
            )]);
            meta.interface.iid = iid.into();
            let output = generate_unsafe_interface_files_with_identity(&meta, identity()).unwrap();
            assert!(output.js.is_none());
            assert!(output.dts.is_none());
            assert_eq!(output.blocked_methods, 1);
            assert!(
                output.support.methods[0]
                    .reasons
                    .contains(&"missing_interface_iid".to_string())
            );
        }

        let mut meta = interface(vec![method(
            "GetValue",
            3,
            Vec::new(),
            raw(RawNativeType::I32, 0),
        )]);
        meta.is_iunknown_rooted = false;
        let output = generate_unsafe_interface_files_with_identity(&meta, identity()).unwrap();
        assert!(output.js.is_none());
        assert_eq!(output.support.root, "Unknown");
        assert!(
            output.support.methods[0]
                .reasons
                .contains(&"missing_interface_root".to_string())
        );
        assert!(
            output.support.methods[0]
                .reasons
                .contains(&"not_addressable".to_string())
        );
    }

    #[test]
    fn namespace_collisions_batch_and_sequential_orders_converge_to_deep_modules() {
        let build = |namespace: &str, iid: &str| {
            let mut meta = interface(vec![method(
                "GetValue",
                3,
                Vec::new(),
                raw(RawNativeType::I32, 0),
            )]);
            meta.interface.namespace = namespace.into();
            meta.interface.name = "IFoo".into();
            meta.interface.iid = iid.into();
            generate_unsafe_interface_files_with_identity(&meta, identity()).unwrap()
        };
        let first = build("Contoso.A", "10000000-0000-0000-c000-000000000046");
        let second = build("Contoso.B", "20000000-0000-0000-c000-000000000046");
        assert_eq!(first.class_name, "IFooUnsafe");
        assert_eq!(second.class_name, "IFooUnsafe");
        assert_eq!(first.module_path, "contoso/a/IFooUnsafe");
        assert_eq!(second.module_path, "contoso/b/IFooUnsafe");
        assert!(first.js.as_deref().unwrap().contains("class IFooUnsafe"));
        assert!(
            first
                .dts
                .as_deref()
                .unwrap()
                .contains("declare class IFooUnsafe")
        );

        let forward =
            render_unsafe_package_files(&[first.support.clone(), second.support.clone()]).unwrap();
        let reverse =
            render_unsafe_package_files(&[second.support.clone(), first.support.clone()]).unwrap();
        assert_eq!(forward, reverse);
        let index = &forward
            .iter()
            .find(|(path, _)| path == "unsafe/index.js")
            .unwrap()
            .1;
        assert!(!index.contains("IFooUnsafe"));
        let support = &forward
            .iter()
            .find(|(path, _)| path == "unsafe/support.json")
            .unwrap()
            .1;
        assert!(support.find("Contoso.A.IFoo").unwrap() < support.find("Contoso.B.IFoo").unwrap());

        let unique = render_unsafe_package_files(&[first.support]).unwrap();
        let cjs = &unique
            .iter()
            .find(|(path, _)| path == "unsafe/index.js")
            .unwrap()
            .1;
        let esm = &unique
            .iter()
            .find(|(path, _)| path == "unsafe/index.mjs")
            .unwrap()
            .1;
        let dts = &unique
            .iter()
            .find(|(path, _)| path == "unsafe/index.d.ts")
            .unwrap()
            .1;
        assert!(cjs.contains("require('./contoso/a/IFooUnsafe.js').IFooUnsafe"));
        assert!(esm.contains("export { IFooUnsafe } from './contoso/a/IFooUnsafe.js'"));
        assert_eq!(esm, dts);
    }

    #[test]
    fn windows_module_paths_reject_case_collisions_traversal_and_reserved_names() {
        for invalid in [
            "",
            "/absolute",
            "\\rooted",
            "C:/absolute",
            "Contoso//IFooUnsafe",
            "Contoso/./IFooUnsafe",
            "Contoso/../IFooUnsafe",
            "Contoso/IFooUnsafe.",
            "Contoso/IFooUnsafe ",
            "Contoso/CON",
            "Contoso/con.txt",
            "Contoso/COM1.js",
            "Contoso/LPT9.d.ts",
            "Contoso/NUL\0suffix",
        ] {
            assert!(
                windows_relative_path_key(invalid).is_err(),
                "accepted unsafe Windows path `{invalid}`"
            );
        }
        assert_eq!(
            windows_relative_path_key("Contoso/A/IFooUnsafe.js").unwrap(),
            "contoso/a/ifoounsafe.js"
        );

        let build = |namespace: &str, name: &str, iid: &str| {
            let mut meta = interface(vec![method(
                "GetValue",
                3,
                Vec::new(),
                raw(RawNativeType::I32, 0),
            )]);
            meta.interface.namespace = namespace.into();
            meta.interface.name = name.into();
            meta.interface.iid = iid.into();
            generate_unsafe_interface_files_with_identity(&meta, identity()).unwrap()
        };
        let upper = build("Contoso.A", "IFoo", "10000000-0000-0000-c000-000000000046");
        let lower_namespace = build("contoso.a", "IFoo", "20000000-0000-0000-c000-000000000046");
        assert!(
            render_unsafe_package_files(&[upper.support.clone(), lower_namespace.support])
                .unwrap_err()
                .contains("case-insensitive Windows path key")
        );
        let lower_type = build("Contoso.A", "ifoo", "30000000-0000-0000-c000-000000000046");
        assert!(
            render_unsafe_package_files(&[upper.support.clone(), lower_type.support])
                .unwrap_err()
                .contains("case-insensitive Windows path key")
        );

        let mut traversal = upper.support.clone();
        traversal.module_path = "../IFooUnsafe".into();
        assert!(
            render_unsafe_package_files(&[traversal])
                .unwrap_err()
                .contains("does not match derived path")
        );
        let mut suffix_collision = upper.support;
        suffix_collision.unsafe_class = "IFoo".into();
        assert!(
            render_unsafe_package_files(&[suffix_collision])
                .unwrap_err()
                .contains("does not match interface")
        );
        assert!(unsafe_module_path("Contoso.CON", "IFooUnsafe").is_err());
    }

    #[test]
    fn official_stage2_coverage_is_exact() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let coverage = measure_stage2_coverage(&winmd).unwrap();
        println!("{}", serde_json::to_string(&coverage).unwrap());
        assert_eq!(coverage.x64_manual_interfaces, 1_554);
        assert_eq!(coverage.x64_manual_interfaces_with_executable_method, 1_550);
        assert_eq!(
            coverage.x64_manual_interfaces_with_executable_manual_method,
            1_549
        );
        assert_eq!(coverage.executable_manual_methods, 6_343);
        assert_eq!(coverage.remaining_manual_methods, 0);
        assert_eq!(coverage.runtime_blocked_methods, 1_163);
    }

    #[test]
    fn strategy_runtime_keeps_state_and_lifecycle_helpers_private() {
        let files = render_unsafe_runtime_files();
        let js = &files
            .iter()
            .find(|(path, _)| path == "unsafe/runtime.js")
            .unwrap()
            .1;
        let dts = &files
            .iter()
            .find(|(path, _)| path == "unsafe/runtime.d.ts")
            .unwrap()
            .1;
        assert!(js.contains("const outputState = new WeakMap()"));
        assert!(js.contains("function __prepareStrategy("));
        assert!(js.contains("function __validateStrategySpans("));
        assert!(js.contains("const ownedFinalizer = new FinalizationRegistry("));
        assert!(!js.contains("__testRunOwnedFinalizer"));
        assert!(!js.contains("__testCreateOwnedPointer"));
        assert!(!dts.contains("prepare("));
        assert!(!dts.contains("finish("));
        assert!(!dts.contains("failure("));
        assert!(dts.contains("takeFailurePointer(): DynComRawPointer"));
        assert!(dts.contains("private readonly __resultType: (value: T) => T"));

        let package = render_unsafe_package_files(&[]).unwrap();
        let index = &package
            .iter()
            .find(|(path, _)| path == "unsafe/index.js")
            .unwrap()
            .1;
        assert!(!index.contains("__prepareStrategy"));
        assert!(!index.contains("__runOwnedFinalizerForTest"));
    }

    #[test]
    fn interface_replacement_requires_exact_double_pointer_and_parameter_reason() {
        let build = |depth: usize| {
            let interface_value = raw(
                RawNativeType::Named {
                    namespace: "Tests".into(),
                    name: "IThing".into(),
                    kind: RawNamedKind::Interface,
                    iid: Some("30000000-0000-0000-c000-000000000046".into()),
                    layout: None,
                },
                depth,
            );
            generate_unsafe_interface_files_with_identity(
                &interface(vec![method(
                    "Replace",
                    3,
                    vec![
                        param("value", interface_value, RawParamDirection::InOut),
                        param(
                            "unrelated",
                            raw(RawNativeType::Void, 1),
                            RawParamDirection::In,
                        ),
                    ],
                    raw(RawNativeType::Void, 0),
                )]),
                identity(),
            )
            .unwrap()
        };
        let single = build(0);
        let double = build(1);
        let triple = build(2);
        let strategies = |output: &UnsafeGeneratedOutput| {
            output.support.methods[0]
                .strategy_requirements
                .iter()
                .filter(|requirement| requirement.parameter_index == Some(0))
                .map(|requirement| requirement.strategy.clone())
                .collect::<Vec<_>>()
        };
        assert!(!strategies(&single).contains(&"UnsafeInterfaceReplacement".into()));
        assert_eq!(strategies(&double), vec!["UnsafeInterfaceReplacement"]);
        assert!(!strategies(&triple).contains(&"UnsafeInterfaceReplacement".into()));
        assert!(
            single
                .js
                .as_deref()
                .unwrap()
                .contains("addIn(DynCom.interfaceType(")
        );
        assert!(
            double
                .js
                .as_deref()
                .unwrap()
                .contains("addIn(DynCom.pointerType())")
        );
        assert!(
            triple
                .js
                .as_deref()
                .unwrap()
                .contains("addIn(DynCom.pointerType())")
        );
    }

    #[test]
    fn missing_count_strategy_does_not_infect_unrelated_pointer_parameters() {
        let unrelated = raw(
            RawNativeType::Named {
                namespace: "Tests".into(),
                name: "IUnrelated".into(),
                kind: RawNamedKind::Interface,
                iid: Some("40000000-0000-0000-c000-000000000046".into()),
                layout: None,
            },
            0,
        );
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "Copy",
                3,
                vec![
                    param(
                        "buffer",
                        raw(RawNativeType::Array(Box::new(raw(RawNativeType::U8, 0))), 0),
                        RawParamDirection::In,
                    ),
                    param("unrelated", unrelated, RawParamDirection::In),
                ],
                raw(RawNativeType::Void, 0),
            )]),
            identity(),
        )
        .unwrap();
        let requirements = &output.support.methods[0].strategy_requirements;
        assert_eq!(requirements.len(), 1);
        assert_eq!(requirements[0].parameter_index, Some(0));
        assert_eq!(requirements[0].strategy, "UnsafeCountedBuffer");
        assert!(output.dts.as_deref().unwrap().contains(
            "copy(buffer: UnsafeCountedBuffer, unrelated: DynWinRtValue | { readonly nativeValue: DynWinRtValue }): void"
        ));
    }

    #[test]
    fn writable_pointee_requirements_carry_direction_and_target_layouts() {
        let nested_layout = RawNativeLayoutSet {
            recursive: false,
            variants: vec![RawNativeLayout {
                architectures: 0b111,
                kind: RawLayoutKind::Sequential,
                packing: crate::com_metadata::RawPacking::Default,
                declared_size: None,
                fields: vec![crate::com_metadata::RawNativeField {
                    name: "pointer".into(),
                    typ: raw(RawNativeType::Void, 1),
                    explicit_offset: None,
                    fixed_count: None,
                    bitfield: false,
                    flexible_array: false,
                }],
                is_union: false,
            }],
        };
        let output = generate_unsafe_interface_files_with_identity(
            &interface(vec![method(
                "Write",
                3,
                vec![
                    param(
                        "known",
                        raw(
                            RawNativeType::Named {
                                namespace: "Tests".into(),
                                name: "Nested".into(),
                                kind: RawNamedKind::Struct,
                                iid: None,
                                layout: Some(Box::new(nested_layout)),
                            },
                            1,
                        ),
                        RawParamDirection::InOut,
                    ),
                    param(
                        "unknown",
                        raw(RawNativeType::Void, 1),
                        RawParamDirection::Out,
                    ),
                    param("actual", raw(RawNativeType::U32, 1), RawParamDirection::Out),
                ],
                raw(RawNativeType::Void, 0),
            )]),
            identity(),
        )
        .unwrap();
        let method = &output.support.methods[0];
        let known = method
            .strategy_requirements
            .iter()
            .find(|requirement| requirement.parameter_name.as_deref() == Some("known"))
            .unwrap();
        assert_eq!(known.strategy, "UnsafePointee");
        assert_eq!(known.direction.as_deref(), Some("inout"));
        assert_eq!(known.nullable, Some(false));
        let layouts = known.pointee_layouts.as_ref().unwrap();
        assert_eq!(layouts["x64"].as_ref().unwrap().size, 8);
        assert_eq!(layouts["x64"].as_ref().unwrap().alignment, 8);
        assert_eq!(layouts["i686"].as_ref().unwrap().size, 4);
        assert_eq!(layouts["arm64"].as_ref().unwrap().size, 8);

        let unknown = method
            .strategy_requirements
            .iter()
            .find(|requirement| requirement.parameter_name.as_deref() == Some("unknown"))
            .unwrap();
        assert_eq!(unknown.strategy, "UnsafePointee");
        assert_eq!(unknown.direction.as_deref(), Some("out"));
        assert!(
            unknown
                .pointee_layouts
                .as_ref()
                .unwrap()
                .values()
                .all(Option::is_none)
        );

        let js = output.js.as_deref().unwrap();
        assert!(js.contains("__prepareStrategy(known, 'UnsafePointee'"));
        assert!(js.contains("__prepareWritableStorage(actual, 'actual'"));
        assert!(js.contains("\"direction\":\"inout\""));
        assert!(js.contains("\"size\":8"));
    }
}

pub fn render_unsafe_package_files(
    supports: &[UnsafeInterfaceSupport],
) -> Result<Vec<(String, String)>, String> {
    validate_unsafe_supports(supports)?;
    let mut supports = supports.to_vec();
    supports.sort_by(|left, right| left.interface_name.cmp(&right.interface_name));
    let executable = supports
        .iter()
        .filter(|support| {
            support
                .methods
                .iter()
                .any(|method| method.status != "raw_runtime_blocked")
        })
        .collect::<Vec<_>>();
    let mut short_name_counts = BTreeMap::<&str, usize>::new();
    for support in &executable {
        *short_name_counts
            .entry(support.unsafe_class.as_str())
            .or_default() += 1;
    }
    let unique = executable
        .iter()
        .copied()
        .filter(|support| short_name_counts[support.unsafe_class.as_str()] == 1)
        .collect::<Vec<_>>();
    let runtime_names = [
        "UnsafePointee",
        "UnsafeRawCall",
        "UnsafeCountedBuffer",
        "UnsafeOwnedPointer",
        "UnsafePointerOutput",
        "UnsafeHandleOutput",
        "UnsafeInterfaceReplacement",
        "UnsafeInterfaceReplacementResult",
    ];
    let mut cjs = runtime_names
        .iter()
        .map(|name| format!("exports.{name} = require('./runtime.js').{name};\n"))
        .collect::<String>();
    cjs.push_str(
        &unique
            .iter()
            .map(|support| {
                format!(
                    "exports.{0} = require('./{1}.js').{0};\n",
                    support.unsafe_class, support.module_path
                )
            })
            .collect::<String>(),
    );
    let mut esm = format!(
        "export {{ {} }} from './runtime.js';\n",
        runtime_names.join(", ")
    );
    esm.push_str(
        &unique
            .iter()
            .map(|support| {
                format!(
                    "export {{ {} }} from './{}.js';\n",
                    support.unsafe_class, support.module_path
                )
            })
            .collect::<String>(),
    );
    let dts = esm.clone();
    let support = serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": UNSAFE_SUPPORT_SCHEMA_VERSION,
        "interfaces": supports,
    }))
    .map_err(|error| format!("Failed to serialize generated unsafe support: {error}"))?;
    let mut files = vec![
        ("unsafe/index.js".into(), format!("'use strict';\n{cjs}")),
        ("unsafe/index.mjs".into(), esm),
        ("unsafe/index.d.ts".into(), dts),
        (
            "unsafe/package.json".into(),
            "{\n  \"type\": \"commonjs\",\n  \"sideEffects\": false\n}\n".into(),
        ),
        ("unsafe/support.json".into(), format!("{support}\n")),
    ];
    files.extend(render_unsafe_runtime_files());
    Ok(files)
}

fn render_unsafe_runtime_files() -> Vec<(String, String)> {
    let raw_runtime_import =
        serde_json::to_string(&super::javascript::render::com_raw_runtime_import_name_for_depth(1))
            .expect("serializing the runtime import name cannot fail");
    let mut js = r#"// Generated by dynwinrt-codegen — do not edit
'use strict';
const {
  DynCom,
  DynComUnsafe,
  DynComRaw,
  DynComRawCleanup,
  DynComRawMemory,
  DynComRawOwnedComPointer,
  DynComRawPointer,
} = require(__DYNWINRT_RAW_RUNTIME_IMPORT__);

const tokens = Object.freeze({
  pointee: Symbol('UnsafePointee'),
  raw: Symbol('UnsafeRawCall'),
  output: Symbol('UnsafePointerOutput'),
  owned: Symbol('UnsafeOwnedPointer'),
  handle: Symbol('UnsafeHandleOutput'),
  counted: Symbol('UnsafeCountedBuffer'),
  replacement: Symbol('UnsafeInterfaceReplacement'),
  replacementResult: Symbol('UnsafeInterfaceReplacementResult'),
});

const pointeeState = new WeakMap();
const rawCallState = new WeakMap();
const countedState = new WeakMap();
const outputState = new WeakMap();
const ownedState = new WeakMap();
const handleState = new WeakMap();
const replacementState = new WeakMap();
const replacementResultState = new WeakMap();
const preparationState = new WeakMap();

function brand(instance, prototype, stateMap, state) {
  if (Object.getPrototypeOf(instance) !== prototype) throw new TypeError('Unsafe strategy subclassing is not supported');
  stateMap.set(instance, state);
  return Object.freeze(instance);
}

function stateFor(value, constructor, stateMap, name) {
  if (Object.getPrototypeOf(value) !== constructor.prototype || !stateMap.has(value)) {
    throw new TypeError(`${name} must be an exact generated ${constructor.name}`);
  }
  return stateMap.get(value);
}

function pointerSpan(pointer, width, name) {
  const start = pointer.address;
  const size = BigInt(width);
  const end = start + size;
  const limit = 1n << BigInt(DynComRaw.pointerSize() * 8);
  if (start < 0n || size < 0n || start >= limit || end < start || end > limit) {
    throw new RangeError(`${name} native span overflow`);
  }
  return Object.freeze({ start, end, name });
}

function rawPointer(value, name, nullable) {
  if (value === null && nullable) return DynComRawPointer.null();
  let pointer;
  if (value instanceof DynComRawMemory) pointer = value.pointer();
  else if (value instanceof DynComRawPointer) pointer = value;
  else throw new TypeError(`${name} must be DynComRawMemory or DynComRawPointer${nullable ? ' or null' : ''}`);
  if (pointer.isNull && !nullable) throw new TypeError(`${name} requires a non-null native pointer`);
  return pointer;
}

function argumentValue(value, name, nullable) {
  return rawPointer(value, name, nullable).toValue();
}

function currentPointeeLayout(contract, name) {
  if (!contract || !['in', 'out', 'inout'].includes(contract.direction) || typeof contract.nullable !== 'boolean') {
    throw new TypeError(`${name} has an invalid generated pointee contract`);
  }
  const target = process.arch === 'ia32' ? 'i686' : process.arch;
  if (!['x64', 'i686', 'arm64'].includes(target)
      || !contract.pointeeLayouts
      || !Object.prototype.hasOwnProperty.call(contract.pointeeLayouts, target)) {
    throw new Error(`${name} has no pointee contract for ${process.arch}`);
  }
  const layout = contract.pointeeLayouts[target];
  if (layout !== null && (!Number.isSafeInteger(layout.size) || layout.size <= 0
      || !Number.isSafeInteger(layout.alignment) || layout.alignment <= 0
      || (layout.alignment & (layout.alignment - 1)) !== 0)) {
    throw new TypeError(`${name} has an invalid generated pointee layout`);
  }
  return layout;
}

function preparePointeeValue(value, nullableStrategy, contract, name) {
  const layout = currentPointeeLayout(contract, name);
  const nullable = contract.nullable && nullableStrategy;
  if (value === null) {
    if (!nullable) throw new TypeError(`${name} requires a non-null native pointer`);
    return { argument: DynComRawPointer.null().toValue(), spans: [] };
  }
  if (contract.direction === 'in') {
    return { argument: argumentValue(value, name, nullable), spans: [] };
  }
  if (value instanceof DynComRawPointer && value.isNull) {
    if (!nullable) throw new TypeError(`${name} requires a non-null native pointer`);
    return { argument: value.toValue(), spans: [] };
  }
  if (!(value instanceof DynComRawMemory)) {
    throw new TypeError(`${name} writable pointee requires bounded DynComRawMemory`);
  }
  if (value.released) throw new Error(`${name} writable pointee storage has been released`);
  const pointer = value.pointer();
  if (pointer.isNull) {
    if (!nullable) throw new TypeError(`${name} requires a non-null native pointer`);
    return { argument: pointer.toValue(), spans: [] };
  }
  const spanWidth = layout === null ? value.size : BigInt(layout.size);
  if (spanWidth <= 0n) throw new RangeError(`${name} writable pointee storage must be non-empty`);
  if (value.size < spanWidth) {
    throw new RangeError(`${name} writable pointee storage is smaller than its native layout`);
  }
  if (layout !== null) {
    const alignment = BigInt(layout.alignment);
    if (value.alignment < alignment || pointer.address % alignment !== 0n) {
      throw new RangeError(`${name} writable pointee storage does not satisfy native alignment`);
    }
  }
  return {
    argument: pointer.toValue(),
    spans: [pointerSpan(pointer, spanWidth, name)],
  };
}

class UnsafePointee {
  constructor(token, value, nullable) {
    if (token !== tokens.pointee) throw new TypeError('Use UnsafePointee factory methods');
    return brand(this, UnsafePointee.prototype, pointeeState, { value, nullable });
  }
  static required(value) { return new UnsafePointee(tokens.pointee, value, false); }
  static nullable(value = null) { return new UnsafePointee(tokens.pointee, value, true); }
}

class UnsafeRawCall {
  constructor(token, value, acknowledgement) {
    if (token !== tokens.raw) throw new TypeError('Use UnsafeRawCall factory methods');
    return brand(this, UnsafeRawCall.prototype, rawCallState, { value, acknowledgement });
  }
  static value(value) { return new UnsafeRawCall(tokens.raw, value, false); }
  static acknowledge() { return new UnsafeRawCall(tokens.raw, null, true); }
}

class UnsafeCountedBuffer {
  constructor(token, value, nullable) {
    if (token !== tokens.counted) throw new TypeError('Use UnsafeCountedBuffer factory methods');
    return brand(this, UnsafeCountedBuffer.prototype, countedState, { value, nullable });
  }
  static required(value) { return new UnsafeCountedBuffer(tokens.counted, value, false); }
  static nullable(value = null) { return new UnsafeCountedBuffer(tokens.counted, value, true); }
}

function finalizeOwnedState(state) {
  if (state.released || state.finalized) return state.released;
  state.finalized = true;
  try {
    state.cleanup(state.pointer);
    state.released = true;
  } catch {
    // Finalizers cannot surface cleanup failure. The native resource is leaked
    // rather than risking a second cleanup with unknown partial effects.
  }
  return state.released;
}

const ownedFinalizer = new FinalizationRegistry(finalizeOwnedState);

function releaseOwnedState(state, unregisterToken = null) {
  if (state.released) return;
  if (state.finalized) throw new Error('Unsafe owned pointer finalizer has already run');
  state.cleanup(state.pointer);
  state.released = true;
  if (unregisterToken) ownedFinalizer.unregister(unregisterToken);
}

class UnsafeOwnedPointer {
  constructor(token, pointer, cleanup) {
    if (token !== tokens.owned) throw new TypeError('UnsafeOwnedPointer cannot be constructed directly');
    const unregisterToken = {};
    const state = { pointer, cleanup, released: false, finalized: false, unregisterToken };
    ownedState.set(this, state);
    ownedFinalizer.register(this, state, unregisterToken);
    return Object.freeze(this);
  }
  get pointer() {
    const state = stateFor(this, UnsafeOwnedPointer, ownedState, 'owned pointer');
    if (state.released) throw new Error('Unsafe owned pointer has been released');
    return state.pointer;
  }
  get released() { return stateFor(this, UnsafeOwnedPointer, ownedState, 'owned pointer').released; }
  view(size, alignment) { return DynComRawMemory.fromUnsafePointer(this.pointer, size, alignment); }
  release() {
    const state = stateFor(this, UnsafeOwnedPointer, ownedState, 'owned pointer');
    releaseOwnedState(state, state.unregisterToken);
  }
}

const pointerCleanups = Object.freeze({
  coTaskMem: pointer => DynComRawCleanup.coTaskMemFree(pointer),
  bstr: pointer => DynComRawCleanup.sysFreeString(pointer),
  local: pointer => DynComRawCleanup.localFree(pointer),
  global: pointer => DynComRawCleanup.globalFree(pointer),
  closeHandle: pointer => DynComRawCleanup.closeHandle(pointer),
  destroyIcon: pointer => DynComRawCleanup.destroyIcon(pointer),
  deleteObject: pointer => DynComRawCleanup.deleteObject(pointer),
});

class UnsafePointerOutput {
  constructor(token, mode, iid, slot, nullable) {
    if (token !== tokens.output) throw new TypeError('Use UnsafePointerOutput factory methods');
    const exactIid = mode === 'com' ? DynComRaw.__validateGuid(iid) : null;
    const actualSlot = slot ?? DynComRawMemory.allocate(DynComRaw.pointerSize(), DynComRaw.pointerSize());
    if (!(actualSlot instanceof DynComRawMemory)) throw new TypeError('output slot must be DynComRawMemory');
    return brand(this, UnsafePointerOutput.prototype, outputState, {
      mode,
      iid: exactIid,
      slot: actualSlot,
      ownsSlot: slot == null,
      nullable,
      phase: 'fresh',
      dispatchEntered: false,
      failurePointer: undefined,
    });
  }
  static unclassified(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'unclassified', null, slot, nullable); }
  static borrowed(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'borrowed', null, slot, nullable); }
  static comOwned(iid, slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'com', iid, slot, nullable); }
  static coTaskMem(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'coTaskMem', null, slot, nullable); }
  static bstr(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'bstr', null, slot, nullable); }
  static localAlloc(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'local', null, slot, nullable); }
  static globalAlloc(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'global', null, slot, nullable); }
  static rawResponsibility(slot = null, nullable = false) { return new UnsafePointerOutput(tokens.output, 'raw', null, slot, nullable); }
  takeFailurePointer() {
    return takeFailurePointer(this);
  }
}

class UnsafeHandleOutput {
  constructor(token, output) {
    if (token !== tokens.handle) throw new TypeError('Use UnsafeHandleOutput factory methods');
    return brand(this, UnsafeHandleOutput.prototype, handleState, { output });
  }
  static borrowed(slot = null, nullable = false) { return new UnsafeHandleOutput(tokens.handle, new UnsafePointerOutput(tokens.output, 'borrowed', null, slot, nullable)); }
  static closeHandle(slot = null, nullable = false) { return new UnsafeHandleOutput(tokens.handle, new UnsafePointerOutput(tokens.output, 'closeHandle', null, slot, nullable)); }
  static destroyIcon(slot = null, nullable = false) { return new UnsafeHandleOutput(tokens.handle, new UnsafePointerOutput(tokens.output, 'destroyIcon', null, slot, nullable)); }
  static deleteObject(slot = null, nullable = false) { return new UnsafeHandleOutput(tokens.handle, new UnsafePointerOutput(tokens.output, 'deleteObject', null, slot, nullable)); }
  static rawResponsibility(slot = null, nullable = false) { return new UnsafeHandleOutput(tokens.handle, new UnsafePointerOutput(tokens.output, 'raw', null, slot, nullable)); }
  takeFailurePointer() { return takeFailurePointer(this); }
}

class UnsafeInterfaceReplacementResult {
  constructor(token, previous, current) {
    if (token !== tokens.replacementResult) throw new TypeError('UnsafeInterfaceReplacementResult cannot be constructed directly');
    replacementResultState.set(this, { previous, current, released: false });
    return Object.freeze(this);
  }
  get previous() { return stateFor(this, UnsafeInterfaceReplacementResult, replacementResultState, 'replacement result').previous; }
  get current() { return stateFor(this, UnsafeInterfaceReplacementResult, replacementResultState, 'replacement result').current; }
  release() {
    const state = stateFor(this, UnsafeInterfaceReplacementResult, replacementResultState, 'replacement result');
    if (state.released) return;
    state.previous?.release();
    if (state.current !== state.previous) state.current?.release();
    state.released = true;
  }
}

class UnsafeInterfaceReplacement {
  constructor(token, mode, owner, iid, slot) {
    if (token !== tokens.replacement) throw new TypeError('Use UnsafeInterfaceReplacement factory methods');
    if (!(owner instanceof DynComRawOwnedComPointer)) throw new TypeError('replacement requires DynComRawOwnedComPointer');
    const exactIid = DynComRaw.__validateGuid(iid);
    const actualSlot = slot ?? DynComRawMemory.allocate(DynComRaw.pointerSize(), DynComRaw.pointerSize());
    if (!(actualSlot instanceof DynComRawMemory)) throw new TypeError('replacement slot must be DynComRawMemory');
    return brand(this, UnsafeInterfaceReplacement.prototype, replacementState, {
      mode,
      owner,
      iid: exactIid,
      slot: actualSlot,
      ownsSlot: slot == null,
      phase: 'fresh',
      dispatchEntered: false,
      oldAddress: owner.address,
      invocationOwner: null,
      failurePointer: undefined,
    });
  }
  static consumesOld(owner, iid, slot = null) { return new UnsafeInterfaceReplacement(tokens.replacement, 'consumes', owner, iid, slot); }
  static preservesOld(owner, iid, slot = null) { return new UnsafeInterfaceReplacement(tokens.replacement, 'preserves', owner, iid, slot); }
  static unchanged(owner, iid, slot = null) { return new UnsafeInterfaceReplacement(tokens.replacement, 'unchanged', owner, iid, slot); }
}

const preparedRecords = new WeakSet();

function preparedFor(record) {
  if (Object.getPrototypeOf(record) !== Object.prototype || !preparedRecords.has(record) || !preparationState.has(record)) {
    throw new TypeError('Prepared strategy record is not authentic');
  }
  return preparationState.get(record);
}

function outputStateFor(strategy) {
  if (Object.getPrototypeOf(strategy) === UnsafePointerOutput.prototype) {
    return outputState.get(strategy);
  }
  if (Object.getPrototypeOf(strategy) === UnsafeHandleOutput.prototype) {
    const handle = stateFor(strategy, UnsafeHandleOutput, handleState, 'handle output');
    return stateFor(handle.output, UnsafePointerOutput, outputState, 'handle output');
  }
  return null;
}

function ensureFresh(state, name) {
  if (state.phase !== 'fresh') throw new Error(`${name} is ${state.phase}; expected fresh`);
}

function outputSpan(state, name) {
  if (state.slot.released) throw new Error(`${name} output slot has been released`);
  if (state.slot.size < BigInt(DynComRaw.pointerSize())) {
    throw new RangeError(`${name} output slot is smaller than pointer width`);
  }
  const pointer = state.slot.pointer();
  if (pointer.isNull) throw new TypeError(`${name} output slot has a null address`);
  if (pointer.address % BigInt(DynComRaw.pointerSize()) !== 0n) {
    throw new RangeError(`${name} output slot is not pointer-aligned`);
  }
  return pointerSpan(pointer, DynComRaw.pointerSize(), name);
}

function opaquePreparation(record) {
  const opaque = Object.freeze({});
  preparationState.set(opaque, record);
  preparedRecords.add(opaque);
  return opaque;
}

function __prepareStrategy(strategy, expected, name, contract) {
  let record;
  if (expected === 'UnsafePointee') {
    const state = stateFor(strategy, UnsafePointee, pointeeState, name);
    const prepared = preparePointeeValue(state.value, state.nullable, contract, name);
    record = { strategy, kind: expected, argument: prepared.argument, spans: prepared.spans, state: null };
  } else if (expected === 'UnsafeRawCall') {
    const state = stateFor(strategy, UnsafeRawCall, rawCallState, name);
    if (state.acknowledgement) throw new TypeError(`${name} requires UnsafeRawCall.value(...)`);
    const argument = state.value instanceof DynComRawMemory || state.value instanceof DynComRawPointer
      ? argumentValue(state.value, name, false)
      : state.value;
    record = { strategy, kind: expected, argument, spans: [], state: null };
  } else if (expected === 'UnsafeCountedBuffer') {
    const state = stateFor(strategy, UnsafeCountedBuffer, countedState, name);
    const pointer = rawPointer(state.value, name, state.nullable);
    const spans = [];
    if (!pointer.isNull) {
      if (!(state.value instanceof DynComRawMemory)) {
        throw new TypeError(`${name} requires DynComRawMemory so its writable span is known`);
      }
      spans.push(pointerSpan(pointer, state.value.size, name));
    }
    record = { strategy, kind: expected, argument: pointer.toValue(), spans, state: null };
  } else if (expected === 'UnsafePointerOutput' || expected === 'UnsafeHandleOutput') {
    const state = outputStateFor(strategy);
    if (!state) throw new TypeError(`${name} must be an exact generated ${expected}`);
    ensureFresh(state, name);
    const span = outputSpan(state, name);
    record = { strategy, kind: expected, argument: state.slot.pointer().toValue(), spans: [span], state };
  } else if (expected === 'UnsafeInterfaceReplacement') {
    const state = stateFor(strategy, UnsafeInterfaceReplacement, replacementState, name);
    ensureFresh(state, name);
    const span = outputSpan(state, name);
    record = { strategy, kind: expected, argument: state.slot.pointer().toValue(), spans: [span], state };
  } else {
    throw new TypeError(`Unsupported unsafe strategy ${expected}`);
  }
  return opaquePreparation(record);
}

function __prepareWritableStorage(value, name, contract) {
  const prepared = preparePointeeValue(value, false, contract, name);
  return opaquePreparation({
    strategy: null,
    kind: 'WritableStorage',
    argument: prepared.argument,
    spans: prepared.spans,
    state: null,
  });
}

function __prepareExactWritableSpan(value, name, optional) {
  if (value === null) {
    if (!optional) throw new TypeError(`${name} requires bounded DynComRawMemory`);
    return opaquePreparation({
      strategy: null,
      kind: 'ExactWritableStorage',
      argument: DynComRawPointer.null().toValue(),
      spans: [],
      state: null,
    });
  }
  if (!(value instanceof DynComRawMemory)) {
    throw new TypeError(`${name} requires bounded DynComRawMemory${optional ? ' or null' : ''}`);
  }
  DynComRaw.__validateExactOutputSlot(value);
  const width = BigInt(DynComRaw.pointerSize());
  const pointer = value.pointer();
  if (!value.readPointer(0).isNull) throw new Error(`${name} output slot must start null`);
  return opaquePreparation({
    strategy: null,
    kind: 'ExactWritableStorage',
    argument: pointer.toValue(),
    spans: [pointerSpan(pointer, width, name)],
    state: null,
  });
}

function __strategyArgument(record) {
  return preparedFor(record).argument;
}

function __assertRawContract(strategy, name = 'unsafeContract') {
  const state = stateFor(strategy, UnsafeRawCall, rawCallState, name);
  if (!state.acknowledgement) {
    throw new TypeError('Use UnsafeRawCall.acknowledge() for a method-level raw contract');
  }
}

function __validateStrategySpans(records) {
  const spans = [];
  const replacementOwners = new Set();
  for (const opaque of records) {
    const record = preparedFor(opaque);
    if (record.kind === 'UnsafeInterfaceReplacement') {
      if (replacementOwners.has(record.state.owner)) {
        throw new RangeError('Replacement owner is used by multiple replacement strategies');
      }
      replacementOwners.add(record.state.owner);
    }
    for (const span of record.spans) {
      for (const existing of spans) {
        if (span.start < existing.end && existing.start < span.end) {
          throw new RangeError(`Unsafe writable spans overlap: ${existing.name} and ${span.name}`);
        }
      }

      spans.push(span);
    }
  }
}

function rollbackActivation(record) {
  const state = record.state;
  try {
    if (record.kind === 'UnsafeInterfaceReplacement' && state.mode === 'consumes' && state.phase === 'prepared') {
      const pointer = state.slot.readPointer(0);
      state.slot.writePointer(0, DynComRawPointer.null());
      if (pointer.isNull) throw new Error('Consumes-old rollback found a null slot');
      state.owner = DynComRawOwnedComPointer.assumeTransferred(pointer);
    } else if (state?.phase === 'prepared') {
      state.slot.writePointer(0, DynComRawPointer.null());
    }
    if (record.kind === 'UnsafeInterfaceReplacement' && state.invocationOwner !== null) {
      state.invocationOwner.release();
      state.invocationOwner = null;
    }
    if (state) {
      state.phase = 'fresh';
      state.dispatchEntered = false;
    }
  } catch (error) {
    if (state) state.phase = 'activation-failed';
    throw error;
  }
}

function __activateStrategies(records) {
  const activated = [];
  try {
    for (const opaque of records) {
      const record = preparedFor(opaque);
      if (record.kind === 'UnsafePointerOutput' || record.kind === 'UnsafeHandleOutput') {
        ensureFresh(record.state, record.kind);
        record.state.slot.writePointer(0, DynComRawPointer.null());
        record.state.phase = 'prepared';
        activated.push(record);
      } else if (record.kind === 'UnsafeInterfaceReplacement' && record.state.mode !== 'consumes') {
        ensureFresh(record.state, record.kind);
        record.state.invocationOwner = record.state.owner.retain();
        try {
          record.state.slot.writePointer(0, record.state.owner.pointer());
          record.state.phase = 'prepared';
          activated.push(record);
        } catch (error) {
          record.state.invocationOwner.release();
          record.state.invocationOwner = null;
          throw error;
        }
      }
    }
    for (const opaque of records) {
      const record = preparedFor(opaque);
      if (record.kind === 'UnsafeInterfaceReplacement' && record.state.mode === 'consumes') {
        ensureFresh(record.state, record.kind);
        record.state.owner.transferTo(record.state.slot);
        record.state.phase = 'prepared';
        activated.push(record);
      }
    }
  } catch (error) {
    const rollbackErrors = [];
    for (let index = activated.length - 1; index >= 0; index--) {
      try { rollbackActivation(activated[index]); } catch (rollbackError) { rollbackErrors.push(rollbackError); }
    }
    if (rollbackErrors.length) throw new AggregateError([error, ...rollbackErrors], 'Unsafe strategy activation and rollback both failed', { cause: error });
    throw error;
  }
}

function __markDispatchEntered(records, dispatch) {
  if (!DynComRaw.__dispatchEntered(dispatch)) return;
  for (const opaque of records) {
    const record = preparedFor(opaque);
    if (record.state?.phase === 'prepared') record.state.dispatchEntered = true;
  }
}

function takeOutputPointer(state) {
  const pointer = state.slot.readPointer(0);
  state.slot.writePointer(0, DynComRawPointer.null());
  if (state.ownsSlot) state.slot.release();
  return pointer;
}

function cleanupOutputPointer(state, pointer) {
  if (pointer.isNull) return;
  if (state.mode === 'com') {
    DynComRawOwnedComPointer.assumeTransferred(pointer).release();
  } else if (pointerCleanups[state.mode]) {
    pointerCleanups[state.mode](pointer);
  }
}

function finishOutput(state) {
  if (state.phase !== 'prepared') throw new Error(`Unsafe output is ${state.phase}; expected prepared`);
  const pointer = state.slot.readPointer(0);
  if (pointer.isNull && !state.nullable) throw new Error('Required pointer output was null');
  if (pointer.isNull) {
    takeOutputPointer(state);
    state.phase = 'finished';
    return null;
  }
  if (state.mode === 'com') {
    const managed = DynComUnsafe.borrowComPointer(pointer.address, state.iid);
    try {
      const extracted = takeOutputPointer(state);
      DynComRawOwnedComPointer.assumeTransferred(extracted).release();
      state.phase = 'finished';
      return managed;
    } catch (error) {
      managed.release();
      throw error;
    }
  }
  const extracted = takeOutputPointer(state);
  state.phase = 'finished';
  if (pointerCleanups[state.mode]) {
    return new UnsafeOwnedPointer(tokens.owned, extracted, pointerCleanups[state.mode]);
  }
  return extracted;
}

function finishReplacement(state) {
  if (state.phase !== 'prepared') throw new Error(`Interface replacement is ${state.phase}; expected prepared`);
  const pointer = state.slot.readPointer(0);
  if (state.mode === 'unchanged' && pointer.address !== state.oldAddress) {
    throw new Error('Interface replacement changed an unchanged-contract slot');
  }
  if (!pointer.isNull && pointer.address !== state.oldAddress) {
    const verified = DynComUnsafe.borrowComPointer(pointer.address, state.iid);
    verified.release();
  }
  let previous = null;
  if (state.mode !== 'consumes') {
    if (state.invocationOwner === null) {
      throw new Error('Interface replacement invocation owner is missing');
    }
    if (state.owner.released) {
      previous = state.invocationOwner;
    } else {
      state.invocationOwner.release();
      previous = state.owner;
    }
    state.invocationOwner = null;
  }
  const extracted = takeOutputPointer(state);
  state.phase = 'finished';
  if (state.mode !== 'consumes' && extracted.address === state.oldAddress) {
    return new UnsafeInterfaceReplacementResult(tokens.replacementResult, previous, previous);
  }
  const current = extracted.isNull ? null : DynComRawOwnedComPointer.assumeTransferred(extracted);
  return new UnsafeInterfaceReplacementResult(
    tokens.replacementResult,
    previous,
    current,
  );
}

function __finishStrategy(record) {
  const prepared = preparedFor(record);
  if (prepared.kind === 'UnsafePointerOutput' || prepared.kind === 'UnsafeHandleOutput') {
    return finishOutput(prepared.state);
  }
  if (prepared.kind === 'UnsafeInterfaceReplacement') return finishReplacement(prepared.state);
  throw new TypeError(`${prepared.kind} does not produce an output result`);
}

function preserveFailurePointer(state, pointer) {
  state.failurePointer = pointer;
  return Object.freeze({ strategy: state.mode, pointer });
}

function failOutput(state) {
  if (state.phase !== 'prepared') return null;
  const pointer = takeOutputPointer(state);
  if (state.mode === 'raw' || state.mode === 'unclassified' || state.mode === 'borrowed') {
    state.phase = 'failed';
    return preserveFailurePointer(state, pointer);
  }
  try {
    cleanupOutputPointer(state, pointer);
    state.phase = 'failed';
  } catch (error) {
    state.failurePointer = pointer;
    state.phase = 'cleanup-failed';
    throw error;
  }
  return null;
}

function failReplacement(state) {
  if (state.phase !== 'prepared') return null;
  if (!state.dispatchEntered) {
    rollbackActivation({ kind: 'UnsafeInterfaceReplacement', state });
    return null;
  }
  try {
    const pointer = takeOutputPointer(state);
    state.phase = 'failed';
    if (!pointer.isNull && pointer.address !== state.oldAddress) {
      DynComRawOwnedComPointer.assumeTransferred(pointer).release();
    }
  } finally {
    if (state.invocationOwner !== null) {
      state.invocationOwner.release();
      state.invocationOwner = null;
    }
  }
  return null;
}

function __failStrategy(record) {
  const prepared = preparedFor(record);
  if (prepared.kind === 'UnsafePointerOutput' || prepared.kind === 'UnsafeHandleOutput') {
    return failOutput(prepared.state);
  }
  if (prepared.kind === 'UnsafeInterfaceReplacement') return failReplacement(prepared.state);
  return null;
}

function takeFailurePointer(strategy) {
  let state = outputStateFor(strategy);
  if (!state && Object.getPrototypeOf(strategy) === UnsafeInterfaceReplacement.prototype) {
    state = stateFor(strategy, UnsafeInterfaceReplacement, replacementState, 'interface replacement');
  }
  if (!state || !['failed', 'cleanup-failed'].includes(state.phase) || state.failurePointer === undefined) {
    throw new Error('No raw failure pointer is available');
  }
  const pointer = state.failurePointer;
  state.failurePointer = undefined;
  state.phase = 'failure-taken';
  return pointer;
}

function __releaseExtracted(value) {
  if (value == null) return null;
  if (Object.getPrototypeOf(value) === UnsafeOwnedPointer.prototype && ownedState.has(value)) {
    value.release();
    return null;
  }
  if (Object.getPrototypeOf(value) === UnsafeInterfaceReplacementResult.prototype && replacementResultState.has(value)) {
    value.release();
    return null;
  }
  if (value instanceof DynComRawPointer) return Object.freeze({ strategy: 'raw', pointer: value });
  if (typeof value.release === 'function') {
    value.release();
    return null;
  }
  return null;
}

function __attachUnsafeOutputs(error, outputs) {
  if (outputs.length === 0) return error;
  Object.defineProperty(error, 'unsafeOutputs', {
    configurable: false,
    enumerable: true,
    writable: false,
    value: Object.freeze(outputs.slice()),
  });
  return error;
}

for (const constructor of [
  UnsafePointee,
  UnsafeRawCall,
  UnsafeCountedBuffer,
  UnsafeOwnedPointer,
  UnsafePointerOutput,
  UnsafeHandleOutput,
  UnsafeInterfaceReplacement,
  UnsafeInterfaceReplacementResult,
]) {
  Object.freeze(constructor.prototype);
  Object.freeze(constructor);
}

exports.UnsafePointee = UnsafePointee;
exports.UnsafeRawCall = UnsafeRawCall;
exports.UnsafeCountedBuffer = UnsafeCountedBuffer;
exports.UnsafeOwnedPointer = UnsafeOwnedPointer;
exports.UnsafePointerOutput = UnsafePointerOutput;
exports.UnsafeHandleOutput = UnsafeHandleOutput;
exports.UnsafeInterfaceReplacement = UnsafeInterfaceReplacement;
exports.UnsafeInterfaceReplacementResult = UnsafeInterfaceReplacementResult;
Object.defineProperties(exports, {
  __prepareStrategy: { value: __prepareStrategy },
  __prepareWritableStorage: { value: __prepareWritableStorage },
  __prepareExactWritableSpan: { value: __prepareExactWritableSpan },
  __strategyArgument: { value: __strategyArgument },
  __assertRawContract: { value: __assertRawContract },
  __validateStrategySpans: { value: __validateStrategySpans },
  __activateStrategies: { value: __activateStrategies },
  __markDispatchEntered: { value: __markDispatchEntered },
  __finishStrategy: { value: __finishStrategy },
  __failStrategy: { value: __failStrategy },
  __releaseExtracted: { value: __releaseExtracted },
  __attachUnsafeOutputs: { value: __attachUnsafeOutputs },
});
/*__TEST_EXPORTS__*/
"#
    .replace("__DYNWINRT_RAW_RUNTIME_IMPORT__", &raw_runtime_import);
    let test_exports =
        if std::env::var("DYNWINRT_CODEGEN_TEST_STRATEGY_RUNTIME").as_deref() == Ok("1") {
            r#"Object.defineProperties(exports, {
  __testRunOwnedFinalizer: {
    value(value) {
      const state = stateFor(value, UnsafeOwnedPointer, ownedState, 'owned pointer');
      return finalizeOwnedState(state);
    },
  },
  __testCreateOwnedPointer: {
    value(cleanup) {
      if (typeof cleanup !== 'function') throw new TypeError('cleanup must be a function');
      return new UnsafeOwnedPointer(tokens.owned, DynComRawPointer.null(), cleanup);
    },
  },
});"#
        } else {
            ""
        };
    js = js.replace("/*__TEST_EXPORTS__*/", test_exports);
    let dts = r#"// Generated by dynwinrt-codegen — do not edit
import type {
  DynComRawMemory,
  DynComRawOwnedComPointer,
  DynComRawPointer,
  DynWinRtValue,
  WinGuid,
} from __DYNWINRT_RAW_RUNTIME_IMPORT__;

export type UnsafeRawArgument = DynWinRtValue | DynComRawMemory | DynComRawPointer;
export type UnsafePointerResult = DynComRawPointer | DynWinRtValue | UnsafeOwnedPointer | null;
export type UnsafeHandleResult = DynComRawPointer | UnsafeOwnedPointer | null;

export declare class UnsafePointee {
  private constructor();
  static required(value: DynComRawMemory | DynComRawPointer): UnsafePointee;
  static nullable(value?: DynComRawMemory | DynComRawPointer | null): UnsafePointee;
}

export declare class UnsafeRawCall {
  private constructor();
  static value(value: UnsafeRawArgument): UnsafeRawCall;
  static acknowledge(): UnsafeRawCall;
}

export declare class UnsafeCountedBuffer {
  private constructor();
  static required(value: DynComRawMemory | DynComRawPointer): UnsafeCountedBuffer;
  static nullable(value?: DynComRawMemory | DynComRawPointer | null): UnsafeCountedBuffer;
}

export declare class UnsafeOwnedPointer {
  private constructor();
  readonly pointer: DynComRawPointer;
  readonly released: boolean;
  view(size: bigint | number, alignment: bigint | number): DynComRawMemory;
  release(): void;
}

export declare class UnsafePointerOutput<T = UnsafePointerResult> {
  private constructor();
  private readonly __resultType: (value: T) => T;
  static unclassified(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<DynComRawPointer | null>;
  static borrowed(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<DynComRawPointer | null>;
  static comOwned(iid: WinGuid, slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<DynWinRtValue | null>;
  static coTaskMem(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<UnsafeOwnedPointer | null>;
  static bstr(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<UnsafeOwnedPointer | null>;
  static localAlloc(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<UnsafeOwnedPointer | null>;
  static globalAlloc(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<UnsafeOwnedPointer | null>;
  static rawResponsibility(slot?: DynComRawMemory | null, nullable?: boolean): UnsafePointerOutput<DynComRawPointer | null>;
  takeFailurePointer(): DynComRawPointer;
}

export declare class UnsafeHandleOutput<T = UnsafeHandleResult> {
  private constructor();
  private readonly __resultType: (value: T) => T;
  static borrowed(slot?: DynComRawMemory | null, nullable?: boolean): UnsafeHandleOutput<DynComRawPointer | null>;
  static closeHandle(slot?: DynComRawMemory | null, nullable?: boolean): UnsafeHandleOutput<UnsafeOwnedPointer | null>;
  static destroyIcon(slot?: DynComRawMemory | null, nullable?: boolean): UnsafeHandleOutput<UnsafeOwnedPointer | null>;
  static deleteObject(slot?: DynComRawMemory | null, nullable?: boolean): UnsafeHandleOutput<UnsafeOwnedPointer | null>;
  static rawResponsibility(slot?: DynComRawMemory | null, nullable?: boolean): UnsafeHandleOutput<DynComRawPointer | null>;
  takeFailurePointer(): DynComRawPointer;
}

export declare class UnsafeInterfaceReplacementResult {
  private constructor();
  readonly previous: DynComRawOwnedComPointer | null;
  readonly current: DynComRawOwnedComPointer | null;
  release(): void;
}

export declare class UnsafeInterfaceReplacement {
  private constructor();
  static consumesOld(owner: DynComRawOwnedComPointer, iid: WinGuid, slot?: DynComRawMemory | null): UnsafeInterfaceReplacement;
  static preservesOld(owner: DynComRawOwnedComPointer, iid: WinGuid, slot?: DynComRawMemory | null): UnsafeInterfaceReplacement;
  static unchanged(owner: DynComRawOwnedComPointer, iid: WinGuid, slot?: DynComRawMemory | null): UnsafeInterfaceReplacement;
}
"#
    .replace("__DYNWINRT_RAW_RUNTIME_IMPORT__", &raw_runtime_import);
    vec![
        ("unsafe/runtime.js".into(), js),
        ("unsafe/runtime.d.ts".into(), dts.into()),
    ]
}
