// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet};

use crate::win32_metadata::{
    RawApis, RawBaseType, RawBufferSize, RawCallingConvention, RawDirection, RawFunction,
    RawNamedKind, RawScalar,
};

use super::ir::{
    AbiType, AsyncIoKind, Cleanup, Conversion, Direction, FunctionContract, InputExpression,
    NativeBuilderFieldKind, NativeFieldType, NativeLayout, NativeOutputFieldKind, OmittedFunction,
    ProjectedApis, ProjectedAsyncFunction, ProjectedFunction, ProjectedNativeBuilder,
    ProjectedNativeBuilderField, ProjectedNativeOutputField, ProjectedOutput, ProjectionResult,
    ReturnShape, RuntimeParameter, RuntimePlan, Scalar, StringEncoding, SurfaceParameter,
    SurfaceType, ValueType,
};
use super::model;

pub(super) fn project_apis(raw: &RawApis) -> ProjectionResult {
    let mut async_functions = Vec::new();
    let mut async_names = BTreeSet::new();
    for name in ["ReadFile", "WriteFile"] {
        let candidates = raw
            .functions
            .iter()
            .filter(|function| function.name == name)
            .collect::<Vec<_>>();
        if let [function] = candidates.as_slice()
            && let Some(projected) = project_async_function(function)
        {
            async_names.insert(name.to_string());
            async_functions.push(projected);
        }
    }
    let sync_raw = RawApis {
        namespace: raw.namespace.clone(),
        class_name: raw.class_name.clone(),
        functions: raw
            .functions
            .iter()
            .filter(|function| !async_names.contains(&function.name))
            .cloned()
            .collect(),
    };
    let (contracts, mut omitted) = model::validate_apis(&sync_raw);
    let mut functions = Vec::new();
    let mut enums = BTreeMap::new();
    for contract in contracts {
        match project_function(&contract) {
            Ok(function) => {
                for definition in &contract.enums {
                    enums.insert(
                        (definition.namespace.clone(), definition.name.clone()),
                        definition.clone(),
                    );
                }
                functions.push(function);
            }
            Err(reason) => omitted.push((
                format!("{}.{}::{}", raw.namespace, raw.class_name, contract.name),
                reason,
            )),
        }
    }
    functions.sort_by(|left, right| left.js_name.cmp(&right.js_name));
    assign_unicode_aliases(&mut functions);
    let native_builders = project_native_builders(&functions);
    ProjectionResult {
        projected: ProjectedApis {
            namespace: raw.namespace.clone(),
            class_name: raw.class_name.clone(),
            functions,
            enums: enums.into_values().collect(),
            native_builders,
            async_functions,
        },
        omitted: omitted
            .into_iter()
            .map(|(identity, reason)| OmittedFunction { identity, reason })
            .collect(),
    }
}

