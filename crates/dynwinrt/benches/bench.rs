// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Benchmarks: dynwinrt dynamic invocation vs windows-rs static projection.
//!
//! Groups:
//!   1. param_count     — 0, 1, 2 input params (fixed return type)
//!   2. input_type      — same param count (1 in), different input types
//!   3. return_type     — same param count (0 in), different return types
//!   4. struct_size     — different struct sizes as input
//!   5. batch           — realistic workload
//!   6. overhead        — raw vtable vs dynamic to isolate framework cost
//!
//! Run:  cargo bench -p dynwinrt --bench bench

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use windows::Foundation::Uri;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, Interface};
use windows_core::HRESULT;

use dynwinrt::metadata_table::{MetadataTable, TypeHandle};
use dynwinrt::{MethodSignature, WinRTValue};

// ======================================================================
// Setup
// ======================================================================

fn ensure_init() {
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.ok();
}

struct UriApi {
    factory_iid: windows::core::GUID,
    uri_iid: windows::core::GUID,
    factory_type: TypeHandle,
    uri_type: TypeHandle,
}

fn setup_uri(table: &std::sync::Arc<MetadataTable>) -> UriApi {
    let factory_iid =
        windows::core::GUID::try_from("44a9796f-723e-4fdf-a218-033e75b0c084").unwrap();
    let factory_type = table
        .register_interface("IUriRuntimeClassFactory", factory_iid)
        .add_method(
            "CreateUri",
            MethodSignature::new(table)
                .add_in(table.hstring())
                .add_out(table.object()),
        )
        .add_method(
            "CreateWithRelativeUri",
            MethodSignature::new(table)
                .add_in(table.hstring())
                .add_in(table.hstring())
                .add_out(table.object()),
        );

    let uri_iid = windows::core::GUID::try_from("9e365e57-48b2-4160-956f-c7385120bbfc").unwrap();
    let hstring_out = || MethodSignature::new(table).add_out(table.hstring());
    let uri_type = table
        .register_interface("IUriRuntimeClass", uri_iid)
        .add_method("get_AbsoluteUri", hstring_out())
        .add_method("get_DisplayUri", hstring_out())
        .add_method("get_RawUri", hstring_out())
        .add_method("get_SchemeName", hstring_out())
        .add_method("get_UserName", hstring_out())
        .add_method("get_Password", hstring_out())
        .add_method("get_Host", hstring_out())
        .add_method("get_Domain", hstring_out())
        .add_method(
            "get_Port",
            MethodSignature::new(table).add_out(table.i32_type()),
        )
        .add_method("get_Path", hstring_out())
        .add_method("get_Query", hstring_out())
        .add_method(
            "get_QueryParsed",
            MethodSignature::new(table).add_out(table.object()),
        )
        .add_method("get_Fragment", hstring_out())
        .add_method("get_Extension", hstring_out())
        .add_method(
            "get_Suspicious",
            MethodSignature::new(table).add_out(table.bool_type()),
        )
        .add_method(
            "Equals",
            MethodSignature::new(table)
                .add_in(table.object())
                .add_out(table.bool_type()),
        )
        .add_method(
            "CombineUri",
            MethodSignature::new(table)
                .add_in(table.hstring())
                .add_out(table.object()),
        );

    UriApi {
        factory_iid,
        uri_iid,
        factory_type,
        uri_type,
    }
}

struct PropertyValueApi {
    statics_iid: windows::core::GUID,
    statics_type: TypeHandle,
}

