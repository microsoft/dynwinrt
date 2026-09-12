// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::*;
use std::collections::BTreeSet;

fn error(message: &str) -> ContractError {
    ContractError::new(message)
}

impl CallContract {
    pub fn validate_structure(&self) -> Result<()> {
        match self.version {
            LEGACY_VERSION if !self.results.is_empty() => {
                return Err(error("Version 1 cannot contain version 2 result contracts"));
            }
            CURRENT_VERSION if !self.outputs.is_empty() => {
                return Err(error("Version 2 cannot contain legacy output actions"));
            }
            LEGACY_VERSION | CURRENT_VERSION => {}
            _ => {
                return Err(error(
                    "Unsupported Win32 call contract version; regenerate bindings with a compatible generator",
                ));
            }
        }
        if self.outputs.len() > 1024
            || self.results.len() > 1025
            || self.resource_effects.len() > 1024
        {
            return Err(error("Win32 call contract exceeds structural limits"));
        }
        let mut outputs = BTreeSet::new();
        for rule in &self.outputs {
            if rule.parameter >= MAX_PARAMETERS || !outputs.insert(rule.parameter) {
                return Err(error("Legacy output rules require unique parameters"));
            }
            if let OutputAction::AliasInput { parameter } = rule.action
                && (parameter >= MAX_PARAMETERS || parameter == rule.parameter)
            {
                return Err(error(
                    "Legacy aliases require a distinct bounded input parameter",
                ));
            }
            validate_condition_structure(&rule.when)?;
            if rule.when.succeeded.is_some() {
                return Err(error(
                    "Legacy output conditions cannot use version 2 success predicates",
                ));
            }
        }
        let mut results = BTreeSet::new();
        for result in &self.results {
            if matches!(result.target, ResultTarget::Parameter { index } if index >= MAX_PARAMETERS)
            {
                return Err(error("Result target exceeds the native parameter limit"));
            }
            if !results.insert(result.target) || result.overrides.len() > 32 {
                return Err(error(
                    "Result contracts require unique targets and bounded cases",
                ));
            }
            for policy in result.policies() {
                validate_policy_structure(*policy)?;
                if let ResultPolicy::Defined {
                    ownership: ResultOwnership::AliasInput { parameter },
                    ..
                } = policy
                    && matches!(result.target, ResultTarget::Parameter { index } if index == *parameter)
                {
                    return Err(error("An output cannot alias itself"));
                }
            }
            for case in &result.overrides {
                validate_condition_structure(&case.when)?;
            }
        }
        let mut resources = BTreeSet::new();
        for effect in &self.resource_effects {
            let ResourceEffect::AddFileCompletionModes {
                handle_parameter,
                flags_parameter,
            } = *effect;
            if handle_parameter >= MAX_PARAMETERS
                || flags_parameter >= MAX_PARAMETERS
                || handle_parameter == flags_parameter
                || !resources.insert(handle_parameter)
            {
                return Err(error(
                    "Resource effects require distinct parameters and unique state targets",
                ));
            }
        }
        Ok(())
    }