fn project_async_function(function: &RawFunction) -> Option<ProjectedAsyncFunction> {
    let kind = match (
        function.namespace.as_str(),
        function.name.as_str(),
        function.dll.to_ascii_lowercase().as_str(),
    ) {
        ("Windows.Win32.Storage.FileSystem", "ReadFile", "kernel32.dll") => AsyncIoKind::Read,
        ("Windows.Win32.Storage.FileSystem", "WriteFile", "kernel32.dll") => AsyncIoKind::Write,
        _ => return None,
    };
    if function.parameters.len() != 5
        || function.calling_convention != RawCallingConvention::System
        || function.variadic
        || !function.architectures.x64
        || !function.architectures.arm64
        || !function.supports_last_error
        || !matches!(
            function.return_type.base,
            RawBaseType::Scalar(RawScalar::Bool32)
        )
    {
        return None;
    }
    let [file, buffer, count, transferred, overlapped] = function.parameters.as_slice() else {
        return None;
    };
    let file_ok = file.name == "hFile"
        && file.direction == RawDirection::In
        && file.typ.pointer_depth == 0
        && matches!(
            &file.typ.base,
            RawBaseType::Named {
                name,
                kind: RawNamedKind::Handle { .. },
                ..
            } if name == "HANDLE"
        );
    let buffer_ok = buffer.name == "lpBuffer"
        && buffer.direction
            == match kind {
                AsyncIoKind::Read => RawDirection::Out,
                AsyncIoKind::Write => RawDirection::In,
            }
        && buffer.typ.pointer_depth > 0
        && matches!(
            buffer.buffer.as_ref().map(|buffer| &buffer.size),
            Some(RawBufferSize::ByteCountParam(2))
        );
    let expected_count = match kind {
        AsyncIoKind::Read => "nNumberOfBytesToRead",
        AsyncIoKind::Write => "nNumberOfBytesToWrite",
    };
    let expected_transferred = match kind {
        AsyncIoKind::Read => "lpNumberOfBytesRead",
        AsyncIoKind::Write => "lpNumberOfBytesWritten",
    };
    let count_ok = count.name == expected_count
        && count.direction == RawDirection::In
        && count.typ.pointer_depth == 0
        && matches!(count.typ.base, RawBaseType::Scalar(RawScalar::U32));
    let transferred_ok = transferred.name == expected_transferred
        && transferred.direction == RawDirection::Out
        && transferred.typ.pointer_depth == 1
        && matches!(transferred.typ.base, RawBaseType::Scalar(RawScalar::U32));
    let overlapped_ok = overlapped.name == "lpOverlapped"
        && overlapped.direction == RawDirection::InOut
        && overlapped.nullable
        && overlapped.typ.pointer_depth == 1
        && matches!(
            &overlapped.typ.base,
            RawBaseType::Named {
                name,
                kind: RawNamedKind::NativeStruct { .. },
                ..
            } if name == "OVERLAPPED"
        );
    (file_ok && buffer_ok && count_ok && transferred_ok && overlapped_ok).then(|| {
        ProjectedAsyncFunction {
            js_name: format!("{}Async", camel_case(&function.name)),
            kind,
        }
    })
}

fn project_native_builders(functions: &[ProjectedFunction]) -> Vec<ProjectedNativeBuilder> {
    let mut layouts = BTreeMap::<(String, String), &NativeLayout>::new();
    for function in functions {
        for input in &function.inputs {
            if let InputExpression::NativeAggregate { layout, .. } = input {
                layouts
                    .entry((layout.namespace.clone(), layout.name.clone()))
                    .or_insert(layout);
            }
        }
    }
    layouts
        .into_values()
        .filter_map(project_native_builder)
        .collect()
}

