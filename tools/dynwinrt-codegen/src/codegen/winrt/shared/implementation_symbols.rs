// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Public projection helpers have identities owned by an interface, not by the
//! spelling of its current alias. Metadata types always reserve their names first.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::meta::InterfaceMeta;

use super::implementation::project_implementation;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ImplementationHelper {
    pub key: String,
    pub suffix: String,
    pub name: String,
    pub implementation_name: String,
}

impl ImplementationHelper {
    pub fn is_valid(&self) -> bool {
        let identifier = |value: &str| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && !value.as_bytes()[0].is_ascii_digit()
        };
        !self.key.is_empty()
            && identifier(&self.suffix)
            && identifier(&self.name)
            && identifier(&self.implementation_name)
    }

    fn new(owner: &str, key: impl Into<String>, suffix: impl Into<String>) -> Self {
        let suffix = suffix.into();
        let name = format!("{owner}{suffix}");
        Self {
            key: key.into(),
            suffix,
            implementation_name: name.clone(),
            name,
        }
    }

    /// Upgrade the previous generator's explicitly exported helper metadata.
    /// This recognizes projection names only; it never infers a native contract.
    pub fn legacy_javascript(owner: &str, symbol: &str) -> Option<Self> {
        let suffix = symbol.strip_prefix(owner)?;
        let key = if suffix == "Handlers" {
            "handlers".into()
        } else if let Some(index) = suffix.strip_prefix("ImplementationDelegate") {
            if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            format!("delegate:{index}")
        } else {
            return None;
        };
        Some(Self::new(owner, key, suffix))
    }
}

pub fn interface_helpers(interface: &InterfaceMeta, python: bool) -> Vec<ImplementationHelper> {
    let Ok(plan) = project_implementation(interface) else {
        return Vec::new();
    };
    let owner = &interface.name;
    let mut helpers = vec![ImplementationHelper::new(owner, "handlers", "Handlers")];
    for (index, delegate) in plan.delegates.iter().enumerate() {
        helpers.push(ImplementationHelper::new(
            owner,
            format!("delegate:{index}"),
            format!("ImplementationDelegate{index}"),
        ));
        if python && delegate.invoke.output_count > 1 {
            helpers.push(ImplementationHelper::new(
                owner,
                format!("delegate-result:{index}"),
                format!("ImplementationDelegate{index}Result"),
            ));
        }
    }
    if python {
        for method in &plan.methods {
            if method.output_count > 1 {
                helpers.push(ImplementationHelper::new(
                    owner,
                    format!("method-result:{}", method.vtable_index),
                    format!("Implementation{}Result", method.name),
                ));
            }
        }
    }
    helpers
}

pub struct HelperOwner {
    pub identity: String,
    pub projected_name: String,
    pub qualified_name: String,
    pub helpers: Vec<ImplementationHelper>,
}

pub fn allocate_helpers(
    owners: impl IntoIterator<Item = HelperOwner>,
    reserved: impl IntoIterator<Item = String>,
    case_insensitive: bool,
) -> BTreeMap<String, Vec<ImplementationHelper>> {
    let normalize = |s: &str| {
        if case_insensitive {
            s.to_ascii_lowercase()
        } else {
            s.into()
        }
    };
    let mut occupied = reserved
        .into_iter()
        .map(|s| normalize(&s))
        .collect::<BTreeSet<_>>();
    let mut owners = owners
        .into_iter()
        .map(|owner| (owner.identity.clone(), owner))
        .collect::<BTreeMap<_, _>>();
    let mut preferred_counts = BTreeMap::<String, usize>::new();
    for owner in owners.values() {
        for helper in &owner.helpers {
            *preferred_counts
                .entry(normalize(&format!(
                    "{}{}",
                    owner.projected_name, helper.suffix
                )))
                .or_default() += 1;
        }
    }
    let mut result = BTreeMap::new();
    for (identity, owner) in &mut owners {
        owner.helpers.sort_by(|a, b| a.key.cmp(&b.key));
        owner.helpers.dedup_by(|a, b| a.key == b.key);
        for helper in &mut owner.helpers {
            let preferred = format!("{}{}", owner.projected_name, helper.suffix);
            let mut name = preferred;
            if preferred_counts[&normalize(&name)] > 1 || occupied.contains(&normalize(&name)) {
                name = format!("{}{}Helper", owner.qualified_name, helper.suffix);
            }
            if occupied.contains(&normalize(&name)) {
                let mut hash = 0xcbf29ce484222325u64;
                for byte in format!("{identity}|{}", helper.key).bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                }
                name = format!("{name}_{hash:016x}");
                while occupied.contains(&normalize(&name)) {
                    name.push('_');
                }
            }
            occupied.insert(normalize(&name));
            helper.name = name;
        }
        result.insert(identity.clone(), owner.helpers.clone());
    }
    result
}
