// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::num::{NonZeroU8, NonZeroU32};

use crate::com_metadata::{
    ComInterfaceMeta, RawComMethod, RawComParam, RawComType, RawConstness, RawCountUnit,
    RawElementOwnership, RawExactMethodContractKind, RawLayoutKind, RawNamedKind, RawNativeLayout,
    RawNativeLayoutSet, RawNativeType, RawPacking, RawParamDirection, RawSafeArrayOwnership,
    RawSafeArrayVartype, RawStringEncoding,
};

use super::ComModel;
use super::abi::{
    BufferElement, BufferElementOwnership, CallingConvention, ComAbiType, ComEnumDefinition,
    ComEnumMember, ComEnumValue as SemanticEnumValue, ComTypeDefinition, Constness, DataPointee,
    HandleKind, QualifiedName, ScalarType, StringEncoding,
};
use super::contract::{
    BufferSizing, ComParamContract, CountRelation, CountUnit, Direction, Nullability,
};
use super::diagnostics::{ModelError, UnsupportedReason};
use super::ids::{ComGuid, EnumId, ParamIndex, TypeId};
use super::layout::{
    Architecture, FieldLayout, LayoutKind, NativeLayout, NativeLayoutSet, StructLayout,
};
use super::method::{
    ComMethodContract, ComMethodSpecialContract, ComReturnKind, DynamicIidContract,
};
use super::ownership::{Cleanup, ComOwnership, HandleCleanup};

#[derive(Debug)]
pub(in crate::codegen::com) struct SemanticComInterface {
    model: ComModel,
    iid: ComGuid,
    is_iunknown_rooted: bool,
    methods: Vec<ComMethodContract>,
    referenced_enums: Vec<EnumId>,
}

impl SemanticComInterface {
    fn validate(&self) -> Result<(), ModelError> {
        let first_method_slot = if self.is_iunknown_rooted { 3 } else { 6 };
        let mut slots = std::collections::HashSet::new();
        for method in &self.methods {
            if method.vtable_slot() < first_method_slot {
                return Err(ModelError::InvalidContract(format!(
                    "{} uses reserved vtable slot {} (first user slot is {first_method_slot})",
                    method.name(),
                    method.vtable_slot()
                )));
            }
            if !slots.insert(method.vtable_slot()) {
                return Err(ModelError::InvalidContract(format!(
                    "duplicate vtable slot {} on method {}",
                    method.vtable_slot(),
                    method.name()
                )));
            }
            method
                .validate(&self.model)
                .map_err(|error| error.context(method.name()))?;
        }
        self.model.validate_complete()?;
        Ok(())
    }

    pub(in crate::codegen::com) const fn iid(&self) -> ComGuid {
        self.iid
    }

    pub(in crate::codegen::com) const fn is_iunknown_rooted(&self) -> bool {
        self.is_iunknown_rooted
    }

    pub(in crate::codegen::com) fn methods(&self) -> &[ComMethodContract] {
        &self.methods
    }

    pub(in crate::codegen::com) fn type_definition(
        &self,
        id: TypeId,
    ) -> Result<&ComTypeDefinition, ModelError> {
        self.model.types().get(id)
    }

    pub(in crate::codegen::com) fn enum_definition(
        &self,
        id: super::ids::EnumId,
    ) -> Result<&ComEnumDefinition, ModelError> {
        self.model.enums.get(id)
    }

    pub(in crate::codegen::com) fn layout_definition(
        &self,
        id: super::ids::LayoutId,
    ) -> Result<&super::layout::NativeLayoutSet, ModelError> {
        self.model.layouts.get(id)
    }

    pub(in crate::codegen::com) fn find_enum(
        &self,
        namespace: &str,
        name: &str,
    ) -> Option<&ComEnumDefinition> {
        self.model.enums.iter().find(|definition| {
            definition.native_name().namespace() == namespace
                && definition.native_name().name() == name
        })
    }

    pub(in crate::codegen::com) fn referenced_enums(
        &self,
    ) -> impl Iterator<Item = &ComEnumDefinition> {
        self.referenced_enums
            .iter()
            .map(|id| self.model.enums.get(*id).expect("validated enum id"))
    }
}

pub(super) fn map_interface(meta: &ComInterfaceMeta) -> Result<SemanticComInterface, ModelError> {
    let raw_methods = meta.raw_methods.as_ref().ok_or_else(|| {
        ModelError::InvalidContract(format!(
            "{}.{} has no raw COM metadata facts",
            meta.interface.namespace, meta.interface.name
        ))
    })?;
    if meta.interface.methods.len() != raw_methods.len() {
        return Err(ModelError::InvalidContract(format!(
            "{}.{} has {} compatibility methods but {} raw methods",
            meta.interface.namespace,
            meta.interface.name,
            meta.interface.methods.len(),
            raw_methods.len()
        )));
    }
    let iid = ComGuid::parse(&meta.interface.iid)?;
    let mut model = ComModel::default();
    let methods = meta
        .interface
        .methods
        .iter()
        .zip(raw_methods)
        .map(|(method, raw)| {
            map_method(
                &mut model,
                &meta.interface.namespace,
                &meta.interface.name,
                &meta.interface.iid,
                iid,
                method,
                raw,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let referenced_enums = attach_enum_metadata(
        &mut model,
        meta.raw_referenced_enums.as_ref().ok_or_else(|| {
            ModelError::InvalidContract(format!(
                "{}.{} has no raw enum metadata facts",
                meta.interface.namespace, meta.interface.name
            ))
        })?,
    )?;
    let interface = SemanticComInterface {
        model,
        iid,
        is_iunknown_rooted: meta.is_iunknown_rooted,
        methods,
        referenced_enums,
    };
    if !interface.is_iunknown_rooted
        && interface
            .methods
            .iter()
            .any(|method| method.return_kind() == ComReturnKind::EnumeratorNextHResult)
    {
        return Err(ModelError::InvalidContract(
            "EnumeratorNext is restricted to IUnknown-rooted interfaces".into(),
        ));
    }
    interface.validate()?;
    Ok(interface)
}

fn raw_contains_named_type(raw: &RawComType, namespace: &str, name: &str) -> bool {
    if matches!(
        &raw.native_type,
        RawNativeType::Named {
            namespace: candidate_namespace,
            name: candidate_name,
            ..
        } if candidate_namespace == namespace && candidate_name == name
    ) {
        return true;
    }
    raw.underlying
        .as_deref()
        .is_some_and(|underlying| raw_contains_named_type(underlying, namespace, name))
        || matches!(
            &raw.native_type,
            RawNativeType::Array(element)
                if raw_contains_named_type(element, namespace, name)
        )
}

fn attach_enum_metadata(
    model: &mut ComModel,
    raw_enums: &[crate::com_metadata::ComEnumMeta],
) -> Result<Vec<EnumId>, ModelError> {
    let mut referenced = Vec::with_capacity(raw_enums.len());
    for raw_enum in raw_enums {
        let underlying = metadata_enum_scalar(&raw_enum.underlying).ok_or(
            ModelError::Unsupported(UnsupportedReason::UnknownNativeType),
        )?;
        let mut ids = model
            .enums
            .ids()
            .filter(|id| {
                model.enums.get(*id).is_ok_and(|definition| {
                    definition.native_name().namespace() == raw_enum.namespace
                        && definition.native_name().name() == raw_enum.name
                })
            })
            .collect::<Vec<_>>();
        if ids.is_empty() {
            ids.push(model.enums.insert(ComEnumDefinition::new(
                QualifiedName::new(&raw_enum.namespace, &raw_enum.name)?,
                underlying,
            )?)?);
        }
        let first = ids[0];
        let members = raw_enum
            .members
            .iter()
            .map(|member| {
                ComEnumMember::new(
                    member.name.clone(),
                    match member.value {
                        crate::com_metadata::ComEnumValue::Signed(value) => {
                            SemanticEnumValue::Signed(value)
                        }
                        crate::com_metadata::ComEnumValue::Unsigned(value) => {
                            SemanticEnumValue::Unsigned(value)
                        }
                    },
                )
            })
            .collect::<Vec<_>>();
        for id in ids {
            let definition = model.enums.get_mut(id)?;
            if definition.underlying() != underlying {
                return Err(ModelError::InvalidContract(format!(
                    "enum {}.{} underlying type disagrees with raw semantic type",
                    raw_enum.namespace, raw_enum.name
                )));
            }
            definition.set_members(members.clone(), raw_enum.is_flags);
        }
        if !referenced.contains(&first) {
            referenced.push(first);
        }
    }
    Ok(referenced)
}

fn metadata_enum_scalar(typ: &crate::types::TypeMeta) -> Option<ScalarType> {
    match typ {
        crate::types::TypeMeta::I8 => Some(ScalarType::I8),
        crate::types::TypeMeta::U8 => Some(ScalarType::U8),
        crate::types::TypeMeta::I16 => Some(ScalarType::I16),
        crate::types::TypeMeta::U16 => Some(ScalarType::U16),
        crate::types::TypeMeta::I32 => Some(ScalarType::I32),
        crate::types::TypeMeta::U32 => Some(ScalarType::U32),
        crate::types::TypeMeta::I64 => Some(ScalarType::I64),
        crate::types::TypeMeta::U64 => Some(ScalarType::U64),
        _ => None,
    }
}

fn map_method(
    model: &mut ComModel,
    interface_namespace: &str,
    interface_name: &str,
    interface_iid_text: &str,
    interface_iid: ComGuid,
    method: &crate::com_metadata::MethodMeta,
    raw: &RawComMethod,
) -> Result<ComMethodContract, ModelError> {
    crate::com_metadata::validate_borrowed_hwnd_output_evidence(raw)
        .map_err(ModelError::InvalidContract)?;
    crate::com_metadata::validate_attached_enumerator_evidence(raw)
        .map_err(ModelError::InvalidContract)?;
    crate::com_metadata::validate_attached_safe_array_evidence(raw)
        .map_err(ModelError::InvalidContract)?;
    if let Some(contract) = &raw.exact_contract {
        crate::com_metadata::validate_exact_method_contract(
            interface_namespace,
            interface_name,
            interface_iid_text,
            raw,
            contract,
        )
        .map_err(ModelError::InvalidContract)?;
        if contract.kind == RawExactMethodContractKind::UnsafePrivateData {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
                "{}; see {}",
                contract.reason, contract.citation
            ))));
        }
    }
    if method.name != raw.projected_name || method.params.len() != raw.params.len() {
        return Err(ModelError::InvalidContract(format!(
            "{} compatibility and raw metadata are not aligned",
            method.name
        )));
    }
    if raw.enumerator_next.is_some() && raw.semantic_hresult.is_none() {
        return Err(ModelError::InvalidContract(format!(
            "{} EnumeratorNext requires exact multiple-success HRESULT evidence",
            method.name
        )));
    }
    let dynamic_iid = dynamic_iid_contract(raw)?;
    let params = method
        .params
        .iter()
        .zip(&raw.params)
        .enumerate()
        .map(|(index, (compatibility, raw_param))| {
            if compatibility.name != raw_param.name {
                return Err(ModelError::InvalidContract(format!(
                    "parameter `{}` is not aligned with raw parameter `{}`",
                    compatibility.name, raw_param.name
                )));
            }
            map_param(
                model,
                &raw.declaring_namespace,
                &raw.declaring_interface,
                index,
                raw_param,
                raw,
                dynamic_iid.is_some_and(|contract| contract.output_param_index().index() == index),
            )
            .map_err(|error| {
                error.context(format!(
                    "{}.{} ({:?})",
                    raw.projected_name, raw_param.name, raw_param.typ
                ))
            })
        })
        .collect::<Result<Vec<_>, ModelError>>()?;
    let return_kind = map_return(model, raw)
        .map_err(|error| error.context(format!("{} return", raw.projected_name)))?;
    if raw.semantic_hresult.is_some()
        && !matches!(
            return_kind,
            ComReturnKind::SemanticHResult | ComReturnKind::EnumeratorNextHResult
        )
    {
        return Err(ModelError::InvalidContract(format!(
            "{} semantic-HRESULT evidence applies to a non-HRESULT return",
            method.name
        )));
    }
    let mut method = ComMethodContract::new(
        raw.projected_name.clone(),
        interface_iid,
        raw.vtable_index,
        CallingConvention::System,
        params,
        return_kind,
    )?;
    if let Some(contract) = dynamic_iid {
        method = method.with_dynamic_iid_contract(contract)?;
    }
    Ok(
        match raw.exact_contract.as_ref().map(|contract| contract.kind) {
            Some(RawExactMethodContractKind::FixedCapacityBytes) => {
                method.with_special_contract(ComMethodSpecialContract::FixedCapacityBytes {
                    guid_param: ParamIndex::new(0),
                })
            }
            Some(RawExactMethodContractKind::UnsafePrivateData) => {
                unreachable!("unsafe private-data contracts fail before method mapping")
            }
            Some(RawExactMethodContractKind::StatStg) => method,
            Some(RawExactMethodContractKind::Malloc) => {
                method.with_special_contract(ComMethodSpecialContract::Malloc)
            }
            None => method,
        },
    )
}

