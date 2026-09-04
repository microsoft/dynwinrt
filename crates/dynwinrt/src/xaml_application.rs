// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetDpiAwarenessContextForProcess, SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
};
use windows_core::{GUID, HRESULT, HSTRING, IUnknown, Interface};

use crate::com_helpers::{E_FAIL, E_NOINTERFACE, E_NOTIMPL, E_POINTER, IInspectableVtbl, S_OK};
use crate::composition::CompositionState;
use crate::{Result, WinRTValue};

const IID_IAPPLICATION_FACTORY: GUID = GUID::from_u128(0x9fd96657_5294_5a65_a1db_4fea143597da);
const IID_IAPPLICATION_OVERRIDES: GUID = GUID::from_u128(0xa33e81ef_c665_503b_8827_d27ef1720a06);
const IID_IXAML_METADATA_PROVIDER: GUID = GUID::from_u128(0xa96251f0_2214_5d53_8746_ce99a2593cd7);
const IID_IXAML_TYPE: GUID = GUID::from_u128(0xd24219df_7ec9_57f1_a27b_6af251d9c5bc);
const IID_XAML_LAUNCHED_CALLBACK: GUID = GUID::from_u128(0xf81c4e72_7a18_4a30_9126_6f62b6bdac83);
const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
const E_OUTOFMEMORY: HRESULT = HRESULT(0x8007000Eu32 as i32);
const HRESULT_ALREADY_EXISTS: HRESULT = HRESULT(0x800700B7u32 as i32);
const TYPE_KIND_METADATA: i32 = 1;
const TYPE_KIND_CUSTOM: i32 = 2;
const RO_E_CLOSED: HRESULT = HRESULT(0x80000013_u32 as i32);

/// Synchronous constructor used by a registered XAML runtime class.
pub type XamlRuntimeClassActivator = Arc<dyn Fn() -> windows_core::Result<IUnknown> + Send + Sync>;

struct XamlRuntimeClassEntry {
    id: u64,
    name: String,
    base_type: String,
    base_iid: GUID,
    activator: XamlRuntimeClassActivator,
    active: Arc<AtomicBool>,
}

static XAML_RUNTIME_CLASSES: LazyLock<Mutex<HashMap<String, Arc<XamlRuntimeClassEntry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_XAML_RUNTIME_CLASS_ID: AtomicU64 = AtomicU64::new(1);

/// Owns one process-local XAML runtime-class registration.
///
/// Dropping or explicitly unregistering the handle prevents new metadata
/// lookups. IXamlType objects already handed to XAML retain the entry until
/// their in-flight use completes.
pub struct XamlRuntimeClassRegistration {
    name: String,
    id: u64,
    supported_overrides: Vec<String>,
    active: Arc<AtomicBool>,
}

impl XamlRuntimeClassRegistration {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn supported_overrides(&self) -> &[String] {
        &self.supported_overrides
    }

    pub fn unregister(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut registrations) = XAML_RUNTIME_CLASSES.lock() else {
            return false;
        };
        if !self.active.swap(false, Ordering::AcqRel) {
            return false;
        }
        if registrations
            .get(&self.name)
            .is_some_and(|entry| entry.id == self.id)
        {
            registrations.remove(&self.name);
            true
        } else {
            false
        }
    }
}

impl Drop for XamlRuntimeClassRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

fn valid_runtime_class_name(name: &str) -> bool {
    let mut count = 0;
    for segment in name.split('.') {
        count += 1;
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !(first == '_' || first.is_alphabetic())
            || chars.any(|character| !(character == '_' || character.is_alphanumeric()))
        {
            return false;
        }
    }
    count >= 2
}

fn valid_override_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|character| character == '_' || character.is_alphanumeric())
}

/// Register a process-local runtime-class name for XAML metadata activation.
///
/// This does not modify the machine/package activation catalog. The class is
/// visible only through dynwinrt's composed `IXamlMetadataProvider`, and its
/// base must be a non-generic WinUI type supplied by the chained provider.
pub fn register_xaml_runtime_class(
    name: &str,
    base_type: &str,
    base_iid: GUID,
    supported_overrides: Vec<String>,
    activator: XamlRuntimeClassActivator,
) -> windows_core::Result<XamlRuntimeClassRegistration> {
    if !valid_runtime_class_name(name)
        || !valid_runtime_class_name(base_type)
        || name == base_type
        || supported_overrides
            .iter()
            .any(|name| !valid_override_name(name))
    {
        return Err(windows_core::Error::from_hresult(E_INVALIDARG));
    }
    let mut supported_overrides = supported_overrides;
    supported_overrides.sort();
    if supported_overrides
        .windows(2)
        .any(|pair| pair[0] == pair[1])
    {
        return Err(windows_core::Error::from_hresult(E_INVALIDARG));
    }

    let id = NEXT_XAML_RUNTIME_CLASS_ID.fetch_add(1, Ordering::Relaxed);
    let active = Arc::new(AtomicBool::new(true));
    let entry = Arc::new(XamlRuntimeClassEntry {
        id,
        name: name.to_string(),
        base_type: base_type.to_string(),
        base_iid,
        activator,
        active: active.clone(),
    });
    let mut registrations = XAML_RUNTIME_CLASSES
        .lock()
        .map_err(|_| windows_core::Error::from_hresult(E_FAIL))?;
    if registrations.contains_key(name) {
        return Err(windows_core::Error::from_hresult(HRESULT_ALREADY_EXISTS));
    }
    registrations.insert(name.to_string(), entry);
    Ok(XamlRuntimeClassRegistration {
        name: name.to_string(),
        id,
        supported_overrides,
        active,
    })
}

fn registered_xaml_runtime_class(
    name: &str,
) -> std::result::Result<Option<Arc<XamlRuntimeClassEntry>>, HRESULT> {
    XAML_RUNTIME_CLASSES
        .lock()
        .map(|registrations| registrations.get(name).cloned())
        .map_err(|_| E_FAIL)
}

