// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
#![cfg(windows)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock, Weak};

use dynwinrt::{MetadataTable, MethodSignature, TypeHandle, WinRTValue};
use windows::Devices::Geolocation::{BasicGeoposition, Geopoint, IGeopoint};
use windows::Foundation::{IClosable, IStringable, IUriRuntimeClassFactory, MemoryBuffer, Uri};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows_core::{HSTRING, Interface};

struct Apartment;

impl Apartment {
    fn new() -> Self {
        static MTA: OnceLock<()> = OnceLock::new();
        MTA.get_or_init(|| {
            // Keep the MTA alive as long as windows-core's process-wide factory cache.
            // Worker apartment initialization below remains balanced.
            let _cookie = unsafe { CoIncrementMTAUsage() }.unwrap();
        });
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        Self
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

fn stringable(table: &Arc<MetadataTable>) -> TypeHandle {
    table
        .register_interface("IStringable", IStringable::IID)
        .add_method(
            "ToString",
            MethodSignature::new(table).add_out(table.hstring()),
        )
}

fn assert_expired(weak: &Weak<MetadataTable>) {
    assert!(
        weak.upgrade().is_none(),
        "registry retains {} strong owners",
        weak.strong_count()
    );
}

fn registration_case(case: usize) -> Weak<MetadataTable> {
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    if case == 6 {
        drop(
            MethodSignature::new(&table)
                .add_in(table.i32_type())
                .add_out(table.hstring()),
        );
    } else if case != 0 {
        let mut interface = table.register_interface("Lifetime", IStringable::IID);
        if case >= 2 {
            let mut signature = MethodSignature::new(&table);
            if matches!(case, 3 | 5) {
                signature = signature.add_in(table.i32_type());
            }
            if matches!(case, 4 | 5) {
                signature = signature.add_out(table.hstring());
            }
            interface = interface.add_method("Method", signature);
            drop(interface.method(6).unwrap());
        }
    }
    drop(table);
    weak
}

#[test]
fn all_registered_and_unregistered_lifetime_cases_expire() {
    let cases = [
        "empty",
        "interface only",
        "zero parameter",
        "scalar input",
        "HSTRING output",
        "scalar input and HSTRING output",
        "unregistered signature",
    ];
    let mut retained = Vec::new();
    for (case, name) in cases.into_iter().enumerate() {
        let observations: Vec<_> = (0..32).map(|_| registration_case(case)).collect();
        let live = observations
            .iter()
            .filter(|weak| weak.upgrade().is_some())
            .count();
        if live != 0 {
            retained.push((name, live));
        }
    }
    assert!(
        retained.is_empty(),
        "retained registries after all caller owners dropped: {retained:?}"
    );
}

#[test]
fn local_compound_types_and_multiple_methods_do_not_retain_registry() {
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let scalar = table.enum_type("Lifetime.Enum", vec![("One".into(), 1)]);
    let inner = table.struct_type("Lifetime.Inner", &[table.i32_type(), scalar.clone()]);
    let outer = table.struct_type("Lifetime.Outer", &[inner, table.f64_type()]);
    let array = table.array(&outer);
    let nested_array = table.array(&array);
    let generic = table.parameterized(
        &table.generic(dynwinrt::metadata_table::IVECTOR_VIEW, 1),
        &[table.hstring()],
    );
    let async_type = table.async_operation(&generic);
    let mut interface = table.register_interface("Lifetime", IStringable::IID);
    for (index, typ) in [scalar, outer, array, nested_array, generic, async_type]
        .into_iter()
        .enumerate()
    {
        let name = format!("Method{index}");
        let signature = MethodSignature::new(&table).add_out(typ);
        interface = interface
            .add_method(&name, signature.clone())
            .add_method(&name, signature);
        assert!(interface.method(6 + index).is_some());
        assert!(interface.method_by_name(&name).is_some());
    }
    drop(interface);
    drop(table);
    assert_expired(&weak);
}

#[test]
fn external_type_handle_keeps_registered_methods_and_layout_usable() {
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    drop(stringable(&table));
    let typ = table.struct_type("Lifetime.Point", &[table.f32_type(), table.f32_type()]);
    let clone = typ.clone();
    drop(table);
    drop(typ);
    assert!(weak.upgrade().is_some());
    assert_eq!(clone.size_of(), 8);
    assert_eq!(clone.field_type(1).size_of(), 4);
    assert!(
        clone
            .table()
            .interface(IStringable::IID)
            .method_by_name("ToString")
            .is_some()
    );
    let value = clone.default_value();
    drop(clone);
    assert_eq!(value.type_handle().field_count(), 2);
    drop(value);
    assert_expired(&weak);
}

#[test]
fn unregistered_signature_clones_keep_constructor_and_parameter_owners() {
    let constructor = MetadataTable::new();
    let parameters = MetadataTable::new();
    let constructor_weak = Arc::downgrade(&constructor);
    let parameters_weak = Arc::downgrade(&parameters);
    let signature = MethodSignature::new(&constructor).add_out(parameters.hstring());
    let clone = signature.clone();
    drop(signature);
    drop(constructor);
    drop(parameters);
    assert!(constructor_weak.upgrade().is_some());
    assert!(parameters_weak.upgrade().is_some());
    let method = clone.build(6);
    assert_expired(&constructor_weak);
    assert!(parameters_weak.upgrade().is_some());
    drop(method);
    assert_expired(&parameters_weak);

    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let empty = MethodSignature::new(&table);
    drop(table);
    assert!(weak.upgrade().is_some());
    drop(empty.build(6));
    assert_expired(&weak);
}

#[test]
fn foreign_parameters_are_rejected_before_publication_or_lock_poisoning() {
    for direction in 0..3 {
        for duplicate in [false, true] {
            let local = MetadataTable::new();
            let foreign = MetadataTable::new();
            let local_weak = Arc::downgrade(&local);
            let foreign_weak = Arc::downgrade(&foreign);
            let mut interface = local.register_interface("Lifetime", IStringable::IID);
            if duplicate {
                interface = interface.add_method("Method", MethodSignature::new(&local));
            }
            let signature = match direction {
                0 => MethodSignature::new(&local).add_in(foreign.i32_type()),
                1 => MethodSignature::new(&local).add_out(foreign.hstring()),
                _ => MethodSignature::new(&local).add_out_fill(foreign.array(&foreign.i32_type())),
            };
            let failure = catch_unwind(AssertUnwindSafe(|| {
                interface.clone().add_method("Method", signature)
            }));
            let failure = failure.expect_err("foreign parameter registration must fail");
            let message = failure
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| failure.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(message.contains("same MetadataTable"), "{message}");
            assert_eq!(interface.method(6).is_some(), duplicate);
            assert!(interface.method(7).is_none());
            interface = interface.add_method(
                "Valid",
                MethodSignature::new(&local).add_out(local.hstring()),
            );
            assert!(interface.method_by_name("Valid").is_some());
            drop(interface);
            drop(local);
            drop(foreign);
            assert_expired(&local_weak);
            assert_expired(&foreign_weak);
        }
    }
}

#[test]
fn foreign_constructor_with_local_or_no_parameters_is_accepted() {
    for has_parameter in [false, true] {
        let local = MetadataTable::new();
        let foreign = MetadataTable::new();
        let local_weak = Arc::downgrade(&local);
        let foreign_weak = Arc::downgrade(&foreign);
        let mut signature = MethodSignature::new(&foreign);
        if has_parameter {
            signature = signature.add_out(local.hstring());
        }
        let interface = local
            .register_interface("Lifetime", IStringable::IID)
            .add_method("Method", signature);
        drop(foreign);
        assert_expired(&foreign_weak);
        let method = interface.method(6).unwrap();
        drop(interface);
        drop(local);
        assert!(local_weak.upgrade().is_some());
        drop(method);
        assert_expired(&local_weak);
    }
}

#[test]
fn reciprocal_foreign_registrations_cannot_create_table_cycles() {
    let first = MetadataTable::new();
    let second = MetadataTable::new();
    let first_weak = Arc::downgrade(&first);
    let second_weak = Arc::downgrade(&second);
    for (local, foreign) in [(&first, &second), (&second, &first)] {
        let interface = local.register_interface("Lifetime", IStringable::IID);
        let signature = MethodSignature::new(local).add_out(foreign.hstring());
        assert!(
            catch_unwind(AssertUnwindSafe(
                || interface.add_method("Method", signature)
            ))
            .is_err()
        );
    }
    drop(first);
    drop(second);
    assert_expired(&first_weak);
    assert_expired(&second_weak);
}

#[test]
fn retained_method_clones_call_sdk_getter_and_release_registry() {
    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let method = stringable(&table).method(6).unwrap();
    let clone = method.clone();
    drop(table);
    drop(method);
    let uri: IStringable = Uri::CreateUri(&HSTRING::from("https://example.com/lifetime"))
        .unwrap()
        .cast()
        .unwrap();
    for _ in 0..8 {
        assert_eq!(
            clone.call_getter_hstring(uri.as_raw()).unwrap(),
            uri.ToString().unwrap()
        );
        assert_eq!(
            clone.invoke(uri.as_raw(), &[]).unwrap()[0]
                .as_hstring()
                .unwrap(),
            uri.ToString().unwrap()
        );
    }
    drop(clone);
    assert_expired(&weak);
}

#[test]
fn zero_parameter_handle_keeps_registry_alive_and_calls_sdk() {
    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let method = table
        .register_interface("IClosable", IClosable::IID)
        .add_method("Close", MethodSignature::new(&table))
        .method(6)
        .unwrap();
    drop(table);
    assert!(weak.upgrade().is_some());
    let buffer: IClosable = MemoryBuffer::Create(16).unwrap().cast().unwrap();
    assert!(method.invoke(buffer.as_raw(), &[]).unwrap().is_empty());
    drop(method);
    assert_expired(&weak);
}

#[test]
fn registered_and_standalone_factory_plans_preserve_needed_owners() {
    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let foreign = MetadataTable::new();
    let table_weak = Arc::downgrade(&table);
    let foreign_weak = Arc::downgrade(&foreign);
    let signature = MethodSignature::new(&table)
        .add_in(table.hstring())
        .add_out(table.object());
    let method = table
        .register_interface("IUriRuntimeClassFactory", IUriRuntimeClassFactory::IID)
        .add_method("CreateUri", signature)
        .method(6)
        .unwrap();
    let standalone = MethodSignature::new(&table)
        .add_in(foreign.hstring())
        .add_out(table.object())
        .build(6);
    let factory = WinRTValue::from_activation_factory(&HSTRING::from("Windows.Foundation.Uri"))
        .unwrap()
        .cast(&IUriRuntimeClassFactory::IID)
        .unwrap()
        .as_object()
        .unwrap();
    drop(table);
    drop(foreign);
    for _ in 0..8 {
        let args = [WinRTValue::HString(HSTRING::from(
            "https://example.com/retained",
        ))];
        for result in [
            method.invoke(factory.as_raw(), &args).unwrap(),
            standalone.call_dynamic(factory.as_raw(), &args).unwrap(),
        ] {
            let uri: Uri = result[0].as_object().unwrap().cast().unwrap();
            assert_eq!(uri.Host().unwrap(), "example.com");
        }
    }
    drop(method);
    assert!(table_weak.upgrade().is_some());
    assert!(foreign_weak.upgrade().is_some());
    drop(standalone);
    assert_expired(&table_weak);
    assert_expired(&foreign_weak);
}

#[test]
fn sdk_struct_result_outlives_method_and_keeps_metadata() {
    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let typ = table.struct_type(
        "Windows.Devices.Geolocation.BasicGeoposition",
        &[table.f64_type(), table.f64_type(), table.f64_type()],
    );
    let method = table
        .register_interface("IGeopoint", IGeopoint::IID)
        .add_method("get_Position", MethodSignature::new(&table).add_out(typ))
        .method(6)
        .unwrap();
    drop(table);
    let point: IGeopoint = Geopoint::Create(BasicGeoposition {
        Latitude: 12.0,
        Longitude: 34.0,
        Altitude: 56.0,
    })
    .unwrap()
    .cast()
    .unwrap();
    let result = method.invoke(point.as_raw(), &[]).unwrap();
    drop(method);
    assert!(weak.upgrade().is_some());
    let value = result[0].as_struct().unwrap();
    assert_eq!(value.get_field::<f64>(0), 12.0);
    assert_eq!(value.get_field::<f64>(2), 56.0);
    assert_eq!(
        value.type_handle().signature_string(),
        "struct(Windows.Devices.Geolocation.BasicGeoposition;f8;f8;f8)"
    );
    drop(result);
    assert_expired(&weak);
}

#[test]
fn method_clones_are_callable_concurrently_after_original_owners_drop() {
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let method = table
        .register_interface("IUriRuntimeClassFactory", IUriRuntimeClassFactory::IID)
        .add_method(
            "CreateUri",
            MethodSignature::new(&table)
                .add_in(table.hstring())
                .add_out(table.object()),
        )
        .method(6)
        .unwrap();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let method = method.clone();
            std::thread::spawn(move || {
                let _apartment = Apartment::new();
                let factory =
                    WinRTValue::from_activation_factory(&HSTRING::from("Windows.Foundation.Uri"))
                        .unwrap()
                        .cast(&IUriRuntimeClassFactory::IID)
                        .unwrap()
                        .as_object()
                        .unwrap();
                for _ in 0..8 {
                    let result = method
                        .invoke(
                            factory.as_raw(),
                            &[WinRTValue::HString(HSTRING::from(
                                "https://example.com/threads",
                            ))],
                        )
                        .unwrap();
                    let uri: Uri = result[0].as_object().unwrap().cast().unwrap();
                    assert_eq!(uri.Host().unwrap(), "example.com");
                }
            })
        })
        .collect();
    drop(table);
    drop(method);
    for thread in threads {
        thread.join().unwrap();
    }
    assert_expired(&weak);
}