fn setup_property_value(table: &std::sync::Arc<MetadataTable>) -> PropertyValueApi {
    let statics_iid =
        windows::core::GUID::try_from("629bdbc8-d932-4ff4-96b9-8d96c5c1e858").unwrap();
    let statics_type = table
        .register_interface("IPropertyValueStatics", statics_iid)
        .add_method(
            "CreateEmpty",
            MethodSignature::new(table).add_out(table.object()),
        ) // 6
        .add_method(
            "CreateUInt8",
            MethodSignature::new(table)
                .add_in(table.u8_type())
                .add_out(table.object()),
        ) // 7
        .add_method(
            "CreateInt16",
            MethodSignature::new(table)
                .add_in(table.i16_type())
                .add_out(table.object()),
        ) // 8
        .add_method(
            "CreateUInt16",
            MethodSignature::new(table)
                .add_in(table.u16_type())
                .add_out(table.object()),
        ) // 9
        .add_method(
            "CreateInt32",
            MethodSignature::new(table)
                .add_in(table.i32_type())
                .add_out(table.object()),
        ) // 10
        .add_method(
            "CreateUInt32",
            MethodSignature::new(table)
                .add_in(table.u32_type())
                .add_out(table.object()),
        ) // 11
        .add_method(
            "CreateInt64",
            MethodSignature::new(table)
                .add_in(table.i64_type())
                .add_out(table.object()),
        ) // 12
        .add_method(
            "CreateUInt64",
            MethodSignature::new(table)
                .add_in(table.u64_type())
                .add_out(table.object()),
        ) // 13
        .add_method(
            "CreateSingle",
            MethodSignature::new(table)
                .add_in(table.f32_type())
                .add_out(table.object()),
        ) // 14
        .add_method(
            "CreateDouble",
            MethodSignature::new(table)
                .add_in(table.f64_type())
                .add_out(table.object()),
        ) // 15
        .add_method(
            "CreateChar16",
            MethodSignature::new(table)
                .add_in(table.u16_type())
                .add_out(table.object()),
        ) // 16
        .add_method(
            "CreateBoolean",
            MethodSignature::new(table)
                .add_in(table.bool_type())
                .add_out(table.object()),
        ) // 17
        .add_method(
            "CreateString",
            MethodSignature::new(table)
                .add_in(table.hstring())
                .add_out(table.object()),
        ) // 18
        .add_method(
            "CreateInspectable",
            MethodSignature::new(table)
                .add_in(table.object())
                .add_out(table.object()),
        ); // 19

    PropertyValueApi {
        statics_iid,
        statics_type,
    }
}

fn get_factory_raw(api: &UriApi) -> *mut std::ffi::c_void {
    let factory =
        dynwinrt::ro_get_activation_factory_2(&HSTRING::from("Windows.Foundation.Uri")).unwrap();
    factory
        .cast(&api.factory_iid)
        .unwrap()
        .as_object()
        .unwrap()
        .as_raw()
}

fn create_uri_raw(
    api: &UriApi,
    fac_raw: *mut std::ffi::c_void,
    s: &str,
) -> (WinRTValue, *mut std::ffi::c_void) {
    let create = api.factory_type.method_by_name("CreateUri").unwrap();
    let result = create
        .invoke(fac_raw, &[WinRTValue::HString(HSTRING::from(s))])
        .unwrap();
    let uri = result.into_iter().next().unwrap();
    let casted = uri.cast(&api.uri_iid).unwrap();
    let raw = casted.as_object().unwrap().as_raw();
    (casted, raw)
}

fn get_pv_statics_raw(api: &PropertyValueApi) -> *mut std::ffi::c_void {
    let factory =
        dynwinrt::ro_get_activation_factory_2(&HSTRING::from("Windows.Foundation.PropertyValue"))
            .unwrap();
    factory
        .cast(&api.statics_iid)
        .unwrap()
        .as_object()
        .unwrap()
        .as_raw()
}

// ======================================================================
// Group 1: Parameter Count (fixed return = object)
// ======================================================================