fn project_native_builder(layout: &NativeLayout) -> Option<ProjectedNativeBuilder> {
    let fields = &layout.x64.fields;
    if layout.namespace == "Windows.Win32.Security" && layout.name == "SECURITY_ATTRIBUTES" {
        if !fields.iter().any(|field| {
            field.name == "nLength"
                && matches!(
                    field.typ,
                    NativeFieldType::Scalar(super::ir::NativeScalar::U32)
                )
        }) || !fields.iter().any(|field| {
            field.name == "lpSecurityDescriptor" && matches!(field.typ, NativeFieldType::Pointer)
        }) || !fields.iter().any(|field| {
            field.name == "bInheritHandle"
                && matches!(
                    field.typ,
                    NativeFieldType::Scalar(super::ir::NativeScalar::I32)
                )
        }) {
            return None;
        }
        return Some(ProjectedNativeBuilder {
            layout_name: layout.name.clone(),
            js_name: "SecurityAttributes".into(),
            size_field: Some("nLength".into()),
            fields: vec![
                ProjectedNativeBuilderField {
                    native_name: "lpSecurityDescriptor".into(),
                    surface_name: "securityDescriptor".into(),
                    kind: NativeBuilderFieldKind::DataPointer { nullable: true },
                    optional: true,
                },
                ProjectedNativeBuilderField {
                    native_name: "bInheritHandle".into(),
                    surface_name: "inheritHandle".into(),
                    kind: NativeBuilderFieldKind::Boolean,
                    optional: true,
                },
            ],
            outputs: Vec::new(),
        });
    }
    if layout.namespace == "Windows.Win32.System.Threading"
        && matches!(layout.name.as_str(), "STARTUPINFOA" | "STARTUPINFOW")
        && fields.iter().any(|field| {
            field.name == "cb"
                && matches!(
                    field.typ,
                    NativeFieldType::Scalar(super::ir::NativeScalar::U32)
                )
        })
    {
        return Some(ProjectedNativeBuilder {
            layout_name: layout.name.clone(),
            js_name: if layout.name == "STARTUPINFOA" {
                "StartupInfoA"
            } else {
                "StartupInfoW"
            }
            .into(),
            size_field: Some("cb".into()),
            fields: Vec::new(),
            outputs: Vec::new(),
        });
    }
    if layout.namespace == "Windows.Win32.System.Threading" && layout.name == "PROCESS_INFORMATION"
    {
        let handle = |name: &str| {
            fields.iter().any(|field| {
                field.name == name
                    && matches!(
                        field.typ,
                        NativeFieldType::Handle {
                            cleanup: Cleanup::CloseHandle
                        }
                    )
            })
        };
        let u32_field = |name: &str| {
            fields.iter().any(|field| {
                field.name == name
                    && matches!(
                        field.typ,
                        NativeFieldType::Scalar(super::ir::NativeScalar::U32)
                    )
            })
        };
        if !(handle("hProcess")
            && handle("hThread")
            && u32_field("dwProcessId")
            && u32_field("dwThreadId"))
        {
            return None;
        }
        return Some(ProjectedNativeBuilder {
            layout_name: layout.name.clone(),
            js_name: "ProcessInformation".into(),
            size_field: None,
            fields: Vec::new(),
            outputs: vec![
                ProjectedNativeOutputField {
                    native_name: "hProcess".into(),
                    surface_name: "process".into(),
                    kind: NativeOutputFieldKind::Resource {
                        cleanup: Cleanup::CloseHandle,
                    },
                },
                ProjectedNativeOutputField {
                    native_name: "hThread".into(),
                    surface_name: "thread".into(),
                    kind: NativeOutputFieldKind::Resource {
                        cleanup: Cleanup::CloseHandle,
                    },
                },
                ProjectedNativeOutputField {
                    native_name: "dwProcessId".into(),
                    surface_name: "processId".into(),
                    kind: NativeOutputFieldKind::U32,
                },
                ProjectedNativeOutputField {
                    native_name: "dwThreadId".into(),
                    surface_name: "threadId".into(),
                    kind: NativeOutputFieldKind::U32,
                },
            ],
        });
    }
    None
}

