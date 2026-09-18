// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use libffi::middle::Type;
use windows_core::{GUID, Interface};

use crate::{
    MethodSignature, TypeHandle, TypeKind,
    native_callback::{CallbackAbiType, CallbackSignature},
    signature::WinRtParameterDirection,
};

use super::{invalid_argument, reserved_iid, values::ValuePlan};

/// The first release deliberately does not marshal language callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinRtThreadingPolicy {
    OwnerThread,
}

/// One complete WinRT method contract, including all output parameters.
#[derive(Debug, Clone)]
pub struct WinRtMethodDefinition {
    pub name: String,
    pub vtable_index: usize,
    pub signature: MethodSignature,
}

/// Metadata for one independently queryable, IInspectable-rooted interface.
///
/// WinRT required interfaces have their own vtables; their methods must not be
/// appended to this interface or aliased to an incompatible vtable prefix.
#[derive(Debug, Clone)]
pub struct WinRtInterfaceDefinition {
    pub name: String,
    pub interface_type: TypeHandle,
    pub required_iids: Vec<GUID>,
    pub methods: Vec<WinRtMethodDefinition>,
}

#[derive(Debug, Clone)]
pub(super) struct ParameterPlan {
    pub abi_index: usize,
    pub direction: WinRtParameterDirection,
    pub value: ValuePlan,
    pub array: bool,
}

#[derive(Debug, Clone)]
pub(super) struct MethodPlan {
    pub name: String,
    pub slot: usize,
    pub parameters: Vec<ParameterPlan>,
    pub signature: CallbackSignature,
    pub input_count: usize,
    pub output_count: usize,
}

#[derive(Debug, Clone)]
pub(super) struct InterfacePlan {
    pub name: String,
    pub iid: GUID,
    pub methods: Vec<MethodPlan>,
}

/// Immutable, validated WinRT reverse-call contracts shared by all bindings.
#[derive(Debug, Clone)]
pub struct WinRtImplementationPlan {
    pub(super) interfaces: Vec<InterfacePlan>,
    pub(super) threading: WinRtThreadingPolicy,
}

impl WinRtImplementationPlan {
    pub fn new(
        definitions: Vec<WinRtInterfaceDefinition>,
        threading: WinRtThreadingPolicy,
    ) -> windows_core::Result<Self> {
        if definitions.is_empty() {
            return Err(invalid_argument(
                "A WinRT implementation requires at least one interface",
            ));
        }
        let mut iids = HashSet::new();
        for definition in &definitions {
            let TypeKind::Interface(iid) = definition.interface_type.kind() else {
                return Err(invalid_argument(format!(
                    "{}: only non-generic IInspectable-rooted interfaces can be implemented",
                    definition.name
                )));
            };
            if definition.name.is_empty() {
                return Err(invalid_argument("WinRT interface names cannot be empty"));
            }
            if reserved_iid(&iid) {
                return Err(invalid_argument(format!(
                    "{}: IID {iid:?} is reserved for the WinRT object infrastructure",
                    definition.name
                )));
            }
            if !iids.insert(iid) {
                return Err(invalid_argument(format!(
                    "Duplicate WinRT implementation IID {iid:?}"
                )));
            }
        }
        let mut required = Vec::with_capacity(definitions.len());
        for definition in &definitions {
            let mut seen = HashSet::new();
            let mut indices = Vec::new();
            for iid in &definition.required_iids {
                if *iid == windows_core::IUnknown::IID || *iid == windows_core::IInspectable::IID {
                    continue;
                }
                if !seen.insert(*iid) {
                    return Err(invalid_argument(format!(
                        "{}: duplicate required interface IID {iid:?}",
                        definition.name
                    )));
                }
                let Some(index) = definitions.iter().position(|candidate| {
                    candidate.interface_type.kind() == TypeKind::Interface(*iid)
                }) else {
                    return Err(invalid_argument(format!(
                        "{} requires a separate implementation of interface {iid:?}",
                        definition.name
                    )));
                };
                indices.push(index);
            }
            required.push(indices);
        }
        fn visit(index: usize, graph: &[Vec<usize>], state: &mut [u8]) -> bool {
            match state[index] {
                1 => return false,
                2 => return true,
                _ => {}
            }
            state[index] = 1;
            for &base in &graph[index] {
                if !visit(base, graph, state) {
                    return false;
                }
            }
            state[index] = 2;
            true
        }
        let mut state = vec![0; definitions.len()];
        for (index, definition) in definitions.iter().enumerate() {
            if !visit(index, &required, &mut state) {
                return Err(invalid_argument(format!(
                    "{}: cyclic WinRT required-interface relationships",
                    definition.name
                )));
            }
        }
        let interfaces = definitions
            .into_iter()
            .map(|definition| {
                let mut methods = Vec::with_capacity(definition.methods.len());
                for (index, method) in definition.methods.into_iter().enumerate() {
                    if method.vtable_index != index + 6 {
                        return Err(invalid_argument(format!(
                            "{}: WinRT methods must occupy every contiguous slot starting at 6; \
                             expected slot {}, found {}",
                            definition.name,
                            index + 6,
                            method.vtable_index
                        )));
                    }
                    if method.name.is_empty() {
                        return Err(invalid_argument(format!(
                            "{}: method at slot {} has no metadata name",
                            definition.name, method.vtable_index
                        )));
                    }
                    methods.push(MethodPlan::new(method).map_err(|error| {
                        invalid_argument(format!("{}: {}", definition.name, error.message()))
                    })?);
                }
                Ok(InterfacePlan {
                    name: definition.name,
                    iid: definition
                        .interface_type
                        .iid()
                        .expect("validated interface IID"),
                    methods,
                })
            })
            .collect::<windows_core::Result<Vec<_>>>()?;
        Ok(Self {
            interfaces,
            threading,
        })
    }

