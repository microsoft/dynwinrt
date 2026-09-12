// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Win32-only wire contracts. This crate has no Windows, libffi or language binding dependency.

mod legacy;
mod shape;
mod validation;

pub use legacy::{OutputAction, OutputRule};
use serde::{Deserialize, Serialize};
pub use shape::{Direction, NativeType, ParameterShape, SignatureShape, SuccessRule};
pub use validation::validate_condition_structure;

pub const CURRENT_VERSION: u32 = 2;
pub const LEGACY_VERSION: u32 = 1;
pub const MAX_CONTRACT_BYTES: usize = 1024 * 1024;
pub const MAX_PARAMETERS: usize = 1024;
pub type Result<T> = std::result::Result<T, ContractError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError(String);

impl ContractError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ContractError {}

const fn legacy_version() -> u32 {
    LEGACY_VERSION
}

fn string_enum<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(
        String::deserialize(deserializer)?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallContract {
    #[serde(default = "legacy_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ResultContract>,
    #[serde(default)]
    pub resource_effects: Vec<ResourceEffect>,
}

impl Default for CallContract {
    fn default() -> Self {
        Self {
            version: LEGACY_VERSION,
            outputs: Vec::new(),
            results: Vec::new(),
            resource_effects: Vec::new(),
        }
    }
}

impl CallContract {
    pub fn decode(json: &str) -> Result<Self> {
        if json.len() > MAX_CONTRACT_BYTES {
            return Err(ContractError::new(
                "Win32 call contract exceeds the descriptor size limit",
            ));
        }
        let contract: Self = serde_json::from_str(json)
            .map_err(|error| ContractError::new(format!("Invalid Win32 call contract: {error}")))?;
        contract.validate_structure()?;
        Ok(contract)
    }

    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty() && self.results.is_empty() && self.resource_effects.is_empty()
    }

    pub fn current(results: Vec<ResultContract>, resource_effects: Vec<ResourceEffect>) -> Self {
        Self {
            version: CURRENT_VERSION,
            outputs: Vec::new(),
            results,
            resource_effects,
        }
    }

    pub fn result(&self, target: ResultTarget) -> Option<&ResultContract> {
        self.results.iter().find(|result| result.target == target)
    }

    pub fn changes_resource(&self, parameter: usize) -> bool {
        self.resource_effects.iter().any(|effect| matches!(
            effect, ResourceEffect::AddFileCompletionModes { handle_parameter, .. } if *handle_parameter == parameter
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResultTarget {
    Return {},
    Parameter { index: usize },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResultContract {
    pub target: ResultTarget,
    pub on_success: ResultPolicy,
    pub on_failure: ResultPolicy,
    #[serde(default)]
    pub overrides: Vec<ResultOverride>,
}

impl ResultContract {
    pub fn policies(&self) -> impl Iterator<Item = &ResultPolicy> {
        [&self.on_success, &self.on_failure]
            .into_iter()
            .chain(self.overrides.iter().map(|rule| &rule.policy))
    }

    pub fn may_be_unavailable(&self) -> bool {
        self.policies().any(|policy| !policy.is_delivered())
    }

    pub fn may_alias(&self) -> bool {
        self.policies().any(|policy| {
            matches!(
                policy,
                ResultPolicy::Defined {
                    ownership: ResultOwnership::AliasInput { .. },
                    ..
                }
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResultOverride {
    pub when: Condition,
    pub policy: ResultPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResultPolicy {
    Undefined {},
    Defined {
        ownership: ResultOwnership,
        #[serde(deserialize_with = "string_enum")]
        delivery: Delivery,
    },
}

impl ResultPolicy {
    pub const fn delivered(ownership: ResultOwnership) -> Self {
        Self::Defined {
            ownership,
            delivery: Delivery::Deliver,
        }
    }

    pub const fn discarded(ownership: ResultOwnership) -> Self {
        Self::Defined {
            ownership,
            delivery: Delivery::Discard,
        }
    }

    pub const fn is_delivered(self) -> bool {
        matches!(
            self,
            Self::Defined {
                delivery: Delivery::Deliver,
                ..
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    Deliver,
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResultOwnership {
    Value {},
    Borrowed {},
    Owned {
        #[serde(deserialize_with = "string_enum")]
        cleanup: Cleanup,
    },
    AliasInput {
        parameter: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Cleanup {
    None,
    CloseHandle,
    RegCloseKey,
    LocalFree,
    GlobalFree,
    FreeLibrary,
    CloseServiceHandle,
    CoTaskMemFree,
    CredFree,
}

impl Cleanup {
    pub const fn owns_resource(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Condition {
    #[serde(default)]
    pub inputs: Vec<InputPredicate>,
    #[serde(default)]
    pub return_value: Option<u64>,
    #[serde(default)]
    pub succeeded: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
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

impl InputPredicate {
    pub const fn parameter(&self) -> usize {
        match self {
            Self::BitsIn { parameter, .. }
            | Self::HandleIn { parameter, .. }
            | Self::NullOrEmpty { parameter, .. } => *parameter,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "schema"), derive(schemars::JsonSchema))]
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

#[cfg(any(test, feature = "schema"))]
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(CallContract))
        .expect("Win32 contract schema is serializable")
}

#[cfg(test)]
mod tests;
