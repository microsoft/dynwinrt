// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod interop;
mod legacy_diagnostics;
mod legacy_types;

use crate::com_metadata::{ComCoclassMeta, ComInterfaceMeta};

use super::ir::{
    ActivationPlan, BufferCountUnit as ProjectedBufferCountUnit, ComEnumUnderlying,
    ComParamDirection, ComPrimitive, ComReturnConvention, ComScalarRepr, ComType, DispatchShape,
    NativePodArchitectureLayout, NativePodField, NativePodFieldType, NativePodInitializer,
    NativePodLayout, NativePodScalar, NativeUnionArchitectureLayout, NativeUnionField,
    NativeUnionFieldType, NativeUnionLayout, OverloadDispatch, OverloadInfo, PointerAliasKind,
    ProjectedComCoclass, ProjectedComEnum, ProjectedComEnumMember, ProjectedComInterface,
    ProjectedComMethod, ProjectedComMethodKind, ProjectedComParam, ProjectedComResult,
    ProjectedEnumValue, ProjectedInterfaceRef, ResultConversion, ResultSource, SafeArrayElement,
    SharedCountPlan, StringBufferPlan, StringEncoding, TypedBufferPlan, TypedBufferRelation,
    TypedBufferSizing, dispatch_shape,
};
use super::javascript::naming::camel_case;
use super::model::ValidatedComInterface;
use super::model::abi::{
    BufferElement, BufferElementOwnership, ComAbiType, ComEnumValue as SemanticEnumValue,
    ComTypeDefinition, Constness, ScalarType, StringEncoding as SemanticStringEncoding,
};
use super::model::contract::{
    BufferSizing, ComParamContract, CountRelation, CountUnit, Direction, Nullability,
};
use super::model::layout::Architecture;
use super::model::method::{ComMethodContract, ComMethodSpecialContract, ComReturnKind};
use super::model::ownership::{Cleanup, ComOwnership};
use super::model::{self, SemanticComInterface};
use interop::resolve_projected_default_iid;

#[cfg(test)]
pub(super) use legacy_diagnostics::project_com_interface_for_test;
#[cfg(test)]
pub(super) use legacy_types::project_type as project_type_for_test;

