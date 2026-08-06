// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::num::NonZeroU32;

use super::diagnostics::ModelError;
use super::ids::{LayoutId, TypeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::codegen::com) enum Architecture {
    X86,
    X64,
    Arm64,
}

impl Architecture {
    pub(super) const ALL: [Self; 3] = [Self::X86, Self::X64, Self::Arm64];

    pub(super) const fn metadata_mask(self) -> u8 {
        match self {
            Self::X86 => 0b001,
            Self::X64 => 0b010,
            Self::Arm64 => 0b100,
        }
    }

    pub(super) const fn pointer_size(self) -> usize {
        match self {
            Self::X86 => 4,
            Self::X64 | Self::Arm64 => 8,
        }
    }

    pub(super) const fn pointer_alignment(self) -> usize {
        self.pointer_size()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum StructLayout {
    Sequential,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::codegen::com) enum LayoutKind {
    Struct(StructLayout),
    Union,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct FieldLayout {
    name: String,
    abi_type: TypeId,
    offset: usize,
    fixed_count: Option<NonZeroU32>,
}

impl FieldLayout {
    pub(super) fn new(
        name: impl Into<String>,
        abi_type: TypeId,
        offset: usize,
        fixed_count: Option<NonZeroU32>,
    ) -> Result<Self, ModelError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ModelError::InvalidLayout(
                "field name must not be empty".into(),
            ));
        }
        Ok(Self {
            name,
            abi_type,
            offset,
            fixed_count,
        })
    }

    pub(in crate::codegen::com) fn name(&self) -> &str {
        &self.name
    }

    pub(in crate::codegen::com) const fn abi_type(&self) -> TypeId {
        self.abi_type
    }

    pub(in crate::codegen::com) const fn offset(&self) -> usize {
        self.offset
    }

    pub(in crate::codegen::com) const fn fixed_count(&self) -> Option<NonZeroU32> {
        self.fixed_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct NativeLayout {
    size: usize,
    alignment: usize,
    packing: usize,
    kind: LayoutKind,
    fields: Vec<FieldLayout>,
}

impl NativeLayout {
    pub(super) fn new(
        size: usize,
        alignment: usize,
        packing: usize,
        kind: LayoutKind,
        fields: Vec<FieldLayout>,
    ) -> Result<Self, ModelError> {
        if size == 0 {
            return Err(ModelError::InvalidLayout(
                "layout size must be non-zero".into(),
            ));
        }
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(ModelError::InvalidLayout(
                "alignment must be a non-zero power of two".into(),
            ));
        }
        if packing == 0 || !packing.is_power_of_two() {
            return Err(ModelError::InvalidLayout(
                "packing must be a non-zero power of two".into(),
            ));
        }
        if size % alignment != 0 {
            return Err(ModelError::InvalidLayout(format!(
                "{size}-byte layout is not a multiple of alignment {alignment}"
            )));
        }
        let mut names = HashSet::new();
        for field in &fields {
            if !names.insert(field.name()) {
                return Err(ModelError::InvalidLayout(format!(
                    "duplicate field `{}`",
                    field.name()
                )));
            }
            if field.offset() >= size {
                return Err(ModelError::InvalidLayout(format!(
                    "field `{}` starts outside the {size}-byte layout",
                    field.name()
                )));
            }
            if kind == LayoutKind::Union && field.offset() != 0 {
                return Err(ModelError::InvalidLayout(format!(
                    "union field `{}` must start at offset zero",
                    field.name()
                )));
            }
        }
        Ok(Self {
            size,
            alignment,
            packing,
            kind,
            fields,
        })
    }

    pub(in crate::codegen::com) const fn size(&self) -> usize {
        self.size
    }

    pub(in crate::codegen::com) const fn alignment(&self) -> usize {
        self.alignment
    }

    pub(in crate::codegen::com) const fn packing(&self) -> usize {
        self.packing
    }

    pub(in crate::codegen::com) const fn kind(&self) -> LayoutKind {
        self.kind
    }

    pub(in crate::codegen::com) fn fields(&self) -> &[FieldLayout] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::codegen::com) struct NativeLayoutSet {
    x86: NativeLayout,
    x64: NativeLayout,
    arm64: NativeLayout,
}

impl NativeLayoutSet {
    pub(super) const fn new(x86: NativeLayout, x64: NativeLayout, arm64: NativeLayout) -> Self {
        Self { x86, x64, arm64 }
    }

