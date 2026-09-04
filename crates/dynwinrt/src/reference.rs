// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;

use windows_core::{GUID, HRESULT, IUnknown, Interface};

use crate::com_helpers::{E_FAIL, E_POINTER, IInspectableVtbl, S_OK};
use crate::metadata_table::{IREFERENCE, TypeHandle, TypeKind};
use crate::{Error, Result, WinRTValue};

#[repr(C)]
struct ReferenceVtbl {
    base: IInspectableVtbl,
    get_value: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DynamicReference {
    vtable: *const ReferenceVtbl,
    ref_count: windows_core::imp::RefCount,
    iid: GUID,
    value_type: TypeHandle,
    value: WinRTValue,
}

unsafe impl Send for DynamicReference {}
unsafe impl Sync for DynamicReference {}

impl DynamicReference {
    const VTBL: ReferenceVtbl = ReferenceVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::qi,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: Self::get_iids_stub,
            get_runtime_class_name: Self::get_runtime_class_name_stub,
            get_trust_level: Self::get_trust_level_stub,
        },
        get_value: Self::get_value,
    };

    fn create(value: WinRTValue, value_type: TypeHandle, iid: GUID) -> IUnknown {
        let reference = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            iid,
            value_type,
            value,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(reference) as *mut c_void) }
    }

    single_vtable_com!(|me: &Self| me.iid);
    inspectable_stubs!(stub);

    unsafe extern "system" fn get_value(this: *mut c_void, result: *mut c_void) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }

        let reference = &*(this as *const Self);
        if write_abi_value(&reference.value_type, &reference.value, result) {
            S_OK
        } else {
            E_FAIL
        }
    }
}

fn value_matches_type(value_type: &TypeHandle, value: &WinRTValue) -> bool {
    match (value_type.kind(), value) {
        (TypeKind::Bool, WinRTValue::Bool(_))
        | (TypeKind::I8, WinRTValue::I8(_))
        | (TypeKind::U8, WinRTValue::U8(_))
        | (TypeKind::I16, WinRTValue::I16(_))
        | (TypeKind::U16 | TypeKind::Char16, WinRTValue::U16(_))
        | (TypeKind::I32, WinRTValue::I32(_))
        | (TypeKind::U32, WinRTValue::U32(_))
        | (TypeKind::I64, WinRTValue::I64(_))
        | (TypeKind::U64, WinRTValue::U64(_))
        | (TypeKind::F32, WinRTValue::F32(_))
        | (TypeKind::F64, WinRTValue::F64(_))
        | (TypeKind::Guid, WinRTValue::Guid(_))
        | (TypeKind::HString, WinRTValue::HString(_))
        | (TypeKind::HResult, WinRTValue::HResult(_))
        | (TypeKind::HResult, WinRTValue::I32(_))
        | (TypeKind::Enum(_), WinRTValue::I32(_)) => true,
        (TypeKind::Enum(_), WinRTValue::Enum { type_handle, .. }) => type_handle == value_type,
        (TypeKind::Struct(_), WinRTValue::Struct(data)) => data.type_handle() == value_type,
        _ => false,
    }
}

unsafe fn write_abi_value(
    value_type: &TypeHandle,
    value: &WinRTValue,
    result: *mut c_void,
) -> bool {
    match (value_type.kind(), value) {
        (TypeKind::Bool, WinRTValue::Bool(value)) => (result as *mut u8).write(u8::from(*value)),
        (TypeKind::I8, WinRTValue::I8(value)) => (result as *mut i8).write(*value),
        (TypeKind::U8, WinRTValue::U8(value)) => (result as *mut u8).write(*value),
        (TypeKind::I16, WinRTValue::I16(value)) => (result as *mut i16).write(*value),
        (TypeKind::U16 | TypeKind::Char16, WinRTValue::U16(value)) => {
            (result as *mut u16).write(*value)
        }
        (TypeKind::I32, WinRTValue::I32(value)) | (TypeKind::Enum(_), WinRTValue::I32(value)) => {
            (result as *mut i32).write(*value)
        }
        (TypeKind::Enum(_), WinRTValue::Enum { value, .. }) => (result as *mut i32).write(*value),
        (TypeKind::U32, WinRTValue::U32(value)) => (result as *mut u32).write(*value),
        (TypeKind::I64, WinRTValue::I64(value)) => (result as *mut i64).write(*value),
        (TypeKind::U64, WinRTValue::U64(value)) => (result as *mut u64).write(*value),
        (TypeKind::F32, WinRTValue::F32(value)) => (result as *mut f32).write(*value),
        (TypeKind::F64, WinRTValue::F64(value)) => (result as *mut f64).write(*value),
        (TypeKind::Guid, WinRTValue::Guid(value)) => (result as *mut GUID).write(*value),
        (TypeKind::HResult, WinRTValue::HResult(value)) => (result as *mut HRESULT).write(*value),
        (TypeKind::HResult, WinRTValue::I32(value)) => {
            (result as *mut HRESULT).write(HRESULT(*value))
        }
        (TypeKind::HString, WinRTValue::HString(value)) => {
            (result as *mut windows_core::HSTRING).write(value.clone())
        }
        (TypeKind::Struct(_), WinRTValue::Struct(data)) => data.copy_to_abi(result),
        _ => return false,
    }
    true
}