fn map_param(
    model: &mut ComModel,
    interface_namespace: &str,
    interface_name: &str,
    param_index: usize,
    raw: &RawComParam,
    raw_method: &RawComMethod,
    dynamic_iid_output: bool,
) -> Result<ComParamContract, ModelError> {
    let effective_direction = documented_bstr_direction_override(
        interface_namespace,
        interface_name,
        &raw_method.metadata_name,
        &raw.name,
    )
    .or_else(|| {
        raw_method.enumerator_next.as_ref().and_then(|enumerator| {
            (enumerator.values_param_index == param_index
                || enumerator.fetched_param_index == param_index)
                .then_some(RawParamDirection::Out)
        })
    })
    .or_else(|| {
        raw.native_array
            .as_ref()
            .is_some_and(|array| array.two_call && raw.direction == RawParamDirection::InOut)
            .then_some(RawParamDirection::Out)
    })
    .unwrap_or(raw.direction);
    let (abi_type, count) = if raw_method.exact_contract.as_ref().is_some_and(|contract| {
        contract.kind == RawExactMethodContractKind::StatStg
            && contract.buffer_param_index == param_index
    }) {
        (
            insert_abi(
                model,
                Some(QualifiedName::new("Windows.Win32.System.Com", "STATSTG")?),
                None,
                ComAbiType::StatStg,
            )?,
            None,
        )
    } else if let Some(array) = &raw.native_array {
        if effective_direction == RawParamDirection::InOut {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(
                "in/out counted-buffer contents require a dedicated preserve-input storage plan"
                    .into(),
            )));
        }
        let count_param = array.count_param_index.ok_or(ModelError::Unsupported(
            UnsupportedReason::MissingCountRelationship,
        ))?;
        let callee_allocated = raw_method
            .params
            .get(count_param)
            .is_some_and(|count| count.direction == RawParamDirection::Out)
            && effective_direction == RawParamDirection::Out;
        let (element, element_ownership) = map_buffer_element(
            model,
            raw,
            callee_allocated,
            array.unit == RawCountUnit::Bytes,
        )?;
        let buffer = model.types_mut().insert(ComTypeDefinition::new(
            raw_native_name(&raw.typ)?,
            None,
            ComAbiType::CountedBuffer {
                element,
                element_ownership,
                pointer_depth: nonzero_pointer_depth(raw.typ.pointer_depth.max(1))?,
                constness: array
                    .constness
                    .map(|constness| map_constness(constness, false))
                    .unwrap_or_else(|| buffer_constness(raw)),
            },
        ))?;
        let count_param = ParamIndex::new(count_param);
        let count_direction = raw_method
            .params
            .get(count_param.index())
            .ok_or_else(|| {
                ModelError::InvalidContract(format!(
                    "NativeArrayInfo count index {} is outside the method",
                    count_param.index()
                ))
            })?
            .direction;
        let actual_length_param = array.actual_length_param_index.map(ParamIndex::new);
        let unit = match array.unit {
            RawCountUnit::Elements => CountUnit::Elements,
            RawCountUnit::Bytes => CountUnit::Bytes,
        };
        let mut relation = map_count_relation(
            effective_direction,
            count_direction,
            count_param,
            actual_length_param,
            unit,
            array.two_call,
            array.projected_capacity,
        );
        if let Some(enumerator) = &raw_method.enumerator_next
            && param_index == enumerator.values_param_index
        {
            if count_param.index() != enumerator.capacity_param_index
                || effective_direction != RawParamDirection::Out
                || actual_length_param.is_some()
                || unit != CountUnit::Elements
                || array.two_call
            {
                return Err(ModelError::InvalidContract(
                    "EnumeratorNext metadata count relation does not match the exact override"
                        .into(),
                ));
            }
            relation = CountRelation::EnumeratorNext {
                capacity_param: ParamIndex::new(enumerator.capacity_param_index),
                fetched_param: ParamIndex::new(enumerator.fetched_param_index),
                fetched_optional_for_single: enumerator.fetched_optional_for_single,
            };
        }
        (buffer, Some(relation))
    } else {
        (map_parameter_type(model, raw)?, None)
    };
    let direction = map_direction(effective_direction);
    if direction == Direction::In && terminal_pointer_pointee_is_bstr(model, abi_type)? {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(
            "input BSTR pointer chains have no proven pointee allocation or borrow contract".into(),
        )));
    }
    if raw.optional
        && direction == Direction::In
        && matches!(model.types().get(abi_type)?.abi(), ComAbiType::Variant)
    {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(
            "optional by-value VARIANT input has no proven native null/default semantics".into(),
        )));
    }

    fn terminal_pointer_pointee_is_bstr(
        model: &ComModel,
        abi_type: TypeId,
    ) -> Result<bool, ModelError> {
        let ComAbiType::Pointer { pointee, .. } = model.types().get(abi_type)?.abi() else {
            return Ok(false);
        };
        Ok(match model.types().get(*pointee)?.abi() {
            ComAbiType::Bstr => true,
            ComAbiType::Pointer { .. } => terminal_pointer_pointee_is_bstr(model, *pointee)?,
            _ => false,
        })
    }
    let (ownership, cleanup) = map_ownership(
        model,
        abi_type,
        direction,
        raw,
        raw_method,
        param_index,
        dynamic_iid_output,
    )?;
    let nullable = if (raw.optional
        || known_nullable_param_override(
            interface_namespace,
            interface_name,
            &raw_method.metadata_name,
            &raw.name,
        )
        || raw
            .safe_array_evidence
            .as_ref()
            .is_some_and(crate::com_safe_array_registry::safe_array_output_allows_null))
        && is_nullable_type(model, abi_type)?
    {
        Nullability::Nullable
    } else {
        Nullability::Required
    };
    ComParamContract::new(
        raw.name.clone(),
        abi_type,
        direction,
        raw.optional,
        nullable,
        count,
        ownership,
        cleanup,
    )
}

fn documented_bstr_direction_override(
    namespace: &str,
    interface: &str,
    method: &str,
    parameter: &str,
) -> Option<RawParamDirection> {
    // The Win32 API declarations are authoritative where win32metadata's
    // OptionalAttribute-derived direction disagrees:
    // https://learn.microsoft.com/windows/win32/api/photoacquire/nf-photoacquire-iphotoacquiredeviceselectiondialog-domodal
    // https://learn.microsoft.com/windows/win32/api/imapi/nf-imapi-idiscrecorder-getdisplaynames
    let documented_out = (namespace == "Windows.Win32.Media.PictureAcquisition"
        && interface == "IPhotoAcquireDeviceSelectionDialog"
        && method == "DoModal"
        && parameter == "pbstrDeviceId")
        || (namespace == "Windows.Win32.Storage.Imapi"
            && interface == "IDiscRecorder"
            && method == "GetDisplayNames"
            && matches!(
                parameter,
                "pbstrVendorID" | "pbstrProductID" | "pbstrRevision"
            ));
    documented_out.then_some(RawParamDirection::Out)
}

fn known_nullable_param_override(
    namespace: &str,
    interface: &str,
    method: &str,
    parameter: &str,
) -> bool {
    namespace == "Windows.Win32.UI.Shell"
        && ((matches!(interface, "ITaskbarList3" | "ITaskbarList4")
            && method == "SetThumbnailClip"
            && parameter == "prcClip")
            || (interface == "IShellLinkW" && method == "GetPath" && parameter == "pfd"))
}

fn map_parameter_type(model: &mut ComModel, raw: &RawComParam) -> Result<TypeId, ModelError> {
    if is_raw_named_type(&raw.typ, "Windows.Win32.System.Com", "SAFEARRAY") {
        let element = known_safe_array_element(model, raw)?;
        let base = insert_abi(
            model,
            Some(QualifiedName::new("Windows.Win32.System.Com", "SAFEARRAY")?),
            None,
            ComAbiType::SafeArray {
                element: Some(element),
            },
        )?;
        if raw.typ.pointer_depth == 0 {
            return Ok(base);
        }
        return insert_abi(
            model,
            None,
            None,
            ComAbiType::Pointer {
                pointee: base,
                depth: nonzero_pointer_depth(raw.typ.pointer_depth)?,
                constness: map_constness(raw.typ.constness, raw.const_attribute),
            },
        );
    }
    map_type(model, &raw.typ, raw.const_attribute)
}

fn known_safe_array_element(model: &mut ComModel, raw: &RawComParam) -> Result<TypeId, ModelError> {
    let evidence = raw
        .safe_array_evidence
        .as_ref()
        .ok_or(ModelError::Unsupported(
            UnsupportedReason::UnsupportedSafeArrayElement,
        ))?;
    match (evidence.ownership, raw.direction, raw.typ.pointer_depth) {
        (RawSafeArrayOwnership::BorrowedInput, RawParamDirection::In, 1)
        | (RawSafeArrayOwnership::OwnedOutput, RawParamDirection::Out, 2) => {}
        _ => {
            return Err(ModelError::InvalidContract(
                "SAFEARRAY ownership evidence does not match parameter shape".into(),
            ));
        }
    }
    match evidence.element_vartype {
        RawSafeArrayVartype::I4 => {
            insert_abi(model, None, None, ComAbiType::Scalar(ScalarType::I32))
        }
        RawSafeArrayVartype::Ui1 => {
            insert_abi(model, None, None, ComAbiType::Scalar(ScalarType::U8))
        }
        RawSafeArrayVartype::Ui4 => {
            insert_abi(model, None, None, ComAbiType::Scalar(ScalarType::U32))
        }
        RawSafeArrayVartype::R8 => {
            insert_abi(model, None, None, ComAbiType::Scalar(ScalarType::F64))
        }
        RawSafeArrayVartype::Bstr => insert_abi(
            model,
            Some(QualifiedName::new("Windows.Win32.Foundation", "BSTR")?),
            None,
            ComAbiType::Bstr,
        ),
        RawSafeArrayVartype::Variant => insert_abi(
            model,
            Some(QualifiedName::new(
                "Windows.Win32.System.Variant",
                "VARIANT",
            )?),
            None,
            ComAbiType::Variant,
        ),
        RawSafeArrayVartype::Unknown => {
            let iid = evidence.element_iid.ok_or_else(|| {
                ModelError::InvalidContract(
                    "VT_UNKNOWN SAFEARRAY evidence requires an exact interface IID".into(),
                )
            })?;
            insert_abi(
                model,
                None,
                None,
                ComAbiType::ComInterface {
                    iid: ComGuid::parse(iid)?,
                },
            )
        }
    }
}

fn is_raw_named_type(raw: &RawComType, namespace: &str, name: &str) -> bool {
    matches!(
        &raw.native_type,
        RawNativeType::Named {
            namespace: candidate_namespace,
            name: candidate_name,
            ..
        } if candidate_namespace == namespace && candidate_name == name
    )
}

fn map_return(model: &mut ComModel, raw: &RawComMethod) -> Result<ComReturnKind, ModelError> {
    if matches!(raw.return_type.native_type, RawNativeType::Void)
        && raw.return_type.pointer_depth == 0
    {
        return Ok(ComReturnKind::Void);
    }
    let abi_type = map_type(model, &raw.return_type, false)?;
    if is_hresult(model, abi_type)? {
        return Ok(if raw.enumerator_next.is_some() {
            ComReturnKind::EnumeratorNextHResult
        } else if raw.semantic_hresult.is_some() {
            ComReturnKind::SemanticHResult
        } else {
            ComReturnKind::HResult
        });
    }
    if matches!(model.types().get(abi_type)?.abi(), ComAbiType::Handle(_)) {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(
            "direct handle returns require explicit resource ownership and cleanup".into(),
        )));
    }
    if model
        .types()
        .get(abi_type)?
        .abi()
        .requires_pointer_return_convention()
    {
        Ok(ComReturnKind::DirectPointer(abi_type))
    } else {
        Ok(ComReturnKind::DirectValue(abi_type))
    }
}

fn map_type(
    model: &mut ComModel,
    raw: &RawComType,
    const_attribute: bool,
) -> Result<TypeId, ModelError> {
    if raw.pointer_depth > 0
        && matches!(raw.native_type, RawNativeType::Object | RawNativeType::Void)
    {
        let depth = nonzero_pointer_depth(raw.pointer_depth)?;
        return model.types_mut().insert(ComTypeDefinition::new(
            None,
            None,
            ComAbiType::DataPointer {
                pointee: DataPointee::Opaque(None),
                depth,
                constness: map_constness(raw.constness, const_attribute),
            },
        ));
    }
    if raw.pointer_depth > 0 && is_opaque_named_pointee(raw) {
        let depth = nonzero_pointer_depth(raw.pointer_depth)?;
        return model.types_mut().insert(ComTypeDefinition::new(
            raw_native_name(raw)?,
            None,
            ComAbiType::DataPointer {
                pointee: DataPointee::Opaque(raw_native_name(raw)?),
                depth,
                constness: map_constness(raw.constness, const_attribute),
            },
        ));
    }
    let base = map_raw_base_type(model, raw)?;
    if raw.pointer_depth == 0 {
        return Ok(base);
    }
    let depth = nonzero_pointer_depth(raw.pointer_depth)?;
    model.types_mut().insert(ComTypeDefinition::new(
        None,
        None,
        ComAbiType::Pointer {
            pointee: base,
            depth,
            constness: map_constness(raw.constness, const_attribute),
        },
    ))
}

fn is_opaque_named_pointee(raw: &RawComType) -> bool {
    let RawNativeType::Named {
        namespace,
        name,
        kind: RawNamedKind::Struct,
        ..
    } = &raw.native_type
    else {
        return false;
    };
    if (namespace == "System" && matches!(name.as_str(), "Guid" | "IntPtr" | "UIntPtr"))
        || (namespace == "Windows.Win32.Foundation"
            && matches!(name.as_str(), "BOOL" | "HRESULT" | "BSTR"))
        || (namespace == "Windows.Win32.System.Variant"
            && matches!(name.as_str(), "VARIANT" | "VARIANTARG"))
        || (namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY")
        || (namespace == "Windows.Win32.System.Com.StructuredStorage" && name == "PROPVARIANT")
        || (namespace == "Windows.Win32.System.WinRT" && name == "HSTRING")
    {
        return false;
    }
    // ITEMIDLIST ends in SHITEMID's variable-length abID payload, so only its
    // documented pointer/allocator contract is usable; it has no fixed layout.
    if namespace == "Windows.Win32.UI.Shell.Common" && name == "ITEMIDLIST" {
        return true;
    }
    if matches!(
        raw.native_type,
        RawNativeType::Named {
            layout: Some(_),
            ..
        }
    ) {
        return false;
    }
    match raw.underlying.as_deref() {
        Some(underlying)
            if raw_scalar_alias(underlying, namespace, name).is_some()
                || underlying.pointer_depth > 0 =>
        {
            false
        }
        _ => true,
    }
}

pub(in crate::codegen::com) fn census_raw_base_category(raw: &RawComType) -> &'static str {
    if matches!(
        raw.native_type,
        RawNativeType::Named {
            kind: RawNamedKind::Delegate,
            ..
        }
    ) {
        return "FunctionPointer";
    }
    let mut model = ComModel::default();
    if let Ok(id) = map_raw_base_type(&mut model, raw)
        && let Ok(definition) = model.types().get(id)
    {
        return match definition.abi() {
            ComAbiType::Scalar(_) => "Scalar",
            ComAbiType::Guid => "Guid",
            ComAbiType::Enum(_) => "Enum",
            ComAbiType::NativeStruct(_) => "NativeStruct",
            ComAbiType::NativeUnion(_) => "NativeUnion",
            ComAbiType::Pointer { .. } => "Pointer",
            ComAbiType::Handle(_) => "Handle",
            ComAbiType::DataPointer { .. } => "DataPointer",
            ComAbiType::StringPointer { .. } => "StringPointer",
            ComAbiType::Bstr => "Bstr",
            ComAbiType::HString => "HString",
            ComAbiType::ComInterface { .. } => "ComInterface",
            ComAbiType::CountedBuffer { .. } => "CountedBuffer",
            ComAbiType::SafeArray { .. } => "SafeArray",
            ComAbiType::Variant => "Variant",
            ComAbiType::PropVariant => "PropVariant",
            ComAbiType::DispatchParams => "DispatchParams",
            ComAbiType::ExcepInfo => "ExcepInfo",
            ComAbiType::StatStg => "StatStg",
            ComAbiType::FunctionPointer(_) => "FunctionPointer",
            ComAbiType::Unknown(_) => "Unknown",
        };
    }
    if let RawNativeType::Named {
        layout: Some(layout),
        ..
    } = &raw.native_type
    {
        if layout
            .variants
            .first()
            .is_some_and(|variant| variant.is_union)
        {
            "NativeUnion"
        } else {
            "NativeStruct"
        }
    } else {
        "Unknown"
    }
}

