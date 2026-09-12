// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet};

use crate::win32_contracts::{
    AggregateContract, FieldContract, FunctionPolicy, Registry, validate_call_contract_structure,
};
use crate::win32_metadata::{
    RawApis, RawArchitectures, RawBaseType, RawBufferSize, RawCallingConvention, RawDirection,
    RawFunction, RawLayoutKind, RawNamedKind, RawNativeLayout, RawNativeLayoutSet, RawPacking,
    RawScalar, RawStatusSemantics, RawStringEncoding, RawType,
};

use super::ir::{
    AbiType, BufferContract, CallingConvention, Cleanup, Constness, Direction, EnumDefinition,
    EnumMember, EnumUnderlying, FunctionContract, InputPredicate, NativeAggregateKind,
    NativeArchitectureLayout, NativeField, NativeFieldType, NativeLayout, NativeScalar,
    OutputAction, ParameterContract, ResourceEffect, Scalar, StringEncoding, SuccessRule,
    ValueType,
};

pub(super) fn validate_apis(raw: &RawApis) -> (Vec<FunctionContract>, Vec<(String, String)>) {
    let name_counts =
        raw.functions
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, function| {
                *counts.entry(&function.name).or_default() += 1;
                counts
            });
    let mut enum_namespaces = BTreeMap::<String, BTreeSet<String>>::new();
    for function in &raw.functions {
        if let Ok(definitions) = enum_definitions(function) {
            for definition in definitions {
                enum_namespaces
                    .entry(definition.name)
                    .or_default()
                    .insert(definition.namespace);
            }
        }
    }
    let ambiguous_enums = enum_namespaces
        .into_iter()
        .filter_map(|(name, namespaces)| (namespaces.len() > 1).then_some(name))
        .collect::<BTreeSet<_>>();
    let mut functions = Vec::new();
    let mut omitted = Vec::new();
    for function in &raw.functions {
        if name_counts
            .get(function.name.as_str())
            .copied()
            .unwrap_or(0)
            > 1
        {
            omitted.push((
                format!(
                    "{}.{}::{}",
                    function.namespace, function.container, function.name
                ),
                "unresolved metadata overload or architecture collision".into(),
            ));
            continue;
        }
        if enum_definitions(function).is_ok_and(|definitions| {
            definitions
                .iter()
                .any(|definition| ambiguous_enums.contains(&definition.name))
        }) {
            omitted.push((
                format!(
                    "{}.{}::{}",
                    function.namespace, function.container, function.name
                ),
                "referenced enum simple name is ambiguous across namespaces".into(),
            ));
            continue;
        }
        match validate_function(function) {
            Ok(function) => functions.push(function),
            Err(reason) => omitted.push((
                format!(
                    "{}.{}::{}",
                    function.namespace, function.container, function.name
                ),
                reason,
            )),
        }
    }
    (functions, omitted)
}

