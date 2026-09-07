// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::num::NonZeroU8;

use super::diagnostics::{ModelError, UnsupportedReason};
use super::ids::{ComGuid, EnumId, LayoutId, SignatureId, TypeId};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::codegen::com) struct QualifiedName {
    namespace: String,
    name: String,
}

impl QualifiedName {
    pub(super) fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, ModelError> {
        let namespace = namespace.into();
        let name = name.into();
        if namespace.trim().is_empty() || name.trim().is_empty() {
            return Err(ModelError::InvalidName(format!("{namespace}.{name}")));
        }
        Ok(Self { namespace, name })
    }

    pub(in crate::codegen::com) fn namespace(&self) -> &str {
        &self.namespace
    }

    pub(in crate::codegen::com) fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum ScalarType {
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
    NativeIsize,
    NativeUsize,
    Win32Bool,
    HResult,
}

impl ScalarType {
    pub(super) const fn is_integer(self) -> bool {
        !matches!(self, Self::F32 | Self::F64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum Constness {
    Const,
    Mutable,
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum StringEncoding {
    Utf16,
    Ansi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallingConvention {
    System,
    Stdcall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct HandleKind {
    native_name: QualifiedName,
}

impl HandleKind {
    pub(super) const fn new(native_name: QualifiedName) -> Self {
        Self { native_name }
    }

    pub(in crate::codegen::com) const fn native_name(&self) -> &QualifiedName {
        &self.native_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum DataPointee {
    Typed(TypeId),
    Opaque(Option<QualifiedName>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum BufferElement {
    Typed(TypeId),
    Character(StringEncoding),
    StringPointer {
        encoding: StringEncoding,
        pointer_depth: NonZeroU8,
        constness: Constness,
    },
    Opaque(QualifiedName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum BufferElementOwnership {
    Plain,
    Borrowed,
    ComOwned,
    BstrOwned,
    VariantOwned,
    CoTaskMemStringOwned,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum ComAbiType {
    Scalar(ScalarType),
    Guid,
    Enum(EnumId),
    NativeStruct(LayoutId),
    NativeUnion(LayoutId),
    Pointer {
        pointee: TypeId,
        depth: NonZeroU8,
        constness: Constness,
    },
    Handle(HandleKind),
    DataPointer {
        pointee: DataPointee,
        depth: NonZeroU8,
        constness: Constness,
    },
    StringPointer {
        encoding: StringEncoding,
        constness: Constness,
    },
    ExactNullPointer,
    Bstr,
    HString,
    ComInterface {
        iid: ComGuid,
    },
    CountedBuffer {
        element: BufferElement,
        element_ownership: BufferElementOwnership,
        pointer_depth: NonZeroU8,
        constness: Constness,
    },
    SafeArray {
        element: Option<TypeId>,
    },
    Variant,
    PropVariant,
    DispatchParams,
    ExcepInfo,
    StatStg,
    FormatEtc,
    StgMedium,
    FunctionPointer(SignatureId),
    Unknown(UnsupportedReason),
}

impl ComAbiType {
    pub(super) fn requires_pointer_return_convention(&self) -> bool {
        matches!(
            self,
            Self::Pointer { .. }
                | Self::DataPointer { .. }
                | Self::StringPointer { .. }
                | Self::ExactNullPointer
                | Self::Bstr
                | Self::ComInterface { .. }
                | Self::CountedBuffer { .. }
                | Self::SafeArray { .. }
                | Self::FunctionPointer(_)
        )
    }

    pub(super) fn referenced_types(&self) -> impl Iterator<Item = TypeId> {
        let referenced = match self {
            Self::Pointer { pointee, .. } => Some(*pointee),
            Self::DataPointer {
                pointee: DataPointee::Typed(pointee),
                ..
            } => Some(*pointee),
            Self::CountedBuffer {
                element: BufferElement::Typed(element),
                ..
            }
            | Self::SafeArray {
                element: Some(element),
            } => Some(*element),
            _ => None,
        };
        referenced.into_iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct ComTypeDefinition {
    native_name: Option<QualifiedName>,
    underlying: Option<TypeId>,
    abi: ComAbiType,
}

impl ComTypeDefinition {
    pub(super) const fn new(
        native_name: Option<QualifiedName>,
        underlying: Option<TypeId>,
        abi: ComAbiType,
    ) -> Self {
        Self {
            native_name,
            underlying,
            abi,
        }
    }

    pub(in crate::codegen::com) const fn native_name(&self) -> Option<&QualifiedName> {
        self.native_name.as_ref()
    }

    pub(in crate::codegen::com) const fn underlying(&self) -> Option<TypeId> {
        self.underlying
    }

    pub(in crate::codegen::com) const fn abi(&self) -> &ComAbiType {
        &self.abi
    }

    pub(super) fn referenced_types(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.underlying
            .into_iter()
            .chain(self.abi.referenced_types())
    }
}

#[derive(Debug, Default)]
pub(super) struct ComTypeTable {
    definitions: Vec<Option<ComTypeDefinition>>,
}

impl ComTypeTable {
    pub(super) fn reserve(&mut self) -> Result<TypeId, ModelError> {
        let id = TypeId::from_index(self.definitions.len())
            .ok_or(ModelError::CapacityExceeded("COM type"))?;
        self.definitions.push(None);
        Ok(id)
    }

    pub(super) fn insert(&mut self, definition: ComTypeDefinition) -> Result<TypeId, ModelError> {
        let id = self.reserve()?;
        self.define(id, definition)?;
        Ok(id)
    }

    pub(super) fn define(
        &mut self,
        id: TypeId,
        definition: ComTypeDefinition,
    ) -> Result<(), ModelError> {
        let Some(slot) = self.definitions.get(id.index()) else {
            return Err(ModelError::UnknownId {
                kind: "COM type",
                index: id.index(),
            });
        };
        if slot.is_some() {
            return Err(ModelError::DuplicateDefinition {
                kind: "COM type",
                index: id.index(),
            });
        }
        for referenced in definition.referenced_types() {
            if referenced.index() >= self.definitions.len() {
                return Err(ModelError::UnknownId {
                    kind: "COM type",
                    index: referenced.index(),
                });
            }
        }
        self.definitions[id.index()] = Some(definition);
        Ok(())
    }

    pub(super) fn get(&self, id: TypeId) -> Result<&ComTypeDefinition, ModelError> {
        match self.definitions.get(id.index()) {
            Some(Some(definition)) => Ok(definition),
            Some(None) => Err(ModelError::IncompleteDefinition {
                kind: "COM type",
                index: id.index(),
            }),
            None => Err(ModelError::UnknownId {
                kind: "COM type",
                index: id.index(),
            }),
        }
    }

    pub(super) fn validate_complete(&self) -> Result<(), ModelError> {
        for (index, definition) in self.definitions.iter().enumerate() {
            if definition.is_none() {
                return Err(ModelError::IncompleteDefinition {
                    kind: "COM type",
                    index,
                });
            }
        }
        Ok(())
    }

    pub(super) fn ids(&self) -> impl Iterator<Item = TypeId> + '_ {
        (0..self.definitions.len()).filter_map(TypeId::from_index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) enum ComEnumValue {
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct ComEnumMember {
    name: String,
    value: ComEnumValue,
}

impl ComEnumMember {
    pub(super) fn new(name: String, value: ComEnumValue) -> Self {
        Self { name, value }
    }

    pub(in crate::codegen::com) fn name(&self) -> &str {
        &self.name
    }

    pub(in crate::codegen::com) const fn value(&self) -> &ComEnumValue {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct ComEnumDefinition {
    native_name: QualifiedName,
    underlying: ScalarType,
    members: Vec<ComEnumMember>,
    is_flags: bool,
}

impl ComEnumDefinition {
    pub(super) fn new(
        native_name: QualifiedName,
        underlying: ScalarType,
    ) -> Result<Self, ModelError> {
        if !underlying.is_integer() {
            return Err(ModelError::InvalidContract(format!(
                "enum {}.{} cannot use non-integer underlying type {underlying:?}",
                native_name.namespace(),
                native_name.name()
            )));
        }
        Ok(Self {
            native_name,
            underlying,
            members: Vec::new(),
            is_flags: false,
        })
    }

    pub(super) fn set_members(&mut self, members: Vec<ComEnumMember>, is_flags: bool) {
        self.members = members;
        self.is_flags = is_flags;
    }

    pub(in crate::codegen::com) const fn native_name(&self) -> &QualifiedName {
        &self.native_name
    }

    pub(in crate::codegen::com) const fn underlying(&self) -> ScalarType {
        self.underlying
    }

    pub(in crate::codegen::com) fn members(&self) -> &[ComEnumMember] {
        &self.members
    }

    pub(in crate::codegen::com) const fn is_flags(&self) -> bool {
        self.is_flags
    }
}

#[derive(Debug, Default)]
pub(super) struct EnumTable {
    definitions: Vec<ComEnumDefinition>,
}

impl EnumTable {
    pub(super) fn insert(&mut self, definition: ComEnumDefinition) -> Result<EnumId, ModelError> {
        let id = EnumId::from_index(self.definitions.len())
            .ok_or(ModelError::CapacityExceeded("enum"))?;
        self.definitions.push(definition);
        Ok(id)
    }

    pub(super) fn get(&self, id: EnumId) -> Result<&ComEnumDefinition, ModelError> {
        self.definitions
            .get(id.index())
            .ok_or(ModelError::UnknownId {
                kind: "enum",
                index: id.index(),
            })
    }

    pub(super) fn get_mut(&mut self, id: EnumId) -> Result<&mut ComEnumDefinition, ModelError> {
        self.definitions
            .get_mut(id.index())
            .ok_or(ModelError::UnknownId {
                kind: "enum",
                index: id.index(),
            })
    }

    pub(super) fn ids(&self) -> impl Iterator<Item = EnumId> + '_ {
        (0..self.definitions.len()).filter_map(EnumId::from_index)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &ComEnumDefinition> {
        self.definitions.iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FunctionReturn {
    Void,
    Value(TypeId),
    Pointer(TypeId),
}

impl FunctionReturn {
    pub(super) const fn abi_type(self) -> Option<TypeId> {
        match self {
            Self::Void => None,
            Self::Value(abi_type) | Self::Pointer(abi_type) => Some(abi_type),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FunctionSignature {
    calling_convention: CallingConvention,
    params: Vec<TypeId>,
    return_kind: FunctionReturn,
}

impl FunctionSignature {
    pub(super) const fn new(
        calling_convention: CallingConvention,
        params: Vec<TypeId>,
        return_kind: FunctionReturn,
    ) -> Self {
        Self {
            calling_convention,
            params,
            return_kind,
        }
    }

    pub(super) const fn calling_convention(&self) -> CallingConvention {
        self.calling_convention
    }

    pub(super) fn params(&self) -> &[TypeId] {
        &self.params
    }

    pub(super) const fn return_kind(&self) -> FunctionReturn {
        self.return_kind
    }
}

#[derive(Debug, Default)]
pub(super) struct SignatureTable {
    definitions: Vec<Option<FunctionSignature>>,
}

impl SignatureTable {
    pub(super) fn reserve(&mut self) -> Result<SignatureId, ModelError> {
        let id = SignatureId::from_index(self.definitions.len())
            .ok_or(ModelError::CapacityExceeded("function signature"))?;
        self.definitions.push(None);
        Ok(id)
    }

    pub(super) fn insert(
        &mut self,
        definition: FunctionSignature,
    ) -> Result<SignatureId, ModelError> {
        let id = self.reserve()?;
        self.define(id, definition)?;
        Ok(id)
    }

    pub(super) fn define(
        &mut self,
        id: SignatureId,
        definition: FunctionSignature,
    ) -> Result<(), ModelError> {
        let Some(slot) = self.definitions.get_mut(id.index()) else {
            return Err(ModelError::UnknownId {
                kind: "function signature",
                index: id.index(),
            });
        };
        if slot.is_some() {
            return Err(ModelError::DuplicateDefinition {
                kind: "function signature",
                index: id.index(),
            });
        }
        *slot = Some(definition);
        Ok(())
    }

    pub(super) fn get(&self, id: SignatureId) -> Result<&FunctionSignature, ModelError> {
        match self.definitions.get(id.index()) {
            Some(Some(definition)) => Ok(definition),
            Some(None) => Err(ModelError::IncompleteDefinition {
                kind: "function signature",
                index: id.index(),
            }),
            None => Err(ModelError::UnknownId {
                kind: "function signature",
                index: id.index(),
            }),
        }
    }

    pub(super) fn validate_complete(&self) -> Result<(), ModelError> {
        for (index, definition) in self.definitions.iter().enumerate() {
            if definition.is_none() {
                return Err(ModelError::IncompleteDefinition {
                    kind: "function signature",
                    index,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::*;

    fn unnamed(abi: ComAbiType) -> ComTypeDefinition {
        ComTypeDefinition::new(None, None, abi)
    }

    #[test]
    fn reserved_ids_support_recursive_pointer_graphs() {
        let mut table = ComTypeTable::default();
        let node = table.reserve().unwrap();
        let node_pointer = table
            .insert(unnamed(ComAbiType::Pointer {
                pointee: node,
                depth: NonZeroU8::new(1).unwrap(),
                constness: Constness::Mutable,
            }))
            .unwrap();
        table
            .define(
                node,
                ComTypeDefinition::new(
                    Some(QualifiedName::new("Example", "NODE").unwrap()),
                    None,
                    ComAbiType::DataPointer {
                        pointee: DataPointee::Typed(node_pointer),
                        depth: NonZeroU8::new(1).unwrap(),
                        constness: Constness::Mutable,
                    },
                ),
            )
            .unwrap();

        table.validate_complete().unwrap();
        assert!(table.get(node).is_ok());
    }

    #[test]
    fn unknown_nested_types_fail_closed() {
        let mut table = ComTypeTable::default();
        let unknown = table
            .insert(unnamed(ComAbiType::Unknown(
                UnsupportedReason::UnknownPointerMeaning,
            )))
            .unwrap();
        let buffer = table
            .insert(unnamed(ComAbiType::CountedBuffer {
                element: BufferElement::Typed(unknown),
                element_ownership: BufferElementOwnership::Unknown,
                pointer_depth: NonZeroU8::new(1).unwrap(),
                constness: Constness::Const,
            }))
            .unwrap();

        assert_eq!(
            table.get(buffer).unwrap().abi(),
            &ComAbiType::CountedBuffer {
                element: BufferElement::Typed(unknown),
                element_ownership: BufferElementOwnership::Unknown,
                pointer_depth: NonZeroU8::new(1).unwrap(),
                constness: Constness::Const,
            }
        );
    }

    #[test]
    fn aliases_preserve_native_and_underlying_identity() {
        let mut table = ComTypeTable::default();
        let u32_type = table
            .insert(unnamed(ComAbiType::Scalar(ScalarType::U32)))
            .unwrap();
        let colorref = table
            .insert(ComTypeDefinition::new(
                Some(QualifiedName::new("Windows.Win32.Foundation", "COLORREF").unwrap()),
                Some(u32_type),
                ComAbiType::Scalar(ScalarType::U32),
            ))
            .unwrap();
        let definition = table.get(colorref).unwrap();

        assert_eq!(definition.underlying(), Some(u32_type));
        assert_eq!(definition.native_name().unwrap().name(), "COLORREF");
        assert_eq!(
            definition.native_name().unwrap().namespace(),
            "Windows.Win32.Foundation"
        );
    }

    #[test]
    fn direct_pointer_convention_is_semantic_not_pointer_width() {
        assert!(
            ComAbiType::ComInterface {
                iid: ComGuid::from_bytes([1; 16])
            }
            .requires_pointer_return_convention()
        );
        assert!(
            !ComAbiType::Handle(HandleKind::new(
                QualifiedName::new("Windows.Win32.Foundation", "HWND").unwrap()
            ))
            .requires_pointer_return_convention()
        );
        assert!(
            !ComAbiType::HString.requires_pointer_return_convention(),
            "opaque handles are direct values even when pointer-width"
        );
    }
}