fn map_raw_base_type(model: &mut ComModel, raw: &RawComType) -> Result<TypeId, ModelError> {
    let scalar = match raw.native_type {
        RawNativeType::Bool => Some(ScalarType::Bool),
        RawNativeType::I8 => Some(ScalarType::I8),
        RawNativeType::U8 => Some(ScalarType::U8),
        RawNativeType::I16 => Some(ScalarType::I16),
        RawNativeType::U16 => Some(ScalarType::U16),
        RawNativeType::I32 => Some(ScalarType::I32),
        RawNativeType::U32 => Some(ScalarType::U32),
        RawNativeType::I64 => Some(ScalarType::I64),
        RawNativeType::U64 => Some(ScalarType::U64),
        RawNativeType::F32 => Some(ScalarType::F32),
        RawNativeType::F64 => Some(ScalarType::F64),
        RawNativeType::Char16 => Some(ScalarType::Char16),
        RawNativeType::ISize => Some(ScalarType::NativeIsize),
        RawNativeType::USize => Some(ScalarType::NativeUsize),
        _ => None,
    };
    if let Some(scalar) = scalar {
        return insert_abi(
            model,
            raw_native_name(raw)?,
            None,
            ComAbiType::Scalar(scalar),
        );
    }

    match &raw.native_type {
        RawNativeType::Named {
            namespace, name, ..
        } if namespace == "System" && name == "Guid" => {
            insert_abi(model, raw_native_name(raw)?, None, ComAbiType::Guid)
        }
        RawNativeType::Named {
            namespace, name, ..
        } if namespace == "Windows.Win32.System.WinRT" && name == "HSTRING" => {
            insert_abi(model, raw_native_name(raw)?, None, ComAbiType::HString)
        }
        RawNativeType::Named {
            namespace,
            name,
            kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
            iid,
            ..
        } => insert_abi(
            model,
            Some(QualifiedName::new(namespace, name)?),
            None,
            ComAbiType::ComInterface {
                iid: ComGuid::parse(iid.as_deref().ok_or(ModelError::Unsupported(
                    UnsupportedReason::MissingInterfaceIid,
                ))?)?,
            },
        ),
        RawNativeType::Named {
            namespace,
            name,
            kind: RawNamedKind::Enum,
            ..
        } => {
            let underlying = raw.underlying.as_deref().ok_or(ModelError::Unsupported(
                UnsupportedReason::UnknownNativeType,
            ))?;
            let scalar = raw_scalar(underlying).ok_or(ModelError::Unsupported(
                UnsupportedReason::UnknownNativeType,
            ))?;
            let enum_id = model.enums_mut().insert(ComEnumDefinition::new(
                QualifiedName::new(namespace, name)?,
                scalar,
            )?)?;
            insert_abi(
                model,
                Some(QualifiedName::new(namespace, name)?),
                None,
                ComAbiType::Enum(enum_id),
            )
        }
        RawNativeType::Named {
            kind: RawNamedKind::Delegate,
            ..
        } => Err(ModelError::Unsupported(
            UnsupportedReason::UnsupportedFunctionPointer,
        )),
        RawNativeType::Named {
            kind: RawNamedKind::Unknown,
            ..
        } => Err(ModelError::Unsupported(
            UnsupportedReason::UnknownNativeType,
        )),
        RawNativeType::Named {
            namespace, name, ..
        } => map_raw_named_struct(model, namespace, name, raw),
        RawNativeType::String => insert_abi(model, None, None, ComAbiType::HString),
        RawNativeType::Object => Err(ModelError::Unsupported(
            UnsupportedReason::UnknownPointerMeaning,
        )),
        RawNativeType::Array(_) | RawNativeType::FixedArray { .. } => {
            Err(ModelError::Unsupported(UnsupportedReason::UnsupportedArray))
        }
        RawNativeType::Unknown(_) | RawNativeType::Void => Err(ModelError::Unsupported(
            UnsupportedReason::UnknownNativeType,
        )),
        _ => unreachable!("scalar raw types returned above"),
    }
}

fn map_raw_named_struct(
    model: &mut ComModel,
    namespace: &str,
    name: &str,
    raw: &RawComType,
) -> Result<TypeId, ModelError> {
    if namespace == "System" && name == "IntPtr" {
        return insert_abi(
            model,
            raw_native_name(raw)?,
            None,
            ComAbiType::Scalar(ScalarType::NativeIsize),
        );
    }
    if namespace == "System" && name == "UIntPtr" {
        return insert_abi(
            model,
            raw_native_name(raw)?,
            None,
            ComAbiType::Scalar(ScalarType::NativeUsize),
        );
    }
    if namespace == "Windows.Win32.Foundation" {
        match name {
            "BOOL" => {
                return insert_abi(
                    model,
                    raw_native_name(raw)?,
                    None,
                    ComAbiType::Scalar(ScalarType::Win32Bool),
                );
            }
            "HRESULT" => {
                return insert_abi(
                    model,
                    raw_native_name(raw)?,
                    None,
                    ComAbiType::Scalar(ScalarType::HResult),
                );
            }
            "BSTR" => return insert_abi(model, raw_native_name(raw)?, None, ComAbiType::Bstr),
            _ => {}
        }
    }
    if namespace == "Windows.Win32.System.WinRT" && name == "HSTRING" {
        return insert_abi(model, raw_native_name(raw)?, None, ComAbiType::HString);
    }
    if namespace == "Windows.Win32.System.Variant" && matches!(name, "VARIANT" | "VARIANTARG") {
        return insert_abi(model, raw_native_name(raw)?, None, ComAbiType::Variant);
    }
    if namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY" {
        return insert_abi(
            model,
            raw_native_name(raw)?,
            None,
            ComAbiType::SafeArray { element: None },
        );
    }
    if namespace == "Windows.Win32.System.Com" && name == "DISPPARAMS" {
        return insert_abi(
            model,
            raw_native_name(raw)?,
            None,
            ComAbiType::DispatchParams,
        );
    }
    if namespace == "Windows.Win32.System.Com" && name == "EXCEPINFO" {
        return insert_abi(model, raw_native_name(raw)?, None, ComAbiType::ExcepInfo);
    }
    if namespace == "Windows.Win32.System.Com.StructuredStorage" && name == "PROPVARIANT" {
        return insert_abi(model, raw_native_name(raw)?, None, ComAbiType::PropVariant);
    }
    if is_explicit_pointer_alias(namespace, name) {
        return map_pointer_alias(model, namespace, name);
    }
    if let Some(underlying) = raw.underlying.as_deref() {
        if let Some(scalar) = raw_scalar_alias(underlying, namespace, name) {
            let underlying_id = insert_abi(model, None, None, ComAbiType::Scalar(scalar))?;
            return insert_abi(
                model,
                Some(QualifiedName::new(namespace, name)?),
                Some(underlying_id),
                ComAbiType::Scalar(scalar),
            );
        }
        if underlying.pointer_depth > 0 {
            return map_pointer_alias(model, namespace, name);
        }
    }
    if let RawNativeType::Named {
        layout: Some(layout),
        ..
    } = &raw.native_type
    {
        return map_native_struct(model, namespace, name, layout);
    }
    Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout))
}

fn map_native_struct(
    model: &mut ComModel,
    namespace: &str,
    name: &str,
    raw: &RawNativeLayoutSet,
) -> Result<TypeId, ModelError> {
    if raw.recursive {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
            "recursive by-value native layout {namespace}.{name}"
        ))));
    }
    let layout_id = model.layouts_mut().reserve()?;
    let is_union = raw.variants.first().is_some_and(|variant| variant.is_union);
    if raw
        .variants
        .iter()
        .any(|variant| variant.is_union != is_union)
    {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
            "architecture variants disagree whether {namespace}.{name} is a union"
        ))));
    }
    let type_id = insert_abi(
        model,
        Some(QualifiedName::new(namespace, name)?),
        None,
        if is_union {
            ComAbiType::NativeUnion(layout_id)
        } else {
            ComAbiType::NativeStruct(layout_id)
        },
    )?;
    let x86 = compute_native_layout(model, raw, Architecture::X86)
        .map_err(|error| error.context(format!("{namespace}.{name} (x86)")))?;
    let x64 = compute_native_layout(model, raw, Architecture::X64)
        .map_err(|error| error.context(format!("{namespace}.{name} (x64)")))?;
    let arm64 = compute_native_layout(model, raw, Architecture::Arm64)
        .map_err(|error| error.context(format!("{namespace}.{name} (ARM64)")))?;
    model
        .layouts_mut()
        .define(layout_id, NativeLayoutSet::new(x86, x64, arm64))?;
    Ok(type_id)
}

fn compute_native_layout(
    model: &mut ComModel,
    raw: &RawNativeLayoutSet,
    architecture: Architecture,
) -> Result<NativeLayout, ModelError> {
    let candidates = raw
        .variants
        .iter()
        .filter(|variant| variant.architectures & architecture.metadata_mask() != 0)
        .collect::<Vec<_>>();
    let [raw] = candidates.as_slice() else {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(
            if candidates.is_empty() {
                format!("missing {:?} native layout facts", architecture)
            } else {
                format!("ambiguous {:?} native layout facts", architecture)
            },
        )));
    };
    compute_native_layout_variant(model, raw, architecture)
}

fn compute_native_layout_variant(
    model: &mut ComModel,
    raw: &RawNativeLayout,
    architecture: Architecture,
) -> Result<NativeLayout, ModelError> {
    let kind = if raw.is_union {
        LayoutKind::Union
    } else {
        match raw.kind {
            RawLayoutKind::Sequential => LayoutKind::Struct(StructLayout::Sequential),
            RawLayoutKind::Explicit => LayoutKind::Struct(StructLayout::Explicit),
            RawLayoutKind::Unknown => {
                return Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout));
            }
        }
    };
    let packing = match raw.packing {
        RawPacking::Default => 8usize,
        RawPacking::Explicit(value) if value.is_power_of_two() => usize::from(value),
        RawPacking::Explicit(_) | RawPacking::Unknown => {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(
                "unknown or invalid native packing".into(),
            )));
        }
    };
    if raw.fields.is_empty() {
        return Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout));
    }

    let mut fields = Vec::with_capacity(raw.fields.len());
    let mut intervals = Vec::with_capacity(raw.fields.len());
    let mut cursor = 0usize;
    let mut structure_alignment = 1usize;
    for raw_field in &raw.fields {
        if raw_field.bitfield {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
                "bitfield `{}` is not supported",
                raw_field.name
            ))));
        }
        if raw_field.flexible_array {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
                "flexible array `{}` is not supported",
                raw_field.name
            ))));
        }
        let field_type = map_type(model, &raw_field.typ, false)?;
        validate_pod_field_type(
            model,
            field_type,
            raw_field.fixed_count.is_some(),
            raw.is_union,
        )?;
        let (element_size, element_alignment) =
            abi_size_alignment(model, field_type, architecture, &mut HashSet::new())?;
        let count = match raw_field.fixed_count {
            Some(count) => Some(
                NonZeroU32::new(u32::try_from(count).map_err(|_| {
                    ModelError::InvalidLayout(format!(
                        "fixed array `{}` length exceeds u32",
                        raw_field.name
                    ))
                })?)
                .ok_or_else(|| {
                    ModelError::InvalidLayout(format!(
                        "fixed array `{}` has zero length",
                        raw_field.name
                    ))
                })?,
            ),
            None => None,
        };
        let field_size = element_size
            .checked_mul(count.map_or(1, |count| count.get() as usize))
            .ok_or_else(|| {
                ModelError::InvalidLayout(format!("field `{}` size overflows", raw_field.name))
            })?;
        let effective_alignment = element_alignment.min(packing);
        structure_alignment = structure_alignment.max(effective_alignment);
        let offset = match kind {
            LayoutKind::Struct(StructLayout::Sequential) => {
                if raw_field.explicit_offset.is_some() {
                    return Err(ModelError::InvalidLayout(format!(
                        "sequential field `{}` unexpectedly has an explicit offset",
                        raw_field.name
                    )));
                }
                align_up(cursor, effective_alignment)?
            }
            LayoutKind::Struct(StructLayout::Explicit) => {
                let offset = raw_field.explicit_offset.ok_or_else(|| {
                    ModelError::Unsupported(UnsupportedReason::Other(format!(
                        "explicit field `{}` has no authoritative offset",
                        raw_field.name
                    )))
                })?;
                if offset % effective_alignment != 0 {
                    return Err(ModelError::InvalidLayout(format!(
                        "explicit field `{}` offset {offset} violates alignment {effective_alignment}",
                        raw_field.name
                    )));
                }
                offset
            }
            LayoutKind::Union => {
                if raw_field.explicit_offset.is_some_and(|offset| offset != 0) {
                    return Err(ModelError::InvalidLayout(format!(
                        "union field `{}` must start at offset zero",
                        raw_field.name
                    )));
                }
                0
            }
        };
        let end = offset.checked_add(field_size).ok_or_else(|| {
            ModelError::InvalidLayout(format!("field `{}` end overflows", raw_field.name))
        })?;
        if kind != LayoutKind::Union
            && intervals.iter().any(|(existing_start, existing_end)| {
                offset < *existing_end && *existing_start < end
            })
        {
            return Err(ModelError::Unsupported(UnsupportedReason::Other(format!(
                "overlapping explicit field `{}` requires union support",
                raw_field.name
            ))));
        }
        intervals.push((offset, end));
        cursor = cursor.max(end);
        fields.push(FieldLayout::new(
            &raw_field.name,
            field_type,
            offset,
            count,
        )?);
    }
    let natural_size = align_up(cursor, structure_alignment)?;
    let size = match raw.declared_size {
        Some(size) if size < cursor || size % structure_alignment != 0 => {
            return Err(ModelError::InvalidLayout(format!(
                "declared size {size} cannot contain {cursor} bytes at alignment {structure_alignment}"
            )));
        }
        Some(size) => size,
        None => natural_size,
    };
    NativeLayout::new(size, structure_alignment, packing, kind, fields)
}