#[repr(C)]
struct ApplicationFactoryVtbl {
    base: IInspectableVtbl,
    create_instance: unsafe extern "system" fn(
        this: *mut c_void,
        outer: *mut c_void,
        inner: *mut *mut c_void,
        instance: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct ApplicationOverridesVtbl {
    base: IInspectableVtbl,
    on_launched: unsafe extern "system" fn(this: *mut c_void, args: *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DelegateVtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(
        this: *mut c_void,
        arg: *mut c_void,
        unused: *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct XamlMetadataProviderVtbl {
    base: IInspectableVtbl,
    get_xaml_type: unsafe extern "system" fn(
        this: *mut c_void,
        type_name: AbiTypeName,
        result: *mut *mut c_void,
    ) -> HRESULT,
    get_xaml_type_by_full_name: unsafe extern "system" fn(
        this: *mut c_void,
        full_name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT,
    get_xmlns_definitions: unsafe extern "system" fn(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct XamlTypeVtbl {
    base: IInspectableVtbl,
    get_base_type:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_content_property:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_full_name:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_is_array: unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_is_collection: unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_is_constructible: unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_is_dictionary: unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_is_markup_extension:
        unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_is_bindable: unsafe extern "system" fn(this: *mut c_void, result: *mut u8) -> HRESULT,
    get_item_type:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_key_type: unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_boxed_type:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    get_underlying_type:
        unsafe extern "system" fn(this: *mut c_void, result: *mut AbiTypeName) -> HRESULT,
    activate_instance:
        unsafe extern "system" fn(this: *mut c_void, result: *mut *mut c_void) -> HRESULT,
    create_from_string: unsafe extern "system" fn(
        this: *mut c_void,
        value: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT,
    get_member: unsafe extern "system" fn(
        this: *mut c_void,
        name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT,
    add_to_vector: unsafe extern "system" fn(
        this: *mut c_void,
        instance: *mut c_void,
        value: *mut c_void,
    ) -> HRESULT,
    add_to_map: unsafe extern "system" fn(
        this: *mut c_void,
        instance: *mut c_void,
        key: *mut c_void,
        value: *mut c_void,
    ) -> HRESULT,
    run_initializer: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AbiTypeName {
    name: *mut c_void,
    kind: i32,
}

#[repr(C)]
struct RegisteredXamlType {
    vtable: *const XamlTypeVtbl,
    ref_count: windows_core::imp::WeakRefCount,
    entry: Arc<XamlRuntimeClassEntry>,
    base_type: IUnknown,
    metadata_base: Option<IUnknown>,
}

impl RegisteredXamlType {
    const VTABLE: XamlTypeVtbl = XamlTypeVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: Self::get_iids,
            get_runtime_class_name: Self::get_runtime_class_name,
            get_trust_level: Self::get_trust_level,
        },
        get_base_type: Self::get_base_type,
        get_content_property: Self::get_content_property,
        get_full_name: Self::get_full_name,
        get_is_array: Self::get_is_array,
        get_is_collection: Self::get_is_collection,
        get_is_constructible: Self::get_is_constructible,
        get_is_dictionary: Self::get_is_dictionary,
        get_is_markup_extension: Self::get_is_markup_extension,
        get_is_bindable: Self::get_is_bindable,
        get_item_type: Self::get_item_type,
        get_key_type: Self::get_key_type,
        get_boxed_type: Self::get_boxed_type,
        get_underlying_type: Self::get_underlying_type,
        activate_instance: Self::activate_instance,
        create_from_string: Self::create_from_string,
        get_member: Self::get_member,
        add_to_vector: Self::add_to_vector,
        add_to_map: Self::add_to_map,
        run_initializer: Self::run_initializer,
    };

    fn create(
        entry: Arc<XamlRuntimeClassEntry>,
        base_type: IUnknown,
        metadata_base: Option<IUnknown>,
    ) -> IUnknown {
        let value = Box::new(Self {
            vtable: &Self::VTABLE,
            ref_count: windows_core::imp::WeakRefCount::new(),
            entry,
            base_type,
            metadata_base,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
    }

    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe fn write_object(result: *mut *mut c_void, value: IUnknown) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = value.as_raw();
        std::mem::forget(value);
        S_OK
    }

    unsafe fn write_hstring(result: *mut *mut c_void, value: &str) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        (result as *mut HSTRING).write(HSTRING::from(value));
        S_OK
    }

    unsafe fn metadata_base_vtable(&self) -> Option<(*mut c_void, *const XamlTypeVtbl)> {
        self.metadata_base.as_ref().map(|base| {
            (
                base.as_raw(),
                *(base.as_raw() as *const *const XamlTypeVtbl),
            )
        })
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            if iid.is_null() || result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            if *iid == IUnknown::IID
                || *iid == windows_core::IInspectable::IID
                || *iid == IID_IXAML_TYPE
            {
                *result = this;
                Self::from_ptr(this).ref_count.add_ref();
                S_OK
            } else {
                E_NOINTERFACE
            }
        })) {
            Ok(hr) => hr,
            Err(_) => E_FAIL,
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            Self::from_ptr(this).ref_count.add_ref()
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        catch_unwind(AssertUnwindSafe(|| {
            let remaining = Self::from_ptr(this).ref_count.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }))
        .unwrap_or(0)
    }

    unsafe extern "system" fn get_iids(
        _this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }
        *count = 0;
        *result = std::ptr::null_mut();
        let value = CoTaskMemAlloc(std::mem::size_of::<GUID>()) as *mut GUID;
        if value.is_null() {
            return E_OUTOFMEMORY;
        }
        value.write(IID_IXAML_TYPE);
        *count = 1;
        *result = value;
        S_OK
    }

    unsafe extern "system" fn get_runtime_class_name(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_hstring(result, "DynWinRT.XamlRuntimeClassType")
    }

    unsafe extern "system" fn get_trust_level(_this: *mut c_void, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = 0;
        S_OK
    }

    unsafe extern "system" fn get_base_type(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_object(result, Self::from_ptr(this).base_type.clone())
    }

    unsafe extern "system" fn get_content_property(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        let value = Self::from_ptr(this);
        match value.metadata_base_vtable() {
            Some((base, vtable)) => ((*vtable).get_content_property)(base, result),
            None => S_OK,
        }
    }

    unsafe extern "system" fn get_full_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_hstring(result, &Self::from_ptr(this).entry.name)
    }

    unsafe fn write_bool(result: *mut u8, value: bool) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = u8::from(value);
        S_OK
    }

    unsafe extern "system" fn get_is_array(_this: *mut c_void, result: *mut u8) -> HRESULT {
        Self::write_bool(result, false)
    }

    unsafe extern "system" fn get_is_collection(_this: *mut c_void, result: *mut u8) -> HRESULT {
        Self::write_bool(result, false)
    }

    unsafe extern "system" fn get_is_constructible(_this: *mut c_void, result: *mut u8) -> HRESULT {
        Self::write_bool(result, true)
    }

    unsafe extern "system" fn get_is_dictionary(_this: *mut c_void, result: *mut u8) -> HRESULT {
        Self::write_bool(result, false)
    }

    unsafe extern "system" fn get_is_markup_extension(
        _this: *mut c_void,
        result: *mut u8,
    ) -> HRESULT {
        Self::write_bool(result, false)
    }

    unsafe extern "system" fn get_is_bindable(this: *mut c_void, result: *mut u8) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        let value = Self::from_ptr(this);
        match value.metadata_base_vtable() {
            Some((base, vtable)) => ((*vtable).get_is_bindable)(base, result),
            None => Self::write_bool(result, true),
        }
    }

    unsafe fn write_optional_null(result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        S_OK
    }

    unsafe extern "system" fn get_item_type(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_optional_null(result)
    }

    unsafe extern "system" fn get_key_type(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_optional_null(result)
    }

    unsafe extern "system" fn get_boxed_type(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::write_optional_null(result)
    }

    unsafe extern "system" fn get_underlying_type(
        this: *mut c_void,
        result: *mut AbiTypeName,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        result.write(AbiTypeName {
            name: std::mem::transmute(HSTRING::from(&Self::from_ptr(this).entry.name)),
            kind: TYPE_KIND_CUSTOM,
        });
        S_OK
    }

    unsafe extern "system" fn activate_instance(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        match catch_unwind(AssertUnwindSafe(|| {
            if result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            if !Self::from_ptr(this).entry.active.load(Ordering::Acquire) {
                return RO_E_CLOSED;
            }
            match (Self::from_ptr(this).entry.activator)() {
                Ok(instance) if !instance.as_raw().is_null() => {
                    let mut base = std::ptr::null_mut();
                    let hr = instance.query(&Self::from_ptr(this).entry.base_iid, &mut base);
                    if hr.is_err() || base.is_null() {
                        if !base.is_null() {
                            drop(IUnknown::from_raw(base));
                        }
                        return if hr.is_err() { hr } else { E_NOINTERFACE };
                    }
                    let base = IUnknown::from_raw(base);
                    let Ok(instance_identity) = instance.cast::<IUnknown>() else {
                        return E_NOINTERFACE;
                    };
                    let Ok(base_identity) = base.cast::<IUnknown>() else {
                        return E_NOINTERFACE;
                    };
                    if instance_identity.as_raw() != base_identity.as_raw() {
                        return E_NOINTERFACE;
                    }
                    Self::write_object(result, instance)
                }
                Ok(_) => E_FAIL,
                Err(error) => error.code(),
            }
        })) {
            Ok(hr) => hr,
            Err(_) => E_FAIL,
        }
    }

    unsafe extern "system" fn create_from_string(
        _this: *mut c_void,
        _value: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if !result.is_null() {
            *result = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    unsafe extern "system" fn get_member(
        this: *mut c_void,
        name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        let value = Self::from_ptr(this);
        match value.metadata_base_vtable() {
            Some((base, vtable)) => ((*vtable).get_member)(base, name, result),
            None => S_OK,
        }
    }

    unsafe extern "system" fn add_to_vector(
        _this: *mut c_void,
        _instance: *mut c_void,
        _value: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }

    unsafe extern "system" fn add_to_map(
        _this: *mut c_void,
        _instance: *mut c_void,
        _key: *mut c_void,
        _value: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }

    unsafe extern "system" fn run_initializer(_this: *mut c_void) -> HRESULT {
        S_OK
    }
}

/// Minimal IXamlType reference used for WinUI built-in base types that are not
/// surfaced by XamlControlsXamlMetaDataProvider. WinUI resolves its
/// UnderlyingType against the built-in metadata table; it is never activatable.
#[repr(C)]
struct ReferencedXamlType {
    vtable: *const XamlTypeVtbl,
    ref_count: windows_core::imp::WeakRefCount,
    name: String,
}

impl ReferencedXamlType {
    const VTABLE: XamlTypeVtbl = XamlTypeVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface,
                AddRef: Self::add_ref,
                Release: Self::release,
            },
            get_iids: RegisteredXamlType::get_iids,
            get_runtime_class_name: RegisteredXamlType::get_runtime_class_name,
            get_trust_level: RegisteredXamlType::get_trust_level,
        },
        get_base_type: Self::get_object,
        get_content_property: Self::get_object,
        get_full_name: Self::get_full_name,
        get_is_array: Self::get_false,
        get_is_collection: Self::get_false,
        get_is_constructible: Self::get_false,
        get_is_dictionary: Self::get_false,
        get_is_markup_extension: Self::get_false,
        get_is_bindable: Self::get_true,
        get_item_type: Self::get_object,
        get_key_type: Self::get_object,
        get_boxed_type: Self::get_object,
        get_underlying_type: Self::get_underlying_type,
        activate_instance: Self::activate_instance,
        create_from_string: Self::create_from_string,
        get_member: Self::get_member,
        add_to_vector: RegisteredXamlType::add_to_vector,
        add_to_map: RegisteredXamlType::add_to_map,
        run_initializer: RegisteredXamlType::run_initializer,
    };

    fn create(name: String) -> IUnknown {
        let value = Box::new(Self {
            vtable: &Self::VTABLE,
            ref_count: windows_core::imp::WeakRefCount::new(),
            name,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
    }

    unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe extern "system" fn query_interface(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        if *iid == IUnknown::IID
            || *iid == windows_core::IInspectable::IID
            || *iid == IID_IXAML_TYPE
        {
            *result = this;
            Self::from_ptr(this).ref_count.add_ref();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
        Self::from_ptr(this).ref_count.add_ref()
    }

    unsafe extern "system" fn release(this: *mut c_void) -> u32 {
        let remaining = Self::from_ptr(this).ref_count.release();
        if remaining == 0 {
            drop(Box::from_raw(this as *mut Self));
        }
        remaining
    }

    unsafe extern "system" fn get_object(_this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
        RegisteredXamlType::write_optional_null(result)
    }

    unsafe extern "system" fn get_full_name(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        RegisteredXamlType::write_hstring(result, &Self::from_ptr(this).name)
    }

    unsafe extern "system" fn get_false(_this: *mut c_void, result: *mut u8) -> HRESULT {
        RegisteredXamlType::write_bool(result, false)
    }

    unsafe extern "system" fn get_true(_this: *mut c_void, result: *mut u8) -> HRESULT {
        RegisteredXamlType::write_bool(result, true)
    }

    unsafe extern "system" fn get_underlying_type(
        this: *mut c_void,
        result: *mut AbiTypeName,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        result.write(AbiTypeName {
            name: std::mem::transmute(HSTRING::from(&Self::from_ptr(this).name)),
            kind: TYPE_KIND_METADATA,
        });
        S_OK
    }

    unsafe extern "system" fn activate_instance(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if !result.is_null() {
            *result = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    unsafe extern "system" fn create_from_string(
        _this: *mut c_void,
        _value: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::activate_instance(std::ptr::null_mut(), result)
    }

    unsafe extern "system" fn get_member(
        _this: *mut c_void,
        _name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        RegisteredXamlType::write_optional_null(result)
    }
}

#[repr(C)]
struct XamlApplicationHost {
    vtable_overrides: *const ApplicationOverridesVtbl,
    vtable_metadata: *const XamlMetadataProviderVtbl,
    state: CompositionState,
    metadata_provider: IUnknown,
    launched_callback: Option<IUnknown>,
}

// Safety: local COM fields are immutable IUnknown handles and CompositionState
// protects its inner. This does not expose IAgileObject; WinUI remains STA-bound.
unsafe impl Send for XamlApplicationHost {}
unsafe impl Sync for XamlApplicationHost {}

impl XamlApplicationHost {
    const OVERRIDES_VTBL: ApplicationOverridesVtbl = ApplicationOverridesVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface_overrides,
                AddRef: Self::add_ref_overrides,
                Release: Self::release_overrides,
            },
            get_iids: Self::get_iids_overrides,
            get_runtime_class_name: Self::get_runtime_class_name_overrides,
            get_trust_level: Self::get_trust_level_overrides,
        },
        on_launched: Self::on_launched,
    };

    const METADATA_VTBL: XamlMetadataProviderVtbl = XamlMetadataProviderVtbl {
        base: IInspectableVtbl {
            base: windows_core::IUnknown_Vtbl {
                QueryInterface: Self::query_interface_metadata,
                AddRef: Self::add_ref_metadata,
                Release: Self::release_metadata,
            },
            get_iids: Self::get_iids_metadata,
            get_runtime_class_name: Self::get_runtime_class_name_metadata,
            get_trust_level: Self::get_trust_level_metadata,
        },
        get_xaml_type: Self::get_xaml_type,
        get_xaml_type_by_full_name: Self::get_xaml_type_by_full_name,
        get_xmlns_definitions: Self::get_xmlns_definitions,
    };

    fn new(metadata_provider: IUnknown, launched_callback: Option<IUnknown>) -> Box<Self> {
        Box::new(Self {
            vtable_overrides: &Self::OVERRIDES_VTBL,
            vtable_metadata: &Self::METADATA_VTBL,
            state: CompositionState::new(true),
            metadata_provider,
            launched_callback,
        })
    }

    unsafe fn from_overrides_ptr(this: *mut c_void) -> &'static Self {
        &*(this as *const Self)
    }

    unsafe fn from_metadata_ptr(this: *mut c_void) -> &'static Self {
        let base = (this as *const *const c_void).sub(1) as *const Self;
        &*base
    }

    fn identity_ptr(&self) -> *mut c_void {
        self as *const Self as *mut c_void
    }

    fn metadata_ptr(&self) -> *mut c_void {
        unsafe { (self.identity_ptr() as *mut *mut c_void).add(1) as *mut c_void }
    }

    unsafe extern "system" fn query_interface_overrides(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::query_interface_impl(Self::from_overrides_ptr(this), iid, result)
    }

    unsafe extern "system" fn query_interface_metadata(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::query_interface_impl(Self::from_metadata_ptr(this), iid, result)
    }

    unsafe fn query_interface_impl(
        host: &Self,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        host.state
            .query_interface(host.identity_ptr(), iid, result, |iid| {
                if *iid == IID_IAPPLICATION_OVERRIDES {
                    Some(host.identity_ptr())
                } else if *iid == IID_IXAML_METADATA_PROVIDER {
                    Some(host.metadata_ptr())
                } else {
                    None
                }
            })
    }

    unsafe extern "system" fn add_ref_overrides(this: *mut c_void) -> u32 {
        Self::from_overrides_ptr(this).state.add_ref()
    }

    unsafe extern "system" fn add_ref_metadata(this: *mut c_void) -> u32 {
        Self::from_metadata_ptr(this).state.add_ref()
    }

    unsafe extern "system" fn release_overrides(this: *mut c_void) -> u32 {
        Self::release_impl(Self::from_overrides_ptr(this))
    }

    unsafe extern "system" fn release_metadata(this: *mut c_void) -> u32 {
        Self::release_impl(Self::from_metadata_ptr(this))
    }

    unsafe fn release_impl(host: &Self) -> u32 {
        let remaining = host.state.release();
        if remaining == 0 {
            drop(Box::from_raw(host.identity_ptr() as *mut Self));
        }
        remaining
    }

    unsafe extern "system" fn get_iids_overrides(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        Self::get_iids_impl(Self::from_overrides_ptr(this), count, result)
    }

    unsafe extern "system" fn get_iids_metadata(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        Self::get_iids_impl(Self::from_metadata_ptr(this), count, result)
    }

    unsafe fn get_iids_impl(host: &Self, count: *mut u32, result: *mut *mut GUID) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }

        host.state.get_iids(
            &[IID_IAPPLICATION_OVERRIDES, IID_IXAML_METADATA_PROVIDER],
            count,
            result,
        )
    }