    /// Validate a single descriptor without requiring its companion views yet.
    pub fn validate_interface(definition: &WinRtInterfaceDefinition) -> windows_core::Result<()> {
        let mut standalone = definition.clone();
        standalone.required_iids.clear();
        Self::new(vec![standalone], WinRtThreadingPolicy::OwnerThread).map(|_| ())
    }
}

impl MethodPlan {
    fn new(definition: WinRtMethodDefinition) -> windows_core::Result<Self> {
        let mut parameters = Vec::new();
        let mut native = Vec::new();
        let mut input_count = 0;
        let mut output_count = 0;
        for (index, (typ, direction)) in definition
            .signature
            .implementation_parameters()?
            .into_iter()
            .enumerate()
        {
            let array = typ.is_array();
            if direction == WinRtParameterDirection::FillArray && !array {
                return Err(invalid_argument(format!(
                    "{} parameter {index}: FillArray requires a WinRT array type",
                    definition.name
                )));
            }
            let value = ValuePlan::new(if array { typ.array_element_type() } else { typ })
                .map_err(|error| {
                    invalid_argument(format!(
                        "{} parameter {index}: {}",
                        definition.name,
                        error.message()
                    ))
                })?;
            let abi_index = native.len() + 1;
            if array {
                native.push(if direction == WinRtParameterDirection::Out {
                    (CallbackAbiType::Pointer, Type::pointer())
                } else {
                    (CallbackAbiType::U32, Type::u32())
                });
                native.push((CallbackAbiType::Pointer, Type::pointer()));
            } else if direction == WinRtParameterDirection::Out {
                native.push((CallbackAbiType::Pointer, Type::pointer()));
            } else {
                native.push(value.callback_abi());
            }
            if direction != WinRtParameterDirection::Out {
                input_count += 1;
            }
            if direction != WinRtParameterDirection::In {
                output_count += 1;
            }
            parameters.push(ParameterPlan {
                abi_index,
                direction,
                value,
                array,
            });
        }
        Ok(Self {
            name: definition.name,
            slot: definition.vtable_index,
            parameters,
            signature: CallbackSignature::hresult(native),
            input_count,
            output_count,
        })
    }
}