pub(super) fn validate_function(raw: &RawFunction) -> Result<FunctionContract, String> {
    let policy = Registry::builtin()?.function_policy(raw)?;
    let expanded = policy.apply_buffers(raw)?;
    let raw = &expanded;
    let calling_convention = match raw.calling_convention {
        RawCallingConvention::System => CallingConvention::System,
        RawCallingConvention::Cdecl => CallingConvention::Cdecl,
        RawCallingConvention::Unsupported => {
            return Err("unsupported native calling convention".into());
        }
    };
    if raw.variadic {
        return Err("variadic flat Win32 exports are unsupported".into());
    }
    if !raw.architectures.x64 || !raw.architectures.arm64 {
        return Err("export is not available on both x64 and ARM64".into());
    }
    validate_module(&raw.dll)?;
    let subsystem = policy.subsystem;
    if raw.entry_point.is_empty() || raw.entry_point.as_bytes().contains(&0) {
        return Err("entry point is empty or contains NUL".into());
    }

    let (return_type, mut return_abi, return_cleanup) = map_return(raw, &policy)?;
    let return_aggregate = match &return_type {
        Some(ValueType::NativeStruct { layout }) => {
            return_abi = None;
            Some(layout.clone())
        }
        _ => None,
    };
    let mut parameters = raw
        .parameters
        .iter()
        .enumerate()
        .map(|(parameter_index, parameter)| {
            if parameter.reserved && parameter.direction != RawDirection::In {
                return Err(format!(
                    "reserved parameter `{}` must be input-only",
                    parameter.name
                ));
            }
            let buffer = parameter
                .buffer
                .as_ref()
                .map(|buffer| -> Result<BufferContract, String> {
                    let (element_size, element_alignment) = match buffer.size {
                        RawBufferSize::ByteCountParam(_)
                            if parameter.typ.pointer_depth == 1
                                || (parameter.typ.pointer_depth == 0
                                    && matches!(
                                        parameter.typ.base,
                                        RawBaseType::Named {
                                            kind: RawNamedKind::DataPointer
                                                | RawNamedKind::StringPointer { .. },
                                            ..
                                        }
                                    )) =>
                        {
                            let alignment = map_buffer_element(&buffer.element)
                                .map(|(_, alignment)| alignment)
                                .unwrap_or(8);
                            (1, alignment)
                        }
                        RawBufferSize::ByteCountParam(_) => {
                            return Err(
                                "byte-sized native buffer does not have one data indirection"
                                    .into(),
                            );
                        }
                        _ => map_buffer_element(&buffer.element)?,
                    };
                    let (count_parameter, constant_count, count_is_bytes) = match buffer.size {
                        RawBufferSize::ElementCountParam(index) => (Some(index), None, false),
                        RawBufferSize::ByteCountParam(index) => (Some(index), None, true),
                        RawBufferSize::Constant(count) => (None, Some(count), false),
                        RawBufferSize::Unknown => {
                            return Err("native buffer has no complete size contract".into());
                        }
                    };
                    Ok(BufferContract {
                        count_parameter,
                        constant_count,
                        count_is_bytes,
                        element_size,
                        element_alignment,
                    })
                })
                .transpose()?;
            let reserved_pointer = parameter.reserved
                && (parameter.typ.pointer_depth > 0
                    || matches!(
                        parameter.typ.base,
                        RawBaseType::Named {
                            kind:
                                RawNamedKind::DataPointer
                                | RawNamedKind::StringPointer { .. }
                                | RawNamedKind::FunctionPointer
                                | RawNamedKind::ComInterface { .. },
                            ..
                        }
                    ));
            let nullable_void = parameter.nullable
                && parameter.direction == RawDirection::In
                && parameter.typ.pointer_depth == 1
                && matches!(parameter.typ.base, RawBaseType::Void);
            let (typ, abi, mut cleanup) = if nullable_void {
                (ValueType::NullPointer, AbiType::Pointer, Cleanup::None)
            } else if buffer.is_some() || reserved_pointer {
                (
                    ValueType::DataPointer,
                    AbiType::Pointer,
                    if reserved_pointer {
                        Cleanup::None
                    } else {
                        parameter
                            .free_with
                            .as_deref()
                            .map(parse_cleanup)
                            .transpose()?
                            .unwrap_or(Cleanup::None)
                    },
                )
            } else {
                map_parameter_type(
                    &parameter.typ,
                    parameter.direction,
                    parameter.free_with.as_deref(),
                )?
            };
            if matches!(&typ, ValueType::Handle { .. })
                && parameter.typ.pointer_depth == 1
                && matches!(parameter.direction, RawDirection::Out | RawDirection::InOut)
            {
                if cleanup == Cleanup::None {
                    cleanup = policy.output_cleanup(parameter_index).ok_or_else(|| {
                        format!(
                            "handle output `{}` has no function-specific ownership and cleanup contract",
                            parameter.name
                        )
                    })?;
                }
                if parameter.direction == RawDirection::InOut {
                    return Err(format!(
                        "owning InOut handle `{}` is unsupported until replacement ownership is modeled",
                        parameter.name
                    ));
                }
            }
            let resource_cleanup = handle_resource_cleanup(&parameter.typ);
            if cleanup != Cleanup::None && resource_cleanup != Cleanup::None && cleanup != resource_cleanup {
                return Err(format!("ownership evidence conflicts with native cleanup for `{}`",parameter.name));
            }
            let consumes_resource = if let Some(expected) = policy.consumed_input(parameter_index) {
                if parameter.direction != RawDirection::In || parameter.typ.pointer_depth != 0
                    || !matches!(typ,ValueType::Handle { .. }) || expected != resource_cleanup {
                    return Err(format!("consuming evidence conflicts with native handle `{}`",parameter.name));
                }
                true
            } else { false };
            let supported_double_null = matches!(
                (&typ, parameter.direction),
                (ValueType::StringPointer(_), RawDirection::In)
                    | (ValueType::DataPointer, RawDirection::Out)
            ) && (matches!(&typ, ValueType::StringPointer(_)) || buffer.is_some());
            if parameter.null_null_terminated && !supported_double_null {
                return Err(format!(
                    "NullNullTerminated parameter `{}` must be an input string pointer or an explicit output buffer",
                    parameter.name
                ));
            }
            Ok(ParameterContract {
                name: parameter.name.clone(),
                native_name: raw_native_name(&parameter.typ),
                nullable: (parameter.nullable || reserved_pointer)
                    && matches!(
                        &typ,
                        ValueType::Handle { .. }
                            | ValueType::DataPointer
                            | ValueType::StringPointer(_)
                            | ValueType::FunctionPointer
                            | ValueType::NativeStructPointer { .. }
                            | ValueType::NativeUnionPointer { .. }
                            | ValueType::ScalarPointer { .. }
                            | ValueType::GuidPointer
                            | ValueType::NullPointer
                            | ValueType::ComInterface { .. }
                            | ValueType::StringPointerPointer(_)
                    ),
                typ,
                abi,
                pointer_depth: parameter.typ.pointer_depth,
                constness: map_constness(parameter.typ.constness),
                direction: map_direction(parameter.direction),
                reserved: parameter.reserved,
                null_null_terminated: parameter.null_null_terminated,
                cleanup,
                consumes_resource,
                resource_cleanup,
                buffer,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    for (index, parameter) in parameters.iter_mut().enumerate() {
        if policy.mutable_string(index) {
            if !matches!(parameter.typ, ValueType::StringPointer(_)) {
                return Err("mutable-string evidence requires a native string pointer".into());
            }
            parameter.direction = Direction::In;
        }
    }
    validate_buffers(&parameters)?;
    if let Some(parameter) = parameters.iter().find(|parameter| {
        parameter.pointer_depth == 0
            && parameter.direction == Direction::Out
            && parameter.buffer.is_none()
            && matches!(
                parameter.typ,
                ValueType::Scalar(_) | ValueType::Enum { .. } | ValueType::Handle { .. }
            )
    }) {
        return Err(format!(
            "by-value parameter `{}` cannot use an output-only contract",
            parameter.name
        ));
    }
    let enums = enum_definitions(raw)?;
    let success_rule = match raw.return_status {
        RawStatusSemantics::ZeroIsSuccess => SuccessRule::ReturnZero,
        RawStatusSemantics::SignedNonNegativeIsSuccess => SuccessRule::SignedNonNegative,
        RawStatusSemantics::None
            if matches!(&return_type, Some(ValueType::Scalar(Scalar::Bool32))) =>
        {
            SuccessRule::ReturnNonZero
        }
        RawStatusSemantics::None
            if return_cleanup != Cleanup::None
                && matches!(&return_type, Some(ValueType::Handle { .. })) =>
        {
            SuccessRule::ReturnValidHandle
        }
        RawStatusSemantics::None if return_cleanup != Cleanup::None => SuccessRule::ReturnNonNull,
        RawStatusSemantics::None => SuccessRule::Always,
    };

    let contract = FunctionContract {
        namespace: raw.namespace.clone(),
        container: raw.container.clone(),
        name: raw.name.clone(),
        dll: raw.dll.clone(),
        entry_point: raw.entry_point.clone(),
        parameters,
        return_type,
        return_abi,
        return_aggregate,
        return_native_name: raw_native_name(&raw.return_type),
        return_pointer_depth: raw.return_type.pointer_depth,
        return_constness: map_constness(raw.return_type.constness),
        return_cleanup,
        return_is_status: raw.return_status != RawStatusSemantics::None,
        success_rule,
        capture_last_error: raw.supports_last_error,
        calling_convention,
        subsystem,
        enums,
        call_contract: policy.call_contract(),
    };
    validate_call_contract(&contract)?;
    Ok(contract)
}

pub(super) fn validate_call_contract(function: &FunctionContract) -> Result<(), String> {
    let contract = &function.call_contract;
    validate_call_contract_structure(contract)?;
    let parameter = |index: usize| {
        function.parameters.get(index).ok_or_else(|| {
            format!("win32.contract-shape-conflict: missing native parameter {index}")
        })
    };
    for rule in &contract.outputs {
        let output = parameter(rule.parameter)?;
        if output.direction == Direction::In
            || output.pointer_depth == 0
            || output.buffer.is_some()
            || !matches!(
                output.typ,
                ValueType::Scalar(_)
                    | ValueType::Enum { .. }
                    | ValueType::Handle { .. }
                    | ValueType::DataPointer
            )
        {
            return Err("win32.contract-shape-conflict: rule requires a native output slot".into());
        }
        if let Some(value) = rule.when.return_value {
            let mask = function.return_abi.and_then(integer_mask).ok_or(
                "win32.contract-shape-conflict: return condition requires native integer bits",
            )?;
            if value & !mask != 0 {
                return Err(
                    "win32.contract-shape-conflict: return condition exceeds native width".into(),
                );
            }
        }
        for predicate in &rule.when.inputs {
            match predicate {
                InputPredicate::BitsIn {
                    parameter: index,
                    mask,
                    ..
                } => {
                    let input = parameter(*index)?;
                    if input.direction == Direction::Out
                        || input.buffer.is_some()
                        || !matches!(
                            (input.direction, input.pointer_depth),
                            (Direction::In, 0) | (Direction::InOut, 1)
                        )
                        || !matches!(
                            input.typ,
                            ValueType::Scalar(_)
                                | ValueType::Enum { .. }
                                | ValueType::Handle { .. }
                        )
                        || !integer_mask(input.abi).is_some_and(|width| mask & !width == 0)
                    {
                        return Err("win32.contract-shape-conflict: bits predicate requires a width-compatible native integer input".into());
                    }
                }
                InputPredicate::HandleIn {
                    parameter: index, ..
                } => {
                    let input = parameter(*index)?;
                    if input.abi != AbiType::Handle
                        || !matches!(input.typ, ValueType::Handle { .. })
                        || input.buffer.is_some()
                        || !matches!(
                            (input.direction, input.pointer_depth),
                            (Direction::In, 0) | (Direction::InOut, 1)
                        )
                    {
                        return Err("win32.contract-shape-conflict: handle-in requires a native handle input".into());
                    }
                }
                InputPredicate::NullOrEmpty {
                    parameter: index,
                    element_width,
                } => {
                    let input = parameter(*index)?;
                    let encoding_matches = matches!(
                        (&input.typ, element_width),
                        (ValueType::StringPointer(StringEncoding::Ansi), 1)
                            | (ValueType::StringPointer(StringEncoding::Wide), 2)
                    );
                    if input.direction != Direction::In
                        || input.pointer_depth != 0
                        || input.buffer.is_some()
                        || input.abi != AbiType::Pointer
                        || !encoding_matches
                    {
                        return Err("win32.contract-shape-conflict: null-or-empty requires a verified native string input and matching encoding width".into());
                    }
                }
            }
        }
        if let OutputAction::AliasInput { parameter: index } = rule.action {
            let input = parameter(index)?;
            if !matches!(output.typ, ValueType::Handle { .. })
                || output.abi != AbiType::Handle
                || output.direction != Direction::Out
                || output.pointer_depth != 1
                || output.cleanup == Cleanup::None
                || !matches!(input.typ, ValueType::Handle { .. })
                || input.abi != AbiType::Handle
                || input.direction != Direction::In
                || input.pointer_depth != 0
                || input.buffer.is_some()
                || input.cleanup != Cleanup::None
                || input.consumes_resource
                || input.resource_cleanup != output.cleanup
            {
                return Err("win32.contract-shape-conflict: alias requires an owned handle output and matching borrowed input resource".into());
            }
        }
    }
    for effect in &contract.resource_effects {
        let ResourceEffect::AddFileCompletionModes {
            handle_parameter,
            flags_parameter,
        } = *effect;
        let handle = parameter(handle_parameter)?;
        let flags = parameter(flags_parameter)?;
        if !matches!(handle.typ, ValueType::Handle { .. })
            || handle.abi != AbiType::Handle
            || handle.direction != Direction::In
            || handle.pointer_depth != 0
            || handle.buffer.is_some()
            || handle.cleanup != Cleanup::None
            || handle.consumes_resource
            || handle.resource_cleanup != Cleanup::CloseHandle
            || flags.abi != AbiType::U8
            || flags.direction != Direction::In
            || flags.pointer_depth != 0
            || flags.buffer.is_some()
            || !matches!(
                flags.typ,
                ValueType::Scalar(Scalar::U8)
                    | ValueType::Enum {
                        underlying: EnumUnderlying::U8,
                        ..
                    }
            )
        {
            return Err("win32.contract-shape-conflict: file completion state requires a borrowed CloseHandle resource and native U8 input flags".into());
        }
    }
    Ok(())
}

fn integer_mask(abi: AbiType) -> Option<u64> {
    match abi {
        AbiType::I8 | AbiType::U8 => Some(u8::MAX.into()),
        AbiType::I16 | AbiType::U16 => Some(u16::MAX.into()),
        AbiType::Bool32 | AbiType::I32 | AbiType::U32 => Some(u32::MAX.into()),
        AbiType::I64 | AbiType::U64 | AbiType::Handle => Some(u64::MAX),
        AbiType::F32 | AbiType::F64 | AbiType::Pointer | AbiType::FunctionPointer => None,
    }
}

fn map_return(
    function: &RawFunction,
    policy: &FunctionPolicy<'_>,
) -> Result<(Option<ValueType>, Option<AbiType>, Cleanup), String> {
    let raw = &function.return_type;
    if matches!(raw.base, RawBaseType::Void) && raw.pointer_depth == 0 {
        return Ok((None, None, Cleanup::None));
    }
    if raw.pointer_depth > 0
        && let Some(cleanup) = function.return_free_with.as_deref()
    {
        return Ok((
            Some(ValueType::DataPointer),
            Some(AbiType::Pointer),
            parse_cleanup(cleanup)?,
        ));
    }
    if let RawBaseType::Named {
        namespace,
        name,
        kind: RawNamedKind::Handle { cleanup },
    } = &raw.base
        && raw.pointer_depth == 0
    {
        let cleanup = if let Some(cleanup) = function.return_free_with.as_deref() {
            parse_cleanup(cleanup)?
        } else if cleanup.is_none() || policy.borrowed_return() {
            Cleanup::None
        } else if let Some(cleanup) = policy.owned_return() {
            if handle_resource_cleanup(raw) != cleanup {
                return Err("owned return evidence conflicts with native handle cleanup".into());
            }
            cleanup
        } else {
            return Err(format!(
                "direct handle return `{name}` has no verified ownership and success-sentinel contract"
            ));
        };
        return Ok((
            Some(ValueType::Handle {
                namespace: namespace.clone(),
                name: name.clone(),
            }),
            Some(AbiType::Handle),
            cleanup,
        ));
    }
    if let RawBaseType::Named {
        kind: RawNamedKind::FunctionPointer,
        ..
    } = &raw.base
        && raw.pointer_depth == 0
    {
        return Ok((
            Some(ValueType::FunctionPointer),
            Some(AbiType::FunctionPointer),
            Cleanup::None,
        ));
    }
    let (typ, abi, cleanup) =
        map_type(raw, RawDirection::In, function.return_free_with.as_deref())?;
    if matches!(
        typ,
        ValueType::DataPointer
            | ValueType::StringPointer(_)
            | ValueType::NativeStructPointer { .. }
            | ValueType::NativeUnionPointer { .. }
            | ValueType::ScalarPointer { .. }
            | ValueType::GuidPointer
            | ValueType::NullPointer
            | ValueType::ComInterface { .. }
            | ValueType::StringPointerPointer(_)
    ) && cleanup == Cleanup::None
    {
        return Err("pointer return lifetime and ownership are not modeled".into());
    }
    Ok((Some(typ), Some(abi), cleanup))
}

fn map_parameter_type(
    raw: &RawType,
    direction: RawDirection,
    free_with: Option<&str>,
) -> Result<(ValueType, AbiType, Cleanup), String> {
    let (typ, abi, mut cleanup) = map_type(raw, direction, free_with)?;
    if direction == RawDirection::In {
        cleanup = Cleanup::None;
    }
    Ok((typ, abi, cleanup))
}

fn map_type(
    raw: &RawType,
    direction: RawDirection,
    free_with: Option<&str>,
) -> Result<(ValueType, AbiType, Cleanup), String> {
    match (&raw.base, raw.pointer_depth) {
        (RawBaseType::Scalar(scalar), 0) => {
            let scalar = map_scalar(*scalar);
            Ok((ValueType::Scalar(scalar), scalar_abi(scalar), Cleanup::None))
        }
        (RawBaseType::Scalar(scalar), 1)
            if matches!(direction, RawDirection::Out | RawDirection::InOut) =>
        {
            let scalar = map_scalar(*scalar);
            Ok((ValueType::Scalar(scalar), scalar_abi(scalar), Cleanup::None))
        }
        (RawBaseType::Scalar(scalar), 1) if direction == RawDirection::In => {
            let scalar = map_scalar(*scalar);
            Ok((
                ValueType::ScalarPointer { scalar },
                AbiType::Pointer,
                Cleanup::None,
            ))
        }
        (
            RawBaseType::Named {
                namespace,
                name,
                kind:
                    RawNamedKind::Enum {
                        underlying,
                        members,
                        is_flags,
                    },
            },
            pointer_depth,
        ) if pointer_depth == 0
            || (pointer_depth == 1
                && matches!(direction, RawDirection::Out | RawDirection::InOut)) =>
        {
            let underlying = map_enum_underlying(*underlying)?;
            let _ = (members, is_flags);
            Ok((
                ValueType::Enum {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    underlying,
                },
                enum_abi(underlying),
                Cleanup::None,
            ))
        }
        (
            RawBaseType::Named {
                namespace,
                name,
                kind: RawNamedKind::Handle { cleanup: _ },
            },
            pointer_depth,
        ) if pointer_depth == 0
            || (pointer_depth == 1
                && matches!(direction, RawDirection::Out | RawDirection::InOut)) =>
        {
            let cleanup = if pointer_depth == 0 || direction == RawDirection::In {
                Cleanup::None
            } else {
                free_with
                    .map(parse_cleanup)
                    .transpose()?
                    .unwrap_or(Cleanup::None)
            };
            Ok((
                ValueType::Handle {
                    namespace: namespace.clone(),
                    name: name.clone(),
                },
                AbiType::Handle,
                cleanup,
            ))
        }

        (
            RawBaseType::Named {
                kind: RawNamedKind::DataPointer,
                ..
            },
            0,
        ) => Ok((
            ValueType::DataPointer,
            AbiType::Pointer,
            free_with
                .map(parse_cleanup)
                .transpose()?
                .unwrap_or(Cleanup::None),
        )),
        (
            RawBaseType::Named {
                kind: RawNamedKind::StringPointer { encoding },
                ..
            },
            0,
        ) => Ok((
            ValueType::StringPointer(match encoding {
                RawStringEncoding::Utf16 => StringEncoding::Wide,
                RawStringEncoding::Ansi => StringEncoding::Ansi,
            }),
            AbiType::Pointer,
            free_with
                .map(parse_cleanup)
                .transpose()?
                .unwrap_or(Cleanup::None),
        )),
        (
            RawBaseType::Named {
                kind: RawNamedKind::FunctionPointer,
                ..
            },
            0,
        ) if direction == RawDirection::In => {
            Err("managed callback thunks are not implemented".into())
        }
        (
            RawBaseType::Named {
                kind: RawNamedKind::FunctionPointer,
                ..
            },
            0,
        ) => Ok((
            ValueType::FunctionPointer,
            AbiType::FunctionPointer,
            Cleanup::None,
        )),
        (
            RawBaseType::Named {
                namespace,
                name,
                kind: RawNamedKind::NativeStruct { layout },
            },
            1,
        ) => {
            let layout = validate_native_layout(namespace, name, layout)?;
            let typ = match layout.kind {
                NativeAggregateKind::Struct => ValueType::NativeStructPointer { layout },
                NativeAggregateKind::Union => ValueType::NativeUnionPointer { layout },
            };
            Ok((typ, AbiType::Pointer, Cleanup::None))
        }
        (
            RawBaseType::Named {
                kind: RawNamedKind::NativeStruct { .. },
                ..
            },
            0,
        ) => {
            let RawBaseType::Named {
                namespace,
                name,
                kind: RawNamedKind::NativeStruct { layout },
            } = &raw.base
            else {
                unreachable!()
            };
            let layout = validate_native_layout(namespace, name, layout)?;
            if layout.kind != NativeAggregateKind::Struct
                || !native_layout_supports_by_value(&layout)
            {
                return Err("by-value native aggregate contains unsupported union fields".into());
            }
            Ok((
                ValueType::NativeStruct { layout },
                AbiType::Pointer,
                Cleanup::None,
            ))
        }
        (
            RawBaseType::Named {
                kind: RawNamedKind::Guid,
                ..
            },
            1,
        ) => Ok((ValueType::GuidPointer, AbiType::Pointer, Cleanup::None)),
        (
            RawBaseType::Named {
                name,
                kind: RawNamedKind::ComInterface { iid },
                ..
            },
            0,
        ) if direction == RawDirection::In => Ok((
            ValueType::ComInterface {
                name: name.clone(),
                iid: iid.clone(),
            },
            AbiType::Pointer,
            Cleanup::None,
        )),
        (
            RawBaseType::Named {
                kind: RawNamedKind::StringPointer { encoding },
                ..
            },
            1,
        ) if direction == RawDirection::In => Ok((
            ValueType::StringPointerPointer(match encoding {
                RawStringEncoding::Utf16 => StringEncoding::Wide,
                RawStringEncoding::Ansi => StringEncoding::Ansi,
            }),
            AbiType::Pointer,
            Cleanup::None,
        )),
        (
            RawBaseType::Named {
                namespace,
                name,
                kind,
            },
            _,
        ) => Err(format!(
            "unsupported {} {namespace}.{name} pointer depth {}",
            raw_named_kind_label(kind),
            raw.pointer_depth
        )),
        (RawBaseType::Unknown(reason), _) => Err(format!("native type is unknown: {reason}")),
        (RawBaseType::Void, _) => Err("void is not a parameter or pointer value".into()),
        (RawBaseType::Scalar(_), _) => Err(format!(
            "unsupported scalar pointer depth {}",
            raw.pointer_depth
        )),
    }
}

fn raw_named_kind_label(kind: &RawNamedKind) -> &'static str {
    match kind {
        RawNamedKind::Enum { .. } => "enum",
        RawNamedKind::Handle { .. } => "handle",
        RawNamedKind::StringPointer { .. } => "string pointer",
        RawNamedKind::DataPointer => "data pointer",
        RawNamedKind::FunctionPointer => "function pointer",
        RawNamedKind::Guid => "GUID",
        RawNamedKind::ComInterface { .. } => "COM interface",
        RawNamedKind::NativeStruct { .. } => "native aggregate",
        RawNamedKind::Unknown => "unknown native type",
    }
}

fn native_layout_supports_by_value(layout: &NativeLayout) -> bool {
    fn architecture(layout: &NativeArchitectureLayout) -> bool {
        layout.fields.iter().all(|field| match &field.typ {
            NativeFieldType::Union { .. } => false,
            NativeFieldType::Struct {
                layout,
                by_value_compatible,
                ..
            } => *by_value_compatible && architecture(layout),
            NativeFieldType::Scalar(_) | NativeFieldType::Guid => true,
            NativeFieldType::Pointer | NativeFieldType::Handle { .. } => false,
        })
    }
    layout.by_value_compatible
        && architecture(&layout.x86)
        && architecture(&layout.x64)
        && architecture(&layout.arm64)
}

#[derive(Debug, Clone, Copy)]
enum LayoutArchitecture {
    X86,
    X64,
    Arm64,
}

impl LayoutArchitecture {
    const fn pointer_size(self) -> usize {
        match self {
            Self::X86 => 4,
            Self::X64 | Self::Arm64 => 8,
        }
    }

    const fn supports(self, architectures: RawArchitectures) -> bool {
        match self {
            Self::X86 => architectures.x86,
            Self::X64 => architectures.x64,
            Self::Arm64 => architectures.arm64,
        }
    }
}

fn validate_native_layout(
    namespace: &str,
    name: &str,
    raw: &RawNativeLayoutSet,
) -> Result<NativeLayout, String> {
    if Registry::builtin()?
        .type_entry(namespace, name)
        .is_some_and(|entry| entry.aggregate.is_some())
        && raw.evidence.is_none()
    {
        return Err(format!("win32.missing-type-evidence: {namespace}.{name}"));
    }
    if raw.recursive {
        return Err(format!(
            "recursive by-value native layout {namespace}.{name}"
        ));
    }
    let mut visiting = BTreeSet::new();
    let x86 = compute_native_layout(raw, LayoutArchitecture::X86, &mut visiting)?;
    let x64 = compute_native_layout(raw, LayoutArchitecture::X64, &mut visiting)?;
    let arm64 = compute_native_layout(raw, LayoutArchitecture::Arm64, &mut visiting)?;
    let kind = raw
        .variants
        .first()
        .map(|variant| match variant.kind {
            RawLayoutKind::Sequential => NativeAggregateKind::Struct,
            RawLayoutKind::Union => NativeAggregateKind::Union,
            RawLayoutKind::Unknown => NativeAggregateKind::Struct,
        })
        .ok_or_else(|| format!("{namespace}.{name} has no native layout variants"))?;
    if raw.variants.iter().any(|variant| {
        matches!(
            (kind, variant.kind),
            (NativeAggregateKind::Struct, RawLayoutKind::Union)
                | (NativeAggregateKind::Union, RawLayoutKind::Sequential)
        )
    }) {
        return Err(format!(
            "architecture variants disagree on aggregate kind for {namespace}.{name}"
        ));
    }
    Ok(NativeLayout {
        namespace: namespace.into(),
        name: name.into(),
        kind,
        by_value_compatible: raw.variants.iter().all(|variant| {
            variant.packing == RawPacking::Default
                && variant.forced_alignment.is_none()
                && variant.kind == RawLayoutKind::Sequential
        }) && [&x86, &x64, &arm64].into_iter().all(|layout| {
            layout.fields.iter().all(|field| {
                !matches!(
                    field.typ,
                    NativeFieldType::Pointer | NativeFieldType::Handle { .. }
                )
            })
        }),
        x86,
        x64,
        arm64,
    })
}

fn compute_native_layout(
    raw: &RawNativeLayoutSet,
    architecture: LayoutArchitecture,
    visiting: &mut BTreeSet<String>,
) -> Result<NativeArchitectureLayout, String> {
    let candidates = raw
        .variants
        .iter()
        .filter(|variant| architecture.supports(variant.architectures))
        .collect::<Vec<_>>();
    let [variant] = candidates.as_slice() else {
        return Err(if candidates.is_empty() {
            format!("missing {architecture:?} native layout facts")
        } else {
            format!("ambiguous {architecture:?} native layout facts")
        });
    };
    let contract = Registry::builtin()?.layout_contract(raw)?;
    if let Some(contract) = contract {
        if contract
            .fields
            .iter()
            .any(|policy| !variant.fields.iter().any(|field| field.name == policy.name))
        {
            return Err("aggregate evidence refers to a missing native field".into());
        }
        if let Some(size_field) = &contract.size_field
            && !variant.fields.iter().any(|field| {
                field.name == *size_field
                    && field.typ.pointer_depth == 0
                    && field.typ.base == RawBaseType::Scalar(RawScalar::U32)
            })
        {
            return Err("aggregate size field is not a native u32".into());
        }
    }
    compute_native_layout_variant(variant, architecture, visiting, contract)
}

fn compute_native_layout_variant(
    raw: &RawNativeLayout,
    architecture: LayoutArchitecture,
    visiting: &mut BTreeSet<String>,
    contract: Option<&AggregateContract>,
) -> Result<NativeArchitectureLayout, String> {
    if raw.kind == RawLayoutKind::Unknown || raw.fields.is_empty() {
        return Err("native aggregate has unknown or empty layout".into());
    }
    let packing = match raw.packing {
        RawPacking::Default => 8usize,
        RawPacking::Explicit(value) if value.is_power_of_two() => usize::from(value),
        RawPacking::Explicit(value) => {
            return Err(format!("invalid native packing {value}"));
        }
    };
    let mut fields = Vec::with_capacity(raw.fields.len());
    let mut cursor = 0usize;
    let mut aggregate_alignment = 1usize;
    for raw_field in &raw.fields {
        if raw_field.bitfield {
            return Err(format!(
                "native bitfield `{}` requires dedicated accessors",
                raw_field.name
            ));
        }
        if raw_field.flexible_array {
            return Err(format!(
                "flexible native array `{}` requires a variable-size contract",
                raw_field.name
            ));
        }
        if let Some(forced_alignment) = raw.forced_alignment {
            if !forced_alignment.is_power_of_two() || forced_alignment > 8 {
                return Err(format!(
                    "unsupported forced native alignment {forced_alignment}"
                ));
            }
            aggregate_alignment = aggregate_alignment.max(forced_alignment);
        }
        let field_contract = contract
            .and_then(|c| c.fields.iter().find(|p| p.name == raw_field.name))
            .map(|p| p.contract);
        let (typ, element_size, element_alignment) =
            native_field_type(&raw_field.typ, architecture, visiting, field_contract)?;
        let count = raw_field.fixed_count.unwrap_or(1);
        let count_u32 = u32::try_from(count)
            .map_err(|_| format!("fixed array `{}` exceeds u32", raw_field.name))?;
        if count_u32 == 0 {
            return Err(format!("fixed array `{}` has zero length", raw_field.name));
        }
        let field_size = element_size
            .checked_mul(count)
            .ok_or_else(|| format!("field `{}` size overflows", raw_field.name))?;
        let effective_alignment = element_alignment.min(packing);
        aggregate_alignment = aggregate_alignment.max(effective_alignment);
        let offset = if raw.kind == RawLayoutKind::Union {
            0
        } else {
            align_up(cursor, effective_alignment)?
        };
        cursor = cursor.max(
            offset
                .checked_add(field_size)
                .ok_or_else(|| format!("field `{}` end overflows", raw_field.name))?,
        );
        fields.push(NativeField {
            name: raw_field.name.clone(),
            offset,
            count: count_u32,
            typ,
        });
    }
    let natural_size = align_up(cursor, aggregate_alignment)?;
    let size = match raw.declared_size {
        Some(size) if size < cursor || size % aggregate_alignment != 0 => {
            return Err(format!(
                "declared aggregate size {size} cannot contain {cursor} bytes at alignment {aggregate_alignment}"
            ));
        }
        Some(size) => size,
        None => natural_size,
    };
    Ok(NativeArchitectureLayout {
        size,
        alignment: aggregate_alignment,
        fields,
    })
}

fn native_field_type(
    raw: &RawType,
    architecture: LayoutArchitecture,
    visiting: &mut BTreeSet<String>,
    contract: Option<FieldContract>,
) -> Result<(NativeFieldType, usize, usize), String> {
    if let Some(contract) = contract {
        let width = architecture.pointer_size();
        match contract {
            FieldContract::RetainedPointer { .. } | FieldContract::NullPointer {} => {
                if raw.pointer_depth == 0
                    && !matches!(
                        &raw.base,
                        RawBaseType::Named {
                            kind: RawNamedKind::DataPointer | RawNamedKind::StringPointer { .. },
                            ..
                        }
                    )
                {
                    return Err("retained field evidence requires a data pointer".into());
                }
                return Ok((NativeFieldType::Pointer, width, width));
            }
            FieldContract::BorrowedHandle {} | FieldContract::OwnedHandle { .. } => {
                if raw.pointer_depth != 0
                    || !matches!(
                        &raw.base,
                        RawBaseType::Named {
                            kind: RawNamedKind::Handle { .. },
                            ..
                        }
                    )
                {
                    return Err("handle field evidence requires a native handle value".into());
                }
                let cleanup = match contract {
                    FieldContract::OwnedHandle { cleanup } => cleanup,
                    _ => Cleanup::None,
                };
                if cleanup != Cleanup::None && cleanup != handle_resource_cleanup(raw) {
                    return Err("owned field evidence conflicts with native cleanup".into());
                }
                return Ok((NativeFieldType::Handle { cleanup }, width, width));
            }
            FieldContract::BooleanInput { .. }
                if raw.pointer_depth == 0
                    && matches!(
                        raw.base,
                        RawBaseType::Scalar(RawScalar::Bool32 | RawScalar::I32)
                    ) => {}
            FieldContract::OutputU32 {}
                if raw.pointer_depth == 0 && raw.base == RawBaseType::Scalar(RawScalar::U32) => {}
            FieldContract::BooleanInput { .. } | FieldContract::OutputU32 {} => {
                return Err("scalar field evidence conflicts with native ABI".into());
            }
        }
    }
    if raw.pointer_depth > 0
        || matches!(
            raw.base,
            RawBaseType::Named {
                kind: RawNamedKind::Handle { .. }
                    | RawNamedKind::DataPointer
                    | RawNamedKind::StringPointer { .. }
                    | RawNamedKind::FunctionPointer
                    | RawNamedKind::ComInterface { .. },
                ..
            }
        )
    {
        return Err("pointer-bearing native aggregate requires retained pointee ownership".into());
    }
    match &raw.base {
        RawBaseType::Scalar(scalar) => {
            let scalar = native_scalar(*scalar)?;
            let (size, alignment) = native_scalar_size_alignment(scalar, architecture);
            Ok((NativeFieldType::Scalar(scalar), size, alignment))
        }
        RawBaseType::Named {
            kind: RawNamedKind::Guid,
            ..
        } => Ok((NativeFieldType::Guid, 16, 4)),
        RawBaseType::Named {
            namespace,
            name,
            kind: RawNamedKind::Enum { underlying, .. },
        } => {
            let _ = (namespace, name);
            let scalar = native_scalar(*underlying)?;
            let (size, alignment) = native_scalar_size_alignment(scalar, architecture);
            Ok((NativeFieldType::Scalar(scalar), size, alignment))
        }
        RawBaseType::Named {
            namespace,
            name,
            kind: RawNamedKind::NativeStruct { layout },
        } => {
            let identity = format!("{namespace}.{name}");
            if !visiting.insert(identity.clone()) {
                return Err(format!("recursive nested native layout {identity}"));
            }
            let nested = compute_native_layout(layout, architecture, visiting)?;
            visiting.remove(&identity);
            let typ = match layout.variants.first().map(|variant| variant.kind) {
                Some(RawLayoutKind::Sequential) => NativeFieldType::Struct {
                    name: identity,
                    layout: Box::new(nested.clone()),
                    by_value_compatible: false,
                },
                Some(RawLayoutKind::Union) => NativeFieldType::Union {
                    name: identity,
                    layout: Box::new(nested.clone()),
                    by_value_compatible: false,
                },
                _ => return Err("nested native aggregate has unknown layout kind".into()),
            };
            Ok((typ, nested.size, nested.alignment))
        }
        RawBaseType::Void => Err("void native aggregate field".into()),
        RawBaseType::Unknown(reason) => Err(format!("unknown aggregate field type: {reason}")),
        RawBaseType::Named {
            namespace,
            name,
            kind: RawNamedKind::Unknown,
        } => Err(format!(
            "unsupported aggregate field type {namespace}.{name}"
        )),
        RawBaseType::Named { kind, .. } => {
            Err(format!("unsupported aggregate field type {kind:?}"))
        }
    }
}

fn native_scalar(scalar: RawScalar) -> Result<NativeScalar, String> {
    Ok(match scalar {
        RawScalar::I8 => NativeScalar::I8,
        RawScalar::Bool8 | RawScalar::U8 => NativeScalar::U8,
        RawScalar::I16 => NativeScalar::I16,
        RawScalar::U16 | RawScalar::Char16 => NativeScalar::U16,
        RawScalar::I32 | RawScalar::Bool32 => NativeScalar::I32,
        RawScalar::U32 => NativeScalar::U32,
        RawScalar::I64 => NativeScalar::I64,
        RawScalar::U64 => NativeScalar::U64,
        RawScalar::F32 => NativeScalar::F32,
        RawScalar::F64 => NativeScalar::F64,
        RawScalar::NativeIsize => NativeScalar::NativeIsize,
        RawScalar::NativeUsize => NativeScalar::NativeUsize,
    })
}

fn native_scalar_size_alignment(
    scalar: NativeScalar,
    architecture: LayoutArchitecture,
) -> (usize, usize) {
    match scalar {
        NativeScalar::I8 | NativeScalar::U8 => (1, 1),
        NativeScalar::I16 | NativeScalar::U16 => (2, 2),
        NativeScalar::I32 | NativeScalar::U32 | NativeScalar::F32 => (4, 4),
        NativeScalar::I64 | NativeScalar::U64 | NativeScalar::F64 => (8, 8),
        NativeScalar::NativeIsize | NativeScalar::NativeUsize => {
            let width = architecture.pointer_size();
            (width, width)
        }
    }
}

fn align_up(value: usize, alignment: usize) -> Result<usize, String> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| "native layout alignment overflow".into())
}

fn handle_resource_cleanup(raw: &RawType) -> Cleanup {
    let RawBaseType::Named {
        kind: RawNamedKind::Handle { cleanup },
        ..
    } = &raw.base
    else {
        return Cleanup::None;
    };
    cleanup
        .as_deref()
        .and_then(|cleanup| parse_cleanup(cleanup).ok())
        .unwrap_or(Cleanup::None)
}

fn validate_buffers(parameters: &[ParameterContract]) -> Result<(), String> {
    for (index, parameter) in parameters.iter().enumerate() {
        let Some(buffer) = &parameter.buffer else {
            if matches!(
                parameter.typ,
                ValueType::DataPointer | ValueType::StringPointer(_)
            ) && parameter.direction != Direction::In
            {
                return Err(format!(
                    "writable pointer parameter `{}` has no size relationship",
                    parameter.name
                ));
            }
            continue;
        };
        if let Some(count_index) = buffer.count_parameter {
            let count = parameters.get(count_index).ok_or_else(|| {
                format!(
                    "buffer `{}` references missing count parameter {count_index}",
                    parameter.name
                )
            })?;
            if count_index == index {
                return Err(format!("buffer `{}` counts itself", parameter.name));
            }
            if !matches!(
                count.typ,
                ValueType::Scalar(
                    Scalar::U16
                        | Scalar::I16
                        | Scalar::U32
                        | Scalar::I32
                        | Scalar::U64
                        | Scalar::I64
                )
            ) {
                return Err(format!(
                    "buffer `{}` count parameter `{}` is not an integer scalar",
                    parameter.name, count.name
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn enum_definitions(raw: &RawFunction) -> Result<Vec<EnumDefinition>, String> {
    let mut definitions = BTreeMap::<(String, String), EnumDefinition>::new();
    for parameter in &raw.parameters {
        collect_raw_enum(&parameter.typ, &mut definitions)?;
        if let Some(buffer) = &parameter.buffer {
            collect_raw_enum(&buffer.element, &mut definitions)?;
        }
    }
    collect_raw_enum(&raw.return_type, &mut definitions)?;
    Ok(definitions.into_values().collect())
}

fn collect_raw_enum(
    raw: &RawType,
    definitions: &mut BTreeMap<(String, String), EnumDefinition>,
) -> Result<(), String> {
    let RawBaseType::Named {
        namespace,
        name,
        kind:
            RawNamedKind::Enum {
                underlying,
                members,
                is_flags,
            },
    } = &raw.base
    else {
        return Ok(());
    };
    let definition = EnumDefinition {
        namespace: namespace.clone(),
        name: name.clone(),
        underlying: map_enum_underlying(*underlying)?,
        members: members
            .iter()
            .map(|member| EnumMember {
                name: member.name.clone(),
                value: member.value,
            })
            .collect(),
        is_flags: *is_flags,
    };
    if let Some(existing) =
        definitions.insert((namespace.clone(), name.clone()), definition.clone())
        && existing != definition
    {
        return Err(format!("enum metadata disagrees for {namespace}.{name}"));
    }
    Ok(())
}

fn map_direction(direction: RawDirection) -> Direction {
    match direction {
        RawDirection::In => Direction::In,
        RawDirection::Out => Direction::Out,
        RawDirection::InOut => Direction::InOut,
    }
}

fn map_constness(constness: crate::win32_metadata::RawConstness) -> Constness {
    match constness {
        crate::win32_metadata::RawConstness::Const => Constness::Const,
        crate::win32_metadata::RawConstness::Mutable => Constness::Mutable,
        crate::win32_metadata::RawConstness::Unspecified => Constness::Unspecified,
        crate::win32_metadata::RawConstness::Mixed => Constness::Mixed,
    }
}

fn raw_native_name(raw: &RawType) -> Option<(String, String)> {
    match &raw.base {
        RawBaseType::Named {
            namespace, name, ..
        } => Some((namespace.clone(), name.clone())),
        RawBaseType::Void | RawBaseType::Scalar(_) | RawBaseType::Unknown(_) => None,
    }
}

fn map_scalar(scalar: RawScalar) -> Scalar {
    match scalar {
        RawScalar::Bool8 => Scalar::Bool8,
        RawScalar::Bool32 => Scalar::Bool32,
        RawScalar::I8 => Scalar::I8,
        RawScalar::U8 => Scalar::U8,
        RawScalar::I16 => Scalar::I16,
        RawScalar::U16 | RawScalar::Char16 => Scalar::U16,
        RawScalar::I32 => Scalar::I32,
        RawScalar::U32 => Scalar::U32,
        RawScalar::I64 => Scalar::I64,
        RawScalar::U64 => Scalar::U64,
        RawScalar::F32 => Scalar::F32,
        RawScalar::F64 => Scalar::F64,
        RawScalar::NativeIsize => Scalar::NativeIsize,
        RawScalar::NativeUsize => Scalar::NativeUsize,
    }
}

fn map_enum_underlying(scalar: RawScalar) -> Result<EnumUnderlying, String> {
    match scalar {
        RawScalar::I8 => Ok(EnumUnderlying::I8),
        RawScalar::U8 => Ok(EnumUnderlying::U8),
        RawScalar::I16 => Ok(EnumUnderlying::I16),
        RawScalar::U16 => Ok(EnumUnderlying::U16),
        RawScalar::I32 => Ok(EnumUnderlying::I32),
        RawScalar::U32 => Ok(EnumUnderlying::U32),
        RawScalar::I64
        | RawScalar::U64
        | RawScalar::NativeIsize
        | RawScalar::NativeUsize
        | RawScalar::F32
        | RawScalar::F64
        | RawScalar::Char16
        | RawScalar::Bool8
        | RawScalar::Bool32 => {
            Err("enum underlying type is not representable as a JS number enum".into())
        }
    }
}

fn scalar_abi(scalar: Scalar) -> AbiType {
    match scalar {
        Scalar::Bool8 => AbiType::U8,
        Scalar::Bool32 => AbiType::Bool32,
        Scalar::I8 => AbiType::I8,
        Scalar::U8 => AbiType::U8,
        Scalar::I16 => AbiType::I16,
        Scalar::U16 => AbiType::U16,
        Scalar::I32 => AbiType::I32,
        Scalar::U32 => AbiType::U32,
        Scalar::I64 => AbiType::I64,
        Scalar::U64 => AbiType::U64,
        Scalar::F32 => AbiType::F32,
        Scalar::F64 => AbiType::F64,
        Scalar::NativeIsize => AbiType::I64,
        Scalar::NativeUsize => AbiType::U64,
    }
}

fn enum_abi(underlying: EnumUnderlying) -> AbiType {
    match underlying {
        EnumUnderlying::I8 => AbiType::I8,
        EnumUnderlying::U8 => AbiType::U8,
        EnumUnderlying::I16 => AbiType::I16,
        EnumUnderlying::U16 => AbiType::U16,
        EnumUnderlying::I32 => AbiType::I32,
        EnumUnderlying::U32 => AbiType::U32,
    }
}

fn map_buffer_element(raw: &RawType) -> Result<(usize, usize), String> {
    match (&raw.base, raw.pointer_depth) {
        (RawBaseType::Scalar(scalar), 0) => {
            let size = element_size(*scalar);
            Ok((size, size.min(8)))
        }
        (
            RawBaseType::Named {
                namespace,
                name,
                kind: RawNamedKind::NativeStruct { layout },
            },
            0,
        ) => {
            let layout = validate_native_layout(namespace, name, layout)?;
            if layout.x64.size != layout.arm64.size
                || layout.x64.alignment != layout.arm64.alignment
            {
                return Err("native buffer element layout differs between x64 and ARM64".into());
            }
            Ok((layout.x64.size, layout.x64.alignment))
        }
        (
            RawBaseType::Named {
                kind: RawNamedKind::Guid,
                ..
            },
            0,
        ) => Ok((16, 4)),
        _ => Err("native buffer element has no validated fixed layout".into()),
    }
}

fn element_size(scalar: RawScalar) -> usize {
    match scalar {
        RawScalar::Bool8 | RawScalar::I8 | RawScalar::U8 => 1,
        RawScalar::I16 | RawScalar::U16 | RawScalar::Char16 => 2,
        RawScalar::I32 | RawScalar::U32 | RawScalar::F32 | RawScalar::Bool32 => 4,
        RawScalar::I64
        | RawScalar::U64
        | RawScalar::F64
        | RawScalar::NativeIsize
        | RawScalar::NativeUsize => 8,
    }
}

fn parse_cleanup(value: &str) -> Result<Cleanup, String> {
    match value {
        "CloseHandle" => Ok(Cleanup::CloseHandle),
        "RegCloseKey" => Ok(Cleanup::RegCloseKey),
        "LocalFree" => Ok(Cleanup::LocalFree),
        "GlobalFree" => Ok(Cleanup::GlobalFree),
        "FreeLibrary" => Ok(Cleanup::FreeLibrary),
        "CloseServiceHandle" => Ok(Cleanup::CloseServiceHandle),
        "CoTaskMemFree" => Ok(Cleanup::CoTaskMemFree),
        "CredFree" => Ok(Cleanup::CredFree),
        other => Err(format!("unsupported native cleanup `{other}`")),
    }
}

fn validate_module(module: &str) -> Result<(), String> {
    let lower = module.to_ascii_lowercase();
    if module.is_empty()
        || !(lower.ends_with(".dll") || lower.ends_with(".drv"))
        || module
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
    {
        return Err(format!("module `{module}` is not a bare System32 DLL name"));
    }
    Ok(())
}