    unsafe extern "system" fn get_runtime_class_name_overrides(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::get_runtime_class_name_impl(Self::from_overrides_ptr(this), result)
    }

    unsafe extern "system" fn get_runtime_class_name_metadata(
        this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        Self::get_runtime_class_name_impl(Self::from_metadata_ptr(this), result)
    }

    unsafe fn get_runtime_class_name_impl(host: &Self, result: *mut *mut c_void) -> HRESULT {
        host.state.get_runtime_class_name(result)
    }

    unsafe extern "system" fn get_trust_level_overrides(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        Self::get_trust_level_impl(Self::from_overrides_ptr(this), result)
    }

    unsafe extern "system" fn get_trust_level_metadata(
        this: *mut c_void,
        result: *mut i32,
    ) -> HRESULT {
        Self::get_trust_level_impl(Self::from_metadata_ptr(this), result)
    }

    unsafe fn get_trust_level_impl(host: &Self, result: *mut i32) -> HRESULT {
        host.state.get_trust_level(result)
    }

    unsafe extern "system" fn on_launched(this: *mut c_void, args: *mut c_void) -> HRESULT {
        let host = Self::from_overrides_ptr(this);
        let Some(callback) = host.launched_callback.as_ref() else {
            return S_OK;
        };
        let callback_ptr = callback.as_raw();
        let vtable = *(callback_ptr as *const *const DelegateVtbl);
        ((*vtable).invoke)(callback_ptr, args, std::ptr::null_mut())
    }