#[test]
fn sdk_array_and_enum_outputs_keep_their_metadata_until_drop() {
    use windows::Foundation::{IPropertyValue, PropertyType, PropertyValue};

    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let enum_type = table.enum_type(
        "Windows.Foundation.PropertyType",
        vec![("Int32Array".into(), PropertyType::Int32Array.0)],
    );
    let mut interface = table
        .register_interface("IPropertyValue", IPropertyValue::IID)
        .add_method("get_Type", MethodSignature::new(&table).add_out(enum_type));
    for (name, typ) in [
        ("get_IsNumericScalar", table.bool_type()),
        ("GetUInt8", table.u8_type()),
        ("GetInt16", table.i16_type()),
        ("GetUInt16", table.u16_type()),
        ("GetInt32", table.i32_type()),
        ("GetUInt32", table.u32_type()),
        ("GetInt64", table.i64_type()),
        ("GetUInt64", table.u64_type()),
        ("GetSingle", table.f32_type()),
        ("GetDouble", table.f64_type()),
        ("GetChar16", table.char16_type()),
        ("GetBoolean", table.bool_type()),
        ("GetString", table.hstring()),
        ("GetGuid", table.guid_type()),
        (
            "GetDateTime",
            table.struct_type("Windows.Foundation.DateTime", &[table.i64_type()]),
        ),
        (
            "GetTimeSpan",
            table.struct_type("Windows.Foundation.TimeSpan", &[table.i64_type()]),
        ),
        (
            "GetPoint",
            table.struct_type(
                "Windows.Foundation.Point",
                &[table.f32_type(), table.f32_type()],
            ),
        ),
        (
            "GetSize",
            table.struct_type(
                "Windows.Foundation.Size",
                &[table.f32_type(), table.f32_type()],
            ),
        ),
        (
            "GetRect",
            table.struct_type(
                "Windows.Foundation.Rect",
                &[
                    table.f32_type(),
                    table.f32_type(),
                    table.f32_type(),
                    table.f32_type(),
                ],
            ),
        ),
        ("GetUInt8Array", table.array(&table.u8_type())),
        ("GetInt16Array", table.array(&table.i16_type())),
        ("GetUInt16Array", table.array(&table.u16_type())),
        ("GetInt32Array", table.array(&table.i32_type())),
    ] {
        interface = interface.add_method(name, MethodSignature::new(&table).add_out(typ));
    }
    let getter = interface.method(6).unwrap();
    let array_getter = interface.method(29).unwrap();
    drop(interface);
    drop(table);
    let property: IPropertyValue = PropertyValue::CreateInt32Array(&[10, 20, 30])
        .unwrap()
        .cast()
        .unwrap();
    let enumeration = getter.invoke(property.as_raw(), &[]).unwrap();
    let array = array_getter.invoke(property.as_raw(), &[]).unwrap();
    drop(getter);
    drop(array_getter);
    let WinRTValue::Enum { value, type_handle } = &enumeration[0] else {
        panic!("expected enum result")
    };
    assert_eq!(*value, PropertyType::Int32Array.0);
    assert_eq!(
        type_handle.enum_member_name(*value).as_deref(),
        Some("Int32Array")
    );
    drop(enumeration);
    assert!(weak.upgrade().is_some());
    assert_eq!(array[0].as_array().unwrap().get_i32(2).unwrap(), 30);
    drop(array);
    assert_expired(&weak);
}

