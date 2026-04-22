// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows::Win32::System::WinRT::{IActivationFactory, RoGetActivationFactory};
use windows::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};
use windows_core::{HSTRING, HRESULT, IUnknown, Interface, PCSTR};

use crate::value::WinRTValue;

#[allow(dead_code)]
pub fn ro_get_activation_factory(class_name: &HSTRING) -> windows_core::Result<IActivationFactory> {
    unsafe { RoGetActivationFactory::<IActivationFactory>(class_name) }
}
pub fn ro_get_activation_factory_2(class_name: &HSTRING) -> crate::result::Result<WinRTValue> {
    let r = unsafe { RoGetActivationFactory::<IActivationFactory>(class_name) };
    match r {
        Ok(factory) => {
            let ukn = unsafe { IUnknown::from_raw(factory.as_raw()) };
            std::mem::forget(factory);
            Ok(WinRTValue::Object(ukn))
        }
        Err(_) => {
            // Fallback: C++/WinRT-style DLL probing.
            // For class "A.B.C.ClassName", try loading "A.B.C.dll", "A.B.dll", "A.dll"
            // and call DllGetActivationFactory to obtain the factory.
            if let Some(val) = dll_get_activation_factory_fallback(class_name) {
                return Ok(val);
            }
            // If fallback also failed, return the original error
            let r2 = unsafe { RoGetActivationFactory::<IActivationFactory>(class_name) };
            match r2 {
                Ok(factory) => {
                    let ukn = unsafe { IUnknown::from_raw(factory.as_raw()) };
                    std::mem::forget(factory);
                    Ok(WinRTValue::Object(ukn))
                }
                Err(e) => Err(crate::result::Error::WindowsError(e)),
            }
        }
    }
}

/// C++/WinRT-style fallback: probe DLLs by trimming the class name at each '.'
/// and calling DllGetActivationFactory. This enables regfree WinRT activation
/// for WinAppSDK classes whose factories aren't in the COM registry.
///
/// Mirrors the logic in C++/WinRT base.h `get_runtime_activation_factory_impl`
/// (lines 6116-6156): for class "A.B.C.Name", tries loading "A.B.C.dll",
/// "A.B.dll", "A.dll" in order and calls DllGetActivationFactory on each.
fn dll_get_activation_factory_fallback(class_name: &HSTRING) -> Option<WinRTValue> {
    use windows::Win32::Foundation::FreeLibrary;

    type DllGetActivationFactoryFn = unsafe extern "system" fn(
        class_id: *mut std::ffi::c_void,  // HSTRING
        factory: *mut *mut std::ffi::c_void,  // IActivationFactory**
    ) -> HRESULT;

    let mut path = class_name.to_string();

    while let Some(pos) = path.rfind('.') {
        path.truncate(pos);
        let dll_hstring = HSTRING::from(format!("{}.dll", path));

        let module = match unsafe { LoadLibraryW(&dll_hstring) } {
            Ok(m) => m,
            Err(_) => continue,
        };

        let proc = unsafe { GetProcAddress(module, PCSTR::from_raw(b"DllGetActivationFactory\0".as_ptr())) };
        let Some(proc) = proc else {
            unsafe { let _ = FreeLibrary(module); }
            continue;
        };

        let dll_get_factory: DllGetActivationFactoryFn = unsafe { std::mem::transmute(proc) };
        let mut factory_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

        // HSTRING is a transparent wrapper around a pointer; pass it as the raw handle value.
        let class_id: *mut std::ffi::c_void = unsafe { std::mem::transmute_copy(class_name) };
        let hr = unsafe { dll_get_factory(class_id, &mut factory_ptr) };

        if hr.is_ok() && !factory_ptr.is_null() {
            // Success — keep the DLL loaded (don't FreeLibrary) so the factory
            // and any objects it creates remain valid for the process lifetime.
            let factory_unk = unsafe { IUnknown::from_raw(factory_ptr) };
            return Some(WinRTValue::Object(factory_unk));
        }

        // Factory not found in this DLL — unload and try next
        unsafe { let _ = FreeLibrary(module); }
    }

    None
}

#[allow(dead_code)]
pub fn query_interface(obj: WinRTValue, iid: &windows_core::GUID) -> windows_core::Result<WinRTValue> {
    let mut result = std::ptr::null_mut();
    let unk = obj.as_object().unwrap();
    unsafe {
        (unk.query(iid, &mut result)).ok()?;
    }
    Ok(WinRTValue::Object(unsafe { IUnknown::from_raw(result)}))
}

#[cfg(test)]
mod tests {
    use windows::{
        Foundation::{IUriRuntimeClassFactory, Uri},
        Win32::System::WinRT::{
            IActivationFactory, RO_INIT_MULTITHREADED, RoGetActivationFactory, RoInitialize,
        },
    };
    use windows_core::{IInspectable, Interface, h};

    use crate::value::WinRTValue;

    #[test]
    fn call_get_activation_factory() -> windows::core::Result<()> {
        // Ignore error if already initialized
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let _esu = Uri::EscapeComponent(h!("1 + 1"))?;
        let factory =
            unsafe { RoGetActivationFactory::<IActivationFactory>(h!("Windows.Foundation.Uri")) }?;
        let uri_factory = factory.cast::<IUriRuntimeClassFactory>()?;

        let reg = crate::metadata_table::MetadataTable::new();
        let mut uri_factory_iface = crate::signature::InterfaceSignature::define_from_iinspectable("", Default::default(), &reg);
        uri_factory_iface.add_method(
            crate::signature::MethodSignature::new(&reg)
                .add_in(reg.hstring())
                .add_out(reg.object()),
        );
        let result = uri_factory_iface.methods[6].call_dynamic(
            uri_factory.as_raw(),
            &[WinRTValue::HString(
                h!("https://www.example.com/anotherpath?query=2#fragment2").clone(),
            )],
        )?;
        let rv1 = &result[0];
        let uri: Uri = rv1.as_object().unwrap().cast()?;
        println!("Result from dynamic call: {:?}", uri);
        println!("Uri: {}", uri.Path()?);

        let inspect: IInspectable = factory.cast()?;
        let _activate_factory: IActivationFactory = inspect.cast()?;
        Ok(())
    }
}