    pub fn validate_signature(&self, shape: &SignatureShape) -> Result<()> {
        self.validate_structure()?;
        if self.version != CURRENT_VERSION {
            return Err(error(
                "Legacy contracts must be upgraded before signature validation",
            ));
        }
        if !matches!(shape.pointer_width, 32 | 64) || shape.parameters.len() > 1024 {
            return Err(error("Unsupported native signature shape"));
        }
        let expected = shape.result_targets().collect::<BTreeSet<_>>();
        let actual = self
            .results
            .iter()
            .map(|result| result.target)
            .collect::<BTreeSet<_>>();
        if expected != actual {
            return Err(error(
                "Version 2 must describe every direct return and native output slot exactly once",
            ));
        }
        for result in &self.results {
            for policy in result.policies() {
                validate_policy_signature(*policy, result.target, shape)?;
            }
            for (index, case) in result.overrides.iter().enumerate() {
                validate_condition_signature(&case.when, shape)?;
                for previous in &result.overrides[..index] {
                    if !conditions_disjoint(&previous.when, &case.when, shape) {
                        return Err(error("Ambiguous overlapping result contract cases"));
                    }
                }
            }
        }
        for effect in &self.resource_effects {
            let ResourceEffect::AddFileCompletionModes {
                handle_parameter,
                flags_parameter,
            } = *effect;
            let handle = shape.parameters.get(handle_parameter);
            let flags = shape.parameters.get(flags_parameter);
            if !matches!(handle, Some(parameter) if parameter.typ == NativeType::Handle
                && parameter.direction == Direction::In && !parameter.consumes_resource
                && parameter.resource_cleanup == Cleanup::CloseHandle)
                || !matches!(flags, Some(parameter) if parameter.typ == NativeType::U8
                    && parameter.direction == Direction::In)
            {
                return Err(error(
                    "File state effects require a borrowed CloseHandle resource and native U8 flags",
                ));
            }
        }
        Ok(())
    }
}

fn validate_policy_structure(policy: ResultPolicy) -> Result<()> {
    if matches!(policy, ResultPolicy::Defined {
        ownership: ResultOwnership::AliasInput { parameter }, ..
    } if parameter >= MAX_PARAMETERS)
    {
        return Err(error("Alias input exceeds the native parameter limit"));
    }
    if matches!(
        policy,
        ResultPolicy::Defined {
            ownership: ResultOwnership::Owned {
                cleanup: Cleanup::None
            },
            ..
        }
    ) {
        return Err(error(
            "An owned result requires a concrete cleanup contract",
        ));
    }
    Ok(())
}

