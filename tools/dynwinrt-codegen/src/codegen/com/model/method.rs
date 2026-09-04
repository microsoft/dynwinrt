// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::ComModel;
use super::abi::{
    BufferElement, BufferElementOwnership, CallingConvention, ComAbiType, Constness, ScalarType,
};
use super::contract::{BufferSizing, ComParamContract, CountRelation};
use super::diagnostics::ModelError;
use super::ids::{ComGuid, ParamIndex, TypeId};
use super::ownership::{Cleanup, ComOwnership};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum ComReturnKind {
    HResult,
    SemanticHResult,
    EnumeratorNextHResult,
    DirectValue(TypeId),
    DirectPointer(TypeId),
    Void,
}

impl ComReturnKind {
    fn abi_type(self) -> Option<TypeId> {
        match self {
            Self::DirectValue(abi_type) | Self::DirectPointer(abi_type) => Some(abi_type),
            Self::HResult | Self::SemanticHResult | Self::EnumeratorNextHResult | Self::Void => {
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum ComMethodSpecialContract {
    FixedCapacityBytes {
        guid_param: ParamIndex,
    },
    FlagSelectedString {
        discriminator_param: ParamIndex,
        reserved_null_param: ParamIndex,
        buffer_param: ParamIndex,
        capacity_param: ParamIndex,
        string_flags: [u32; 2],
        validation_flag: u32,
    },
    ConditionalInterfaceOutput {
        public_input_params: [Option<ParamIndex>; 3],
        flags_param: ParamIndex,
        context_param: ParamIndex,
        synchronous_output: Option<ParamIndex>,
        semisynchronous_output: Option<ParamIndex>,
        synchronous_flags: i32,
        semisynchronous_flags: i32,
    },
    DataObjectSetData {
        release_param: ParamIndex,
    },
    AudioFormatSupport {
        share_mode_param: ParamIndex,
        closest_match_param: ParamIndex,
    },
    Malloc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) struct DynamicIidContract {
    iid_param_index: ParamIndex,
    output_param_index: ParamIndex,
}

impl DynamicIidContract {
    pub(super) fn new(
        iid_param_index: ParamIndex,
        output_param_index: ParamIndex,
    ) -> Result<Self, ModelError> {
        if iid_param_index == output_param_index {
            return Err(ModelError::InvalidContract(
                "dynamic-IID source and output parameters must be distinct".into(),
            ));
        }
        Ok(Self {
            iid_param_index,
            output_param_index,
        })
    }

    pub(in crate::codegen::com) const fn iid_param_index(self) -> ParamIndex {
        self.iid_param_index
    }

    pub(in crate::codegen::com) const fn output_param_index(self) -> ParamIndex {
        self.output_param_index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct ComMethodContract {
    name: String,
    interface_iid: ComGuid,
    vtable_slot: usize,
    calling_convention: CallingConvention,
    params: Vec<ComParamContract>,
    return_kind: ComReturnKind,
    special_contract: Option<ComMethodSpecialContract>,
    dynamic_iid_contract: Option<DynamicIidContract>,
}

impl ComMethodContract {
    pub(super) fn new(
        name: impl Into<String>,
        interface_iid: ComGuid,
        vtable_slot: usize,
        calling_convention: CallingConvention,
        params: Vec<ComParamContract>,
        return_kind: ComReturnKind,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ModelError::InvalidContract(
                "method name must not be empty".into(),
            ));
        }
        if interface_iid.is_zero() {
            return Err(ModelError::InvalidContract(
                "interface IID must not be zero".into(),
            ));
        }
        if vtable_slot < 3 {
            return Err(ModelError::InvalidContract(format!(
                "COM method `{name}` cannot occupy reserved IUnknown slot {vtable_slot}"
            )));
        }
        for (index, param) in params.iter().enumerate() {
            param.validate_param_references(ParamIndex::new(index), params.len())?;
        }
        validate_relationship_directions(&params)?;
        Ok(Self {
            name,
            interface_iid,
            vtable_slot,
            calling_convention,
            params,
            return_kind,
            special_contract: None,
            dynamic_iid_contract: None,
        })
    }

    pub(super) fn validate(&self, model: &ComModel) -> Result<(), ModelError> {
        for param in &self.params {
            (|| {
                model.require_supported_type(param.abi_type())?;
                let abi = model.types().get(param.abi_type())?.abi();
                let is_counted_buffer = matches!(abi, ComAbiType::CountedBuffer { .. });
                if is_counted_buffer != param.count().is_some() {
                    return Err(ModelError::InvalidContract(
                        "CountedBuffer ABI semantics require exactly one count relationship".into(),
                    ));
                }
                if param.direction().is_output()
                    && !matches!(
                        abi,
                        ComAbiType::Pointer {
                            constness: Constness::Mutable,
                            ..
                        } | ComAbiType::DataPointer {
                            constness: Constness::Mutable,
                            ..
                        } | ComAbiType::CountedBuffer {
                            constness: Constness::Mutable,
                            ..
                        }
                        | ComAbiType::StatStg
                        | ComAbiType::FormatEtc
                        | ComAbiType::StgMedium
                        | ComAbiType::AudioFormat
                    )
                {
                    return Err(ModelError::InvalidContract(format!(
                        "[out] or [in, out] parameter `{}` requires writable native pointer indirection",
                        param.name()
                    )));
                }
                if matches!(
                    abi,
                    ComAbiType::CountedBuffer {
                        element: BufferElement::Opaque(_),
                        ..
                    }
                ) {
                    return Err(ModelError::Unsupported(
                        super::diagnostics::UnsupportedReason::UnknownLayout,
                    ));
                }
                if let ComAbiType::CountedBuffer {
                    element,
                    element_ownership,
                    pointer_depth,
                    constness,
                    ..
                } = model.types().get(param.abi_type())?.abi()
                {
                    match (element, element_ownership, param.count(), param.direction()) {
                        (
                            BufferElement::StringPointer { .. },
                            BufferElementOwnership::Borrowed,
                            Some(CountRelation::InputCount { .. }),
                            super::contract::Direction::In,
                        ) => {}
                        (
                            BufferElement::Typed(_),
                            BufferElementOwnership::ComOwned
                            | BufferElementOwnership::BstrOwned
                            | BufferElementOwnership::VariantOwned,
                            Some(CountRelation::EnumeratorNext { .. }),
                            super::contract::Direction::Out,
                        ) => {}
                        (
                            BufferElement::StringPointer { .. },
                            BufferElementOwnership::CoTaskMemStringOwned,
                            Some(CountRelation::EnumeratorNext { .. }),
                            super::contract::Direction::Out,
                        ) => {}
                        (
                            BufferElement::Typed(_),
                            BufferElementOwnership::ComOwned
                            | BufferElementOwnership::BstrOwned
                            | BufferElementOwnership::VariantOwned,
                            Some(CountRelation::InputCount { .. }),
                            super::contract::Direction::In,
                        ) => {}
                        (
                            BufferElement::Typed(_),
                            BufferElementOwnership::ComOwned
                            | BufferElementOwnership::BstrOwned
                            | BufferElementOwnership::VariantOwned,
                            Some(CountRelation::CallerCapacity { .. }),
                            super::contract::Direction::Out,
                        ) => {}
                        (_, BufferElementOwnership::Plain, _, _) => {}
                        _ => {
                            return Err(ModelError::Unsupported(
                                super::diagnostics::UnsupportedReason::Other(format!(
                                    "owned counted-buffer elements or unknown element ownership ({element_ownership:?}) require explicit initialized-range cleanup"
                                )),
                            ));
                        }
                    }
                    if let BufferElement::Typed(element) = element
                        && matches!(model.types().get(*element)?.abi(), ComAbiType::NativeStruct(_))
                        && !matches!(
                            param.count(),
                            Some(
                                CountRelation::InputCount { .. }
                                    | CountRelation::EnumeratorNext { .. }
                            )
                        )
                    {
                        return Err(ModelError::Unsupported(
                            super::diagnostics::UnsupportedReason::Other(
                                "native struct counted buffers currently support input contracts only"
                                    .into(),
                            ),
                        ));
                    }
                    if let BufferElement::StringPointer {
                        pointer_depth,
                        constness: element_constness,
                        ..
                    } = element
                        && (pointer_depth.get() != 1
                            || *element_constness == Constness::Unspecified)
                    {
                        return Err(ModelError::InvalidContract(
                            "string array elements require one pointer level with explicit constness"
                                .into(),
                        ));
                    }
                    match param.count().expect("counted buffer relation") {
                        CountRelation::InputCount { .. } => {
                            if pointer_depth.get() != 1 {
                                return Err(ModelError::InvalidContract(
                                    "input counted buffers require an authoritative T* contract"
                                        .into(),
                                ));
                            }
                            if !matches!(
                                (param.ownership(), param.cleanup()),
                                (ComOwnership::Borrowed, Cleanup::None)
                            ) {
                                return Err(ModelError::InvalidOwnership(
                                        "input counted buffers must be borrowed".into(),
                                ));
                            }
                        }
                        CountRelation::CallerCapacity { .. } => {
                            if pointer_depth.get() != 1 || *constness != Constness::Mutable {
                                return Err(ModelError::InvalidContract(
                                    "caller-owned output buffers require an authoritative mutable T* contract"
                                        .into(),
                                ));
                            }
                            if !matches!(
                                (param.ownership(), param.cleanup()),
                                (ComOwnership::Borrowed, Cleanup::None)
                            ) {
                                return Err(ModelError::InvalidOwnership(
                                        "caller-owned output buffers cannot transfer allocator ownership"
                                            .into(),
                                ));
                            }
                        }
                        CountRelation::EnumeratorNext { .. } => {
                            if pointer_depth.get() != 1 || *constness != Constness::Mutable {
                                return Err(ModelError::InvalidContract(
                                    "IEnum::Next values require an authoritative mutable T* contract"
                                        .into(),
                                ));
                            }
                            if !matches!(
                                (param.ownership(), param.cleanup()),
                                (ComOwnership::Borrowed, Cleanup::None)
                            ) {
                                return Err(ModelError::InvalidOwnership(
                                    "IEnum::Next transfers element ownership, not buffer ownership"
                                        .into(),
                                ));
                            }
                        }
                        CountRelation::CalleeAllocated { .. } => {
                            if pointer_depth.get() != 2 || *constness != Constness::Mutable {
                                return Err(ModelError::InvalidContract(
                                    "callee-allocated buffers require an authoritative mutable T** contract"
                                        .into(),
                                ));
                            }
                            if !matches!(
                                (param.ownership(), param.cleanup()),
                                (
                                        ComOwnership::CoTaskMemOwned,
                                        Cleanup::CoTaskMemFree
                                )
                            ) {
                                return Err(ModelError::Unsupported(
                                        super::diagnostics::UnsupportedReason::UnknownOwnership,
                                ));
                            }
                            if !matches!(element_ownership, BufferElementOwnership::Plain) {
                                return Err(ModelError::Unsupported(
                                        super::diagnostics::UnsupportedReason::UnknownOwnership,
                                ));
                            }
                        }
                    }
                    if matches!(element, BufferElement::Typed(_)) {
                        for count_index in
                            count_param_indices(param.count().expect("counted buffer relation"))
                        {
                            require_integer_count_type(
                                model,
                                &self.params[count_index.index()],
                            )?;
                        }
                    }
                }
                if let Cleanup::Custom(cleanup_id) = param.cleanup() {
                    model.cleanups().get(*cleanup_id)?;
                }
                Ok(())
            })()
            .map_err(|error: ModelError| error.context(param.name()))?;
        }
        validate_shared_count_groups(model, &self.params)?;
        self.validate_enumerator_next(model)?;
        self.validate_dynamic_iid(model)?;
        if let Some(abi_type) = self.return_kind.abi_type() {
            model.require_supported_type(abi_type)?;
            let pointer_return = model
                .types()
                .get(abi_type)?
                .abi()
                .requires_pointer_return_convention();
            match self.return_kind {
                ComReturnKind::DirectPointer(_) if !pointer_return => {
                    return Err(ModelError::InvalidContract(format!(
                        "{} direct pointer return uses a non-pointer ABI type",
                        self.name
                    )));
                }
                ComReturnKind::DirectValue(_) if pointer_return => {
                    return Err(ModelError::InvalidContract(format!(
                        "{} direct value return uses a pointer-return ABI type",
                        self.name
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(in crate::codegen::com) fn name(&self) -> &str {
        &self.name
    }

    pub(super) const fn interface_iid(&self) -> ComGuid {
        self.interface_iid
    }

    pub(in crate::codegen::com) const fn vtable_slot(&self) -> usize {
        self.vtable_slot
    }

    pub(super) const fn calling_convention(&self) -> CallingConvention {
        self.calling_convention
    }

    pub(in crate::codegen::com) fn params(&self) -> &[ComParamContract] {
        &self.params
    }

    pub(in crate::codegen::com) const fn return_kind(&self) -> ComReturnKind {
        self.return_kind
    }

    pub(in crate::codegen::com) const fn special_contract(
        &self,
    ) -> Option<ComMethodSpecialContract> {
        self.special_contract
    }

    pub(in crate::codegen::com) const fn dynamic_iid_contract(&self) -> Option<DynamicIidContract> {
        self.dynamic_iid_contract
    }

    pub(super) fn with_special_contract(
        mut self,
        special_contract: ComMethodSpecialContract,
    ) -> Self {
        self.special_contract = Some(special_contract);
        self
    }

    pub(super) fn with_dynamic_iid_contract(
        mut self,
        contract: DynamicIidContract,
    ) -> Result<Self, ModelError> {
        if contract.iid_param_index().index() >= self.params.len()
            || contract.output_param_index().index() >= self.params.len()
        {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID parameter indices are outside its {} parameters",
                self.name,
                self.params.len()
            )));
        }
        self.dynamic_iid_contract = Some(contract);
        Ok(self)
    }

    fn validate_dynamic_iid(&self, model: &ComModel) -> Result<(), ModelError> {
        let Some(contract) = self.dynamic_iid_contract else {
            return Ok(());
        };
        if self.return_kind != ComReturnKind::HResult {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID methods require an HRESULT return",
                self.name
            )));
        }
        if self
            .params
            .iter()
            .any(|param| param.direction() == super::contract::Direction::InOut)
        {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID methods cannot contain [in, out] parameters",
                self.name
            )));
        }

        let iid_index = contract.iid_param_index().index();
        let output_index = contract.output_param_index().index();
        let iid = &self.params[iid_index];
        let output = &self.params[output_index];
        if iid.direction() != super::contract::Direction::In
            || iid.optional()
            || iid.nullability() != super::contract::Nullability::Required
            || iid.count().is_some()
            || !matches!(
                (iid.ownership(), iid.cleanup()),
                (ComOwnership::Borrowed, Cleanup::None)
            )
            || !matches!(iid.name().to_ascii_lowercase().as_str(), "iid" | "riid")
        {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID source `{}` must be one required borrowed iid/riid input",
                self.name,
                iid.name()
            )));
        }
        let iid_type = model.types().get(iid.abi_type())?.abi();
        let ComAbiType::Pointer {
            pointee,
            depth,
            constness: Constness::Const,
        } = iid_type
        else {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID source `{}` must be a const GUID*",
                self.name,
                iid.name()
            )));
        };
        if depth.get() != 1 || !matches!(model.types().get(*pointee)?.abi(), ComAbiType::Guid) {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID source `{}` must be a const GUID*",
                self.name,
                iid.name()
            )));
        }

        if output.direction() != super::contract::Direction::Out
            || output.optional()
            || output.nullability() != super::contract::Nullability::Required
            || output.count().is_some()
            || !matches!(
                (output.ownership(), output.cleanup()),
                (ComOwnership::ComOwned, Cleanup::ComRelease)
            )
        {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID output `{}` must be one required +1 COM output",
                self.name,
                output.name()
            )));
        }
        if !matches!(
            model.types().get(output.abi_type())?.abi(),
            ComAbiType::DataPointer {
                pointee: super::abi::DataPointee::Opaque(_),
                depth,
                constness: Constness::Mutable,
            } if depth.get() == 2
        ) {
            return Err(ModelError::InvalidContract(format!(
                "{} dynamic-IID output `{}` must be a mutable void**/Object**",
                self.name,
                output.name()
            )));
        }

        for (index, param) in self.params.iter().enumerate() {
            if index != iid_index
                && matches!(param.name().to_ascii_lowercase().as_str(), "iid" | "riid")
                && matches!(
                    model.types().get(param.abi_type())?.abi(),
                    ComAbiType::Pointer { pointee, .. }
                        if matches!(model.types().get(*pointee)?.abi(), ComAbiType::Guid)
                )
            {
                return Err(ModelError::InvalidContract(format!(
                    "{} has a competing IID parameter `{}`",
                    self.name,
                    param.name()
                )));
            }
            if index != output_index
                && param.direction().is_output()
                && matches!(
                    model.types().get(param.abi_type())?.abi(),
                    ComAbiType::DataPointer {
                        pointee: super::abi::DataPointee::Opaque(_),
                        depth,
                        ..
                    } if depth.get() == 2
                )
            {
                return Err(ModelError::InvalidContract(format!(
                    "{} has a competing void**/Object** output `{}`",
                    self.name,
                    param.name()
                )));
            }
        }
        Ok(())
    }

    fn validate_enumerator_next(&self, model: &ComModel) -> Result<(), ModelError> {
        let relation = self.params.iter().enumerate().find_map(|(index, param)| {
            matches!(param.count(), Some(CountRelation::EnumeratorNext { .. }))
                .then_some((index, param))
        });
        let Some((values_index, values)) = relation else {
            if self.return_kind == ComReturnKind::EnumeratorNextHResult {
                return Err(ModelError::InvalidContract(
                    "EnumeratorNext HRESULT requires an EnumeratorNext values parameter".into(),
                ));
            }
            return Ok(());
        };
        let CountRelation::EnumeratorNext {
            capacity_param,
            fetched_param,
            fetched_optional_for_single,
        } = values.count().expect("matched EnumeratorNext")
        else {
            unreachable!()
        };
        if self.return_kind != ComReturnKind::EnumeratorNextHResult
            || self.name != "Next"
            || self.params.len() != 3
            || values_index != 1
            || capacity_param.index() != 0
            || fetched_param.index() != 2
            || values.direction() != super::contract::Direction::Out
        {
            return Err(ModelError::InvalidContract(
                "EnumeratorNext is restricted to an exact proven IEnum*::Next ABI shape".into(),
            ));
        }
        let capacity = &self.params[capacity_param.index()];
        let fetched = &self.params[fetched_param.index()];
        if capacity.direction() != super::contract::Direction::In
            || fetched.direction() != super::contract::Direction::Out
            || fetched.optional() != *fetched_optional_for_single
        {
            return Err(ModelError::InvalidContract(
                "IEnum::Next capacity/fetched directions or optionality do not match the proven contract"
                    .into(),
            ));
        }
        require_exact_u32(model, capacity)?;
        require_exact_u32_pointer(model, fetched)?;
        Ok(())
    }
}

