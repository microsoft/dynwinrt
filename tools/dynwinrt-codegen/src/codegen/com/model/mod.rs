// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Classic COM ABI semantics.
//!
//! This model is independent from both WinRT metadata and language
//! projections. Metadata adapters must fully describe a contract here before
//! it can be lowered to a native call or rendered for JavaScript.

// The closed model intentionally includes categories that later rewrite
// phases will wire into production one at a time.
#![allow(dead_code)]

pub(super) mod abi;
pub(super) mod contract;
pub(super) mod diagnostics;
pub(super) mod ids;
pub(super) mod layout;
pub(super) mod metadata;
pub(super) mod method;
pub(super) mod ownership;

pub(super) use metadata::SemanticComInterface;

use std::collections::HashSet;

use abi::{ComAbiType, ComTypeTable, Constness, EnumTable, FunctionReturn, SignatureTable};
use diagnostics::{ModelError, UnsupportedReason};
use ids::TypeId;
use layout::{Architecture, LayoutKind, LayoutTable};
use ownership::CleanupTable;

pub(super) struct ValidatedComInterface<'a> {
    meta: &'a crate::com_metadata::ComInterfaceMeta,
    semantic: metadata::SemanticComInterface,
    evidence_dependencies: crate::contract_registry::EvidenceDependencies,
}

impl<'a> ValidatedComInterface<'a> {
    pub(super) const fn metadata(&self) -> &'a crate::com_metadata::ComInterfaceMeta {
        self.meta
    }

    pub(super) const fn semantic(&self) -> &metadata::SemanticComInterface {
        &self.semantic
    }

    pub(super) const fn evidence_dependencies(
        &self,
    ) -> &crate::contract_registry::EvidenceDependencies {
        &self.evidence_dependencies
    }
}

pub(super) struct ValidatedComCoclass<'a> {
    meta: &'a crate::com_metadata::ComCoclassMeta,
    clsid: ids::ComGuid,
}

impl<'a> ValidatedComCoclass<'a> {
    pub(super) const fn metadata(&self) -> &'a crate::com_metadata::ComCoclassMeta {
        self.meta
    }

    pub(super) const fn clsid(&self) -> ids::ComGuid {
        self.clsid
    }
}

pub(super) fn validate_coclass(
    meta: &crate::com_metadata::ComCoclassMeta,
) -> Result<ValidatedComCoclass<'_>, String> {
    let clsid = ids::ComGuid::parse(&meta.clsid).map_err(|error| error.to_string())?;
    if clsid.is_zero() {
        return Err(format!("{}.{} has a zero CLSID", meta.namespace, meta.name));
    }
    Ok(ValidatedComCoclass { meta, clsid })
}

pub(super) fn validate_interface(
    meta: &crate::com_metadata::ComInterfaceMeta,
) -> Result<ValidatedComInterface<'_>, String> {
    let semantic = metadata::map_interface(meta).map_err(|error| error.to_string())?;
    let evidence_dependencies = crate::com_metadata::collect_evidence_dependencies(meta);
    Ok(ValidatedComInterface {
        meta,
        semantic,
        evidence_dependencies,
    })
}

#[derive(Debug, Default)]
struct ComModel {
    types: ComTypeTable,
    layouts: LayoutTable,
    enums: EnumTable,
    signatures: SignatureTable,
    cleanups: CleanupTable,
}

impl ComModel {
    const fn types(&self) -> &ComTypeTable {
        &self.types
    }

    const fn types_mut(&mut self) -> &mut ComTypeTable {
        &mut self.types
    }

    const fn layouts_mut(&mut self) -> &mut LayoutTable {
        &mut self.layouts
    }

    const fn enums_mut(&mut self) -> &mut EnumTable {
        &mut self.enums
    }

    const fn signatures_mut(&mut self) -> &mut SignatureTable {
        &mut self.signatures
    }

    const fn cleanups(&self) -> &CleanupTable {
        &self.cleanups
    }

    const fn cleanups_mut(&mut self) -> &mut CleanupTable {
        &mut self.cleanups
    }

