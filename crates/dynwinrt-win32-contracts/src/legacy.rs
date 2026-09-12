// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputRule {
    pub parameter: usize,
    pub when: Condition,
    pub action: OutputAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
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

impl CallContract {
    /// Metadata ownership transfers alone do not prove an output pointer is
    /// defined on failure. Such a guarantee requires an explicit result policy.
    pub fn upgrade_metadata(&self, shape: &SignatureShape) -> Result<Self> {
        let mut contract = self.upgrade(shape)?;
        if self.version == LEGACY_VERSION {
            for result in &mut contract.results {
                if matches!(
                    result.on_failure,
                    ResultPolicy::Defined {
                        ownership: ResultOwnership::Owned { .. },
                        ..
                    }
                ) {
                    result.on_failure = ResultPolicy::Undefined {};
                }
            }
        }
        Ok(contract)
    }

    /// Resolves legacy behavior into explicit, versioned result policies.
    /// Metadata planners may replace any policy with stronger reviewed evidence.
    pub fn upgrade(&self, shape: &SignatureShape) -> Result<Self> {
        self.validate_structure()?;
        if self.version == CURRENT_VERSION {
            self.validate_signature(shape)?;
            return Ok(self.clone());
        }
        let mut contract = Self::defaults(shape);
        contract.resource_effects.clone_from(&self.resource_effects);
        for rule in &self.outputs {
            let target = ResultTarget::Parameter {
                index: rule.parameter,
            };
            let output = contract
                .results
                .iter_mut()
                .find(|output| output.target == target)
                .ok_or_else(|| {
                    ContractError::new("Legacy output rule does not target a native output")
                })?;
            match rule.action {
                OutputAction::Unavailable {} => {
                    output.overrides.push(ResultOverride {
                        when: rule.when.clone(),
                        policy: ResultPolicy::Undefined {},
                    });
                }
                OutputAction::AliasInput { parameter } => {
                    let source = shape
                        .parameters
                        .get(parameter)
                        .ok_or_else(|| ContractError::new("Legacy alias input is missing"))?;
                    let target_shape = &shape.parameters[rule.parameter];
                    if target_shape.typ != NativeType::Handle
                        || target_shape.direction != Direction::Out
                        || !target_shape.cleanup.owns_resource()
                        || source.typ != NativeType::Handle
                        || source.direction != Direction::In
                        || source.consumes_resource
                        || source.resource_cleanup != target_shape.cleanup
                    {
                        return Err(ContractError::new(
                            "Legacy alias ownership does not match its input handle",
                        ));
                    }
                    for succeeded in [true, false] {
                        if !succeeded && shape.success_rule == SuccessRule::Always {
                            continue;
                        }
                        if rule
                            .when
                            .return_value
                            .is_some_and(|bits| shape.succeeds(bits) != succeeded)
                        {
                            continue;
                        }
                        let mut when = rule.when.clone();
                        when.succeeded = Some(succeeded);
                        output.overrides.push(ResultOverride {
                            when,
                            policy: if succeeded {
                                ResultPolicy::delivered(ResultOwnership::AliasInput { parameter })
                            } else {
                                ResultPolicy::Undefined {}
                            },
                        });
                    }
                }
            }
        }
        contract.validate_signature(shape)?;
        Ok(contract)
    }

    pub fn defaults(shape: &SignatureShape) -> Self {
        let results = shape
            .result_targets()
            .map(|target| {
                let typ = shape
                    .result_type(target)
                    .expect("a declared result has a type");
                let cleanup = shape.result_cleanup(target);
                let ownership = if cleanup.owns_resource() {
                    ResultOwnership::Owned { cleanup }
                } else if typ.is_pointer() {
                    ResultOwnership::Borrowed {}
                } else {
                    ResultOwnership::Value {}
                };
                ResultContract {
                    target,
                    on_success: ResultPolicy::delivered(ownership),
                    on_failure: match ownership {
                        ResultOwnership::Owned { .. }
                            if matches!(target, ResultTarget::Return {}) =>
                        {
                            ResultPolicy::Undefined {}
                        }
                        ResultOwnership::Owned { .. } => ResultPolicy::discarded(ownership),
                        _ => ResultPolicy::delivered(ownership),
                    },
                    overrides: Vec::new(),
                }
            })
            .collect();
        Self::current(results, Vec::new())
    }
}