fn bench_param_count(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();
    let uri_api = setup_uri(&table);
    let fac_raw = get_factory_raw(&uri_api);
    let (_uri_owner, uri_raw) =
        create_uri_raw(&uri_api, fac_raw, "https://example.com/path?q=1#frag");
    let static_uri = Uri::CreateUri(&HSTRING::from("https://example.com/path?q=1#frag")).unwrap();

    let mut g = c.benchmark_group("param_count");

    // 0 in → 1 out (getter)
    let get_host = uri_api.uri_type.method_by_name("get_Host").unwrap();
    g.bench_function("0_in/static", |b| {
        b.iter(|| black_box(static_uri.Host().unwrap()));
    });
    g.bench_function("0_in/dynamic", |b| {
        b.iter(|| black_box(get_host.invoke(uri_raw, &[]).unwrap()));
    });

    // 1 in → 1 out
    let combine = uri_api.uri_type.method_by_name("CombineUri").unwrap();
    g.bench_function("1_in/static", |b| {
        let rel = HSTRING::from("/relative");
        b.iter(|| black_box(static_uri.CombineUri(&rel).unwrap()));
    });
    g.bench_function("1_in/dynamic", |b| {
        let rel = WinRTValue::HString(HSTRING::from("/relative"));
        b.iter(|| black_box(combine.invoke(uri_raw, &[rel.clone()]).unwrap()));
    });

    // 2 in → 1 out
    let create_rel = uri_api
        .factory_type
        .method_by_name("CreateWithRelativeUri")
        .unwrap();
    g.bench_function("2_in/static", |b| {
        let base = HSTRING::from("https://example.com");
        let rel = HSTRING::from("/path?q=1");
        b.iter(|| black_box(Uri::CreateWithRelativeUri(&base, &rel).unwrap()));
    });
    g.bench_function("2_in/dynamic", |b| {
        let base = WinRTValue::HString(HSTRING::from("https://example.com"));
        let rel = WinRTValue::HString(HSTRING::from("/path?q=1"));
        b.iter(|| {
            black_box(
                create_rel
                    .invoke(fac_raw, &[base.clone(), rel.clone()])
                    .unwrap(),
            )
        });
    });

    g.finish();
}

// ======================================================================
// Group 2: Input Type (fixed: 1 in → 1 out object)
// ======================================================================

