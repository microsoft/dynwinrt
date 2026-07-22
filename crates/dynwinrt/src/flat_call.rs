// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use core::ffi::c_void;
use std::ffi::CString;

use libffi::middle::{Arg, Cif, CodePtr, Type};
use windows::Win32::Foundation::{FreeLibrary, GetLastError, HMODULE, SetLastError};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{HRESULT, HSTRING, PCSTR};

use crate::{
    result::{Error, Result},
    value::WinRTValue,
};

struct LoadedLibrary {
    module: HMODULE,
    name: String,
}

impl LoadedLibrary {
    fn load(dll: &str) -> Result<Self> {
        if dll.encode_utf16().any(|unit| unit == 0) {
            return Err(invalid_arg_error());
        }

        unsafe { LoadLibraryW(&HSTRING::from(dll)) }
            .map(|module| Self {
                module,
                name: dll.to_string(),
            })
            .map_err(Error::WindowsError)
    }

    fn proc_address(&self, entry: &str) -> Result<*mut c_void> {
        let proc_name = CString::new(entry).map_err(|_| invalid_arg_error())?;
        let proc =
            unsafe { GetProcAddress(self.module, PCSTR::from_raw(proc_name.as_ptr().cast())) };
        match proc {
            Some(proc) => Ok(unsafe { std::mem::transmute(proc) }),
            None => Err(proc_not_found_error(&self.name, entry)),
        }
    }
}

impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        unsafe {
            let last_error = GetLastError();
            let _ = FreeLibrary(self.module);
            SetLastError(last_error);
        }
    }
}

/// Owns a NUL-terminated UTF-16 string for passing as a stable `LPCWSTR` argument.
pub struct WideStringArg {
    buffer: Vec<u16>,
}

impl WideStringArg {
    /// Returns a raw `LPCWSTR` pointer wrapped as a `WinRTValue`.
    ///
    /// The returned pointer is valid only while this `WideStringArg` is alive;
    /// do not store or use the value after the owner is dropped.
    pub fn as_winrt_value(&self) -> WinRTValue {
        WinRTValue::RawPtr(self.buffer.as_ptr() as *mut c_void)
    }
}

pub fn wide_string_arg(value: &str) -> Result<WideStringArg> {
    if value.encode_utf16().any(|unit| unit == 0) {
        return Err(invalid_arg_error());
    }

    let mut buffer: Vec<u16> = value.encode_utf16().collect();
    buffer.push(0);
    Ok(WideStringArg { buffer })
}

pub fn get_last_error() -> u32 {
    unsafe { GetLastError().0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatReturnKind {
    I32,
    U32,
    Ptr,
}

/// Invokes a flat Win32 export through libffi.
///
/// # Safety
///
/// The caller must ensure that `dll`/`entry`, `ret`, and `args` exactly match
/// the target export's ABI signature, and that all pointer arguments remain
/// valid for the duration of the call. The DLL is unloaded before this function
/// returns, so `FlatReturnKind::Ptr` may only be used for pointers or handles
/// whose validity does not depend on that loaded module remaining resident.
///
/// `LoadLibraryW` uses the default DLL search order, so pass a trusted or
/// fully qualified DLL path to avoid DLL preloading/hijacking risks.
pub unsafe fn flat_invoke(
    dll: &str,
    entry: &str,
    ret: FlatReturnKind,
    args: &[WinRTValue],
) -> Result<WinRTValue> {
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = (dll, entry, ret, args);
        return Err(unsupported_platform_error());
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        let library = LoadedLibrary::load(dll)?;
        let proc = library.proc_address(entry)?;
        let arg_types = args
            .iter()
            .map(flat_arg_type)
            .collect::<Result<Vec<Type>>>()?;
        let ffi_args = args.iter().map(flat_arg).collect::<Result<Vec<Arg>>>()?;
        let ret_type = flat_return_type(ret)?;
        let cif = Cif::new(arg_types, ret_type);

        // On x64 Windows there is a single native calling convention, so libffi's
        // default ABI is correct for Winapi/stdcall and cdecl flat exports.
        unsafe { call_and_convert(&cif, proc, &ffi_args, ret) }
    }
}

fn flat_arg_type(value: &WinRTValue) -> Result<Type> {
    match value {
        WinRTValue::RawPtr(_) => Ok(Type::pointer()),
        WinRTValue::I32(_) => Ok(Type::i32()),
        WinRTValue::U32(_) => Ok(Type::u32()),
        WinRTValue::I64(_) => Ok(Type::i64()),
        WinRTValue::U64(_) => Ok(Type::u64()),
        _ => Err(invalid_arg_error()),
    }
}

fn flat_arg(value: &WinRTValue) -> Result<Arg<'_>> {
    match value {
        WinRTValue::I32(_)
        | WinRTValue::U32(_)
        | WinRTValue::I64(_)
        | WinRTValue::U64(_)
        | WinRTValue::RawPtr(_) => Ok(value.libffi_arg()),
        _ => Err(invalid_arg_error()),
    }
}