fn validate_pod_field_type(
    model: &ComModel,
    field_type: TypeId,
    fixed_array: bool,
    enclosing_union: bool,
) -> Result<(), ModelError> {
    let abi = model.types().get(field_type)?.abi();
    if fixed_array && !matches!(abi, ComAbiType::Scalar(_) | ComAbiType::Enum(_)) {
        return Err(ModelError::Unsupported(UnsupportedReason::Other(
            "fixed POD arrays currently require primitive or enum elements".into(),
        )));
    }
    match abi {
        ComAbiType::Scalar(_)
        | ComAbiType::Guid
        | ComAbiType::Enum(_)
        | ComAbiType::DataPointer { .. }
        | ComAbiType::Handle(_)
        | ComAbiType::StringPointer { .. }
        | ComAbiType::NativeStruct(_) => Ok(()),
        ComAbiType::NativeUnion(_) if enclosing_union => {
            Err(ModelError::Unsupported(UnsupportedReason::Other(
                "nested native unions require an explicit nested active-field contract".into(),
            )))
        }
        ComAbiType::Pointer { pointee, .. } => match model.types().get(*pointee)?.abi() {
            ComAbiType::ComInterface { .. }
            | ComAbiType::Bstr
            | ComAbiType::HString
            | ComAbiType::Handle(_) => Err(ModelError::Unsupported(UnsupportedReason::Other(
                "nested interface, string, or resource pointer ownership is not supported".into(),
            ))),
            _ => Ok(()),
        },
        ComAbiType::Bstr | ComAbiType::HString | ComAbiType::ComInterface { .. } => {
            Err(ModelError::Unsupported(UnsupportedReason::Other(
                "nested string/interface ownership is not supported".into(),
            )))
        }
        ComAbiType::NativeUnion(_) => Err(ModelError::Unsupported(UnsupportedReason::Other(
            "native unions may not be nested in structs without a discriminant contract".into(),
        ))),
        ComAbiType::CountedBuffer { .. }
        | ComAbiType::SafeArray { .. }
        | ComAbiType::Variant
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
        | ComAbiType::FunctionPointer(_)
        | ComAbiType::Unknown(_) => Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout)),
    }
}

fn abi_size_alignment(
    model: &ComModel,
    type_id: TypeId,
    architecture: Architecture,
    visiting: &mut HashSet<TypeId>,
) -> Result<(usize, usize), ModelError> {
    if !visiting.insert(type_id) {
        return Err(ModelError::InvalidLayout(
            "recursive by-value native layout".into(),
        ));
    }
    let result = match model.types().get(type_id)?.abi() {
        ComAbiType::Scalar(scalar) => match scalar {
            ScalarType::Bool | ScalarType::I8 | ScalarType::U8 => (1, 1),
            ScalarType::I16 | ScalarType::U16 | ScalarType::Char16 => (2, 2),
            ScalarType::I32
            | ScalarType::U32
            | ScalarType::F32
            | ScalarType::Win32Bool
            | ScalarType::HResult => (4, 4),
            ScalarType::I64 | ScalarType::U64 | ScalarType::F64 => (8, 8),
            ScalarType::NativeIsize | ScalarType::NativeUsize => (
                architecture.pointer_size(),
                architecture.pointer_alignment(),
            ),
        },
        ComAbiType::Guid => (16, 4),
        ComAbiType::Enum(enum_id) => {
            let scalar = model.enums.get(*enum_id)?.underlying();
            match scalar {
                ScalarType::I8 | ScalarType::U8 => (1, 1),
                ScalarType::I16 | ScalarType::U16 => (2, 2),
                ScalarType::I32 | ScalarType::U32 => (4, 4),
                ScalarType::I64 | ScalarType::U64 => (8, 8),
                _ => {
                    return Err(ModelError::InvalidLayout(
                        "enum has a non-integral ABI".into(),
                    ));
                }
            }
        }
        ComAbiType::NativeStruct(layout_id) | ComAbiType::NativeUnion(layout_id) => {
            let layout = model.layouts.get(*layout_id)?.get(architecture);
            (layout.size(), layout.alignment())
        }
        ComAbiType::Pointer { .. }
        | ComAbiType::Handle(_)
        | ComAbiType::DataPointer { .. }
        | ComAbiType::StringPointer { .. }
        | ComAbiType::Bstr
        | ComAbiType::HString
        | ComAbiType::ComInterface { .. }
        | ComAbiType::CountedBuffer { .. }
        | ComAbiType::SafeArray { .. }
        | ComAbiType::FunctionPointer(_) => (
            architecture.pointer_size(),
            architecture.pointer_alignment(),
        ),
        ComAbiType::Variant
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
        | ComAbiType::Unknown(_) => {
            return Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout));
        }
    };
    visiting.remove(&type_id);
    Ok(result)
}

fn align_up(value: usize, alignment: usize) -> Result<usize, ModelError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| ModelError::InvalidLayout("alignment overflow".into()))
}

fn map_pointer_alias(
    model: &mut ComModel,
    namespace: &str,
    name: &str,
) -> Result<TypeId, ModelError> {
    let native_name = QualifiedName::new(namespace, name)?;
    if namespace == "Windows.Win32.Foundation"
        && matches!(name, "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR")
    {
        return insert_abi(
            model,
            Some(native_name),
            None,
            ComAbiType::StringPointer {
                encoding: StringEncoding::Utf16,
                constness: if matches!(name, "PCWSTR" | "LPCWSTR") {
                    Constness::Const
                } else {
                    Constness::Mutable
                },
            },
        );
    }
    if namespace == "Windows.Win32.Foundation"
        && matches!(name, "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR")
    {
        return insert_abi(
            model,
            Some(native_name),
            None,
            ComAbiType::StringPointer {
                encoding: StringEncoding::Ansi,
                constness: if matches!(name, "PCSTR" | "LPCSTR") {
                    Constness::Const
                } else {
                    Constness::Mutable
                },
            },
        );
    }
    if is_known_data_pointer_alias(namespace, name) {
        let underlying = insert_abi(
            model,
            None,
            None,
            ComAbiType::DataPointer {
                pointee: DataPointee::Opaque(None),
                depth: NonZeroU8::new(1).unwrap(),
                constness: if matches!(name, "PCVOID" | "LPCVOID") {
                    Constness::Const
                } else {
                    Constness::Mutable
                },
            },
        )?;
        return insert_abi(
            model,
            Some(native_name.clone()),
            Some(underlying),
            ComAbiType::DataPointer {
                pointee: DataPointee::Opaque(Some(native_name)),
                depth: NonZeroU8::new(1).unwrap(),
                constness: if matches!(name, "PCVOID" | "LPCVOID") {
                    Constness::Const
                } else {
                    Constness::Mutable
                },
            },
        );
    }
    if is_known_handle_alias(namespace, name) {
        return insert_abi(
            model,
            Some(native_name.clone()),
            None,
            ComAbiType::Handle(HandleKind::new(native_name)),
        );
    }
    Err(ModelError::Unsupported(
        UnsupportedReason::UnknownPointerMeaning,
    ))
}

fn map_buffer_element(
    model: &mut ComModel,
    param: &RawComParam,
    callee_allocated: bool,
    byte_counted: bool,
) -> Result<(BufferElement, BufferElementOwnership), ModelError> {
    let raw = &param.typ;
    if let Some(string) = &param.string_pointer_array {
        let encoding = match string.encoding {
            RawStringEncoding::Utf16 => StringEncoding::Utf16,
            RawStringEncoding::Ansi => StringEncoding::Ansi,
            RawStringEncoding::Unknown => {
                return Err(ModelError::Unsupported(
                    UnsupportedReason::UnknownNativeType,
                ));
            }
        };
        let ownership = match string.ownership {
            RawElementOwnership::Borrowed => BufferElementOwnership::Borrowed,
            RawElementOwnership::CoTaskMemOwned => BufferElementOwnership::CoTaskMemStringOwned,
            RawElementOwnership::Unknown => BufferElementOwnership::Unknown,
        };
        return Ok((
            BufferElement::StringPointer {
                encoding,
                pointer_depth: nonzero_pointer_depth(string.pointer_depth)?,
                constness: map_constness(string.constness, false),
            },
            ownership,
        ));
    }
    if raw.pointer_depth == 0
        && let RawNativeType::Named {
            namespace, name, ..
        } = &raw.native_type
    {
        if namespace == "Windows.Win32.Foundation"
            && matches!(name.as_str(), "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR")
        {
            return Ok((
                BufferElement::Character(StringEncoding::Utf16),
                BufferElementOwnership::Plain,
            ));
        }
        if namespace == "Windows.Win32.Foundation"
            && matches!(name.as_str(), "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR")
        {
            return Ok((
                BufferElement::Character(StringEncoding::Ansi),
                BufferElementOwnership::Plain,
            ));
        }
    }
    if let RawNativeType::Array(raw_element) = &raw.native_type {
        let element = map_type(model, raw_element, false)?;
        let ownership = buffer_element_ownership(model, element)?;
        return Ok((BufferElement::Typed(element), ownership));
    }
    let mut element_raw = raw.clone();
    let pointer_layers = if callee_allocated { 2 } else { 1 };
    while element_raw.pointer_depth < pointer_layers {
        let Some(underlying) = element_raw.underlying.as_deref() else {
            break;
        };
        let mut expanded = underlying.clone();
        expanded.pointer_depth = expanded
            .pointer_depth
            .saturating_add(element_raw.pointer_depth);
        element_raw = expanded;
    }
    if element_raw.pointer_depth < pointer_layers {
        return Err(ModelError::Unsupported(
            UnsupportedReason::UnknownNativeType,
        ));
    }
    element_raw.pointer_depth -= pointer_layers;
    if byte_counted && matches!(element_raw.native_type, RawNativeType::Void) {
        let element = insert_abi(model, None, None, ComAbiType::Scalar(ScalarType::U8))?;
        return Ok((BufferElement::Typed(element), BufferElementOwnership::Plain));
    }
    let element = map_type(model, &element_raw, false)?;
    let ownership = buffer_element_ownership(model, element)?;
    Ok((BufferElement::Typed(element), ownership))
}

fn buffer_element_ownership(
    model: &ComModel,
    element: TypeId,
) -> Result<BufferElementOwnership, ModelError> {
    let definition = model.types().get(element)?;
    let ownership = match definition.abi() {
        ComAbiType::Scalar(_)
        | ComAbiType::Guid
        | ComAbiType::Enum(_)
        | ComAbiType::NativeStruct(_) => BufferElementOwnership::Plain,
        ComAbiType::Bstr => BufferElementOwnership::BstrOwned,
        ComAbiType::ComInterface { .. } => BufferElementOwnership::ComOwned,
        ComAbiType::Variant => BufferElementOwnership::VariantOwned,
        ComAbiType::Pointer { pointee, .. }
            if matches!(
                model.types().get(*pointee)?.abi(),
                ComAbiType::ComInterface { .. }
            ) =>
        {
            BufferElementOwnership::ComOwned
        }
        ComAbiType::StringPointer { .. }
        | ComAbiType::Pointer { .. }
        | ComAbiType::Handle(_)
        | ComAbiType::DataPointer { .. }
        | ComAbiType::HString
        | ComAbiType::NativeUnion(_)
        | ComAbiType::CountedBuffer { .. }
        | ComAbiType::SafeArray { .. }
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
        | ComAbiType::FunctionPointer(_)
        | ComAbiType::Unknown(_) => BufferElementOwnership::Unknown,
    };
    Ok(ownership)
}