pub fn validate_condition_structure(condition: &Condition) -> Result<()> {
    if condition.inputs.len() > 16 {
        return Err(error("Too many native input predicates"));
    }
    for predicate in &condition.inputs {
        if predicate.parameter() >= MAX_PARAMETERS {
            return Err(error("Predicate input exceeds the native parameter limit"));
        }
        match predicate {
            InputPredicate::BitsIn { mask, values, .. } => {
                if *mask == 0
                    || values.is_empty()
                    || values.len() > 64
                    || values.iter().any(|value| value & !mask != 0)
                    || values.iter().copied().collect::<BTreeSet<_>>().len() != values.len()
                {
                    return Err(error("Invalid native masked integer predicate"));
                }
            }
            InputPredicate::HandleIn { values, .. } => {
                if values.is_empty()
                    || values.len() > 64
                    || values.iter().copied().collect::<BTreeSet<_>>().len() != values.len()
                {
                    return Err(error("Invalid native signed handle predicate"));
                }
            }
            InputPredicate::NullOrEmpty { element_width, .. } => {
                if !matches!(element_width, 1 | 2) {
                    return Err(error(
                        "Native string predicates require byte or UTF-16 width",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_condition_signature(condition: &Condition, shape: &SignatureShape) -> Result<()> {
    if let Some(bits) = condition.return_value {
        let mask = shape
            .return_type
            .and_then(|typ| typ.integer_mask(shape.pointer_width))
            .ok_or_else(|| error("Return predicates require an integer native return"))?;
        if bits & !mask != 0 {
            return Err(error("Return predicate exceeds the native return width"));
        }
        if condition
            .succeeded
            .is_some_and(|succeeded| shape.succeeds(bits) != succeeded)
        {
            return Err(error("Return and success predicates contradict each other"));
        }
    }
    if condition.succeeded == Some(false) && shape.success_rule == SuccessRule::Always {
        return Err(error(
            "An always-successful signature cannot match a failure predicate",
        ));
    }
    for (index, predicate) in condition.inputs.iter().enumerate() {
        let input = shape
            .parameters
            .get(predicate.parameter())
            .filter(|input| input.direction != Direction::Out)
            .ok_or_else(|| error("A condition refers to a missing native input"))?;
        match predicate {
            InputPredicate::BitsIn { mask, .. } => {
                if !input
                    .typ
                    .integer_mask(shape.pointer_width)
                    .is_some_and(|width| mask & !width == 0)
                {
                    return Err(error(
                        "Bits predicate does not match the native input width",
                    ));
                }
            }
            InputPredicate::HandleIn { values, .. } => {
                if input.typ != NativeType::Handle
                    || (shape.pointer_width == 32
                        && values.iter().any(|value| i32::try_from(*value).is_err()))
                {
                    return Err(error(
                        "Handle predicates require pointer-width signed handle values",
                    ));
                }
            }
            InputPredicate::NullOrEmpty { .. } => {
                if input.typ != NativeType::Pointer {
                    return Err(error("A string condition requires a native data pointer"));
                }
            }
        }
        for previous in &condition.inputs[..index] {
            if predicates_disjoint(previous, predicate, shape.pointer_width) {
                return Err(error("Contradictory native input predicates"));
            }
        }
    }
    Ok(())
}

fn validate_policy_signature(
    policy: ResultPolicy,
    target: ResultTarget,
    shape: &SignatureShape,
) -> Result<()> {
    let typ = shape.result_type(target)?;
    let ResultPolicy::Defined { ownership, .. } = policy else {
        return Ok(());
    };
    match ownership {
        ResultOwnership::Value {} if typ.is_pointer() => {
            return Err(error(
                "A pointer-shaped result must declare borrowed, owned or alias ownership",
            ));
        }
        ResultOwnership::Borrowed {} if !typ.is_pointer() => {
            return Err(error("Borrowed ownership requires a pointer-shaped result"));
        }
        ResultOwnership::Owned { cleanup } => {
            if !matches!(typ, NativeType::Pointer | NativeType::Handle)
                || (shape.result_cleanup(target).owns_resource()
                    && shape.result_cleanup(target) != cleanup)
            {
                return Err(error(
                    "Owned result cleanup conflicts with the native result",
                ));
            }
        }
        ResultOwnership::AliasInput { parameter } => {
            let input = shape
                .parameters
                .get(parameter)
                .ok_or_else(|| error("Alias refers to a missing native input"))?;
            if typ != NativeType::Handle
                || input.typ != NativeType::Handle
                || input.direction != Direction::In
                || input.consumes_resource
                || (shape.result_cleanup(target).owns_resource()
                    && input.resource_cleanup != shape.result_cleanup(target))
            {
                return Err(error(
                    "Alias results require a matching non-consuming handle input",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn conditions_disjoint(left: &Condition, right: &Condition, shape: &SignatureShape) -> bool {
    if matches!((left.succeeded, right.succeeded), (Some(a), Some(b)) if a != b)
        || matches!((left.return_value, right.return_value), (Some(a), Some(b)) if a != b)
        || matches!((left.return_value, right.succeeded), (Some(bits), Some(success)) if shape.succeeds(bits) != success)
        || matches!((right.return_value, left.succeeded), (Some(bits), Some(success)) if shape.succeeds(bits) != success)
    {
        return true;
    }
    left.inputs.iter().any(|a| {
        right
            .inputs
            .iter()
            .any(|b| predicates_disjoint(a, b, shape.pointer_width))
    })
}

fn predicates_disjoint(left: &InputPredicate, right: &InputPredicate, pointer_width: u8) -> bool {
    if left.parameter() != right.parameter() {
        return false;
    }
    let bits = |predicate: &InputPredicate| match predicate {
        InputPredicate::BitsIn { mask, values, .. } => Some((*mask, values.clone())),
        InputPredicate::HandleIn { values, .. } => {
            let mask = if pointer_width == 32 {
                u32::MAX.into()
            } else {
                u64::MAX
            };
            Some((
                mask,
                values.iter().map(|value| (*value as u64) & mask).collect(),
            ))
        }
        InputPredicate::NullOrEmpty { .. } => None,
    };
    let (Some((a_mask, a)), Some((b_mask, b))) = (bits(left), bits(right)) else {
        return false;
    };
    let common = a_mask & b_mask;
    a.iter().all(|a| b.iter().all(|b| a & common != b & common))
}