fn validate_relationship_directions(params: &[ComParamContract]) -> Result<(), ModelError> {
    for param in params {
        match param.count() {
            Some(CountRelation::InputCount {
                count_param,
                actual_length_param,
                ..
            }) => {
                require_input(params, *count_param, "count")?;
                if let Some(actual_length_param) = actual_length_param {
                    let actual = &params[actual_length_param.index()];
                    let valid_direction = if actual_length_param == count_param {
                        actual.direction() == super::contract::Direction::InOut
                    } else {
                        actual.direction() == super::contract::Direction::Out
                    };
                    if !valid_direction {
                        return Err(ModelError::InvalidContract(format!(
                            "actual-length parameter `{}` must be Out, or InOut only when it is the shared input-count/actual slot",
                            actual.name(),
                        )));
                    }
                }
            }
            Some(CountRelation::CalleeAllocated { count_param, .. }) => {
                require_output(params, *count_param, "callee-allocated count")?;
            }
            Some(CountRelation::CallerCapacity {
                capacity_param,
                actual_length_param,
                sizing,
                ..
            }) => {
                require_input(params, *capacity_param, "capacity")?;
                if let Some(actual_length_param) = actual_length_param {
                    let actual = &params[actual_length_param.index()];
                    let valid_direction = if actual_length_param == capacity_param {
                        actual.direction() == super::contract::Direction::InOut
                    } else {
                        actual.direction() == super::contract::Direction::Out
                    };
                    if !valid_direction {
                        return Err(ModelError::InvalidContract(format!(
                            "actual-length parameter `{}` must be Out, or InOut only when it is the shared capacity/actual slot",
                            actual.name(),
                        )));
                    }
                }
                if matches!(sizing, BufferSizing::TwoCall { max_retries: 0 }) {
                    return Err(ModelError::InvalidContract(
                        "two-call sizing requires at least one bounded retry".into(),
                    ));
                }
            }
            Some(CountRelation::EnumeratorNext {
                capacity_param,
                fetched_param,
                ..
            }) => {
                require_input(params, *capacity_param, "enumerator capacity")?;
                require_output(params, *fetched_param, "enumerator fetched count")?;
            }
            None => {}
        }
    }
    Ok(())
}