    unsafe extern "system" fn get_xaml_type(
        this: *mut c_void,
        type_name: AbiTypeName,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        let name = hstring_from_abi(type_name.name);
        match registered_xaml_runtime_class(&name) {
            Ok(Some(entry)) => {
                if type_name.kind != TYPE_KIND_CUSTOM {
                    return E_INVALIDARG;
                }
                return create_registered_xaml_type(&host.metadata_provider, entry, result);
            }
            Ok(None) => {}
            Err(error) => return error,
        }
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xaml_type)(provider, type_name, result)
    }

    unsafe extern "system" fn get_xaml_type_by_full_name(
        this: *mut c_void,
        full_name: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        if result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        let name = hstring_from_abi(full_name);
        match registered_xaml_runtime_class(&name) {
            Ok(Some(entry)) => {
                return create_registered_xaml_type(&host.metadata_provider, entry, result);
            }
            Ok(None) => {}
            Err(error) => return error,
        }
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xaml_type_by_full_name)(provider, full_name, result)
    }

    unsafe extern "system" fn get_xmlns_definitions(
        this: *mut c_void,
        count: *mut u32,
        result: *mut *mut c_void,
    ) -> HRESULT {
        let host = Self::from_metadata_ptr(this);
        let provider = host.metadata_provider.as_raw();
        let vtable = *(provider as *const *const XamlMetadataProviderVtbl);
        ((*vtable).get_xmlns_definitions)(provider, count, result)
    }
}