/// Box a WinRT value as `IReference<T>`.
///
/// Null values and existing COM wrappers pass through unchanged.
pub fn box_ireference(value: WinRTValue, value_type: TypeHandle) -> Result<WinRTValue> {
    match value {
        // Preserve null and legacy wrapper inputs; method invocation performs
        // the final QueryInterface against the declared IReference<T>.
        WinRTValue::Null | WinRTValue::Object(_) => return Ok(value),
        _ => {}
    }

    if !value_matches_type(&value_type, &value) {
        return Err(Error::InvalidType(value_type.kind(), value.get_type_kind()));
    }

    let table = value_type.table();
    let generic = table.generic(IREFERENCE, 1);
    let reference_type = table.parameterized(&generic, std::slice::from_ref(&value_type));
    let iid = reference_type
        .iid()
        .ok_or_else(|| Error::TypeNotFound("Windows.Foundation.IReference<T>".into()))?;
    Ok(WinRTValue::Object(DynamicReference::create(
        value, value_type, iid,
    )))
}

#[cfg(test)]
mod tests {
    use windows::Foundation::{IReference, Point, PropertyType};
    use windows_core::HSTRING;

    use super::*;
    use crate::{MetadataTable, MethodSignature};

    #[test]
    fn boxes_primitive_and_string_values() {
        let table = MetadataTable::new();

        let value = box_ireference(WinRTValue::U32(17), table.u32_type()).unwrap();
        let reference: IReference<u32> = value.as_object().unwrap().cast().unwrap();
        assert_eq!(reference.Value().unwrap(), 17);

        let value =
            box_ireference(WinRTValue::HString(HSTRING::from("hello")), table.hstring()).unwrap();
        let reference: IReference<HSTRING> = value.as_object().unwrap().cast().unwrap();
        assert_eq!(reference.Value().unwrap(), "hello");
    }

    #[test]
    fn stores_reference_object_fields_with_owned_lifetime() {
        let table = MetadataTable::new();
        let value_type = table.u64_type();
        let generic = table.generic(IREFERENCE, 1);
        let reference_type = table.parameterized(&generic, std::slice::from_ref(&value_type));
        let holder_type = table.struct_type("Test.OptionalUInt64Holder", &[reference_type.clone()]);
        let mut holder = holder_type.default_value();
        let boxed = box_ireference(WinRTValue::U64(17), value_type.clone()).unwrap();
        let boxed_object = boxed.as_object().unwrap();

        holder
            .set_field_object(0, Some(&boxed_object))
            .expect("store reference object");
        drop(boxed);
        drop(boxed_object);
        let stored = holder
            .get_field_object(0)
            .expect("read reference field")
            .expect("non-null reference field");
        let interface = table
            .register_interface(
                "IReference_UInt64_Struct_Test",
                reference_type.iid().unwrap(),
            )
            .add_method(
                "get_Value",
                MethodSignature::new(&table).add_out(value_type),
            );
        let result = interface
            .method(6)
            .unwrap()
            .invoke(stored.as_raw(), &[])
            .unwrap();
        assert!(matches!(result[0], WinRTValue::U64(17)));

        let incompatible = box_ireference(WinRTValue::U32(9), table.u32_type()).unwrap();
        let incompatible_object = incompatible.as_object().unwrap();
        assert!(
            holder
                .set_field_object(0, Some(&incompatible_object))
                .is_err()
        );
        let unchanged = holder
            .get_field_object(0)
            .unwrap()
            .expect("failed replacement must preserve the old field");
        let result = interface
            .method(6)
            .unwrap()
            .invoke(unchanged.as_raw(), &[])
            .unwrap();
        assert!(matches!(result[0], WinRTValue::U64(17)));

        holder
            .set_field_object(0, None)
            .expect("clear reference object");
        assert!(holder.get_field_object(0).unwrap().is_none());
        assert!(
            table
                .struct_type("Test.ScalarHolder", &[table.u32_type()])
                .default_value()
                .get_field_object(0)
                .is_err()
        );
    }

