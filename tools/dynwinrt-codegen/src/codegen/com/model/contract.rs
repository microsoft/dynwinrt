// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::diagnostics::ModelError;
use super::ids::{ParamIndex, TypeId};
use super::ownership::{Cleanup, ComOwnership, validate_ownership_cleanup};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum Direction {
    In,
    Out,
    InOut,
}

impl Direction {
    pub(in crate::codegen::com) const fn is_input(self) -> bool {
        matches!(self, Self::In | Self::InOut)
    }

    pub(super) const fn is_output(self) -> bool {
        matches!(self, Self::Out | Self::InOut)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum Nullability {
    Required,
    Nullable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum CountUnit {
    Elements,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum BufferSizing {
    SingleCall,
    FixedCapacity,
    TwoCall { max_retries: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum CountRelation {
    InputCount {
        count_param: ParamIndex,
        actual_length_param: Option<ParamIndex>,
        unit: CountUnit,
    },
    CallerCapacity {
        capacity_param: ParamIndex,
        actual_length_param: Option<ParamIndex>,
        unit: CountUnit,
        sizing: BufferSizing,
    },
    EnumeratorNext {
        capacity_param: ParamIndex,
        fetched_param: ParamIndex,
        fetched_optional_for_single: bool,
    },
    CalleeAllocated {
        count_param: ParamIndex,
        unit: CountUnit,
    },
}

impl CountRelation {
    fn referenced_params(&self) -> impl Iterator<Item = ParamIndex> {
        let (first, second) = match self {
            Self::InputCount {
                count_param,
                actual_length_param,
                ..
            } => (*count_param, *actual_length_param),
            Self::CalleeAllocated { count_param, .. } => (*count_param, None),
            Self::CallerCapacity {
                capacity_param,
                actual_length_param,
                ..
            } => (*capacity_param, *actual_length_param),
            Self::EnumeratorNext {
                capacity_param,
                fetched_param,
                ..
            } => (*capacity_param, Some(*fetched_param)),
        };
        std::iter::once(first).chain(second)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct ComParamContract {
    name: String,
    abi_type: TypeId,
    direction: Direction,
    optional: bool,
    nullability: Nullability,
    count: Option<CountRelation>,
    ownership: ComOwnership,
    cleanup: Cleanup,
}

impl ComParamContract {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        name: impl Into<String>,
        abi_type: TypeId,
        direction: Direction,
        optional: bool,
        nullability: Nullability,
        count: Option<CountRelation>,
        ownership: ComOwnership,
        cleanup: Cleanup,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ModelError::InvalidContract(
                "parameter name must not be empty".into(),
            ));
        }
        validate_ownership_cleanup(&ownership, &cleanup)?;
        Ok(Self {
            name,
            abi_type,
            direction,
            optional,
            nullability,
            count,
            ownership,
            cleanup,
        })
    }

    pub(in crate::codegen::com) fn name(&self) -> &str {
        &self.name
    }

    pub(in crate::codegen::com) const fn abi_type(&self) -> TypeId {
        self.abi_type
    }

    pub(in crate::codegen::com) const fn direction(&self) -> Direction {
        self.direction
    }

    pub(in crate::codegen::com) const fn optional(&self) -> bool {
        self.optional
    }

    pub(in crate::codegen::com) const fn nullability(&self) -> Nullability {
        self.nullability
    }

    pub(in crate::codegen::com) const fn count(&self) -> Option<&CountRelation> {
        self.count.as_ref()
    }

    pub(in crate::codegen::com) const fn ownership(&self) -> &ComOwnership {
        &self.ownership
    }

    pub(in crate::codegen::com) const fn cleanup(&self) -> &Cleanup {
        &self.cleanup
    }

    pub(super) fn validate_param_references(
        &self,
        own_index: ParamIndex,
        param_count: usize,
    ) -> Result<(), ModelError> {
        if let Some(count) = &self.count {
            for referenced in count.referenced_params() {
                validate_distinct_param_reference("count", own_index, referenced, param_count)?;
            }
        }
        Ok(())
    }
}

fn validate_distinct_param_reference(
    role: &str,
    own_index: ParamIndex,
    referenced: ParamIndex,
    param_count: usize,
) -> Result<(), ModelError> {
    if referenced.index() >= param_count {
        return Err(ModelError::InvalidContract(format!(
            "{role} parameter index {} is outside the {param_count}-parameter method",
            referenced.index()
        )));
    }
    if referenced == own_index {
        return Err(ModelError::InvalidContract(format!(
            "parameter {} cannot be its own {role}",
            own_index.index()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn borrowed_param(count: Option<CountRelation>) -> ComParamContract {
        ComParamContract::new(
            "buffer",
            TypeId::from_index(0).unwrap(),
            Direction::In,
            false,
            Nullability::Required,
            count,
            ComOwnership::Borrowed,
            Cleanup::None,
        )
        .unwrap()
    }

    #[test]
    fn parameter_relationships_are_indexed_and_bounded() {
        let param = borrowed_param(Some(CountRelation::InputCount {
            count_param: ParamIndex::new(1),
            actual_length_param: None,
            unit: CountUnit::Bytes,
        }));
        param
            .validate_param_references(ParamIndex::new(0), 2)
            .unwrap();
        assert!(param.direction().is_input());
        assert!(!param.direction().is_output());
        assert_eq!(param.nullability(), Nullability::Required);
        assert!(param.count().is_some());

        assert!(
            borrowed_param(Some(CountRelation::InputCount {
                count_param: ParamIndex::new(2),
                actual_length_param: None,
                unit: CountUnit::Elements,
            }))
            .validate_param_references(ParamIndex::new(0), 2)
            .is_err()
        );
    }
}