#[test]
fn sdk_async_result_keeps_result_and_progress_metadata_until_drop() {
    use windows::Storage::Streams::{Buffer, IBuffer, IOutputStream, InMemoryRandomAccessStream};

    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let async_type = table.async_operation_with_progress(&table.u32_type(), &table.u32_type());
    let method = table
        .register_interface("IOutputStream", IOutputStream::IID)
        .add_method(
            "WriteAsync",
            MethodSignature::new(&table)
                .add_in(table.interface(IBuffer::IID))
                .add_out(async_type),
        )
        .method(6)
        .unwrap();
    let stream: IOutputStream = InMemoryRandomAccessStream::new().unwrap().cast().unwrap();
    let buffer = Buffer::Create(16).unwrap();
    buffer.SetLength(16).unwrap();
    drop(table);
    let result = method
        .invoke(
            stream.as_raw(),
            &[WinRTValue::Object(buffer.cast().unwrap())],
        )
        .unwrap();
    drop(method);
    let WinRTValue::Async(info) = &result[0] else {
        panic!("expected async result")
    };
    assert_eq!(info.result_type().unwrap().signature_string(), "u4");
    assert_eq!(info.progress_type().unwrap().signature_string(), "u4");
    assert!(matches!(
        futures::executor::block_on(async { (&result[0]).await }).unwrap(),
        WinRTValue::U32(16)
    ));
    assert!(weak.upgrade().is_some());
    drop(result);
    assert_expired(&weak);
}

