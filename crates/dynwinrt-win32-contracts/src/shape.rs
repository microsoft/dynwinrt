// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::{Cleanup, ContractError, Result, ResultTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeType {
    Bool32,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Pointer,
    FunctionPointer,
    Handle,
    Aggregate,
}

impl NativeType {
    pub fn integer_mask(self, pointer_width: u8) -> Option<u64> {
        Some(match self {
            Self::I8 | Self::U8 => u8::MAX.into(),
            Self::I16 | Self::U16 => u16::MAX.into(),
            Self::Bool32 | Self::I32 | Self::U32 => u32::MAX.into(),
            Self::I64 | Self::U64 => u64::MAX,
            Self::Handle if pointer_width == 32 => u32::MAX.into(),
            Self::Handle if pointer_width == 64 => u64::MAX,
            _ => return None,
        })
    }

    pub const fn is_pointer(self) -> bool {
        matches!(self, Self::Pointer | Self::Handle | Self::FunctionPointer)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    InOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuccessRule {
    Always,
    ReturnZero,
    ReturnNonZero,
    ReturnNonNull,
    HResultSucceeded,
    SignedNonNegative,
    ReturnValidHandle,
}

#[derive(Clone, Debug)]
pub struct ParameterShape {
    pub typ: NativeType,
    pub direction: Direction,
    pub cleanup: Cleanup,
    pub resource_cleanup: Cleanup,
    pub consumes_resource: bool,
}

#[derive(Clone, Debug)]
pub struct SignatureShape {
    pub pointer_width: u8,
    pub parameters: Vec<ParameterShape>,
    pub return_type: Option<NativeType>,
    pub return_cleanup: Cleanup,
    pub success_rule: SuccessRule,
}

impl SignatureShape {
    pub fn result_targets(&self) -> impl Iterator<Item = ResultTarget> + '_ {
        self.return_type
            .iter()
            .map(|_| ResultTarget::Return {})
            .chain(
                self.parameters
                    .iter()
                    .enumerate()
                    .filter_map(|(index, parameter)| {
                        (parameter.direction != Direction::In)
                            .then_some(ResultTarget::Parameter { index })
                    }),
            )
    }

    pub fn result_type(&self, target: ResultTarget) -> Result<NativeType> {
        match target {
            ResultTarget::Return {} => self
                .return_type
                .ok_or_else(|| ContractError::new("Void return has no result slot")),
            ResultTarget::Parameter { index } => self
                .parameters
                .get(index)
                .filter(|parameter| parameter.direction != Direction::In)
                .map(|parameter| parameter.typ)
                .ok_or_else(|| {
                    ContractError::new("Result target is not a native output parameter")
                }),
        }
    }

    pub fn result_cleanup(&self, target: ResultTarget) -> Cleanup {
        match target {
            ResultTarget::Return {} => self.return_cleanup,
            ResultTarget::Parameter { index } => self.parameters[index].cleanup,
        }
    }

    pub fn succeeds(&self, bits: u64) -> bool {
        match self.success_rule {
            SuccessRule::Always => true,
            SuccessRule::ReturnZero => bits == 0,
            SuccessRule::ReturnNonZero | SuccessRule::ReturnNonNull => bits != 0,
            SuccessRule::HResultSucceeded => bits as u32 as i32 >= 0,
            SuccessRule::SignedNonNegative => match self.return_type {
                Some(NativeType::I8) => bits as u8 as i8 >= 0,
                Some(NativeType::I16) => bits as u16 as i16 >= 0,
                Some(NativeType::I32) => bits as u32 as i32 >= 0,
                _ => bits as i64 >= 0,
            },
            SuccessRule::ReturnValidHandle => {
                bits != 0
                    && bits
                        != if self.pointer_width == 32 {
                            u32::MAX.into()
                        } else {
                            u64::MAX
                        }
            }
        }
    }
}