fn map_ownership(
    model: &mut ComModel,
    abi_type: TypeId,
    direction: Direction,
    raw: &RawComParam,
    raw_method: &RawComMethod,
    param_index: usize,
    dynamic_iid_output: bool,
) -> Result<(ComOwnership, Cleanup), ModelError> {
    if !direction.is_output() {
        if raw.free_with.is_some() {
            return Err(ModelError::InvalidOwnership(format!(
                "input parameter `{}` declares output cleanup",
                raw.name
            )));
        }
        return Ok((ComOwnership::Borrowed, Cleanup::None));
    }
    if let ComAbiType::CountedBuffer {
        element_ownership, ..
    } = model.types().get(abi_type)?.abi()
    {
        if matches!(element_ownership, BufferElementOwnership::Plain)
            && raw.typ.pointer_depth == 2
            && raw
                .free_with
                .as_ref()
                .is_some_and(|free_with| free_with.function == "CoTaskMemFree")
        {
            return Ok((ComOwnership::CoTaskMemOwned, Cleanup::CoTaskMemFree));
        }
        match (element_ownership, raw.free_with.as_ref()) {
            (BufferElementOwnership::BstrOwned, Some(free_with))
                if free_with.function == "SysFreeString" => {}
            (
                BufferElementOwnership::Plain
                | BufferElementOwnership::ComOwned
                | BufferElementOwnership::VariantOwned
                | BufferElementOwnership::CoTaskMemStringOwned,
                None,
            ) => {}
            _ => return Err(ModelError::Unsupported(UnsupportedReason::UnknownCleanup)),
        }
        return Ok((ComOwnership::Borrowed, Cleanup::None));
    }
    let bstr_in_out = direction == Direction::InOut
        && raw
            .free_with
            .as_ref()
            .is_some_and(|free_with| free_with.function == "SysFreeString")
        && matches!(output_value_type(model, abi_type)?, ComAbiType::Bstr);
    if direction == Direction::InOut && raw.free_with.is_some() && !bstr_in_out {
        return Err(ModelError::InvalidOwnership(format!(
            "in/out parameter `{}` transfers allocator ownership",
            raw.name
        )));
    }
    if let Some(free_with) = &raw.free_with {
        return match free_with.function.as_str() {
            "CoTaskMemFree" => Ok((ComOwnership::CoTaskMemOwned, Cleanup::CoTaskMemFree)),
            "SysFreeString" if bstr_in_out => {
                Ok((ComOwnership::BstrReplaced, Cleanup::SysFreeString))
            }
            "SysFreeString" => Ok((ComOwnership::BstrOwned, Cleanup::SysFreeString)),
            "WindowsDeleteString" => Ok((ComOwnership::HStringOwned, Cleanup::WindowsDeleteString)),
            "LocalFree" => Ok((ComOwnership::LocalOwned, Cleanup::LocalFree)),
            function if handle_cleanup_namespace(function).is_some() => {
                let cleanup = HandleCleanup::new(QualifiedName::new(
                    handle_cleanup_namespace(function).unwrap(),
                    function,
                )?);
                Ok((
                    ComOwnership::HandleOwned(cleanup.clone()),
                    Cleanup::Handle(cleanup),
                ))
            }
            _ => Err(ModelError::Unsupported(UnsupportedReason::UnknownCleanup)),
        };
    }
    let output = output_value_type(model, abi_type)?;
    match output {
        ComAbiType::ComInterface { .. } if direction == Direction::Out => {
            Ok((ComOwnership::ComOwned, Cleanup::ComRelease))
        }
        ComAbiType::HString if direction == Direction::Out => {
            Ok((ComOwnership::HStringOwned, Cleanup::WindowsDeleteString))
        }
        ComAbiType::Handle(handle)
            if direction == Direction::Out
                && handle.native_name().namespace() == "Windows.Win32.Foundation"
                && handle.native_name().name() == "HWND"
                && crate::com_metadata::is_registered_borrowed_hwnd_output(
                    raw_method,
                    param_index,
                ) =>
        {
            Ok((ComOwnership::Borrowed, Cleanup::None))
        }
        ComAbiType::Handle(handle) if direction == Direction::Out => Err(
            ModelError::Unsupported(UnsupportedReason::Other(format!(
                "owned handle output `{}` requires an explicit projected cleanup owner",
                handle.native_name().name()
            ))),
        ),
        ComAbiType::Scalar(_)
        | ComAbiType::Guid
        | ComAbiType::Enum(_)
        | ComAbiType::NativeStruct(_)
        | ComAbiType::CountedBuffer { .. } => Ok((ComOwnership::Borrowed, Cleanup::None)),
        ComAbiType::Handle(_) => Ok((ComOwnership::Borrowed, Cleanup::None)),
        ComAbiType::NativeUnion(_) if direction == Direction::Out => Err(
            ModelError::Unsupported(UnsupportedReason::Other(
                "native union outputs require an explicit active-field/discriminant contract"
                    .into(),
            )),
        ),
        ComAbiType::NativeUnion(_) => Ok((ComOwnership::Borrowed, Cleanup::None)),
        ComAbiType::Variant if direction == Direction::Out => {
            Ok((ComOwnership::VariantOwned, Cleanup::VariantClear))
        }
        ComAbiType::PropVariant if direction == Direction::Out => Ok((
            ComOwnership::PropVariantOwned,
            Cleanup::PropVariantClear,
        )),
        ComAbiType::ExcepInfo if direction == Direction::Out => {
            Ok((ComOwnership::ExcepInfoOwned, Cleanup::ExcepInfoClear))
        }
        ComAbiType::StatStg if direction == Direction::Out => {
            Ok((ComOwnership::StatStgOwned, Cleanup::StatStgClear))
        }
        ComAbiType::DispatchParams if direction == Direction::Out => Err(
            ModelError::Unsupported(UnsupportedReason::Other(
                "DISPPARAMS is input-only".into(),
            )),
        ),
        ComAbiType::SafeArray { .. } if direction == Direction::Out => {
            Ok((ComOwnership::SafeArrayOwned, Cleanup::SafeArrayDestroy))
        }
        ComAbiType::Pointer { pointee, depth, .. }
            if direction == Direction::Out
                && depth.get() == 1
                && matches!(
                    model.types().get(pointee)?.abi(),
                    ComAbiType::SafeArray { .. }
                ) =>
        {
            Ok((ComOwnership::SafeArrayOwned, Cleanup::SafeArrayDestroy))
        }
        ComAbiType::Variant
        | ComAbiType::SafeArray { .. }
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
            if direction == Direction::InOut =>
        {
            Err(ModelError::Unsupported(UnsupportedReason::Other(
                "Automation BYREF/InOut parameters require an explicit borrow, replacement, and cleanup contract"
                    .into(),
            )))
        }
        ComAbiType::DataPointer { .. } if dynamic_iid_output && direction == Direction::Out => {
            Ok((ComOwnership::ComOwned, Cleanup::ComRelease))
        }
        ComAbiType::DataPointer { .. } if direction == Direction::InOut => {
            if output_value_type_id(model, abi_type).is_some_and(|output| {
                model
                    .types()
                    .get(output)
                    .is_ok_and(|definition| definition.underlying().is_some())
            }) {
                Ok((ComOwnership::Borrowed, Cleanup::None))
            } else {
                Err(ModelError::Unsupported(UnsupportedReason::UnknownLayout))
            }
        }
        ComAbiType::Pointer { .. }
        | ComAbiType::DataPointer { .. }
        | ComAbiType::StringPointer { .. }
        | ComAbiType::Bstr
        | ComAbiType::HString
        | ComAbiType::ComInterface { .. }
        | ComAbiType::SafeArray { .. }
        | ComAbiType::Variant
        | ComAbiType::PropVariant
        | ComAbiType::DispatchParams
        | ComAbiType::ExcepInfo
        | ComAbiType::StatStg
        | ComAbiType::FunctionPointer(_)
        | ComAbiType::Unknown(_) => {
            Err(ModelError::Unsupported(UnsupportedReason::UnknownOwnership))
        }
    }
}

fn output_value_type(model: &ComModel, abi_type: TypeId) -> Result<ComAbiType, ModelError> {
    let abi = model.types().get(abi_type)?.abi();
    if let ComAbiType::Pointer {
        pointee,
        depth,
        constness,
    } = abi
    {
        if depth.get() == 1 {
            return Ok(model.types().get(*pointee)?.abi().clone());
        }
        return Ok(ComAbiType::Pointer {
            pointee: *pointee,
            depth: NonZeroU8::new(depth.get() - 1).unwrap(),
            constness: *constness,
        });
    }
    if let ComAbiType::DataPointer {
        pointee,
        depth,
        constness,
    } = abi
    {
        if depth.get() > 1 {
            return Ok(ComAbiType::DataPointer {
                pointee: pointee.clone(),
                depth: NonZeroU8::new(depth.get() - 1).unwrap(),
                constness: *constness,
            });
        }
    }
    Ok(abi.clone())
}

fn output_value_type_id(model: &ComModel, abi_type: TypeId) -> Option<TypeId> {
    match model.types().get(abi_type).ok()?.abi() {
        ComAbiType::Pointer { pointee, depth, .. } if depth.get() == 1 => Some(*pointee),
        _ => Some(abi_type),
    }
}

fn dynamic_iid_contract(method: &RawComMethod) -> Result<Option<DynamicIidContract>, ModelError> {
    let iid_like = method
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| is_iid_like_parameter(param).then_some(index))
        .collect::<Vec<_>>();
    let output_like = method
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| is_dynamic_output_like(param).then_some(index))
        .collect::<Vec<_>>();
    let iid_candidates = iid_like
        .iter()
        .copied()
        .filter(|index| is_exact_iid_parameter(&method.params[*index]))
        .collect::<Vec<_>>();
    let output_candidates = output_like
        .iter()
        .copied()
        .filter(|index| is_exact_dynamic_output(&method.params[*index]))
        .collect::<Vec<_>>();

    if iid_like.is_empty() || output_like.is_empty() {
        return Ok(None);
    }
    if iid_candidates.is_empty() && output_candidates.is_empty() {
        return Ok(None);
    }
    if iid_candidates.len() != 1
        || output_candidates.len() != 1
        || iid_like.len() != 1
        || output_like.len() != 1
    {
        return Err(ModelError::InvalidContract(format!(
            "{} has an ambiguous or unsupported dynamic-IID parameter contract",
            method.projected_name
        )));
    }
    if !is_plain_hresult_return(method) {
        return Err(ModelError::InvalidContract(format!(
            "{} dynamic-IID methods require an HRESULT return",
            method.projected_name
        )));
    }
    if method
        .params
        .iter()
        .any(|param| param.direction == RawParamDirection::InOut)
    {
        return Err(ModelError::InvalidContract(format!(
            "{} dynamic-IID methods cannot contain [in, out] parameters",
            method.projected_name
        )));
    }
    DynamicIidContract::new(
        ParamIndex::new(iid_candidates[0]),
        ParamIndex::new(output_candidates[0]),
    )
    .map(Some)
}

fn is_plain_hresult_return(method: &RawComMethod) -> bool {
    method.semantic_hresult.is_none()
        && method.enumerator_next.is_none()
        && method.return_type.pointer_depth == 0
        && matches!(
            &method.return_type.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.Foundation" && name == "HRESULT"
        )
}

fn is_iid_like_parameter(param: &RawComParam) -> bool {
    matches!(param.name.to_ascii_lowercase().as_str(), "iid" | "riid")
        && matches!(
            &param.typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "System" && name == "Guid"
        )
}

fn is_exact_iid_parameter(param: &RawComParam) -> bool {
    if param.direction != RawParamDirection::In
        || param.optional
        || !matches!(param.name.to_ascii_lowercase().as_str(), "iid" | "riid")
        || param.typ.pointer_depth != 1
        || !(param.const_attribute || param.typ.constness == RawConstness::Const)
        || param.native_array.is_some()
        || param.free_with.is_some()
    {
        return false;
    }
    matches!(
        &param.typ.native_type,
        RawNativeType::Named {
            namespace,
            name,
            ..
        } if namespace == "System" && name == "Guid"
    )
}

fn is_dynamic_output_like(param: &RawComParam) -> bool {
    param.direction != RawParamDirection::In
        && param.typ.pointer_depth > 0
        && matches!(
            param.typ.native_type,
            RawNativeType::Object | RawNativeType::Void
        )
}

fn is_exact_dynamic_output(param: &RawComParam) -> bool {
    param.direction == RawParamDirection::Out
        && !param.optional
        && param.typ.pointer_depth == 2
        && param.typ.constness == RawConstness::Mutable
        && matches!(
            param.typ.native_type,
            RawNativeType::Object | RawNativeType::Void
        )
        && param.native_array.is_none()
        && param.free_with.is_none()
}

fn is_hresult(model: &ComModel, abi_type: TypeId) -> Result<bool, ModelError> {
    Ok(matches!(
        model.types().get(abi_type)?.abi(),
        ComAbiType::Scalar(ScalarType::HResult)
    ))
}

fn is_nullable_type(model: &ComModel, abi_type: TypeId) -> Result<bool, ModelError> {
    Ok(matches!(
        model.types().get(abi_type)?.abi(),
        ComAbiType::Pointer { .. }
            | ComAbiType::Handle(_)
            | ComAbiType::DataPointer { .. }
            | ComAbiType::StringPointer { .. }
            | ComAbiType::Bstr
            | ComAbiType::HString
            | ComAbiType::ComInterface { .. }
            | ComAbiType::CountedBuffer { .. }
            | ComAbiType::SafeArray { .. }
            | ComAbiType::FunctionPointer(_)
    ))
}

fn raw_native_name(raw: &RawComType) -> Result<Option<QualifiedName>, ModelError> {
    match &raw.native_type {
        RawNativeType::Named {
            namespace, name, ..
        } => Ok(Some(QualifiedName::new(namespace, name)?)),
        _ => Ok(None),
    }
}

fn insert_abi(
    model: &mut ComModel,
    native_name: Option<QualifiedName>,
    underlying: Option<TypeId>,
    abi: ComAbiType,
) -> Result<TypeId, ModelError> {
    model
        .types_mut()
        .insert(ComTypeDefinition::new(native_name, underlying, abi))
}

fn map_direction(direction: RawParamDirection) -> Direction {
    match direction {
        RawParamDirection::In => Direction::In,
        RawParamDirection::Out => Direction::Out,
        RawParamDirection::InOut => Direction::InOut,
    }
}

fn map_count_relation(
    buffer_direction: RawParamDirection,
    count_direction: RawParamDirection,
    count_param: ParamIndex,
    actual_length_param: Option<ParamIndex>,
    unit: CountUnit,
    two_call: bool,
    projected_capacity: bool,
) -> CountRelation {
    match buffer_direction {
        RawParamDirection::In => CountRelation::InputCount {
            count_param,
            actual_length_param,
            unit,
        },
        RawParamDirection::Out | RawParamDirection::InOut => match count_direction {
            RawParamDirection::In => CountRelation::CallerCapacity {
                capacity_param: count_param,
                actual_length_param,
                unit,
                sizing: if two_call {
                    BufferSizing::TwoCall { max_retries: 2 }
                } else if projected_capacity {
                    BufferSizing::FixedCapacity
                } else {
                    BufferSizing::SingleCall
                },
            },
            RawParamDirection::InOut => CountRelation::CallerCapacity {
                capacity_param: count_param,
                actual_length_param: Some(actual_length_param.unwrap_or(count_param)),
                unit,
                sizing: if two_call {
                    BufferSizing::TwoCall { max_retries: 2 }
                } else if projected_capacity {
                    BufferSizing::FixedCapacity
                } else {
                    BufferSizing::SingleCall
                },
            },
            RawParamDirection::Out => CountRelation::CalleeAllocated { count_param, unit },
        },
    }
}

