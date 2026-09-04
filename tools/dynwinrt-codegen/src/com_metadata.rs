// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use sha2::{Digest, Sha256};
use windows_metadata::{HasAttributes, reader};

pub use crate::contract_registry::ComStandardRule as RawComStandardRule;
pub use crate::contract_registry::ContractKind as RawContractKind;
pub use crate::contract_registry::ExactFamilyId as RawExactFamilyId;
use crate::types::TypeMeta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawConstness {
    Const,
    Mutable,
    Mixed,
    Unspecified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawNamedKind {
    Interface,
    Enum,
    Struct,
    Delegate,
    RuntimeClass,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLayoutKind {
    Sequential,
    Explicit,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPacking {
    Default,
    Explicit(u16),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeField {
    pub name: String,
    pub typ: RawComType,
    pub explicit_offset: Option<usize>,
    pub fixed_count: Option<usize>,
    pub bitfield: bool,
    pub flexible_array: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeLayout {
    /// SupportedArchitectureAttribute mask: x86=1, x64=2, ARM64=4.
    pub architectures: u8,
    pub kind: RawLayoutKind,
    pub packing: RawPacking,
    pub declared_size: Option<usize>,
    pub fields: Vec<RawNativeField>,
    /// Win32 metadata uses explicit layout for C unions. Synthetic metadata
    /// tests may set this to false to exercise non-overlapping explicit PODs.
    pub is_union: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNativeLayoutSet {
    pub recursive: bool,
    pub variants: Vec<RawNativeLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawNativeType {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Char16,
    ISize,
    USize,
    String,
    Object,
    Named {
        namespace: String,
        name: String,
        kind: RawNamedKind,
        iid: Option<String>,
        layout: Option<Box<RawNativeLayoutSet>>,
    },
    Array(Box<RawComType>),
    FixedArray {
        element: Box<RawComType>,
        count: usize,
    },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawComType {
    pub native_type: RawNativeType,
    pub underlying: Option<Box<RawComType>>,
    pub pointer_depth: usize,
    /// Constness shared by every pointer level. Mixed per-level qualifiers
    /// become `Unspecified` and fail semantic validation until modeled
    /// explicitly.
    pub constness: RawConstness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawParamDirection {
    In,
    Out,
    InOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStringEncoding {
    Utf16,
    Ansi,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawElementOwnership {
    Borrowed,
    CoTaskMemOwned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawStringPointerArray {
    pub encoding: RawStringEncoding,
    pub pointer_depth: usize,
    pub constness: RawConstness,
    pub ownership: RawElementOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArrayRelation {
    pub count_param_index: Option<usize>,
    pub actual_length_param_index: Option<usize>,
    pub unit: RawCountUnit,
    pub two_call: bool,
    pub projected_capacity: bool,
    pub constness: Option<RawConstness>,
    pub evidence: Vec<RawEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEnumeratorNext {
    pub capacity_param_index: usize,
    pub values_param_index: usize,
    pub fetched_param_index: usize,
    pub fetched_optional_for_single: bool,
    pub evidence: RawEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCountUnit {
    Elements,
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawEvidence {
    MetadataAttribute(&'static str),
    ComStandard(crate::contract_registry::ComStandardRule),
    ExactRegistry {
        entry_id: String,
        family_id: crate::contract_registry::ExactFamilyId,
        contract_kind: crate::contract_registry::ContractKind,
        reason: String,
        citation: String,
    },
}

impl RawEvidence {
    pub fn exact_registry(
        entry_id: impl Into<String>,
        family_id: crate::contract_registry::ExactFamilyId,
        contract_kind: crate::contract_registry::ContractKind,
        reason: impl Into<String>,
        citation: impl Into<String>,
    ) -> Self {
        Self::ExactRegistry {
            entry_id: entry_id.into(),
            family_id,
            contract_kind,
            reason: reason.into(),
            citation: citation.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFreeWith {
    pub function: String,
    pub evidence: RawEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawComParam {
    pub name: String,
    pub typ: RawComType,
    pub direction: RawParamDirection,
    pub optional: bool,
    pub const_attribute: bool,
    pub native_array: Option<RawArrayRelation>,
    pub string_pointer_array: Option<RawStringPointerArray>,
    pub free_with: Option<RawFreeWith>,
    pub safe_array_evidence: Option<RawSafeArrayEvidence>,
    pub exact_interface_output: Option<RawExactInterfaceOutputContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExactInterfaceOutputContract {
    pub interface_iid: String,
    pub argument_optional: bool,
    pub nullable_on_success: bool,
    pub evidence: RawEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawSafeArrayVartype {
    I4,
    Ui1,
    Ui4,
    R8,
    Bstr,
    Unknown,
    Variant,
}

impl RawSafeArrayVartype {
    pub const fn value(self) -> u16 {
        match self {
            Self::I4 => 3,
            Self::R8 => 5,
            Self::Bstr => 8,
            Self::Variant => 12,
            Self::Unknown => 13,
            Self::Ui1 => 17,
            Self::Ui4 => 19,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawSafeArrayOwnership {
    BorrowedInput,
    OwnedOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSafeArrayEvidence {
    pub declaring_namespace: &'static str,
    pub declaring_interface: &'static str,
    pub declaring_iid: &'static str,
    pub method_name: &'static str,
    pub vtable_index: usize,
    pub parameter_index: usize,
    pub parameter_name: &'static str,
    pub element_vartype: RawSafeArrayVartype,
    pub element_iid: Option<&'static str>,
    pub ownership: RawSafeArrayOwnership,
    pub raw_method_shape: &'static str,
    pub reason: &'static str,
    pub citation: &'static str,
}

impl RawSafeArrayEvidence {
    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_parameter_entry_id(
            self.family_id(),
            self.declaring_namespace,
            self.declaring_interface,
            self.declaring_iid,
            self.method_name,
            self.vtable_index,
            self.parameter_index,
            self.parameter_name,
        )
    }

    pub(crate) const fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        crate::contract_registry::ExactFamilyId::SafeArray
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::Safearray
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawComMethod {
    pub declaring_namespace: String,
    pub declaring_interface: String,
    pub declaring_iid: String,
    pub metadata_name: String,
    pub projected_name: String,
    pub vtable_index: usize,
    pub params: Vec<RawComParam>,
    pub return_type: RawComType,
    pub semantic_hresult: Option<RawEvidence>,
    pub enumerator_next: Option<RawEnumeratorNext>,
    pub exact_contract: Option<RawExactMethodContract>,
    pub interface_replacement_contracts: Vec<RawInterfaceReplacementContract>,
    pub exact_interface_output_call: Option<RawExactInterfaceOutputCallContract>,
    pub safe_array_contract_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExactInterfaceOutputCallContract {
    pub source_fingerprint: String,
    pub public_input_param_indices: Vec<usize>,
    pub flags_param_index: usize,
    pub context_param_index: usize,
    pub synchronous_output_param_index: usize,
    pub semisynchronous_output_param_index: usize,
    pub synchronous_flags: i32,
    pub semisynchronous_flag_value: i32,
    pub flags_option_name: String,
    pub synchronous_output_option_name: String,
    pub semisynchronous_output_option_name: String,
    pub evidence: RawEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawInterfaceReplacementSemantics {
    ConsumesOldReturnsOwnedNew,
    PreservesOldReturnsOwnedNew,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawInterfaceReplacementContract {
    pub parameter_index: usize,
    pub semantics: RawInterfaceReplacementSemantics,
    pub evidence: RawEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawExactMethodContractKind {
    FixedCapacityBytes,
    UnsafePrivateData,
    StatStg,
    Malloc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawExactMethodContract {
    pub kind: RawExactMethodContractKind,
    pub declaring_namespace: &'static str,
    pub declaring_interface: &'static str,
    pub declaring_iid: &'static str,
    pub method_name: &'static str,
    pub vtable_index: usize,
    pub buffer_param_index: usize,
    pub capacity_param_index: usize,
    pub actual_length_param_index: Option<usize>,
    pub citation: &'static str,
    pub reason: &'static str,
}

impl RawExactMethodContract {
    pub(crate) fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        match self.kind {
            RawExactMethodContractKind::FixedCapacityBytes => {
                crate::contract_registry::ExactFamilyId::BoundedTwoCall
            }
            RawExactMethodContractKind::UnsafePrivateData => {
                crate::contract_registry::ExactFamilyId::PrivateDataHazard
            }
            RawExactMethodContractKind::StatStg | RawExactMethodContractKind::Malloc => {
                crate::contract_registry::ExactFamilyId::Ownership
            }
        }
    }

    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_method_entry_id(
            self.family_id(),
            self.declaring_namespace,
            self.declaring_interface,
            self.declaring_iid,
            self.method_name,
            self.vtable_index,
        )
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        match self.kind {
            RawExactMethodContractKind::FixedCapacityBytes => {
                crate::contract_registry::ContractKind::BoundedTwoCall
            }
            RawExactMethodContractKind::UnsafePrivateData => {
                crate::contract_registry::ContractKind::Hazard
            }
            RawExactMethodContractKind::StatStg | RawExactMethodContractKind::Malloc => {
                crate::contract_registry::ContractKind::Ownership
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamDirection {
    In,
    Out,
    InOut,
    OutFill,
    OutStringBuffer { count_param_index: usize },
    UnsupportedNativeArray { count_param_index: Option<usize> },
}

impl ParamDirection {
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::In | Self::InOut | Self::UnsupportedNativeArray { .. }
        )
    }

    pub fn is_output(&self) -> bool {
        matches!(
            self,
            Self::Out
                | Self::InOut
                | Self::OutFill
                | Self::OutStringBuffer { .. }
                | Self::UnsupportedNativeArray { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name: String,
    pub typ: TypeMeta,
    pub direction: ParamDirection,
}

#[derive(Debug, Clone, Default)]
pub struct MethodMeta {
    pub name: String,
    pub vtable_index: usize,
    pub params: Vec<ParamMeta>,
    pub return_type: Option<TypeMeta>,
    pub preserve_hresult: bool,
    pub doc: Option<String>,
    pub owned_outputs: Vec<OwnedOutput>,
}

#[derive(Debug, Clone)]
pub struct OwnedOutput {
    pub param_index: usize,
    pub free_with: String,
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceMeta {
    pub name: String,
    pub namespace: String,
    pub iid: String,
    pub methods: Vec<MethodMeta>,
    pub generic_piid: Option<String>,
    pub generic_args: Vec<TypeMeta>,
    pub doc: Option<String>,
    pub deprecated: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComInterfaceMeta {
    pub interface: InterfaceMeta,
    pub base_offset: usize,
    pub is_iunknown_rooted: bool,
    pub base_chain: Vec<String>,
    pub base_iids: Vec<String>,
    pub coclass_clsid: Option<String>,
    pub coclass_name: Option<String>,
    pub own_methods_start: usize,
    pub referenced_enums: Vec<ComEnumMeta>,
    pub raw_referenced_enums: Option<Vec<ComEnumMeta>>,
    pub raw_methods: Option<Vec<RawComMethod>>,
}

#[derive(Debug, Clone)]
pub struct ComCoclassMeta {
    pub name: String,
    pub namespace: String,
    pub clsid: String,
    pub primary_interface: ComInterfaceMeta,
    pub associated_interfaces: Vec<ComInterfaceMeta>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComEnumValue {
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Clone)]
pub struct ComEnumMember {
    pub name: String,
    pub value: ComEnumValue,
}

#[derive(Debug, Clone)]
pub struct ComEnumMeta {
    pub namespace: String,
    pub name: String,
    pub underlying: TypeMeta,
    pub members: Vec<ComEnumMember>,
    pub is_flags: bool,
}

pub fn parse_com_interface(
    winmd_paths: &str,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let index = crate::meta::load_index(winmd_paths)?;
    parse_com_interface_from_index(&index, namespace, name)
}

pub fn parse_all_com_interfaces(winmd_paths: &str) -> Option<Vec<ComInterfaceMeta>> {
    let index = crate::meta::load_index(winmd_paths)?;
    Some(
        index
            .all()
            .filter_map(|definition| {
                parse_com_interface_from_index(&index, definition.namespace(), definition.name())
            })
            .collect(),
    )
}

pub fn parse_com_coclass(
    winmd_paths: &str,
    namespace: &str,
    name: &str,
) -> Result<Option<ComCoclassMeta>, String> {
    let Some(index) = crate::meta::load_index(winmd_paths) else {
        return Ok(None);
    };
    parse_com_coclass_from_index(&index, namespace, name)
}

pub fn parse_com_enum(winmd_paths: &str, namespace: &str, name: &str) -> Option<ComEnumMeta> {
    let index = crate::meta::load_index(winmd_paths)?;
    let def = index.get(namespace, name).next()?;
    parse_com_enum_def(&def)
}

pub fn first_classic_com_interface_in_namespace(
    winmd_paths: &str,
    namespace: &str,
) -> Option<String> {
    let index = crate::meta::load_index(winmd_paths)?;
    let names = index
        .all()
        .filter(|def| {
            def.namespace() == namespace
                && def
                    .flags()
                    .contains(windows_metadata::TypeAttributes::Interface)
        })
        .map(|def| def.name().to_string())
        .collect::<Vec<_>>();
    names.into_iter().find(|name| {
        parse_com_interface_from_index(&index, namespace, name).is_some_and(|interface| {
            interface.is_iunknown_rooted || interface.interface.name.ends_with("Interop")
        })
    })
}

fn parse_com_interface_from_index(
    index: &reader::Index,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let def = index.get(namespace, name).next()?;
    if !def
        .flags()
        .contains(windows_metadata::TypeAttributes::Interface)
    {
        return None;
    }

    let (is_iunknown_rooted, root_offset, base_chain) =
        resolve_com_base_chain(namespace, name, |current_namespace, current_name| {
            let current_def = index.get(current_namespace, current_name).next()?;
            current_def
                .interface_impls()
                .map(|implementation| match implementation.interface(&[]) {
                    windows_metadata::Type::Name(name) => {
                        let method_count =
                            if matches!(name.name.as_str(), "IUnknown" | "IInspectable") {
                                0
                            } else {
                                index
                                    .get(&name.namespace, &name.name)
                                    .next()?
                                    .methods()
                                    .count()
                            };
                        Some((name.namespace, name.name, method_count))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
        })?;
    let own_methods_start = root_offset
        + base_chain
            .iter()
            .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
            .map(|(_, _, count)| count)
            .sum::<usize>();
    let base_iids = base_chain
        .iter()
        .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
        .filter_map(|(namespace, name, _)| {
            let definition = index.get(namespace, name).next()?;
            let iid = crate::meta::extract_iid(&definition);
            (!iid.is_empty()).then_some(iid)
        })
        .collect::<Vec<_>>();

    let mut methods = Vec::new();
    let mut raw_methods = Vec::new();
    let mut slot = root_offset;
    for (base_namespace, base_name, _) in base_chain
        .iter()
        .rev()
        .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
    {
        let base_def = index.get(base_namespace, base_name).next()?;
        let (mut base_methods, mut base_raw_methods) = parse_methods(index, &base_def, slot);
        slot += base_methods.len();
        methods.append(&mut base_methods);
        raw_methods.append(&mut base_raw_methods);
    }

    if slot != own_methods_start {
        return None;
    }
    let (own_methods, own_raw_methods) = parse_methods(index, &def, slot);
    methods.extend(own_methods);
    raw_methods.extend(own_raw_methods);

    let iid = crate::meta::extract_iid(&def);
    let interface = InterfaceMeta {
        name: name.to_string(),
        namespace: namespace.to_string(),
        iid,
        methods,
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };
    let (coclass_name, coclass_clsid) = find_coclass(index, namespace, name);
    let referenced_enums = collect_referenced_enums(index, &interface);
    let raw_referenced_enums = Some(referenced_enums.clone());

    Some(ComInterfaceMeta {
        interface,
        base_offset: root_offset,
        is_iunknown_rooted,
        base_chain: base_chain.into_iter().map(|(_, name, _)| name).collect(),
        base_iids,
        coclass_clsid,
        coclass_name,
        own_methods_start,
        referenced_enums,
        raw_referenced_enums,
        raw_methods: Some(raw_methods),
    })
}

fn resolve_com_base_chain(
    namespace: &str,
    name: &str,
    mut direct_bases: impl FnMut(&str, &str) -> Option<Vec<(String, String, usize)>>,
) -> Option<(bool, usize, Vec<(String, String, usize)>)> {
    let mut base_chain = Vec::new();
    let mut current = (namespace.to_string(), name.to_string());
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..32 {
        if !visited.insert(current.clone()) {
            return None;
        }
        let bases = direct_bases(&current.0, &current.1)?;
        let [base] = bases.as_slice() else {
            return None;
        };
        match base.1.as_str() {
            "IUnknown" => {
                base_chain.push((
                    "Windows.Win32.System.Com".to_string(),
                    "IUnknown".to_string(),
                    0,
                ));
                return Some((true, 3, base_chain));
            }
            "IInspectable" => {
                base_chain.push((
                    "Windows.Foundation".to_string(),
                    "IInspectable".to_string(),
                    0,
                ));
                return Some((false, 6, base_chain));
            }
            _ => {
                base_chain.push(base.clone());
                current = (base.0.clone(), base.1.clone());
            }
        }
    }
    None
}

fn parse_com_coclass_from_index(
    index: &reader::Index,
    namespace: &str,
    name: &str,
) -> Result<Option<ComCoclassMeta>, String> {
    let Some(def) = index.get(namespace, name).next() else {
        return Ok(None);
    };
    if !is_com_coclass(&def) {
        return Ok(None);
    }
    let clsid = crate::meta::extract_iid(&def);
    if clsid.is_empty() {
        return Err(format!(
            "{namespace}.{name} is coclass-shaped but has no CLSID"
        ));
    }

    // Windows.Win32.winmd represents coclasses as GUID-bearing ValueType
    // definitions without InterfaceImpl rows. Associate interfaces using the
    // metadata naming convention already used for interface activation, then
    // choose a primary from the real interface inheritance graph. Numeric
    // suffixes never determine which interface is most derived.
    let mut associated_interfaces = index
        .all()
        .filter(|candidate| {
            candidate.namespace() == namespace
                && candidate
                    .flags()
                    .contains(windows_metadata::TypeAttributes::Interface)
                && coclass_name_candidates(candidate.name())
                    .iter()
                    .any(|candidate_name| candidate_name == name)
        })
        .filter_map(|candidate| {
            parse_com_interface_from_index(index, namespace, candidate.name())
                .filter(|interface| interface.is_iunknown_rooted)
        })
        .collect::<Vec<_>>();
    associated_interfaces.sort_by(|left, right| {
        left.own_methods_start
            .cmp(&right.own_methods_start)
            .then_with(|| left.interface.name.cmp(&right.interface.name))
    });
    associated_interfaces.dedup_by(|left, right| left.interface.iid == right.interface.iid);

    let primary_interface =
        select_primary_coclass_interface(namespace, name, &associated_interfaces)?;

    Ok(Some(ComCoclassMeta {
        name: name.to_string(),
        namespace: namespace.to_string(),
        clsid,
        primary_interface,
        associated_interfaces,
    }))
}

fn select_primary_coclass_interface(
    namespace: &str,
    coclass_name: &str,
    associated_interfaces: &[ComInterfaceMeta],
) -> Result<ComInterfaceMeta, String> {
    let leaves = associated_interfaces
        .iter()
        .filter(|candidate| {
            !associated_interfaces.iter().any(|other| {
                other.interface.iid != candidate.interface.iid
                    && other.base_chain.contains(&candidate.interface.name)
            })
        })
        .collect::<Vec<_>>();
    match leaves.as_slice() {
        [primary] => Ok((*primary).clone()),
        [] => Err(format!(
            "{namespace}.{coclass_name} has no safely associated IUnknown-rooted interface"
        )),
        multiple => Err(format!(
            "{namespace}.{coclass_name} has multiple unrelated most-derived interfaces ({}); \
                 Windows.Win32 metadata does not identify a default interface",
            multiple
                .iter()
                .map(|interface| interface.interface.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn parse_methods(
    index: &reader::Index,
    def: &reader::TypeDef,
    base_offset: usize,
) -> (Vec<MethodMeta>, Vec<RawComMethod>) {
    def.methods()
        .enumerate()
        .map(|(index_in_interface, method)| {
            let signature = method.signature(&[]);
            let declaring_iid = crate::meta::extract_iid(def);
            let absolute_slot = base_offset + index_in_interface;
            let raw_name = method.name().to_string();
            let name = method
                .find_attribute("OverloadAttribute")
                .and_then(|attribute| {
                    attribute
                        .value()
                        .into_iter()
                        .next()
                        .and_then(|(_, value)| match value {
                            windows_metadata::Value::Utf8(value) => Some(value),
                            _ => None,
                        })
                })
                .unwrap_or(raw_name);
            let mut params = Vec::new();
            let mut raw_params = Vec::new();
            let mut owned_outputs = Vec::new();
            for (param_index, (param, typ)) in method
                .params()
                .filter(|param| param.sequence() > 0)
                .zip(signature.types.iter())
                .enumerate()
            {
                let mut direction = classify_direction(
                    param.flags(),
                    matches!(typ, windows_metadata::Type::Array(_)),
                );
                let raw_direction = raw_param_direction(param.flags());
                let metadata_array =
                    native_array_count_param(&param).map(|count_param_index| RawArrayRelation {
                        count_param_index,
                        actual_length_param_index: None,
                        unit: RawCountUnit::Elements,
                        two_call: false,
                        projected_capacity: false,
                        constness: None,
                        evidence: vec![RawEvidence::MetadataAttribute("NativeArrayInfoAttribute")],
                    });
                let native_array = known_array_contract_override(
                    def.namespace(),
                    def.name(),
                    &declaring_iid,
                    method.name(),
                    absolute_slot,
                    param_index,
                    param.name(),
                    metadata_array,
                );
                let mapped_type = map_parameter_type(typ, &direction, index);
                let raw_type = map_raw_com_type(typ, index);
                // Use NativeArrayInfo or an exact documented registry contract as the
                // authoritative source for buffer/count relationships. A `[out]`
                // `PWSTR`/`PSTR` buffer with an authoritative count becomes a caller-owned
                // string buffer; any other `[out]`/`[out, in]` array-shaped output with a
                // count relationship is an unsupported native buffer.
                if let Some(count_param_index) = native_array
                    .as_ref()
                    .map(|relation| relation.count_param_index)
                {
                    if direction == ParamDirection::Out && is_string_buffer(&mapped_type) {
                        if let Some(count_param_index) = count_param_index {
                            direction = ParamDirection::OutStringBuffer { count_param_index };
                        }
                    } else if direction.is_output() {
                        direction = ParamDirection::UnsupportedNativeArray { count_param_index };
                    }
                }
                let free_with =
                    param
                        .find_attribute("FreeWithAttribute")
                        .and_then(|attribute| {
                            attribute.value().into_iter().next().and_then(
                                |(_, value)| match value {
                                    windows_metadata::Value::Utf8(value) => Some(value),
                                    _ => None,
                                },
                            )
                        })
                        .map(|function| RawFreeWith {
                            function,
                            evidence: RawEvidence::MetadataAttribute("FreeWithAttribute"),
                        })
                        .or_else(|| {
                            known_free_with_override(
                                def.namespace(),
                                def.name(),
                                &declaring_iid,
                                method.name(),
                                absolute_slot,
                                param_index,
                                param.name(),
                                typ,
                                &direction,
                            )
                        });
                let mut string_pointer_array = native_array.as_ref().and_then(|_| {
                    classify_raw_string_pointer_array(&raw_type, raw_direction, free_with.as_ref())
                });
                if def.namespace() == "Windows.Win32.System.Com"
                    && def.name() == "IEnumString"
                    && crate::meta::extract_iid(def)
                        .eq_ignore_ascii_case("00000101-0000-0000-c000-000000000046")
                    && method.name() == "Next"
                    && base_offset + index_in_interface == 3
                    && param_index == 1
                    && let Some(string) = &mut string_pointer_array
                {
                    string.ownership = RawElementOwnership::CoTaskMemOwned;
                }
                if let Some(free_with) = &free_with {
                    owned_outputs.push(OwnedOutput {
                        param_index,
                        free_with: free_with.function.clone(),
                    });
                }
                raw_params.push(RawComParam {
                    name: param.name().to_string(),
                    typ: raw_type,
                    direction: raw_direction,
                    optional: param
                        .flags()
                        .contains(windows_metadata::ParamAttributes::Optional),
                    const_attribute: param.has_attribute("ConstAttribute"),
                    native_array,
                    string_pointer_array,
                    free_with,
                    safe_array_evidence: None,
                    exact_interface_output: None,
                });
                params.push(ParamMeta {
                    name: param.name().to_string(),
                    typ: mapped_type,
                    direction,
                });
            }
            let return_type = (signature.return_type != windows_metadata::Type::Void)
                .then(|| map_return_type(&signature.return_type, index));
            let mut semantic_hresult =
                if method.has_attribute("CanReturnMultipleSuccessValuesAttribute") {
                    Some(RawEvidence::MetadataAttribute(
                        "CanReturnMultipleSuccessValuesAttribute",
                    ))
                } else {
                    known_semantic_hresult_override(
                        def.namespace(),
                        def.name(),
                        &declaring_iid,
                        method.name(),
                        absolute_slot,
                    )
                };
            let enumerator_next = known_enumerator_next_override(
                def.namespace(),
                def.name(),
                &declaring_iid,
                method.name(),
                absolute_slot,
                &raw_params,
                &map_raw_com_type(&signature.return_type, index),
            );
            if semantic_hresult.is_none()
                && let Some(enumerator) = &enumerator_next
            {
                semantic_hresult = Some(enumerator.evidence.clone());
            }
            let preserve_hresult = semantic_hresult.is_some();
            // win32metadata does not embed textual doc comments (no sibling
            // `.xml` file like WinRT's `xml_doc.rs`), but it does attach a
            // `DocumentationAttribute` with a `learn.microsoft.com` reference
            // URL to most methods. Surface that as the method's `doc`, so
            // generated code can at least link back to the canonical docs.
            let doc = method
                .find_attribute("DocumentationAttribute")
                .and_then(|attribute| {
                    attribute
                        .value()
                        .into_iter()
                        .next()
                        .and_then(|(_, value)| match value {
                            windows_metadata::Value::Utf8(value) => Some(value),
                            _ => None,
                        })
                });
            let mut compatibility = MethodMeta {
                name: name.clone(),
                vtable_index: absolute_slot,
                params,
                return_type,
                preserve_hresult,
                doc,
                owned_outputs,
            };
            let mut raw = RawComMethod {
                declaring_namespace: def.namespace().to_string(),
                declaring_interface: def.name().to_string(),
                declaring_iid,
                metadata_name: method.name().to_string(),
                projected_name: name,
                vtable_index: absolute_slot,
                params: raw_params,
                return_type: map_raw_com_type(&signature.return_type, index),
                semantic_hresult,
                enumerator_next,
                exact_contract: None,
                interface_replacement_contracts: Vec::new(),
                exact_interface_output_call: None,
                safe_array_contract_error: None,
            };
            apply_exact_method_contract(
                def.namespace(),
                def.name(),
                &crate::meta::extract_iid(def),
                &mut compatibility,
                &mut raw,
            );
            apply_safe_array_evidence(&mut raw);
            apply_exact_parameter_direction_overrides(&mut compatibility, &mut raw);
            (compatibility, raw)
        })
        .unzip()
}

pub(crate) fn canonical_raw_method(method: &RawComMethod) -> String {
    let params = method
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            format!(
                "{index}:name={}:type={:?}:direction={:?}:optional={}:const_attribute={}:array={:?}:string_array={:?}:free={:?}:safe_array={:?}:exact_interface_output={:?}",
                param.name,
                param.typ,
                param.direction,
                param.optional,
                param.const_attribute,
                param.native_array,
                param.string_pointer_array,
                param.free_with,
                param.safe_array_evidence,
                param.exact_interface_output,
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "namespace={};interface={};iid={};method={};projected={};slot={};params=[{}];return={:?};semantic_hresult={:?};enumerator={:?};exact_contract={:?};interface_replacements={:?};exact_interface_output_call={:?};safe_array_error={:?}",
        method.declaring_namespace,
        method.declaring_interface,
        method.declaring_iid,
        method.metadata_name,
        method.projected_name,
        method.vtable_index,
        params,
        method.return_type,
        method.semantic_hresult,
        method.enumerator_next,
        method.exact_contract,
        method.interface_replacement_contracts,
        method.exact_interface_output_call,
        method.safe_array_contract_error,
    )
}

pub(crate) fn raw_method_fingerprint(method: &RawComMethod) -> String {
    format!(
        "{:X}",
        Sha256::digest(canonical_raw_method(method).as_bytes())
    )
}

pub(crate) fn collect_evidence_dependencies(
    interface: &ComInterfaceMeta,
) -> crate::contract_registry::EvidenceDependencies {
    use crate::contract_registry::{ComStandardRule, EvidenceDependencies};

    let mut dependencies = EvidenceDependencies::default();
    dependencies.add_standard(ComStandardRule::IUnknownIdentityRefcount);
    dependencies.add_standard(ComStandardRule::QueryInterfaceOutputPlusOne);
    if interface.coclass_clsid.is_some() {
        dependencies.add_standard(ComStandardRule::ActivationOutputPlusOne);
    }
    for method in interface.raw_methods.as_deref().unwrap_or_default() {
        if raw_hresult(&method.return_type) {
            dependencies.add_standard(ComStandardRule::HresultFailure);
        }
        if let Some(evidence) = &method.semantic_hresult {
            dependencies.consume_raw_evidence(evidence);
        }
        if let Some(enumerator) = &method.enumerator_next {
            dependencies.consume_raw_evidence(&enumerator.evidence);
        }
        if let Some(contract) = &method.exact_contract {
            dependencies.add_exact(
                contract.entry_id(),
                contract.family_id(),
                contract.contract_kind(),
            );
        }
        if let Some(call) = &method.exact_interface_output_call {
            dependencies.consume_raw_evidence(&call.evidence);
        }
        for replacement in &method.interface_replacement_contracts {
            dependencies.consume_raw_evidence(&replacement.evidence);
        }
        if let Some(borrowed) =
            crate::com_borrowed_handle_registry::borrowed_hwnd_evidence_for_declaration(
                &method.declaring_namespace,
                &method.declaring_interface,
                &method.metadata_name,
                method.vtable_index,
            )
        {
            dependencies.add_exact(
                borrowed.entry_id(),
                borrowed.family_id(),
                borrowed.contract_kind(),
            );
        }
        for (parameter_index, parameter) in method.params.iter().enumerate() {
            if parameter.const_attribute {
                dependencies
                    .metadata_attributes
                    .insert("ConstAttribute".into());
            }
            for evidence in parameter
                .native_array
                .iter()
                .flat_map(|array| &array.evidence)
            {
                dependencies.consume_raw_evidence(evidence);
            }
            if let Some(free_with) = &parameter.free_with {
                dependencies.consume_raw_evidence(&free_with.evidence);
                dependencies.add_standard(ComStandardRule::MatchingStandardCleanup);
            }
            if let Some(safe_array) = &parameter.safe_array_evidence {
                dependencies.add_exact(
                    safe_array.entry_id(),
                    safe_array.family_id(),
                    safe_array.contract_kind(),
                );
            }
            if let Some(output) = &parameter.exact_interface_output {
                dependencies.consume_raw_evidence(&output.evidence);
            }
            if matches!(
                &parameter.typ.native_type,
                RawNativeType::Named {
                    kind: RawNamedKind::Interface | RawNamedKind::RuntimeClass,
                    ..
                }
            ) {
                match parameter.direction {
                    RawParamDirection::In => {
                        dependencies.add_standard(ComStandardRule::InterfaceInputBorrow);
                    }
                    RawParamDirection::Out => {
                        dependencies.add_standard(ComStandardRule::TypedInterfaceOutputPlusOne);
                    }
                    RawParamDirection::InOut => {}
                }
            }
            if is_registered_borrowed_hwnd_output(method, parameter_index) {
                dependencies.add_standard(ComStandardRule::BorrowedHandleNoCleanup);
            }
        }
    }
    dependencies
}

pub(crate) fn collect_exact_registry_entries(
    interface: &ComInterfaceMeta,
) -> Vec<crate::contract_registry::ExactRegistryEntry> {
    use crate::contract_registry::{ExactEntrySelector, ExactRegistryEntry};

    let mut entries = Vec::new();
    for method in interface.raw_methods.as_deref().unwrap_or_default() {
        let method_fingerprint = method
            .exact_interface_output_call
            .as_ref()
            .map(|contract| contract.source_fingerprint.clone())
            .unwrap_or_else(|| raw_method_fingerprint(method));
        let method_selector = || ExactEntrySelector {
            namespace: method.declaring_namespace.clone(),
            interface: method.declaring_interface.clone(),
            iid: method.declaring_iid.clone(),
            method: method.metadata_name.clone(),
            slot: method.vtable_index,
            parameter: None,
        };
        let record = |evidence: &RawEvidence, parameter: Option<(usize, &RawComParam)>| {
            let RawEvidence::ExactRegistry {
                entry_id,
                family_id,
                contract_kind,
                reason,
                citation,
            } = evidence
            else {
                return None;
            };
            let mut selector = method_selector();
            if entry_id.contains(".param-")
                && let Some((index, parameter)) = parameter
            {
                selector.parameter = Some((index, parameter.name.clone()));
            }
            Some(ExactRegistryEntry {
                entry_id: entry_id.clone(),
                family_id: *family_id,
                contract_kind: *contract_kind,
                selector,
                source_fingerprint: method_fingerprint.clone(),
                reason: reason.clone(),
                citation: citation.clone(),
            })
        };

        if let Some(evidence) = &method.semantic_hresult {
            entries.extend(record(evidence, None));
        }
        if let Some(enumerator) = &method.enumerator_next {
            entries.extend(record(&enumerator.evidence, None));
        }
        if let Some(contract) = &method.exact_contract {
            entries.push(ExactRegistryEntry {
                entry_id: contract.entry_id(),
                family_id: contract.family_id(),
                contract_kind: contract.contract_kind(),
                selector: method_selector(),
                source_fingerprint: method_fingerprint.clone(),
                reason: contract.reason.into(),
                citation: contract.citation.into(),
            });
        }
        if let Some(call) = &method.exact_interface_output_call {
            entries.extend(record(&call.evidence, None));
        }
        for replacement in &method.interface_replacement_contracts {
            entries.extend(record(
                &replacement.evidence,
                method
                    .params
                    .get(replacement.parameter_index)
                    .map(|parameter| (replacement.parameter_index, parameter)),
            ));
        }
        if let Some(borrowed) =
            crate::com_borrowed_handle_registry::borrowed_hwnd_evidence_for_declaration(
                &method.declaring_namespace,
                &method.declaring_interface,
                &method.metadata_name,
                method.vtable_index,
            )
        {
            entries.push(ExactRegistryEntry {
                entry_id: borrowed.entry_id(),
                family_id: borrowed.family_id(),
                contract_kind: borrowed.contract_kind(),
                selector: ExactEntrySelector {
                    parameter: Some((borrowed.parameter_index, borrowed.parameter_name.into())),
                    ..method_selector()
                },
                source_fingerprint: method_fingerprint.clone(),
                reason: borrowed.reason.into(),
                citation: borrowed.citation.into(),
            });
        }
        for (parameter_index, parameter) in method.params.iter().enumerate() {
            for evidence in parameter
                .native_array
                .iter()
                .flat_map(|array| &array.evidence)
            {
                entries.extend(record(evidence, Some((parameter_index, parameter))));
            }
            if let Some(free_with) = &parameter.free_with {
                entries.extend(record(
                    &free_with.evidence,
                    Some((parameter_index, parameter)),
                ));
            }
            if let Some(safe_array) = &parameter.safe_array_evidence {
                entries.push(ExactRegistryEntry {
                    entry_id: safe_array.entry_id(),
                    family_id: safe_array.family_id(),
                    contract_kind: safe_array.contract_kind(),
                    selector: ExactEntrySelector {
                        parameter: Some((
                            safe_array.parameter_index,
                            safe_array.parameter_name.into(),
                        )),
                        ..method_selector()
                    },
                    source_fingerprint: method_fingerprint.clone(),
                    reason: safe_array.reason.into(),
                    citation: safe_array.citation.into(),
                });
            }
            if let Some(output) = &parameter.exact_interface_output {
                entries.extend(record(&output.evidence, Some((parameter_index, parameter))));
            }
        }
    }
    entries
}

fn apply_exact_parameter_direction_overrides(
    compatibility: &mut MethodMeta,
    raw: &mut RawComMethod,
) -> bool {
    let entry = crate::contract_registry::conditional_output_contract(
        crate::contract_registry::WMI_OPEN_NAMESPACE_ENTRY_ID,
    )
    .expect("embedded WMI contract registry must validate");
    let selector = &entry.selector;
    if raw.declaring_namespace != selector.interface.namespace
        || raw.declaring_interface != selector.interface.name
        || !raw
            .declaring_iid
            .eq_ignore_ascii_case(&selector.interface.iid)
        || !raw
            .declaring_iid
            .eq_ignore_ascii_case(&selector.declaring_iid)
        || raw.metadata_name != selector.method
        || raw.vtable_index != selector.absolute_slot
        || raw.params.len() != selector.parameter_count
        || compatibility.params.len() != selector.parameter_count
    {
        return false;
    }
    if raw_method_fingerprint(raw) != selector.source_fingerprint {
        return false;
    }
    if !selector
        .parameters
        .iter()
        .zip(&raw.params)
        .all(|(expected, actual)| {
            expected.name == actual.name
                && expected.native_type == raw_type_registry_name(&actual.typ)
                && expected.pointer_depth == actual.typ.pointer_depth
                && expected.direction == raw_direction_key(actual.direction)
                && expected.optional == actual.optional
                && expected.constness == raw_constness_key(actual.typ.constness)
                && expected.const_attribute == actual.const_attribute
        })
    {
        return false;
    }
    let citation = entry
        .evidence
        .iter()
        .filter_map(|source| {
            source
                .url
                .as_deref()
                .or(source.file.as_deref())
                .map(|value| format!("{}:{value}", source.kind.key()))
        })
        .collect::<Vec<_>>()
        .join("; ");
    for output in &entry.contract.outputs {
        let index = output.parameter_index;
        raw.params[index].direction = RawParamDirection::Out;
        raw.params[index].exact_interface_output = Some(RawExactInterfaceOutputContract {
            interface_iid: output.interface_iid.clone(),
            argument_optional: output.argument_optional,
            nullable_on_success: output.nullable_on_success,
            evidence: RawEvidence::exact_registry(
                entry.entry_id.clone(),
                entry.family_id,
                entry.kind,
                entry.reason.clone(),
                citation.clone(),
            ),
        });
        compatibility.params[index].direction = ParamDirection::Out;
    }
    raw.exact_interface_output_call = Some(RawExactInterfaceOutputCallContract {
        source_fingerprint: selector.source_fingerprint.clone(),
        public_input_param_indices: entry.contract.public_input_parameter_indices.clone(),
        flags_param_index: entry.contract.flags_parameter_index,
        context_param_index: entry.contract.context_parameter_index,
        synchronous_output_param_index: entry.contract.synchronous.output_parameter_index,
        semisynchronous_output_param_index: entry.contract.semisynchronous.output_parameter_index,
        synchronous_flags: entry.contract.synchronous.flags,
        semisynchronous_flag_value: entry.contract.semisynchronous.flags,
        flags_option_name: entry.contract.flags_option_name.clone(),
        synchronous_output_option_name: entry.contract.synchronous.option_name.clone(),
        semisynchronous_output_option_name: entry.contract.semisynchronous.option_name.clone(),
        evidence: RawEvidence::exact_registry(
            entry.entry_id.clone(),
            entry.family_id,
            entry.kind,
            entry.reason.clone(),
            citation,
        ),
    });
    true
}

fn raw_type_registry_name(typ: &RawComType) -> String {
    match &typ.native_type {
        RawNativeType::Named {
            namespace, name, ..
        } => format!("{namespace}.{name}"),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

const fn raw_direction_key(direction: RawParamDirection) -> &'static str {
    match direction {
        RawParamDirection::In => "in",
        RawParamDirection::Out => "out",
        RawParamDirection::InOut => "inout",
    }
}

const fn raw_constness_key(constness: RawConstness) -> &'static str {
    match constness {
        RawConstness::Const => "const",
        RawConstness::Mutable => "mutable",
        RawConstness::Mixed => "mixed",
        RawConstness::Unspecified => "unspecified",
    }
}

fn apply_safe_array_evidence(raw: &mut RawComMethod) {
    for parameter_index in 0..raw.params.len() {
        let declaration_evidence =
            crate::com_safe_array_registry::safe_array_evidence_for_declaration(
                &raw.declaring_namespace,
                &raw.declaring_interface,
                &raw.metadata_name,
                parameter_index,
            );
        let is_safe_array = matches!(
            &raw.params[parameter_index].typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY"
        );
        if !is_safe_array {
            if declaration_evidence.is_some() {
                raw.safe_array_contract_error = Some(format!(
                    "{}.{} SAFEARRAY evidence signature no longer matches metadata",
                    raw.declaring_interface, raw.metadata_name
                ));
            }
            continue;
        }
        let Some(evidence) = crate::com_safe_array_registry::registered_safe_array_evidence(
            &raw.declaring_namespace,
            &raw.declaring_interface,
            &raw.declaring_iid,
            &raw.metadata_name,
            raw.vtable_index,
            parameter_index,
        ) else {
            if declaration_evidence.is_some() {
                raw.safe_array_contract_error = Some(format!(
                    "{}.{} SAFEARRAY evidence identity no longer matches metadata",
                    raw.declaring_interface, raw.metadata_name
                ));
            }
            continue;
        };
        if let Err(error) = validate_safe_array_evidence(raw, &evidence) {
            raw.safe_array_contract_error = Some(error);
            continue;
        }
        raw.params[parameter_index].safe_array_evidence = Some(evidence);
    }
}

fn validate_safe_array_evidence(
    raw: &RawComMethod,
    evidence: &RawSafeArrayEvidence,
) -> Result<(), String> {
    if raw.declaring_namespace != evidence.declaring_namespace
        || raw.declaring_interface != evidence.declaring_interface
        || !raw
            .declaring_iid
            .eq_ignore_ascii_case(evidence.declaring_iid)
        || raw.metadata_name != evidence.method_name
        || raw.projected_name != evidence.method_name
        || raw.vtable_index != evidence.vtable_index
        || evidence.parameter_index >= raw.params.len()
    {
        return Err(format!(
            "{}.{} SAFEARRAY evidence identity no longer matches metadata",
            evidence.declaring_interface, evidence.method_name
        ));
    }
    let parameter = &raw.params[evidence.parameter_index];
    if parameter.name != evidence.parameter_name
        || !matches!(
            &parameter.typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.System.Com" && name == "SAFEARRAY"
        )
        || parameter.optional
        || parameter.native_array.is_some()
        || parameter.string_pointer_array.is_some()
        || parameter.free_with.is_some()
        || match evidence.ownership {
            RawSafeArrayOwnership::BorrowedInput => {
                parameter.direction != RawParamDirection::In || parameter.typ.pointer_depth != 1
            }
            RawSafeArrayOwnership::OwnedOutput => {
                parameter.direction != RawParamDirection::Out || parameter.typ.pointer_depth != 2
            }
        }
        || parameter.typ.constness != RawConstness::Mutable
        || evidence.element_iid.is_some()
            != (evidence.element_vartype == RawSafeArrayVartype::Unknown)
        || raw_method_shape(raw) != evidence.raw_method_shape
    {
        return Err(format!(
            "{}.{} SAFEARRAY signature no longer matches exact documented evidence",
            evidence.declaring_interface, evidence.method_name
        ));
    }
    Ok(())
}

pub(crate) fn validate_attached_safe_array_evidence(raw: &RawComMethod) -> Result<(), String> {
    if let Some(error) = &raw.safe_array_contract_error {
        return Err(error.clone());
    }
    for evidence in raw
        .params
        .iter()
        .filter_map(|parameter| parameter.safe_array_evidence.as_ref())
    {
        let registered = crate::com_safe_array_registry::registered_safe_array_evidence(
            &raw.declaring_namespace,
            &raw.declaring_interface,
            &raw.declaring_iid,
            &raw.metadata_name,
            raw.vtable_index,
            evidence.parameter_index,
        )
        .ok_or_else(|| {
            format!(
                "{}.{} SAFEARRAY evidence is no longer registered",
                evidence.declaring_interface, evidence.method_name
            )
        })?;
        if registered != *evidence {
            return Err(format!(
                "{}.{} SAFEARRAY evidence no longer matches the registry",
                evidence.declaring_interface, evidence.method_name
            ));
        }
        validate_safe_array_evidence(raw, evidence)?;
    }
    Ok(())
}

pub(crate) fn validate_borrowed_hwnd_output_evidence(raw: &RawComMethod) -> Result<(), String> {
    let Some(evidence) =
        crate::com_borrowed_handle_registry::borrowed_hwnd_evidence_for_declaration(
            &raw.declaring_namespace,
            &raw.declaring_interface,
            &raw.metadata_name,
            raw.vtable_index,
        )
    else {
        return Ok(());
    };
    let registered = crate::com_borrowed_handle_registry::registered_borrowed_hwnd_output(
        &raw.declaring_namespace,
        &raw.declaring_interface,
        &raw.declaring_iid,
        &raw.metadata_name,
        raw.vtable_index,
        evidence.parameter_index,
    )
    .ok_or_else(|| {
        format!(
            "{}.{} borrowed HWND evidence identity no longer matches metadata",
            evidence.declaring_interface, evidence.method_name
        )
    })?;
    if registered != evidence
        || raw.params.len() != evidence.parameter_count
        || evidence.parameter_index >= raw.params.len()
    {
        return Err(format!(
            "{}.{} borrowed HWND evidence no longer matches the registry",
            evidence.declaring_interface, evidence.method_name
        ));
    }
    let parameter = &raw.params[evidence.parameter_index];
    if parameter.name != evidence.parameter_name
        || parameter.direction != RawParamDirection::Out
        || parameter.optional != evidence.optional
        || parameter.const_attribute
        || parameter.typ.pointer_depth != 1
        || parameter.typ.constness != RawConstness::Mutable
        || !matches!(
            &parameter.typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.Foundation" && name == "HWND"
        )
        || parameter.native_array.is_some()
        || parameter.string_pointer_array.is_some()
        || parameter.free_with.is_some()
        || parameter.safe_array_evidence.is_some()
        || !raw_hresult(&raw.return_type)
        || raw.semantic_hresult.is_some()
        || raw.enumerator_next.is_some()
        || raw.exact_contract.is_some()
        || raw.safe_array_contract_error.is_some()
    {
        return Err(format!(
            "{}.{} signature no longer matches exact documented borrowed HWND evidence",
            evidence.declaring_interface, evidence.method_name
        ));
    }
    Ok(())
}

pub(crate) fn is_registered_borrowed_hwnd_output(
    raw: &RawComMethod,
    parameter_index: usize,
) -> bool {
    crate::com_borrowed_handle_registry::registered_borrowed_hwnd_output(
        &raw.declaring_namespace,
        &raw.declaring_interface,
        &raw.declaring_iid,
        &raw.metadata_name,
        raw.vtable_index,
        parameter_index,
    )
    .is_some()
}

pub(crate) fn raw_method_shape(raw: &RawComMethod) -> String {
    let mut shape = format!("{}@{}(", raw.metadata_name, raw.vtable_index);
    for (index, parameter) in raw.params.iter().enumerate() {
        if index != 0 {
            shape.push(',');
        }

        shape.push_str(&parameter.name);
        shape.push(':');
        shape.push_str(match parameter.direction {
            RawParamDirection::In => "in",
            RawParamDirection::Out => "out",
            RawParamDirection::InOut => "inout",
        });
        shape.push(':');
        shape.push_str(if parameter.optional {
            "optional"
        } else {
            "required"
        });
        shape.push(':');
        shape.push_str(if parameter.const_attribute {
            "constattr"
        } else {
            "noconstattr"
        });
        shape.push(':');
        push_raw_type_shape(&mut shape, &parameter.typ);
        if let Some(relation) = &parameter.native_array {
            shape.push_str(&format!(
                ":array={:?}/{:?}/{:?}/{}/{}/{:?}",
                relation.count_param_index,
                relation.actual_length_param_index,
                relation.unit,
                relation.two_call,
                relation.projected_capacity,
                relation.constness
            ));
        }
        if let Some(array) = &parameter.string_pointer_array {
            shape.push_str(&format!(
                ":strings={:?}/{}/{:?}/{:?}",
                array.encoding, array.pointer_depth, array.constness, array.ownership
            ));
        }
        if let Some(free_with) = &parameter.free_with {
            shape.push_str(":free=");
            shape.push_str(&free_with.function);
        }
    }
    shape.push_str(")->");
    push_raw_type_shape(&mut shape, &raw.return_type);
    shape.push_str(if raw.semantic_hresult.is_some() {
        ":semantic_hresult"
    } else {
        ":plain_hresult"
    });
    shape.push_str(if raw.enumerator_next.is_some() {
        ":enumerator_next"
    } else {
        ":not_enumerator_next"
    });
    shape
}

fn push_raw_type_shape(shape: &mut String, typ: &RawComType) {
    match &typ.native_type {
        RawNativeType::Void => shape.push_str("void"),
        RawNativeType::Bool => shape.push_str("bool"),
        RawNativeType::I8 => shape.push_str("i8"),
        RawNativeType::U8 => shape.push_str("u8"),
        RawNativeType::I16 => shape.push_str("i16"),
        RawNativeType::U16 => shape.push_str("u16"),
        RawNativeType::I32 => shape.push_str("i32"),
        RawNativeType::U32 => shape.push_str("u32"),
        RawNativeType::I64 => shape.push_str("i64"),
        RawNativeType::U64 => shape.push_str("u64"),
        RawNativeType::F32 => shape.push_str("f32"),
        RawNativeType::F64 => shape.push_str("f64"),
        RawNativeType::Char16 => shape.push_str("char16"),
        RawNativeType::ISize => shape.push_str("isize"),
        RawNativeType::USize => shape.push_str("usize"),
        RawNativeType::String => shape.push_str("string"),
        RawNativeType::Object => shape.push_str("object"),
        RawNativeType::Named {
            namespace,
            name,
            kind,
            iid,
            ..
        } => {
            shape.push_str(namespace);
            shape.push('.');
            shape.push_str(name);
            shape.push_str(&format!("[{kind:?}]"));
            if let Some(iid) = iid {
                shape.push('{');
                shape.push_str(iid);
                shape.push('}');
            }
        }
        RawNativeType::Array(element) => {
            shape.push_str("array<");
            push_raw_type_shape(shape, element);
            shape.push('>');
        }
        RawNativeType::FixedArray { element, count } => {
            shape.push_str(&format!("fixed[{count}]<"));
            push_raw_type_shape(shape, element);
            shape.push('>');
        }
        RawNativeType::Unknown(name) => {
            shape.push_str("unknown<");
            shape.push_str(name);
            shape.push('>');
        }
    }
    shape.push_str(&format!("/ptr{}/{:?}", typ.pointer_depth, typ.constness));
    if let Some(underlying) = &typ.underlying {
        shape.push_str("/underlying=");
        push_raw_type_shape(shape, underlying);
    }
}

fn apply_exact_method_contract(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    compatibility: &mut MethodMeta,
    raw: &mut RawComMethod,
) {
    let Some(contract) =
        registered_exact_method_contract(interface_namespace, interface_name, &raw.metadata_name)
    else {
        return;
    };
    raw.exact_contract = Some(contract.clone());
    if !interface_iid.eq_ignore_ascii_case(contract.declaring_iid)
        || raw.vtable_index != contract.vtable_index
    {
        return;
    }
    match contract.kind {
        RawExactMethodContractKind::FixedCapacityBytes => {
            if !raw_imf_get_blob_metadata_shape(raw) {
                return;
            }
            let relation = raw.params[contract.buffer_param_index]
                .native_array
                .as_mut()
                .expect("validated GetBlob NativeArrayInfo");
            relation.actual_length_param_index = contract.actual_length_param_index;
            relation.unit = RawCountUnit::Bytes;
            relation.projected_capacity = true;
            relation.constness = Some(RawConstness::Mutable);
            relation.evidence.push(RawEvidence::exact_registry(
                contract.entry_id(),
                contract.family_id(),
                crate::contract_registry::ContractKind::BoundedTwoCall,
                contract.reason,
                contract.citation,
            ));
            let actual = contract
                .actual_length_param_index
                .expect("fixed-capacity GetBlob has an actual byte count");
            raw.params[actual].direction = RawParamDirection::Out;
            compatibility.params[actual].direction = ParamDirection::Out;
        }
        RawExactMethodContractKind::UnsafePrivateData => {
            let Some(buffer) = raw.params.get(contract.buffer_param_index) else {
                return;
            };
            if !raw_private_data_shape(raw, buffer.optional) {
                return;
            }
        }
        RawExactMethodContractKind::StatStg => {}
        RawExactMethodContractKind::Malloc => {}
    }
}

fn known_exact_method_contract(
    namespace: &str,
    interface: &str,
    iid: &str,
    method: &str,
    slot: usize,
) -> Option<RawExactMethodContract> {
    registered_exact_method_contract(namespace, interface, method).filter(|contract| {
        iid.eq_ignore_ascii_case(contract.declaring_iid) && slot == contract.vtable_index
    })
}

fn registered_exact_method_contract(
    namespace: &str,
    interface: &str,
    method: &str,
) -> Option<RawExactMethodContract> {
    let (kind, buffer, capacity, actual, reason, citation) = match (namespace, interface, method) {
        ("Windows.Win32.Media.MediaFoundation", "IMFAttributes", "GetBlob") => (
            RawExactMethodContractKind::FixedCapacityBytes,
            1,
            2,
            Some(3),
            "IMFAttributes::GetBlob documents a caller-allocated byte buffer with an input byte capacity and output actual byte count",
            "https://learn.microsoft.com/windows/win32/api/mfobjects/nf-mfobjects-imfattributes-getblob",
        ),
        ("Windows.Win32.System.Com", "IStream", "Stat") => (
            RawExactMethodContractKind::StatStg,
            0,
            1,
            None,
            "IStream::Stat returns an owned STATSTG whose nested name is allocated with CoTaskMem",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-istream-stat",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "Alloc") => (
            RawExactMethodContractKind::Malloc,
            0,
            0,
            None,
            "IMalloc::Alloc returns allocator-owned memory that must be released with the same IMalloc::Free",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-alloc",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "Realloc") => (
            RawExactMethodContractKind::Malloc,
            0,
            1,
            None,
            "IMalloc::Realloc consumes a nullable allocation address and returns memory owned by the same allocator",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-realloc",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "Free") => (
            RawExactMethodContractKind::Malloc,
            0,
            0,
            None,
            "IMalloc::Free accepts only memory allocated by a compatible IMalloc instance",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-free",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "GetSize") => (
            RawExactMethodContractKind::Malloc,
            0,
            0,
            None,
            "IMalloc::GetSize accepts a nullable allocation address owned by a compatible allocator",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-getsize",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "DidAlloc") => (
            RawExactMethodContractKind::Malloc,
            0,
            0,
            None,
            "IMalloc::DidAlloc inspects a nullable allocation address without taking ownership",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-didalloc",
        ),
        ("Windows.Win32.System.Com", "IMalloc", "HeapMinimize") => (
            RawExactMethodContractKind::Malloc,
            0,
            0,
            None,
            "IMalloc::HeapMinimize has no parameters and no return value",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-imalloc-heapminimize",
        ),
        ("Windows.Win32.Graphics.Dxgi", "IDXGIObject", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "IDXGIObject::GetPrivateData may return an AddRef'd interface pointer set by SetPrivateDataInterface, so a generic Buffer result would lose ownership",
            "https://learn.microsoft.com/windows/win32/api/dxgi/nf-dxgi-idxgiobject-getprivatedata",
        ),
        ("Windows.Win32.Graphics.Direct3D10", "ID3D10DeviceChild", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "ID3D10DeviceChild::GetPrivateData may return an AddRef'd interface pointer, and its documentation gives destructive rather than sizing semantics for NULL data",
            "https://learn.microsoft.com/windows/win32/api/d3d10/nf-d3d10-id3d10devicechild-getprivatedata",
        ),
        ("Windows.Win32.Graphics.Direct3D10", "ID3D10Device", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "ID3D10Device::GetPrivateData may return an AddRef'd interface pointer, and its documentation gives destructive rather than sizing semantics for NULL data",
            "https://learn.microsoft.com/windows/win32/api/d3d10/nf-d3d10-id3d10device-getprivatedata",
        ),
        ("Windows.Win32.Graphics.Direct3D11", "ID3D11DeviceChild", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "ID3D11DeviceChild::GetPrivateData may return an AddRef'd interface pointer set by SetPrivateDataInterface, so a generic Buffer result would lose ownership",
            "https://learn.microsoft.com/windows/win32/api/d3d11/nf-d3d11-id3d11devicechild-getprivatedata",
        ),
        ("Windows.Win32.Graphics.Direct3D11", "ID3D11Device", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "ID3D11Device::GetPrivateData may return an AddRef'd interface pointer set by SetPrivateDataInterface, so a generic Buffer result would lose ownership",
            "https://learn.microsoft.com/windows/win32/api/d3d11/nf-d3d11-id3d11device-getprivatedata",
        ),
        ("Windows.Win32.Graphics.Direct3D12", "ID3D12Object", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "ID3D12Object::GetPrivateData may return an AddRef'd interface pointer set by SetPrivateDataInterface, so a generic Buffer result would lose ownership",
            "https://learn.microsoft.com/windows/win32/api/d3d12/nf-d3d12-id3d12object-getprivatedata",
        ),
        ("Windows.Win32.AI.MachineLearning.DirectML", "IDMLObject", "GetPrivateData") => (
            RawExactMethodContractKind::UnsafePrivateData,
            2,
            1,
            Some(1),
            "IDMLObject::GetPrivateData may return an AddRef'd interface pointer set by SetPrivateDataInterface, so a generic Buffer result would lose ownership",
            "https://learn.microsoft.com/windows/win32/api/directml/nf-directml-idmlobject-getprivatedata",
        ),
        _ => return None,
    };
    Some(RawExactMethodContract {
        kind,
        declaring_namespace: match interface {
            "IMFAttributes" => "Windows.Win32.Media.MediaFoundation",
            "IDXGIObject" => "Windows.Win32.Graphics.Dxgi",
            "ID3D10DeviceChild" | "ID3D10Device" => "Windows.Win32.Graphics.Direct3D10",
            "ID3D11DeviceChild" | "ID3D11Device" => "Windows.Win32.Graphics.Direct3D11",
            "ID3D12Object" => "Windows.Win32.Graphics.Direct3D12",
            "IDMLObject" => "Windows.Win32.AI.MachineLearning.DirectML",
            "IStream" => "Windows.Win32.System.Com",
            "IMalloc" => "Windows.Win32.System.Com",
            _ => unreachable!("matched exact method interface"),
        },
        declaring_interface: match interface {
            "IMFAttributes" => "IMFAttributes",
            "IDXGIObject" => "IDXGIObject",
            "ID3D10DeviceChild" => "ID3D10DeviceChild",
            "ID3D10Device" => "ID3D10Device",
            "ID3D11DeviceChild" => "ID3D11DeviceChild",
            "ID3D11Device" => "ID3D11Device",
            "ID3D12Object" => "ID3D12Object",
            "IDMLObject" => "IDMLObject",
            "IStream" => "IStream",
            "IMalloc" => "IMalloc",
            _ => unreachable!("matched exact method interface"),
        },
        declaring_iid: match kind {
            RawExactMethodContractKind::FixedCapacityBytes => {
                "2cd2d921-c447-44a7-a13c-4adabfc247e3"
            }
            RawExactMethodContractKind::UnsafePrivateData => match interface {
                "IDXGIObject" => "aec22fb8-76f3-4639-9be0-28eb43a67a2e",
                "ID3D10DeviceChild" => "9b7e4c00-342c-4106-a19f-4f2704f689f0",
                "ID3D10Device" => "9b7e4c0f-342c-4106-a19f-4f2704f689f0",
                "ID3D11DeviceChild" => "1841e5c8-16b0-489b-bcc8-44cfb0d5deae",
                "ID3D11Device" => "db6f6ddb-ac77-4e88-8253-819df9bbf140",
                "ID3D12Object" => "c4fec28f-7966-4e95-9f94-f431cb56c3b8",
                "IDMLObject" => "c8263aac-9e0c-4a2d-9b8e-007521a3317c",
                _ => unreachable!("matched exact private-data interface"),
            },
            RawExactMethodContractKind::StatStg => "0000000c-0000-0000-c000-000000000046",
            RawExactMethodContractKind::Malloc => "00000002-0000-0000-c000-000000000046",
        },
        method_name: match kind {
            RawExactMethodContractKind::FixedCapacityBytes => "GetBlob",
            RawExactMethodContractKind::UnsafePrivateData => "GetPrivateData",
            RawExactMethodContractKind::StatStg => "Stat",
            RawExactMethodContractKind::Malloc => match method {
                "Alloc" => "Alloc",
                "Realloc" => "Realloc",
                "Free" => "Free",
                "GetSize" => "GetSize",
                "DidAlloc" => "DidAlloc",
                "HeapMinimize" => "HeapMinimize",
                _ => unreachable!("matched exact IMalloc method"),
            },
        },
        vtable_index: match (interface, method) {
            ("IMFAttributes", "GetBlob") => 15,
            ("IDXGIObject", "GetPrivateData") => 5,
            ("ID3D10DeviceChild", "GetPrivateData") => 4,
            ("ID3D10Device", "GetPrivateData") => 66,
            ("ID3D11DeviceChild", "GetPrivateData") => 4,
            ("ID3D11Device", "GetPrivateData") => 34,
            ("ID3D12Object", "GetPrivateData") | ("IDMLObject", "GetPrivateData") => 3,
            ("IStream", "Stat") => 12,
            ("IMalloc", "Alloc") => 3,
            ("IMalloc", "Realloc") => 4,
            ("IMalloc", "Free") => 5,
            ("IMalloc", "GetSize") => 6,
            ("IMalloc", "DidAlloc") => 7,
            ("IMalloc", "HeapMinimize") => 8,
            _ => unreachable!("matched exact method identity"),
        },
        buffer_param_index: buffer,
        capacity_param_index: capacity,
        actual_length_param_index: actual,
        citation,
        reason,
    })
}

fn raw_imf_get_blob_metadata_shape(raw: &RawComMethod) -> bool {
    raw.params.len() == 4
        && raw.metadata_name == "GetBlob"
        && raw.params[0].name == "guidKey"
        && raw_refguid(&raw.params[0])
        && raw.params[1].name == "pBuf"
        && raw_mutable_pointer(
            &raw.params[1],
            RawNativeType::U8,
            RawParamDirection::Out,
            false,
        )
        && raw.params[1].native_array.as_ref().is_some_and(|relation| {
            relation.count_param_index == Some(2)
                && relation.actual_length_param_index.is_none()
                && relation.unit == RawCountUnit::Elements
                && !relation.two_call
                && !relation.projected_capacity
        })
        && raw.params[2].name == "cbBufSize"
        && raw_u32_scalar(&raw.params[2], RawParamDirection::In, false)
        && raw.params[3].name == "pcbBlobSize"
        && raw_mutable_pointer(
            &raw.params[3],
            RawNativeType::U32,
            RawParamDirection::InOut,
            true,
        )
        && raw_hresult(&raw.return_type)
        && raw.semantic_hresult.is_none()
        && raw.enumerator_next.is_none()
}

fn raw_private_data_shape(raw: &RawComMethod, buffer_optional: bool) -> bool {
    raw.params.len() == 3
        && raw.metadata_name == "GetPrivateData"
        && raw_refguid(&raw.params[0])
        && raw_mutable_pointer(
            &raw.params[1],
            RawNativeType::U32,
            RawParamDirection::InOut,
            false,
        )
        && raw_mutable_pointer(
            &raw.params[2],
            RawNativeType::Void,
            RawParamDirection::Out,
            buffer_optional,
        )
        && raw.params.iter().all(|param| param.native_array.is_none())
        && raw_hresult(&raw.return_type)
        && raw.semantic_hresult.is_none()
        && raw.enumerator_next.is_none()
}

fn raw_refguid(param: &RawComParam) -> bool {
    param.direction == RawParamDirection::In
        && !param.optional
        && param.const_attribute
        && param.typ.pointer_depth == 1
        && param.typ.constness == RawConstness::Mutable
        && matches!(
            &param.typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "System" && name == "Guid"
        )
        && param.free_with.is_none()
}

fn raw_mutable_pointer(
    param: &RawComParam,
    native_type: RawNativeType,
    direction: RawParamDirection,
    optional: bool,
) -> bool {
    param.direction == direction
        && param.optional == optional
        && !param.const_attribute
        && param.typ.pointer_depth == 1
        && param.typ.constness == RawConstness::Mutable
        && std::mem::discriminant(&param.typ.native_type) == std::mem::discriminant(&native_type)
        && param.free_with.is_none()
}

fn raw_u32_scalar(param: &RawComParam, direction: RawParamDirection, optional: bool) -> bool {
    param.direction == direction
        && param.optional == optional
        && !param.const_attribute
        && param.typ.pointer_depth == 0
        && matches!(param.typ.native_type, RawNativeType::U32)
        && param.free_with.is_none()
}

fn raw_hresult(typ: &RawComType) -> bool {
    typ.pointer_depth == 0
        && matches!(
            &typ.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.Foundation" && name == "HRESULT"
        )
        && typ
            .underlying
            .as_deref()
            .is_some_and(|underlying| matches!(underlying.native_type, RawNativeType::I32))
}

pub(crate) fn validate_exact_method_contract(
    current_namespace: &str,
    current_interface: &str,
    current_iid: &str,
    raw: &RawComMethod,
    contract: &RawExactMethodContract,
) -> Result<(), String> {
    let expected = known_exact_method_contract(
        contract.declaring_namespace,
        contract.declaring_interface,
        contract.declaring_iid,
        contract.method_name,
        contract.vtable_index,
    )
    .ok_or_else(|| "exact method contract is not registered".to_string())?;
    if &expected != contract {
        return Err("exact method contract evidence does not match the registry".into());
    }
    if raw.declaring_namespace != contract.declaring_namespace
        || raw.declaring_interface != contract.declaring_interface
        || !raw
            .declaring_iid
            .eq_ignore_ascii_case(contract.declaring_iid)
    {
        return Err(format!(
            "{}.{} declaring interface identity no longer matches exact contract evidence",
            contract.declaring_interface, contract.method_name
        ));
    }
    if current_namespace == contract.declaring_namespace
        && current_interface == contract.declaring_interface
        && !current_iid.eq_ignore_ascii_case(contract.declaring_iid)
    {
        return Err(format!(
            "{}.{} IID no longer matches exact contract evidence",
            contract.declaring_namespace, contract.declaring_interface
        ));
    }
    if raw.metadata_name != contract.method_name
        || raw.projected_name != contract.method_name
        || raw.vtable_index != contract.vtable_index
    {
        return Err(format!(
            "{}.{} method identity or slot no longer matches exact contract evidence",
            contract.declaring_interface, contract.method_name
        ));
    }
    let valid_shape = match contract.kind {
        RawExactMethodContractKind::FixedCapacityBytes => {
            raw.params.len() == 4
                && raw.params[0].name == "guidKey"
                && raw_refguid(&raw.params[0])
                && raw.params[1].name == "pBuf"
                && raw_mutable_pointer(
                    &raw.params[1],
                    RawNativeType::U8,
                    RawParamDirection::Out,
                    false,
                )
                && raw.params[1].native_array.as_ref().is_some_and(|relation| {
                    relation.count_param_index == Some(contract.capacity_param_index)
                        && relation.actual_length_param_index == contract.actual_length_param_index
                        && relation.unit == RawCountUnit::Bytes
                        && !relation.two_call
                        && relation.projected_capacity
                        && relation.constness == Some(RawConstness::Mutable)
                        && relation.evidence.iter().any(|evidence| {
                            matches!(
                                evidence,
                                RawEvidence::ExactRegistry {
                                    reason,
                                    citation,
                                    ..
                                } if reason == contract.reason && citation == contract.citation
                            )
                        })
                })
                && raw.params[2].name == "cbBufSize"
                && raw_u32_scalar(&raw.params[2], RawParamDirection::In, false)
                && raw.params[3].name == "pcbBlobSize"
                && raw_mutable_pointer(
                    &raw.params[3],
                    RawNativeType::U32,
                    RawParamDirection::Out,
                    true,
                )
                && raw_hresult(&raw.return_type)
        }
        RawExactMethodContractKind::UnsafePrivateData => {
            let optional = contract.declaring_interface != "IDXGIObject";
            raw_private_data_shape(raw, optional)
        }
        RawExactMethodContractKind::StatStg => {
            raw_method_shape(raw)
                == "Stat@12(pstatstg:out:required:noconstattr:Windows.Win32.System.Com.STATSTG[Struct]/ptr1/Mutable,grfStatFlag:in:required:noconstattr:u32/ptr0/Unspecified)->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:plain_hresult:not_enumerator_next"
        }
        RawExactMethodContractKind::Malloc => {
            let expected = match contract.method_name {
                "Alloc" => {
                    "Alloc@3(cb:in:required:noconstattr:usize/ptr0/Unspecified)->void/ptr1/Mutable:plain_hresult:not_enumerator_next"
                }
                "Realloc" => {
                    "Realloc@4(pv:in:optional:noconstattr:void/ptr1/Mutable,cb:in:required:noconstattr:usize/ptr0/Unspecified)->void/ptr1/Mutable:plain_hresult:not_enumerator_next"
                }
                "Free" => {
                    "Free@5(pv:in:optional:noconstattr:void/ptr1/Mutable)->void/ptr0/Unspecified:plain_hresult:not_enumerator_next"
                }
                "GetSize" => {
                    "GetSize@6(pv:in:optional:noconstattr:void/ptr1/Mutable)->usize/ptr0/Unspecified:plain_hresult:not_enumerator_next"
                }
                "DidAlloc" => {
                    "DidAlloc@7(pv:in:optional:noconstattr:void/ptr1/Mutable)->i32/ptr0/Unspecified:plain_hresult:not_enumerator_next"
                }
                "HeapMinimize" => {
                    "HeapMinimize@8()->void/ptr0/Unspecified:plain_hresult:not_enumerator_next"
                }
                _ => return Err("unknown exact IMalloc method contract".into()),
            };
            raw_method_shape(raw) == expected
        }
    };
    let indices_valid = match contract.kind {
        RawExactMethodContractKind::FixedCapacityBytes
        | RawExactMethodContractKind::UnsafePrivateData
        | RawExactMethodContractKind::StatStg => {
            contract.buffer_param_index < raw.params.len()
                && contract.capacity_param_index < raw.params.len()
                && contract
                    .actual_length_param_index
                    .is_none_or(|index| index < raw.params.len())
        }
        RawExactMethodContractKind::Malloc => true,
    };
    if !valid_shape
        || raw.semantic_hresult.is_some()
        || raw.enumerator_next.is_some()
        || !indices_valid
    {
        return Err(format!(
            "{}.{} signature no longer matches exact contract evidence",
            contract.declaring_interface, contract.method_name
        ));
    }
    Ok(())
}

fn known_array_contract_override(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    method_name: &str,
    vtable_index: usize,
    param_index: usize,
    param_name: &str,
    metadata: Option<RawArrayRelation>,
) -> Option<RawArrayRelation> {
    if metadata.is_none()
        && method_name == "Next"
        && param_index == 1
        && let Some(contract) = crate::com_enumerator_registry::exact_contract(
            interface_namespace,
            interface_name,
            interface_iid,
            vtable_index,
        )
    {
        return Some(RawArrayRelation {
            count_param_index: Some(0),
            actual_length_param_index: None,
            unit: RawCountUnit::Elements,
            two_call: false,
            projected_capacity: false,
            constness: None,
            evidence: vec![enumerator_contract_evidence(
                contract,
                ENUMERATOR_ARRAY_REASON,
            )],
        });
    }
    let exact = match (
        interface_namespace,
        interface_name,
        method_name,
        param_index,
    ) {
        ("Windows.Win32.System.Com", "ISequentialStream", "Read", 0) => Some((
            1,
            Some(2),
            RawCountUnit::Bytes,
            false,
            RawConstness::Mutable,
            "ISequentialStream::Read documents pv as a cb-byte output buffer and pcbRead as the actual byte count",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-isequentialstream-read",
            false,
        )),
        ("Windows.Win32.System.Com", "ISequentialStream", "Write", 0) => Some((
            1,
            Some(2),
            RawCountUnit::Bytes,
            false,
            RawConstness::Const,
            "ISequentialStream::Write documents pv as a cb-byte const input buffer and pcbWritten as the actual byte count",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-isequentialstream-write",
            false,
        )),
        ("Windows.Win32.System.Com", "ITypeInfo", "GetNames", 1) => Some((
            2,
            Some(3),
            RawCountUnit::Elements,
            false,
            RawConstness::Mutable,
            "ITypeInfo::GetNames documents rgBstrNames as cMaxNames caller-owned slots and pcNames as the actual initialized count",
            "https://learn.microsoft.com/windows/win32/api/oaidl/nf-oaidl-itypeinfo-getnames",
            true,
        )),
        ("Windows.Win32.Storage.Imapi", "IDiscRecorder", "Init", 0) => Some((
            1,
            None,
            RawCountUnit::Bytes,
            false,
            RawConstness::Const,
            "IDiscRecorder::Init documents pbyUniqueID as a nulIDSize-byte input buffer",
            "https://learn.microsoft.com/windows/win32/api/imapi/nf-imapi-idiscrecorder-init",
            true,
        )),
        ("Windows.Win32.Storage.Imapi", "IDiscRecorder", "GetRecorderGUID", 0) => Some((
            1,
            Some(2),
            RawCountUnit::Bytes,
            true,
            RawConstness::Mutable,
            "IDiscRecorder::GetRecorderGUID explicitly documents NULL/zero sizing followed by a caller-owned byte buffer",
            "https://learn.microsoft.com/windows/win32/api/imapi/nf-imapi-idiscrecorder-getrecorderguid",
            true,
        )),
        ("Windows.Win32.Storage.Packaging.Opc", "IOpcSignatureCustomObject", "GetXml", 0) => {
            Some((
                1,
                None,
                RawCountUnit::Bytes,
                false,
                RawConstness::Mutable,
                "IOpcSignatureCustomObject::GetXml documents a callee-allocated byte buffer and output byte count",
                "https://learn.microsoft.com/windows/win32/api/msopc/nf-msopc-iopcsignaturecustomobject-getxml",
                true,
            ))
        }
        _ => None,
    };
    let Some((count, actual, unit, two_call, constness, reason, citation, requires_metadata)) =
        exact
    else {
        return metadata;
    };
    if requires_metadata
        && !metadata
            .as_ref()
            .is_some_and(|relation| relation.count_param_index == Some(count))
    {
        return metadata;
    }
    let mut evidence = metadata
        .map(|relation| relation.evidence)
        .unwrap_or_default();
    let (family_id, contract_kind) = if interface_namespace == "Windows.Win32.System.Com"
        && interface_name == "ISequentialStream"
        && matches!(method_name, "Read" | "Write")
    {
        (
            crate::contract_registry::ExactFamilyId::SequentialStreamBuffer,
            crate::contract_registry::ContractKind::CountedBuffer,
        )
    } else if two_call {
        (
            crate::contract_registry::ExactFamilyId::BoundedTwoCall,
            crate::contract_registry::ContractKind::BoundedTwoCall,
        )
    } else {
        (
            crate::contract_registry::ExactFamilyId::CountedBuffer,
            crate::contract_registry::ContractKind::CountedBuffer,
        )
    };
    evidence.push(RawEvidence::exact_registry(
        crate::contract_registry::exact_parameter_entry_id(
            family_id,
            interface_namespace,
            interface_name,
            interface_iid,
            method_name,
            vtable_index,
            param_index,
            param_name,
        ),
        family_id,
        contract_kind,
        reason,
        citation,
    ));
    Some(RawArrayRelation {
        count_param_index: Some(count),
        actual_length_param_index: actual,
        unit,
        two_call,
        projected_capacity: false,
        constness: Some(constness),
        evidence,
    })
}

const ENUMERATOR_ARRAY_REASON: &str =
    "the exact registered IEnum*::Next contract defines rgelt as celt caller-owned element slots";

fn known_enumerator_next_override(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    method_name: &str,
    vtable_index: usize,
    params: &[RawComParam],
    return_type: &RawComType,
) -> Option<RawEnumeratorNext> {
    let expected = crate::com_enumerator_registry::exact_contract(
        interface_namespace,
        interface_name,
        interface_iid,
        vtable_index,
    )?;
    let (expected_fetched_direction, expected_fetched_optional) =
        crate::com_enumerator_registry::fetched_shape(interface_namespace, interface_name);
    let expected_fetched_direction = match expected_fetched_direction {
        crate::com_enumerator_registry::EnumeratorDirection::Out => RawParamDirection::Out,
        crate::com_enumerator_registry::EnumeratorDirection::InOut => RawParamDirection::InOut,
    };
    let expected_values_direction =
        match crate::com_enumerator_registry::values_direction(interface_namespace, interface_name)
        {
            crate::com_enumerator_registry::EnumeratorDirection::Out => RawParamDirection::Out,
            crate::com_enumerator_registry::EnumeratorDirection::InOut => RawParamDirection::InOut,
        };
    let exact_hresult = return_type.pointer_depth == 0
        && matches!(
            &return_type.native_type,
            RawNativeType::Named {
                namespace,
                name,
                ..
            } if namespace == "Windows.Win32.Foundation" && name == "HRESULT"
        )
        && matches!(
            return_type.underlying.as_deref(),
            Some(RawComType {
                native_type: RawNativeType::I32,
                pointer_depth: 0,
                ..
            })
        );
    let exact_count = |param: &RawComParam, direction, pointer_depth| {
        param.direction == direction
            && !param.const_attribute
            && param.typ.pointer_depth == pointer_depth
            && matches!(param.typ.native_type, RawNativeType::U32)
            && param.typ.underlying.is_none()
    };
    let exact_element = params.get(1).is_some_and(|param| {
        let exact_known_type = match &param.typ.native_type {
            RawNativeType::Named {
                namespace,
                name,
                kind,
                iid,
                ..
            } => {
                namespace == expected.element_namespace
                    && name == expected.element_name
                    && matches!(
                        (expected.element_kind, kind),
                        (
                            crate::com_enumerator_registry::EnumeratorElementKind::Interface,
                            RawNamedKind::Interface
                        ) | (
                            crate::com_enumerator_registry::EnumeratorElementKind::Struct,
                            RawNamedKind::Struct
                        ) | (
                            crate::com_enumerator_registry::EnumeratorElementKind::Unknown,
                            RawNamedKind::Unknown
                        )
                    )
                    && match (expected.element_iid, iid.as_deref()) {
                        (Some(expected), Some(actual)) => actual.eq_ignore_ascii_case(expected),
                        (None, None) => true,
                        _ => false,
                    }
            }
            _ => false,
        };
        param.direction == expected_values_direction
            && !param.optional
            && !param.const_attribute
            && (param.free_with.is_none()
                || param.free_with.as_ref().is_some_and(|free_with| {
                    free_with.function == "SysFreeString"
                        && matches!(
                            &param.typ.native_type,
                            RawNativeType::Named { namespace, name, .. }
                                if namespace == "Windows.Win32.Foundation" && name == "BSTR"
                        )
                }))
            && (param.string_pointer_array.is_none()
                || (interface_namespace == "Windows.Win32.System.Com"
                    && interface_name == "IEnumString"))
            && param.typ.pointer_depth == 1
            && param.typ.constness == RawConstness::Mutable
            && exact_known_type
            && param.native_array.as_ref().is_some_and(|relation| {
                relation.count_param_index == Some(0)
                && relation.actual_length_param_index.is_none()
                && relation.unit == RawCountUnit::Elements
                && !relation.two_call
                && relation.evidence.iter().any(|evidence| {
                    matches!(
                        evidence,
                        RawEvidence::MetadataAttribute("NativeArrayInfoAttribute")
                    ) || matches!(
                        evidence,
                        RawEvidence::ComStandard(
                            crate::contract_registry::ComStandardRule::GenericEnumeratorNext
                        ) if expected.uses_generic_standard()
                    ) || matches!(
                        evidence,
                        RawEvidence::ExactRegistry {
                            contract_kind: crate::contract_registry::ContractKind::EnumeratorNext,
                            reason,
                            citation,
                            ..
                        } if !expected.uses_generic_standard()
                                && reason == ENUMERATOR_ARRAY_REASON
                                && citation == expected.citation
                    )
                })
            })
    });
    let exact_shape = method_name == "Next"
        && params.len() == 3
        && params
            .first()
            .is_some_and(|param| exact_count(param, RawParamDirection::In, 0) && !param.optional)
        && exact_element
        && params.get(2).is_some_and(|param| {
            exact_count(param, expected_fetched_direction, 1)
                && param.optional == expected_fetched_optional
                && param.typ.constness == RawConstness::Mutable
                && param.native_array.is_none()
                && param.free_with.is_none()
        })
        && exact_hresult;
    exact_shape.then_some(RawEnumeratorNext {
        capacity_param_index: 0,
        values_param_index: 1,
        fetched_param_index: 2,
        fetched_optional_for_single: expected_fetched_optional,
        evidence: enumerator_contract_evidence(
            expected,
            "the standard IEnum* contract defines pceltFetched as the initialized element count and permits omission only where this exact interface metadata marks it optional",
        ),
    })
}

fn enumerator_contract_evidence(
    contract: &crate::com_enumerator_registry::EnumeratorContract,
    reason: &str,
) -> RawEvidence {
    if contract.uses_generic_standard() {
        RawEvidence::ComStandard(crate::contract_registry::ComStandardRule::GenericEnumeratorNext)
    } else {
        RawEvidence::exact_registry(
            contract.entry_id(),
            contract.family_id(),
            contract.contract_kind(),
            reason,
            contract.citation,
        )
    }
}

fn is_registered_enumerator_interface(interface_namespace: &str, interface_name: &str) -> bool {
    crate::com_enumerator_registry::contract_for_declaration(interface_namespace, interface_name)
        .is_some()
}

pub(crate) fn validate_attached_enumerator_evidence(raw: &RawComMethod) -> Result<(), String> {
    if raw.metadata_name != "Next" {
        return if raw.enumerator_next.is_none() {
            Ok(())
        } else {
            Err(format!(
                "{}.{} has EnumeratorNext evidence on a non-Next method",
                raw.declaring_interface, raw.metadata_name
            ))
        };
    }
    let registered =
        is_registered_enumerator_interface(&raw.declaring_namespace, &raw.declaring_interface);
    if raw.declaring_interface.starts_with("IEnum") && !registered {
        return Err(format!(
            "{}.Next requires an exact enumerator registry entry",
            raw.declaring_interface
        ));
    }
    if !registered {
        return if raw.enumerator_next.is_none() {
            Ok(())
        } else {
            Err(format!(
                "{}.Next has unregistered EnumeratorNext evidence",
                raw.declaring_interface
            ))
        };
    }
    let expected = known_enumerator_next_override(
        &raw.declaring_namespace,
        &raw.declaring_interface,
        &raw.declaring_iid,
        &raw.metadata_name,
        raw.vtable_index,
        &raw.params,
        &raw.return_type,
    )
    .ok_or_else(|| {
        format!(
            "{}.Next signature no longer matches exact enumerator evidence (EnumeratorNext)",
            raw.declaring_interface
        )
    })?;
    if raw.enumerator_next.as_ref() != Some(&expected) {
        return Err(format!(
            "{}.Next EnumeratorNext evidence no longer matches the registry",
            raw.declaring_interface
        ));
    }
    Ok(())
}

fn known_free_with_override(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    method_name: &str,
    vtable_index: usize,
    parameter_index: usize,
    parameter_name: &str,
    typ: &windows_metadata::Type,
    direction: &ParamDirection,
) -> Option<RawFreeWith> {
    let (windows_metadata::Type::PtrMut(inner, depth)
    | windows_metadata::Type::PtrConst(inner, depth)) = typ
    else {
        return None;
    };
    if !matches!(
        direction,
        ParamDirection::Out | ParamDirection::InOut | ParamDirection::UnsupportedNativeArray { .. }
    ) {
        return None;
    }

    if *depth == 1
        && matches!(
            inner.as_ref(),
            windows_metadata::Type::Name(name)
                if name.namespace == "Windows.Win32.Foundation" && name.name == "BSTR"
        )
    {
        return Some(RawFreeWith {
            function: "SysFreeString".into(),
            evidence: RawEvidence::ComStandard(if matches!(direction, ParamDirection::InOut) {
                crate::contract_registry::ComStandardRule::BstrReplacement
            } else {
                crate::contract_registry::ComStandardRule::BstrOutputOwnershipCleanup
            }),
        });
    }
    let is_known_cotaskmem_wide_string = matches!(
        (interface_namespace, interface_name, method_name),
        ("Windows.Win32.UI.Shell", "IShellItem", "GetDisplayName")
            | ("Windows.Win32.UI.Shell", "IFileDialog", "GetFileName")
            | ("Windows.Win32.System.Com", "IPersistFile", "GetCurFile")
    );
    if *depth == 1
        && is_known_cotaskmem_wide_string
        && matches!(
            inner.as_ref(),
            windows_metadata::Type::Name(name)
                if name.namespace == "Windows.Win32.Foundation" && name.name == "PWSTR"
        )
    {
        return Some(RawFreeWith {
            function: "CoTaskMemFree".into(),
            evidence: RawEvidence::exact_registry(
                crate::contract_registry::exact_parameter_entry_id(
                    crate::contract_registry::ExactFamilyId::Ownership,
                    interface_namespace,
                    interface_name,
                    interface_iid,
                    method_name,
                    vtable_index,
                    parameter_index,
                    parameter_name,
                ),
                crate::contract_registry::ExactFamilyId::Ownership,
                crate::contract_registry::ContractKind::Ownership,
                "API documentation assigns the returned string to the COM task allocator",
                match (interface_namespace, interface_name, method_name) {
                    ("Windows.Win32.UI.Shell", "IShellItem", "GetDisplayName") => {
                        "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitem-getdisplayname"
                    }
                    ("Windows.Win32.UI.Shell", "IFileDialog", "GetFileName") => {
                        "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifiledialog-getfilename"
                    }
                    ("Windows.Win32.System.Com", "IPersistFile", "GetCurFile") => {
                        "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-ipersistfile-getcurfile"
                    }
                    _ => unreachable!("guarded by exact override registry"),
                },
            ),
        });
    }
    if *depth == 2
        && matches!(
            (interface_namespace, interface_name, method_name),
            (
                "Windows.Win32.Storage.Packaging.Opc",
                "IOpcSignatureCustomObject",
                "GetXml"
            )
        )
        && matches!(inner.as_ref(), windows_metadata::Type::U8)
    {
        return Some(RawFreeWith {
            function: "CoTaskMemFree".into(),
            evidence: RawEvidence::exact_registry(
                crate::contract_registry::exact_parameter_entry_id(
                    crate::contract_registry::ExactFamilyId::Ownership,
                    interface_namespace,
                    interface_name,
                    interface_iid,
                    method_name,
                    vtable_index,
                    parameter_index,
                    parameter_name,
                ),
                crate::contract_registry::ExactFamilyId::Ownership,
                crate::contract_registry::ContractKind::Ownership,
                "IOpcSignatureCustomObject::GetXml assigns the returned byte buffer to the COM task allocator",
                "https://learn.microsoft.com/windows/win32/api/msopc/nf-msopc-iopcsignaturecustomobject-getxml",
            ),
        });
    }
    // Windows.Win32.winmd omits FreeWith on IShellLink::GetIDList.
    if *depth < 2
        || !matches!(
            (interface_namespace, interface_name, method_name),
            ("Windows.Win32.UI.Shell", "IShellLinkW", "GetIDList")
                | ("Windows.Win32.UI.Shell", "IShellLinkA", "GetIDList")
        )
    {
        return None;
    }
    match inner.as_ref() {
        windows_metadata::Type::Name(name)
            if name.namespace == "Windows.Win32.UI.Shell.Common" && name.name == "ITEMIDLIST" =>
        {
            Some(RawFreeWith {
                function: "CoTaskMemFree".into(),
                evidence: RawEvidence::exact_registry(
                    crate::contract_registry::exact_parameter_entry_id(
                        crate::contract_registry::ExactFamilyId::Ownership,
                        interface_namespace,
                        interface_name,
                        interface_iid,
                        method_name,
                        vtable_index,
                        parameter_index,
                        parameter_name,
                    ),
                    crate::contract_registry::ExactFamilyId::Ownership,
                    crate::contract_registry::ContractKind::Ownership,
                    "IShellLink::GetIDList returns a PIDL allocated by the Shell task allocator",
                    "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishelllinkw-getidlist",
                ),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
fn known_free_with(
    interface_namespace: &str,
    interface_name: &str,
    method_name: &str,
    typ: &windows_metadata::Type,
    direction: &ParamDirection,
) -> Option<String> {
    known_free_with_override(
        interface_namespace,
        interface_name,
        "",
        method_name,
        3,
        0,
        "value",
        typ,
        direction,
    )
    .map(|free_with| free_with.function)
}

fn known_semantic_hresult_override(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    method_name: &str,
    vtable_index: usize,
) -> Option<RawEvidence> {
    matches!(
        (interface_namespace, interface_name, method_name),
        ("Windows.Win32.System.Com", "IPersistFile", "GetCurFile")
    )
    .then(|| {
        RawEvidence::exact_registry(
            crate::contract_registry::exact_method_entry_id(
                crate::contract_registry::ExactFamilyId::SemanticHresult,
                interface_namespace,
                interface_name,
                interface_iid,
                method_name,
                vtable_index,
            ),
            crate::contract_registry::ExactFamilyId::SemanticHresult,
            crate::contract_registry::ContractKind::SemanticHresult,
            "GetCurFile uses S_OK and S_FALSE as distinct successful ownership states",
            "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-ipersistfile-getcurfile",
        )
    })
}

#[cfg(test)]
fn is_known_semantic_hresult(
    interface_namespace: &str,
    interface_name: &str,
    method_name: &str,
) -> bool {
    known_semantic_hresult_override(interface_namespace, interface_name, "", method_name, 0)
        .is_some()
}

fn map_parameter_type(
    typ: &windows_metadata::Type,
    direction: &ParamDirection,
    index: &reader::Index,
) -> TypeMeta {
    use windows_metadata::Type;

    match typ {
        Type::PtrMut(inner, depth) | Type::PtrConst(inner, depth) => {
            if matches!(direction, ParamDirection::Out | ParamDirection::InOut) && *depth == 1 {
                map_com_type(inner, index)
            } else {
                TypeMeta::Object
            }
        }
        Type::ConstRef(inner)
            if matches!(direction, ParamDirection::Out | ParamDirection::InOut) =>
        {
            map_com_type(inner, index)
        }
        Type::ConstRef(_) => TypeMeta::Object,
        _ => map_com_type(typ, index),
    }
}

fn map_return_type(typ: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    use windows_metadata::Type;

    match typ {
        Type::PtrMut(_, _) | Type::PtrConst(_, _) | Type::ConstRef(_) => TypeMeta::Object,
        _ => map_com_type(typ, index),
    }
}

fn map_raw_com_type(typ: &windows_metadata::Type, index: &reader::Index) -> RawComType {
    map_raw_com_type_inner(typ, index, &mut Vec::new())
}

fn map_raw_com_type_inner(
    typ: &windows_metadata::Type,
    index: &reader::Index,
    layout_stack: &mut Vec<(String, String)>,
) -> RawComType {
    use windows_metadata::Type;

    let scalar = |native_type| RawComType {
        native_type,
        underlying: None,
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    };
    match typ {
        Type::PtrMut(inner, depth) => add_raw_pointer(
            map_raw_com_type_inner(inner, index, layout_stack),
            *depth,
            RawConstness::Mutable,
        ),
        Type::PtrConst(inner, depth) => add_raw_pointer(
            map_raw_com_type_inner(inner, index, layout_stack),
            *depth,
            RawConstness::Const,
        ),
        Type::ConstRef(inner) => add_raw_pointer(
            map_raw_com_type_inner(inner, index, layout_stack),
            1,
            RawConstness::Const,
        ),
        Type::Void => scalar(RawNativeType::Void),
        Type::Bool => scalar(RawNativeType::Bool),
        Type::I8 => scalar(RawNativeType::I8),
        Type::U8 => scalar(RawNativeType::U8),
        Type::I16 => scalar(RawNativeType::I16),
        Type::U16 => scalar(RawNativeType::U16),
        Type::I32 => scalar(RawNativeType::I32),
        Type::U32 => scalar(RawNativeType::U32),
        Type::I64 => scalar(RawNativeType::I64),
        Type::U64 => scalar(RawNativeType::U64),
        Type::F32 => scalar(RawNativeType::F32),
        Type::F64 => scalar(RawNativeType::F64),
        Type::Char => scalar(RawNativeType::Char16),
        Type::ISize => scalar(RawNativeType::ISize),
        Type::USize => scalar(RawNativeType::USize),
        Type::String => scalar(RawNativeType::String),
        Type::Object => scalar(RawNativeType::Object),
        Type::Name(name) => {
            if !name.generics.is_empty() {
                return scalar(RawNativeType::Unknown(format!(
                    "closed generic type {}.{} requires a computed IID",
                    name.namespace, name.name
                )));
            }
            let definition = index.get(&name.namespace, &name.name).next();
            let underlying = definition.as_ref().and_then(|definition| {
                let mut fields = definition.fields().filter(|field| {
                    !field
                        .flags()
                        .contains(windows_metadata::FieldAttributes::Static)
                });
                let field = fields.next()?;
                (matches!(field.name(), "Value" | "value__") && fields.next().is_none())
                    .then(|| Box::new(map_raw_com_type_inner(&field.ty(), index, layout_stack)))
            });
            let kind = definition
                .as_ref()
                .map_or(RawNamedKind::Unknown, |definition| {
                    if definition
                        .flags()
                        .contains(windows_metadata::TypeAttributes::Interface)
                    {
                        RawNamedKind::Interface
                    } else if parse_com_enum_def(definition).is_some() {
                        RawNamedKind::Enum
                    } else if parse_com_delegate_def(definition).is_some() {
                        RawNamedKind::Delegate
                    } else if definition.extends().is_some_and(|base| {
                        base.namespace() == "System" && base.name() == "ValueType"
                    }) {
                        RawNamedKind::Struct
                    } else {
                        RawNamedKind::RuntimeClass
                    }
                });
            let iid = definition.as_ref().and_then(|definition| {
                let iid = if kind == RawNamedKind::RuntimeClass {
                    raw_runtime_class_default_iid(definition, index)?
                } else {
                    crate::meta::extract_iid(definition)
                };
                (!iid.is_empty()).then_some(iid)
            });
            let layout = (kind == RawNamedKind::Struct).then(|| {
                Box::new(parse_raw_native_layouts(
                    index,
                    &name.namespace,
                    &name.name,
                    layout_stack,
                ))
            });
            RawComType {
                native_type: RawNativeType::Named {
                    namespace: name.namespace.clone(),
                    name: name.name.clone(),
                    kind,
                    iid,
                    layout,
                },
                underlying,
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            }
        }
        Type::Array(inner) => scalar(RawNativeType::Array(Box::new(map_raw_com_type_inner(
            inner,
            index,
            layout_stack,
        )))),
        Type::ArrayFixed(inner, count) => scalar(RawNativeType::FixedArray {
            element: Box::new(map_raw_com_type_inner(inner, index, layout_stack)),
            count: *count,
        }),
        unsupported => scalar(RawNativeType::Unknown(format!("{unsupported:?}"))),
    }
}

fn parse_raw_native_layouts(
    index: &reader::Index,
    namespace: &str,
    name: &str,
    layout_stack: &mut Vec<(String, String)>,
) -> RawNativeLayoutSet {
    let key = (namespace.to_string(), name.to_string());
    if layout_stack.contains(&key) {
        return RawNativeLayoutSet {
            recursive: true,
            variants: Vec::new(),
        };
    }
    layout_stack.push(key);
    let variants = index
        .get(namespace, name)
        .filter(|definition| {
            definition
                .extends()
                .is_some_and(|base| base.namespace() == "System" && base.name() == "ValueType")
        })
        .map(|definition| {
            let flags = definition.flags();
            let is_union = is_known_win32_union(&definition.namespace(), definition.name());
            let kind = if flags.contains(windows_metadata::TypeAttributes::SequentialLayout) {
                RawLayoutKind::Sequential
            } else if flags.contains(windows_metadata::TypeAttributes::ExplicitLayout) && is_union {
                RawLayoutKind::Explicit
            } else {
                RawLayoutKind::Unknown
            };
            let (packing, declared_size) =
                definition
                    .class_layout()
                    .map_or((RawPacking::Default, None), |layout| {
                        (
                            if layout.packing_size() == 0 {
                                RawPacking::Default
                            } else {
                                RawPacking::Explicit(layout.packing_size())
                            },
                            (layout.class_size() != 0).then_some(layout.class_size() as usize),
                        )
                    });
            let fields = definition
                .fields()
                .filter(|field| field.name() != "value__" && field.constant().is_none())
                .map(|field| {
                    let field_type = field.ty();
                    let (typ, fixed_count) = match field_type {
                        windows_metadata::Type::ArrayFixed(ref element, count) => (
                            map_raw_com_type_inner(element, index, layout_stack),
                            Some(count),
                        ),
                        _ => (
                            map_raw_com_type_inner(&field_type, index, layout_stack),
                            None,
                        ),
                    };
                    RawNativeField {
                        name: field.name().to_string(),
                        typ,
                        // windows-metadata 0.59 does not expose the ECMA-335
                        // FieldLayout table. Explicit Win32 layouts therefore
                        // remain fail-closed unless an adapter supplies offsets.
                        explicit_offset: None,
                        fixed_count,
                        bitfield: field.has_attribute("NativeBitfieldAttribute"),
                        flexible_array: field.has_attribute("FlexibleArrayAttribute"),
                    }
                })
                .collect();
            RawNativeLayout {
                architectures: supported_architecture_mask(&definition),
                kind,
                packing,
                declared_size,
                fields,
                is_union,
            }
        })
        .collect();
    layout_stack.pop();
    RawNativeLayoutSet {
        recursive: false,
        variants,
    }
}

fn is_known_win32_union(namespace: &str, name: &str) -> bool {
    matches!((namespace, name), ("Windows.Win32.System.Com", "BINDPTR"))
}

#[cfg(test)]
mod explicit_layout_tests {
    use super::is_known_win32_union;

    #[test]
    fn explicit_layout_requires_exact_union_identity() {
        assert!(is_known_win32_union("Windows.Win32.System.Com", "BINDPTR"));
        assert!(!is_known_win32_union("Contoso", "BINDPTR"));
        assert!(!is_known_win32_union(
            "Windows.Win32.System.Com",
            "EXPLICIT_STRUCT"
        ));
    }
}

fn supported_architecture_mask(definition: &reader::TypeDef) -> u8 {
    definition
        .find_attribute("SupportedArchitectureAttribute")
        .and_then(|attribute| {
            attribute
                .value()
                .into_iter()
                .next()
                .and_then(|(_, value)| match value {
                    windows_metadata::Value::I32(value) => u8::try_from(value).ok(),
                    windows_metadata::Value::U32(value) => u8::try_from(value).ok(),
                    _ => None,
                })
        })
        .unwrap_or(0b111)
        & 0b111
}

fn raw_runtime_class_default_iid(
    definition: &reader::TypeDef,
    index: &reader::Index,
) -> Option<String> {
    definition
        .interface_impls()
        .find(|implementation| implementation.has_attribute("DefaultAttribute"))
        .and_then(|implementation| match implementation.interface(&[]) {
            windows_metadata::Type::Name(name) => index.get(&name.namespace, &name.name).next(),
            _ => None,
        })
        .map(|interface| crate::meta::extract_iid(&interface))
        .filter(|iid| !iid.is_empty())
}

fn add_raw_pointer(mut inner: RawComType, depth: usize, constness: RawConstness) -> RawComType {
    if inner.pointer_depth > 0
        && !matches!(
            inner.constness,
            RawConstness::Mixed | RawConstness::Unspecified
        )
        && inner.constness != constness
    {
        inner.constness = RawConstness::Mixed;
    } else if !matches!(
        inner.constness,
        RawConstness::Mixed | RawConstness::Unspecified
    ) {
        inner.constness = constness;
    } else if inner.pointer_depth == 0 {
        inner.constness = constness;
    }
    inner.pointer_depth = inner.pointer_depth.saturating_add(depth);
    inner
}

fn map_com_type(typ: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    match typ {
        windows_metadata::Type::ISize => native_isize_type(),
        windows_metadata::Type::USize => native_usize_type(),
        windows_metadata::Type::Name(name)
            if is_canonical_hstring_name(&name.namespace, &name.name) =>
        {
            TypeMeta::String
        }
        windows_metadata::Type::Name(name) => {
            if let Some(def) = index.get(&name.namespace, &name.name).next() {
                if let Some(enum_meta) = parse_com_enum_def(&def) {
                    return enum_meta.as_type_meta();
                }
                if let Some(delegate) = parse_com_delegate_def(&def) {
                    return delegate;
                }
            }
            crate::meta::map_winmd_type_with_generics(typ, index, &[])
        }
        _ => crate::meta::map_winmd_type_with_generics(typ, index, &[]),
    }
}

fn is_canonical_hstring_name(namespace: &str, name: &str) -> bool {
    namespace == "Windows.Win32.System.WinRT" && name == "HSTRING"
}

fn parse_com_delegate_def(def: &reader::TypeDef) -> Option<TypeMeta> {
    let extends = def.extends()?;
    if !matches!(
        (extends.namespace(), extends.name()),
        ("System", "Delegate") | ("System", "MulticastDelegate")
    ) {
        return None;
    }
    Some(TypeMeta::Delegate {
        namespace: def.namespace().to_string(),
        name: def.name().to_string(),
        iid: crate::meta::extract_iid(def),
    })
}

impl ComEnumMeta {
    fn as_type_meta(&self) -> TypeMeta {
        TypeMeta::Enum {
            namespace: self.namespace.clone(),
            name: self.name.clone(),
            underlying: Box::new(self.underlying.clone()),
            members: Vec::new(),
            is_flags: self.is_flags,
            doc: None,
            deprecated: None,
        }
    }
}

fn parse_com_enum_def(def: &reader::TypeDef) -> Option<ComEnumMeta> {
    let mut fields = def.fields();
    let underlying = fields
        .find(|field| field.name() == "value__")
        .and_then(|field| map_com_enum_underlying(&field.ty()))?;
    let members = def
        .fields()
        .filter(|field| field.name() != "value__")
        .filter_map(|field| {
            let value = match field.constant()?.value() {
                windows_metadata::Value::I8(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U8(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I16(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U16(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I32(value) => ComEnumValue::Signed(i64::from(value)),
                windows_metadata::Value::U32(value) => ComEnumValue::Unsigned(u64::from(value)),
                windows_metadata::Value::I64(value) => ComEnumValue::Signed(value),
                windows_metadata::Value::U64(value) => ComEnumValue::Unsigned(value),
                _ => return None,
            };
            Some(ComEnumMember {
                name: field.name().to_string(),
                value,
            })
        })
        .collect();
    Some(ComEnumMeta {
        namespace: def.namespace().to_string(),
        name: def.name().to_string(),
        underlying,
        members,
        is_flags: def.has_attribute("FlagsAttribute"),
    })
}

fn map_com_enum_underlying(typ: &windows_metadata::Type) -> Option<TypeMeta> {
    match typ {
        windows_metadata::Type::I8 => Some(TypeMeta::I8),
        windows_metadata::Type::U8 => Some(TypeMeta::U8),
        windows_metadata::Type::I16 => Some(TypeMeta::I16),
        windows_metadata::Type::U16 => Some(TypeMeta::U16),
        windows_metadata::Type::I32 => Some(TypeMeta::I32),
        windows_metadata::Type::U32 => Some(TypeMeta::U32),
        windows_metadata::Type::I64 => Some(TypeMeta::I64),
        windows_metadata::Type::U64 => Some(TypeMeta::U64),
        _ => None,
    }
}

pub fn native_isize_type() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "System".into(),
        name: "IntPtr".into(),
        fields: Vec::new(),
    }
}

pub fn native_usize_type() -> TypeMeta {
    TypeMeta::Struct {
        namespace: "System".into(),
        name: "UIntPtr".into(),
        fields: Vec::new(),
    }
}

pub fn is_native_isize(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct {
            namespace,
            name,
            ..
        } if namespace == "System" && name == "IntPtr"
    )
}

pub fn is_native_usize(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct {
            namespace,
            name,
            ..
        } if namespace == "System" && name == "UIntPtr"
    )
}

fn native_array_count_param(param: &reader::MethodParam) -> Option<Option<usize>> {
    let attribute = param.find_attribute("NativeArrayInfoAttribute")?;
    let count = attribute
        .value()
        .into_iter()
        .find(|(name, _)| name == "CountParamIndex")
        .and_then(|(_, value)| match value {
            windows_metadata::Value::I16(value) if value >= 0 => Some(value as usize),
            windows_metadata::Value::U16(value) => Some(value as usize),
            windows_metadata::Value::I32(value) if value >= 0 => Some(value as usize),
            windows_metadata::Value::U32(value) => usize::try_from(value).ok(),
            _ => None,
        });
    Some(count)
}

fn classify_raw_string_pointer_array(
    typ: &RawComType,
    direction: RawParamDirection,
    free_with: Option<&RawFreeWith>,
) -> Option<RawStringPointerArray> {
    if typ.pointer_depth == 0 {
        return None;
    }
    let RawNativeType::Named {
        namespace,
        name,
        layout,
        ..
    } = &typ.native_type
    else {
        return None;
    };
    if namespace != "Windows.Win32.Foundation" {
        return None;
    }
    let encoding = if matches!(name.as_str(), "PWSTR" | "PCWSTR" | "LPWSTR" | "LPCWSTR") {
        RawStringEncoding::Utf16
    } else if matches!(name.as_str(), "PSTR" | "PCSTR" | "LPSTR" | "LPCSTR") {
        RawStringEncoding::Ansi
    } else {
        return None;
    };
    let (pointer_depth, constness) = layout
        .as_deref()
        .and_then(|set| set.variants.first())
        .and_then(|layout| layout.fields.first())
        .map(|field| (field.typ.pointer_depth, field.typ.constness))
        .unwrap_or((
            1,
            if matches!(name.as_str(), "PCWSTR" | "LPCWSTR" | "PCSTR" | "LPCSTR") {
                RawConstness::Const
            } else {
                RawConstness::Mutable
            },
        ));
    Some(RawStringPointerArray {
        encoding,
        pointer_depth,
        constness,
        ownership: if direction == RawParamDirection::In && free_with.is_none() {
            RawElementOwnership::Borrowed
        } else {
            RawElementOwnership::Unknown
        },
    })
}

fn classify_direction(flags: windows_metadata::ParamAttributes, is_array: bool) -> ParamDirection {
    let is_in = flags.contains(windows_metadata::ParamAttributes::In);
    let is_out = flags.contains(windows_metadata::ParamAttributes::Out);
    match (is_in, is_out, is_array) {
        (true, true, _) => ParamDirection::InOut,
        (_, true, true) => ParamDirection::OutFill,
        (_, true, false) => ParamDirection::Out,
        _ => ParamDirection::In,
    }
}

fn raw_param_direction(flags: windows_metadata::ParamAttributes) -> RawParamDirection {
    let is_in = flags.contains(windows_metadata::ParamAttributes::In);
    let is_out = flags.contains(windows_metadata::ParamAttributes::Out);
    match (is_in, is_out) {
        (true, true) => RawParamDirection::InOut,
        (_, true) => RawParamDirection::Out,
        _ => RawParamDirection::In,
    }
}

fn find_coclass(
    index: &reader::Index,
    namespace: &str,
    interface_name: &str,
) -> (Option<String>, Option<String>) {
    for candidate in coclass_name_candidates(interface_name) {
        let Some(def) = index.get(namespace, &candidate).next() else {
            continue;
        };
        if is_com_coclass(&def) {
            let clsid = crate::meta::extract_iid(&def);
            if !clsid.is_empty() {
                return (Some(candidate), Some(clsid));
            }
        }
    }
    (None, None)
}

fn coclass_name_candidates(interface_name: &str) -> Vec<String> {
    let Some(stripped) = interface_name.strip_prefix('I') else {
        return Vec::new();
    };
    let mut candidates = vec![stripped.to_string()];
    let without_version = stripped
        .trim_end_matches(|character: char| character.is_ascii_digit())
        .to_string();
    if without_version != stripped {
        candidates.push(without_version);
    }
    candidates
}

fn is_com_coclass(def: &reader::TypeDef) -> bool {
    !def.flags()
        .contains(windows_metadata::TypeAttributes::Interface)
        && matches!(
            def.extends()
                .map(|base| (base.namespace().to_string(), base.name().to_string())),
            Some((namespace, name)) if namespace == "System" && name == "ValueType"
        )
        && !crate::meta::extract_iid(def).is_empty()
}

fn collect_referenced_enums(index: &reader::Index, interface: &InterfaceMeta) -> Vec<ComEnumMeta> {
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for method in &interface.methods {
        for typ in method
            .params
            .iter()
            .map(|param| &param.typ)
            .chain(method.return_type.iter())
        {
            if let TypeMeta::Enum {
                namespace, name, ..
            } = typ
            {
                let full_name = format!("{namespace}.{name}");
                if names.insert(full_name)
                    && let Some(enum_meta) = index
                        .get(namespace, name)
                        .next()
                        .and_then(|def| parse_com_enum_def(&def))
                {
                    result.push(enum_meta);
                }
            }
        }
    }
    result
}

fn is_string_buffer(typ: &TypeMeta) -> bool {
    matches!(
        typ,
        TypeMeta::Struct { namespace, name, .. }
            if namespace == "Windows.Win32.Foundation" && (name == "PWSTR" || name == "PSTR")
    )
}

pub fn find_runtime_class_default_iid(
    winmd_paths: &str,
    simple_name: &str,
) -> Option<(String, String, String)> {
    let index = crate::meta::load_index(winmd_paths)?;
    let mut found = None;
    let mut collision = false;
    for def in index.all() {
        if def.name() != simple_name
            || !def
                .flags()
                .contains(windows_metadata::TypeAttributes::WindowsRuntime)
            || def
                .flags()
                .contains(windows_metadata::TypeAttributes::Interface)
        {
            continue;
        }
        for implementation in def.interface_impls() {
            if !implementation.has_attribute("DefaultAttribute") {
                continue;
            }
            let windows_metadata::Type::Name(name) = implementation.interface(&[]) else {
                continue;
            };
            if !name.generics.is_empty() {
                continue;
            }
            let interface = index.get(&name.namespace, &name.name).next()?;
            let iid = crate::meta::extract_iid(&interface);
            if iid.is_empty() {
                continue;
            }
            let candidate = (def.namespace().to_string(), name.name, iid);
            match &found {
                None => found = Some(candidate),
                Some(existing) if existing == &candidate => {}
                Some(_) => collision = true,
            }
            break;
        }
    }
    (!collision).then_some(found).flatten()
}

pub fn discover_newest_windows_winmd() -> Option<String> {
    let base = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata");
    let mut versions = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("10."))
        .collect::<Vec<_>>();
    versions.sort_by_key(|version| {
        version
            .split('.')
            .filter_map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
    });
    versions.into_iter().rev().find_map(|version| {
        let path = base.join(version).join("Windows.winmd");
        path.exists().then(|| path.to_string_lossy().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IWBEM_SERVICES_IID: &str = "9556dc99-828c-11cf-a37e-00aa003240c7";
    const IWBEM_CALL_RESULT_IID: &str = "44aca675-e8fc-11d0-a07c-00c04fb68820";

    fn wbem_open_namespace_fixture() -> (MethodMeta, RawComMethod) {
        let scalar_type = |native_type| RawComType {
            native_type,
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let wrapper_type =
            |namespace: &str, name: &str, field_type: RawComType, underlying: RawComType| {
                RawComType {
                    native_type: RawNativeType::Named {
                        namespace: namespace.into(),
                        name: name.into(),
                        kind: RawNamedKind::Struct,
                        iid: None,
                        layout: Some(Box::new(RawNativeLayoutSet {
                            recursive: false,
                            variants: vec![RawNativeLayout {
                                architectures: 7,
                                kind: RawLayoutKind::Sequential,
                                packing: RawPacking::Default,
                                declared_size: None,
                                fields: vec![RawNativeField {
                                    name: "Value".into(),
                                    typ: field_type,
                                    explicit_offset: None,
                                    fixed_count: None,
                                    bitfield: false,
                                    flexible_array: false,
                                }],
                                is_union: false,
                            }],
                        })),
                    },
                    underlying: Some(Box::new(underlying)),
                    pointer_depth: 0,
                    constness: RawConstness::Unspecified,
                }
            };
        let input = |name: &str, typ: RawComType, const_attribute| RawComParam {
            name: name.into(),
            typ,
            direction: RawParamDirection::In,
            optional: false,
            const_attribute,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        };
        let char_pointer = RawComType {
            native_type: RawNativeType::Char16,
            underlying: None,
            pointer_depth: 1,
            constness: RawConstness::Mutable,
        };
        let bstr = wrapper_type(
            "Windows.Win32.Foundation",
            "BSTR",
            char_pointer.clone(),
            char_pointer,
        );
        let i32_type = scalar_type(RawNativeType::I32);
        let flags = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.System.Wmi".into(),
                name: "WBEM_GENERIC_FLAG_TYPE".into(),
                kind: RawNamedKind::Enum,
                iid: None,
                layout: None,
            },
            underlying: Some(Box::new(i32_type.clone())),
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let context = RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.System.Wmi".into(),
                name: "IWbemContext".into(),
                kind: RawNamedKind::Interface,
                iid: Some("44aca674-e8fc-11d0-a07c-00c04fb68820".into()),
                layout: None,
            },
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let hresult = wrapper_type(
            "Windows.Win32.Foundation",
            "HRESULT",
            i32_type.clone(),
            i32_type,
        );
        let output = |name: &str, interface: &str, iid: &str, optional| RawComParam {
            name: name.into(),
            typ: RawComType {
                native_type: RawNativeType::Named {
                    namespace: "Windows.Win32.System.Wmi".into(),
                    name: interface.into(),
                    kind: RawNamedKind::Interface,
                    iid: Some(iid.into()),
                    layout: None,
                },
                underlying: None,
                pointer_depth: 1,
                constness: RawConstness::Mutable,
            },
            direction: RawParamDirection::InOut,
            optional,
            const_attribute: false,
            native_array: None,
            string_pointer_array: None,
            free_with: None,
            safe_array_evidence: None,
            exact_interface_output: None,
        };
        let raw = RawComMethod {
            declaring_namespace: "Windows.Win32.System.Wmi".into(),
            declaring_interface: "IWbemServices".into(),
            declaring_iid: IWBEM_SERVICES_IID.into(),
            metadata_name: "OpenNamespace".into(),
            projected_name: "OpenNamespace".into(),
            vtable_index: 3,
            params: vec![
                input("strNamespace", bstr, true),
                input("lFlags", flags, false),
                input("pCtx", context, false),
                output(
                    "ppWorkingNamespace",
                    "IWbemServices",
                    IWBEM_SERVICES_IID,
                    true,
                ),
                output("ppResult", "IWbemCallResult", IWBEM_CALL_RESULT_IID, true),
            ],
            return_type: hresult,
            semantic_hresult: None,
            enumerator_next: None,
            exact_contract: None,
            interface_replacement_contracts: Vec::new(),
            exact_interface_output_call: None,
            safe_array_contract_error: None,
        };
        let compatibility = MethodMeta {
            name: "openNamespace".into(),
            vtable_index: 3,
            params: raw
                .params
                .iter()
                .map(|param| ParamMeta {
                    name: param.name.clone(),
                    typ: crate::types::TypeMeta::I32,
                    direction: match param.direction {
                        RawParamDirection::In => ParamDirection::In,
                        RawParamDirection::Out => ParamDirection::Out,
                        RawParamDirection::InOut => ParamDirection::InOut,
                    },
                })
                .collect(),
            ..MethodMeta::default()
        };
        (compatibility, raw)
    }

    fn assert_open_namespace_override_rejects_drift(mutate: impl FnOnce(&mut RawComMethod)) {
        let (mut compatibility, mut raw) = wbem_open_namespace_fixture();
        mutate(&mut raw);
        compatibility.params.truncate(raw.params.len());
        assert!(!apply_exact_parameter_direction_overrides(
            &mut compatibility,
            &mut raw
        ));
    }

    #[test]
    fn iwbem_open_namespace_out_override_is_exact_and_cited() {
        let (mut compatibility, mut raw) = wbem_open_namespace_fixture();
        assert_eq!(
            raw_method_fingerprint(&raw),
            crate::contract_registry::conditional_output_contract(
                crate::contract_registry::WMI_OPEN_NAMESPACE_ENTRY_ID
            )
            .unwrap()
            .selector
            .source_fingerprint
        );
        assert!(apply_exact_parameter_direction_overrides(
            &mut compatibility,
            &mut raw
        ));
        let call = raw.exact_interface_output_call.as_ref().unwrap();
        assert_eq!(call.synchronous_flags, 0);
        assert_eq!(call.semisynchronous_flag_value, 16);
        for index in [3, 4] {
            assert_eq!(raw.params[index].direction, RawParamDirection::Out);
            assert_eq!(compatibility.params[index].direction, ParamDirection::Out);
            let evidence = &raw.params[index]
                .exact_interface_output
                .as_ref()
                .unwrap()
                .evidence;
            let RawEvidence::ExactRegistry {
                entry_id,
                family_id,
                contract_kind,
                reason,
                citation,
            } = evidence
            else {
                panic!("OpenNamespace output must carry override evidence");
            };
            assert_eq!(
                entry_id,
                crate::contract_registry::WMI_OPEN_NAMESPACE_ENTRY_ID
            );
            assert_eq!(
                *family_id,
                crate::contract_registry::ExactFamilyId::ConditionalOutput
            );
            assert_eq!(
                *contract_kind,
                crate::contract_registry::ContractKind::ConditionalOutput
            );
            assert!(reason.contains("owned +1"));
            assert!(citation.contains("learn.microsoft.com"));
            assert!(citation.contains("WbemIdl.idl"));
        }
    }

    #[test]
    fn external_evidence_families_have_stable_typed_provenance() {
        let borrowed = crate::com_borrowed_handle_registry::borrowed_hwnd_evidence_for_declaration(
            "Windows.Win32.System.Ole",
            "IOleWindow",
            "GetWindow",
            3,
        )
        .unwrap();
        assert!(
            borrowed
                .entry_id()
                .starts_with("windows.borrowed-hwnd-output.entry.")
        );
        assert_eq!(
            borrowed.family_id(),
            crate::contract_registry::ExactFamilyId::BorrowedHwndOutput
        );
        assert_eq!(
            borrowed.contract_kind(),
            crate::contract_registry::ContractKind::BorrowedHandle
        );

        let enumerator = crate::com_enumerator_registry::contract_for_declaration(
            "Windows.Win32.System.Com",
            "IEnumString",
        )
        .unwrap();
        assert!(
            enumerator
                .entry_id()
                .starts_with("com.enumerator-next-exception.entry.")
        );
        assert_eq!(
            enumerator.family_id(),
            crate::contract_registry::ExactFamilyId::EnumeratorException
        );
        assert_eq!(
            enumerator.contract_kind(),
            crate::contract_registry::ContractKind::EnumeratorNext
        );

        let safe_array = &crate::com_safe_array_registry::all_safe_array_evidence()[0];
        assert!(
            safe_array
                .entry_id()
                .starts_with("automation.safearray.entry.")
        );
        assert_eq!(
            safe_array.family_id(),
            crate::contract_registry::ExactFamilyId::SafeArray
        );
        assert_eq!(
            safe_array.contract_kind(),
            crate::contract_registry::ContractKind::Safearray
        );

        for (namespace, interface, method, expected_family, kind) in [
            (
                "Windows.Win32.System.Com",
                "IStream",
                "Stat",
                crate::contract_registry::ExactFamilyId::Ownership,
                crate::contract_registry::ContractKind::Ownership,
            ),
            (
                "Windows.Win32.System.Com",
                "IMalloc",
                "Alloc",
                crate::contract_registry::ExactFamilyId::Ownership,
                crate::contract_registry::ContractKind::Ownership,
            ),
            (
                "Windows.Win32.Graphics.Dxgi",
                "IDXGIObject",
                "GetPrivateData",
                crate::contract_registry::ExactFamilyId::PrivateDataHazard,
                crate::contract_registry::ContractKind::Hazard,
            ),
        ] {
            let contract = registered_exact_method_contract(namespace, interface, method).unwrap();
            assert_eq!(contract.family_id(), expected_family);
            assert!(crate::contract_registry::valid_exact_entry_id(
                &contract.entry_id()
            ));
            assert_eq!(contract.contract_kind(), kind);
        }
    }

    #[test]
    fn iwbem_open_namespace_out_override_rejects_every_identity_and_shape_drift() {
        assert_open_namespace_override_rejects_drift(|method| {
            method.declaring_namespace.push_str(".Drift")
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.declaring_interface.push_str("Drift")
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.declaring_iid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.metadata_name.push_str("Drift")
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.projected_name.push_str("Drift")
        });
        assert_open_namespace_override_rejects_drift(|method| method.vtable_index += 1);
        assert_open_namespace_override_rejects_drift(|method| {
            method.params.pop();
        });
        assert_open_namespace_override_rejects_drift(|method| method.params.swap(3, 4));
        for index in 0..3 {
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].name.push_str("Drift")
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].typ.pointer_depth += 1
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].direction = RawParamDirection::Out
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].optional = true
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].const_attribute = !method.params[index].const_attribute
            });
        }
        assert_open_namespace_override_rejects_drift(|method| {
            method.return_type = RawComType {
                native_type: RawNativeType::Void,
                underlying: None,
                pointer_depth: 0,
                constness: RawConstness::Unspecified,
            }
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.semantic_hresult = Some(RawEvidence::MetadataAttribute(
                "CanReturnMultipleSuccessValuesAttribute",
            ))
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.enumerator_next = Some(RawEnumeratorNext {
                capacity_param_index: 1,
                values_param_index: 3,
                fetched_param_index: 4,
                fetched_optional_for_single: false,
                evidence: RawEvidence::exact_registry(
                    "tests.enumerator.drift.v1",
                    crate::contract_registry::ExactFamilyId::EnumeratorException,
                    crate::contract_registry::ContractKind::EnumeratorNext,
                    "drift",
                    "test://drift",
                ),
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.params[0].native_array = Some(RawArrayRelation {
                count_param_index: Some(1),
                actual_length_param_index: None,
                unit: RawCountUnit::Elements,
                two_call: false,
                projected_capacity: false,
                constness: Some(RawConstness::Const),
                evidence: Vec::new(),
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.params[0].string_pointer_array = Some(RawStringPointerArray {
                encoding: RawStringEncoding::Utf16,
                pointer_depth: 1,
                constness: RawConstness::Const,
                ownership: RawElementOwnership::Borrowed,
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.params[0].free_with = Some(RawFreeWith {
                function: "CoTaskMemFree".into(),
                evidence: RawEvidence::MetadataAttribute("FreeWithAttribute"),
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.params[0].safe_array_evidence = Some(RawSafeArrayEvidence {
                declaring_namespace: "Tests",
                declaring_interface: "ITest",
                declaring_iid: IWBEM_SERVICES_IID,
                method_name: "OpenNamespace",
                vtable_index: 3,
                parameter_index: 0,
                parameter_name: "strNamespace",
                element_vartype: RawSafeArrayVartype::I4,
                element_iid: None,
                ownership: RawSafeArrayOwnership::BorrowedInput,
                raw_method_shape: "drift",
                reason: "drift",
                citation: "test://drift",
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.exact_contract = Some(RawExactMethodContract {
                kind: RawExactMethodContractKind::FixedCapacityBytes,
                declaring_namespace: "Tests",
                declaring_interface: "ITest",
                declaring_iid: IWBEM_SERVICES_IID,
                method_name: "OpenNamespace",
                vtable_index: 3,
                buffer_param_index: 0,
                capacity_param_index: 1,
                actual_length_param_index: None,
                citation: "test://drift",
                reason: "drift",
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method
                .interface_replacement_contracts
                .push(RawInterfaceReplacementContract {
                    parameter_index: 3,
                    semantics: RawInterfaceReplacementSemantics::Unchanged,
                    evidence: RawEvidence::exact_registry(
                        "tests.replacement.drift.v1",
                        crate::contract_registry::ExactFamilyId::Ownership,
                        crate::contract_registry::ContractKind::Ownership,
                        "drift",
                        "test://drift",
                    ),
                })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.exact_interface_output_call = Some(RawExactInterfaceOutputCallContract {
                source_fingerprint: "drift".into(),
                public_input_param_indices: vec![0],
                flags_param_index: 1,
                context_param_index: 2,
                synchronous_output_param_index: 3,
                semisynchronous_output_param_index: 4,
                synchronous_flags: 0,
                semisynchronous_flag_value: 0x10,
                flags_option_name: "lFlags".into(),
                synchronous_output_option_name: "workingNamespace".into(),
                semisynchronous_output_option_name: "result".into(),
                evidence: RawEvidence::exact_registry(
                    "tests.conditional-output.drift.v1",
                    crate::contract_registry::ExactFamilyId::ConditionalOutput,
                    crate::contract_registry::ContractKind::ConditionalOutput,
                    "drift",
                    "test://drift",
                ),
            })
        });
        assert_open_namespace_override_rejects_drift(|method| {
            method.safe_array_contract_error = Some("drift".into())
        });
        for index in [3, 4] {
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].name.push_str("Drift")
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].typ.native_type = RawNativeType::U32
            });
            assert_open_namespace_override_rejects_drift(|method| {
                if let RawNativeType::Named { namespace, .. } =
                    &mut method.params[index].typ.native_type
                {
                    namespace.push_str(".Drift");
                }
            });
            assert_open_namespace_override_rejects_drift(|method| {
                if let RawNativeType::Named { name, .. } = &mut method.params[index].typ.native_type
                {
                    name.push_str("Drift");
                }
            });
            assert_open_namespace_override_rejects_drift(|method| {
                if let RawNativeType::Named { kind, .. } = &mut method.params[index].typ.native_type
                {
                    *kind = RawNamedKind::Struct;
                }
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].typ.pointer_depth += 1
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].typ.underlying = Some(Box::new(RawComType {
                    native_type: RawNativeType::U32,
                    underlying: None,
                    pointer_depth: 0,
                    constness: RawConstness::Unspecified,
                }))
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].typ.constness = RawConstness::Const
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].exact_interface_output =
                    Some(RawExactInterfaceOutputContract {
                        interface_iid: IWBEM_SERVICES_IID.into(),
                        argument_optional: true,
                        nullable_on_success: false,
                        evidence: RawEvidence::exact_registry(
                            "tests.output.drift.v1",
                            crate::contract_registry::ExactFamilyId::Ownership,
                            crate::contract_registry::ContractKind::Ownership,
                            "drift",
                            "test://drift",
                        ),
                    })
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].optional = !method.params[index].optional
            });
            assert_open_namespace_override_rejects_drift(|method| {
                method.params[index].direction = RawParamDirection::In
            });
            assert_open_namespace_override_rejects_drift(|method| {
                if let RawNativeType::Named { iid, .. } = &mut method.params[index].typ.native_type
                {
                    *iid = Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into());
                }
            });
        }
    }

    #[test]
    fn safe_array_registry_matches_configured_win32_metadata() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let index = crate::meta::load_index(&winmd).expect("configured Win32 metadata must load");
        let evidence = crate::com_safe_array_registry::all_safe_array_evidence();
        let mut interfaces = std::collections::BTreeMap::new();
        for entry in evidence {
            interfaces
                .entry((entry.declaring_namespace, entry.declaring_interface))
                .or_insert_with(|| {
                    parse_com_interface_from_index(
                        &index,
                        entry.declaring_namespace,
                        entry.declaring_interface,
                    )
                    .unwrap_or_else(|| {
                        panic!(
                            "{}.{} is missing",
                            entry.declaring_namespace, entry.declaring_interface
                        )
                    })
                });
        }
        for entry in evidence {
            let interface = &interfaces[&(entry.declaring_namespace, entry.declaring_interface)];
            let method = interface
                .raw_methods
                .as_ref()
                .unwrap()
                .iter()
                .find(|method| {
                    method.declaring_namespace == entry.declaring_namespace
                        && method.declaring_interface == entry.declaring_interface
                        && method
                            .declaring_iid
                            .eq_ignore_ascii_case(entry.declaring_iid)
                        && method.metadata_name == entry.method_name
                        && method.vtable_index == entry.vtable_index
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{}.{}::{}@{} is missing",
                        entry.declaring_namespace,
                        entry.declaring_interface,
                        entry.method_name,
                        entry.vtable_index
                    )
                });
            assert!(
                method.safe_array_contract_error.is_none(),
                "{}.{}::{}@{}: {:?}",
                entry.declaring_namespace,
                entry.declaring_interface,
                entry.method_name,
                entry.vtable_index,
                method.safe_array_contract_error
            );
            assert_eq!(
                method
                    .params
                    .get(entry.parameter_index)
                    .and_then(|param| param.safe_array_evidence.as_ref()),
                Some(entry),
                "{}.{}::{}@{} parameter {}",
                entry.declaring_namespace,
                entry.declaring_interface,
                entry.method_name,
                entry.vtable_index,
                entry.parameter_index
            );
        }

        let entry = &evidence[0];
        let interface = &interfaces[&(entry.declaring_namespace, entry.declaring_interface)];
        let mut drifted = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| {
                method.declaring_namespace == entry.declaring_namespace
                    && method.declaring_interface == entry.declaring_interface
                    && method.metadata_name == entry.method_name
                    && method.vtable_index == entry.vtable_index
            })
            .unwrap()
            .clone();
        drifted.params[entry.parameter_index].typ.native_type = RawNativeType::U32;
        drifted.params[entry.parameter_index].safe_array_evidence = None;
        drifted.safe_array_contract_error = None;
        apply_safe_array_evidence(&mut drifted);
        assert!(
            drifted.safe_array_contract_error.is_some(),
            "registered SAFEARRAY type drift must fail closed"
        );
    }

    fn interface(name: &str, iid: &str, base_chain: &[&str]) -> ComInterfaceMeta {
        ComInterfaceMeta {
            interface: InterfaceMeta {
                name: name.into(),
                namespace: "Tests".into(),
                iid: iid.into(),
                ..Default::default()
            },
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: base_chain.iter().map(|name| (*name).into()).collect(),
            base_iids: Vec::new(),
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
            raw_referenced_enums: None,
            raw_methods: None,
        }
    }

    #[test]
    fn coclass_primary_uses_inheritance_leaf_not_numeric_suffix() {
        let interfaces = vec![
            interface("IThing9", "1", &["IUnknown"]),
            interface("IThing", "2", &["IThing9", "IUnknown"]),
        ];

        let primary = select_primary_coclass_interface("Tests", "Thing", &interfaces).unwrap();
        assert_eq!(primary.interface.name, "IThing");
    }

    #[test]
    fn coclass_primary_rejects_unrelated_leaves() {
        let interfaces = vec![
            interface("IThing", "1", &["IUnknown"]),
            interface("IThing2", "2", &["IUnknown"]),
        ];

        let error = select_primary_coclass_interface("Tests", "Thing", &interfaces).unwrap_err();
        assert!(error.contains("multiple unrelated"));
    }

    #[test]
    fn in_out_is_com_only() {
        use windows_metadata::ParamAttributes;

        assert_eq!(
            classify_direction(ParamAttributes::In | ParamAttributes::Out, false),
            ParamDirection::InOut
        );
    }

    #[test]
    fn recognized_exact_contract_shape_drift_remains_fail_closed() {
        let Some(winmd) = std::env::var("DYNWINRT_WIN32_WINMD")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let mut interface = parse_com_interface(
            &winmd,
            "Windows.Win32.Media.MediaFoundation",
            "IMFAttributes",
        )
        .unwrap();
        let raw_index = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .position(|method| method.metadata_name == "GetBlob")
            .unwrap();
        let method_index = interface
            .interface
            .methods
            .iter()
            .position(|method| method.name == "GetBlob")
            .unwrap();
        let raw = &mut interface.raw_methods.as_mut().unwrap()[raw_index];
        raw.params.truncate(1);
        raw.exact_contract = None;
        let compatibility = &mut interface.interface.methods[method_index];
        apply_exact_method_contract(
            "Windows.Win32.Media.MediaFoundation",
            "IMFAttributes",
            "2cd2d921-c447-44a7-a13c-4adabfc247e3",
            compatibility,
            raw,
        );

        let contract = raw
            .exact_contract
            .as_ref()
            .expect("recognized method must retain fail-closed contract evidence");
        assert!(
            validate_exact_method_contract(
                "Windows.Win32.Media.MediaFoundation",
                "IMFAttributes",
                "2cd2d921-c447-44a7-a13c-4adabfc247e3",
                raw,
                contract,
            )
            .is_err()
        );
    }

    #[test]
    fn hstring_mapping_requires_the_canonical_namespace() {
        assert!(is_canonical_hstring_name(
            "Windows.Win32.System.WinRT",
            "HSTRING"
        ));
        assert!(!is_canonical_hstring_name("Contoso.Interop", "HSTRING"));
    }

    #[test]
    fn mixed_pointer_constness_fails_closed_as_unspecified() {
        let scalar = RawComType {
            native_type: RawNativeType::U8,
            underlying: None,
            pointer_depth: 0,
            constness: RawConstness::Unspecified,
        };
        let inner = add_raw_pointer(scalar, 1, RawConstness::Const);
        let mixed = add_raw_pointer(inner, 1, RawConstness::Mutable);

        assert_eq!(mixed.pointer_depth, 2);
        assert_eq!(mixed.constness, RawConstness::Mixed);
    }

    #[test]
    fn com_inheritance_requires_one_acyclic_bounded_base_chain() {
        let resolve = |graph: std::collections::BTreeMap<
            (&'static str, &'static str),
            Vec<(&'static str, &'static str, usize)>,
        >| {
            resolve_com_base_chain("Tests", "ILeaf", |namespace, name| {
                graph.get(&(namespace, name)).map(|bases| {
                    bases
                        .iter()
                        .map(|(namespace, name, count)| {
                            ((*namespace).into(), (*name).into(), *count)
                        })
                        .collect()
                })
            })
        };

        let valid = std::collections::BTreeMap::from([
            (("Tests", "ILeaf"), vec![("Tests", "IBase", 2)]),
            (
                ("Tests", "IBase"),
                vec![("Windows.Win32.System.Com", "IUnknown", 0)],
            ),
        ]);
        let (iunknown, slot, chain) = resolve(valid).unwrap();
        assert!(iunknown);
        assert_eq!(slot, 3);
        assert_eq!(chain[0].1, "IBase");

        let multiple = std::collections::BTreeMap::from([(
            ("Tests", "ILeaf"),
            vec![("Tests", "IBase", 2), ("Tests", "ISecond", 1)],
        )]);
        assert!(resolve(multiple).is_none());

        let cyclic = std::collections::BTreeMap::from([
            (("Tests", "ILeaf"), vec![("Tests", "IBase", 2)]),
            (("Tests", "IBase"), vec![("Tests", "ILeaf", 1)]),
        ]);
        assert!(resolve(cyclic).is_none());

        let mut deep = std::collections::BTreeMap::new();
        deep.insert(("Tests", "ILeaf"), vec![("Tests", "I0", 1)]);
        for index in 0..32 {
            let current: &'static str = Box::leak(format!("I{index}").into_boxed_str());
            let next: &'static str = Box::leak(format!("I{}", index + 1).into_boxed_str());
            deep.insert(("Tests", current), vec![("Tests", next, 1)]);
        }
        assert!(resolve(deep).is_none());
    }

    #[test]
    fn only_terminated_character_pointer_aliases_are_strings() {
        let named = |name: &str| RawComType {
            native_type: RawNativeType::Named {
                namespace: "Windows.Win32.Foundation".into(),
                name: name.into(),
                kind: RawNamedKind::Struct,
                iid: None,
                layout: None,
            },
            underlying: None,
            pointer_depth: 1,
            constness: RawConstness::Const,
        };
        for name in [
            "PWSTR", "PCWSTR", "LPWSTR", "LPCWSTR", "PSTR", "PCSTR", "LPSTR", "LPCSTR",
        ] {
            assert!(
                classify_raw_string_pointer_array(&named(name), RawParamDirection::In, None)
                    .is_some(),
                "{name}"
            );
        }
        for name in ["PWCHAR", "PCWCHAR", "LPWCH", "LPCWCH", "LPCH", "LPCCH"] {
            assert!(
                classify_raw_string_pointer_array(&named(name), RawParamDirection::In, None)
                    .is_none(),
                "{name}"
            );
        }
    }

    #[test]
    fn enumerator_contracts_require_exact_registered_identity() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        for (namespace, name) in [
            ("Windows.Win32.System.Com", "IEnumGUID"),
            ("Windows.Win32.System.Com", "IEnumConnectionPoints"),
            ("Windows.Win32.System.Ole", "IEnumVARIANT"),
            ("Windows.Win32.System.Com", "IEnumString"),
            ("Windows.Win32.System.Com", "IEnumUnknown"),
            ("Windows.Win32.Storage.VirtualDiskService", "IEnumVdsObject"),
            ("Windows.Win32.System.Com.Events", "IEnumEventObject"),
            ("Windows.Win32.UI.TextServices", "IEnumITfCompositionView"),
        ] {
            let interface = parse_com_interface(&winmd, namespace, name)
                .unwrap_or_else(|| panic!("{namespace}.{name} is missing"));
            let next = interface
                .raw_methods
                .as_ref()
                .unwrap()
                .iter()
                .find(|method| {
                    method.declaring_namespace == namespace
                        && method.declaring_interface == name
                        && method.metadata_name == "Next"
                })
                .unwrap_or_else(|| panic!("{namespace}.{name}::Next is missing"));
            assert!(
                next.enumerator_next.is_some(),
                "{namespace}.{name}: slot={}, element={:?}",
                next.vtable_index,
                next.params[1].typ
            );
        }

        let unknown =
            parse_com_interface(&winmd, "Windows.Win32.System.Com", "IEnumUnknown").unwrap();
        let next = unknown
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .find(|method| method.metadata_name == "Next")
            .unwrap();
        assert!(
            known_enumerator_next_override(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
                "Next",
                next.vtable_index,
                &next.params,
                &next.return_type,
            )
            .is_none()
        );
        assert!(
            known_enumerator_next_override(
                "Contoso",
                "IEnumUnknown",
                &unknown.interface.iid,
                "Next",
                next.vtable_index,
                &next.params,
                &next.return_type,
            )
            .is_none()
        );
        assert!(
            known_enumerator_next_override(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                &unknown.interface.iid,
                "Next",
                next.vtable_index + 1,
                &next.params,
                &next.return_type,
            )
            .is_none()
        );
        let mut element_drift = next.params.clone();
        {
            let RawNativeType::Named { name, .. } = &mut element_drift[1].typ.native_type else {
                panic!("IEnumUnknown must expose a named IUnknown element");
            };
            *name = "INotUnknown".into();
        }
        assert!(
            known_enumerator_next_override(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                &unknown.interface.iid,
                "Next",
                next.vtable_index,
                &element_drift,
                &next.return_type,
            )
            .is_none()
        );
        {
            let RawNativeType::Named { name, iid, .. } = &mut element_drift[1].typ.native_type
            else {
                unreachable!()
            };
            *name = "IUnknown".into();
            *iid = Some("ffffffff-ffff-ffff-ffff-ffffffffffff".into());
        }
        assert!(
            known_enumerator_next_override(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                &unknown.interface.iid,
                "Next",
                next.vtable_index,
                &element_drift,
                &next.return_type,
            )
            .is_none()
        );
        let mut signature_drift = next.params.clone();
        signature_drift[2].direction = RawParamDirection::In;
        assert!(
            known_enumerator_next_override(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                &unknown.interface.iid,
                "Next",
                next.vtable_index,
                &signature_drift,
                &next.return_type,
            )
            .is_none()
        );

        let mut identity_drift = next.clone();
        identity_drift.declaring_iid = "ffffffff-ffff-ffff-ffff-ffffffffffff".into();
        assert!(validate_attached_enumerator_evidence(&identity_drift).is_err());

        let mut slot_drift = next.clone();
        slot_drift.vtable_index += 1;
        assert!(validate_attached_enumerator_evidence(&slot_drift).is_err());

        let mut element_iid_drift = next.clone();
        let RawNativeType::Named { iid, .. } = &mut element_iid_drift.params[1].typ.native_type
        else {
            unreachable!()
        };
        *iid = Some("ffffffff-ffff-ffff-ffff-ffffffffffff".into());
        assert!(validate_attached_enumerator_evidence(&element_iid_drift).is_err());

        for (namespace, name) in [
            ("Windows.Win32.System.Wmi", "IEnumWbemClassObject"),
            ("Windows.Win32.System.WindowsSync", "IEnumItemIds"),
            ("Windows.Win32.Web.MsHtml", "IEnumPrivacyRecords"),
        ] {
            let unregistered = parse_com_interface(&winmd, namespace, name).unwrap();
            let next = unregistered
                .raw_methods
                .as_ref()
                .unwrap()
                .iter()
                .find(|method| method.declaring_interface == name && method.metadata_name == "Next")
                .unwrap();
            assert!(next.enumerator_next.is_none(), "{namespace}.{name}");
            assert!(
                validate_attached_enumerator_evidence(next).is_err(),
                "{namespace}.{name}"
            );
        }
    }

    #[test]
    fn registered_enumerator_contracts_match_win32_metadata() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let index = crate::meta::load_index(&winmd).unwrap();
        let mut failures = Vec::new();
        for contract in crate::com_enumerator_registry::contracts() {
            let Some(interface) = parse_com_interface_from_index(
                &index,
                contract.interface_namespace,
                contract.interface_name,
            ) else {
                failures.push(format!(
                    "{}.{} is missing",
                    contract.interface_namespace, contract.interface_name
                ));
                continue;
            };
            let next = interface
                .raw_methods
                .as_ref()
                .unwrap()
                .iter()
                .find(|method| {
                    method.declaring_namespace == contract.interface_namespace
                        && method.declaring_interface == contract.interface_name
                        && method.metadata_name == "Next"
                });
            match next {
                Some(next) if next.enumerator_next.is_some() => {}
                Some(next) => failures.push(format!(
                    "{}.{} slot={} element={:?}",
                    contract.interface_namespace,
                    contract.interface_name,
                    next.vtable_index,
                    next.params.get(1).map(|param| &param.typ)
                )),
                None => failures.push(format!(
                    "{}.{}::Next is missing",
                    contract.interface_namespace, contract.interface_name
                )),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn item_id_list_double_pointer_uses_cotaskmem_ownership() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.UI.Shell.Common",
                "ITEMIDLIST",
            )),
            2,
        );
        assert_eq!(
            known_free_with(
                "Windows.Win32.UI.Shell",
                "IShellLinkW",
                "GetIDList",
                &typ,
                &ParamDirection::Out
            )
            .as_deref(),
            Some("CoTaskMemFree")
        );
    }

    #[test]
    fn bstr_array_does_not_claim_scalar_sysfree_ownership() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.Foundation",
                "BSTR",
            )),
            2,
        );
        assert_eq!(
            known_free_with("", "", "", &typ, &ParamDirection::Out),
            None
        );
    }

    #[test]
    fn documented_shell_wide_string_outputs_use_cotaskmem() {
        let typ = windows_metadata::Type::PtrMut(
            Box::new(windows_metadata::Type::named(
                "Windows.Win32.Foundation",
                "PWSTR",
            )),
            1,
        );
        for (interface, method) in [
            ("IShellItem", "GetDisplayName"),
            ("IFileDialog", "GetFileName"),
        ] {
            assert_eq!(
                known_free_with(
                    "Windows.Win32.UI.Shell",
                    interface,
                    method,
                    &typ,
                    &ParamDirection::Out
                )
                .as_deref(),
                Some("CoTaskMemFree")
            );
        }
        assert_eq!(
            known_free_with(
                "Windows.Win32.System.Com",
                "IPersistFile",
                "GetCurFile",
                &typ,
                &ParamDirection::Out
            )
            .as_deref(),
            Some("CoTaskMemFree")
        );
    }

    #[test]
    fn documented_get_cur_file_hresult_is_semantic() {
        assert!(is_known_semantic_hresult(
            "Windows.Win32.System.Com",
            "IPersistFile",
            "GetCurFile"
        ));
        assert!(!is_known_semantic_hresult(
            "Windows.Win32.System.Com",
            "IPersistFile",
            "Load"
        ));
    }
}