    fn validate_complete(&self) -> Result<(), ModelError> {
        self.types.validate_complete()?;
        self.layouts.validate_complete()?;
        self.signatures.validate_complete()?;
        for id in self.types.ids() {
            self.require_supported_type(id)?;
        }
        Ok(())
    }

    fn require_supported_type(&self, root: TypeId) -> Result<(), ModelError> {
        self.validate_underlying_chain(root)?;
        let mut visited = HashSet::new();
        self.validate_type_graph(root, &mut visited)
    }

    fn validate_underlying_chain(&self, root: TypeId) -> Result<(), ModelError> {
        let mut current = Some(root);
        let mut visited = HashSet::new();
        while let Some(id) = current {
            if !visited.insert(id) {
                return Err(ModelError::InvalidContract(format!(
                    "COM type {} has a cyclic underlying-type chain",
                    root.index()
                )));
            }
            current = self.types.get(id)?.underlying();
        }
        Ok(())
    }

    fn validate_type_graph(
        &self,
        id: TypeId,
        visited: &mut HashSet<TypeId>,
    ) -> Result<(), ModelError> {
        if !visited.insert(id) {
            return Ok(());
        }
        let definition = self.types.get(id)?;
        match definition.abi() {
            ComAbiType::Unknown(reason) => {
                return Err(ModelError::Unsupported(reason.clone()));
            }
            ComAbiType::ExactNullPointer => {}
            ComAbiType::Pointer {
                constness: Constness::Unspecified,
                ..
            }
            | ComAbiType::DataPointer {
                constness: Constness::Unspecified,
                ..
            }
            | ComAbiType::StringPointer {
                constness: Constness::Unspecified,
                ..
            }
            | ComAbiType::CountedBuffer {
                constness: Constness::Unspecified,
                ..
            } => {
                return Err(ModelError::Unsupported(
                    UnsupportedReason::UnknownPointerMeaning,
                ));
            }
            ComAbiType::ComInterface { iid } if iid.is_zero() => {
                return Err(ModelError::Unsupported(
                    UnsupportedReason::MissingInterfaceIid,
                ));
            }
            ComAbiType::SafeArray { element: None } => {
                return Err(ModelError::Unsupported(
                    UnsupportedReason::UnsupportedSafeArrayElement,
                ));
            }
            ComAbiType::Enum(enum_id) => {
                self.enums.get(*enum_id)?;
            }
            ComAbiType::NativeStruct(layout_id) => {
                let layouts = self.layouts.get(*layout_id)?;
                for architecture in Architecture::ALL {
                    let layout = layouts.get(architecture);
                    if !matches!(layout.kind(), LayoutKind::Struct(_)) {
                        return Err(ModelError::InvalidLayout(format!(
                            "COM struct type {} references a union layout",
                            id.index()
                        )));
                    }
                    for field in layout.fields() {
                        self.validate_type_graph(field.abi_type(), visited)?;
                    }
                }
            }
            ComAbiType::NativeUnion(layout_id) => {
                let layouts = self.layouts.get(*layout_id)?;
                for architecture in Architecture::ALL {
                    let layout = layouts.get(architecture);
                    if layout.kind() != LayoutKind::Union {
                        return Err(ModelError::InvalidLayout(format!(
                            "COM union type {} references a struct layout",
                            id.index()
                        )));
                    }
                    for field in layout.fields() {
                        self.validate_type_graph(field.abi_type(), visited)?;
                    }
                }
            }
            ComAbiType::FunctionPointer(signature_id) => {
                let signature = self.signatures.get(*signature_id)?;
                for param in signature.params() {
                    self.validate_type_graph(*param, visited)?;
                }
                if let Some(return_type) = signature.return_kind().abi_type() {
                    self.validate_type_graph(return_type, visited)?;
                    let pointer_return = self
                        .types
                        .get(return_type)?
                        .abi()
                        .requires_pointer_return_convention();
                    match signature.return_kind() {
                        FunctionReturn::Pointer(_) if !pointer_return => {
                            return Err(ModelError::InvalidContract(
                                "function pointer uses a non-pointer direct return".into(),
                            ));
                        }
                        FunctionReturn::Value(_) if pointer_return => {
                            return Err(ModelError::InvalidContract(
                                "function pointer uses a pointer ABI as a direct value".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        for referenced in definition.referenced_types() {
            self.validate_type_graph(referenced, visited)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::*;
    use abi::{
        CallingConvention, ComEnumDefinition, ComTypeDefinition, FunctionSignature, QualifiedName,
        ScalarType,
    };
    use ids::{ComGuid, EnumId, LayoutId, SignatureId};

    fn unnamed(abi: ComAbiType) -> ComTypeDefinition {
        ComTypeDefinition::new(None, None, abi)
    }

    #[test]
    fn unresolved_cross_table_ids_fail_closed() {
        let mut enum_model = ComModel::default();
        let enum_type = enum_model
            .types_mut()
            .insert(unnamed(ComAbiType::Enum(EnumId::from_index(0).unwrap())))
            .unwrap();
        assert!(matches!(
            enum_model.require_supported_type(enum_type),
            Err(ModelError::UnknownId { kind: "enum", .. })
        ));

        let mut layout_model = ComModel::default();
        let struct_type = layout_model
            .types_mut()
            .insert(unnamed(ComAbiType::NativeStruct(
                LayoutId::from_index(0).unwrap(),
            )))
            .unwrap();
        assert!(matches!(
            layout_model.require_supported_type(struct_type),
            Err(ModelError::UnknownId {
                kind: "native layout",
                ..
            })
        ));

        let mut signature_model = ComModel::default();
        let callback = signature_model
            .types_mut()
            .insert(unnamed(ComAbiType::FunctionPointer(
                SignatureId::from_index(0).unwrap(),
            )))
            .unwrap();
        assert!(matches!(
            signature_model.require_supported_type(callback),
            Err(ModelError::UnknownId {
                kind: "function signature",
                ..
            })
        ));
    }

    #[test]
    fn complete_enum_and_function_pointer_graphs_validate() {
        let mut model = ComModel::default();
        let u32_type = model
            .types_mut()
            .insert(unnamed(ComAbiType::Scalar(ScalarType::U32)))
            .unwrap();
        let enum_id = model
            .enums_mut()
            .insert(
                ComEnumDefinition::new(
                    QualifiedName::new("Example", "FLAGS").unwrap(),
                    ScalarType::U32,
                )
                .unwrap(),
            )
            .unwrap();
        let enum_type = model
            .types_mut()
            .insert(unnamed(ComAbiType::Enum(enum_id)))
            .unwrap();
        let signature = model
            .signatures_mut()
            .insert(FunctionSignature::new(
                CallingConvention::System,
                vec![enum_type],
                FunctionReturn::Value(u32_type),
            ))
            .unwrap();
        let callback = model
            .types_mut()
            .insert(unnamed(ComAbiType::FunctionPointer(signature)))
            .unwrap();

        model.validate_complete().unwrap();
        model.require_supported_type(callback).unwrap();
    }

    #[test]
    fn incomplete_pointer_and_interface_facts_fail_closed() {
        let mut model = ComModel::default();
        let byte = model
            .types_mut()
            .insert(unnamed(ComAbiType::Scalar(ScalarType::U8)))
            .unwrap();
        let pointer = model
            .types_mut()
            .insert(unnamed(ComAbiType::Pointer {
                pointee: byte,
                depth: NonZeroU8::new(1).unwrap(),
                constness: Constness::Unspecified,
            }))
            .unwrap();
        let interface = model
            .types_mut()
            .insert(unnamed(ComAbiType::ComInterface { iid: ComGuid::ZERO }))
            .unwrap();

        assert_eq!(
            model.require_supported_type(pointer),
            Err(ModelError::Unsupported(
                UnsupportedReason::UnknownPointerMeaning
            ))
        );
        assert_eq!(
            model.require_supported_type(interface),
            Err(ModelError::Unsupported(
                UnsupportedReason::MissingInterfaceIid
            ))
        );
    }
}