#[test]
fn delegate_keeps_parameter_metadata_until_final_release() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows::Foundation::{IMemoryBufferReference, TypedEventHandler};
    use windows_core::{HRESULT, IInspectable};

    type Handler = TypedEventHandler<IMemoryBufferReference, IInspectable>;
    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    drop(stringable(&table));
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let delegate: Handler = dynwinrt::delegate::try_create_delegate(
        Handler::IID,
        vec![table.interface(IMemoryBufferReference::IID), table.object()],
        Box::new(move |args| {
            assert_eq!(args.len(), 2);
            callback_calls.fetch_add(1, Ordering::SeqCst);
            HRESULT(0)
        }),
    )
    .unwrap()
    .cast()
    .unwrap();
    drop(table);
    let reference = MemoryBuffer::Create(16).unwrap().CreateReference().unwrap();
    delegate.Invoke(&reference, None).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(weak.upgrade().is_some());
    drop(delegate);
    assert_expired(&weak);
}

#[test]
fn reverse_call_can_reenter_registration_without_retaining_registry() {
    use dynwinrt::{
        WinRtImplementation, WinRtImplementationPlan, WinRtInterfaceDefinition,
        WinRtMethodDefinition, WinRtThreadingPolicy,
    };

    let _apartment = Apartment::new();
    let table = MetadataTable::new();
    let weak = Arc::downgrade(&table);
    let method = stringable(&table).method(6).unwrap();
    let plan = WinRtImplementationPlan::new(
        vec![WinRtInterfaceDefinition {
            name: "IStringable".into(),
            interface_type: table.interface(IStringable::IID),
            required_iids: vec![],
            methods: vec![WinRtMethodDefinition {
                name: "ToString".into(),
                vtable_index: 6,
                signature: MethodSignature::new(&table).add_out(table.hstring()),
            }],
        }],
        WinRtThreadingPolicy::OwnerThread,
    )
    .unwrap();
    let callback_table = table.clone();
    let owner = WinRtImplementation::new(
        plan,
        Arc::new(move |_, _, _| {
            drop(
                callback_table
                    .register_interface("IClosable", IClosable::IID)
                    .add_method("Close", MethodSignature::new(&callback_table)),
            );
            Ok(vec![WinRTValue::HString(HSTRING::from("reentrant"))])
        }),
        None,
    )
    .unwrap();
    drop(table);
    let object = owner
        .to_value()
        .unwrap()
        .cast(&IStringable::IID)
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        method.invoke(object.as_raw(), &[]).unwrap()[0]
            .as_hstring()
            .unwrap(),
        "reentrant"
    );
    drop(method);
    assert_eq!(
        object.cast::<IStringable>().unwrap().ToString().unwrap(),
        "reentrant"
    );
    drop(object);
    drop(owner);
    assert_expired(&weak);
}

#[test]
fn classic_com_methods_keep_foreign_parameter_owners_without_registry_cycles() {
    use dynwinrt::com;

    let table = MetadataTable::new();
    let foreign = MetadataTable::new();
    let local_weak = Arc::downgrade(&table);
    let foreign_weak = Arc::downgrade(&foreign);
    drop(stringable(&table));
    drop(stringable(&foreign));
    let interface = com::register_interface(
        &table,
        "Lifetime",
        IStringable::IID,
        com::InterfaceBase::IUnknown,
    )
    .add_method(
        "Method",
        com::MethodSignature::new(&table).add_in(com::Type::winrt(foreign.i32_type())),
    );
    let method = interface.method(3).unwrap();
    drop(interface);
    drop(table);
    drop(foreign);
    assert_expired(&local_weak);
    assert!(foreign_weak.upgrade().is_some());
    drop(method);
    assert_expired(&foreign_weak);
}