pub(super) fn project_com_interface(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<ProjectedComInterface, String> {
    let validated = model::validate_interface(meta)
        .map_err(|error| diagnostic_compatibility(meta, winmd_paths, error))?;
    project_validated_interface(&validated, winmd_paths)
        .map_err(|error| diagnostic_compatibility(meta, winmd_paths, error))
}

pub(super) fn project_com_reference_interface(
    meta: &ComInterfaceMeta,
) -> Result<ProjectedComInterface, String> {
    if meta.interface.iid.is_empty() {
        return Err(format!(
            "{}.{} has no exact IID for a managed array element wrapper",
            meta.interface.namespace, meta.interface.name
        ));
    }
    let projected = ProjectedComInterface {
        name: meta.interface.name.clone(),
        namespace: meta.interface.namespace.clone(),
        iid: meta.interface.iid.clone(),
        is_iunknown_rooted: meta.is_iunknown_rooted,
        methods: Vec::new(),
        activation: ActivationPlan::None,
        referenced_enums: Vec::new(),
    };
    validate_projected_surface_names(&projected)?;
    Ok(projected)
}

fn project_validated_interface(
    validated: &ValidatedComInterface<'_>,
    winmd_paths: &str,
) -> Result<ProjectedComInterface, String> {
    let meta = validated.metadata();
    let semantic = validated.semantic();
    let interop_target = detect_interop_target(validated, winmd_paths)?;

    let mut method_indices = (0..semantic.methods().len()).collect::<Vec<_>>();
    method_indices.sort_by_key(|index| semantic.methods()[*index].vtable_slot());
    let methods = method_indices
        .into_iter()
        .map(|index| {
            project_method(
                semantic,
                &semantic.methods()[index],
                meta.interface.methods[index].doc.clone(),
                interop_target.as_ref(),
                &meta.interface.namespace,
                &meta.interface.name,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let methods = group_overloads(methods, &meta.interface.name)?;

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

    let referenced_enums = semantic
        .referenced_enums()
        .map(|definition| {
            Ok(ProjectedComEnum {
                namespace: definition.native_name().namespace().to_string(),
                name: definition.native_name().name().to_string(),
                underlying: project_enum_underlying(definition.underlying())?,
                members: definition
                    .members()
                    .iter()
                    .map(|member| ProjectedComEnumMember {
                        name: member.name().to_string(),
                        value: match member.value() {
                            SemanticEnumValue::Signed(value) => ProjectedEnumValue::Signed(*value),
                            SemanticEnumValue::Unsigned(value) => {
                                ProjectedEnumValue::Unsigned(*value)
                            }
                        },
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let projected = ProjectedComInterface {
        name: meta.interface.name.clone(),
        namespace: meta.interface.namespace.clone(),
        iid: format_guid(semantic.iid().as_bytes()),
        is_iunknown_rooted: semantic.is_iunknown_rooted(),
        methods,
        activation,
        referenced_enums,
    };
    validate_projected_surface_names(&projected)?;
    Ok(projected)
}

fn validate_projected_surface_names(meta: &ProjectedComInterface) -> Result<(), String> {
    let mut names = std::collections::BTreeMap::<String, String>::new();
    insert_surface_name(
        &mut names,
        &meta.name,
        format!("interface {}.{}", meta.namespace, meta.name),
    )?;
    for definition in &meta.referenced_enums {
        insert_surface_name(
            &mut names,
            &definition.name,
            format!("enum {}.{}", definition.namespace, definition.name),
        )?;
    }
    for method in &meta.methods {
        for typ in method
            .params
            .iter()
            .map(|param| &param.typ)
            .chain(method.results.iter().map(|result| &result.typ))
            .chain(match &method.return_convention {
                ComReturnConvention::Direct(typ) => Some(typ),
                _ => None,
            })
        {
            match typ {
                ComType::NativePod { layout } | ComType::NativePodPointer { layout } => {
                    insert_surface_name(
                        &mut names,
                        &layout.name,
                        format!("native POD {}.{}", layout.namespace, layout.name),
                    )?;
                }
                ComType::NativeUnionPointer { layout } => {
                    insert_surface_name(
                        &mut names,
                        &layout.name,
                        format!("native union {}.{}", layout.namespace, layout.name),
                    )?;
                }
                ComType::ScalarAlias {
                    namespace, name, ..
                } => {
                    insert_surface_name(
                        &mut names,
                        name,
                        format!("scalar alias {namespace}.{name}"),
                    )?;
                }
                ComType::PointerAlias {
                    namespace, name, ..
                } => {
                    insert_surface_name(
                        &mut names,
                        name,
                        format!("pointer alias {namespace}.{name}"),
                    )?;
                }
                ComType::Enum {
                    namespace, name, ..
                } => {
                    insert_surface_name(&mut names, name, format!("enum {namespace}.{name}"))?;
                }
                ComType::TypedBuffer { element } => {
                    if let ComType::NativePod { layout } = element.as_ref() {
                        insert_surface_name(
                            &mut names,
                            &layout.name,
                            format!("native POD {}.{}", layout.namespace, layout.name),
                        )?;
                    }
                }
                ComType::OwningArray { .. } => {}
                ComType::Primitive(_)
                | ComType::NativeIsize
                | ComType::NativeUsize
                | ComType::Win32Bool
                | ComType::HResult
                | ComType::Guid
                | ComType::HString
                | ComType::RawPointer
                | ComType::AllocatorPointer
                | ComType::ConsumedAllocatorPointer
                | ComType::InspectedAllocatorPointer
                | ComType::GuidPointer
                | ComType::Bstr
                | ComType::Variant
                | ComType::VariantByValue
                | ComType::SafeArray { .. }
                | ComType::PropVariant
                | ComType::DispatchParams
                | ComType::ExcepInfo
                | ComType::StatStg
                | ComType::ManagedInterface { .. }
                | ComType::CoTaskMemWideString
                | ComType::StringArray { .. } => {}
            }
        }
    }
    Ok(())
}

fn insert_surface_name(
    names: &mut std::collections::BTreeMap<String, String>,
    name: &str,
    identity: String,
) -> Result<(), String> {
    if let Some(existing) = names.insert(name.to_string(), identity.clone())
        && existing != identity
    {
        return Err(format!(
            "projected module identifier `{name}` is ambiguous between `{existing}` and `{identity}`"
        ));
    }
    Ok(())
}

fn diagnostic_compatibility(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
    semantic_error: String,
) -> String {
    if semantic_error.contains("BIND_OPTS requires cbStruct")
        || semantic_error.contains("Automation BYREF/InOut")
        || semantic_error.contains("native union")
        || semantic_error.contains("SAFEARRAY")
        || semantic_error.contains("VARIANT")
        || semantic_error.contains("DISPPARAMS")
        || semantic_error.contains("EXCEPINFO")
        || semantic_error.contains("STATSTG")
        || semantic_error.contains("IDispatch::Invoke")
        || semantic_error.contains("optional COM outputs")
        || semantic_error.contains("nested interface")
        || semantic_error.contains("nested string/interface")
        || semantic_error.contains("exact contract evidence")
        || semantic_error.contains("AddRef'd interface pointer")
        || semantic_error.contains("dynamic-IID")
        || semantic_error.contains("competing IID")
        || semantic_error.contains("competing void**")
    {
        return semantic_error;
    }
    match legacy_diagnostics::diagnostic_preflight(meta, winmd_paths) {
        Ok(()) => semantic_error,
        Err(legacy_error) => legacy_error,
    }
}

pub(super) fn project_com_coclass(
    meta: &ComCoclassMeta,
    winmd_paths: &str,
) -> Result<ProjectedComCoclass, String> {
    let validated_coclass = model::validate_coclass(meta)?;
    let meta = validated_coclass.metadata();
    let associated_interfaces = meta
        .associated_interfaces
        .iter()
        .map(|interface| project_com_interface(interface, winmd_paths))
        .collect::<Result<Vec<_>, _>>()?;
    let primary_interface = associated_interfaces
        .iter()
        .find(|interface| {
            interface
                .iid
                .eq_ignore_ascii_case(&meta.primary_interface.interface.iid)
        })
        .cloned()
        .ok_or_else(|| {
            format!(
                "{}.{} primary interface {} was not projected",
                meta.namespace, meta.name, meta.primary_interface.interface.name
            )
        })?;
    for method in &primary_interface.methods {
        let public_name = method
            .overload
            .as_ref()
            .map_or(method.camel_name.as_str(), |overload| {
                overload.public_name.as_str()
            });
        if matches!(public_name, "as" | "tryAs" | "supports") {
            return Err(format!(
                "{}.{} primary interface method `{public_name}` conflicts with the coclass interface-view API",
                meta.namespace, meta.name
            ));
        }
    }
    validate_coclass_enum_files(&meta.namespace, &meta.name, &associated_interfaces)?;

    Ok(ProjectedComCoclass {
        name: meta.name.clone(),
        namespace: meta.namespace.clone(),
        clsid: format_guid(validated_coclass.clsid().as_bytes()),
        primary_interface,
        associated_interfaces,
    })
}

fn validate_coclass_enum_files(
    namespace: &str,
    name: &str,
    interfaces: &[ProjectedComInterface],
) -> Result<(), String> {
    let mut enum_files = std::collections::BTreeMap::new();
    for interface in interfaces {
        for definition in &interface.referenced_enums {
            if let Some(existing) = enum_files.insert(definition.name.clone(), definition.clone())
                && existing != *definition
            {
                return Err(format!(
                    "{namespace}.{name} associated interfaces define conflicting `{}` enum files",
                    definition.name
                ));
            }
        }
    }
    Ok(())
}

fn project_method(
    semantic: &SemanticComInterface,
    method: &ComMethodContract,
    doc: Option<String>,
    interop_target: Option<&(String, String, String)>,
    interface_namespace: &str,
    interface_name: &str,
) -> Result<ProjectedComMethod, String> {
    let context = || method.name().to_string();
    let dynamic_iid = method.dynamic_iid_contract();
    let mut kind = if let Some(ComMethodSpecialContract::FixedCapacityBytes { guid_param }) =
        method.special_contract()
    {
        if dynamic_iid.is_some() {
            return Err(format!(
                "{}: fixed-capacity bytes cannot also use a dynamic IID output",
                context()
            ));
        }
        ProjectedComMethodKind::FixedCapacityBytes {
            guid_param_index: guid_param.index(),
        }
    } else {
        project_dynamic_method_kind(
            method.name(),
            dynamic_iid.map(|contract| {
                (
                    contract.iid_param_index().index(),
                    contract.output_param_index().index(),
                )
            }),
            interop_target.map(|(_, _, target_iid)| target_iid.as_str()),
        )
    };
    let is_idispatch_invoke = interface_namespace == "Windows.Win32.System.Com"
        && interface_name == "IDispatch"
        && method.name() == "Invoke";

    let string_buffer = project_string_buffer(semantic, method)?;
    let typed_buffers = project_typed_buffers(semantic, method)?;
    let shared_counts = project_shared_counts(method, &typed_buffers)?;
    let enumerator_buffers = typed_buffers
        .iter()
        .filter(|plan| matches!(plan.relation, TypedBufferRelation::EnumeratorNext { .. }))
        .collect::<Vec<_>>();
    if method.return_kind() == ComReturnKind::EnumeratorNextHResult {
        let [plan] = enumerator_buffers.as_slice() else {
            return Err(format!(
                "{}: EnumeratorNext requires exactly one partial caller-output plan",
                context()
            ));
        };
        let TypedBufferRelation::EnumeratorNext {
            capacity_param_index,
            fetched_param_index,
            fetched_optional_for_single,
        } = plan.relation
        else {
            unreachable!()
        };
        kind = ProjectedComMethodKind::EnumeratorNext {
            buffer_param_index: plan.buffer_param_index,
            capacity_param_index,
            fetched_param_index,
            fetched_optional_for_single,
            interface: project_enumerator_interface_ref(semantic, method, plan.buffer_param_index)?,
        };
    } else if !enumerator_buffers.is_empty() {
        return Err(format!(
            "{}: EnumeratorNext buffer requires its preserved HRESULT convention",
            context()
        ));
    } else {
        let owning_outputs = typed_buffers
            .iter()
            .filter(|plan| {
                matches!(
                    plan.relation,
                    TypedBufferRelation::CallerOutput {
                        sizing: TypedBufferSizing::SingleCall,
                        ..
                    }
                ) && is_owning_array_output_element(&plan.element)
                    && !shared_counts.iter().any(|group| {
                        matches!(
                            group,
                            SharedCountPlan::Parallel {
                                output_param_indices,
                                ..
                            } if output_param_indices.contains(&plan.buffer_param_index)
                        )
                    })
            })
            .collect::<Vec<_>>();
        if let [plan] = owning_outputs.as_slice() {
            let TypedBufferRelation::CallerOutput {
                capacity_param_index,
                ..
            } = plan.relation
            else {
                unreachable!()
            };
            kind = ProjectedComMethodKind::OwningCallerOutput {
                buffer_param_index: plan.buffer_param_index,
                capacity_param_index,
            };
        } else if owning_outputs.len() > 1 {
            return Err(format!(
                "{}: multiple owning caller-output arrays require an explicit compound projection",
                context()
            ));
        }
    }
    if matches!(kind, ProjectedComMethodKind::OwningCallerOutput { .. })
        && !shared_counts.is_empty()
    {
        return Err(format!(
            "{}: owning caller-output arrays cannot be combined with independent shared buffer groups",
            context()
        ));
    }

    validate_buffer_plan_composition(method.name(), string_buffer.as_ref(), &typed_buffers)?;
    validate_dynamic_iid_buffers(
        &context(),
        dynamic_iid.is_some(),
        string_buffer.as_ref(),
        &typed_buffers,
    )?;

    let malloc_contract = matches!(
        method.special_contract(),
        Some(ComMethodSpecialContract::Malloc)
    );
    let mut params = Vec::with_capacity(method.params().len());
    for (index, param) in method.params().iter().enumerate() {
        let mut typ = project_param_type(semantic, param)?;
        if malloc_contract && matches!(typ, ComType::RawPointer) {
            typ = match method.name() {
                "Free" => ComType::ConsumedAllocatorPointer,
                "DidAlloc" => ComType::InspectedAllocatorPointer,
                _ => ComType::AllocatorPointer,
            };
        }
        let typed_buffer = typed_buffers
            .iter()
            .find(|plan| plan.buffer_param_index == index);
        let direction = if let Some(plan) = typed_buffer {
            match plan.relation {
                TypedBufferRelation::Input { .. } => ComParamDirection::InputBuffer,
                TypedBufferRelation::CallerOutput { .. }
                | TypedBufferRelation::EnumeratorNext { .. } => {
                    ComParamDirection::CallerOutputBuffer
                }
                TypedBufferRelation::CalleeAllocated { .. } => {
                    ComParamDirection::CalleeAllocatedBuffer
                }
            }
        } else if string_buffer
            .as_ref()
            .is_some_and(|plan| plan.buffer_param_index == index)
        {
            ComParamDirection::OutStringBuffer
        } else {
            match param.direction() {
                Direction::In => ComParamDirection::In,
                Direction::Out
                    if param.optional()
                        && (is_idispatch_invoke
                            || matches!(
                                &kind,
                                ProjectedComMethodKind::EnumeratorNext {
                                    fetched_param_index,
                                    ..
                                } if *fetched_param_index == index
                            )) =>
                {
                    ComParamDirection::OptionalOut
                }
                Direction::Out => ComParamDirection::Out,
                Direction::InOut => ComParamDirection::InOut,
            }
        };
        let mut surface_input = direction.is_input();
        let mut surface_result = matches!(
            direction,
            ComParamDirection::Out
                | ComParamDirection::OptionalOut
                | ComParamDirection::InOut
                | ComParamDirection::CallerOutputBuffer
                | ComParamDirection::CalleeAllocatedBuffer
        );
        for plan in &typed_buffers {
            match plan.relation {
                TypedBufferRelation::Input {
                    count_param_index, ..
                } if index == count_param_index => surface_input = false,
                TypedBufferRelation::CallerOutput {
                    capacity_param_index,
                    actual_length_param_index,
                    sizing,
                    ..
                } => {
                    if index == capacity_param_index {
                        surface_input = sizing == TypedBufferSizing::FixedCapacity
                            || matches!(
                                kind,
                                ProjectedComMethodKind::OwningCallerOutput {
                                    capacity_param_index: owning_capacity,
                                    ..
                                } if owning_capacity == index
                            );
                        surface_result = false;
                    }
                    if actual_length_param_index == Some(index) {
                        let owning_capacity_actual = capacity_param_index == index
                            && matches!(
                                kind,
                                ProjectedComMethodKind::OwningCallerOutput {
                                    capacity_param_index: owning_capacity,
                                    ..
                                } if owning_capacity == index
                            );
                        if !owning_capacity_actual {
                            surface_input = false;
                        }
                        surface_result = false;
                    }
                }
                TypedBufferRelation::EnumeratorNext {
                    capacity_param_index,
                    fetched_param_index,
                    ..
                } => {
                    if index == capacity_param_index {
                        surface_input = true;
                        surface_result = false;
                    }
                    if index == fetched_param_index {
                        surface_input = false;
                        surface_result = false;
                    }
                    if typed_buffer.is_some() {
                        surface_input = false;
                    }
                }
                TypedBufferRelation::CalleeAllocated {
                    count_param_index, ..
                } if index == count_param_index => {
                    surface_input = false;
                    surface_result = false;
                }
                TypedBufferRelation::Input { .. } | TypedBufferRelation::CalleeAllocated { .. } => {
                }
            }
        }
        if typed_buffer.is_some_and(|plan| {
            matches!(
                plan.relation,
                TypedBufferRelation::CallerOutput {
                    sizing: TypedBufferSizing::FixedCapacity | TypedBufferSizing::TwoCall { .. },
                    ..
                }
            )
        }) {
            surface_input = false;
        }
        if matches!(
            kind,
            ProjectedComMethodKind::OwningCallerOutput {
                buffer_param_index,
                ..
            } if buffer_param_index == index
        ) {
            surface_input = false;
        }
        if shared_counts.iter().any(|group| match group {
            SharedCountPlan::StringInputScalarOutput {
                scalar_output_param_index,
                ..
            } => *scalar_output_param_index == index,
            SharedCountPlan::Parallel {
                output_param_indices,
                ..
            } => output_param_indices.contains(&index),
        }) {
            surface_input = false;
        }
        if matches!(
            &kind,
            ProjectedComMethodKind::SynthesizedGetForWindow {
                iid_param_index,
                ..
            } if *iid_param_index == index
        ) {
            surface_input = false;
        }
        if direction == ComParamDirection::InOut && !is_scalar_in_out(&typ) {
            return Err(format!(
                "{}: unsupported [in, out] parameter `{}`",
                context(),
                param.name()
            ));
        }
        let nullable = param.nullability() == Nullability::Nullable
            && (surface_input || (surface_result && matches!(typ, ComType::SafeArray { .. })));
        if nullable && !supports_nullable_projection(&typ) {
            return Err(format!(
                "{}: nullable parameter `{}` has no safe language/runtime lowering",
                context(),
                param.name()
            ));
        }
        params.push(ProjectedComParam {
            name: param.name().into(),
            typ,
            direction,
            surface_input,
            surface_result,
            nullable,
        });
    }
    if let ProjectedComMethodKind::EnumeratorNext {
        capacity_param_index,
        ..
    } = &kind
    {
        params[*capacity_param_index].name = "count".into();
    }
    for plan in &typed_buffers {
        if let TypedBufferRelation::CallerOutput {
            capacity_param_index,
            sizing: TypedBufferSizing::FixedCapacity,
            ..
        } = plan.relation
        {
            params[capacity_param_index].name = "capacity".into();
        }
    }
    if let ProjectedComMethodKind::OwningCallerOutput {
        capacity_param_index,
        ..
    } = kind
    {
        params[capacity_param_index].name = "capacity".into();
    }

    let optional_outputs = params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            (param.direction == ComParamDirection::OptionalOut).then_some(index)
        })
        .collect::<Vec<_>>();
    if !optional_outputs.is_empty()
        && !matches!(kind, ProjectedComMethodKind::EnumeratorNext { .. })
    {
        if interface_namespace != "Windows.Win32.System.Com"
            || interface_name != "IDispatch"
            || method.name() != "Invoke"
            || params.len() != 8
            || optional_outputs != [5, 6, 7]
            || params[..5]
                .iter()
                .any(|param| param.direction != ComParamDirection::In)
            || !matches!(
                params[0].typ,
                ComType::Primitive(ComPrimitive::I32)
                    | ComType::ScalarAlias {
                        underlying: ComScalarRepr::Primitive(ComPrimitive::I32),
                        ..
                    }
            )
            || !matches!(params[1].typ, ComType::GuidPointer)
            || !matches!(
                params[2].typ,
                ComType::Primitive(ComPrimitive::U32)
                    | ComType::ScalarAlias {
                        underlying: ComScalarRepr::Primitive(ComPrimitive::U32),
                        ..
                    }
            )
            || !matches!(
                params[3].typ,
                ComType::Primitive(ComPrimitive::U16)
                    | ComType::Enum {
                        underlying: ComEnumUnderlying::U16,
                        ..
                    }
                    | ComType::ScalarAlias {
                        underlying: ComScalarRepr::Primitive(ComPrimitive::U16),
                        ..
                    }
            )
            || params[4].direction != ComParamDirection::In
            || !matches!(params[4].typ, ComType::DispatchParams)
            || !matches!(params[5].typ, ComType::Variant)
            || !matches!(params[6].typ, ComType::ExcepInfo)
            || !matches!(
                params[7].typ,
                ComType::Primitive(ComPrimitive::U32)
                    | ComType::ScalarAlias {
                        underlying: ComScalarRepr::Primitive(ComPrimitive::U32),
                        ..
                    }
            )
        {
            return Err(format!(
                "{}: optional COM outputs require the exact validated IDispatch::Invoke contract",
                context()
            ));
        }
        kind = ProjectedComMethodKind::DispatchInvoke {
            result_param_index: 5,
            excep_info_param_index: 6,
            arg_err_param_index: 7,
        };
        params[4].name = "dispParams".into();
        params[6].surface_result = false;
        params[7].surface_result = false;
    }

    fn supports_nullable_projection(typ: &ComType) -> bool {
        matches!(
            typ,
            ComType::RawPointer
                | ComType::AllocatorPointer
                | ComType::ConsumedAllocatorPointer
                | ComType::InspectedAllocatorPointer
                | ComType::GuidPointer
                | ComType::PointerAlias { .. }
                | ComType::NativePodPointer { .. }
                | ComType::Bstr
                | ComType::ManagedInterface { .. }
                | ComType::SafeArray { .. }
        )
    }

    let mut return_convention = match method.return_kind() {
        ComReturnKind::HResult => ComReturnConvention::HResult,
        ComReturnKind::SemanticHResult => ComReturnConvention::SemanticHResult,
        ComReturnKind::EnumeratorNextHResult => ComReturnConvention::SemanticHResult,
        ComReturnKind::Void => ComReturnConvention::Void,
        ComReturnKind::DirectValue(abi_type) => {
            let typ = project_value_type(semantic, abi_type)?;
            if !is_supported_direct_return(&typ) {
                return Err(format!(
                    "{}: unsupported direct native return type {}",
                    context(),
                    semantic_type_name(semantic, abi_type)
                ));
            }
            ComReturnConvention::Direct(typ)
        }
        ComReturnKind::DirectPointer(abi_type) => {
            let typ = project_value_type(semantic, abi_type)?;
            if !malloc_contract || !matches!(typ, ComType::RawPointer) {
                return Err(format!(
                    "{}: unsupported direct native return type {}",
                    context(),
                    semantic_type_name(semantic, abi_type)
                ));
            }
            ComReturnConvention::Direct(ComType::AllocatorPointer)
        }
    };
    if matches!(kind, ProjectedComMethodKind::DispatchInvoke { .. })
        && return_convention != ComReturnConvention::HResult
    {
        return Err(format!(
            "{}: IDispatch::Invoke requires the HRESULT return convention",
            context()
        ));
    }
    if matches!(kind, ProjectedComMethodKind::DispatchInvoke { .. }) {
        return_convention = ComReturnConvention::DispatchInvokeHResult;
    }

    let mut results = Vec::new();
    if !matches!(kind, ProjectedComMethodKind::EnumeratorNext { .. })
        && let ComReturnConvention::SemanticHResult | ComReturnConvention::Direct(_) =
            &return_convention
    {
        let typ = match &return_convention {
            ComReturnConvention::SemanticHResult => ComType::HResult,
            ComReturnConvention::Direct(typ) => typ.clone(),
            ComReturnConvention::HResult
            | ComReturnConvention::DispatchInvokeHResult
            | ComReturnConvention::Void => unreachable!(),
        };
        results.push(ProjectedComResult {
            typ,
            source: ResultSource::DirectReturn,
            conversion: if malloc_contract && method.name() == "Alloc" {
                ResultConversion::MallocAllocation
            } else if malloc_contract && method.name() == "Realloc" {
                ResultConversion::MallocReallocation
            } else {
                ResultConversion::Value
            },
        });
    }
    for (index, (contract, param)) in method.params().iter().zip(&params).enumerate() {
        if param.surface_result {
            results.push(ProjectedComResult {
                typ: param.typ.clone(),
                source: ResultSource::Param(index),
                conversion: if matches!(
                    param.direction,
                    ComParamDirection::CallerOutputBuffer
                        | ComParamDirection::CalleeAllocatedBuffer
                ) {
                    if matches!(
                        typed_buffers
                            .iter()
                            .find(|plan| plan.buffer_param_index == index)
                            .map(|plan| &plan.relation),
                        Some(TypedBufferRelation::EnumeratorNext { .. })
                    ) {
                        let interface = match &kind {
                            ProjectedComMethodKind::EnumeratorNext { interface, .. } => {
                                interface.clone()
                            }
                            _ => unreachable!("EnumeratorNext buffer has EnumeratorNext kind"),
                        };
                        ResultConversion::EnumeratorArray { interface }
                    } else if is_owning_array_output_element(
                        &typed_buffers
                            .iter()
                            .find(|plan| plan.buffer_param_index == index)
                            .expect("caller output buffer plan")
                            .element,
                    ) {
                        ResultConversion::OwningArray {
                            interface: project_buffer_interface_ref(semantic, method, index)?,
                        }
                    } else if shared_counts.iter().any(|group| match group {
                        SharedCountPlan::StringInputScalarOutput {
                            scalar_output_param_index,
                            ..
                        } => *scalar_output_param_index == index,
                        SharedCountPlan::Parallel {
                            output_param_indices,
                            ..
                        } => output_param_indices.contains(&index),
                    }) {
                        ResultConversion::PlainArray
                    } else {
                        ResultConversion::Buffer
                    }
                } else {
                    result_conversion(
                        contract,
                        &param.typ,
                        dynamic_iid
                            .is_some_and(|dynamic| dynamic.output_param_index().index() == index),
                    )?
                },
            });
        }
    }

    Ok(ProjectedComMethod {
        name: method.name().into(),
        camel_name: camel_case(method.name()),
        vtable_index: method.vtable_slot(),
        params,
        return_convention,
        results,
        string_buffer,
        typed_buffers,
        shared_counts,
        kind,
        doc,
        overload: None,
    })
}

fn project_dynamic_method_kind(
    method_name: &str,
    dynamic_iid: Option<(usize, usize)>,
    interop_target_iid: Option<&str>,
) -> ProjectedComMethodKind {
    match (method_name, dynamic_iid, interop_target_iid) {
        ("GetForWindow", Some((iid_param_index, output_param_index)), Some(target_iid)) => {
            ProjectedComMethodKind::SynthesizedGetForWindow {
                iid_param_index,
                output_param_index,
                target_iid: target_iid.into(),
            }
        }
        (_, Some((iid_param_index, output_param_index)), _) => {
            ProjectedComMethodKind::CallerSuppliedDynamicIid {
                iid_param_index,
                output_param_index,
            }
        }
        _ => ProjectedComMethodKind::Normal,
    }
}

fn validate_buffer_plan_composition(
    method: &str,
    string_buffer: Option<&StringBufferPlan>,
    typed_buffers: &[TypedBufferPlan],
) -> Result<(), String> {
    if string_buffer.is_some() && !typed_buffers.is_empty() {
        return Err(format!(
            "{method}: mixed string and typed buffer plans are not supported"
        ));
    }
    let two_call_count = typed_buffers
        .iter()
        .filter(|plan| {
            matches!(
                plan.relation,
                TypedBufferRelation::CallerOutput {
                    sizing: TypedBufferSizing::TwoCall { .. },
                    ..
                }
            )
        })
        .count();
    if two_call_count > 0 && typed_buffers.len() != 1 {
        return Err(format!(
            "{method}: two-call sizing cannot be combined with another buffer plan"
        ));
    }
    Ok(())
}

fn validate_dynamic_iid_buffers(
    method: &str,
    has_dynamic_iid: bool,
    string_buffer: Option<&StringBufferPlan>,
    typed_buffers: &[TypedBufferPlan],
) -> Result<(), String> {
    if has_dynamic_iid && (string_buffer.is_some() || !typed_buffers.is_empty()) {
        return Err(format!(
            "{method}: dynamic-IID methods with hidden string/buffer parameters are not supported"
        ));
    }
    Ok(())
}

fn project_param_type(
    semantic: &SemanticComInterface,
    param: &ComParamContract,
) -> Result<ComType, String> {
    match param.direction() {
        Direction::In => project_input_type(semantic, param.abi_type()),
        Direction::InOut if param.nullability() == Nullability::Nullable => {
            let typ = project_input_type(semantic, param.abi_type())?;
            if matches!(typ, ComType::NativePodPointer { .. }) {
                Ok(typ)
            } else {
                Err(format!(
                    "nullable [in, out] parameter `{}` requires a native POD pointer contract",
                    param.name()
                ))
            }
        }
        Direction::Out | Direction::InOut => project_output_type(semantic, param.abi_type()),
    }
}

fn project_input_type(
    semantic: &SemanticComInterface,
    abi_type: super::model::ids::TypeId,
) -> Result<ComType, String> {
    let definition = semantic
        .type_definition(abi_type)
        .map_err(|error| error.to_string())?;
    match definition.abi() {
        ComAbiType::Pointer { pointee, depth, .. } if depth.get() == 1 => {
            let pointee_definition = semantic
                .type_definition(*pointee)
                .map_err(|error| error.to_string())?;
            if matches!(pointee_definition.abi(), ComAbiType::ComInterface { .. }) {
                project_value_type(semantic, *pointee)
            } else if matches!(pointee_definition.abi(), ComAbiType::Guid) {
                Ok(ComType::GuidPointer)
            } else if matches!(pointee_definition.abi(), ComAbiType::NativeStruct(_)) {
                let ComType::NativePod { layout } = project_value_type(semantic, *pointee)? else {
                    unreachable!("validated native struct projection")
                };
                Ok(ComType::NativePodPointer { layout })
            } else if matches!(pointee_definition.abi(), ComAbiType::NativeUnion(_)) {
                let ComAbiType::NativeUnion(layout_id) = pointee_definition.abi() else {
                    unreachable!()
                };
                let native_name = pointee_definition
                    .native_name()
                    .ok_or_else(|| "validated native union has no qualified name".to_string())?;
                Ok(ComType::NativeUnionPointer {
                    layout: project_native_union_layout(
                        semantic,
                        native_name.namespace(),
                        native_name.name(),
                        *layout_id,
                    )?,
                })
            } else if matches!(
                pointee_definition.abi(),
                ComAbiType::Variant
                    | ComAbiType::PropVariant
                    | ComAbiType::SafeArray { .. }
                    | ComAbiType::DispatchParams
            ) {
                project_value_type(semantic, *pointee)
            } else if matches!(pointee_definition.abi(), ComAbiType::ExcepInfo) {
                Err("EXCEPINFO is output-only".into())
            } else if matches!(pointee_definition.abi(), ComAbiType::StatStg) {
                Err("STATSTG is output-only".into())
            } else {
                Ok(ComType::RawPointer)
            }
        }
        ComAbiType::Pointer { .. } => Ok(ComType::RawPointer),
        ComAbiType::CountedBuffer { .. } => project_counted_buffer_type(semantic, definition),
        ComAbiType::Variant => Ok(ComType::VariantByValue),
        ComAbiType::PropVariant
        | ComAbiType::SafeArray { .. }
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg => Err(format!(
            "{} must be passed through its native pointer contract",
            semantic_type_definition_name(definition)
        )),
        ComAbiType::NativeUnion(_) => Err(format!(
            "{} by-value union input is not supported; use an explicit tagged pointer input",
            semantic_type_definition_name(definition)
        )),
        _ => project_value_type(semantic, abi_type),
    }
}

fn project_output_type(
    semantic: &SemanticComInterface,
    abi_type: super::model::ids::TypeId,
) -> Result<ComType, String> {
    let definition = semantic
        .type_definition(abi_type)
        .map_err(|error| error.to_string())?;
    match definition.abi() {
        ComAbiType::Pointer { pointee, depth, .. } if depth.get() == 1 => {
            let pointee_definition = semantic
                .type_definition(*pointee)
                .map_err(|error| error.to_string())?;
            if let ComAbiType::NativeStruct(layout_id) = pointee_definition.abi()
                && native_layout_requires_output_ownership(
                    semantic,
                    *layout_id,
                    &mut std::collections::BTreeSet::new(),
                )?
            {
                return Err(
                    "native struct outputs with nested pointer lifetimes require ownership projection"
                        .into(),
                );
            }
            if matches!(pointee_definition.abi(), ComAbiType::NativeUnion(_)) {
                return Err(
                    "native union outputs require an explicit active-field/discriminant contract"
                        .into(),
                );
            }
            if matches!(pointee_definition.abi(), ComAbiType::SafeArray { .. }) {
                return Err(
                    "SAFEARRAY output requires SAFEARRAY** ownership, not writable descriptor bytes"
                        .into(),
                );
            }
            if matches!(pointee_definition.abi(), ComAbiType::DispatchParams) {
                return Err("DISPPARAMS is input-only".into());
            }
            project_value_type(semantic, *pointee)
        }
        ComAbiType::Pointer { pointee, depth, .. }
            if depth.get() == 2
                && matches!(
                    semantic
                        .type_definition(*pointee)
                        .map_err(|error| error.to_string())?
                        .abi(),
                    ComAbiType::SafeArray { .. }
                ) =>
        {
            project_value_type(semantic, *pointee)
        }
        ComAbiType::Pointer { .. } => Ok(ComType::RawPointer),
        ComAbiType::DataPointer { depth, .. } if depth.get() > 1 => Ok(ComType::RawPointer),
        ComAbiType::CountedBuffer { .. } => project_counted_buffer_type(semantic, definition),
        ComAbiType::NativeStruct(layout_id)
            if native_layout_requires_output_ownership(
                semantic,
                *layout_id,
                &mut std::collections::BTreeSet::new(),
            )? =>
        {
            Err(
                "native struct outputs with nested pointer lifetimes require ownership projection"
                    .into(),
            )
        }
        ComAbiType::Variant
        | ComAbiType::PropVariant
        | ComAbiType::SafeArray { .. }
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo => Err(
            "Automation outputs require VARIANT*, PROPVARIANT*, or SAFEARRAY** pointer metadata"
                .into(),
        ),
        ComAbiType::StatStg => project_value_type(semantic, abi_type),
        _ => project_value_type(semantic, abi_type),
    }
}

fn native_layout_requires_output_ownership(
    semantic: &SemanticComInterface,
    layout_id: super::model::ids::LayoutId,
    visiting: &mut std::collections::BTreeSet<super::model::ids::LayoutId>,
) -> Result<bool, String> {
    if !visiting.insert(layout_id) {
        return Err("recursive native layout while checking resource ownership".into());
    }
    let layouts = semantic
        .layout_definition(layout_id)
        .map_err(|error| error.to_string())?;
    let mut contains = false;
    'architectures: for architecture in [Architecture::X86, Architecture::X64, Architecture::Arm64]
    {
        for field in layouts.get(architecture).fields() {
            let definition = semantic
                .type_definition(field.abi_type())
                .map_err(|error| error.to_string())?;
            match definition.abi() {
                ComAbiType::Handle(_)
                | ComAbiType::DataPointer { .. }
                | ComAbiType::Pointer { .. }
                | ComAbiType::StringPointer { .. }
                | ComAbiType::Bstr
                | ComAbiType::HString
                | ComAbiType::ComInterface { .. } => {
                    contains = true;
                    break 'architectures;
                }
                ComAbiType::NativeStruct(nested) => {
                    if native_layout_requires_output_ownership(semantic, *nested, visiting)? {
                        contains = true;
                        break 'architectures;
                    }
                }
                _ => {}
            }
        }
    }
    visiting.remove(&layout_id);
    Ok(contains)
}

fn project_value_type(
    semantic: &SemanticComInterface,
    abi_type: super::model::ids::TypeId,
) -> Result<ComType, String> {
    let definition = semantic
        .type_definition(abi_type)
        .map_err(|error| error.to_string())?;
    match definition.abi() {
        ComAbiType::Scalar(scalar) => project_scalar(definition, *scalar),
        ComAbiType::Guid => Ok(ComType::Guid),
        ComAbiType::Enum(enum_id) => {
            let definition = semantic
                .enum_definition(*enum_id)
                .map_err(|error| error.to_string())?;
            Ok(ComType::Enum {
                namespace: definition.native_name().namespace().into(),
                name: definition.native_name().name().into(),
                underlying: project_enum_underlying(definition.underlying())?,
            })
        }
        ComAbiType::Handle(handle) => Ok(ComType::PointerAlias {
            namespace: handle.native_name().namespace().into(),
            name: handle.native_name().name().into(),
            kind: PointerAliasKind::HandleValue,
        }),
        ComAbiType::DataPointer { .. } => {
            if definition.underlying().is_some() {
                let name = definition
                    .native_name()
                    .ok_or_else(|| "validated data-pointer alias has no native name".to_string())?;
                Ok(ComType::PointerAlias {
                    namespace: name.namespace().into(),
                    name: name.name().into(),
                    kind: PointerAliasKind::DataPointer,
                })
            } else {
                Ok(ComType::RawPointer)
            }
        }
        ComAbiType::StringPointer { encoding, .. } => {
            let name = definition
                .native_name()
                .ok_or_else(|| "validated string pointer has no native name".to_string())?;
            Ok(ComType::PointerAlias {
                namespace: name.namespace().into(),
                name: name.name().into(),
                kind: PointerAliasKind::StringPointer(project_string_encoding(*encoding)),
            })
        }
        ComAbiType::Bstr => Ok(ComType::Bstr),
        ComAbiType::HString => Ok(ComType::HString),
        ComAbiType::ComInterface { iid } => Ok(ComType::ManagedInterface {
            iid: format_guid(iid.as_bytes()),
        }),
        ComAbiType::Pointer { .. } => Ok(ComType::RawPointer),
        ComAbiType::CountedBuffer { .. } => project_counted_buffer_type(semantic, definition),
        ComAbiType::NativeStruct(layout_id) => {
            let native_name = definition
                .native_name()
                .ok_or_else(|| "validated native POD struct has no qualified name".to_string())?;
            Ok(ComType::NativePod {
                layout: project_native_pod_layout(
                    semantic,
                    native_name.namespace(),
                    native_name.name(),
                    *layout_id,
                )?,
            })
        }
        ComAbiType::Variant => Ok(ComType::Variant),
        ComAbiType::SafeArray {
            element: Some(element),
        } => Ok(ComType::SafeArray {
            element: project_safe_array_element(semantic, *element)?,
        }),
        ComAbiType::SafeArray { element: None } => {
            Err("SAFEARRAY element VARTYPE is not proven".into())
        }
        ComAbiType::PropVariant => Ok(ComType::PropVariant),
        ComAbiType::DispatchParams => Ok(ComType::DispatchParams),
        ComAbiType::ExcepInfo => Ok(ComType::ExcepInfo),
        ComAbiType::StatStg => Ok(ComType::StatStg),
        ComAbiType::NativeUnion(_) | ComAbiType::FunctionPointer(_) | ComAbiType::Unknown(_) => {
            Err(format!(
                "unsupported Classic-COM semantic type {}",
                semantic_type_definition_name(definition)
            ))
        }
    }
}

fn project_native_union_layout(
    semantic: &SemanticComInterface,
    namespace: &str,
    name: &str,
    layout_id: super::model::ids::LayoutId,
) -> Result<NativeUnionLayout, String> {
    let layouts = semantic
        .layout_definition(layout_id)
        .map_err(|error| error.to_string())?;
    let x86 = project_native_union_architecture(
        semantic,
        layouts.get(Architecture::X86),
        Architecture::X86,
    )?;
    let x64 = project_native_union_architecture(
        semantic,
        layouts.get(Architecture::X64),
        Architecture::X64,
    )?;
    let arm64 = project_native_union_architecture(
        semantic,
        layouts.get(Architecture::Arm64),
        Architecture::Arm64,
    )?;
    let field_contract = |layout: &NativeUnionArchitectureLayout| {
        layout
            .fields
            .iter()
            .map(|field| (field.name.clone(), field.count))
            .collect::<Vec<_>>()
    };
    if field_contract(&x86) != field_contract(&x64)
        || field_contract(&x64) != field_contract(&arm64)
    {
        return Err(format!(
            "native union `{namespace}.{name}` has architecture-dependent active fields"
        ));
    }
    Ok(NativeUnionLayout {
        namespace: namespace.into(),
        name: name.into(),
        x86,
        x64,
        arm64,
    })
}

fn project_safe_array_element(
    semantic: &SemanticComInterface,
    type_id: super::model::ids::TypeId,
) -> Result<SafeArrayElement, String> {
    let definition = semantic
        .type_definition(type_id)
        .map_err(|error| error.to_string())?;
    Ok(match definition.abi() {
        ComAbiType::Scalar(ScalarType::I8) => SafeArrayElement::I8,
        ComAbiType::Scalar(ScalarType::U8) => SafeArrayElement::U8,
        ComAbiType::Scalar(ScalarType::I16) => SafeArrayElement::I16,
        ComAbiType::Scalar(ScalarType::U16) => SafeArrayElement::U16,
        ComAbiType::Scalar(ScalarType::I32) => SafeArrayElement::I32,
        ComAbiType::Scalar(ScalarType::U32) => SafeArrayElement::U32,
        ComAbiType::Scalar(ScalarType::I64) => SafeArrayElement::I64,
        ComAbiType::Scalar(ScalarType::U64) => SafeArrayElement::U64,
        ComAbiType::Scalar(ScalarType::F32) => SafeArrayElement::F32,
        ComAbiType::Scalar(ScalarType::F64) => SafeArrayElement::F64,
        ComAbiType::Scalar(ScalarType::Bool | ScalarType::Win32Bool) => SafeArrayElement::Bool,
        ComAbiType::Bstr => SafeArrayElement::Bstr,
        ComAbiType::Variant => SafeArrayElement::Variant,
        ComAbiType::ComInterface { iid } => SafeArrayElement::Interface {
            iid: *iid.as_bytes(),
        },
        unsupported => {
            return Err(format!(
                "unsupported SAFEARRAY element semantic {unsupported:?}"
            ));
        }
    })
}

fn project_native_union_architecture(
    semantic: &SemanticComInterface,
    layout: &super::model::layout::NativeLayout,
    architecture: Architecture,
) -> Result<NativeUnionArchitectureLayout, String> {
    if layout.kind() != super::model::layout::LayoutKind::Union {
        return Err("native union type references a non-union layout".into());
    }
    if layout.packing() != 8 {
        return Err(format!(
            "packed native union layouts are not supported (packing {})",
            layout.packing()
        ));
    }
    let fields = layout
        .fields()
        .iter()
        .map(|field| {
            if field.offset() != 0 {
                return Err(format!(
                    "native union field `{}` does not overlap at offset zero",
                    field.name()
                ));
            }
            Ok(NativeUnionField {
                name: field.name().into(),
                count: field.fixed_count().map_or(1, |count| count.get()),
                typ: project_native_union_field_type(semantic, field.abi_type(), architecture)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(NativeUnionArchitectureLayout {
        size: layout.size(),
        alignment: layout.alignment(),
        fields,
    })
}

fn project_native_union_field_type(
    semantic: &SemanticComInterface,
    type_id: super::model::ids::TypeId,
    architecture: Architecture,
) -> Result<NativeUnionFieldType, String> {
    let definition = semantic
        .type_definition(type_id)
        .map_err(|error| error.to_string())?;
    match definition.abi() {
        ComAbiType::Scalar(scalar) => Ok(NativeUnionFieldType::Scalar(match scalar {
            ScalarType::Bool | ScalarType::I8 => NativePodScalar::I8,
            ScalarType::U8 => NativePodScalar::U8,
            ScalarType::I16 => NativePodScalar::I16,
            ScalarType::U16 | ScalarType::Char16 => NativePodScalar::U16,
            ScalarType::I32 | ScalarType::Win32Bool | ScalarType::HResult => NativePodScalar::I32,
            ScalarType::U32 => NativePodScalar::U32,
            ScalarType::I64 => NativePodScalar::I64,
            ScalarType::U64 => NativePodScalar::U64,
            ScalarType::F32 => NativePodScalar::F32,
            ScalarType::F64 => NativePodScalar::F64,
            ScalarType::NativeIsize => NativePodScalar::NativeIsize,
            ScalarType::NativeUsize => NativePodScalar::NativeUsize,
        })),
        ComAbiType::Guid => Ok(NativeUnionFieldType::Guid),
        ComAbiType::Enum(enum_id) => {
            let underlying = semantic
                .enum_definition(*enum_id)
                .map_err(|error| error.to_string())?
                .underlying();
            Ok(NativeUnionFieldType::Scalar(match underlying {
                ScalarType::I8 => NativePodScalar::I8,
                ScalarType::U8 => NativePodScalar::U8,
                ScalarType::I16 => NativePodScalar::I16,
                ScalarType::U16 => NativePodScalar::U16,
                ScalarType::I32 => NativePodScalar::I32,
                ScalarType::U32 => NativePodScalar::U32,
                ScalarType::I64 => NativePodScalar::I64,
                ScalarType::U64 => NativePodScalar::U64,
                _ => return Err("native union enum has a non-integral ABI".into()),
            }))
        }
        ComAbiType::Pointer { .. } | ComAbiType::DataPointer { .. } => {
            Ok(NativeUnionFieldType::Pointer)
        }
        ComAbiType::NativeStruct(layout_id) => {
            let native_name = definition
                .native_name()
                .ok_or_else(|| "nested native POD has no qualified name".to_string())?;
            let nested = semantic
                .layout_definition(*layout_id)
                .map_err(|error| error.to_string())?;
            let mut visiting = std::collections::HashSet::new();
            Ok(NativeUnionFieldType::Struct {
                name: format!("{}.{}", native_name.namespace(), native_name.name()),
                layout: Box::new(project_native_pod_architecture(
                    semantic,
                    nested.get(architecture),
                    architecture,
                    &mut visiting,
                )?),
            })
        }
        _ => Err(format!(
            "unsupported owned or nested union field {}",
            semantic_type_definition_name(definition)
        )),
    }
}

fn project_native_pod_layout(
    semantic: &SemanticComInterface,
    namespace: &str,
    name: &str,
    layout_id: super::model::ids::LayoutId,
) -> Result<NativePodLayout, String> {
    let layouts = semantic
        .layout_definition(layout_id)
        .map_err(|error| error.to_string())?;
    let mut visiting = std::collections::HashSet::new();
    let x86 = project_native_pod_architecture(
        semantic,
        layouts.get(Architecture::X86),
        Architecture::X86,
        &mut visiting,
    )?;
    let x64 = project_native_pod_architecture(
        semantic,
        layouts.get(Architecture::X64),
        Architecture::X64,
        &mut visiting,
    )?;
    let arm64 = project_native_pod_architecture(
        semantic,
        layouts.get(Architecture::Arm64),
        Architecture::Arm64,
        &mut visiting,
    )?;
    let initializers = native_pod_initializers(namespace, name, &x86, &x64, &arm64)?;
    Ok(NativePodLayout {
        namespace: namespace.into(),
        name: name.into(),
        initializers,
        x86,
        x64,
        arm64,
    })
}

fn native_pod_initializers(
    namespace: &str,
    name: &str,
    x86: &NativePodArchitectureLayout,
    x64: &NativePodArchitectureLayout,
    arm64: &NativePodArchitectureLayout,
) -> Result<Vec<NativePodInitializer>, String> {
    if namespace != "Windows.Win32.System.Com"
        || !matches!(name, "BIND_OPTS" | "BIND_OPTS2" | "BIND_OPTS3")
    {
        return Ok(Vec::new());
    }
    let expected = match name {
        "BIND_OPTS" => [(16, 4), (16, 4), (16, 4)],
        "BIND_OPTS2" => [(32, 4), (40, 8), (40, 8)],
        "BIND_OPTS3" => [(36, 4), (48, 8), (48, 8)],
        _ => unreachable!(),
    };
    for (architecture, layout, (size, alignment)) in [
        ("x86", x86, expected[0]),
        ("x64", x64, expected[1]),
        ("ARM64", arm64, expected[2]),
    ] {
        if layout.size != size || layout.alignment != alignment {
            return Err(format!(
                "{namespace}.{name} {architecture} layout must be size {size}, alignment {alignment}; found size {}, alignment {}",
                layout.size, layout.alignment
            ));
        }
        let Some(field) = layout.fields.iter().find(|field| field.name == "cbStruct") else {
            return Err(format!(
                "{namespace}.{name} {architecture} layout is missing cbStruct"
            ));
        };
        if field.offset != 0
            || field.count != 1
            || field.typ != NativePodFieldType::Scalar(NativePodScalar::U32)
        {
            return Err(format!(
                "{namespace}.{name} {architecture} cbStruct must be one u32 at offset 0"
            ));
        }
    }
    Ok(vec![NativePodInitializer::SizeOfLayout {
        field: "cbStruct".into(),
    }])
}

fn project_native_pod_architecture(
    semantic: &SemanticComInterface,
    layout: &super::model::layout::NativeLayout,
    architecture: Architecture,
    visiting: &mut std::collections::HashSet<super::model::ids::LayoutId>,
) -> Result<NativePodArchitectureLayout, String> {
    if layout.packing() != 8 {
        return Err(format!(
            "packed native POD layouts are not supported (packing {})",
            layout.packing()
        ));
    }
    let fields = layout
        .fields()
        .iter()
        .map(|field| {
            Ok(NativePodField {
                name: field.name().into(),
                offset: field.offset(),
                count: field.fixed_count().map_or(1, |count| count.get()),
                typ: project_native_pod_field_type(
                    semantic,
                    field.abi_type(),
                    architecture,
                    visiting,
                )?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(NativePodArchitectureLayout {
        size: layout.size(),
        alignment: layout.alignment(),
        fields,
    })
}

fn project_native_pod_field_type(
    semantic: &SemanticComInterface,
    type_id: super::model::ids::TypeId,
    architecture: Architecture,
    visiting: &mut std::collections::HashSet<super::model::ids::LayoutId>,
) -> Result<NativePodFieldType, String> {
    let definition = semantic
        .type_definition(type_id)
        .map_err(|error| error.to_string())?;
    match definition.abi() {
        ComAbiType::Scalar(scalar) => Ok(NativePodFieldType::Scalar(match scalar {
            ScalarType::Bool | ScalarType::I8 => NativePodScalar::I8,
            ScalarType::U8 => NativePodScalar::U8,
            ScalarType::I16 => NativePodScalar::I16,
            ScalarType::U16 | ScalarType::Char16 => NativePodScalar::U16,
            ScalarType::I32 | ScalarType::Win32Bool | ScalarType::HResult => NativePodScalar::I32,
            ScalarType::U32 => NativePodScalar::U32,
            ScalarType::I64 => NativePodScalar::I64,
            ScalarType::U64 => NativePodScalar::U64,
            ScalarType::F32 => NativePodScalar::F32,
            ScalarType::F64 => NativePodScalar::F64,
            ScalarType::NativeIsize => NativePodScalar::NativeIsize,
            ScalarType::NativeUsize => NativePodScalar::NativeUsize,
        })),
        ComAbiType::Guid => Ok(NativePodFieldType::Guid),
        ComAbiType::Enum(enum_id) => {
            let underlying = semantic
                .enum_definition(*enum_id)
                .map_err(|error| error.to_string())?
                .underlying();
            Ok(NativePodFieldType::Scalar(match underlying {
                ScalarType::I8 => NativePodScalar::I8,
                ScalarType::U8 => NativePodScalar::U8,
                ScalarType::I16 => NativePodScalar::I16,
                ScalarType::U16 => NativePodScalar::U16,
                ScalarType::I32 => NativePodScalar::I32,
                ScalarType::U32 => NativePodScalar::U32,
                ScalarType::I64 => NativePodScalar::I64,
                ScalarType::U64 => NativePodScalar::U64,
                _ => return Err("native POD enum has a non-integral ABI".into()),
            }))
        }
        ComAbiType::Pointer { .. }
        | ComAbiType::DataPointer { .. }
        | ComAbiType::Handle(_)
        | ComAbiType::StringPointer { .. } => Ok(NativePodFieldType::Pointer),
        ComAbiType::NativeStruct(layout_id) => {
            if !visiting.insert(*layout_id) {
                return Err("recursive native POD layout".into());
            }
            let native_name = definition
                .native_name()
                .ok_or_else(|| "nested native POD has no qualified name".to_string())?;
            let nested = semantic
                .layout_definition(*layout_id)
                .map_err(|error| error.to_string())?;
            let projected = project_native_pod_architecture(
                semantic,
                nested.get(architecture),
                architecture,
                visiting,
            )?;
            visiting.remove(layout_id);
            Ok(NativePodFieldType::Struct {
                name: format!("{}.{}", native_name.namespace(), native_name.name()),
                layout: Box::new(projected),
            })
        }
        ComAbiType::Bstr
        | ComAbiType::HString
        | ComAbiType::ComInterface { .. }
        | ComAbiType::NativeUnion(_)
        | ComAbiType::CountedBuffer { .. }
        | ComAbiType::SafeArray { .. }
        | ComAbiType::Variant
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
        | ComAbiType::FunctionPointer(_)
        | ComAbiType::Unknown(_) => Err(format!(
            "unsupported nested native POD field {}",
            semantic_type_definition_name(definition)
        )),
    }
}

fn project_scalar(definition: &ComTypeDefinition, scalar: ScalarType) -> Result<ComType, String> {
    if definition.underlying().is_some() {
        let name = definition
            .native_name()
            .ok_or_else(|| "validated scalar alias has no native name".to_string())?;
        return Ok(ComType::ScalarAlias {
            namespace: name.namespace().into(),
            name: name.name().into(),
            underlying: project_scalar_repr(scalar)?,
        });
    }
    match scalar {
        ScalarType::NativeIsize => Ok(ComType::NativeIsize),
        ScalarType::NativeUsize => Ok(ComType::NativeUsize),
        ScalarType::Win32Bool => Ok(ComType::Win32Bool),
        ScalarType::HResult => Ok(ComType::HResult),
        _ => Ok(ComType::Primitive(project_primitive(scalar)?)),
    }
}

fn project_primitive(scalar: ScalarType) -> Result<ComPrimitive, String> {
    match scalar {
        ScalarType::Bool => Ok(ComPrimitive::Bool),
        ScalarType::I8 => Ok(ComPrimitive::I8),
        ScalarType::U8 => Ok(ComPrimitive::U8),
        ScalarType::I16 => Ok(ComPrimitive::I16),
        ScalarType::U16 => Ok(ComPrimitive::U16),
        ScalarType::I32 => Ok(ComPrimitive::I32),
        ScalarType::U32 => Ok(ComPrimitive::U32),
        ScalarType::I64 => Ok(ComPrimitive::I64),
        ScalarType::U64 => Ok(ComPrimitive::U64),
        ScalarType::F32 => Ok(ComPrimitive::F32),
        ScalarType::F64 => Ok(ComPrimitive::F64),
        ScalarType::Char16 => Ok(ComPrimitive::Char16),
        ScalarType::NativeIsize
        | ScalarType::NativeUsize
        | ScalarType::Win32Bool
        | ScalarType::HResult => Err(format!(
            "non-primitive scalar {scalar:?} cannot use a primitive projection"
        )),
    }
}

fn project_scalar_repr(scalar: ScalarType) -> Result<ComScalarRepr, String> {
    match scalar {
        ScalarType::NativeIsize => Ok(ComScalarRepr::NativeIsize),
        ScalarType::NativeUsize => Ok(ComScalarRepr::NativeUsize),
        ScalarType::Win32Bool | ScalarType::HResult => Err(format!(
            "special scalar {scalar:?} cannot use a transparent alias projection"
        )),
        _ => Ok(ComScalarRepr::Primitive(project_primitive(scalar)?)),
    }
}

fn project_enum_underlying(scalar: ScalarType) -> Result<ComEnumUnderlying, String> {
    match scalar {
        ScalarType::I8 => Ok(ComEnumUnderlying::I8),
        ScalarType::U8 => Ok(ComEnumUnderlying::U8),
        ScalarType::I16 => Ok(ComEnumUnderlying::I16),
        ScalarType::U16 => Ok(ComEnumUnderlying::U16),
        ScalarType::I32 => Ok(ComEnumUnderlying::I32),
        ScalarType::U32 => Ok(ComEnumUnderlying::U32),
        ScalarType::I64 => Ok(ComEnumUnderlying::I64),
        ScalarType::U64 => Ok(ComEnumUnderlying::U64),
        ScalarType::Bool
        | ScalarType::F32
        | ScalarType::F64
        | ScalarType::Char16
        | ScalarType::NativeIsize
        | ScalarType::NativeUsize
        | ScalarType::Win32Bool
        | ScalarType::HResult => Err(format!("unsupported COM enum underlying type {scalar:?}")),
    }
}

fn project_counted_buffer_type(
    semantic: &SemanticComInterface,
    definition: &ComTypeDefinition,
) -> Result<ComType, String> {
    let ComAbiType::CountedBuffer {
        element,
        element_ownership,
        ..
    } = definition.abi()
    else {
        unreachable!("counted-buffer projection requires CountedBuffer semantics");
    };
    match element {
        BufferElement::Character(encoding) => {
            let name = definition
                .native_name()
                .ok_or_else(|| "validated string buffer has no native name".to_string())?;
            Ok(ComType::PointerAlias {
                namespace: name.namespace().into(),
                name: name.name().into(),
                kind: PointerAliasKind::StringPointer(project_string_encoding(*encoding)),
            })
        }
        BufferElement::StringPointer {
            encoding,
            pointer_depth,
            constness,
        } => match element_ownership {
            BufferElementOwnership::Borrowed => Ok(ComType::StringArray {
                encoding: project_string_encoding(*encoding),
                element_pointer_depth: pointer_depth.get(),
                element_const: *constness == Constness::Const,
            }),
            BufferElementOwnership::CoTaskMemStringOwned
                if *encoding == super::model::abi::StringEncoding::Utf16 =>
            {
                Ok(ComType::OwningArray {
                    element: Box::new(ComType::CoTaskMemWideString),
                    interface: None,
                })
            }
            _ => Err("string pointer array ownership is not safely projected".into()),
        },
        BufferElement::Typed(element) => {
            let projected = project_value_type(semantic, *element)?;
            if matches!(
                element_ownership,
                BufferElementOwnership::ComOwned
                    | BufferElementOwnership::BstrOwned
                    | BufferElementOwnership::VariantOwned
            ) {
                Ok(ComType::OwningArray {
                    interface: project_interface_ref_for_type(semantic, *element)?,
                    element: Box::new(projected),
                })
            } else {
                Ok(ComType::TypedBuffer {
                    element: Box::new(projected),
                })
            }
        }
        BufferElement::Opaque(_) => {
            Err("opaque counted-buffer elements are not safe to project".into())
        }
    }
}

fn project_typed_buffers(
    semantic: &SemanticComInterface,
    method: &ComMethodContract,
) -> Result<Vec<TypedBufferPlan>, String> {
    let mut plans = Vec::new();
    for (buffer_param_index, param) in method.params().iter().enumerate() {
        let definition = semantic
            .type_definition(param.abi_type())
            .map_err(|error| error.to_string())?;
        let ComAbiType::CountedBuffer {
            element,
            element_ownership,
            ..
        } = definition.abi()
        else {
            continue;
        };
        let element = match element {
            BufferElement::StringPointer {
                encoding,
                pointer_depth,
                constness,
            } => {
                if matches!(element_ownership, BufferElementOwnership::Borrowed)
                    && (param.direction() != Direction::In
                        || !matches!(
                            param.count(),
                            Some(CountRelation::InputCount {
                                actual_length_param: None,
                                unit: CountUnit::Elements,
                                ..
                            })
                        ))
                {
                    return Err(format!(
                        "{}: string pointer array `{}` must be a borrowed element-counted input",
                        method.name(),
                        param.name()
                    ));
                }
                match element_ownership {
                    BufferElementOwnership::Borrowed => ComType::StringArray {
                        encoding: project_string_encoding(*encoding),
                        element_pointer_depth: pointer_depth.get(),
                        element_const: *constness == Constness::Const,
                    },
                    BufferElementOwnership::CoTaskMemStringOwned
                        if *encoding == super::model::abi::StringEncoding::Utf16 =>
                    {
                        ComType::CoTaskMemWideString
                    }
                    _ => {
                        return Err(format!(
                            "{}: string pointer array `{}` has unsupported element ownership",
                            method.name(),
                            param.name()
                        ));
                    }
                }
            }
            BufferElement::Typed(element) => project_value_type(semantic, *element)?,
            BufferElement::Character(_) | BufferElement::Opaque(_) => continue,
        };
        if matches!(element, ComType::NativePod { .. })
            && !matches!(
                param.count(),
                Some(CountRelation::InputCount { .. } | CountRelation::EnumeratorNext { .. })
            )
        {
            return Err(format!(
                "{}: native struct counted buffer `{}` currently supports input contracts only",
                method.name(),
                param.name()
            ));
        }
        if !is_supported_buffer_element(&element) {
            return Err(format!(
                "{}: counted buffer `{}` has unsupported element type",
                method.name(),
                param.name()
            ));
        }
        let relation = match param
            .count()
            .ok_or_else(|| format!("{}: counted buffer has no relation", param.name()))?
        {
            CountRelation::InputCount {
                count_param,
                actual_length_param,
                unit,
            } => TypedBufferRelation::Input {
                count_param_index: count_param.index(),
                actual_length_param_index: actual_length_param.map(|index| index.index()),
                unit: project_count_unit(*unit),
            },
            CountRelation::CallerCapacity {
                capacity_param,
                actual_length_param,
                unit,
                sizing,
            } => {
                let sizing = match sizing {
                    BufferSizing::SingleCall => TypedBufferSizing::SingleCall,
                    BufferSizing::FixedCapacity => TypedBufferSizing::FixedCapacity,
                    BufferSizing::TwoCall { max_retries } => {
                        if *unit != CountUnit::Bytes
                            || !matches!(element, ComType::Primitive(ComPrimitive::U8))
                        {
                            return Err(format!(
                                "{}: bounded two-call sizing currently requires a byte buffer",
                                method.name()
                            ));
                        }
                        TypedBufferSizing::TwoCall {
                            max_retries: *max_retries,
                        }
                    }
                };
                TypedBufferRelation::CallerOutput {
                    capacity_param_index: capacity_param.index(),
                    actual_length_param_index: actual_length_param.map(|index| index.index()),
                    unit: project_count_unit(*unit),
                    sizing,
                }
            }
            CountRelation::EnumeratorNext {
                capacity_param,
                fetched_param,
                fetched_optional_for_single,
            } => TypedBufferRelation::EnumeratorNext {
                capacity_param_index: capacity_param.index(),
                fetched_param_index: fetched_param.index(),
                fetched_optional_for_single: *fetched_optional_for_single,
            },
            CountRelation::CalleeAllocated { count_param, unit } => {
                TypedBufferRelation::CalleeAllocated {
                    count_param_index: count_param.index(),
                    unit: project_count_unit(*unit),
                }
            }
        };
        plans.push(TypedBufferPlan {
            buffer_param_index,
            element,
            relation,
        });
    }
    Ok(plans)
}

fn project_enumerator_interface_ref(
    semantic: &SemanticComInterface,
    method: &ComMethodContract,
    buffer_param_index: usize,
) -> Result<Option<ProjectedInterfaceRef>, String> {
    let param = method
        .params()
        .get(buffer_param_index)
        .ok_or_else(|| "EnumeratorNext buffer index is outside the method".to_string())?;
    let definition = semantic
        .type_definition(param.abi_type())
        .map_err(|error| error.to_string())?;
    let ComAbiType::CountedBuffer {
        element: BufferElement::Typed(element),
        ..
    } = definition.abi()
    else {
        return Ok(None);
    };
    project_interface_ref_for_type(semantic, *element)
}

fn project_buffer_interface_ref(
    semantic: &SemanticComInterface,
    method: &ComMethodContract,
    buffer_param_index: usize,
) -> Result<Option<ProjectedInterfaceRef>, String> {
    project_enumerator_interface_ref(semantic, method, buffer_param_index)
}

fn project_interface_ref_for_type(
    semantic: &SemanticComInterface,
    element: super::model::ids::TypeId,
) -> Result<Option<ProjectedInterfaceRef>, String> {
    let element = semantic
        .type_definition(element)
        .map_err(|error| error.to_string())?;
    let ComAbiType::ComInterface { iid } = element.abi() else {
        return Ok(None);
    };
    if iid.as_bytes()
        == &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x46,
        ]
    {
        return Ok(None);
    }
    let name = element.native_name().ok_or_else(|| {
        "EnumeratorNext interface element has no exact native identity".to_string()
    })?;
    Ok(Some(ProjectedInterfaceRef {
        namespace: name.namespace().into(),
        name: name.name().into(),
    }))
}

fn project_count_unit(unit: CountUnit) -> ProjectedBufferCountUnit {
    match unit {
        CountUnit::Elements => ProjectedBufferCountUnit::Elements,
        CountUnit::Bytes => ProjectedBufferCountUnit::Bytes,
    }
}

fn project_shared_counts(
    method: &ComMethodContract,
    buffers: &[TypedBufferPlan],
) -> Result<Vec<SharedCountPlan>, String> {
    let mut by_count = std::collections::BTreeMap::<usize, Vec<&TypedBufferPlan>>::new();
    for plan in buffers {
        let count_param_index = match plan.relation {
            TypedBufferRelation::Input {
                count_param_index, ..
            } => count_param_index,
            TypedBufferRelation::CallerOutput {
                capacity_param_index,
                ..
            } => capacity_param_index,
            TypedBufferRelation::EnumeratorNext {
                capacity_param_index,
                ..
            } => capacity_param_index,
            TypedBufferRelation::CalleeAllocated {
                count_param_index, ..
            } => count_param_index,
        };
        by_count.entry(count_param_index).or_default().push(plan);
    }
    let mut groups = Vec::new();
    for (count_param_index, plans) in by_count {
        if plans.len() == 1 {
            continue;
        }
        let input_units = plans
            .iter()
            .map(|plan| match plan.relation {
                TypedBufferRelation::Input {
                    count_param_index: count,
                    actual_length_param_index: None,
                    unit,
                } if count == count_param_index => Some(unit),
                _ => None,
            })
            .collect::<Option<Vec<_>>>();
        if input_units.is_some_and(|units| {
            units
                .first()
                .is_some_and(|first| units.iter().all(|unit| unit == first))
        }) {
            continue;
        }
        if plans.len() == 2
            && plans
                .iter()
                .any(|plan| matches!(plan.element, ComType::StringArray { .. }))
        {
            let mut string_input = None;
            let mut scalar_output = None;
            for plan in &plans {
                match (&plan.element, &plan.relation) {
                    (
                        ComType::StringArray { .. },
                        TypedBufferRelation::Input {
                            count_param_index: count,
                            actual_length_param_index: None,
                            unit: ProjectedBufferCountUnit::Elements,
                        },
                    ) if *count == count_param_index && string_input.is_none() => {
                        string_input = Some(plan.buffer_param_index);
                    }
                    (
                        element,
                        TypedBufferRelation::CallerOutput {
                            capacity_param_index,
                            actual_length_param_index: None,
                            unit: ProjectedBufferCountUnit::Elements,
                            sizing: TypedBufferSizing::SingleCall,
                        },
                    ) if *capacity_param_index == count_param_index
                        && is_plain_array_output_element(element)
                        && scalar_output.is_none() =>
                    {
                        scalar_output = Some(plan.buffer_param_index);
                    }
                    _ => {}
                }
            }
            if let (Some(string_input_param_index), Some(scalar_output_param_index)) =
                (string_input, scalar_output)
            {
                groups.push(SharedCountPlan::StringInputScalarOutput {
                    count_param_index,
                    string_input_param_index,
                    scalar_output_param_index,
                });
                continue;
            }
            return Err(format!(
                "{}: count parameter {count_param_index} has an unsupported string-array group",
                method.name()
            ));
        }
        if plans
            .iter()
            .any(|plan| matches!(plan.element, ComType::StringArray { .. }))
        {
            return Err(format!(
                "{}: count parameter {count_param_index} has an unsupported string-array group",
                method.name()
            ));
        }
        let mut parallel_inputs = Vec::new();
        let mut parallel_outputs = Vec::new();
        let parallel = plans.iter().all(|plan| match plan.relation {
            TypedBufferRelation::Input {
                count_param_index: count,
                actual_length_param_index: None,
                unit: ProjectedBufferCountUnit::Elements,
            } if count == count_param_index => {
                parallel_inputs.push(plan.buffer_param_index);
                true
            }
            TypedBufferRelation::CallerOutput {
                capacity_param_index: count,
                actual_length_param_index: None,
                unit: ProjectedBufferCountUnit::Elements,
                sizing: TypedBufferSizing::SingleCall,
            } if count == count_param_index => {
                parallel_outputs.push(plan.buffer_param_index);
                true
            }
            _ => false,
        });
        if parallel && !parallel_inputs.is_empty() && !parallel_outputs.is_empty() {
            groups.push(SharedCountPlan::Parallel {
                count_param_index,
                input_param_indices: parallel_inputs,
                output_param_indices: parallel_outputs,
            });
            continue;
        }
        if plans.len() != 2 {
            return Err(format!(
                "{}: count parameter {count_param_index} describes an ambiguous buffer group",
                method.name()
            ));
        }
        let mut string_input = None;
        let mut scalar_output = None;
        for plan in plans {
            match (&plan.element, &plan.relation) {
                (
                    ComType::StringArray { .. },
                    TypedBufferRelation::Input {
                        count_param_index: count,
                        actual_length_param_index: None,
                        unit: ProjectedBufferCountUnit::Elements,
                    },
                ) if *count == count_param_index && string_input.is_none() => {
                    string_input = Some(plan.buffer_param_index);
                }
                (
                    element,
                    TypedBufferRelation::CallerOutput {
                        capacity_param_index,
                        actual_length_param_index: None,
                        unit: ProjectedBufferCountUnit::Elements,
                        sizing: TypedBufferSizing::SingleCall,
                    },
                ) if *capacity_param_index == count_param_index
                    && is_plain_array_output_element(element)
                    && scalar_output.is_none() =>
                {
                    scalar_output = Some(plan.buffer_param_index);
                }
                _ => {
                    return Err(format!(
                        "{}: count parameter {count_param_index} describes unrelated buffers",
                        method.name()
                    ));
                }
            }
        }
        groups.push(SharedCountPlan::StringInputScalarOutput {
            count_param_index,
            string_input_param_index: string_input.ok_or_else(|| {
                format!(
                    "{}: shared count {count_param_index} has no string input array",
                    method.name()
                )
            })?,
            scalar_output_param_index: scalar_output.ok_or_else(|| {
                format!(
                    "{}: shared count {count_param_index} has no plain scalar output array",
                    method.name()
                )
            })?,
        });
    }
    Ok(groups)
}

fn is_plain_array_output_element(typ: &ComType) -> bool {
    matches!(
        typ,
        ComType::Primitive(_)
            | ComType::NativeIsize
            | ComType::NativeUsize
            | ComType::Win32Bool
            | ComType::HResult
            | ComType::Enum { .. }
            | ComType::ScalarAlias { .. }
    )
}

fn is_supported_buffer_element(typ: &ComType) -> bool {
    matches!(
        typ,
        ComType::Primitive(_)
            | ComType::NativeIsize
            | ComType::NativeUsize
            | ComType::Win32Bool
            | ComType::HResult
            | ComType::Guid
            | ComType::Enum { .. }
            | ComType::ScalarAlias { .. }
            | ComType::NativePod { .. }
            | ComType::StringArray { .. }
            | ComType::ManagedInterface { .. }
            | ComType::Bstr
            | ComType::Variant
            | ComType::CoTaskMemWideString
    )
}

fn is_owning_array_output_element(typ: &ComType) -> bool {
    matches!(
        typ,
        ComType::ManagedInterface { .. }
            | ComType::Bstr
            | ComType::Variant
            | ComType::CoTaskMemWideString
    )
}

fn project_string_buffer(
    semantic: &SemanticComInterface,
    method: &ComMethodContract,
) -> Result<Option<StringBufferPlan>, String> {
    let mut plans = Vec::new();
    for (buffer_param_index, param) in method.params().iter().enumerate() {
        let definition = semantic
            .type_definition(param.abi_type())
            .map_err(|error| error.to_string())?;
        let ComAbiType::CountedBuffer { element, .. } = definition.abi() else {
            continue;
        };
        let BufferElement::Character(element_encoding) = element else {
            continue;
        };
        if param.direction() == Direction::In {
            continue;
        }
        let (capacity_param, encoding) = match param.count() {
            Some(CountRelation::CallerCapacity {
                capacity_param,
                actual_length_param: None,
                unit: CountUnit::Elements,
                sizing: BufferSizing::SingleCall,
            }) => (
                capacity_param.index(),
                project_string_encoding(*element_encoding),
            ),
            _ => {
                return Err(format!(
                    "{}: caller-sized native buffers are not supported (`{}`)",
                    method.name(),
                    param.name()
                ));
            }
        };
        if encoding == StringEncoding::Ansi {
            return Err(format!(
                "{}: caller-owned ANSI output buffers are not yet decoded safely",
                method.name()
            ));
        }
        let optional_param_indices = if method
            .params()
            .iter()
            .skip(capacity_param + 1)
            .all(|param| !param.direction().is_input())
        {
            vec![capacity_param]
        } else {
            Vec::new()
        };
        plans.push(StringBufferPlan {
            buffer_param_index,
            count_param_index: capacity_param,
            encoding,
            optional_param_indices,
        });
    }
    match plans.len() {
        0 => Ok(None),
        1 => Ok(plans.pop()),
        _ => Err(format!(
            "{}: multiple caller-owned string buffers are not supported",
            method.name()
        )),
    }
}

fn result_conversion(
    param: &ComParamContract,
    typ: &ComType,
    dynamic_iid_output: bool,
) -> Result<ResultConversion, String> {
    if dynamic_iid_output {
        return match (param.ownership(), param.cleanup()) {
            (ComOwnership::ComOwned, Cleanup::ComRelease) => {
                Ok(ResultConversion::DynamicIidAdoption)
            }
            _ => Err(format!(
                "{}: dynamic-IID output lacks COM ownership",
                param.name()
            )),
        };
    }
    match (param.ownership(), param.cleanup()) {
        (ComOwnership::Borrowed, Cleanup::None)
            if matches!(
                typ,
                ComType::PointerAlias {
                    kind: PointerAliasKind::HandleValue,
                    ..
                }
            ) =>
        {
            Ok(ResultConversion::BorrowedHandle)
        }
        (ComOwnership::Borrowed, Cleanup::None) => Ok(ResultConversion::Value),
        (ComOwnership::ComOwned, Cleanup::ComRelease)
            if matches!(typ, ComType::ManagedInterface { .. }) =>
        {
            Ok(ResultConversion::ManagedCom)
        }
        (ComOwnership::BstrOwned, Cleanup::SysFreeString) if matches!(typ, ComType::Bstr) => {
            Ok(ResultConversion::Bstr)
        }
        (ComOwnership::BstrReplaced, Cleanup::SysFreeString) if matches!(typ, ComType::Bstr) => {
            Ok(ResultConversion::Bstr)
        }
        (ComOwnership::CoTaskMemOwned, Cleanup::CoTaskMemFree) => match typ {
            ComType::PointerAlias {
                kind: PointerAliasKind::StringPointer(encoding),
                ..
            } => Ok(ResultConversion::CoTaskMemString(*encoding)),
            ComType::RawPointer
            | ComType::PointerAlias {
                kind: PointerAliasKind::DataPointer,
                ..
            } => Ok(ResultConversion::CoTaskMemData),
            _ => Err(format!(
                "{}: CoTaskMem ownership requires a data or string pointer",
                param.name()
            )),
        },
        (ComOwnership::HStringOwned, Cleanup::WindowsDeleteString)
            if matches!(typ, ComType::HString) =>
        {
            Ok(ResultConversion::HString)
        }
        (ComOwnership::VariantOwned, Cleanup::VariantClear) if matches!(typ, ComType::Variant) => {
            Ok(ResultConversion::Variant)
        }
        (ComOwnership::SafeArrayOwned, Cleanup::SafeArrayDestroy)
            if matches!(typ, ComType::SafeArray { .. }) =>
        {
            Ok(ResultConversion::SafeArray)
        }
        (ComOwnership::PropVariantOwned, Cleanup::PropVariantClear)
            if matches!(typ, ComType::PropVariant) =>
        {
            Ok(ResultConversion::PropVariant)
        }
        (ComOwnership::ExcepInfoOwned, Cleanup::ExcepInfoClear)
            if matches!(typ, ComType::ExcepInfo) =>
        {
            Ok(ResultConversion::ExcepInfo)
        }
        (ComOwnership::StatStgOwned, Cleanup::StatStgClear) if matches!(typ, ComType::StatStg) => {
            Ok(ResultConversion::StatStg)
        }
        (ownership, cleanup) => Err(format!(
            "{}: unsupported projected ownership {ownership:?} with cleanup {cleanup:?}",
            param.name()
        )),
    }
}

fn detect_interop_target(
    validated: &ValidatedComInterface<'_>,
    winmd_paths: &str,
) -> Result<Option<(String, String, String)>, String> {
    let meta = validated.metadata();
    if !meta.interface.name.ends_with("Interop")
        || !validated.semantic().methods().iter().any(|method| {
            method.name() == "GetForWindow" && method.dynamic_iid_contract().is_some()
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

fn project_string_encoding(encoding: SemanticStringEncoding) -> StringEncoding {
    match encoding {
        SemanticStringEncoding::Utf16 => StringEncoding::Wide,
        SemanticStringEncoding::Ansi => StringEncoding::Ansi,
    }
}

fn is_scalar_in_out(typ: &ComType) -> bool {
    matches!(
        typ,
        ComType::Primitive(_)
            | ComType::NativeIsize
            | ComType::NativeUsize
            | ComType::Win32Bool
            | ComType::HResult
            | ComType::Enum { .. }
            | ComType::ScalarAlias { .. }
            | ComType::Bstr
            | ComType::NativePod { .. }
            | ComType::NativePodPointer { .. }
    )
}

fn is_supported_direct_return(typ: &ComType) -> bool {
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

fn semantic_type_name(
    semantic: &SemanticComInterface,
    abi_type: super::model::ids::TypeId,
) -> String {
    semantic
        .type_definition(abi_type)
        .map(semantic_type_definition_name)
        .unwrap_or_else(|_| format!("type#{}", abi_type.index()))
}

fn semantic_type_definition_name(definition: &ComTypeDefinition) -> String {
    definition.native_name().map_or_else(
        || format!("{:?}", definition.abi()),
        |name| format!("{}.{}", name.namespace(), name.name()),
    )
}

pub(super) fn format_guid(bytes: &[u8; 16]) -> String {
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn input_arity(method: &ProjectedComMethod) -> usize {
    method
        .params
        .iter()
        .filter(|param| param.surface_input)
        .count()
}

fn group_overloads(
    methods: Vec<ProjectedComMethod>,
    interface_name: &str,
) -> Result<Vec<ProjectedComMethod>, String> {
    let mut order = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, method) in methods.iter().enumerate() {
        groups
            .entry(method.camel_name.clone())
            .or_insert_with(|| {
                order.push(method.camel_name.clone());
                Vec::new()
            })
            .push(index);
    }

    let mut overloads = vec![None; methods.len()];
    for name in &order {
        let indices = &groups[name];
        if indices.len() < 2 {
            continue;
        }
        for &index in indices {
            let method = &methods[index];
            if method.kind != ProjectedComMethodKind::Normal {
                return Err(format!(
                    "{interface_name}.{name}: cannot project {} overloads sharing the name `{name}` \
                     because at least one is a synthesized/dynamic-IID method, which is not a \
                     safely dispatchable JS shape",
                    indices.len()
                ));
            }
            if method.string_buffer.is_some() || !method.typed_buffers.is_empty() {
                return Err(format!(
                    "{interface_name}.{name}: cannot project {} overloads sharing the name `{name}` \
                     because at least one uses a projected buffer, which is not a \
                     safely dispatchable JS shape",
                    indices.len()
                ));
            }
        }

        let mut arity_buckets: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for &index in indices {
            arity_buckets
                .entry(input_arity(&methods[index]))
                .or_default()
                .push(index);
        }

        for (arity, bucket) in arity_buckets {
            if bucket.len() == 1 {
                overloads[bucket[0]] = Some(OverloadInfo {
                    public_name: name.clone(),
                    impl_name: format!("_{name}_{}", methods[bucket[0]].vtable_index),
                    dispatch: OverloadDispatch::Arity,
                });
                continue;
            }
            let key_param_index = (0..arity).find(|&key_index| {
                let mut seen: Vec<DispatchShape> = Vec::new();
                bucket.iter().all(|&index| {
                    let (_, param) = input_params_of(&methods[index])[key_index];
                    match dispatch_shape(&param.typ) {
                        Some(shape) if !seen.contains(&shape) => {
                            seen.push(shape);
                            true
                        }
                        _ => false,
                    }
                })
            });
            let Some(key_param_index) = key_param_index else {
                return Err(format!(
                    "{interface_name}.{name}: {} overloads share arity {arity} but no parameter \
                     position has mutually distinguishable JS shapes (arity/type/category); \
                     overload dispatch cannot be safely generated",
                    bucket.len()
                ));
            };
            for &index in &bucket {
                let (_, param) = input_params_of(&methods[index])[key_param_index];
                let shape =
                    dispatch_shape(&param.typ).expect("validated distinguishable shape above");
                overloads[index] = Some(OverloadInfo {
                    public_name: name.clone(),
                    impl_name: format!("_{name}_{}", methods[index].vtable_index),
                    dispatch: OverloadDispatch::ArityAndShape {
                        key_param_index,
                        shape,
                    },
                });
            }
        }
    }

    Ok(methods
        .into_iter()
        .zip(overloads)
        .map(|(mut method, overload)| {
            method.overload = overload;
            method
        })
        .collect())
}

fn input_params_of(method: &ProjectedComMethod) -> Vec<(usize, &ProjectedComParam)> {
    method
        .params
        .iter()
        .enumerate()
        .filter(|(_, param)| param.surface_input)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_format_preserves_canonical_semantic_identity() {
        assert_eq!(
            format_guid(&[
                0x3a, 0x3d, 0xcd, 0x6c, 0x3e, 0xab, 0x43, 0xdc, 0xbc, 0xde, 0x45, 0x67, 0x1c, 0xe8,
                0x00, 0xc8,
            ]),
            "3a3dcd6c-3eab-43dc-bcde-45671ce800c8"
        );
    }

    #[test]
    fn dynamic_iid_projection_preserves_explicit_parameter_indices() {
        assert_eq!(
            project_dynamic_method_kind("Resolve", Some((3, 1)), None),
            ProjectedComMethodKind::CallerSuppliedDynamicIid {
                iid_param_index: 3,
                output_param_index: 1,
            }
        );
        assert_eq!(
            project_dynamic_method_kind(
                "GetForWindow",
                Some((2, 5)),
                Some("11111111-2222-3333-4444-555555555555"),
            ),
            ProjectedComMethodKind::SynthesizedGetForWindow {
                iid_param_index: 2,
                output_param_index: 5,
                target_iid: "11111111-2222-3333-4444-555555555555".into(),
            }
        );
    }

    #[test]
    fn validated_semantic_projection_only_diverges_for_pod_upgrades() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let meta = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .unwrap();

        let validated = model::validate_interface(&meta).unwrap();
        let mut semantic = project_validated_interface(&validated, &winmd).unwrap();
        let mut legacy = legacy_diagnostics::project_com_interface_for_test(&meta, &winmd).unwrap();
        let semantic_clip = semantic
            .methods
            .iter()
            .find(|method| method.name == "SetThumbnailClip")
            .unwrap();
        let legacy_clip = legacy
            .methods
            .iter()
            .find(|method| method.name == "SetThumbnailClip")
            .unwrap();
        assert!(matches!(
            semantic_clip.params[1].typ,
            ComType::NativePodPointer { .. }
        ));
        assert_eq!(legacy_clip.params[1].typ, ComType::RawPointer);

        for name in ["ThumbBarAddButtons", "ThumbBarUpdateButtons"] {
            let semantic_method = semantic
                .methods
                .iter()
                .find(|method| method.name == name)
                .unwrap();
            assert!(matches!(
                semantic_method
                    .params
                    .iter()
                    .find(|param| param.name == "pButton")
                    .unwrap()
                    .typ,
                ComType::TypedBuffer { .. }
            ));
        }

        semantic.methods.retain(|method| {
            !matches!(
                method.name.as_str(),
                "SetThumbnailClip" | "ThumbBarAddButtons" | "ThumbBarUpdateButtons"
            )
        });
        legacy.methods.retain(|method| {
            !matches!(
                method.name.as_str(),
                "SetThumbnailClip" | "ThumbBarAddButtons" | "ThumbBarUpdateButtons"
            )
        });
        assert_eq!(semantic, legacy);
    }

    #[test]
    fn shell_link_get_path_projects_nullable_in_out_pod() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let meta = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "IShellLinkW",
        )
        .unwrap();
        let validated = model::validate_interface(&meta).unwrap();
        let projected = project_validated_interface(&validated, &winmd).unwrap();
        let get_path = projected
            .methods
            .iter()
            .find(|method| method.name == "GetPath")
            .unwrap();
        let pfd = get_path
            .params
            .iter()
            .find(|param| param.name == "pfd")
            .unwrap();
        assert!(pfd.nullable);
        assert!(matches!(pfd.typ, ComType::NativePodPointer { .. }));
    }

    #[test]
    fn bstr_inputs_outputs_and_replacements_project_as_strings() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let meta = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.Web.InternetExplorer",
            "ITargetNotify2",
        )
        .unwrap();
        let projected = project_com_interface(&meta, &winmd).unwrap();
        let method = projected
            .methods
            .iter()
            .find(|method| method.name == "GetOptionString")
            .unwrap();
        let (index, replacement) = method
            .params
            .iter()
            .enumerate()
            .find(|(_, parameter)| parameter.direction == ComParamDirection::InOut)
            .unwrap();
        assert_eq!(replacement.typ, ComType::Bstr);
        assert!(replacement.surface_input);
        assert!(replacement.surface_result);
        assert!(!replacement.nullable);
        assert!(method.results.iter().any(|result| {
            result.source == ResultSource::Param(index)
                && result.conversion == ResultConversion::Bstr
        }));

        let input_meta = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.TaskScheduler",
            "IExecAction2",
        )
        .unwrap();
        let input_projected = project_com_interface(&input_meta, &winmd).unwrap();
        assert!(input_projected.methods.iter().any(|method| {
            method.params.iter().any(|parameter| {
                parameter.direction == ComParamDirection::In
                    && parameter.typ == ComType::Bstr
                    && parameter.surface_input
                    && !parameter.surface_result
            })
        }));
    }

    #[test]
    fn documented_bstr_output_overrides_project_or_fail_on_later_contracts() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let photo = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.Media.PictureAcquisition",
            "IPhotoAcquireDeviceSelectionDialog",
        )
        .unwrap();
        let photo = model::validate_interface(&photo).unwrap();
        let error = project_validated_interface(&photo, &winmd).unwrap_err();
        assert!(error.contains("pnDeviceType"), "{error}");

        let recorder = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.Storage.Imapi",
            "IDiscRecorder",
        )
        .unwrap();
        let recorder = model::validate_interface(&recorder).unwrap();
        let projected = project_validated_interface(&recorder, &winmd).unwrap();
        let method = projected
            .methods
            .iter()
            .find(|method| method.name == "GetDisplayNames")
            .unwrap();
        let outputs = method
            .params
            .iter()
            .filter(|parameter| {
                parameter.direction == ComParamDirection::Out && parameter.typ == ComType::Bstr
            })
            .count();
        assert_eq!(outputs, 3);
        assert!(method.params.iter().all(|parameter| {
            parameter.typ != ComType::Bstr
                || parameter.direction != ComParamDirection::Out
                || (!parameter.surface_input && !parameter.nullable)
        }));
    }

    #[test]
    fn scalar_bstr_pointer_inputs_remain_fail_closed() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let mut meta = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "IFileSearchBand",
        )
        .unwrap();
        let method_index = meta
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .position(|method| method.metadata_name == "SetSearchParameters")
            .unwrap();
        meta.raw_methods = Some(vec![
            meta.raw_methods.as_ref().unwrap()[method_index].clone(),
        ]);
        meta.interface.methods = vec![meta.interface.methods[method_index].clone()];
        meta.own_methods_start = 0;
        let error = project_com_interface(&meta, &winmd).unwrap_err();
        assert!(
            error.contains("BSTR") || error.contains("string"),
            "{error}"
        );

        let mut nested = meta;
        nested.raw_methods.as_mut().unwrap()[0].params[0]
            .typ
            .pointer_depth = 2;
        let error = project_com_interface(&nested, &winmd).unwrap_err();
        assert!(
            error.contains("BSTR") || error.contains("string"),
            "{error}"
        );
    }

    #[test]
    fn native_pod_surface_names_reject_namespace_collisions() {
        let architecture = NativePodArchitectureLayout {
            size: 4,
            alignment: 4,
            fields: Vec::new(),
        };
        let pod = |namespace: &str| ComType::NativePod {
            layout: NativePodLayout {
                namespace: namespace.into(),
                name: "COLLISION".into(),
                initializers: Vec::new(),
                x86: architecture.clone(),
                x64: architecture.clone(),
                arm64: architecture.clone(),
            },
        };
        let method = |name: &str, typ: ComType, slot| ProjectedComMethod {
            name: name.into(),
            camel_name: name.into(),
            vtable_index: slot,
            params: vec![ProjectedComParam {
                name: "value".into(),
                typ,
                direction: ComParamDirection::In,
                surface_input: true,
                surface_result: false,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: Vec::new(),
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        };
        let interface = ProjectedComInterface {
            name: "ITest".into(),
            namespace: "Tests".into(),
            iid: "00000000-0000-0000-c000-000000000046".into(),
            is_iunknown_rooted: true,
            methods: vec![
                method("first", pod("Contoso.One"), 3),
                method("second", pod("Contoso.Two"), 4),
            ],
            activation: ActivationPlan::None,
            referenced_enums: Vec::new(),
        };

        let error = validate_projected_surface_names(&interface).unwrap_err();
        assert!(error.contains("Contoso.One.COLLISION"));
        assert!(error.contains("Contoso.Two.COLLISION"));
    }

    #[test]
    fn bind_opts_initializer_requires_exact_cross_architecture_layout() {
        let bind_opts = |size, alignment, offset| NativePodArchitectureLayout {
            size,
            alignment,
            fields: vec![NativePodField {
                name: "cbStruct".into(),
                offset,
                count: 1,
                typ: NativePodFieldType::Scalar(NativePodScalar::U32),
            }],
        };
        let initializers = native_pod_initializers(
            "Windows.Win32.System.Com",
            "BIND_OPTS",
            &bind_opts(16, 4, 0),
            &bind_opts(16, 4, 0),
            &bind_opts(16, 4, 0),
        )
        .unwrap();
        assert_eq!(
            initializers,
            [NativePodInitializer::SizeOfLayout {
                field: "cbStruct".into()
            }]
        );

        let error = native_pod_initializers(
            "Windows.Win32.System.Com",
            "BIND_OPTS",
            &bind_opts(16, 4, 4),
            &bind_opts(16, 4, 0),
            &bind_opts(16, 4, 0),
        )
        .unwrap_err();
        assert!(error.contains("offset 0"), "{error}");

        assert!(
            native_pod_initializers(
                "Contoso",
                "BIND_OPTS",
                &bind_opts(16, 4, 4),
                &bind_opts(16, 4, 4),
                &bind_opts(16, 4, 4),
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn specialized_buffer_rendering_rejects_uncomposable_plans() {
        let string = StringBufferPlan {
            buffer_param_index: 0,
            count_param_index: 1,
            encoding: StringEncoding::Wide,
            optional_param_indices: Vec::new(),
        };
        let input = TypedBufferPlan {
            buffer_param_index: 2,
            element: ComType::Primitive(ComPrimitive::U8),
            relation: TypedBufferRelation::Input {
                count_param_index: 3,
                actual_length_param_index: None,
                unit: ProjectedBufferCountUnit::Bytes,
            },
        };
        assert!(
            validate_buffer_plan_composition("Mixed", Some(&string), &[input.clone()]).is_err()
        );

        let two_call = TypedBufferPlan {
            buffer_param_index: 0,
            element: ComType::Primitive(ComPrimitive::U8),
            relation: TypedBufferRelation::CallerOutput {
                capacity_param_index: 1,
                actual_length_param_index: Some(2),
                unit: ProjectedBufferCountUnit::Bytes,
                sizing: TypedBufferSizing::TwoCall { max_retries: 2 },
            },
        };
        assert!(validate_buffer_plan_composition("One", None, &[two_call.clone()]).is_ok());
        assert!(validate_buffer_plan_composition("Many", None, &[two_call, input]).is_err());
    }

    #[test]
    fn dynamic_iid_rejects_hidden_string_buffers() {
        let string = StringBufferPlan {
            buffer_param_index: 0,
            count_param_index: 1,
            encoding: StringEncoding::Wide,
            optional_param_indices: Vec::new(),
        };
        assert!(
            validate_dynamic_iid_buffers("Resolve", true, Some(&string), &[])
                .unwrap_err()
                .contains("hidden string/buffer")
        );
        assert!(validate_dynamic_iid_buffers("Resolve", false, Some(&string), &[]).is_ok());
        let fixed = TypedBufferPlan {
            buffer_param_index: 0,
            element: ComType::Primitive(ComPrimitive::U8),
            relation: TypedBufferRelation::CallerOutput {
                capacity_param_index: 1,
                actual_length_param_index: Some(2),
                unit: ProjectedBufferCountUnit::Bytes,
                sizing: TypedBufferSizing::FixedCapacity,
            },
        };
        assert!(validate_buffer_plan_composition("Fixed", None, &[fixed]).is_ok());
    }

    #[test]
    fn projected_surface_names_reject_cross_category_and_qualified_collisions() {
        let mut interface = ProjectedComInterface {
            name: "ITest".into(),
            namespace: "Tests".into(),
            iid: "00000000-0000-0000-c000-000000000046".into(),
            is_iunknown_rooted: true,
            methods: Vec::new(),
            activation: ActivationPlan::None,
            referenced_enums: vec![
                ProjectedComEnum {
                    namespace: "Contoso.One".into(),
                    name: "MODE".into(),
                    underlying: ComEnumUnderlying::I32,
                    members: Vec::new(),
                },
                ProjectedComEnum {
                    namespace: "Contoso.Two".into(),
                    name: "MODE".into(),
                    underlying: ComEnumUnderlying::I32,
                    members: Vec::new(),
                },
            ],
        };
        assert!(validate_projected_surface_names(&interface).is_err());

        interface.referenced_enums.pop();
        interface.methods.push(ProjectedComMethod {
            name: "useMode".into(),
            camel_name: "useMode".into(),
            vtable_index: 3,
            params: vec![ProjectedComParam {
                name: "mode".into(),
                typ: ComType::ScalarAlias {
                    namespace: "Contoso.Values".into(),
                    name: "MODE".into(),
                    underlying: ComScalarRepr::Primitive(ComPrimitive::I32),
                },
                direction: ComParamDirection::In,
                surface_input: true,
                surface_result: false,
                nullable: false,
            }],
            return_convention: ComReturnConvention::HResult,
            results: Vec::new(),
            string_buffer: None,
            typed_buffers: Vec::new(),
            shared_counts: Vec::new(),
            kind: ProjectedComMethodKind::Normal,
            doc: None,
            overload: None,
        });
        assert!(validate_projected_surface_names(&interface).is_err());
    }

    #[test]
    fn coclass_rejects_conflicting_enum_filenames() {
        let interface = |name: &str, namespace: &str| ProjectedComInterface {
            name: name.into(),
            namespace: "Tests".into(),
            iid: "00000000-0000-0000-c000-000000000046".into(),
            is_iunknown_rooted: true,
            methods: Vec::new(),
            activation: ActivationPlan::None,
            referenced_enums: vec![ProjectedComEnum {
                namespace: namespace.into(),
                name: "STATE".into(),
                underlying: ComEnumUnderlying::I32,
                members: Vec::new(),
            }],
        };
        assert!(
            validate_coclass_enum_files(
                "Tests",
                "Class",
                &[
                    interface("IFirst", "Contoso.One"),
                    interface("ISecond", "Contoso.Two"),
                ],
            )
            .is_err()
        );
    }
}
