// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Shared Python codegen helpers used by both `python` (implementation) and
//! `python_stub` (.pyi stub) emitters.

use crate::meta::{InterfaceMeta, MethodMeta};
use crate::types::TypeMeta;

pub(super) const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";

pub(super) fn is_closable(interface: &InterfaceMeta) -> bool {
    interface.generic_piid.is_none() && interface.iid.eq_ignore_ascii_case(ICLOSABLE_IID)
}

pub(super) fn requires_closable(interface: &InterfaceMeta) -> bool {
    if interface.implementation_metadata.is_generic_definition
        || interface.is_delegate()
        || (interface.iid.is_empty() && interface.generic_piid.is_none())
    {
        return false;
    }
    is_closable(interface)
        || interface
            .base_interfaces
            .iter()
            // The parser records the substituted, transitive WinMD requirements here.
            .chain(&interface.implementation_metadata.required_interfaces)
            .any(|typ| {
                matches!(typ, TypeMeta::Interface { iid, .. } if iid.eq_ignore_ascii_case(ICLOSABLE_IID))
            })
}

pub(super) const CLOSE_STUB: &str = "    def close(self) -> None: ...\n";
pub(super) const CONTEXT_MANAGER_STUB: &str = "\
    \x20   def __enter__(self) -> Self: ...\n\
    \x20   def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> Literal[False]: ...\n";

pub(super) const NATIVE_OBJECT_PROTOCOL: &str = "

class _DynWinRTObject(Protocol):
    @property
    def _obj(self) -> DynWinRTValue: ...
";

pub(super) const MAPPING_INPUT_PROTOCOL: &str = "
_MapKeyT = TypeVar('_MapKeyT')
_MapValueT_co = TypeVar('_MapValueT_co', covariant=True)


class _SupportsKeysAndGetItem(Protocol[_MapKeyT, _MapValueT_co]):
    def keys(self) -> Iterable[_MapKeyT]: ...
    def __getitem__(self, key: _MapKeyT, /) -> _MapValueT_co: ...
";

/// Reorder methods so that property getters always come before their matching setters.
/// Python requires `@property` to appear before `@prop.setter`.
pub(crate) fn reorder_getters_before_setters(methods: &[MethodMeta]) -> Vec<&MethodMeta> {
    let mut getters: Vec<&MethodMeta> = Vec::new();
    let mut setters: Vec<&MethodMeta> = Vec::new();
    let mut others: Vec<&MethodMeta> = Vec::new();
    for m in methods {
        if m.is_property_getter {
            getters.push(m);
        } else if m.is_property_setter {
            setters.push(m);
        } else {
            others.push(m);
        }
    }
    let mut result = Vec::with_capacity(methods.len());
    for g in &getters {
        result.push(*g);
        let getter_prop = g.name.strip_prefix("get_").unwrap_or(&g.name);
        if let Some(pos) = setters
            .iter()
            .position(|s| s.name.strip_prefix("put_").unwrap_or(&s.name) == getter_prop)
        {
            result.push(setters.remove(pos));
        }
    }
    result.extend(setters);
    result.extend(others);
    result
}