fn validate_shared_count_groups(
    model: &ComModel,
    params: &[ComParamContract],
) -> Result<(), ModelError> {
    let mut owners = vec![Vec::new(); params.len()];
    for (buffer_index, param) in params.iter().enumerate() {
        let Some(relation) = param.count() else {
            continue;
        };
        let mut local = std::collections::BTreeSet::new();
        for index in count_param_indices(relation) {
            if local.insert(index.index()) {
                owners[index.index()].push(buffer_index);
            }
        }
    }
    for (count_index, buffers) in owners.iter().enumerate() {
        if buffers.len() <= 1 {
            continue;
        }
        let shared_input_unit = buffers
            .iter()
            .map(|&buffer_index| {
                let param = &params[buffer_index];
                match (param.direction(), param.count()) {
                    (
                        super::contract::Direction::In,
                        Some(CountRelation::InputCount {
                            count_param,
                            actual_length_param: None,
                            unit,
                        }),
                    ) if count_param.index() == count_index => Some(*unit),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>();
        if shared_input_unit.is_some_and(|units| {
            units
                .first()
                .is_some_and(|first| units.iter().all(|unit| unit == first))
        }) {
            continue;
        }
        let mut parallel_inputs = 0usize;
        let mut parallel_outputs = 0usize;
        let parallel = buffers.iter().all(|&buffer_index| {
            let param = &params[buffer_index];
            match (param.direction(), param.count()) {
                (
                    super::contract::Direction::In,
                    Some(CountRelation::InputCount {
                        count_param,
                        actual_length_param: None,
                        unit: super::contract::CountUnit::Elements,
                    }),
                ) if count_param.index() == count_index => {
                    parallel_inputs += 1;
                    true
                }
                (
                    super::contract::Direction::Out,
                    Some(CountRelation::CallerCapacity {
                        capacity_param,
                        actual_length_param: None,
                        unit: super::contract::CountUnit::Elements,
                        sizing: BufferSizing::SingleCall,
                    }),
                ) if capacity_param.index() == count_index => {
                    parallel_outputs += 1;
                    true
                }
                _ => false,
            }
        });
        if parallel && parallel_inputs != 0 && parallel_outputs != 0 {
            continue;
        }
        if buffers.len() != 2 {
            return Err(ModelError::InvalidContract(format!(
                "count parameter {count_index} describes an ambiguous buffer group"
            )));
        }
        let mut string_input = false;
        let mut scalar_output = false;
        for &buffer_index in buffers {
            let param = &params[buffer_index];
            let ComAbiType::CountedBuffer {
                element,
                element_ownership,
                pointer_depth,
                constness,
            } = model.types().get(param.abi_type())?.abi()
            else {
                return Err(ModelError::InvalidContract(
                    "shared count owner is not a counted buffer".into(),
                ));
            };
            match (param.direction(), param.count(), element, element_ownership) {
                (
                    super::contract::Direction::In,
                    Some(CountRelation::InputCount {
                        count_param,
                        actual_length_param: None,
                        unit: super::contract::CountUnit::Elements,
                    }),
                    BufferElement::StringPointer { .. },
                    BufferElementOwnership::Borrowed,
                ) if count_param.index() == count_index
                    && pointer_depth.get() == 1
                    && !string_input =>
                {
                    string_input = true;
                }
                (
                    super::contract::Direction::Out,
                    Some(CountRelation::CallerCapacity {
                        capacity_param,
                        actual_length_param: None,
                        unit: super::contract::CountUnit::Elements,
                        sizing: BufferSizing::SingleCall,
                    }),
                    BufferElement::Typed(element),
                    BufferElementOwnership::Plain,
                ) if capacity_param.index() == count_index
                    && pointer_depth.get() == 1
                    && *constness == Constness::Mutable
                    && matches!(
                        model.types().get(*element)?.abi(),
                        ComAbiType::Scalar(_) | ComAbiType::Enum(_)
                    )
                    && !scalar_output =>
                {
                    scalar_output = true;
                }
                _ => {
                    return Err(ModelError::InvalidContract(format!(
                        "count parameter {count_index} describes unrelated buffers"
                    )));
                }
            }
        }
        if !(string_input && scalar_output) {
            return Err(ModelError::InvalidContract(format!(
                "count parameter {count_index} does not form one string-input/scalar-output group"
            )));
        }
    }
    Ok(())
}

fn count_param_indices(relation: &CountRelation) -> impl Iterator<Item = ParamIndex> {
    let (first, second) = match relation {
        CountRelation::InputCount {
            count_param,
            actual_length_param,
            ..
        } => (*count_param, *actual_length_param),
        CountRelation::CallerCapacity {
            capacity_param,
            actual_length_param,
            ..
        } => (*capacity_param, *actual_length_param),
        CountRelation::EnumeratorNext {
            capacity_param,
            fetched_param,
            ..
        } => (*capacity_param, Some(*fetched_param)),
        CountRelation::CalleeAllocated { count_param, .. } => (*count_param, None),
    };
    std::iter::once(first).chain(second)
}

fn require_integer_count_type(
    model: &ComModel,
    param: &ComParamContract,
) -> Result<(), ModelError> {
    let mut abi = model.types().get(param.abi_type())?.abi();
    if param.direction().is_output()
        && let ComAbiType::Pointer { pointee, depth, .. } = abi
        && depth.get() == 1
    {
        abi = model.types().get(*pointee)?.abi();
    }

    if matches!(
        abi,
        ComAbiType::Scalar(
            ScalarType::I8
                | ScalarType::U8
                | ScalarType::I16
                | ScalarType::U16
                | ScalarType::I32
                | ScalarType::U32
                | ScalarType::I64
                | ScalarType::U64
                | ScalarType::NativeIsize
                | ScalarType::NativeUsize
        )
    ) {
        Ok(())
    } else {
        Err(ModelError::InvalidContract(format!(
            "count parameter `{}` must use an integer scalar ABI",
            param.name()
        )))
    }
}

fn require_exact_u32(model: &ComModel, param: &ComParamContract) -> Result<(), ModelError> {
    if matches!(
        model.types().get(param.abi_type())?.abi(),
        ComAbiType::Scalar(ScalarType::U32)
    ) {
        Ok(())
    } else {
        Err(ModelError::InvalidContract(format!(
            "IEnum::Next capacity `{}` must use ULONG/u32",
            param.name()
        )))
    }
}

fn require_exact_u32_pointer(model: &ComModel, param: &ComParamContract) -> Result<(), ModelError> {
    let ComAbiType::Pointer {
        pointee,
        depth,
        constness,
    } = model.types().get(param.abi_type())?.abi()
    else {
        return Err(ModelError::InvalidContract(format!(
            "IEnum::Next fetched count `{}` must use ULONG*",
            param.name()
        )));
    };
    if depth.get() == 1
        && *constness == Constness::Mutable
        && matches!(
            model.types().get(*pointee)?.abi(),
            ComAbiType::Scalar(ScalarType::U32)
        )
    {
        Ok(())
    } else {
        Err(ModelError::InvalidContract(format!(
            "IEnum::Next fetched count `{}` must use mutable ULONG*",
            param.name()
        )))
    }
}

fn require_input(
    params: &[ComParamContract],
    index: ParamIndex,
    role: &str,
) -> Result<(), ModelError> {
    let param = &params[index.index()];
    if !param.direction().is_input() {
        return Err(ModelError::InvalidContract(format!(
            "{role} parameter `{}` must be an input",
            param.name()
        )));
    }
    Ok(())
}

fn require_output(
    params: &[ComParamContract],
    index: ParamIndex,
    role: &str,
) -> Result<(), ModelError> {
    let param = &params[index.index()];
    if !param.direction().is_output() {
        return Err(ModelError::InvalidContract(format!(
            "{role} parameter `{}` must be an output",
            param.name()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::*;
    use crate::codegen::com::model::abi::{
        ComTypeDefinition, Constness, HandleKind, QualifiedName, ScalarType,
    };
    use crate::codegen::com::model::contract::{CountUnit, Direction, Nullability};
    use crate::codegen::com::model::ownership::{Cleanup, ComOwnership};

    fn param(
        name: &str,
        abi_type: TypeId,
        direction: Direction,
        count: Option<CountRelation>,
    ) -> ComParamContract {
        ComParamContract::new(
            name,
            abi_type,
            direction,
            false,
            Nullability::Required,
            count,
            ComOwnership::Borrowed,
            Cleanup::None,
        )
        .unwrap()
    }

    #[test]
    fn method_contract_validates_count_directions_and_types() {
        let mut model = ComModel::default();
        let byte = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::U8),
            ))
            .unwrap();
        let buffer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Example", "BYTE_BUFFER").unwrap()),
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::Typed(byte),
                    element_ownership: BufferElementOwnership::Plain,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        let count = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::U32),
            ))
            .unwrap();
        let method = ComMethodContract::new(
            "Write",
            ComGuid::from_bytes([1; 16]),
            3,
            CallingConvention::System,
            vec![
                param(
                    "buffer",
                    buffer,
                    Direction::In,
                    Some(CountRelation::InputCount {
                        count_param: ParamIndex::new(1),
                        actual_length_param: None,
                        unit: CountUnit::Bytes,
                    }),
                ),
                param("count", count, Direction::In, None),
            ],
            ComReturnKind::HResult,
        )
        .unwrap();

        method.validate(&model).unwrap();
        assert_eq!(method.name(), "Write");
        assert_eq!(method.interface_iid(), ComGuid::from_bytes([1; 16]));
        assert_eq!(method.vtable_slot(), 3);
        assert_eq!(method.calling_convention(), CallingConvention::System);
        assert_eq!(method.params().len(), 2);
        assert_eq!(method.return_kind(), ComReturnKind::HResult);
    }

    #[test]
    fn direct_pointer_returns_require_pointer_semantics() {
        let mut model = ComModel::default();
        let scalar = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::NativeUsize),
            ))
            .unwrap();
        let pointer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Pointer {
                    pointee: scalar,
                    depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let pointer_method = ComMethodContract::new(
            "Alloc",
            ComGuid::from_bytes([2; 16]),
            3,
            CallingConvention::Stdcall,
            vec![],
            ComReturnKind::DirectPointer(pointer),
        )
        .unwrap();
        pointer_method.validate(&model).unwrap();

        let invalid = ComMethodContract::new(
            "Invalid",
            ComGuid::from_bytes([2; 16]),
            4,
            CallingConvention::Stdcall,
            vec![],
            ComReturnKind::DirectPointer(scalar),
        )
        .unwrap();
        assert!(invalid.validate(&model).is_err());

        let hwnd = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Windows.Win32.Foundation", "HWND").unwrap()),
                None,
                ComAbiType::Handle(HandleKind::new(
                    QualifiedName::new("Windows.Win32.Foundation", "HWND").unwrap(),
                )),
            ))
            .unwrap();
        let handle_method = ComMethodContract::new(
            "GetWindow",
            ComGuid::from_bytes([2; 16]),
            5,
            CallingConvention::Stdcall,
            vec![],
            ComReturnKind::DirectValue(hwnd),
        )
        .unwrap();
        handle_method.validate(&model).unwrap();
    }

    #[test]
    fn dynamic_iid_semantics_require_const_guid_and_owned_com_output() {
        let mut model = ComModel::default();
        let guid = model
            .types_mut()
            .insert(ComTypeDefinition::new(None, None, ComAbiType::Guid))
            .unwrap();
        let guid_pointer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Pointer {
                    pointee: guid,
                    depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        let object_output = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::DataPointer {
                    pointee: super::super::abi::DataPointee::Opaque(None),
                    depth: NonZeroU8::new(2).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let iid = ComParamContract::new(
            "riid",
            guid_pointer,
            Direction::In,
            false,
            Nullability::Required,
            None,
            ComOwnership::Borrowed,
            Cleanup::None,
        )
        .unwrap();
        let output = ComParamContract::new(
            "object",
            object_output,
            Direction::Out,
            false,
            Nullability::Required,
            None,
            ComOwnership::ComOwned,
            Cleanup::ComRelease,
        )
        .unwrap();
        let contract = DynamicIidContract::new(ParamIndex::new(0), ParamIndex::new(1)).unwrap();
        let method = ComMethodContract::new(
            "Resolve",
            ComGuid::from_bytes([1; 16]),
            3,
            CallingConvention::System,
            vec![iid.clone(), output],
            ComReturnKind::HResult,
        )
        .unwrap()
        .with_dynamic_iid_contract(contract)
        .unwrap();
        method.validate(&model).unwrap();

        let borrowed_output = ComParamContract::new(
            "object",
            object_output,
            Direction::Out,
            false,
            Nullability::Required,
            None,
            ComOwnership::Borrowed,
            Cleanup::None,
        )
        .unwrap();
        let borrowed = ComMethodContract::new(
            "ResolveBorrowed",
            ComGuid::from_bytes([1; 16]),
            3,
            CallingConvention::System,
            vec![iid, borrowed_output],
            ComReturnKind::HResult,
        )
        .unwrap()
        .with_dynamic_iid_contract(contract)
        .unwrap();
        assert!(borrowed.validate(&model).is_err());
    }

    #[test]
    fn counted_buffers_cannot_lose_their_count_contract() {
        let mut model = ComModel::default();
        let byte = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::U8),
            ))
            .unwrap();
        let buffer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::Typed(byte),
                    element_ownership: BufferElementOwnership::Plain,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        let method = ComMethodContract::new(
            "Invalid",
            ComGuid::from_bytes([2; 16]),
            3,
            CallingConvention::System,
            vec![param("buffer", buffer, Direction::In, None)],
            ComReturnKind::HResult,
        )
        .unwrap();

        assert!(method.validate(&model).is_err());
    }

    #[test]
    fn shared_count_groups_accept_parallel_inputs_and_outputs_with_exact_relations() {
        let mut model = ComModel::default();
        let count = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::U32),
            ))
            .unwrap();
        let output_element = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Tests", "DISPID").unwrap()),
                None,
                ComAbiType::Scalar(ScalarType::I32),
            ))
            .unwrap();
        let string_array = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Windows.Win32.Foundation", "PCWSTR").unwrap()),
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::StringPointer {
                        encoding: super::super::abi::StringEncoding::Utf16,
                        pointer_depth: NonZeroU8::new(1).unwrap(),
                        constness: Constness::Const,
                    },
                    element_ownership: BufferElementOwnership::Borrowed,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        let output_array = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Tests", "DISPID").unwrap()),
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::Typed(output_element),
                    element_ownership: BufferElementOwnership::Plain,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let make_method = |input_type| {
            ComMethodContract::new(
                "Resolve",
                ComGuid::from_bytes([4; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "names",
                        input_type,
                        Direction::In,
                        Some(CountRelation::InputCount {
                            count_param: ParamIndex::new(1),
                            actual_length_param: None,
                            unit: CountUnit::Elements,
                        }),
                    ),
                    param("count", count, Direction::In, None),
                    param(
                        "ids",
                        output_array,
                        Direction::Out,
                        Some(CountRelation::CallerCapacity {
                            capacity_param: ParamIndex::new(1),
                            actual_length_param: None,
                            unit: CountUnit::Elements,
                            sizing: BufferSizing::SingleCall,
                        }),
                    ),
                ],
                ComReturnKind::HResult,
            )
            .unwrap()
        };

        make_method(string_array).validate(&model).unwrap();

        let unrelated_input = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::Typed(output_element),
                    element_ownership: BufferElementOwnership::Plain,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        make_method(unrelated_input).validate(&model).unwrap();

        let owning_strings = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::StringPointer {
                        encoding: super::super::abi::StringEncoding::Utf16,
                        pointer_depth: NonZeroU8::new(1).unwrap(),
                        constness: Constness::Const,
                    },
                    element_ownership: BufferElementOwnership::BstrOwned,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        assert!(make_method(owning_strings).validate(&model).is_err());

        let pointer_element = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Pointer {
                    pointee: output_element,
                    depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let pointer_output = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::CountedBuffer {
                    element: BufferElement::Typed(pointer_element),
                    element_ownership: BufferElementOwnership::Unknown,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let pointer_method = ComMethodContract::new(
            "ResolvePointers",
            ComGuid::from_bytes([4; 16]),
            4,
            CallingConvention::System,
            vec![
                param(
                    "names",
                    string_array,
                    Direction::In,
                    Some(CountRelation::InputCount {
                        count_param: ParamIndex::new(1),
                        actual_length_param: None,
                        unit: CountUnit::Elements,
                    }),
                ),
                param("count", count, Direction::In, None),
                param(
                    "outputs",
                    pointer_output,
                    Direction::Out,
                    Some(CountRelation::CallerCapacity {
                        capacity_param: ParamIndex::new(1),
                        actual_length_param: None,
                        unit: CountUnit::Elements,
                        sizing: BufferSizing::SingleCall,
                    }),
                ),
            ],
            ComReturnKind::HResult,
        )
        .unwrap();
        assert!(pointer_method.validate(&model).is_err());
    }

    #[test]
    fn output_directions_require_writable_pointer_indirection() {
        let mut model = ComModel::default();
        let scalar = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Scalar(ScalarType::U32),
            ))
            .unwrap();
        let writable_pointer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Pointer {
                    pointee: scalar,
                    depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Mutable,
                },
            ))
            .unwrap();
        let const_pointer = model
            .types_mut()
            .insert(ComTypeDefinition::new(
                None,
                None,
                ComAbiType::Pointer {
                    pointee: scalar,
                    depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
            ))
            .unwrap();
        let method = |typ| {
            ComMethodContract::new(
                "GetValue",
                ComGuid::from_bytes([3; 16]),
                3,
                CallingConvention::System,
                vec![param("value", typ, Direction::Out, None)],
                ComReturnKind::HResult,
            )
            .unwrap()
        };

        assert!(method(writable_pointer).validate(&model).is_ok());
        assert!(method(scalar).validate(&model).is_err());
        assert!(method(const_pointer).validate(&model).is_err());
    }

    #[test]
    fn callee_allocated_buffer_counts_are_outputs() {
        let buffer_type = TypeId::from_index(0).unwrap();
        let count_type = TypeId::from_index(1).unwrap();
        let make_method = |count_direction| {
            ComMethodContract::new(
                "Read",
                ComGuid::from_bytes([1; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "buffer",
                        buffer_type,
                        Direction::Out,
                        Some(CountRelation::CalleeAllocated {
                            count_param: ParamIndex::new(1),
                            unit: CountUnit::Elements,
                        }),
                    ),
                    param("count", count_type, count_direction, None),
                ],
                ComReturnKind::HResult,
            )
        };

        assert!(make_method(Direction::Out).is_ok());
        assert!(make_method(Direction::In).is_err());
    }

    #[test]
    fn caller_capacity_actual_direction_depends_on_slot_sharing() {
        let buffer_type = TypeId::from_index(0).unwrap();
        let count_type = TypeId::from_index(1).unwrap();
        let distinct = |actual_direction| {
            ComMethodContract::new(
                "Read",
                ComGuid::from_bytes([1; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "buffer",
                        buffer_type,
                        Direction::Out,
                        Some(CountRelation::CallerCapacity {
                            capacity_param: ParamIndex::new(1),
                            actual_length_param: Some(ParamIndex::new(2)),
                            unit: CountUnit::Elements,
                            sizing: BufferSizing::SingleCall,
                        }),
                    ),
                    param("capacity", count_type, Direction::In, None),
                    param("actual", count_type, actual_direction, None),
                ],
                ComReturnKind::HResult,
            )
        };
        assert!(distinct(Direction::Out).is_ok());
        assert!(distinct(Direction::InOut).is_err());

        let shared = |count_direction| {
            ComMethodContract::new(
                "Read",
                ComGuid::from_bytes([1; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "buffer",
                        buffer_type,
                        Direction::Out,
                        Some(CountRelation::CallerCapacity {
                            capacity_param: ParamIndex::new(1),
                            actual_length_param: Some(ParamIndex::new(1)),
                            unit: CountUnit::Elements,
                            sizing: BufferSizing::SingleCall,
                        }),
                    ),
                    param("capacityAndActual", count_type, count_direction, None),
                ],
                ComReturnKind::HResult,
            )
        };
        assert!(shared(Direction::InOut).is_ok());
        assert!(shared(Direction::Out).is_err());
    }

    #[test]
    fn input_count_actual_direction_depends_on_slot_sharing() {
        let buffer_type = TypeId::from_index(0).unwrap();
        let count_type = TypeId::from_index(1).unwrap();
        let distinct = |actual_direction| {
            ComMethodContract::new(
                "Write",
                ComGuid::from_bytes([1; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "buffer",
                        buffer_type,
                        Direction::In,
                        Some(CountRelation::InputCount {
                            count_param: ParamIndex::new(1),
                            actual_length_param: Some(ParamIndex::new(2)),
                            unit: CountUnit::Elements,
                        }),
                    ),
                    param("count", count_type, Direction::In, None),
                    param("actual", count_type, actual_direction, None),
                ],
                ComReturnKind::HResult,
            )
        };
        assert!(distinct(Direction::Out).is_ok());
        assert!(distinct(Direction::InOut).is_err());

        let shared = |count_direction| {
            ComMethodContract::new(
                "Write",
                ComGuid::from_bytes([1; 16]),
                3,
                CallingConvention::System,
                vec![
                    param(
                        "buffer",
                        buffer_type,
                        Direction::In,
                        Some(CountRelation::InputCount {
                            count_param: ParamIndex::new(1),
                            actual_length_param: Some(ParamIndex::new(1)),
                            unit: CountUnit::Elements,
                        }),
                    ),
                    param("countAndActual", count_type, count_direction, None),
                ],
                ComReturnKind::HResult,
            )
        };
        assert!(shared(Direction::InOut).is_ok());
        assert!(shared(Direction::Out).is_err());
    }

    #[test]
    fn iunknown_slots_and_zero_iids_are_rejected() {
        assert!(
            ComMethodContract::new(
                "Reserved",
                ComGuid::from_bytes([1; 16]),
                2,
                CallingConvention::System,
                vec![],
                ComReturnKind::Void,
            )
            .is_err()
        );
        assert!(
            ComMethodContract::new(
                "NoIdentity",
                ComGuid::ZERO,
                3,
                CallingConvention::System,
                vec![],
                ComReturnKind::SemanticHResult,
            )
            .is_err()
        );
    }
}
