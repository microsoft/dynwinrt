// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows_core::{HSTRING, IUnknown, Interface};

use crate::{Error, Result, TypeHandle, TypeKind, WinRTValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CollectionStorage {
    Word(usize),
    HString,
    Object,
    EmptyOnly(usize),
}

impl CollectionStorage {
    pub(crate) fn is_value_type(self) -> bool {
        matches!(self, Self::Word(_) | Self::EmptyOnly(_))
    }

    pub(crate) fn is_hstring(self) -> bool {
        self == Self::HString
    }

    pub(crate) fn is_empty_only(self) -> bool {
        matches!(self, Self::EmptyOnly(_))
    }

    pub(crate) fn element_size(self) -> usize {
        match self {
            Self::Word(size) | Self::EmptyOnly(size) => size,
            Self::HString | Self::Object => size_of::<usize>(),
        }
    }

    pub(crate) fn array_stride(self) -> usize {
        self.element_size()
    }

    pub(crate) fn raw_value(size: usize, empty: bool) -> Self {
        assert!(
            size > 0,
            "create_value_vector: element size must be nonzero"
        );
        // The unsafe byte constructor's caller must exclude floating aggregates.
        aggregate_storage(CollectionTarget::current(), size, false, empty)
            .expect("create_value_vector: element size has no supported collection ABI")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionTarget {
    X86,
    X64,
    Arm64,
    Unsupported,
}

impl CollectionTarget {
    fn current() -> Self {
        match std::env::consts::ARCH {
            "x86" => Self::X86,
            "x86_64" => Self::X64,
            "aarch64" => Self::Arm64,
            _ => Self::Unsupported,
        }
    }

    fn word_size(self) -> usize {
        match self {
            Self::X86 => 4,
            Self::X64 | Self::Arm64 => 8,
            Self::Unsupported => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArgumentAbi {
    DirectWord,
    IndirectPointer,
    MultipleWords,
    FloatingAggregate,
    Unsupported,
}

// Windows ARM64 classifies HFAs before applying the >16-byte indirect rule.
fn aggregate_abi(target: CollectionTarget, size: usize, hfa: bool) -> ArgumentAbi {
    if size == 0 {
        return ArgumentAbi::Unsupported;
    }
    match target {
        CollectionTarget::X86 if size <= 4 => ArgumentAbi::DirectWord,
        CollectionTarget::X86 => ArgumentAbi::MultipleWords,
        CollectionTarget::X64 if matches!(size, 1 | 2 | 4 | 8) => ArgumentAbi::DirectWord,
        CollectionTarget::X64 => ArgumentAbi::IndirectPointer,
        CollectionTarget::Arm64 if hfa => ArgumentAbi::FloatingAggregate,
        CollectionTarget::Arm64 if size <= 8 => ArgumentAbi::DirectWord,
        CollectionTarget::Arm64 if size <= 16 => ArgumentAbi::MultipleWords,
        CollectionTarget::Arm64 => ArgumentAbi::IndirectPointer,
        CollectionTarget::Unsupported => ArgumentAbi::Unsupported,
    }
}

fn aggregate_storage(
    target: CollectionTarget,
    size: usize,
    hfa: bool,
    allow_empty: bool,
) -> Option<CollectionStorage> {
    match aggregate_abi(target, size, hfa) {
        ArgumentAbi::DirectWord => Some(CollectionStorage::Word(size)),
        ArgumentAbi::IndirectPointer if allow_empty && size > target.word_size() => {
            Some(CollectionStorage::EmptyOnly(size))
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct PodLayout {
    size: usize,
    align: usize,
    homogeneous: Option<(TypeKind, usize)>,
}

fn pod_layout(typ: &TypeHandle, depth: usize) -> Result<PodLayout> {
    let unsupported = || Error::UnsupportedCollectionElement(typ.kind());
    if depth > 64 {
        return Err(unsupported());
    }
    if let Some(size) = typ.kind().primitive_size() {
        return Ok(PodLayout {
            size,
            align: typ.align_of(),
            homogeneous: match typ.kind() {
                TypeKind::F32 | TypeKind::F64 => Some((typ.kind(), 1)),
                _ => None,
            },
        });
    }
    if !matches!(typ.kind(), TypeKind::Struct(_)) || typ.field_count() == 0 {
        return Err(unsupported());
    }
    let mut end = 0usize;
    let mut align = 1;
    let mut homogeneous = None;
    for index in 0..typ.field_count() {
        let field = pod_layout(&typ.field_type(index), depth + 1)?;
        let offset = end
            .checked_next_multiple_of(field.align)
            .ok_or_else(unsupported)?;
        if typ.field_offset(index) != offset {
            return Err(unsupported());
        }
        end = offset.checked_add(field.size).ok_or_else(unsupported)?;
        align = align.max(field.align);
        homogeneous = if index == 0 {
            field.homogeneous
        } else {
            match (homogeneous, field.homogeneous) {
                (Some((left, count)), Some((right, added))) if left == right => {
                    Some((left, count.saturating_add(added)))
                }
                _ => None,
            }
        };
    }
    let size = end
        .checked_next_multiple_of(align)
        .ok_or_else(unsupported)?;
    if size == 0 || size != typ.size_of() || align != typ.align_of() {
        return Err(unsupported());
    }
    let homogeneous = homogeneous.filter(|(kind, count)| {
        (1..=4).contains(count)
            && kind
                .primitive_size()
                .and_then(|width| width.checked_mul(*count))
                == Some(size)
    });
    Ok(PodLayout {
        size,
        align,
        homogeneous,
    })
}

pub(crate) struct CollectionElementPlan {
    typ: TypeHandle,
    pub(crate) storage: CollectionStorage,
}

impl CollectionElementPlan {
    pub(crate) fn new(typ: &TypeHandle, allow_empty: bool) -> Result<Self> {
        Self::for_target(typ, allow_empty, CollectionTarget::current())
    }

    fn for_target(typ: &TypeHandle, allow_empty: bool, target: CollectionTarget) -> Result<Self> {
        let unsupported = || Error::UnsupportedCollectionElement(typ.kind());
        let storage = match typ.kind() {
            TypeKind::Bool
            | TypeKind::I8
            | TypeKind::U8
            | TypeKind::I16
            | TypeKind::U16
            | TypeKind::Char16
            | TypeKind::I32
            | TypeKind::U32
            | TypeKind::I64
            | TypeKind::U64
            | TypeKind::Enum(_)
            | TypeKind::HResult => {
                let size = typ.size_of();
                if size > target.word_size() {
                    return Err(unsupported());
                }
                CollectionStorage::Word(size)
            }
            TypeKind::Struct(_) => {
                let layout = pod_layout(typ, 0).map_err(|_| unsupported())?;
                aggregate_storage(
                    target,
                    layout.size,
                    layout.homogeneous.is_some(),
                    allow_empty,
                )
                .ok_or_else(unsupported)?
            }
            TypeKind::HString => CollectionStorage::HString,
            kind if kind.is_com_pointer() => CollectionStorage::Object,
            _ => return Err(unsupported()),
        };
        typ.table().try_closed_signature_string_kind(typ.kind())?;
        if storage == CollectionStorage::Object
            && typ.kind() != TypeKind::Object
            && typ
                .iid()
                .is_none_or(|iid| iid == windows_core::GUID::zeroed())
        {
            return Err(unsupported());
        }
        Ok(Self {
            typ: typ.clone(),
            storage,
        })
    }

    pub(crate) fn prepare(&self, item: &WinRTValue) -> Result<PreparedCollectionItem> {
        if self.storage.is_empty_only() {
            return Err(Error::UnsupportedCollectionElement(self.typ.kind()));
        }
        if self.storage == CollectionStorage::Object {
            if item.is_null_object() {
                return Ok(PreparedCollectionItem::Object(None));
            }
            let Some(WinRTValue::Object(object)) =
                crate::native_call::coerce_input_object(&self.typ, item)?
            else {
                return Err(Error::InvalidCollectionValue(
                    "an object implementing the declared interface",
                ));
            };
            return Ok(PreparedCollectionItem::Object(Some(object)));
        }
        if self.storage == CollectionStorage::HString && !matches!(item, WinRTValue::HString(_)) {
            return Err(Error::InvalidCollectionValue("HSTRING"));
        }
        let coerced = crate::native_call::coerce_scalar_input(&self.typ, item)?;
        let item = coerced.as_ref().unwrap_or(item);
        if let WinRTValue::HString(value) = item {
            return Ok(PreparedCollectionItem::HString(value.clone()));
        }
        let word = match item {
            WinRTValue::Bool(value) => usize::from(*value),
            WinRTValue::I8(value) => *value as usize,
            WinRTValue::U8(value) => *value as usize,
            WinRTValue::I16(value) => *value as usize,
            WinRTValue::U16(value) => *value as usize,
            WinRTValue::I32(value) => *value as usize,
            WinRTValue::U32(value) => *value as usize,
            WinRTValue::I64(value) => *value as usize,
            WinRTValue::U64(value) => *value as usize,
            WinRTValue::Enum { value, .. } => *value as usize,
            WinRTValue::HResult(value) => value.0 as usize,
            WinRTValue::Struct(data) => {
                let actual = data.type_handle();
                if actual != &self.typ
                    || actual.size_of() != self.storage.element_size()
                    || actual.align_of() != self.typ.align_of()
                {
                    return Err(Error::InvalidCollectionValue(
                        "the exact declared struct layout",
                    ));
                }
                let mut word = 0usize;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        (&mut word as *mut usize).cast::<u8>(),
                        self.storage.element_size(),
                    );
                }
                word
            }
            _ => {
                return Err(Error::InvalidCollectionValue(
                    "the declared collection element type",
                ));
            }
        };
        Ok(PreparedCollectionItem::Word(word))
    }
}

pub(crate) enum PreparedCollectionItem {
    Word(usize),
    HString(HSTRING),
    Object(Option<IUnknown>),
}

impl PreparedCollectionItem {
    /// Borrow the ABI word without transferring ownership.
    pub(crate) fn as_raw(&self) -> usize {
        match self {
            Self::Word(word) => *word,
            Self::HString(value) => unsafe { std::mem::transmute_copy::<HSTRING, usize>(value) },
            Self::Object(object) => object.as_ref().map_or(0, |object| object.as_raw() as usize),
        }
    }

    pub(crate) fn into_raw(self) -> usize {
        match self {
            Self::Word(word) => word,
            Self::HString(value) => {
                let raw: *mut core::ffi::c_void = unsafe { std::mem::transmute(value) };
                raw as usize
            }
            Self::Object(object) => object.map_or(0, |object| object.into_raw() as usize),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetadataTable, map::create_map_from_values, vector::create_vector_from_values};

    #[test]
    fn target_argument_classification_and_empty_policy() {
        use ArgumentAbi::*;
        use CollectionTarget::*;
        for size in [1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 16, 17, 24, 32] {
            assert_eq!(
                aggregate_abi(X86, size, false),
                if size <= 4 { DirectWord } else { MultipleWords }
            );
            assert_eq!(
                aggregate_abi(X64, size, true),
                if matches!(size, 1 | 2 | 4 | 8) {
                    DirectWord
                } else {
                    IndirectPointer
                }
            );
            assert_eq!(aggregate_abi(Arm64, size, true), FloatingAggregate);
            assert_eq!(
                aggregate_abi(Arm64, size, false),
                if size <= 8 {
                    DirectWord
                } else if size <= 16 {
                    MultipleWords
                } else {
                    IndirectPointer
                }
            );
            assert_eq!(
                aggregate_storage(X86, size, false, true).is_some(),
                size <= 4
            );
            assert_eq!(
                aggregate_storage(X64, size, false, true).is_some(),
                matches!(size, 1 | 2 | 4 | 8) || size > 8
            );
            assert_eq!(
                aggregate_storage(Arm64, size, false, true).is_some(),
                size <= 8 || size > 16
            );
            assert!(aggregate_storage(Arm64, size, true, true).is_none());
            for target in [X86, X64, Arm64] {
                assert_eq!(
                    aggregate_storage(target, size, false, false).is_some(),
                    aggregate_abi(target, size, false) == DirectWord
                );
            }
        }
        assert_eq!(aggregate_abi(Arm64, 0, false), ArgumentAbi::Unsupported);
        assert_eq!(
            aggregate_abi(CollectionTarget::Unsupported, 8, false),
            ArgumentAbi::Unsupported
        );
    }

    #[test]
    fn recursive_layout_distinguishes_hfa_from_mixed_and_five_members() {
        let table = MetadataTable::new();
        for float in [table.f32_type(), table.f64_type()] {
            for count in 1..=5 {
                let inner = table.struct_type(
                    &format!("Test.Float{float:?}.{count}"),
                    &vec![float.clone(); count],
                );
                let nested = table.struct_type(
                    &format!("Test.Nested{float:?}.{count}"),
                    std::slice::from_ref(&inner),
                );
                for typ in [inner, nested] {
                    let layout = pod_layout(&typ, 0).unwrap();
                    assert_eq!(layout.homogeneous.is_some(), count <= 4);
                    let arm =
                        CollectionElementPlan::for_target(&typ, true, CollectionTarget::Arm64);
                    assert_eq!(arm.is_ok(), count == 5);
                }
            }
        }
        let mixed = table.struct_type("Test.Mixed", &[table.f32_type(), table.i32_type()]);
        assert!(pod_layout(&mixed, 0).unwrap().homogeneous.is_none());
        assert_eq!(
            CollectionElementPlan::for_target(&mixed, false, CollectionTarget::Arm64)
                .unwrap()
                .storage,
            CollectionStorage::Word(8)
        );
    }

    #[test]
    fn rejects_recursive_ownership_and_incomplete_element_kinds() {
        let table = MetadataTable::new();
        for (index, field) in [
            table.hstring(),
            table.object(),
            table.interface(windows_core::GUID::zeroed()),
        ]
        .into_iter()
        .enumerate()
        {
            let direct = table.struct_type(&format!("Test.Owned{index}"), &[field]);
            let nested = table.struct_type(
                &format!("Test.NestedOwned{index}"),
                std::slice::from_ref(&direct),
            );
            for typ in [direct, nested] {
                for target in [
                    CollectionTarget::X86,
                    CollectionTarget::X64,
                    CollectionTarget::Arm64,
                ] {
                    assert!(CollectionElementPlan::for_target(&typ, true, target).is_err());
                }
                assert!(create_vector_from_values(&[], &typ, table.vector_iids(&typ)).is_err());
                assert!(
                    create_map_from_values(
                        &[],
                        &typ,
                        &table.i32_type(),
                        table.map_iids(&typ, &table.i32_type())
                    )
                    .is_err()
                );
                assert!(
                    create_map_from_values(
                        &[],
                        &table.i32_type(),
                        &typ,
                        table.map_iids(&table.i32_type(), &typ)
                    )
                    .is_err()
                );
            }
        }
        for typ in [
            table.f32_type(),
            table.f64_type(),
            table.guid_type(),
            table.struct_type("Test.Empty", &[]),
        ] {
            assert!(CollectionElementPlan::new(&typ, true).is_err());
        }
        for kind in [
            TypeKind::ArrayOfIUnknown,
            TypeKind::Generic {
                piid: windows_core::GUID::zeroed(),
                arity: 1,
            },
        ] {
            assert!(CollectionElementPlan::new(&table.make(kind), true).is_err());
        }
    }

    #[test]
    fn exact_struct_identity_precedes_byte_copy() {
        let table = MetadataTable::new();
        let expected = table.struct_type("Test.Expected", &[table.u16_type(), table.u16_type()]);
        let plan = CollectionElementPlan::new(&expected, false).unwrap();
        let other_table = MetadataTable::new();
        for actual in [
            table.struct_type("Test.Other", &[table.u16_type(), table.u16_type()]),
            table.struct_type("Test.Smaller", &[table.u8_type()]),
            other_table.struct_type(
                "Test.Expected",
                &[other_table.u16_type(), other_table.u16_type()],
            ),
        ] {
            let value = WinRTValue::Struct(actual.default_value());
            assert!(plan.prepare(&value).is_err());
            assert!(
                create_vector_from_values(
                    std::slice::from_ref(&value),
                    &expected,
                    table.vector_iids(&expected)
                )
                .is_err()
            );
            assert!(
                create_map_from_values(
                    &[(WinRTValue::I32(1), value)],
                    &table.i32_type(),
                    &expected,
                    table.map_iids(&table.i32_type(), &expected)
                )
                .is_err()
            );
        }
        assert!(plan.prepare(&WinRTValue::I32(0)).is_err());
        assert!(
            plan.prepare(&WinRTValue::Struct(expected.default_value()))
                .is_ok()
        );
        assert!(
            CollectionElementPlan::new(&table.i32_type(), false)
                .unwrap()
                .prepare(&WinRTValue::Struct(expected.default_value()))
                .is_err()
        );
    }

    #[test]
    fn checked_scalar_aliases_do_not_silently_narrow() {
        let table = MetadataTable::new();
        for (typ, good, bad) in [
            (table.i8_type(), -128, 128),
            (table.u8_type(), 255, 257),
            (table.char16_type(), 65535, 65536),
        ] {
            let plan = CollectionElementPlan::new(&typ, false).unwrap();
            assert!(plan.prepare(&WinRTValue::I32(good)).is_ok());
            assert!(plan.prepare(&WinRTValue::I32(bad)).is_err());
            if typ.kind() != TypeKind::I8 {
                assert!(plan.prepare(&WinRTValue::I32(-1)).is_err());
            }
        }
        assert!(
            CollectionElementPlan::new(&table.char16_type(), false)
                .unwrap()
                .prepare(&WinRTValue::U16(65))
                .is_ok()
        );
        assert!(
            CollectionElementPlan::new(&table.u16_type(), false)
                .unwrap()
                .prepare(&WinRTValue::I32(1))
                .is_err()
        );
        assert!(
            CollectionElementPlan::new(&table.bool_type(), false)
                .unwrap()
                .prepare(&WinRTValue::I32(1))
                .is_err()
        );
        let expected = table.enum_type("Test.Enum", vec![]);
        let other = table.enum_type("Test.OtherEnum", vec![]);
        let plan = CollectionElementPlan::new(&expected, false).unwrap();
        assert!(plan.prepare(&WinRTValue::I32(i32::MAX)).is_ok());
        assert!(
            plan.prepare(&WinRTValue::Enum {
                value: 7,
                type_handle: expected
            })
            .is_ok()
        );
        assert!(
            plan.prepare(&WinRTValue::Enum {
                value: 7,
                type_handle: other
            })
            .is_err()
        );
    }
}