    pub(in crate::codegen::com) const fn get(&self, architecture: Architecture) -> &NativeLayout {
        match architecture {
            Architecture::X86 => &self.x86,
            Architecture::X64 => &self.x64,
            Architecture::Arm64 => &self.arm64,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct LayoutTable {
    layouts: Vec<Option<NativeLayoutSet>>,
}

impl LayoutTable {
    pub(super) fn reserve(&mut self) -> Result<LayoutId, ModelError> {
        let id = LayoutId::from_index(self.layouts.len())
            .ok_or(ModelError::CapacityExceeded("native layout"))?;
        self.layouts.push(None);
        Ok(id)
    }

    pub(super) fn insert(&mut self, layout: NativeLayoutSet) -> Result<LayoutId, ModelError> {
        let id = self.reserve()?;
        self.define(id, layout)?;
        Ok(id)
    }

    pub(super) fn define(
        &mut self,
        id: LayoutId,
        layout: NativeLayoutSet,
    ) -> Result<(), ModelError> {
        let Some(slot) = self.layouts.get_mut(id.index()) else {
            return Err(ModelError::UnknownId {
                kind: "native layout",
                index: id.index(),
            });
        };
        if slot.is_some() {
            return Err(ModelError::DuplicateDefinition {
                kind: "native layout",
                index: id.index(),
            });
        }
        *slot = Some(layout);
        Ok(())
    }

    pub(super) fn get(&self, id: LayoutId) -> Result<&NativeLayoutSet, ModelError> {
        match self.layouts.get(id.index()) {
            Some(Some(layout)) => Ok(layout),
            Some(None) => Err(ModelError::IncompleteDefinition {
                kind: "native layout",
                index: id.index(),
            }),
            None => Err(ModelError::UnknownId {
                kind: "native layout",
                index: id.index(),
            }),
        }
    }

    pub(super) fn validate_complete(&self) -> Result<(), ModelError> {
        for (index, layout) in self.layouts.iter().enumerate() {
            if layout.is_none() {
                return Err(ModelError::IncompleteDefinition {
                    kind: "native layout",
                    index,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_field(offset: usize) -> FieldLayout {
        FieldLayout::new("value", TypeId::from_index(0).unwrap(), offset, None).unwrap()
    }

    fn layout(size: usize, architecture: Architecture) -> NativeLayout {
        NativeLayout::new(
            size,
            architecture.pointer_alignment().min(4),
            8,
            LayoutKind::Struct(StructLayout::Sequential),
            vec![scalar_field(0)],
        )
        .unwrap()
    }

    #[test]
    fn architecture_has_explicit_pointer_width() {
        assert_eq!(Architecture::X86.pointer_size(), 4);
        assert_eq!(Architecture::X64.pointer_alignment(), 8);
        assert_eq!(Architecture::Arm64.pointer_size(), 8);
    }

    #[test]
    fn union_fields_must_overlap_at_zero() {
        let field = FieldLayout::new("value", TypeId::from_index(0).unwrap(), 4, None).unwrap();
        assert!(matches!(
            NativeLayout::new(8, 8, 8, LayoutKind::Union, vec![field]),
            Err(ModelError::InvalidLayout(_))
        ));
    }

    #[test]
    fn layout_table_preserves_all_architecture_variants() {
        let mut table = LayoutTable::default();
        let id = table.reserve().unwrap();
        table
            .define(
                id,
                NativeLayoutSet::new(
                    layout(4, Architecture::X86),
                    layout(8, Architecture::X64),
                    layout(8, Architecture::Arm64),
                ),
            )
            .unwrap();

        assert_eq!(table.get(id).unwrap().get(Architecture::X86).size(), 4);
        assert_eq!(table.get(id).unwrap().get(Architecture::X64).size(), 8);
        assert_eq!(table.get(id).unwrap().get(Architecture::Arm64).size(), 8);
    }
}