fn flat_return_type(kind: FlatReturnKind) -> Result<Type> {
    match kind {
        FlatReturnKind::I32 => Ok(Type::i32()),
        FlatReturnKind::U32 => Ok(Type::u32()),
        FlatReturnKind::Ptr => Ok(Type::pointer()),
    }
}

unsafe fn call_and_convert(
    cif: &Cif,
    proc: *mut c_void,
    args: &[Arg<'_>],
    ret: FlatReturnKind,
) -> Result<WinRTValue> {
    match ret {
        FlatReturnKind::I32 => Ok(WinRTValue::I32(unsafe { cif.call(CodePtr(proc), args) })),
        FlatReturnKind::U32 => Ok(WinRTValue::U32(unsafe { cif.call(CodePtr(proc), args) })),
        FlatReturnKind::Ptr => Ok(WinRTValue::RawPtr(unsafe {
            cif.call::<*mut c_void>(CodePtr(proc), args)
        })),
    }
}

fn invalid_arg_error() -> Error {
    Error::WindowsError(windows_core::Error::from_hresult(HRESULT(
        0x80070057u32 as i32,
    )))
}

fn proc_not_found_error(dll: &str, entry: &str) -> Error {
    Error::WindowsError(windows_core::Error::new(
        HRESULT(0x8007007Fu32 as i32),
        format!("Export '{entry}' not found in '{dll}'"),
    ))
}

#[cfg(not(all(windows, target_pointer_width = "64")))]
fn unsupported_platform_error() -> Error {
    Error::WindowsError(windows_core::Error::from_hresult(HRESULT(
        0x80004001u32 as i32,
    )))
}

#[cfg(all(test, windows, target_pointer_width = "64"))]
mod tests {
    use super::*;
    use windows::Win32::Foundation::WIN32_ERROR;

    fn invoke(
        dll: &str,
        entry: &str,
        ret: FlatReturnKind,
        args: &[WinRTValue],
    ) -> Result<WinRTValue> {
        unsafe { flat_invoke(dll, entry, ret, args) }
    }

    #[test]
    fn flat_call_mul_div_multiplies_divides_and_rounds() -> Result<()> {
        let result = invoke(
            "kernel32.dll",
            "MulDiv",
            FlatReturnKind::I32,
            &[WinRTValue::I32(100), WinRTValue::I32(3), WinRTValue::I32(2)],
        )?;
        assert_eq!(result.as_i32(), Some(150));

        let rounded = invoke(
            "kernel32.dll",
            "MulDiv",
            FlatReturnKind::I32,
            &[WinRTValue::I32(7), WinRTValue::I32(1), WinRTValue::I32(2)],
        )?;
        assert_eq!(rounded.as_i32(), Some(4));
        Ok(())
    }

    #[test]
    fn flat_call_get_current_process_id_matches_rust_process_id() -> Result<()> {
        let result = invoke(
            "kernel32.dll",
            "GetCurrentProcessId",
            FlatReturnKind::U32,
            &[],
        )?;
        let WinRTValue::U32(pid) = result else {
            panic!("expected U32 process id");
        };
        assert_eq!(pid, std::process::id());
        Ok(())
    }

    #[test]
    fn flat_call_lstrlenw_accepts_wide_string_pointer() -> Result<()> {
        let hello = wide_string_arg("hello")?;
        let result = invoke(
            "kernel32.dll",
            "lstrlenW",
            FlatReturnKind::I32,
            &[hello.as_winrt_value()],
        )?;
        assert_eq!(result.as_i32(), Some(5));

        let empty = wide_string_arg("")?;
        let result = invoke(
            "kernel32.dll",
            "lstrlenW",
            FlatReturnKind::I32,
            &[empty.as_winrt_value()],
        )?;
        assert_eq!(result.as_i32(), Some(0));
        Ok(())
    }

    #[test]
    fn flat_call_nonexistent_dll_returns_error() {
        let result = invoke("no_such_dll_xyz.dll", "MulDiv", FlatReturnKind::I32, &[]);
        let Err(Error::WindowsError(err)) = result else {
            panic!("expected WindowsError for missing DLL");
        };
        assert_eq!(err.code(), HRESULT(0x8007007Eu32 as i32));
    }

    #[test]
    fn flat_call_rejects_interior_nul_dll_name() {
        let result = invoke(
            "kernel32.dll\0ignored.dll",
            "MulDiv",
            FlatReturnKind::I32,
            &[],
        );
        let Err(Error::WindowsError(err)) = result else {
            panic!("expected WindowsError for interior-NUL DLL name");
        };
        assert_eq!(err.code(), HRESULT(0x80070057u32 as i32));
    }

    #[test]
    fn flat_call_nonexistent_export_returns_error() {
        let result = invoke(
            "kernel32.dll",
            "ThisExportDoesNotExist",
            FlatReturnKind::I32,
            &[],
        );
        let Err(Error::WindowsError(err)) = result else {
            panic!("expected WindowsError for missing export");
        };
        assert_eq!(err.code(), HRESULT(0x8007007Fu32 as i32));
    }

    #[test]
    fn wide_string_arg_rejects_interior_nul() {
        assert!(wide_string_arg("prefix\0suffix").is_err());
    }

    #[test]
    fn flat_call_get_module_handlew_uses_get_last_error_model() -> Result<()> {
        let bogus_module = wide_string_arg("no_such_module_xyz.dll")?;
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let result = invoke(
            "kernel32.dll",
            "GetModuleHandleW",
            FlatReturnKind::Ptr,
            &[bogus_module.as_winrt_value()],
        )?;
        let WinRTValue::RawPtr(module) = result else {
            panic!("expected raw pointer return");
        };
        assert!(module.is_null());
        assert_eq!(get_last_error(), 126);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Registry marshalling primitives.
    //
    // These exercise the three flat-Win32 argument shapes needed by real
    // Win32 APIs, using the advapi32 registry ABI:
    //
    //   1. Out handle via pointer-to-pointer
    //      (RegOpenKeyExW's `PHKEY phkResult` last arg)
    //   2. Caller-allocated in/out byte buffer + in/out DWORD size
    //      (RegQueryValueExW's `LPBYTE lpData` + `LPDWORD lpcbData`)
    //   3. Wide-string out buffer -> Rust String (UTF-16LE decode)
    //
    // Each buffer is a plain Vec/u32 slot owned by the test; we pass its
    // address as a WinRTValue::RawPtr. That is exactly the same shape the
    // napi layer uses when the JS caller passes a Node `Buffer` through
    // `.pointer(buf)`. If these tests pass, the marshalling that the JS
    // Registry wrapper depends on is proven at the Rust layer.
    // ------------------------------------------------------------------

    // HKEY_LOCAL_MACHINE — predefined pointer-sized HKEY constant.
    // (The Win32 header defines this as (HKEY)(LONG_PTR)(LONG)0x80000002.)
    const HKEY_LOCAL_MACHINE: usize = 0x80000002;

    // KEY_READ = STANDARD_RIGHTS_READ | KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS
    //          | KEY_NOTIFY
    const KEY_READ: u32 = 0x20019;

    // Win32 registry error codes (LSTATUS = LONG).
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_MORE_DATA: i32 = 234;

    // REG_SZ registry value type.
    const REG_SZ: u32 = 1;

    /// RegOpenKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD ulOptions,
    ///               REGSAM samDesired, PHKEY phkResult) -> LSTATUS
    fn reg_open_key(parent: usize, sub_key: &str) -> Result<(i32, usize)> {
        let sub_key_arg = wide_string_arg(sub_key)?;
        // Caller-allocated slot for the out HKEY. Pass its address as a raw
        // pointer. The callee writes an HKEY (pointer-sized) into it.
        let mut hkey_out: usize = 0;
        let phkey = WinRTValue::RawPtr(&mut hkey_out as *mut usize as *mut c_void);
        let status = invoke(
            "advapi32.dll",
            "RegOpenKeyExW",
            FlatReturnKind::I32,
            &[
                WinRTValue::RawPtr(parent as *mut c_void),
                sub_key_arg.as_winrt_value(),
                WinRTValue::U32(0), // ulOptions
                WinRTValue::U32(KEY_READ),
                phkey,
            ],
        )?;
        let code = status.as_i32().expect("LSTATUS is a signed LONG");
        Ok((code, hkey_out))
    }

    /// RegCloseKey(HKEY) -> LSTATUS
    fn reg_close_key(hkey: usize) -> Result<i32> {
        let status = invoke(
            "advapi32.dll",
            "RegCloseKey",
            FlatReturnKind::I32,
            &[WinRTValue::RawPtr(hkey as *mut c_void)],
        )?;
        Ok(status.as_i32().unwrap())
    }

    /// RegQueryValueExW(HKEY, LPCWSTR lpValueName, LPDWORD lpReserved,
    ///                  LPDWORD lpType, LPBYTE lpData, LPDWORD lpcbData) -> LSTATUS
    ///
    /// Returns `(status, type, bytes_written, buffer)` where `buffer` is the
    /// caller-allocated data buffer (unchanged on error but with valid length
    /// on ERROR_MORE_DATA).
    fn reg_query_value(
        hkey: usize,
        value_name: &str,
        mut buffer: Vec<u8>,
    ) -> Result<(i32, u32, u32, Vec<u8>)> {
        let name_arg = wide_string_arg(value_name)?;
        let mut reg_type: u32 = 0;
        let mut cb_data: u32 = buffer.len() as u32; // in: capacity; out: bytes written
        let data_ptr = if buffer.is_empty() {
            std::ptr::null_mut()
        } else {
            buffer.as_mut_ptr() as *mut c_void
        };
        let status = invoke(
            "advapi32.dll",
            "RegQueryValueExW",
            FlatReturnKind::I32,
            &[
                WinRTValue::RawPtr(hkey as *mut c_void),
                name_arg.as_winrt_value(),
                WinRTValue::RawPtr(std::ptr::null_mut()), // lpReserved
                WinRTValue::RawPtr(&mut reg_type as *mut u32 as *mut c_void),
                WinRTValue::RawPtr(data_ptr),
                WinRTValue::RawPtr(&mut cb_data as *mut u32 as *mut c_void),
            ],
        )?;
        Ok((status.as_i32().unwrap(), reg_type, cb_data, buffer))
    }

    /// Decode a REG_SZ payload (UTF-16LE bytes, possibly NUL-terminated) into
    /// a Rust String. `cb_bytes` is the count reported by RegQueryValueExW.
    fn decode_reg_sz(buffer: &[u8], cb_bytes: u32) -> String {
        let byte_len = cb_bytes as usize;
        assert!(byte_len <= buffer.len(), "cb_bytes exceeds buffer");
        // REG_SZ values are wide-char aligned. Truncate a trailing NUL if any.
        let mut u16s: Vec<u16> = buffer[..byte_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if u16s.last() == Some(&0) {
            u16s.pop();
        }
        String::from_utf16_lossy(&u16s)
    }

    /// Normal path: open HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion,
    /// read the REG_SZ "ProductName" value, and verify it looks like Windows.
    ///
    /// Proves: (a) HKEY out via pointer-to-pointer, (b) caller-allocated
    /// LPBYTE lpData + in/out LPDWORD lpcbData, (c) UTF-16LE decode.
    #[test]
    fn flat_call_reads_registry_product_name() -> Result<()> {
        let (status, hkey) = reg_open_key(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )?;
        assert_eq!(status, ERROR_SUCCESS, "RegOpenKeyExW failed: {status}");
        assert_ne!(hkey, 0, "RegOpenKeyExW returned a null HKEY");

        let buffer = vec![0u8; 512];
        let (status, reg_type, cb, buffer) = reg_query_value(hkey, "ProductName", buffer)?;
        // Always close the key, even if the query failed.
        let close_status = reg_close_key(hkey)?;
        assert_eq!(close_status, ERROR_SUCCESS);

        assert_eq!(status, ERROR_SUCCESS, "RegQueryValueExW failed: {status}");
        assert_eq!(reg_type, REG_SZ, "ProductName should be REG_SZ");
        assert!(cb > 0, "cb_data should reflect bytes written");

        let product_name = decode_reg_sz(&buffer, cb);
        assert!(!product_name.is_empty(), "ProductName should not be empty");
        assert!(
            product_name.to_lowercase().contains("windows"),
            "ProductName should mention Windows, got {product_name:?}"
        );
        Ok(())
    }

    /// Corner case: opening a non-existent subkey returns ERROR_FILE_NOT_FOUND
    /// and the out HKEY slot stays null.
    #[test]
    fn flat_call_reg_open_key_missing_returns_file_not_found() -> Result<()> {
        let (status, hkey) = reg_open_key(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\DynWinrt\NoSuchKey\Nope",
        )?;
        assert_eq!(status, ERROR_FILE_NOT_FOUND, "expected ERROR_FILE_NOT_FOUND");
        assert_eq!(hkey, 0, "out HKEY should stay null on failure");
        Ok(())
    }

    /// Corner case: querying a value that doesn't exist returns
    /// ERROR_FILE_NOT_FOUND (the same LSTATUS the flat wrapper must surface).
    #[test]
    fn flat_call_reg_query_missing_value_returns_file_not_found() -> Result<()> {
        let (status, hkey) = reg_open_key(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )?;
        assert_eq!(status, ERROR_SUCCESS);

        let (query_status, _reg_type, _cb, _buf) =
            reg_query_value(hkey, "ThisValueShouldNeverExist_DynWinrt", vec![0u8; 32])?;
        let _ = reg_close_key(hkey)?;
        assert_eq!(query_status, ERROR_FILE_NOT_FOUND);
        Ok(())
    }

    /// Corner case: buffer-too-small returns ERROR_MORE_DATA and the in/out
    /// `lpcbData` slot is rewritten with the required byte count. This
    /// specifically proves the in/out DWORD marshalling: we pass 4 in and
    /// read a >4 out from the same slot.
    ///
    /// NOTE: RegQueryValueExW treats `lpData == NULL` as a size-query and
    /// returns SUCCESS, not ERROR_MORE_DATA. We therefore pass a real (too
    /// small) buffer.
    #[test]
    fn flat_call_reg_query_buffer_too_small_reports_required_size() -> Result<()> {
        let (status, hkey) = reg_open_key(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )?;
        assert_eq!(status, ERROR_SUCCESS);

        // 4 bytes is guaranteed to be smaller than any REG_SZ ProductName.
        let (query_status, reg_type, required_bytes, _buf) =
            reg_query_value(hkey, "ProductName", vec![0u8; 4])?;
        let _ = reg_close_key(hkey)?;
        assert_eq!(query_status, ERROR_MORE_DATA);
        assert_eq!(reg_type, REG_SZ);
        assert!(
            required_bytes > 4,
            "lpcbData in/out slot should be rewritten with the required byte count \
             (got {required_bytes})"
        );
        Ok(())
    }

    /// Corner case (size-query idiom): passing `lpData == NULL` with
    /// `cb == 0` is the documented way to query the required size. This
    /// specifically proves the null-pointer marshalling path.
    #[test]
    fn flat_call_reg_query_null_data_returns_size_query() -> Result<()> {
        let (status, hkey) = reg_open_key(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )?;
        assert_eq!(status, ERROR_SUCCESS);

        let (query_status, reg_type, required_bytes, _buf) =
            reg_query_value(hkey, "ProductName", Vec::new())?;
        let _ = reg_close_key(hkey)?;
        // Win32 documents this "null data, 0 cb" path as returning
        // ERROR_SUCCESS with the required size in cb_data.
        assert_eq!(query_status, ERROR_SUCCESS);
        assert_eq!(reg_type, REG_SZ);
        assert!(required_bytes > 0);
        Ok(())
    }
}