fn map_constness(constness: RawConstness, const_attribute: bool) -> Constness {
    if constness == RawConstness::Mixed {
        Constness::Unspecified
    } else if const_attribute {
        Constness::Const
    } else {
        match constness {
            RawConstness::Const => Constness::Const,
            RawConstness::Mutable => Constness::Mutable,
            RawConstness::Mixed | RawConstness::Unspecified => Constness::Unspecified,
        }
    }
}

fn buffer_constness(raw: &RawComParam) -> Constness {
    if raw.const_attribute {
        return Constness::Const;
    }
    match raw.typ.constness {
        RawConstness::Const => return Constness::Const,
        RawConstness::Mutable => return Constness::Mutable,
        RawConstness::Mixed | RawConstness::Unspecified => {}
    }
    if raw.typ.pointer_depth > 0 {
        return Constness::Unspecified;
    }
    if let RawNativeType::Named { name, .. } = &raw.typ.native_type {
        if matches!(name.as_str(), "PCWSTR" | "LPCWSTR" | "PCSTR" | "LPCSTR") {
            return Constness::Const;
        }
    }
    Constness::Mutable
}

fn nonzero_pointer_depth(depth: usize) -> Result<NonZeroU8, ModelError> {
    u8::try_from(depth)
        .ok()
        .and_then(NonZeroU8::new)
        .ok_or_else(|| ModelError::InvalidContract(format!("unsupported pointer depth {depth}")))
}

fn raw_scalar(typ: &RawComType) -> Option<ScalarType> {
    if typ.pointer_depth != 0 {
        return None;
    }
    match typ.native_type {
        RawNativeType::Bool => Some(ScalarType::Bool),
        RawNativeType::I8 => Some(ScalarType::I8),
        RawNativeType::U8 => Some(ScalarType::U8),
        RawNativeType::I16 => Some(ScalarType::I16),
        RawNativeType::U16 => Some(ScalarType::U16),
        RawNativeType::I32 => Some(ScalarType::I32),
        RawNativeType::U32 => Some(ScalarType::U32),
        RawNativeType::I64 => Some(ScalarType::I64),
        RawNativeType::U64 => Some(ScalarType::U64),
        RawNativeType::F32 => Some(ScalarType::F32),
        RawNativeType::F64 => Some(ScalarType::F64),
        RawNativeType::Char16 => Some(ScalarType::Char16),
        RawNativeType::ISize => Some(ScalarType::NativeIsize),
        RawNativeType::USize => Some(ScalarType::NativeUsize),
        _ => None,
    }
}

fn raw_scalar_alias(typ: &RawComType, namespace: &str, alias_name: &str) -> Option<ScalarType> {
    match (namespace, alias_name) {
        ("Windows.Win32.Foundation", "LPARAM" | "LRESULT") => Some(ScalarType::NativeIsize),
        ("Windows.Win32.Foundation", "WPARAM") => Some(ScalarType::NativeUsize),
        _ => raw_scalar(typ),
    }
}

fn handle_cleanup_namespace(function: &str) -> Option<&'static str> {
    match function {
        "CloseHandle" | "DestroyIcon" => Some("Windows.Win32.Foundation"),
        "RegCloseKey" => Some("Windows.Win32.System.Registry"),
        _ => None,
    }
}

fn is_known_handle_alias(namespace: &str, name: &str) -> bool {
    match namespace {
        "Windows.Win32.Foundation" => matches!(
            name,
            "HANDLE" | "HWND" | "HGLOBAL" | "HINSTANCE" | "HLOCAL" | "HMODULE" | "HRSRC"
        ),
        "Windows.Win32.Graphics.Gdi" => matches!(
            name,
            "HBITMAP"
                | "HBRUSH"
                | "HDC"
                | "HENHMETAFILE"
                | "HFONT"
                | "HGDIOBJ"
                | "HMETAFILE"
                | "HPALETTE"
                | "HPEN"
                | "HRGN"
        ),
        "Windows.Win32.UI.WindowsAndMessaging" => matches!(
            name,
            "HWND"
                | "HACCEL"
                | "HCURSOR"
                | "HDESK"
                | "HDWP"
                | "HHOOK"
                | "HICON"
                | "HMENU"
                | "HMONITOR"
                | "HWINSTA"
        ),
        "Windows.Win32.System.Registry" => name == "HKEY",
        "Windows.Win32.UI.Controls" => matches!(name, "HIMAGELIST" | "HTHEME"),
        "Windows.Win32.UI.Input.KeyboardAndMouse" => name == "HKL",
        "Windows.Win32.System.Services" => {
            matches!(name, "SC_HANDLE" | "SERVICE_STATUS_HANDLE")
        }
        "Windows.Win32.UI.HiDpi" => name == "DPI_AWARENESS_CONTEXT",
        _ => false,
    }
}

fn is_known_data_pointer_alias(namespace: &str, name: &str) -> bool {
    matches!(
        (namespace, name),
        ("Windows.Win32.Security", "PSID" | "PSECURITY_DESCRIPTOR")
            | ("Windows.Win32.System.Memory", "MEMORY_MAPPED_VIEW_ADDRESS")
            | (
                "Windows.Win32.System.Threading",
                "LPPROC_THREAD_ATTRIBUTE_LIST"
            )
            | (
                "Windows.Win32.Foundation",
                "PVOID" | "PCVOID" | "LPVOID" | "LPCVOID"
            )
    )
}