unsafe fn hstring_from_abi(value: *mut c_void) -> String {
    let value: &HSTRING = &*(&value as *const *mut c_void as *const HSTRING);
    value.to_string()
}

unsafe fn create_registered_xaml_type(
    provider: &IUnknown,
    entry: Arc<XamlRuntimeClassEntry>,
    result: *mut *mut c_void,
) -> HRESULT {
    if result.is_null() {
        return E_POINTER;
    }
    *result = std::ptr::null_mut();
    let provider_ptr = provider.as_raw();
    let provider_vtable = *(provider_ptr as *const *const XamlMetadataProviderVtbl);
    let base_name = HSTRING::from(&entry.base_type);
    let mut base = std::ptr::null_mut();
    let hr = ((*provider_vtable).get_xaml_type_by_full_name)(
        provider_ptr,
        std::mem::transmute_copy(&base_name),
        &mut base,
    );
    if hr.is_err() {
        if !base.is_null() {
            drop(IUnknown::from_raw(base));
        }
        return hr;
    }
    let (base_type, metadata_base) = if base.is_null() {
        (ReferencedXamlType::create(entry.base_type.clone()), None)
    } else {
        let base = IUnknown::from_raw(base);
        let mut base_type = std::ptr::null_mut();
        let hr = base.query(&IID_IXAML_TYPE, &mut base_type);
        if hr.is_err() || base_type.is_null() {
            if !base_type.is_null() {
                drop(IUnknown::from_raw(base_type));
            }
            return if hr.is_err() { hr } else { E_NOINTERFACE };
        }
        let base_type = IUnknown::from_raw(base_type);

        let vtable = *(base_type.as_raw() as *const *const XamlTypeVtbl);
        for getter in [
            (*vtable).get_is_array,
            (*vtable).get_is_collection,
            (*vtable).get_is_dictionary,
            (*vtable).get_is_markup_extension,
        ] {
            let mut unsupported = 0;
            let hr = getter(base_type.as_raw(), &mut unsupported);
            if hr.is_err() {
                return hr;
            }
            if unsupported != 0 {
                return E_INVALIDARG;
            }
        }
        (base_type.clone(), Some(base_type))
    };

    let xaml_type = RegisteredXamlType::create(entry, base_type, metadata_base);
    *result = xaml_type.as_raw();
    std::mem::forget(xaml_type);
    S_OK
}

fn query_interface(object: &IUnknown, iid: &GUID) -> windows_core::Result<IUnknown> {
    let mut result = std::ptr::null_mut();
    unsafe {
        object.query(iid, &mut result).ok()?;
        Ok(IUnknown::from_raw(result))
    }
}

fn enable_per_monitor_v2() -> windows_core::Result<()> {
    // WinUI popup hosts consult the process context, so it must match the UI thread.
    let process_context = unsafe { GetDpiAwarenessContextForProcess(GetCurrentProcess()) };
    if !unsafe {
        AreDpiAwarenessContextsEqual(process_context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .as_bool()
    } {
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)? };
    }

    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        Err(windows_core::Error::from_thread())
    } else {
        Ok(())
    }
}

struct InitialHostRef(*mut XamlApplicationHost);

impl Drop for InitialHostRef {
    fn drop(&mut self) {
        unsafe {
            XamlApplicationHost::release_impl(&*self.0);
        }
    }
}

fn compose_xaml_application(
    factory: &IUnknown,
    metadata_provider: IUnknown,
    launched_callback: Option<IUnknown>,
) -> windows_core::Result<IUnknown> {
    let host = XamlApplicationHost::new(metadata_provider, launched_callback);
    let host_ptr = Box::into_raw(host);
    let _initial_ref = InitialHostRef(host_ptr);
    let outer = host_ptr as *mut c_void;
    let mut inner = std::ptr::null_mut();
    let mut instance = std::ptr::null_mut();

    let vtable = unsafe { *(factory.as_raw() as *const *const ApplicationFactoryVtbl) };
    let hr =
        unsafe { ((*vtable).create_instance)(factory.as_raw(), outer, &mut inner, &mut instance) };
    if hr.is_err() {
        unsafe {
            if !instance.is_null() {
                drop(IUnknown::from_raw(instance));
            }
            if !inner.is_null() {
                drop(IUnknown::from_raw(inner));
            }
        }
        return Err(windows_core::Error::from_hresult(hr));
    }
    if inner.is_null() || instance.is_null() {
        unsafe {
            if !instance.is_null() {
                drop(IUnknown::from_raw(instance));
            }
            if !inner.is_null() {
                drop(IUnknown::from_raw(inner));
            }
        }
        return Err(windows_core::Error::from_hresult(E_POINTER));
    }

    unsafe {
        let inner = IUnknown::from_raw(inner);
        let application = IUnknown::from_raw(instance);
        (*host_ptr).state.set_inner(inner)?;
        Ok(application)
    }
}

