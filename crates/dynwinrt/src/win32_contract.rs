// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::{CallPlanSpec, Cleanup, Direction, Type, invalid_argument};
use crate::{abi::AbiValue, result::Result};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallContract {
    #[serde(default)]
    pub outputs: Vec<OutputRule>,
    #[serde(default)]
    pub resource_effects: Vec<ResourceEffect>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputRule {
    pub parameter: usize,
    pub when: Condition,
    pub action: OutputAction,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Condition {
    #[serde(default)]
    pub inputs: Vec<InputPredicate>,
    #[serde(default)]
    pub return_value: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InputPredicate {
    BitsIn {
        parameter: usize,
        mask: u64,
        values: Vec<u64>,
    },
    HandleIn {
        parameter: usize,
        values: Vec<i64>,
    },
    NullOrEmpty {
        parameter: usize,
        element_width: u8,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OutputAction {
    Unavailable {},
    AliasInput { parameter: usize },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResourceEffect {
    AddFileCompletionModes {
        handle_parameter: usize,
        flags_parameter: usize,
    },
}

impl CallContract {
    pub(super) fn validate(&self, spec: &CallPlanSpec) -> Result<()> {
        if self.outputs.len() > spec.parameters.len()
            || self.resource_effects.len() > spec.parameters.len()
        {
            return Err(invalid_argument("Win32 call contract has too many rules"));
        }
        let mut targets = BTreeSet::new();
        for rule in &self.outputs {
            let output = spec.parameters.get(rule.parameter).ok_or_else(|| {
                invalid_argument("Win32 output rule refers to an unknown native parameter")
            })?;
            if !output.direction.is_output() || !targets.insert(rule.parameter) {
                return Err(invalid_argument(
                    "Win32 output rules require unique native output parameters",
                ));
            }
            if rule.when.inputs.len() > 16 {
                return Err(invalid_argument(
                    "Win32 output condition has too many predicates",
                ));
            }
            if let Some(value) = rule.when.return_value {
                let mask = spec.return_type.and_then(integer_mask).ok_or_else(|| {
                    invalid_argument("Win32 return conditions require an integer native return")
                })?;
                if value & !mask != 0 {
                    return Err(invalid_argument(
                        "Win32 return condition exceeds its native width",
                    ));
                }
            }
            for predicate in &rule.when.inputs {
                let parameter = match predicate {
                    InputPredicate::BitsIn { parameter, .. }
                    | InputPredicate::HandleIn { parameter, .. }
                    | InputPredicate::NullOrEmpty { parameter, .. } => *parameter,
                };
                let input = spec.parameters.get(parameter).ok_or_else(|| {
                    invalid_argument("Win32 input condition refers to an unknown native parameter")
                })?;
                if input.direction == Direction::Out
                    || spec.parameter_aggregates[parameter].is_some()
                {
                    return Err(invalid_argument(
                        "Win32 input condition requires a scalar input",
                    ));
                }
                match predicate {
                    InputPredicate::BitsIn { mask, values, .. } => {
                        let width = integer_mask(input.typ).ok_or_else(|| {
                            invalid_argument("Win32 bits condition requires an integer or handle")
                        })?;
                        if *mask == 0
                            || mask & !width != 0
                            || values.is_empty()
                            || values.len() > 64
                            || values.iter().any(|value| value & !mask != 0)
                            || values.iter().copied().collect::<BTreeSet<_>>().len() != values.len()
                        {
                            return Err(invalid_argument("Invalid Win32 native bits condition"));
                        }
                    }
                    InputPredicate::HandleIn { values, .. } => {
                        if input.typ != Type::Handle
                            || values.is_empty()
                            || values.len() > 64
                            || values.iter().any(|value| isize::try_from(*value).is_err())
                            || values.iter().copied().collect::<BTreeSet<_>>().len() != values.len()
                        {
                            return Err(invalid_argument(
                                "Native handle conditions require unique pointer-width signed handle values",
                            ));
                        }
                    }
                    InputPredicate::NullOrEmpty { element_width, .. } => {
                        if input.typ != Type::Pointer || !matches!(element_width, 1 | 2) {
                            return Err(invalid_argument(
                                "Win32 empty-string condition requires a pointer and byte/UTF-16 width",
                            ));
                        }
                    }
                }
            }
            if let OutputAction::AliasInput { parameter } = rule.action {
                let input = spec.parameters.get(parameter).ok_or_else(|| {
                    invalid_argument("Win32 alias refers to an unknown native input")
                })?;
                if output.typ != Type::Handle
                    || output.direction != Direction::Out
                    || !output.cleanup.owns_resource()
                    || input.typ != Type::Handle
                    || input.direction != Direction::In
                    || input.consumes_resource
                    || input.resource_cleanup != output.cleanup
                {
                    return Err(invalid_argument(
                        "Win32 alias requires an owned handle output and a matching non-consuming input",
                    ));
                }
            }
        }
        let mut resources = BTreeSet::new();
        for effect in &self.resource_effects {
            let ResourceEffect::AddFileCompletionModes {
                handle_parameter,
                flags_parameter,
            } = *effect;
            let handle = spec.parameters.get(handle_parameter);
            let flags = spec.parameters.get(flags_parameter);
            if !matches!(handle, Some(parameter) if parameter.typ == Type::Handle
                && parameter.direction == Direction::In && !parameter.consumes_resource
                && parameter.resource_cleanup == Cleanup::CloseHandle)
                || !matches!(flags, Some(parameter) if parameter.typ == Type::U8
                    && parameter.direction == Direction::In)
                || !resources.insert(handle_parameter)
            {
                return Err(invalid_argument(
                    "File completion mode effects require a unique CloseHandle input and native U8 flags",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn changes_resource(&self, parameter: usize) -> bool {
        self.resource_effects.iter().any(|effect| {
            matches!(effect, ResourceEffect::AddFileCompletionModes { handle_parameter, .. }
                if *handle_parameter == parameter)
        })
    }
}

impl Condition {
    pub(super) unsafe fn matches_inputs(&self, inputs: &[Option<AbiValue>]) -> bool {
        self.inputs.iter().all(|predicate| match predicate {
            InputPredicate::BitsIn {
                parameter,
                mask,
                values,
            } => inputs[*parameter]
                .as_ref()
                .and_then(abi_bits)
                .is_some_and(|bits| values.contains(&(bits & mask))),
            InputPredicate::HandleIn { parameter, values } => {
                let Some(AbiValue::Pointer(pointer)) = &inputs[*parameter] else {
                    return false;
                };
                values.contains(&(*pointer as isize as i64))
            }
            InputPredicate::NullOrEmpty {
                parameter,
                element_width,
            } => {
                let Some(AbiValue::Pointer(pointer)) = &inputs[*parameter] else {
                    return false;
                };
                pointer.is_null()
                    || unsafe {
                        match element_width {
                            1 => *pointer.cast::<u8>() == 0,
                            2 => pointer.cast::<u16>().read_unaligned() == 0,
                            _ => unreachable!("validated native character width"),
                        }
                    }
            }
        })
    }
}

fn integer_mask(typ: Type) -> Option<u64> {
    Some(match typ {
        Type::I8 | Type::U8 => u8::MAX as u64,
        Type::I16 | Type::U16 => u16::MAX as u64,
        Type::Bool32 | Type::I32 | Type::U32 => u32::MAX as u64,
        Type::I64 | Type::U64 => u64::MAX,
        Type::Handle => usize::MAX as u64,
        _ => return None,
    })
}

pub(super) fn abi_bits(value: &AbiValue) -> Option<u64> {
    Some(match value {
        AbiValue::Bool(value) => u64::from(*value),
        AbiValue::I8(value) => *value as u8 as u64,
        AbiValue::U8(value) => *value as u64,
        AbiValue::I16(value) => *value as u16 as u64,
        AbiValue::U16(value) => *value as u64,
        AbiValue::I32(value) => *value as u32 as u64,
        AbiValue::U32(value) => *value as u64,
        AbiValue::I64(value) => *value as u64,
        AbiValue::U64(value) => *value,
        AbiValue::Pointer(value) => *value as usize as u64,
        _ => return None,
    })
}