fn is_explicit_pointer_alias(namespace: &str, name: &str) -> bool {
    (namespace == "Windows.Win32.Foundation"
        && matches!(
            name,
            "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR" | "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR"
        ))
        || is_known_data_pointer_alias(namespace, name)
        || is_known_handle_alias(namespace, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_attribute_does_not_erase_mixed_pointer_qualifiers() {
        assert_eq!(
            map_constness(RawConstness::Mixed, true),
            Constness::Unspecified
        );
        assert_eq!(
            map_constness(RawConstness::Unspecified, true),
            Constness::Const
        );
    }

    fn win32_winmd() -> Option<String> {
        std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
    }

    fn raw_string_alias(name: &str, pointer_depth: usize) -> RawComType {
        RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: name.into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: None,
            pointer_depth,
            constness: RawConstness::Mutable,
        }
    }

    fn raw_guid_param(name: &str, pointer_depth: usize) -> RawComParam {
        RawComParam {
            name: name.into(),
            typ: RawComType {
                native_type: RawNativeType::Named {
                    namespace: "System".into(),
                    name: "Guid".into(),
                    kind: RawNamedKind::Unknown,
                    iid: None,
                    layout: None,
                },
                underlying: None,
                pointer_depth,
                constness: RawConstness::Mutable,
            },
            direction: RawParamDirection::In,
            optional: false,
            const_attribute: true,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        }
    }

    fn raw_void_output(name: &str, pointer_depth: usize) -> RawComParam {
        RawComParam {
            name: name.into(),
            typ: RawComType {
                native_type: RawNativeType::Void,
                underlying: None,
                pointer_depth,
                constness: RawConstness::Mutable,
            },
            direction: RawParamDirection::Out,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        }
    }

    fn raw_dynamic_method(params: Vec<RawComParam>) -> RawComMethod {
        RawComMethod {
            declaring_namespace: "Test".into(),
            declaring_interface: "ITest".into(),
            declaring_iid: "00000000-0000-0000-c000-000000000046".into(),
            metadata_name: "Resolve".into(),
            projected_name: "Resolve".into(),
            vtable_index: 3,
            params,
            return_type: RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "HRESULT".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(RawComType {
                    native_type: RawNativeType::I32,
                    underlying: None,
                    pointer_depth: 0,
                    constness: RawConstness::Unspecified,
                })),
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
            semantic_hresult: None,
            enumerator_next: None,
            exact_contract: None,
            interface_replacement_contracts: Vec::new(),
            exact_interface_output_call: None,
            safe_array_contract_error: None,
        }
    }

    #[test]
    fn dynamic_iid_contract_uses_explicit_non_positional_indices() {
        let scalar = RawComParam {
            name: "flags".into(),
            typ: RawComType {
                native_type: RawNativeType::U32,
                underlying: None,
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            },
            direction: RawParamDirection::In,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        };
        let method = raw_dynamic_method(vec![
            scalar.clone(),
            raw_guid_param("riid", 1),
            scalar.clone(),
            raw_void_output("object", 2),
            scalar,
        ]);
        let contract = dynamic_iid_contract(&method).unwrap().unwrap();
        assert_eq!(contract.iid_param_index().index(), 1);
        assert_eq!(contract.output_param_index().index(), 3);
    }

    #[test]
    fn dynamic_iid_contract_fails_closed_on_nearest_unsafe_shapes() {
        let assert_rejected = |params: Vec<RawComParam>| {
            assert!(dynamic_iid_contract(&raw_dynamic_method(params)).is_err());
        };

        let mut by_value = raw_guid_param("iid", 0);
        by_value.const_attribute = false;
        assert_rejected(vec![by_value, raw_void_output("object", 2)]);

        let mut mutable = raw_guid_param("iid", 1);
        mutable.const_attribute = false;
        assert_rejected(vec![mutable, raw_void_output("object", 2)]);
        assert_rejected(vec![raw_guid_param("iid", 2), raw_void_output("object", 2)]);
        assert_rejected(vec![raw_guid_param("iid", 1), raw_void_output("object", 1)]);
        assert_rejected(vec![raw_guid_param("iid", 1), raw_void_output("object", 3)]);

        let mut optional_iid = raw_guid_param("iid", 1);
        optional_iid.optional = true;
        assert_rejected(vec![optional_iid, raw_void_output("object", 2)]);
        let mut optional_output = raw_void_output("object", 2);
        optional_output.optional = true;
        assert_rejected(vec![raw_guid_param("iid", 1), optional_output]);

        let mut array_iid = raw_guid_param("iid", 1);
        array_iid.native_array = Some(crate::com_metadata::RawArrayRelation {
            count_param_index: Some(0),
            actual_length_param_index: None,
            unit: RawCountUnit::Elements,
            two_call: false,
            projected_capacity: false,
            constness: None,
            evidence: Vec::new(),
        });
        assert_rejected(vec![array_iid, raw_void_output("object", 2)]);

        let mut free_output = raw_void_output("object", 2);
        free_output.free_with = Some(crate::com_metadata::RawFreeWith {
            function: "CoTaskMemFree".into(),
            evidence: crate::com_metadata::RawEvidence::MetadataAttribute("FreeWithAttribute"),
        });
        assert_rejected(vec![raw_guid_param("iid", 1), free_output]);

        assert_rejected(vec![
            raw_guid_param("iid", 1),
            raw_guid_param("riid", 1),
            raw_void_output("object", 2),
        ]);
        assert_rejected(vec![
            raw_guid_param("iid", 1),
            raw_void_output("first", 2),
            raw_void_output("second", 2),
        ]);

        let mut in_out = RawComParam {
            name: "count".into(),
            typ: RawComType {
                native_type: RawNativeType::U32,
                underlying: None,
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawParamDirection::InOut,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        };
        assert_rejected(vec![
            raw_guid_param("iid", 1),
            raw_void_output("object", 2),
            in_out.clone(),
        ]);
        in_out.native_array = Some(crate::com_metadata::RawArrayRelation {
            count_param_index: Some(2),
            actual_length_param_index: None,
            unit: RawCountUnit::Elements,
            two_call: false,
            projected_capacity: false,
            constness: None,
            evidence: Vec::new(),
        });
        assert_rejected(vec![
            raw_guid_param("iid", 1),
            raw_void_output("objects", 2),
            in_out,
        ]);

        let mut non_hresult =
            raw_dynamic_method(vec![raw_guid_param("iid", 1), raw_void_output("object", 2)]);
        non_hresult.return_type = RawComType {
            native_type: RawNativeType::U32,
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        assert!(dynamic_iid_contract(&non_hresult).is_err());
    }

    #[test]
    fn string_pointer_arrays_are_distinct_from_character_buffers() {
        let mut model = ComModel::default();
        let string_array = RawComParam {
            name: "names".into(),
            typ: raw_string_alias("PCWSTR", 1),
            direction: RawParamDirection::In,
            optional: false,
            const_attribute: false,
            native_array: None,
            string_pointer_array: Some(crate::com_metadata::RawStringPointerArray {
                encoding: RawStringEncoding::Utf16,
                pointer_depth: 1,
                constness: RawConstness::Const,
                ownership: RawElementOwnership::Borrowed,
            }),
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        };
        assert_eq!(
            map_buffer_element(&mut model, &string_array, false, false).unwrap(),
            (
                BufferElement::StringPointer {
                    encoding: StringEncoding::Utf16,
                    pointer_depth: NonZeroU8::new(1).unwrap(),
                    constness: Constness::Const,
                },
                BufferElementOwnership::Borrowed,
            )
        );

        let character_buffer = RawComParam {
            name: "text".into(),
            typ: raw_string_alias("PWSTR", 0),
            direction: RawParamDirection::Out,
            string_pointer_array: None,
            ..string_array.clone()
        };
        assert_eq!(
            map_buffer_element(&mut model, &character_buffer, false, false).unwrap(),
            (
                BufferElement::Character(StringEncoding::Utf16),
                BufferElementOwnership::Plain,
            )
        );

        let counted_nonterminated = RawComParam {
            name: "characters".into(),
            typ: RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.Foundation".into(),
                    name: "PWCHAR".into(),
                    kind: RawNamedKind::Struct,
                    iid: None,
                    layout: None,
                },
                underlying: Some(Box::new(RawComType {
                    native_type: RawNativeType::Char16,
                    underlying: None,
                    pointer_depth: 1,
                    constness: RawConstness::Mutable,
                })),
                pointer_depth: 0,
                constness: RawConstness::Mutable,
            },
            direction: RawParamDirection::Out,
            string_pointer_array: None,
            ..character_buffer.clone()
        };
        assert!(map_type(&mut model, &counted_nonterminated.typ, false).is_err());
        let (element, ownership) =
            map_buffer_element(&mut model, &counted_nonterminated, false, false).unwrap();
        let BufferElement::Typed(element) = element else {
            panic!("counted PWCHAR must remain a typed character buffer");
        };
        assert_eq!(
            model.types().get(element).unwrap().abi(),
            &ComAbiType::Scalar(ScalarType::Char16)
        );
        assert_eq!(ownership, BufferElementOwnership::Plain);

        let unknown = RawComParam {
            string_pointer_array: Some(crate::com_metadata::RawStringPointerArray {
                encoding: RawStringEncoding::Unknown,
                pointer_depth: 1,
                constness: RawConstness::Const,
                ownership: RawElementOwnership::Borrowed,
            }),
            ..string_array
        };
        assert!(map_buffer_element(&mut model, &unknown, false, false).is_err());
    }

    #[test]
    fn taskbar_metadata_maps_to_complete_semantic_contracts() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "ITaskbarList3",
        )
        .unwrap();
        let semantic = map_interface(&interface).unwrap();

        assert_eq!(
            semantic.iid,
            ComGuid::parse(&interface.interface.iid).unwrap()
        );
        assert_eq!(semantic.methods.len(), interface.interface.methods.len());
        semantic.validate().unwrap();
        assert_windows_rs_iid::<windows::Win32::UI::Shell::ITaskbarList3>(&semantic);

        let add_buttons = semantic
            .methods()
            .iter()
            .find(|method| method.name() == "ThumbBarAddButtons")
            .unwrap();
        let buttons = semantic
            .type_definition(add_buttons.params()[2].abi_type())
            .unwrap();
        let ComAbiType::CountedBuffer {
            element: BufferElement::Typed(element),
            ..
        } = buttons.abi()
        else {
            panic!("THUMBBUTTON input must be a typed counted buffer");
        };
        let ComAbiType::NativeStruct(layout_id) = semantic.type_definition(*element).unwrap().abi()
        else {
            panic!("THUMBBUTTON element must be a native POD");
        };
        assert_windows_rs_host_layout::<windows::Win32::UI::Shell::THUMBBUTTON>(
            &semantic, *layout_id,
        );
        assert_windows_rs_field_offsets(
            &semantic,
            *layout_id,
            &[
                (
                    "dwMask",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, dwMask),
                ),
                (
                    "iId",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, iId),
                ),
                (
                    "iBitmap",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, iBitmap),
                ),
                (
                    "hIcon",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, hIcon),
                ),
                (
                    "szTip",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, szTip),
                ),
                (
                    "dwFlags",
                    std::mem::offset_of!(windows::Win32::UI::Shell::THUMBBUTTON, dwFlags),
                ),
            ],
        );

        let set_clip = semantic
            .methods()
            .iter()
            .find(|method| method.name() == "SetThumbnailClip")
            .unwrap();
        let clip = semantic
            .type_definition(set_clip.params()[1].abi_type())
            .unwrap();
        let ComAbiType::Pointer { pointee, .. } = clip.abi() else {
            panic!("RECT input must be pointer-shaped");
        };
        let ComAbiType::NativeStruct(layout_id) = semantic.type_definition(*pointee).unwrap().abi()
        else {
            panic!("RECT pointee must be a native POD");
        };
        assert_windows_rs_host_layout::<windows::Win32::Foundation::RECT>(&semantic, *layout_id);
        assert_windows_rs_field_offsets(
            &semantic,
            *layout_id,
            &[
                (
                    "left",
                    std::mem::offset_of!(windows::Win32::Foundation::RECT, left),
                ),
                (
                    "top",
                    std::mem::offset_of!(windows::Win32::Foundation::RECT, top),
                ),
                (
                    "right",
                    std::mem::offset_of!(windows::Win32::Foundation::RECT, right),
                ),
                (
                    "bottom",
                    std::mem::offset_of!(windows::Win32::Foundation::RECT, bottom),
                ),
            ],
        );
    }

    #[test]
    fn raw_pointer_depth_distinguishes_refiid_and_void_double_pointer() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "IDataTransferManagerInterop",
        )
        .unwrap();
        let method = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| method.metadata_name == "GetForWindow")
            .unwrap();
        let iid = &method.params[method.params.len() - 2];
        let result = &method.params[method.params.len() - 1];

        assert_eq!(iid.typ.pointer_depth, 1);
        assert!(iid.const_attribute);
        assert_eq!(result.typ.pointer_depth, 2);
        map_interface(&interface).unwrap();
    }

    #[test]
    fn shell_link_find_data_has_exact_nested_fixed_array_layouts() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.UI.Shell",
            "IShellLinkW",
        )
        .unwrap();
        let semantic = map_interface(&interface).unwrap();
        assert_windows_rs_iid::<windows::Win32::UI::Shell::IShellLinkW>(&semantic);
        let get_path = semantic
            .methods()
            .iter()
            .find(|method| method.name() == "GetPath")
            .unwrap();
        let pfd = semantic
            .type_definition(get_path.params()[2].abi_type())
            .unwrap();
        let ComAbiType::Pointer { pointee, depth, .. } = pfd.abi() else {
            panic!("WIN32_FIND_DATAW must be pointer-shaped");
        };
        assert_eq!(depth.get(), 1);
        let ComAbiType::NativeStruct(layout_id) = semantic.type_definition(*pointee).unwrap().abi()
        else {
            panic!("WIN32_FIND_DATAW pointee must be a native POD");
        };
        for architecture in Architecture::ALL {
            let layout = semantic
                .layout_definition(*layout_id)
                .unwrap()
                .get(architecture);
            assert_eq!(layout.size(), 592);
            assert_eq!(layout.alignment(), 4);
            assert_eq!(layout.fields().len(), 10);
            assert_eq!(layout.fields()[1].offset(), 4);
            assert_eq!(layout.fields()[8].offset(), 44);
            assert_eq!(layout.fields()[8].fixed_count().unwrap().get(), 260);
            assert_eq!(layout.fields()[9].offset(), 564);
            assert_eq!(layout.fields()[9].fixed_count().unwrap().get(), 14);
        }
        assert_windows_rs_host_layout::<windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW>(
            &semantic, *layout_id,
        );
        assert_windows_rs_field_offsets(
            &semantic,
            *layout_id,
            &[
                (
                    "dwFileAttributes",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        dwFileAttributes
                    ),
                ),
                (
                    "ftCreationTime",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        ftCreationTime
                    ),
                ),
                (
                    "ftLastAccessTime",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        ftLastAccessTime
                    ),
                ),
                (
                    "ftLastWriteTime",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        ftLastWriteTime
                    ),
                ),
                (
                    "nFileSizeHigh",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        nFileSizeHigh
                    ),
                ),
                (
                    "nFileSizeLow",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        nFileSizeLow
                    ),
                ),
                (
                    "dwReserved0",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        dwReserved0
                    ),
                ),
                (
                    "dwReserved1",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        dwReserved1
                    ),
                ),
                (
                    "cFileName",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        cFileName
                    ),
                ),
                (
                    "cAlternateFileName",
                    std::mem::offset_of!(
                        windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW,
                        cAlternateFileName
                    ),
                ),
            ],
        );
    }

    #[test]
    fn windows_rs_interface_iids_match_dynamic_win32_metadata() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        assert_interface_iid::<windows::Win32::System::Com::IPersistFile>(
            &winmd,
            "Windows.Win32.System.Com",
            "IPersistFile",
            true,
        );
        assert_interface_iid::<windows::Win32::System::Com::ISequentialStream>(
            &winmd,
            "Windows.Win32.System.Com",
            "ISequentialStream",
            true,
        );
        assert_interface_iid::<windows::Win32::UI::Shell::IDataTransferManagerInterop>(
            &winmd,
            "Windows.Win32.UI.Shell",
            "IDataTransferManagerInterop",
            true,
        );
        assert_interface_iid::<windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop>(
            &winmd,
            "Windows.Win32.System.WinRT",
            "ISystemMediaTransportControlsInterop",
            false,
        );
    }

    fn assert_interface_iid<T: windows::core::Interface>(
        winmd: &str,
        namespace: &str,
        name: &str,
        is_iunknown_rooted: bool,
    ) {
        let interface = crate::com_metadata::parse_com_interface(winmd, namespace, name).unwrap();
        let semantic = map_interface(&interface).unwrap();
        assert_eq!(semantic.is_iunknown_rooted(), is_iunknown_rooted);
        assert_windows_rs_iid::<T>(&semantic);
    }

    fn assert_windows_rs_iid<T: windows::core::Interface>(semantic: &SemanticComInterface) {
        assert_eq!(
            semantic.iid(),
            ComGuid::from_bytes(T::IID.to_u128().to_be_bytes())
        );
    }

    fn assert_windows_rs_host_layout<T>(
        semantic: &SemanticComInterface,
        layout_id: super::super::ids::LayoutId,
    ) {
        let architecture = host_architecture();
        let layout = semantic
            .layout_definition(layout_id)
            .unwrap()
            .get(architecture);
        assert_eq!(layout.size(), std::mem::size_of::<T>());
        assert_eq!(layout.alignment(), std::mem::align_of::<T>());
    }

    fn assert_windows_rs_field_offsets(
        semantic: &SemanticComInterface,
        layout_id: super::super::ids::LayoutId,
        expected: &[(&str, usize)],
    ) {
        let architecture = host_architecture();
        let layout = semantic
            .layout_definition(layout_id)
            .unwrap()
            .get(architecture);
        assert_eq!(layout.fields().len(), expected.len());
        for (name, offset) in expected {
            let field = layout
                .fields()
                .iter()
                .find(|field| field.name() == *name)
                .unwrap_or_else(|| panic!("semantic layout has no `{name}` field"));
            assert_eq!(field.offset(), *offset, "offset mismatch for `{name}`");
        }
    }

    fn host_architecture() -> Architecture {
        if cfg!(target_arch = "x86") {
            Architecture::X86
        } else if cfg!(target_arch = "x86_64") {
            Architecture::X64
        } else if cfg!(target_arch = "aarch64") {
            Architecture::Arm64
        } else {
            panic!("unsupported Windows architecture for ABI oracle");
        }
    }

    fn raw_scalar_type(native_type: RawNativeType) -> RawComType {
        RawComType {
            native_type,
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        }
    }

    fn raw_field(name: &str, native_type: RawNativeType) -> crate::com_metadata::RawNativeField {
        crate::com_metadata::RawNativeField {
            name: name.into(),
            typ: raw_scalar_type(native_type),
            explicit_offset: None,
            fixed_count: None,
            bitfield: false,
            flexible_array: false,
        }
    }

    fn raw_named_type(namespace: &str, name: &str, pointer_depth: usize) -> RawComType {
        RawComType {
            native_type: RawNativeType::Named {
                namespace: namespace.into(),
                name: name.into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: None,
            pointer_depth,
            constness: RawConstness::Unspecified,
        }
    }

    #[test]
    fn automation_families_map_to_dedicated_semantic_types() {
        let cases = [
            ("Windows.Win32.System.Variant", "VARIANT", "VARIANT"),
            ("Windows.Win32.System.Com", "SAFEARRAY", "SAFEARRAY"),
            ("Windows.Win32.System.Com", "DISPPARAMS", "DISPPARAMS"),
            ("Windows.Win32.System.Com", "EXCEPINFO", "EXCEPINFO"),
            (
                "Windows.Win32.System.Com.StructuredStorage",
                "PROPVARIANT",
                "PROPVARIANT",
            ),
        ];
        for (namespace, name, expected) in cases {
            let mut model = ComModel::default();
            let typ = map_type(&mut model, &raw_named_type(namespace, name, 0), false).unwrap();
            let abi = model.types().get(typ).unwrap().abi();
            assert!(
                matches!(
                    (expected, abi),
                    ("VARIANT", ComAbiType::Variant)
                        | ("SAFEARRAY", ComAbiType::SafeArray { .. })
                        | ("PROPVARIANT", ComAbiType::PropVariant)
                        | ("DISPPARAMS", ComAbiType::DispatchParams)
                        | ("EXCEPINFO", ComAbiType::ExcepInfo)
                ),
                "{name} mapped to {abi:?}"
            );
        }
    }

    #[test]
    fn sequential_and_explicit_layouts_are_architecture_exact() {
        let sequential = RawNativeLayout {
            architectures: 0b111,
            kind: RawLayoutKind::Sequential,
            packing: RawPacking::Default,
            declared_size: None,
            fields: vec![
                raw_field("tag", RawNativeType::U32),
                raw_field("address", RawNativeType::USize),
            ],
            is_union: false,
        };
        let mut x86_model = ComModel::default();
        let x86 =
            compute_native_layout_variant(&mut x86_model, &sequential, Architecture::X86).unwrap();
        assert_eq!((x86.size(), x86.alignment()), (8, 4));
        assert_eq!(x86.fields()[1].offset(), 4);

        let mut x64_model = ComModel::default();
        let x64 =
            compute_native_layout_variant(&mut x64_model, &sequential, Architecture::X64).unwrap();
        assert_eq!((x64.size(), x64.alignment()), (16, 8));
        assert_eq!(x64.fields()[1].offset(), 8);

        let mut explicit = sequential.clone();
        explicit.kind = RawLayoutKind::Explicit;
        explicit.declared_size = Some(16);
        explicit.fields[0].explicit_offset = Some(0);
        explicit.fields[1].explicit_offset = Some(8);
        let mut model = ComModel::default();
        let layout =
            compute_native_layout_variant(&mut model, &explicit, Architecture::Arm64).unwrap();
        assert_eq!((layout.size(), layout.alignment()), (16, 8));
        assert_eq!(layout.fields()[1].offset(), 8);
    }

    #[test]
    fn unions_are_overlapping_and_nearest_unsupported_pod_shapes_fail_closed() {
        let base = RawNativeLayout {
            architectures: 0b111,
            kind: RawLayoutKind::Sequential,
            packing: RawPacking::Default,
            declared_size: None,
            fields: vec![raw_field("value", RawNativeType::U32)],
            is_union: false,
        };
        let assert_rejected = |layout: RawNativeLayout, expected: &str| {
            let error =
                compute_native_layout_variant(&mut ComModel::default(), &layout, Architecture::X64)
                    .unwrap_err()
                    .to_string();
            assert!(error.contains(expected), "{error}");
        };

        let mut union = base.clone();
        union.kind = RawLayoutKind::Explicit;
        union.is_union = true;
        union.fields[0].explicit_offset = Some(0);
        union.fields.push(raw_field("other", RawNativeType::U16));
        union.fields[1].explicit_offset = Some(0);
        let union =
            compute_native_layout_variant(&mut ComModel::default(), &union, Architecture::X64)
                .unwrap();
        assert_eq!(union.kind(), LayoutKind::Union);
        assert!(union.fields().iter().all(|field| field.offset() == 0));

        let mut bitfield = base.clone();
        bitfield.fields[0].bitfield = true;
        assert_rejected(bitfield.clone(), "bitfield");

        let mut flexible = base.clone();
        flexible.fields[0].flexible_array = true;
        assert_rejected(flexible, "flexible array");

        let pointer_to_defined_bitfield = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Test".into(),
                name: "BITFIELD".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: Some(Box::new(RawNativeLayoutSet {
                    recursive: false,
                    variants: vec![bitfield],
                })),
            },
            underlying: None,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        let error = map_type(
            &mut ComModel::default(),
            &pointer_to_defined_bitfield,
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("bitfield"), "{error}");

        let mut unknown_packing = base.clone();
        unknown_packing.packing = RawPacking::Unknown;
        assert_rejected(unknown_packing, "packing");

        let mut misaligned = base.clone();
        misaligned.kind = RawLayoutKind::Explicit;
        misaligned.fields[0].explicit_offset = Some(2);
        assert_rejected(misaligned, "alignment");

        let mut overlap = base.clone();
        overlap.kind = RawLayoutKind::Explicit;
        overlap.fields[0].explicit_offset = Some(0);
        let mut second = raw_field("other", RawNativeType::U32);
        second.explicit_offset = Some(0);
        overlap.fields.push(second);
        assert_rejected(overlap, "overlapping");

        let mut out_of_bounds = base.clone();
        out_of_bounds.declared_size = Some(2);
        assert_rejected(out_of_bounds, "cannot contain");

        let mut overflow = base.clone();
        overflow.kind = RawLayoutKind::Explicit;
        overflow.fields[0].explicit_offset = Some(usize::MAX - 3);
        assert_rejected(overflow, "overflows");

        let mut owned = base.clone();
        owned.fields[0].typ = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: "BSTR".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        assert_rejected(owned, "ownership");

        let mut interface_pointer = base;
        interface_pointer.fields[0].typ = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Test".into(),
                name: "IFoo".into(),
                kind: RawNamedKind::Interface,
                iid: Some("00000000-0000-0000-c000-000000000046".into()),
                layout: None,
            },
            underlying: None,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        assert_rejected(interface_pointer, "ownership");
    }

    #[test]
    fn hfile_preserves_its_signed_32_bit_abi() {
        let raw = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HFILE".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: Some(Box::new(raw_scalar_type(RawNativeType::I32))),
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let mut model = ComModel::default();
        let typ = map_type(&mut model, &raw, false).unwrap();
        assert!(matches!(
            model.types().get(typ).unwrap().abi(),
            ComAbiType::Scalar(ScalarType::I32)
        ));
    }

    #[test]
    fn pointer_aliases_require_exact_qualified_identity() {
        let mut model = ComModel::default();
        assert!(map_pointer_alias(&mut model, "Windows.Win32.Foundation", "HWND").is_ok());
        assert!(map_pointer_alias(&mut model, "Contoso", "HWND").is_err());
        assert!(map_pointer_alias(&mut model, "Contoso", "PWSTR").is_err());
        let u32_type = raw_scalar_type(RawNativeType::U32);
        assert_eq!(
            raw_scalar_alias(&u32_type, "Windows.Win32.Foundation", "WPARAM"),
            Some(ScalarType::NativeUsize)
        );
        assert_eq!(
            raw_scalar_alias(&u32_type, "Contoso", "WPARAM"),
            Some(ScalarType::U32)
        );
    }

    #[test]
    fn recursive_and_missing_architecture_layouts_fail_closed() {
        let variant = RawNativeLayout {
            architectures: Architecture::X64.metadata_mask(),
            kind: RawLayoutKind::Sequential,
            packing: RawPacking::Default,
            declared_size: None,
            fields: vec![raw_field("value", RawNativeType::U32)],
            is_union: false,
        };
        let recursive = RawNativeLayoutSet {
            recursive: true,
            variants: vec![variant.clone()],
        };
        let error = map_native_struct(&mut ComModel::default(), "Test", "Recursive", &recursive)
            .unwrap_err()
            .to_string();
        assert!(error.contains("recursive"), "{error}");

        let missing = RawNativeLayoutSet {
            recursive: false,
            variants: vec![variant],
        };
        let error = compute_native_layout(&mut ComModel::default(), &missing, Architecture::X86)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing X86"), "{error}");
    }

    #[test]
    fn every_live_codegen_interface_maps_to_semantic_contracts() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        for (namespace, name) in [
            ("Windows.Win32.UI.Shell", "ITaskbarList4"),
            ("Windows.Win32.UI.Shell", "IDataTransferManagerInterop"),
            ("Windows.Win32.UI.Shell", "IFileOperation"),
            ("Windows.Win32.UI.Shell", "IFileOpenDialog"),
            ("Windows.Win32.System.Com", "IPersistFile"),
            (
                "Windows.Win32.System.WinRT",
                "ISystemMediaTransportControlsInterop",
            ),
            ("Windows.Win32.Graphics.Imaging", "IWICImagingFactory"),
        ] {
            let interface =
                crate::com_metadata::parse_com_interface(&winmd, namespace, name).unwrap();
            map_interface(&interface)
                .unwrap_or_else(|error| panic!("{namespace}.{name} failed to map: {error}"));
        }
    }

    #[test]
    fn automation_bstr_contracts_follow_metadata_direction_and_ownership() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.Web.InternetExplorer",
            "ITargetNotify2",
        )
        .unwrap();
        let semantic = map_interface(&interface).unwrap();
        let method = semantic
            .methods()
            .iter()
            .find(|method| method.name() == "GetOptionString")
            .unwrap();

        let input_interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.TaskScheduler",
            "IExecAction2",
        )
        .unwrap();
        let input_semantic = map_interface(&input_interface).unwrap();
        let input = input_semantic
            .methods()
            .iter()
            .flat_map(ComMethodContract::params)
            .find(|param| {
                param.direction() == Direction::In
                    && matches!(
                        input_semantic
                            .type_definition(param.abi_type())
                            .unwrap()
                            .abi(),
                        ComAbiType::Bstr
                    )
            })
            .unwrap();
        assert_eq!(input.ownership(), &ComOwnership::Borrowed);
        assert_eq!(input.cleanup(), &Cleanup::None);

        let replacement = method
            .params()
            .iter()
            .find(|param| param.direction() == Direction::InOut)
            .unwrap();
        assert_eq!(replacement.ownership(), &ComOwnership::BstrReplaced);
        assert_eq!(replacement.cleanup(), &Cleanup::SysFreeString);
        assert_eq!(replacement.nullability(), Nullability::Required);
    }

    #[test]
    fn documented_optional_bstr_outputs_override_incorrect_in_out_metadata() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        for (namespace, interface_name, method_name, parameter_names) in [
            (
                "Windows.Win32.Media.PictureAcquisition",
                "IPhotoAcquireDeviceSelectionDialog",
                "DoModal",
                &["pbstrDeviceId"][..],
            ),
            (
                "Windows.Win32.Storage.Imapi",
                "IDiscRecorder",
                "GetDisplayNames",
                &["pbstrVendorID", "pbstrProductID", "pbstrRevision"][..],
            ),
        ] {
            let interface =
                crate::com_metadata::parse_com_interface(&winmd, namespace, interface_name)
                    .unwrap();
            let semantic = map_interface(&interface).unwrap_or_else(|error| {
                panic!("{namespace}.{interface_name} failed to map: {error}")
            });
            let method = semantic
                .methods()
                .iter()
                .find(|method| method.name() == method_name)
                .unwrap();
            for parameter_name in parameter_names {
                let parameter = method
                    .params()
                    .iter()
                    .find(|parameter| parameter.name() == *parameter_name)
                    .unwrap();
                assert_eq!(parameter.direction(), Direction::Out);
                assert_eq!(parameter.ownership(), &ComOwnership::BstrOwned);
                assert_eq!(parameter.cleanup(), &Cleanup::SysFreeString);
            }
        }
    }

    #[test]
    fn inherited_bstr_replacement_contracts_are_preserved() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.TaskScheduler",
            "IExecAction2",
        )
        .unwrap();
        let inherited = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| {
                method.vtable_index < interface.own_methods_start
                    && method.params.iter().any(|parameter| {
                        parameter.direction == RawParamDirection::InOut
                            && parameter.typ.pointer_depth == 1
                            && is_raw_named_type(&parameter.typ, "Windows.Win32.Foundation", "BSTR")
                    })
            })
            .unwrap();
        let semantic = map_interface(&interface).unwrap();
        let method = semantic
            .methods()
            .iter()
            .find(|method| method.vtable_slot() == inherited.vtable_index)
            .unwrap();
        assert!(method.params().iter().any(|parameter| {
            parameter.ownership() == &ComOwnership::BstrReplaced
                && parameter.cleanup() == &Cleanup::SysFreeString
        }));
    }

    #[test]
    fn bstr_arrays_map_while_double_pointers_and_unknown_cleanup_fail_closed() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.ApplicationInstallationAndServicing",
            "IPMExtensionFileSavePickerInfo",
        )
        .unwrap();
        let error = map_interface(&interface).unwrap_err().to_string();
        assert!(error.contains("unknown ownership"), "{error}");

        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.RemoteDesktop",
            "ITSGPolicyEngine",
        )
        .unwrap();
        let semantic = map_interface(&interface).unwrap();
        let resource_names = semantic
            .methods()
            .iter()
            .find(|method| method.name() == "AuthorizeResource")
            .unwrap()
            .params()
            .iter()
            .find(|parameter| parameter.name() == "resourceNames")
            .unwrap();
        assert!(matches!(
            semantic
                .type_definition(resource_names.abi_type())
                .unwrap()
                .abi(),
            ComAbiType::CountedBuffer {
                element_ownership: BufferElementOwnership::BstrOwned,
                ..
            }
        ));

        let mut interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.Web.InternetExplorer",
            "ITargetNotify2",
        )
        .unwrap();
        let parameter = interface
            .raw_methods
            .as_mut()
            .unwrap()
            .iter_mut()
            .flat_map(|method| &mut method.params)
            .find(|parameter| {
                parameter.direction == RawParamDirection::InOut
                    && is_raw_named_type(&parameter.typ, "Windows.Win32.Foundation", "BSTR")
            })
            .unwrap();
        parameter.free_with.as_mut().unwrap().function = "CustomBstrFree".into();
        let error = map_interface(&interface).unwrap_err().to_string();
        assert!(error.contains("transfers allocator ownership"), "{error}");
    }

    #[test]
    fn every_live_coclass_associated_interface_maps() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        for coclass_name in ["TaskbarList", "FileOperation", "FileOpenDialog"] {
            let coclass = crate::com_metadata::parse_com_coclass(
                &winmd,
                "Windows.Win32.UI.Shell",
                coclass_name,
            )
            .unwrap()
            .unwrap();
            for interface in &coclass.associated_interfaces {
                map_interface(interface).unwrap_or_else(|error| {
                    panic!(
                        "{}.{} associated interface {} failed to map: {error}",
                        coclass.namespace, coclass.name, interface.interface.name
                    )
                });
            }
        }
    }

    #[test]
    fn override_evidence_is_exact_and_cited() {
        let Some(winmd) = win32_winmd() else {
            return;
        };
        let interface = crate::com_metadata::parse_com_interface(
            &winmd,
            "Windows.Win32.System.Com",
            "IPersistFile",
        )
        .unwrap();
        let get_cur_file = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| method.metadata_name == "GetCurFile")
            .unwrap();
        let output = get_cur_file.params.last().unwrap();
        let free_with = output.free_with.as_ref().unwrap();

        assert_eq!(free_with.function, "CoTaskMemFree");
        assert!(matches!(
            free_with.evidence,
            crate::com_metadata::RawEvidence::ExactRegistry { ref citation, .. }
                if citation.contains("ipersistfile-getcurfile")
        ));
        assert!(matches!(
            get_cur_file.semantic_hresult,
            Some(crate::com_metadata::RawEvidence::ExactRegistry { ref citation, .. })
                if citation.contains("ipersistfile-getcurfile")
        ));
    }

    #[test]
    fn in_out_count_is_both_capacity_and_actual_length() {
        let count_param = ParamIndex::new(2);
        assert_eq!(
            map_count_relation(
                RawParamDirection::Out,
                RawParamDirection::InOut,
                count_param,
                None,
                CountUnit::Elements,
                false,
                false,
            ),
            CountRelation::CallerCapacity {
                capacity_param: count_param,
                actual_length_param: Some(count_param),
                unit: CountUnit::Elements,
                sizing: BufferSizing::SingleCall,
            }
        );
        assert_eq!(
            map_count_relation(
                RawParamDirection::Out,
                RawParamDirection::In,
                ParamIndex::new(2),
                Some(ParamIndex::new(3)),
                CountUnit::Bytes,
                false,
                true,
            ),
            CountRelation::CallerCapacity {
                capacity_param: ParamIndex::new(2),
                actual_length_param: Some(ParamIndex::new(3)),
                unit: CountUnit::Bytes,
                sizing: BufferSizing::FixedCapacity,
            }
        );
    }

    #[test]
    fn bind_opts_detection_depends_on_qualified_type_identity() {
        let raw = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.System.Com".into(),
                name: "BIND_OPTS".into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: None,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        assert!(raw_contains_named_type(
            &raw,
            "Windows.Win32.System.Com",
            "BIND_OPTS"
        ));
        assert!(!raw_contains_named_type(&raw, "Contoso.Com", "BIND_OPTS"));
    }
}
