// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::ir::*;
use crate::win32_contracts::{
    Cleanup, Contract, Entry, FailureOutput, ParameterContract as ContractParam,
    PredefinedOwnership, identifier,
};
use crate::win32_metadata::{Export, NativeType};

fn hkey(typ: &NativeType, pointers: &[bool]) -> bool {
    typ.name == "Windows.Win32.System.Registry.HKEY"
        && typ.kind == "typedef"
        && typ.pointers == pointers
        && typ
            .underlying
            .as_ref()
            .is_some_and(|underlying| underlying.pointer_to("void"))
        && typ.attributes.iter().any(|attribute| {
            attribute.name == "Windows.Win32.Foundation.Metadata.RAIIFreeAttribute"
                && attribute.value == "01000B526567436C6F73654B65790000"
        })
}

fn u32_shape(typ: &NativeType, pointers: &[bool]) -> bool {
    (typ.name == "u32" && typ.kind == "scalar" && typ.pointers == pointers)
        || (typ.kind == "enum"
            && typ.pointers == pointers
            && typ
                .underlying
                .as_ref()
                .is_some_and(|underlying| underlying.scalar("u32")))
}

fn camel(name: &str) -> Result<String, String> {
    if !identifier(name)
        || [
            "arguments",
            "eval",
            "default",
            "function",
            "return",
            "status",
        ]
        .contains(&name)
    {
        return Err("win32.unsupported-identifier".into());
    }
    let mut result = name.to_owned();
    result[..1].make_ascii_lowercase();
    Ok(result)
}

pub(super) fn function(raw: &Export, entry: &Entry) -> Result<Function, String> {
    entry.validate_selector(raw)?;
    let contracts = entry.contract.parameters();
    if raw.parameters.len() != contracts.len()
        || raw.signature_flags != "MethodCallAttributes(0)"
        || raw.calling_convention != "system"
        || raw.architectures != 7
    {
        return Err("win32.incomplete-call-contract".into());
    }
    let returns = match &entry.contract {
        Contract::DirectScalar { .. } if raw.return_type.scalar("u32") => ReturnKind::U32,
        Contract::DirectScalar { .. } if raw.return_type.scalar("u64") => ReturnKind::U64,
        Contract::StatusZero { .. }
            if raw.return_type.kind == "enum"
                && raw
                    .return_type
                    .named("Windows.Win32.Foundation.WIN32_ERROR", &[], "u32") =>
        {
            ReturnKind::Status
        }
        Contract::DirectScalar { .. } | Contract::StatusZero { .. } => {
            return Err("win32.unsupported-return-contract".into());
        }
    };
    let mut parameters = Vec::new();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for (index, (raw_param, contract)) in raw.parameters.iter().zip(contracts).enumerate() {
        let typ = &raw_param.typ;
        let input = raw_param.input && !raw_param.output;
        let output = raw_param.output && !raw_param.input;
        let result: Option<(Argument, Option<InputType>, Option<OutputType>)> = match contract {
            ContractParam::U32Input {} if input && u32_shape(typ, &[]) => {
                Some((Argument::U32, Some(InputType::U32), None))
            }
            ContractParam::Utf16Input { nullable }
                if input
                    && *nullable == raw_param.optional
                    && raw_param.const_attribute
                    && typ.kind == "typedef"
                    && typ.name == "Windows.Win32.Foundation.PWSTR"
                    && typ.pointers.is_empty()
                    && typ
                        .underlying
                        .as_ref()
                        .is_some_and(|u| u.pointer_to("char16")) =>
            {
                Some((
                    Argument::Utf16 {
                        nullable: *nullable,
                    },
                    Some(InputType::Utf16 {
                        nullable: *nullable,
                    }),
                    None,
                ))
            }
            ContractParam::BorrowedHkey {
                reject_performance_data,
            } if input && !raw_param.optional && hkey(typ, &[]) => Some((
                Argument::BorrowHkey {
                    reject_performance_data: *reject_performance_data,
                },
                Some(InputType::Hkey),
                None,
            )),
            ContractParam::OwnedHkeyOutput {
                cleanup: Cleanup::RegCloseKey,
                borrowed_from,
                predefined: PredefinedOwnership::Borrowed,
                failure: FailureOutput::Unspecified,
            } if output
                && !raw_param.optional
                && hkey(typ, &[false])
                && returns == ReturnKind::Status
                && matches!(
                    contracts.get(*borrowed_from),
                    Some(ContractParam::BorrowedHkey { .. })
                ) =>
            {
                Some((
                    Argument::OwnHkey {
                        borrowed_from: *borrowed_from,
                    },
                    None,
                    Some(OutputType::Hkey),
                ))
            }
            ContractParam::ConsumedHkey {
                cleanup: Cleanup::RegCloseKey,
            } if input
                && !raw_param.optional
                && hkey(typ, &[])
                && returns == ReturnKind::Status
                && contracts.len() == 1
                && raw.dll == "ADVAPI32.dll"
                && raw.entry_point == "RegCloseKey" =>
            {
                Some((Argument::ConsumeHkey, Some(InputType::Resource), None))
            }
            ContractParam::ReservedNull {}
                if raw_param.reserved
                    && raw_param.optional
                    && !raw_param.input
                    && !raw_param.output
                    && typ.pointer_to("u32") =>
            {
                Some((Argument::ReservedNull, None, None))
            }
            ContractParam::U32Output {}
                if output
                    && !raw_param.const_attribute
                    && u32_shape(typ, &[false])
                    && returns == ReturnKind::Status =>
            {
                Some((Argument::OutU32, None, Some(OutputType::U32)))
            }
            ContractParam::OptionalByteOutput { count_parameter }
                if output
                    && raw_param.optional
                    && !raw_param.const_attribute
                    && typ.pointer_to("u8")
                    && returns == ReturnKind::Status
                    && raw_param.byte_count_parameter.map(usize::from)
                        == Some(*count_parameter)
                    && matches!(contracts.get(*count_parameter), Some(ContractParam::ByteCapacityRequiredSize { buffer_parameter }) if *buffer_parameter == index)
                    && contracts.iter().any(|p| {
                        matches!(
                            p,
                            ContractParam::BorrowedHkey {
                                reject_performance_data: true
                            }
                        )
                    }) =>
            {
                Some((
                    Argument::Bytes {
                        count_parameter: *count_parameter,
                    },
                    Some(InputType::OptionalBytes),
                    Some(OutputType::Bytes),
                ))
            }
            ContractParam::ByteCapacityRequiredSize { buffer_parameter }
                if raw_param.input
                    && raw_param.output
                    && raw_param.optional
                    && !raw_param.const_attribute
                    && typ.pointer_to("u32")
                    && matches!(contracts.get(*buffer_parameter), Some(ContractParam::OptionalByteOutput { count_parameter }) if *count_parameter == index) =>
            {
                Some((
                    Argument::ByteCount {
                        buffer_parameter: *buffer_parameter,
                    },
                    None,
                    Some(OutputType::U32),
                ))
            }
            _ => None,
        };
        let Some((argument, input_type, output_type)) = result else {
            return Err(format!("win32.unsupported-parameter-contract:{index}"));
        };
        let name = camel(&raw_param.name)?;
        if let Some(typ) = input_type {
            inputs.push(Input {
                name: name.clone(),
                typ,
            });
        }
        if let Some(typ) = output_type {
            outputs.push(Output { name, typ });
        }
        parameters.push(argument);
    }
    Ok(Function {
        name: camel(&raw.name)?,
        plan: Plan {
            version: 1,
            dll: raw.dll.clone(),
            entry_point: raw.entry_point.clone(),
            calling_convention: "system",
            architectures: raw.architectures,
            returns,
            parameters,
        },
        inputs,
        outputs,
    })
}