fn project_function(contract: &FunctionContract) -> Result<ProjectedFunction, String> {
    let count_buffers = count_buffer_relations(contract)?;
    let mut parameters = Vec::<SurfaceParameter>::new();
    let mut native_surface = vec![None; contract.parameters.len()];
    let mut inputs = Vec::<InputExpression>::new();
    let mut runtime_parameters = Vec::<RuntimeParameter>::new();
    let mut output_index = 0;
    let mut outputs = Vec::new();

    for (index, parameter) in contract.parameters.iter().enumerate() {
        let is_buffer_count = count_buffers.contains_key(&index);
        if should_surface_input(parameter, is_buffer_count) {
            let surface_index = parameters.len();
            let minimum_bytes = parameter
                .buffer
                .as_ref()
                .and_then(|buffer| {
                    buffer.constant_count.map(|count| {
                        count.checked_mul(buffer.element_size).ok_or_else(|| {
                            format!("buffer `{}` fixed size overflows usize", parameter.name)
                        })
                    })
                })
                .transpose()?
                .or(matches!(parameter.typ, ValueType::GuidPointer).then_some(16));
            parameters.push(SurfaceParameter {
                name: input_name(&parameter.name, surface_index),
                typ: if parameter.consumes_resource {
                    SurfaceType::ManagedResource
                } else if parameter.null_null_terminated
                    && matches!(parameter.typ, ValueType::StringPointer(_))
                {
                    match parameter.typ {
                        ValueType::StringPointer(encoding) => SurfaceType::MultiString(encoding),
                        _ => unreachable!("validated NullNullTerminated string pointer"),
                    }
                } else {
                    input_surface_type(&parameter.typ)
                },
                nullable: parameter.nullable,
                minimum_bytes,
                alignment: parameter
                    .buffer
                    .as_ref()
                    .map(|buffer| buffer.element_alignment)
                    .or(matches!(parameter.typ, ValueType::GuidPointer).then_some(4)),
            });
            native_surface[index] = Some(surface_index);
        }
    }

    for (index, parameter) in contract.parameters.iter().enumerate() {
        let reserved_pointer =
            parameter.reserved && matches!(parameter.typ, ValueType::DataPointer);
        let surface_index = native_surface[index];

        let runtime = if let ValueType::NativeStruct { layout } = &parameter.typ {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: false,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: Some(layout.clone()),
            }
        } else if matches!(
            &parameter.typ,
            ValueType::NativeStructPointer { .. } | ValueType::NativeUnionPointer { .. }
        ) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if matches!(parameter.typ, ValueType::ScalarPointer { .. }) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if matches!(parameter.typ, ValueType::GuidPointer) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if matches!(parameter.typ, ValueType::NullPointer) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: true,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if matches!(parameter.typ, ValueType::ComInterface { .. }) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if matches!(parameter.typ, ValueType::StringPointerPointer(_)) {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if parameter.buffer.is_some() {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if reserved_pointer {
            RuntimeParameter {
                abi: AbiType::Pointer,
                direction: Direction::In,
                nullable: true,
                cleanup: Cleanup::None,
                consumes_resource: false,
                resource_cleanup: Cleanup::None,
                aggregate: None,
            }
        } else if parameter.pointer_depth == 0 {
            RuntimeParameter {
                abi: parameter.abi,
                direction: Direction::In,
                nullable: parameter.nullable,
                cleanup: Cleanup::None,
                consumes_resource: parameter.consumes_resource,
                resource_cleanup: parameter.resource_cleanup,
                aggregate: None,
            }
        } else {
            RuntimeParameter {
                abi: parameter.abi,
                direction: parameter.direction,
                nullable: parameter.nullable,
                cleanup: parameter.cleanup,
                consumes_resource: parameter.consumes_resource,
                resource_cleanup: if matches!(parameter.direction, Direction::In | Direction::InOut)
                {
                    parameter.resource_cleanup
                } else {
                    Cleanup::None
                },
                aggregate: None,
            }
        };

        if matches!(runtime.direction, Direction::In | Direction::InOut) {
            let expression = if parameter.reserved {
                if runtime.abi == AbiType::Pointer {
                    InputExpression::NullPointer
                } else {
                    InputExpression::Zero(runtime.abi)
                }
            } else if let ValueType::NativeStruct { layout } = &parameter.typ {
                InputExpression::NativeAggregate {
                    parameter_index: surface_index.ok_or_else(|| {
                        format!(
                            "native aggregate parameter `{}` has no projected input",
                            parameter.name
                        )
                    })?,
                    layout: layout.clone(),
                    nullable: false,
                    by_value: true,
                }
            } else if let ValueType::NativeStructPointer { layout }
            | ValueType::NativeUnionPointer { layout } = &parameter.typ
            {
                InputExpression::NativeAggregate {
                    parameter_index: surface_index.ok_or_else(|| {
                        format!(
                            "native aggregate parameter `{}` has no projected input",
                            parameter.name
                        )
                    })?,
                    layout: layout.clone(),
                    nullable: parameter.nullable,
                    by_value: false,
                }
            } else if let ValueType::ScalarPointer { scalar } = parameter.typ {
                InputExpression::ScalarPointer {
                    parameter_index: surface_index.ok_or_else(|| {
                        format!(
                            "scalar pointer parameter `{}` has no projected input",
                            parameter.name
                        )
                    })?,
                    scalar,
                    nullable: parameter.nullable,
                }
            } else if let ValueType::ComInterface { iid, .. } = &parameter.typ {
                InputExpression::ComInterface {
                    parameter_index: surface_index.ok_or_else(|| {
                        format!(
                            "COM interface parameter `{}` has no projected input",
                            parameter.name
                        )
                    })?,
                    iid: iid.clone(),
                }
            } else if let ValueType::StringPointerPointer(encoding) = parameter.typ {
                InputExpression::StringPointerPointer {
                    parameter_index: surface_index.ok_or_else(|| {
                        format!(
                            "string pointer slot `{}` has no projected input",
                            parameter.name
                        )
                    })?,
                    encoding,
                    nullable: parameter.nullable,
                }
            } else if matches!(parameter.typ, ValueType::NullPointer) {
                InputExpression::NullPointer
            } else if let Some(buffer_index) = count_buffers.get(&index).copied() {
                let buffer = &contract.parameters[buffer_index];
                let buffer_surface = native_surface[buffer_index]
                    .ok_or_else(|| format!("buffer `{}` has no projected input", buffer.name))?;
                let buffer_contract = buffer.buffer.as_ref().expect("count relation");
                InputExpression::BufferLength {
                    parameter_index: buffer_surface,
                    divisor: if buffer_contract.count_is_bytes {
                        1
                    } else {
                        buffer_contract.element_size
                    },
                    abi: parameter.abi,
                }
            } else {
                let surface = surface_index.ok_or_else(|| {
                    format!(
                        "input parameter `{}` has no projected input",
                        parameter.name
                    )
                })?;
                InputExpression::Surface {
                    parameter_index: surface,
                    conversion: input_conversion(parameter),
                }
            };
            inputs.push(expression);
        }

        if matches!(runtime.direction, Direction::Out | Direction::InOut) {
            outputs.push(ProjectedOutput {
                name: output_name(parameter),
                output_index,
                typ: output_surface_type(parameter),
                conversion: output_conversion(parameter),
            });
            output_index += 1;
        }
        runtime_parameters.push(runtime);
    }

    let return_shape = if contract.return_is_status {
        ReturnShape::Object {
            status: true,
            return_value: None,
            outputs,
            last_error: contract.capture_last_error,
        }
    } else if !outputs.is_empty() || contract.capture_last_error {
        ReturnShape::Object {
            status: false,
            return_value: contract.return_type.as_ref().map(|typ| {
                (
                    return_surface_type(typ, contract.return_cleanup),
                    return_conversion(typ, contract.return_cleanup),
                )
            }),
            outputs,
            last_error: contract.capture_last_error,
        }
    } else if let Some(typ) = &contract.return_type {
        ReturnShape::Direct {
            typ: return_surface_type(typ, contract.return_cleanup),
            conversion: return_conversion(typ, contract.return_cleanup),
        }
    } else {
        ReturnShape::Void
    };

    Ok(ProjectedFunction {
        metadata_name: contract.name.clone(),
        js_name: camel_case(&contract.name),
        unicode_alias: None,
        parameters,
        inputs,
        runtime: RuntimePlan {
            dll: contract.dll.clone(),
            entry_point: contract.entry_point.clone(),
            parameters: runtime_parameters,
            return_abi: contract.return_abi,
            return_aggregate: contract.return_aggregate.clone(),
            return_cleanup: contract.return_cleanup,
            success_rule: contract.success_rule,
            capture_last_error: contract.capture_last_error,
            calling_convention: contract.calling_convention,
        },
        return_shape,
        subsystem: contract.subsystem,
    })
}

fn count_buffer_relations(contract: &FunctionContract) -> Result<BTreeMap<usize, usize>, String> {
    let mut relations = BTreeMap::new();
    for (buffer_index, parameter) in contract.parameters.iter().enumerate() {
        let Some(count_index) = parameter
            .buffer
            .as_ref()
            .and_then(|buffer| buffer.count_parameter)
        else {
            continue;
        };
        if let Some(existing) = relations.insert(count_index, buffer_index)
            && existing != buffer_index
        {
            return Err(format!(
                "count parameter {} controls multiple buffers; grouped buffer projection is not implemented",
                contract.parameters[count_index].name
            ));
        }
    }
    Ok(relations)
}

fn should_surface_input(parameter: &super::ir::ParameterContract, is_buffer_count: bool) -> bool {
    if parameter.reserved || is_buffer_count || matches!(parameter.typ, ValueType::NullPointer) {
        return false;
    }
    parameter.buffer.is_some()
        || matches!(
            &parameter.typ,
            ValueType::NativeStructPointer { .. } | ValueType::NativeUnionPointer { .. }
        )
        || matches!(parameter.typ, ValueType::NativeStruct { .. })
        || matches!(parameter.typ, ValueType::ScalarPointer { .. })
        || matches!(parameter.typ, ValueType::GuidPointer)
        || matches!(parameter.direction, Direction::In | Direction::InOut)
}

fn input_surface_type(typ: &ValueType) -> SurfaceType {
    match typ {
        ValueType::Scalar(Scalar::Bool8 | Scalar::Bool32) => SurfaceType::Boolean,
        ValueType::Scalar(
            Scalar::I64 | Scalar::U64 | Scalar::NativeIsize | Scalar::NativeUsize,
        ) => SurfaceType::BigInt,
        ValueType::Scalar(_) => SurfaceType::Number,
        ValueType::Enum { name, .. } => SurfaceType::Enum(name.clone()),
        ValueType::Handle { name, .. } => SurfaceType::Handle(name.clone()),
        ValueType::DataPointer => SurfaceType::Buffer,
        ValueType::StringPointer(encoding) => SurfaceType::String(*encoding),
        ValueType::FunctionPointer => SurfaceType::BigInt,
        ValueType::NativeStructPointer { layout } => SurfaceType::NativeStruct(layout.name.clone()),
        ValueType::NativeUnionPointer { layout } => SurfaceType::NativeUnion(layout.name.clone()),
        ValueType::NativeStruct { layout } => SurfaceType::NativeStruct(layout.name.clone()),
        ValueType::ScalarPointer { scalar } => scalar_surface_type(*scalar),
        ValueType::GuidPointer => SurfaceType::Buffer,
        ValueType::NullPointer => unreachable!("null-only pointer is hidden"),
        ValueType::ComInterface { name, .. } => SurfaceType::ComInterface(name.clone()),
        ValueType::StringPointerPointer(encoding) => SurfaceType::String(*encoding),
    }
}

fn scalar_surface_type(scalar: Scalar) -> SurfaceType {
    match scalar {
        Scalar::Bool8 | Scalar::Bool32 => SurfaceType::Boolean,
        Scalar::I64 | Scalar::U64 | Scalar::NativeIsize | Scalar::NativeUsize => {
            SurfaceType::BigInt
        }
        Scalar::I8
        | Scalar::U8
        | Scalar::I16
        | Scalar::U16
        | Scalar::I32
        | Scalar::U32
        | Scalar::F32
        | Scalar::F64 => SurfaceType::Number,
    }
}

fn output_surface_type(parameter: &super::ir::ParameterContract) -> SurfaceType {
    if parameter.cleanup != Cleanup::None {
        SurfaceType::Resource
    } else {
        match &parameter.typ {
            ValueType::Scalar(Scalar::Bool8 | Scalar::Bool32) => SurfaceType::Boolean,
            ValueType::Scalar(
                Scalar::I64 | Scalar::U64 | Scalar::NativeIsize | Scalar::NativeUsize,
            ) => SurfaceType::BigInt,
            ValueType::Scalar(_) => SurfaceType::Number,
            ValueType::Enum { name, .. } => SurfaceType::Enum(name.clone()),
            ValueType::Handle { name, .. } => SurfaceType::Handle(name.clone()),
            ValueType::DataPointer | ValueType::FunctionPointer => SurfaceType::BigInt,
            ValueType::StringPointer(_) => SurfaceType::BigInt,
            ValueType::NativeStructPointer { layout } => {
                SurfaceType::NativeStruct(layout.name.clone())
            }
            ValueType::NativeUnionPointer { layout } => {
                SurfaceType::NativeUnion(layout.name.clone())
            }
            ValueType::NativeStruct { layout } => SurfaceType::NativeStruct(layout.name.clone()),
            ValueType::ScalarPointer { scalar } => scalar_surface_type(*scalar),
            ValueType::GuidPointer => SurfaceType::Buffer,
            ValueType::NullPointer => unreachable!("null-only pointer has no output"),
            ValueType::ComInterface { name, .. } => SurfaceType::ComInterface(name.clone()),
            ValueType::StringPointerPointer(encoding) => SurfaceType::String(*encoding),
        }
    }
}

fn return_surface_type(typ: &ValueType, cleanup: Cleanup) -> SurfaceType {
    if cleanup != Cleanup::None {
        SurfaceType::Resource
    } else {
        input_surface_type(typ)
    }
}

fn input_conversion(parameter: &super::ir::ParameterContract) -> Conversion {
    if parameter.consumes_resource {
        return Conversion::ResourceInput(parameter.resource_cleanup);
    }
    if parameter.null_null_terminated && matches!(parameter.typ, ValueType::StringPointer(_)) {
        return match parameter.typ {
            ValueType::StringPointer(StringEncoding::Wide) => Conversion::WideMultiString,
            ValueType::StringPointer(StringEncoding::Ansi) => Conversion::AnsiMultiString,
            _ => unreachable!("validated NullNullTerminated string pointer"),
        };
    }
    match &parameter.typ {
        ValueType::Scalar(Scalar::Bool8) => Conversion::Boolean8,
        ValueType::Scalar(Scalar::Bool32) => Conversion::Boolean,
        ValueType::Scalar(Scalar::I8) => Conversion::I8,
        ValueType::Scalar(Scalar::U8) => Conversion::U8,
        ValueType::Scalar(Scalar::I16) => Conversion::I16,
        ValueType::Scalar(Scalar::U16) => Conversion::U16,
        ValueType::Scalar(Scalar::I32) => Conversion::I32,
        ValueType::Scalar(Scalar::U32) => Conversion::U32,
        ValueType::Scalar(Scalar::I64) => Conversion::I64,
        ValueType::Scalar(Scalar::U64) => Conversion::U64,
        ValueType::Scalar(Scalar::F32) => Conversion::F32,
        ValueType::Scalar(Scalar::F64) => Conversion::F64,
        ValueType::Scalar(Scalar::NativeIsize) => Conversion::I64,
        ValueType::Scalar(Scalar::NativeUsize) => Conversion::U64,
        ValueType::Enum { underlying, .. } => match underlying {
            super::ir::EnumUnderlying::I8 => Conversion::I8,
            super::ir::EnumUnderlying::U8 => Conversion::U8,
            super::ir::EnumUnderlying::I16 => Conversion::I16,
            super::ir::EnumUnderlying::U16 => Conversion::U16,
            super::ir::EnumUnderlying::I32 => Conversion::I32,
            super::ir::EnumUnderlying::U32 => Conversion::U32,
        },
        ValueType::Handle { .. } => Conversion::Handle,
        ValueType::DataPointer => Conversion::DataPointer,
        ValueType::StringPointer(StringEncoding::Wide) => Conversion::WideString,
        ValueType::StringPointer(StringEncoding::Ansi) => Conversion::AnsiString,
        ValueType::FunctionPointer => Conversion::BigInt,
        ValueType::NativeStructPointer { .. } | ValueType::NativeUnionPointer { .. } => {
            unreachable!("native aggregate inputs use a dedicated expression")
        }
        ValueType::NativeStruct { .. } => {
            unreachable!("by-value native aggregate inputs use a dedicated expression")
        }
        ValueType::ScalarPointer { .. } => {
            unreachable!("scalar pointer inputs use a dedicated expression")
        }
        ValueType::GuidPointer => Conversion::DataPointer,
        ValueType::NullPointer => unreachable!("null-only pointer has no surface input"),
        ValueType::ComInterface { .. } => {
            unreachable!("COM interface inputs use a dedicated expression")
        }
        ValueType::StringPointerPointer(_) => {
            unreachable!("string pointer slots use a dedicated expression")
        }
    }
}

fn output_conversion(parameter: &super::ir::ParameterContract) -> Conversion {
    if parameter.cleanup != Cleanup::None {
        Conversion::Resource
    } else {
        return_conversion(&parameter.typ, Cleanup::None)
    }
}

fn return_conversion(typ: &ValueType, cleanup: Cleanup) -> Conversion {
    if cleanup != Cleanup::None {
        return Conversion::Resource;
    }
    match typ {
        ValueType::Scalar(Scalar::Bool8 | Scalar::Bool32) => Conversion::Boolean,
        ValueType::Scalar(
            Scalar::I64 | Scalar::U64 | Scalar::NativeIsize | Scalar::NativeUsize,
        ) => Conversion::BigInt,
        ValueType::Scalar(_) | ValueType::Enum { .. } => Conversion::Number,
        ValueType::Handle { .. }
        | ValueType::DataPointer
        | ValueType::StringPointer(_)
        | ValueType::FunctionPointer => Conversion::BigInt,
        ValueType::NativeStructPointer { .. } | ValueType::NativeUnionPointer { .. } => {
            unreachable!("native aggregate pointer results are not projected")
        }
        ValueType::NativeStruct { .. } => Conversion::NativeAggregate,
        ValueType::ScalarPointer { .. } => {
            unreachable!("scalar pointer results are not projected")
        }
        ValueType::GuidPointer => {
            unreachable!("GUID pointer results are caller-owned buffers")
        }
        ValueType::NullPointer => unreachable!("null-only pointer cannot be a return"),
        ValueType::ComInterface { .. } => {
            unreachable!("COM interface returns require ownership projection")
        }
        ValueType::StringPointerPointer(_) => {
            unreachable!("string pointer slot returns require ownership projection")
        }
    }
}

fn input_name(raw: &str, index: usize) -> String {
    let stripped = strip_prefix(raw);
    safe_identifier(if stripped.is_empty() {
        format!("arg{index}")
    } else {
        lower_first(stripped)
    })
}

fn output_name(parameter: &super::ir::ParameterContract) -> String {
    if let ValueType::Handle { name, .. } = &parameter.typ
        && parameter.name.to_ascii_lowercase().ends_with("result")
    {
        return lower_first(name.trim_start_matches('H'));
    }
    let mut name = strip_prefix(&parameter.name).to_string();
    let lower = parameter.name.to_ascii_lowercase();
    if lower.contains("cb") {
        name.push_str("Size");
    } else if lower.contains("cch") || lower.contains("ch") {
        name.push_str("Length");
    }
    if name.eq_ignore_ascii_case("result") {
        name = "value".into();
    }
    safe_identifier(lower_first(&name))
}

fn strip_prefix(value: &str) -> &str {
    for prefix in [
        "lpp", "lpcb", "lpch", "lpcch", "lpdw", "lp", "pp", "phk", "ph", "pcb", "pdw", "pcch",
        "pch", "p",
    ] {
        if let Some(rest) = value.strip_prefix(prefix)
            && rest
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_uppercase())
        {
            return rest;
        }
    }
    value
}