fn bench_input_type(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();
    let pv_api = setup_property_value(&table);
    let pv_raw = get_pv_statics_raw(&pv_api);

    use windows::Foundation::PropertyValue;

    let mut g = c.benchmark_group("input_type");

    // i32 in
    let create_i32 = pv_api.statics_type.method_by_name("CreateInt32").unwrap();
    g.bench_function("i32/static", |b| {
        b.iter(|| black_box(PropertyValue::CreateInt32(42).unwrap()));
    });
    g.bench_function("i32/dynamic", |b| {
        b.iter(|| black_box(create_i32.invoke(pv_raw, &[WinRTValue::I32(42)]).unwrap()));
    });

    // f64 in
    let create_f64 = pv_api.statics_type.method_by_name("CreateDouble").unwrap();
    g.bench_function("f64/static", |b| {
        b.iter(|| black_box(PropertyValue::CreateDouble(3.14).unwrap()));
    });
    g.bench_function("f64/dynamic", |b| {
        b.iter(|| black_box(create_f64.invoke(pv_raw, &[WinRTValue::F64(3.14)]).unwrap()));
    });

    // bool in
    let create_bool = pv_api.statics_type.method_by_name("CreateBoolean").unwrap();
    g.bench_function("bool/static", |b| {
        b.iter(|| black_box(PropertyValue::CreateBoolean(true).unwrap()));
    });
    g.bench_function("bool/dynamic", |b| {
        b.iter(|| {
            black_box(
                create_bool
                    .invoke(pv_raw, &[WinRTValue::Bool(true)])
                    .unwrap(),
            )
        });
    });

    // hstring in
    let create_str = pv_api.statics_type.method_by_name("CreateString").unwrap();
    g.bench_function("hstring/static", |b| {
        let s = HSTRING::from("hello world");
        b.iter(|| black_box(PropertyValue::CreateString(&s).unwrap()));
    });
    g.bench_function("hstring/dynamic", |b| {
        let s = WinRTValue::HString(HSTRING::from("hello world"));
        b.iter(|| black_box(create_str.invoke(pv_raw, &[s.clone()]).unwrap()));
    });

    // object in
    let create_obj = pv_api
        .statics_type
        .method_by_name("CreateInspectable")
        .unwrap();
    let dummy_obj = PropertyValue::CreateInt32(0).unwrap();
    g.bench_function("object/static", |b| {
        b.iter(|| black_box(PropertyValue::CreateInspectable(&dummy_obj).unwrap()));
    });
    g.bench_function("object/dynamic", |b| {
        let obj = WinRTValue::Object(dummy_obj.cast().unwrap());
        b.iter(|| black_box(create_obj.invoke(pv_raw, &[obj.clone()]).unwrap()));
    });

    // struct in (Point: 2×f32)
    let point_type = table.struct_type(
        "Windows.Foundation.Point",
        &[table.f32_type(), table.f32_type()],
    );
    // CreatePoint is at vtable 23 (6 + 17 methods before it)
    // Let's register a separate interface for it to get the right vtable offset
    let pv_statics2 = table.register_interface("IPropertyValueStatics_point", pv_api.statics_iid);
    // Skip to CreatePoint: 6(IInspectable) + 12(CreateEmpty..CreateString) + 1(CreateInspectable) + 1(CreateGuid) + 1(CreateDateTime) + 1(CreateTimeSpan) = index 22
    // Actually let's just use Geopoint which we know works
    let geo_factory_iid = windows::Devices::Geolocation::IGeopointFactory::IID;
    let geo_type = table.struct_type(
        "Windows.Devices.Geolocation.BasicGeoposition",
        &[table.f64_type(), table.f64_type(), table.f64_type()],
    );
    let geo_factory_type = table
        .register_interface("IGeopointFactory", geo_factory_iid)
        .add_method(
            "Create",
            MethodSignature::new(&table)
                .add_in(geo_type.clone())
                .add_out(table.object()),
        );
    let geo_fac = dynwinrt::ro_get_activation_factory_2(&HSTRING::from(
        "Windows.Devices.Geolocation.Geopoint",
    ))
    .unwrap();
    let geo_fac_raw = geo_fac
        .cast(&geo_factory_iid)
        .unwrap()
        .as_object()
        .unwrap()
        .as_raw();
    let geo_create = geo_factory_type.method_by_name("Create").unwrap();

    g.bench_function("struct_3xf64/static", |b| {
        use windows::Devices::Geolocation::{BasicGeoposition, Geopoint};
        let pos = BasicGeoposition {
            Latitude: 47.643,
            Longitude: -122.131,
            Altitude: 100.0,
        };
        b.iter(|| black_box(Geopoint::Create(pos).unwrap()));
    });
    g.bench_function("struct_3xf64/dynamic", |b| {
        b.iter(|| {
            let mut val = geo_type.default_value();
            val.set_field(0, 47.643f64);
            val.set_field(1, -122.131f64);
            val.set_field(2, 100.0f64);
            black_box(
                geo_create
                    .invoke(geo_fac_raw, &[WinRTValue::Struct(val)])
                    .unwrap(),
            );
        });
    });

    g.finish();
}

// ======================================================================
// Group 3: Return Type (fixed: 0 in → 1 out getter)
// ======================================================================

fn bench_return_type(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();
    let uri_api = setup_uri(&table);
    let fac_raw = get_factory_raw(&uri_api);
    let (_uri_owner, uri_raw) =
        create_uri_raw(&uri_api, fac_raw, "https://example.com:8080/path?q=1");
    let static_uri = Uri::CreateUri(&HSTRING::from("https://example.com:8080/path?q=1")).unwrap();

    let mut g = c.benchmark_group("return_type");

    // i32
    let get_port = uri_api.uri_type.method_by_name("get_Port").unwrap();
    g.bench_function("i32/static", |b| {
        b.iter(|| black_box(static_uri.Port().unwrap()));
    });
    g.bench_function("i32/dynamic", |b| {
        b.iter(|| black_box(get_port.invoke(uri_raw, &[]).unwrap()));
    });

    // bool
    let get_sus = uri_api.uri_type.method_by_name("get_Suspicious").unwrap();
    g.bench_function("bool/static", |b| {
        b.iter(|| black_box(static_uri.Suspicious().unwrap()));
    });
    g.bench_function("bool/dynamic", |b| {
        b.iter(|| black_box(get_sus.invoke(uri_raw, &[]).unwrap()));
    });

    // hstring
    let get_host = uri_api.uri_type.method_by_name("get_Host").unwrap();
    g.bench_function("hstring/static", |b| {
        b.iter(|| black_box(static_uri.Host().unwrap()));
    });
    g.bench_function("hstring/dynamic", |b| {
        b.iter(|| black_box(get_host.invoke(uri_raw, &[]).unwrap()));
    });

    // object (CombineUri: 1 in → 1 out, but object return)
    let combine = uri_api.uri_type.method_by_name("CombineUri").unwrap();
    g.bench_function("object/static", |b| {
        let rel = HSTRING::from("/path");
        b.iter(|| black_box(static_uri.CombineUri(&rel).unwrap()));
    });
    g.bench_function("object/dynamic", |b| {
        let rel = WinRTValue::HString(HSTRING::from("/path"));
        b.iter(|| black_box(combine.invoke(uri_raw, &[rel.clone()]).unwrap()));
    });

    g.finish();
}