    #[test]
    fn boxes_enum_and_struct_values() {
        let table = MetadataTable::new();
        let enum_type = table.enum_type(
            "Windows.Foundation.PropertyType",
            vec![("UInt8".into(), PropertyType::UInt8.0)],
        );
        let value = box_ireference(
            WinRTValue::Enum {
                value: PropertyType::UInt8.0,
                type_handle: enum_type.clone(),
            },
            enum_type,
        )
        .unwrap();
        let reference: IReference<PropertyType> = value.as_object().unwrap().cast().unwrap();
        assert_eq!(reference.Value().unwrap(), PropertyType::UInt8);

        let struct_type = table.struct_type(
            "Windows.Foundation.Point",
            &[table.f32_type(), table.f32_type()],
        );
        let mut data = struct_type.default_value();
        data.set_field(0, 1.5f32);
        data.set_field(1, 2.5f32);
        let value = box_ireference(WinRTValue::Struct(data), struct_type.clone()).unwrap();
        let reference: IReference<Point> = value.as_object().unwrap().cast().unwrap();
        assert_eq!(reference.Value().unwrap(), Point { X: 1.5, Y: 2.5 });
    }

    #[test]
    fn copies_non_blittable_struct_values_for_the_caller() {
        let table = MetadataTable::new();
        let struct_type = table.struct_type("Test.StringStruct", &[table.hstring()]);
        let mut data = struct_type.default_value();
        let raw: *mut c_void = unsafe { std::mem::transmute(HSTRING::from("owned by caller")) };
        unsafe {
            (data.as_mut_ptr().add(struct_type.field_offset(0)) as *mut *mut c_void).write(raw);
        }

        let boxed = box_ireference(WinRTValue::Struct(data), struct_type.clone()).unwrap();
        let generic = table.generic(IREFERENCE, 1);
        let reference_type = table.parameterized(&generic, std::slice::from_ref(&struct_type));
        let interface = table
            .register_interface(
                "IReference_StringStruct_Test",
                reference_type.iid().unwrap(),
            )
            .add_method(
                "get_Value",
                MethodSignature::new(&table).add_out(struct_type.clone()),
            );
        let raw = boxed.as_object().unwrap().as_raw();
        let result = interface.method(6).unwrap().invoke(raw, &[]).unwrap();
        drop(boxed);

        let result = result[0].as_struct().unwrap();
        let raw: *mut c_void = result.get_field(0);
        let value: &HSTRING = unsafe { &*(&raw as *const *mut c_void as *const HSTRING) };
        assert_eq!(value, "owned by caller");
    }

    #[test]
    fn preserves_null_and_existing_reference_values() {
        let table = MetadataTable::new();
        assert!(matches!(
            box_ireference(WinRTValue::Null, table.u32_type()).unwrap(),
            WinRTValue::Null
        ));

        let existing = windows::Foundation::PropertyValue::CreateUInt32(42)
            .unwrap()
            .cast::<IUnknown>()
            .unwrap();
        let boxed = box_ireference(WinRTValue::Object(existing), table.u32_type()).unwrap();
        let reference: IReference<u32> = boxed.as_object().unwrap().cast().unwrap();
        assert_eq!(reference.Value().unwrap(), 42);
    }

    #[test]
    fn rejects_mismatched_value_types() {
        let table = MetadataTable::new();
        let error = box_ireference(WinRTValue::I32(1), table.u32_type()).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidType(TypeKind::U32, TypeKind::I32)
        ));
    }
}