/// Create a composed WinUI `Application` whose outer object exposes the supplied
/// `IXamlMetadataProvider`.
pub fn create_xaml_application(
    metadata_provider: &IUnknown,
    launched_callback: Option<&IUnknown>,
) -> Result<WinRTValue> {
    enable_per_monitor_v2()?;
    let provider = query_interface(metadata_provider, &IID_IXAML_METADATA_PROVIDER)?;
    let activation_factory =
        crate::ro_get_activation_factory_2(&HSTRING::from("Microsoft.UI.Xaml.Application"))?;
    let factory = activation_factory
        .as_object()
        .expect("activation factory must be an object");
    let factory = query_interface(&factory, &IID_IAPPLICATION_FACTORY)?;
    let callback = launched_callback
        .map(|callback| query_interface(callback, &IID_XAML_LAUNCHED_CALLBACK))
        .transpose()?;
    let application = compose_xaml_application(&factory, provider, callback)?;
    Ok(WinRTValue::Object(application))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use windows::Win32::UI::HiDpi::GetThreadDpiAwarenessContext;

    #[repr(C)]
    struct TestInspectable {
        vtable: *const IInspectableVtbl,
        ref_count: windows_core::imp::WeakRefCount,
    }

    unsafe extern "system" fn test_qi(
        this: *mut c_void,
        iid: *const GUID,
        result: *mut *mut c_void,
    ) -> HRESULT {
        if iid.is_null() || result.is_null() {
            return E_POINTER;
        }
        *result = std::ptr::null_mut();
        if *iid == IUnknown::IID || *iid == windows_core::IInspectable::IID {
            *result = this;
            (*(this as *const TestInspectable)).ref_count.add_ref();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn test_add_ref(this: *mut c_void) -> u32 {
        (*(this as *const TestInspectable)).ref_count.add_ref()
    }

    unsafe extern "system" fn test_release(this: *mut c_void) -> u32 {
        let remaining = (*(this as *const TestInspectable)).ref_count.release();
        if remaining == 0 {
            drop(Box::from_raw(this as *mut TestInspectable));
        }
        remaining
    }

    unsafe extern "system" fn test_get_iids(
        _this: *mut c_void,
        count: *mut u32,
        result: *mut *mut GUID,
    ) -> HRESULT {
        if count.is_null() || result.is_null() {
            return E_POINTER;
        }
        *count = 0;
        *result = std::ptr::null_mut();
        S_OK
    }

    unsafe extern "system" fn test_runtime_name(
        _this: *mut c_void,
        result: *mut *mut c_void,
    ) -> HRESULT {
        RegisteredXamlType::write_hstring(result, "DynWinRT.TestInspectable")
    }

    unsafe extern "system" fn test_trust(_this: *mut c_void, result: *mut i32) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        *result = 0;
        S_OK
    }

    static TEST_INSPECTABLE_VTABLE: IInspectableVtbl = IInspectableVtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: test_qi,
            AddRef: test_add_ref,
            Release: test_release,
        },
        get_iids: test_get_iids,
        get_runtime_class_name: test_runtime_name,
        get_trust_level: test_trust,
    };

    fn test_inspectable() -> IUnknown {
        let value = Box::new(TestInspectable {
            vtable: &TEST_INSPECTABLE_VTABLE,
            ref_count: windows_core::imp::WeakRefCount::new(),
        });
        unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
    }

    #[repr(C)]
    struct TestBaseXamlType {
        vtable: *const XamlTypeVtbl,
        ref_count: windows_core::imp::WeakRefCount,
    }

    impl TestBaseXamlType {
        unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
            &*(this as *const Self)
        }

        unsafe extern "system" fn qi(
            this: *mut c_void,
            iid: *const GUID,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if iid.is_null() || result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            if *iid == IUnknown::IID
                || *iid == windows_core::IInspectable::IID
                || *iid == IID_IXAML_TYPE
            {
                *result = this;
                Self::from_ptr(this).ref_count.add_ref();
                S_OK
            } else {
                E_NOINTERFACE
            }
        }

        unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
            Self::from_ptr(this).ref_count.add_ref()
        }

        unsafe extern "system" fn release(this: *mut c_void) -> u32 {
            let remaining = Self::from_ptr(this).ref_count.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }

        unsafe extern "system" fn object(_this: *mut c_void, result: *mut *mut c_void) -> HRESULT {
            RegisteredXamlType::write_optional_null(result)
        }

        unsafe extern "system" fn full_name(
            _this: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            RegisteredXamlType::write_hstring(result, "Microsoft.UI.Xaml.Controls.Control")
        }

        unsafe extern "system" fn false_value(_this: *mut c_void, result: *mut u8) -> HRESULT {
            RegisteredXamlType::write_bool(result, false)
        }

        unsafe extern "system" fn true_value(_this: *mut c_void, result: *mut u8) -> HRESULT {
            RegisteredXamlType::write_bool(result, true)
        }

        unsafe extern "system" fn underlying(
            _this: *mut c_void,
            result: *mut AbiTypeName,
        ) -> HRESULT {
            if result.is_null() {
                return E_POINTER;
            }
            result.write(AbiTypeName {
                name: std::mem::transmute(HSTRING::from("Microsoft.UI.Xaml.Controls.Control")),
                kind: TYPE_KIND_CUSTOM,
            });
            S_OK
        }

        unsafe extern "system" fn activate(
            _this: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if !result.is_null() {
                *result = std::ptr::null_mut();
            }
            E_NOTIMPL
        }

        unsafe extern "system" fn create_from_string(
            _this: *mut c_void,
            _value: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            Self::activate(std::ptr::null_mut(), result)
        }

        unsafe extern "system" fn get_member(
            _this: *mut c_void,
            _name: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            RegisteredXamlType::write_optional_null(result)
        }

        unsafe extern "system" fn add_vector(
            _this: *mut c_void,
            _instance: *mut c_void,
            _value: *mut c_void,
        ) -> HRESULT {
            E_NOTIMPL
        }

        unsafe extern "system" fn add_map(
            _this: *mut c_void,
            _instance: *mut c_void,
            _key: *mut c_void,
            _value: *mut c_void,
        ) -> HRESULT {
            E_NOTIMPL
        }

        unsafe extern "system" fn initialize(_this: *mut c_void) -> HRESULT {
            S_OK
        }

        const VTABLE: XamlTypeVtbl = XamlTypeVtbl {
            base: IInspectableVtbl {
                base: windows_core::IUnknown_Vtbl {
                    QueryInterface: Self::qi,
                    AddRef: Self::add_ref,
                    Release: Self::release,
                },
                get_iids: test_get_iids,
                get_runtime_class_name: test_runtime_name,
                get_trust_level: test_trust,
            },
            get_base_type: Self::object,
            get_content_property: Self::object,
            get_full_name: Self::full_name,
            get_is_array: Self::false_value,
            get_is_collection: Self::false_value,
            get_is_constructible: Self::true_value,
            get_is_dictionary: Self::false_value,
            get_is_markup_extension: Self::false_value,
            get_is_bindable: Self::true_value,
            get_item_type: Self::object,
            get_key_type: Self::object,
            get_boxed_type: Self::object,
            get_underlying_type: Self::underlying,
            activate_instance: Self::activate,
            create_from_string: Self::create_from_string,
            get_member: Self::get_member,
            add_to_vector: Self::add_vector,
            add_to_map: Self::add_map,
            run_initializer: Self::initialize,
        };

        fn create() -> IUnknown {
            let value = Box::new(Self {
                vtable: &Self::VTABLE,
                ref_count: windows_core::imp::WeakRefCount::new(),
            });
            unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
        }
    }

    #[repr(C)]
    struct TestMetadataProvider {
        vtable: *const XamlMetadataProviderVtbl,
        ref_count: windows_core::imp::WeakRefCount,
        base_type: IUnknown,
    }

    impl TestMetadataProvider {
        unsafe fn from_ptr(this: *mut c_void) -> &'static Self {
            &*(this as *const Self)
        }

        unsafe extern "system" fn qi(
            this: *mut c_void,
            iid: *const GUID,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if iid.is_null() || result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            if *iid == IUnknown::IID
                || *iid == windows_core::IInspectable::IID
                || *iid == IID_IXAML_METADATA_PROVIDER
            {
                *result = this;
                Self::from_ptr(this).ref_count.add_ref();
                S_OK
            } else {
                E_NOINTERFACE
            }
        }

        unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
            Self::from_ptr(this).ref_count.add_ref()
        }

        unsafe extern "system" fn release(this: *mut c_void) -> u32 {
            let remaining = Self::from_ptr(this).ref_count.release();
            if remaining == 0 {
                drop(Box::from_raw(this as *mut Self));
            }
            remaining
        }

        unsafe fn get(this: *mut c_void, name: &str, result: *mut *mut c_void) -> HRESULT {
            if result.is_null() {
                return E_POINTER;
            }
            *result = std::ptr::null_mut();
            if name != "Microsoft.UI.Xaml.Controls.Control" {
                return S_OK;
            }
            let base = Self::from_ptr(this).base_type.clone();
            *result = base.as_raw();
            std::mem::forget(base);
            S_OK
        }

        unsafe extern "system" fn get_type(
            this: *mut c_void,
            name: AbiTypeName,
            result: *mut *mut c_void,
        ) -> HRESULT {
            Self::get(this, &hstring_from_abi(name.name), result)
        }

        unsafe extern "system" fn get_name(
            this: *mut c_void,
            name: *mut c_void,
            result: *mut *mut c_void,
        ) -> HRESULT {
            Self::get(this, &hstring_from_abi(name), result)
        }

        unsafe extern "system" fn get_xmlns(
            _this: *mut c_void,
            count: *mut u32,
            result: *mut *mut c_void,
        ) -> HRESULT {
            if count.is_null() || result.is_null() {
                return E_POINTER;
            }
            *count = 0;
            *result = std::ptr::null_mut();
            S_OK
        }

        const VTABLE: XamlMetadataProviderVtbl = XamlMetadataProviderVtbl {
            base: IInspectableVtbl {
                base: windows_core::IUnknown_Vtbl {
                    QueryInterface: Self::qi,
                    AddRef: Self::add_ref,
                    Release: Self::release,
                },
                get_iids: test_get_iids,
                get_runtime_class_name: test_runtime_name,
                get_trust_level: test_trust,
            },
            get_xaml_type: Self::get_type,
            get_xaml_type_by_full_name: Self::get_name,
            get_xmlns_definitions: Self::get_xmlns,
        };

        fn create() -> IUnknown {
            let value = Box::new(Self {
                vtable: &Self::VTABLE,
                ref_count: windows_core::imp::WeakRefCount::new(),
                base_type: TestBaseXamlType::create(),
            });
            unsafe { IUnknown::from_raw(Box::into_raw(value).cast()) }
        }
    }

    fn test_registration(
        name: &str,
        activator: XamlRuntimeClassActivator,
    ) -> XamlRuntimeClassRegistration {
        register_xaml_runtime_class(
            name,
            "Microsoft.UI.Xaml.Controls.Control",
            windows_core::IInspectable::IID,
            vec!["measure_override".to_string()],
            activator,
        )
        .unwrap()
    }

    #[test]
    fn registration_rejects_duplicates_and_unsupported_shapes() {
        let activator: XamlRuntimeClassActivator = Arc::new(|| Ok(test_inspectable()));
        assert!(
            register_xaml_runtime_class(
                "NotQualified",
                "Microsoft.UI.Xaml.Controls.Control",
                windows_core::IInspectable::IID,
                Vec::new(),
                activator.clone(),
            )
            .is_err()
        );
        assert!(
            register_xaml_runtime_class(
                "DynWinRT.Tests.BadGeneric`1",
                "Microsoft.UI.Xaml.Controls.Control",
                windows_core::IInspectable::IID,
                Vec::new(),
                activator.clone(),
            )
            .is_err()
        );
        assert!(
            register_xaml_runtime_class(
                "DynWinRT.Tests.DuplicateOverride",
                "Microsoft.UI.Xaml.Controls.Control",
                windows_core::IInspectable::IID,
                vec!["measure_override".into(), "measure_override".into()],
                activator.clone(),
            )
            .is_err()
        );

        let registration = test_registration("DynWinRT.Tests.Duplicate", activator.clone());
        assert!(
            register_xaml_runtime_class(
                "DynWinRT.Tests.Duplicate",
                "Microsoft.UI.Xaml.Controls.Control",
                windows_core::IInspectable::IID,
                Vec::new(),
                activator,
            )
            .is_err()
        );
        assert!(registration.unregister());
        assert!(!registration.unregister());
    }

    #[test]
    fn unregister_releases_callback_and_removes_lookup() {
        struct DropMarker(Arc<AtomicUsize>);
        impl Drop for DropMarker {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let marker = DropMarker(drops.clone());
        let activator: XamlRuntimeClassActivator = Arc::new(move || {
            let _ = &marker;
            Ok(test_inspectable())
        });
        let registration = test_registration("DynWinRT.Tests.Lifetime", activator);
        assert!(
            registered_xaml_runtime_class("DynWinRT.Tests.Lifetime")
                .unwrap()
                .is_some()
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(registration.unregister());
        assert!(
            registered_xaml_runtime_class("DynWinRT.Tests.Lifetime")
                .unwrap()
                .is_none()
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn provider_lookup_activates_with_controlling_identity_and_unregisters() {
        let instance = test_inspectable();
        let expected_identity = instance.as_raw();
        let instance_ptr = instance.as_raw() as usize;
        let activator: XamlRuntimeClassActivator = Arc::new(move || unsafe {
            let pointer = instance_ptr as *mut c_void;
            Ok(IUnknown::from_raw_borrowed(&pointer).unwrap().clone())
        });
        let registration = test_registration("DynWinRT.Tests.NamedControl", activator);
        let host = XamlApplicationHost::new(TestMetadataProvider::create(), None);
        let metadata = host.metadata_ptr();
        let name = HSTRING::from("DynWinRT.Tests.NamedControl");
        let mut xaml_type = std::ptr::null_mut();
        let hr = unsafe {
            (XamlApplicationHost::METADATA_VTBL.get_xaml_type_by_full_name)(
                metadata,
                std::mem::transmute_copy(&name),
                &mut xaml_type,
            )
        };
        assert!(hr.is_ok());
        assert!(!xaml_type.is_null());

        let vtable = unsafe { *(xaml_type as *const *const XamlTypeVtbl) };
        let mut full_name = HSTRING::new();
        assert!(
            unsafe {
                ((*vtable).get_full_name)(
                    xaml_type,
                    (&mut full_name as *mut HSTRING).cast::<*mut c_void>(),
                )
            }
            .is_ok()
        );
        assert_eq!(full_name, "DynWinRT.Tests.NamedControl");

        let mut activated = std::ptr::null_mut();
        assert!(unsafe { ((*vtable).activate_instance)(xaml_type, &mut activated) }.is_ok());
        assert!(!activated.is_null());
        let activated = unsafe { IUnknown::from_raw(activated) };
        assert_eq!(activated.as_raw(), expected_identity);

        unsafe { drop(IUnknown::from_raw(xaml_type)) };
        let mut by_type_name = std::ptr::null_mut();
        let hr = unsafe {
            (XamlApplicationHost::METADATA_VTBL.get_xaml_type)(
                metadata,
                AbiTypeName {
                    name: std::mem::transmute_copy(&name),
                    kind: TYPE_KIND_CUSTOM,
                },
                &mut by_type_name,
            )
        };
        assert!(hr.is_ok());
        assert!(!by_type_name.is_null());

        let mut unsupported_kind = 1usize as *mut c_void;
        let hr = unsafe {
            (XamlApplicationHost::METADATA_VTBL.get_xaml_type)(
                metadata,
                AbiTypeName {
                    name: std::mem::transmute_copy(&name),
                    kind: TYPE_KIND_METADATA,
                },
                &mut unsupported_kind,
            )
        };
        assert_eq!(hr, E_INVALIDARG);
        assert!(unsupported_kind.is_null());

        assert!(registration.unregister());
        let cached_vtable = unsafe { *(by_type_name as *const *const XamlTypeVtbl) };
        let mut cached_activation = std::ptr::null_mut();
        assert_eq!(
            unsafe { ((*cached_vtable).activate_instance)(by_type_name, &mut cached_activation,) },
            RO_E_CLOSED
        );
        assert!(cached_activation.is_null());
        unsafe { drop(IUnknown::from_raw(by_type_name)) };
        let mut missing = std::ptr::null_mut();
        let hr = unsafe {
            (XamlApplicationHost::METADATA_VTBL.get_xaml_type_by_full_name)(
                metadata,
                std::mem::transmute_copy(&name),
                &mut missing,
            )
        };
        assert!(hr.is_ok());
        assert!(missing.is_null());
    }

    #[test]
    fn activation_errors_fail_without_success_shaped_nulls() {
        let activator: XamlRuntimeClassActivator =
            Arc::new(|| Err(windows_core::Error::from_hresult(E_INVALIDARG)));
        let registration = test_registration("DynWinRT.Tests.FailingControl", activator);
        let entry = registered_xaml_runtime_class("DynWinRT.Tests.FailingControl")
            .unwrap()
            .unwrap();
        let base = TestBaseXamlType::create();
        let xaml_type = RegisteredXamlType::create(entry, base.clone(), Some(base));
        let vtable = unsafe { *(xaml_type.as_raw() as *const *const XamlTypeVtbl) };
        let mut result = 1usize as *mut c_void;
        let hr = unsafe { ((*vtable).activate_instance)(xaml_type.as_raw(), &mut result) };
        assert_eq!(hr, E_INVALIDARG);
        assert!(result.is_null());
        assert!(registration.unregister());

        let wrong_base = register_xaml_runtime_class(
            "DynWinRT.Tests.WrongBase",
            "Microsoft.UI.Xaml.Controls.Control",
            GUID::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee),
            Vec::new(),
            Arc::new(|| Ok(test_inspectable())),
        )
        .unwrap();
        let entry = registered_xaml_runtime_class("DynWinRT.Tests.WrongBase")
            .unwrap()
            .unwrap();
        let base = TestBaseXamlType::create();
        let xaml_type = RegisteredXamlType::create(entry, base.clone(), Some(base));
        let vtable = unsafe { *(xaml_type.as_raw() as *const *const XamlTypeVtbl) };
        let mut result = 1usize as *mut c_void;
        let hr = unsafe { ((*vtable).activate_instance)(xaml_type.as_raw(), &mut result) };
        assert_eq!(hr, E_NOINTERFACE);
        assert!(result.is_null());
        assert!(wrong_base.unregister());
    }

    #[test]
    fn enables_per_monitor_v2_on_the_process_and_ui_thread() {
        enable_per_monitor_v2().unwrap();

        let process = unsafe { GetDpiAwarenessContextForProcess(GetCurrentProcess()) };
        let current = unsafe { GetThreadDpiAwarenessContext() };
        assert!(unsafe {
            AreDpiAwarenessContextsEqual(process, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .as_bool()
        });
        assert!(unsafe {
            AreDpiAwarenessContextsEqual(current, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .as_bool()
        });
    }
}
