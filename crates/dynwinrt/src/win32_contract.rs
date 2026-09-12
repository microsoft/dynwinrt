// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt_win32_contracts as protocol;
pub use dynwinrt_win32_contracts::{
    CallContract, Condition, Delivery, InputPredicate, OutputAction, OutputRule, ResourceEffect,
    ResultContract, ResultOverride, ResultOwnership, ResultPolicy, ResultTarget,
};

use super::{CallPlanSpec, Cleanup, Direction, SuccessRule, Type, invalid_argument};
use crate::{abi::AbiValue, result::Result};

#[derive(Debug)]
pub(super) struct LegacySurface {
    owned: Vec<ResultTarget>,
    explicit_undefined: Vec<ResultTarget>,
}

impl LegacySurface {
    pub(super) fn new(spec: &CallPlanSpec, contract: &CallContract) -> Self {
        if contract.version != protocol::LEGACY_VERSION {
            return Self {
                owned: Vec::new(),
                explicit_undefined: Vec::new(),
            };
        }
        Self {
            owned: shape(spec)
                .result_targets()
                .filter(|target| match target {
                    ResultTarget::Return {} => spec.return_cleanup.owns_resource(),
                    ResultTarget::Parameter { index } => {
                        spec.parameters[*index].cleanup.owns_resource()
                    }
                })
                .collect(),
            explicit_undefined: contract
                .outputs
                .iter()
                .filter_map(|rule| {
                    matches!(rule.action, OutputAction::Unavailable {}).then_some(
                        ResultTarget::Parameter {
                            index: rule.parameter,
                        },
                    )
                })
                .collect(),
        }
    }

    pub(super) fn null_on_failure(
        &self,
        target: ResultTarget,
        succeeded: bool,
        overridden: bool,
    ) -> bool {
        !succeeded
            && self.owned.contains(&target)
            && !(overridden && self.explicit_undefined.contains(&target))
    }
}

pub(super) fn shape(spec: &CallPlanSpec) -> protocol::SignatureShape {
    protocol::SignatureShape {
        pointer_width: usize::BITS as u8,
        parameters: spec
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| protocol::ParameterShape {
                typ: if spec.parameter_aggregates[index].is_some() {
                    protocol::NativeType::Aggregate
                } else {
                    native_type(parameter.typ)
                },
                direction: match parameter.direction {
                    Direction::In => protocol::Direction::In,
                    Direction::Out => protocol::Direction::Out,
                    Direction::InOut => protocol::Direction::InOut,
                },
                cleanup: parameter.cleanup.into(),
                resource_cleanup: parameter.resource_cleanup.into(),
                consumes_resource: parameter.consumes_resource,
            })
            .collect(),
        return_type: if spec.return_aggregate.is_some() {
            Some(protocol::NativeType::Aggregate)
        } else {
            spec.return_type.map(native_type)
        },
        return_cleanup: spec.return_cleanup.into(),
        success_rule: match spec.success_rule {
            SuccessRule::Always => protocol::SuccessRule::Always,
            SuccessRule::ReturnZero => protocol::SuccessRule::ReturnZero,
            SuccessRule::ReturnNonZero => protocol::SuccessRule::ReturnNonZero,
            SuccessRule::ReturnNonNull => protocol::SuccessRule::ReturnNonNull,
            SuccessRule::HResultSucceeded => protocol::SuccessRule::HResultSucceeded,
            SuccessRule::SignedNonNegative => protocol::SuccessRule::SignedNonNegative,
            SuccessRule::ReturnValidHandle => protocol::SuccessRule::ReturnValidHandle,
        },
    }
}

pub(super) fn resolve(spec: &CallPlanSpec, contract: &CallContract) -> Result<CallContract> {
    contract
        .upgrade(&shape(spec))
        .map_err(|error| invalid_argument(&error.to_string()))
}

fn native_type(typ: Type) -> protocol::NativeType {
    match typ {
        Type::Bool32 => protocol::NativeType::Bool32,
        Type::I8 => protocol::NativeType::I8,
        Type::U8 => protocol::NativeType::U8,
        Type::I16 => protocol::NativeType::I16,
        Type::U16 => protocol::NativeType::U16,
        Type::I32 => protocol::NativeType::I32,
        Type::U32 => protocol::NativeType::U32,
        Type::I64 => protocol::NativeType::I64,
        Type::U64 => protocol::NativeType::U64,
        Type::F32 => protocol::NativeType::F32,
        Type::F64 => protocol::NativeType::F64,
        Type::Pointer => protocol::NativeType::Pointer,
        Type::FunctionPointer => protocol::NativeType::FunctionPointer,
        Type::Handle => protocol::NativeType::Handle,
    }
}

impl From<Cleanup> for protocol::Cleanup {
    fn from(value: Cleanup) -> Self {
        match value {
            Cleanup::None => Self::None,
            Cleanup::CloseHandle => Self::CloseHandle,
            Cleanup::RegCloseKey => Self::RegCloseKey,
            Cleanup::LocalFree => Self::LocalFree,
            Cleanup::GlobalFree => Self::GlobalFree,
            Cleanup::FreeLibrary => Self::FreeLibrary,
            Cleanup::CloseServiceHandle => Self::CloseServiceHandle,
            Cleanup::CoTaskMemFree => Self::CoTaskMemFree,
            Cleanup::CredFree => Self::CredFree,
        }
    }
}

impl From<protocol::Cleanup> for Cleanup {
    fn from(value: protocol::Cleanup) -> Self {
        match value {
            protocol::Cleanup::None => Self::None,
            protocol::Cleanup::CloseHandle => Self::CloseHandle,
            protocol::Cleanup::RegCloseKey => Self::RegCloseKey,
            protocol::Cleanup::LocalFree => Self::LocalFree,
            protocol::Cleanup::GlobalFree => Self::GlobalFree,
            protocol::Cleanup::FreeLibrary => Self::FreeLibrary,
            protocol::Cleanup::CloseServiceHandle => Self::CloseServiceHandle,
            protocol::Cleanup::CoTaskMemFree => Self::CoTaskMemFree,
            protocol::Cleanup::CredFree => Self::CredFree,
        }
    }
}

pub(super) unsafe fn matches_inputs(
    condition: &Condition,
    inputs: &[Option<AbiValue>],
) -> Result<bool> {
    for predicate in &condition.inputs {
        let value = inputs
            .get(predicate.parameter())
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid_argument("Call condition is missing native input storage"))?;
        let matched = match predicate {
            InputPredicate::BitsIn { mask, values, .. } => {
                let bits = abi_bits(value).ok_or_else(|| {
                    invalid_argument("Call condition has non-integer input storage")
                })?;
                values.contains(&(bits & mask))
            }
            InputPredicate::HandleIn { values, .. } => {
                let AbiValue::Pointer(pointer) = value else {
                    return Err(invalid_argument(
                        "Handle condition has non-handle input storage",
                    ));
                };
                values.contains(&(*pointer as isize as i64))
            }
            InputPredicate::NullOrEmpty { element_width, .. } => {
                let AbiValue::Pointer(pointer) = value else {
                    return Err(invalid_argument(
                        "String condition has non-pointer input storage",
                    ));
                };
                pointer.is_null()
                    || unsafe {
                        match element_width {
                            1 => *pointer.cast::<u8>() == 0,
                            2 => pointer.cast::<u16>().read_unaligned() == 0,
                            _ => return Err(invalid_argument("Unsupported native string width")),
                        }
                    }
            }
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
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