// ======================================================================
// Group 4: Struct Size (varying field count)
// ======================================================================

fn bench_struct_size(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();

    let mut g = c.benchmark_group("struct_size");

    // 2 fields (Point: 2×f32 = 8 bytes)
    // Use PropertyValue.CreatePoint (vtable 23)
    // For simplicity, use Geopoint-style test but with different struct sizes

    // 3 fields (BasicGeoposition: 3×f64 = 24 bytes) — already works
    let f64_h = table.f64_type();
    let geo_type = table.struct_type(
        "Windows.Devices.Geolocation.BasicGeoposition",
        &[f64_h.clone(), f64_h.clone(), f64_h],
    );
    let geo_iid = windows::Devices::Geolocation::IGeopointFactory::IID;
    let geo_factory = table
        .register_interface("IGeopointFactory", geo_iid)
        .add_method(
            "Create",
            MethodSignature::new(&table)
                .add_in(geo_type.clone())
                .add_out(table.object()),
        );
    let geo_fac = dynwinrt::ro_get_activation_factory_2(&HSTRING::from(
        "Windows.Devices.Geolocation.Geopoint",
    ))
    .unwrap();
    let geo_fac_raw = geo_fac
        .cast(&geo_iid)
        .unwrap()
        .as_object()
        .unwrap()
        .as_raw();
    let geo_create = geo_factory.method_by_name("Create").unwrap();

    g.bench_function("3_fields_24bytes/static", |b| {
        use windows::Devices::Geolocation::{BasicGeoposition, Geopoint};
        let pos = BasicGeoposition {
            Latitude: 47.643,
            Longitude: -122.131,
            Altitude: 100.0,
        };
        b.iter(|| black_box(Geopoint::Create(pos).unwrap()));
    });
    g.bench_function("3_fields_24bytes/dynamic", |b| {
        b.iter(|| {
            let mut val = geo_type.default_value();
            val.set_field(0, 47.643f64);
            val.set_field(1, -122.131f64);
            val.set_field(2, 100.0f64);
            black_box(
                geo_create
                    .invoke(geo_fac_raw, &[WinRTValue::Struct(val)])
                    .unwrap(),
            );
        });
    });

    // Struct alloc only (isolate alloc cost)
    g.bench_function("3_fields_24bytes/alloc_only", |b| {
        b.iter(|| {
            let mut val = geo_type.default_value();
            val.set_field(0, 47.643f64);
            val.set_field(1, -122.131f64);
            val.set_field(2, 100.0f64);
            black_box(val);
        });
    });

    g.finish();
}

// ======================================================================
// Group 5: Batch Workload
// ======================================================================