fn lower_first(value: &str) -> String {
    if value
        .chars()
        .all(|character| !character.is_ascii_alphabetic() || character.is_ascii_uppercase())
    {
        return value.to_ascii_lowercase();
    }
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_ascii_lowercase().to_string() + characters.as_str()
}

fn safe_identifier(value: String) -> String {
    if matches!(
        value.as_str(),
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "status"
            | "result"
            | "lastError"
    ) {
        format!("{value}_")
    } else {
        value
    }
}

fn camel_case(value: &str) -> String {
    let uppercase = value
        .chars()
        .take_while(|character| character.is_ascii_uppercase())
        .count();
    if uppercase <= 1 {
        return lower_first(value);
    }
    if uppercase == value.len() {
        return value.to_ascii_lowercase();
    }
    value[..uppercase - 1].to_ascii_lowercase() + &value[uppercase - 1..]
}

fn assign_unicode_aliases(functions: &mut [ProjectedFunction]) {
    let names = functions
        .iter()
        .map(|function| function.js_name.clone())
        .collect::<BTreeSet<_>>();
    let mut aliases = BTreeSet::new();
    for function in functions {
        let Some(base) = function.js_name.strip_suffix('W') else {
            continue;
        };
        if !base.is_empty() && !names.contains(base) && aliases.insert(base.to_string()) {
            function.unicode_alias = Some(base.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_name_gets_natural_alias() {
        let mut functions = vec![ProjectedFunction {
            metadata_name: "RegOpenKeyExW".into(),
            js_name: "regOpenKeyExW".into(),
            unicode_alias: None,
            parameters: vec![],
            inputs: vec![],
            runtime: RuntimePlan {
                dll: "advapi32.dll".into(),
                entry_point: "RegOpenKeyExW".into(),
                parameters: vec![],
                return_abi: Some(AbiType::I32),
                return_aggregate: None,
                return_cleanup: Cleanup::None,
                success_rule: super::super::ir::SuccessRule::ReturnZero,
                capture_last_error: false,
                calling_convention: super::super::ir::CallingConvention::System,
            },
            return_shape: ReturnShape::Object {
                status: true,
                return_value: None,
                outputs: vec![],
                last_error: false,
            },
            subsystem: None,
        }];
        assign_unicode_aliases(&mut functions);
        assert_eq!(functions[0].unicode_alias.as_deref(), Some("regOpenKeyEx"));
    }

    #[test]
    fn javascript_reserved_parameter_names_are_escaped() {
        assert_eq!(input_name("lpIn", 0), "in_");
        assert_eq!(input_name("class", 0), "class_");
    }
}
