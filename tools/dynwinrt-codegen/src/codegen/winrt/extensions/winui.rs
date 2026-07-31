// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Language-neutral policy for the optional WinUI `Application` bootstrap.
//!
//! This module describes which WinUI types and ABI facts are required. Language
//! projectors remain responsible for rendering the helpers.

use std::collections::HashSet;

use crate::meta::ClassMeta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinUiClassRef {
    pub namespace: &'static str,
    pub name: &'static str,
}

impl WinUiClassRef {
    pub const fn new(namespace: &'static str, name: &'static str) -> Self {
        Self { namespace, name }
    }

    pub fn full_name(self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinUiAbiType {
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinUiCallBehavior {
    Default,
    BlockingReentrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinUiBootstrapSpec {
    pub application: WinUiClassRef,
    pub metadata_provider: WinUiClassRef,
    pub controls_resources: WinUiClassRef,
    pub resource_manager: WinUiClassRef,
    pub launched_callback_iid: &'static str,
    pub launched_callback_params: &'static [WinUiAbiType],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedWinUiBootstrap {
    pub spec: &'static WinUiBootstrapSpec,
    pub supports_unpackaged_resources: bool,
}

const LAUNCHED_CALLBACK_PARAMS: &[WinUiAbiType] = &[WinUiAbiType::Object];

pub const APPLICATION_BOOTSTRAP: WinUiBootstrapSpec = WinUiBootstrapSpec {
    application: WinUiClassRef::new("Microsoft.UI.Xaml", "Application"),
    metadata_provider: WinUiClassRef::new(
        "Microsoft.UI.Xaml.XamlTypeInfo",
        "XamlControlsXamlMetaDataProvider",
    ),
    controls_resources: WinUiClassRef::new("Microsoft.UI.Xaml.Controls", "XamlControlsResources"),
    resource_manager: WinUiClassRef::new(
        "Microsoft.Windows.ApplicationModel.Resources",
        "ResourceManager",
    ),
    launched_callback_iid: "f81c4e72-7a18-4a30-9126-6f62b6bdac83",
    launched_callback_params: LAUNCHED_CALLBACK_PARAMS,
};

pub fn is_application(class: &ClassMeta) -> bool {
    class.full_name == APPLICATION_BOOTSTRAP.application.full_name()
}

pub fn call_behavior(interface_name: &str, method_name: &str) -> WinUiCallBehavior {
    match (interface_name, method_name) {
        ("IApplicationStatics", "Start")
        | ("IDispatcherQueue3", "RunEventLoop")
        | ("IDispatcherQueue3", "RunEventLoopWithOptions") => WinUiCallBehavior::BlockingReentrant,
        _ => WinUiCallBehavior::Default,
    }
}

pub fn resolve_application_bootstrap(
    class: &ClassMeta,
    known_types: &HashSet<String>,
) -> Option<ResolvedWinUiBootstrap> {
    let spec = &APPLICATION_BOOTSTRAP;
    (is_application(class)
        && known_types.contains(&spec.metadata_provider.full_name())
        && known_types.contains(&spec.controls_resources.full_name()))
    .then(|| ResolvedWinUiBootstrap {
        spec,
        supports_unpackaged_resources: known_types.contains(&spec.resource_manager.full_name()),
    })
}

pub fn add_implicit_classes(winmd: &str, classes: &mut Vec<ClassMeta>) {
    let spec = &APPLICATION_BOOTSTRAP;
    if !classes.iter().any(is_application) {
        return;
    }

    for reference in [
        spec.metadata_provider,
        spec.controls_resources,
        spec.resource_manager,
    ] {
        let full_name = reference.full_name();
        if classes.iter().any(|class| class.full_name == full_name) {
            continue;
        }
        if let Some(class) = crate::meta::parse_class(winmd, reference.namespace, reference.name) {
            classes.push(class);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_when_required_types_are_available() {
        let application = ClassMeta {
            full_name: APPLICATION_BOOTSTRAP.application.full_name(),
            ..Default::default()
        };
        let required = HashSet::from([
            APPLICATION_BOOTSTRAP.metadata_provider.full_name(),
            APPLICATION_BOOTSTRAP.controls_resources.full_name(),
        ]);

        let resolved = resolve_application_bootstrap(&application, &required)
            .expect("bootstrap should resolve");
        assert!(!resolved.supports_unpackaged_resources);
        assert_eq!(
            resolved.spec.launched_callback_params,
            &[WinUiAbiType::Object]
        );

        let mut unpackaged = required;
        unpackaged.insert(APPLICATION_BOOTSTRAP.resource_manager.full_name());
        assert!(
            resolve_application_bootstrap(&application, &unpackaged)
                .expect("bootstrap should resolve")
                .supports_unpackaged_resources
        );
    }

    #[test]
    fn does_not_resolve_for_other_classes_or_missing_dependencies() {
        let other = ClassMeta {
            full_name: "Contoso.Application".into(),
            ..Default::default()
        };
        assert!(resolve_application_bootstrap(&other, &HashSet::new()).is_none());

        let application = ClassMeta {
            full_name: APPLICATION_BOOTSTRAP.application.full_name(),
            ..Default::default()
        };
        assert!(resolve_application_bootstrap(&application, &HashSet::new()).is_none());
    }

    #[test]
    fn unrelated_short_resource_manager_name_does_not_enable_unpackaged_support() {
        let application = ClassMeta {
            full_name: APPLICATION_BOOTSTRAP.application.full_name(),
            ..Default::default()
        };
        let known = HashSet::from([
            APPLICATION_BOOTSTRAP.metadata_provider.full_name(),
            APPLICATION_BOOTSTRAP.controls_resources.full_name(),
            APPLICATION_BOOTSTRAP.resource_manager.name.to_string(),
        ]);

        assert!(
            !resolve_application_bootstrap(&application, &known)
                .expect("bootstrap should resolve")
                .supports_unpackaged_resources
        );
    }

    #[test]
    fn marks_only_blocking_reentrant_host_calls() {
        assert_eq!(
            call_behavior("IApplicationStatics", "Start"),
            WinUiCallBehavior::BlockingReentrant
        );
        assert_eq!(
            call_behavior("IDispatcherQueue3", "RunEventLoop"),
            WinUiCallBehavior::BlockingReentrant
        );
        assert_eq!(
            call_behavior("IApplicationStatics", "get_Current"),
            WinUiCallBehavior::Default
        );
    }
}