fn bench_batch(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();
    let uri_api = setup_uri(&table);
    let fac_raw = get_factory_raw(&uri_api);

    let create = uri_api.factory_type.method_by_name("CreateUri").unwrap();
    let get_abs = uri_api.uri_type.method_by_name("get_AbsoluteUri").unwrap();

    let mut g = c.benchmark_group("batch");

    let n = 100;
    g.bench_function(&format!("{n}x_create_and_read/static"), |b| {
        b.iter(|| {
            for i in 0..n {
                let hstr = HSTRING::from(format!("https://example.com/{i}"));
                let uri = Uri::CreateUri(&hstr).unwrap();
                black_box(uri.AbsoluteUri().unwrap());
            }
        });
    });

    g.bench_function(&format!("{n}x_create_and_read/dynamic"), |b| {
        b.iter(|| {
            for i in 0..n {
                let arg = WinRTValue::HString(HSTRING::from(format!("https://example.com/{i}")));
                let uri_val = create.invoke(fac_raw, &[arg]).unwrap();
                let uri_obj = uri_val.into_iter().next().unwrap();
                let casted = uri_obj.cast(&uri_api.uri_iid).unwrap();
                black_box(
                    get_abs
                        .invoke(casted.as_object().unwrap().as_raw(), &[])
                        .unwrap(),
                );
            }
        });
    });

    g.finish();
}

// ======================================================================
// Group 6: Overhead Isolation (static vs raw vtable vs dynamic)
// ======================================================================

fn bench_overhead(c: &mut Criterion) {
    ensure_init();
    let table = MetadataTable::new();
    let uri_api = setup_uri(&table);
    let fac_raw = get_factory_raw(&uri_api);
    let (_uri_owner, uri_raw) = create_uri_raw(&uri_api, fac_raw, "https://example.com/path?q=1");
    let static_uri = Uri::CreateUri(&HSTRING::from("https://example.com/path?q=1")).unwrap();

    let mut g = c.benchmark_group("overhead");

    // hstring getter
    let get_abs = uri_api.uri_type.method_by_name("get_AbsoluteUri").unwrap();
    g.bench_function("hstring_getter/static", |b| {
        b.iter(|| black_box(static_uri.AbsoluteUri().unwrap()));
    });
    g.bench_function("hstring_getter/raw_vtable", |b| {
        b.iter(|| unsafe {
            let vtable = *(uri_raw as *const *const *mut std::ffi::c_void);
            let fptr = *vtable.add(6);
            type Fn = unsafe extern "system" fn(
                *mut std::ffi::c_void,
                *mut *mut std::ffi::c_void,
            ) -> HRESULT;
            let method: Fn = std::mem::transmute(fptr);
            let mut result: *mut std::ffi::c_void = std::ptr::null_mut();
            method(uri_raw, &mut result).ok().unwrap();
            black_box(WinRTValue::HString(std::mem::transmute(result)));
        });
    });
    g.bench_function("hstring_getter/dynamic", |b| {
        b.iter(|| black_box(get_abs.invoke(uri_raw, &[]).unwrap()));
    });

    // i32 getter
    let (_port_owner, port_raw) =
        create_uri_raw(&uri_api, fac_raw, "https://example.com:8080/path");
    let static_port_uri = Uri::CreateUri(&HSTRING::from("https://example.com:8080/path")).unwrap();
    let get_port = uri_api.uri_type.method_by_name("get_Port").unwrap();

    g.bench_function("i32_getter/static", |b| {
        b.iter(|| black_box(static_port_uri.Port().unwrap()));
    });
    g.bench_function("i32_getter/raw_vtable", |b| {
        b.iter(|| unsafe {
            let vtable = *(port_raw as *const *const *mut std::ffi::c_void);
            let fptr = *vtable.add(14); // get_Port
            type Fn = unsafe extern "system" fn(*mut std::ffi::c_void, *mut i32) -> HRESULT;
            let method: Fn = std::mem::transmute(fptr);
            let mut result: i32 = 0;
            method(port_raw, &mut result).ok().unwrap();
            black_box(result);
        });
    });
    g.bench_function("i32_getter/dynamic", |b| {
        b.iter(|| black_box(get_port.invoke(port_raw, &[]).unwrap()));
    });

    g.finish();
}

// ======================================================================
// Main
// ======================================================================

criterion_group!(
    benches,
    bench_param_count,
    bench_input_type,
    bench_return_type,
    bench_struct_size,
    bench_batch,
    bench_overhead,
);
criterion_main!(benches);