pub(super) fn file(runtime_import: &str, functions: Vec<Function>) -> Result<File, String> {
    let unsafe_import = if let Some(prefix) = runtime_import
        .strip_suffix("win32.js")
        .filter(|prefix| prefix.is_empty() || prefix.ends_with(['/', '\\']))
    {
        format!("{prefix}win32-unsafe.js")
    } else if runtime_import.ends_with("/win32") {
        format!("{runtime_import}/unsafe")
    } else {
        return Err(
            "win32.runtime-import: expected a /win32 package subpath or local win32.js facade"
                .into(),
        );
    };
    Ok(File {
        safe_import: runtime_import.into(),
        unsafe_import,
        functions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32_contracts::Registry;

    #[test]
    fn win32_projection_rejects_nearby_unsupported_shapes() {
        let Ok(paths) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let raw = crate::win32_metadata::read_exports(&paths, "Windows.Win32.System.Registry")
            .unwrap()
            .into_iter()
            .find(|e| e.name == "RegQueryValueExW")
            .unwrap();
        let entry = Registry::builtin().unwrap().find(&raw).unwrap();
        function(&raw, entry).unwrap();
        let mutations: &[fn(&mut Export)] = &[
            |e| e.parameters[4].typ.name = "void".into(),
            |e| e.parameters[4].byte_count_parameter = Some(3),
            |e| e.parameters[1].const_attribute = false,
            |e| e.parameters[0].typ.name = "Windows.Win32.Foundation.HANDLE".into(),
            |e| e.parameters[2].reserved = false,
            |e| e.parameters[5].output = false,
        ];
        for mutate in mutations {
            let mut changed = raw.clone();
            mutate(&mut changed);
            let mut test_entry = entry.clone();
            // Test semantic validation independently of the production pin.
            test_entry.selector.source_fingerprint = changed.fingerprint();
            assert!(function(&changed, &test_entry).is_err());
        }
        let mut changed = entry.clone();
        let Contract::StatusZero { parameters } = &mut changed.contract else {
            unreachable!()
        };
        parameters[0] = ContractParam::BorrowedHkey {
            reject_performance_data: false,
        };
        assert!(function(&raw, &changed).is_err());
    }
}
