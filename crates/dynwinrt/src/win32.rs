// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Contract-planned flat Win32 calls. No WinRT values or COM registry are used.

use std::{
    ffi::{CString, c_void},
    sync::{Arc, Mutex},
};

use libffi::middle::{Arg, Cif, CodePtr, Type};
use serde::{Deserialize, Serialize};
use windows::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::{
        LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
        Registry::{HKEY, RegCloseKey},
    },
};
use windows_core::{PCSTR, PCWSTR};

pub type Result<T> = std::result::Result<T, String>;
pub const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReturnKind {
    U32,
    U64,
    Status,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Parameter {
    U32 {},
    Utf16 { nullable: bool },
    BorrowHkey { reject_performance_data: bool },
    OwnHkey { borrowed_from: usize },
    ConsumeHkey {},
    ReservedNull {},
    OutU32 {},
    Bytes { count_parameter: usize },
    ByteCount { buffer_parameter: usize },
}

impl Parameter {
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            Self::U32 {}
                | Self::Utf16 { .. }
                | Self::BorrowHkey { .. }
                | Self::ConsumeHkey {}
                | Self::Bytes { .. }
        )
    }

    fn ffi_type(&self) -> Type {
        match self {
            Self::U32 {} => Type::u32(),
            Self::Utf16 { .. }
            | Self::BorrowHkey { .. }
            | Self::OwnHkey { .. }
            | Self::ConsumeHkey {}
            | Self::ReservedNull {}
            | Self::OutU32 {}
            | Self::Bytes { .. }
            | Self::ByteCount { .. } => Type::pointer(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CallingConvention {
    System,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Specification {
    pub version: u32,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: CallingConvention,
    pub architectures: u32,
    pub returns: ReturnKind,
    pub parameters: Vec<Parameter>,
}

impl Specification {
    fn validate(&self) -> Result<()> {
        let architecture = if cfg!(target_arch = "x86") {
            1
        } else if cfg!(target_arch = "x86_64") {
            2
        } else if cfg!(target_arch = "aarch64") {
            4
        } else {
            0
        };
        if self.version != 1
            || architecture == 0
            || self.architectures & architecture == 0
            || self.architectures & !7 != 0
        {
            return Err("win32.plan: unsupported version or architecture".into());
        }
        if !self.dll.to_ascii_lowercase().ends_with(".dll")
            || self.dll.contains("..")
            || !self
                .dll
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            || self.entry_point.is_empty()
            || !self
                .entry_point
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || self.parameters.len() > 32
        {
            return Err("win32.plan: require a System32 DLL basename and an export name".into());
        }
        if (self.entry_point == "RegCloseKey"
            || self.parameters.contains(&Parameter::ConsumeHkey {}))
            && (self.parameters != [Parameter::ConsumeHkey {}]
                || self.returns != ReturnKind::Status
                || !self.dll.eq_ignore_ascii_case("advapi32.dll")
                || self.entry_point != "RegCloseKey")
        {
            return Err("win32.plan: HKEY consumption requires exact RegCloseKey cleanup".into());
        }
        for (index, parameter) in self.parameters.iter().enumerate() {
            let valid = match parameter {
                Parameter::OwnHkey { borrowed_from } => {
                    matches!(
                        self.parameters.get(*borrowed_from),
                        Some(Parameter::BorrowHkey { .. })
                    ) && self.returns == ReturnKind::Status
                }
                Parameter::Bytes { count_parameter } => {
                    matches!(self.parameters.get(*count_parameter), Some(Parameter::ByteCount { buffer_parameter }) if *buffer_parameter == index)
                        && self.returns == ReturnKind::Status
                        && self.parameters.iter().any(|p| {
                            matches!(
                                p,
                                Parameter::BorrowHkey {
                                    reject_performance_data: true
                                }
                            )
                        })
                }
                Parameter::ByteCount { buffer_parameter } => {
                    matches!(self.parameters.get(*buffer_parameter), Some(Parameter::Bytes { count_parameter }) if *count_parameter == index)
                }
                Parameter::OutU32 {} => self.returns == ReturnKind::Status,
                Parameter::U32 {}
                | Parameter::Utf16 { .. }
                | Parameter::BorrowHkey { .. }
                | Parameter::ConsumeHkey {}
                | Parameter::ReservedNull {} => true,
            };
            if !valid {
                return Err(format!(
                    "win32.plan: incomplete parameter contract at {index}"
                ));
            }
        }
        Ok(())
    }
}

/// Explicit borrowed HKEY bits, never an address inferred from byte storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle(usize);

impl Handle {
    pub fn hkey(bits: usize) -> Result<Self> {
        let bits = if matches!(bits, 0x80000000..=0x80000006 | 0x80000050 | 0x80000060) {
            bits as u32 as i32 as isize as usize
        } else {
            bits
        };
        if bits == 0 || bits == usize::MAX {
            return Err("win32.handle: invalid HKEY value".into());
        }
        Ok(Self(bits))
    }

    fn predefined(self) -> bool {
        matches!(
            self.0 as isize as i64,
            -2147483648..=-2147483642 | -2147483568 | -2147483552
        )
    }

    fn performance_data(self) -> bool {
        self.0 == 0x80000004_u32 as i32 as isize as usize
    }
}

#[derive(Debug)]
struct ResourceState {
    bits: Option<usize>,
    leases: usize,
}

#[derive(Debug)]
struct ResourceInner(Mutex<ResourceState>);

#[cfg(test)]
thread_local! {
    static CLEANUP_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn cleanup_hkey(bits: usize) -> u32 {
    // This is the only compiled API-specific call: the exact resource
    // finalizer. Normal export invocation always goes through libffi.
    #[cfg(test)]
    CLEANUP_COUNT.with(|count| count.set(count.get() + 1));
    unsafe { RegCloseKey(HKEY(bits as *mut c_void)).0 }
}

impl Drop for ResourceInner {
    fn drop(&mut self) {
        let state = self.0.get_mut().unwrap_or_else(|error| error.into_inner());
        if let Some(bits) = state.bits.take() {
            let status = cleanup_hkey(bits);
            if status != 0 {
                eprintln!("[dynwinrt] Win32 HKEY finalization failed with status {status}");
            }
        }
    }
}

/// Shared ownership state. Numeric aliases cannot create another owner.
#[derive(Clone, Debug)]
pub struct Resource(Arc<ResourceInner>);

impl Resource {
    fn owned(bits: usize) -> Self {
        Self(Arc::new(ResourceInner(Mutex::new(ResourceState {
            bits: Some(bits),
            leases: 0,
        }))))
    }

    pub fn closed(&self) -> bool {
        self.0
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .bits
            .is_none()
    }

    pub fn close(&self) -> Result<u32> {
        self.close_using(cleanup_hkey)
    }

    fn close_using(&self, close: impl FnOnce(usize) -> u32) -> Result<u32> {
        let mut state = self
            .0
            .0
            .lock()
            .map_err(|_| "win32.resource: poisoned state")?;
        if state.leases != 0 {
            return Err("win32.resource: resource is in use".into());
        }
        let Some(bits) = state.bits else { return Ok(0) };
        let status = close(bits);
        if status == 0 {
            state.bits = None;
        }
        Ok(status)
    }

    fn lease(&self) -> Result<Lease> {
        let mut state = self
            .0
            .0
            .lock()
            .map_err(|_| "win32.resource: poisoned state")?;
        let bits = state.bits.ok_or("win32.resource: resource is closed")?;
        state.leases = state
            .leases
            .checked_add(1)
            .ok_or("win32.resource: lease overflow")?;
        Ok(Lease {
            resource: self.clone(),
            bits,
        })
    }
}

struct Lease {
    resource: Resource,
    bits: usize,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.resource
            .0
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .leases -= 1;
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    U32(u32),
    Utf16(Option<String>),
    Hkey(Handle),
    Resource(Resource),
    Bytes(Option<Vec<u8>>),
}

#[derive(Debug)]
pub enum Output {
    U32(u32),
    Hkey(Handle),
    Resource(Resource),
    Bytes(Vec<u8>),
    Null,
}

#[derive(Debug)]
pub struct CallResult {
    pub value: u64,
    pub outputs: Vec<Output>,
}

struct Module(HMODULE);

impl Drop for Module {
    fn drop(&mut self) {
        if let Err(error) = unsafe { FreeLibrary(self.0) } {
            eprintln!("[dynwinrt] Win32 module finalization failed: {error}");
        }
    }
}

/// Prepared once, then immutable. Each invocation owns all mutable ABI storage.
pub struct CallPlan {
    specification: Specification,
    cif: Cif,
    entry: CodePtr,
    _module: Option<Module>,
}

// The CIF and its owned type graph are completely prepared before publication;
// ffi_call only reads them. The entry is retained by the module, is a system
// export (not a JS callback), and uses only call-local slots. HKEY lifetimes are
// synchronized separately. Neither invoke nor any public API mutates this plan.
unsafe impl Send for CallPlan {}
unsafe impl Sync for CallPlan {}

impl CallPlan {
    /// # Safety
    /// The caller must prove the complete native signature, lifetime, output
    /// validity and resource contracts. Codegen does so using pinned evidence;
    /// manual specifications belong exclusively to an explicitly unsafe API.
    pub unsafe fn bind(specification: Specification) -> Result<Self> {
        specification.validate()?;
        let dll = specification
            .dll
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let module = Module(
            unsafe { LoadLibraryExW(PCWSTR(dll.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|error| format!("win32.load: {error}"))?,
        );
        let name = CString::new(specification.entry_point.as_str()).map_err(|e| e.to_string())?;
        let entry = unsafe { GetProcAddress(module.0, PCSTR(name.as_ptr().cast())) }
            .ok_or_else(|| format!("win32.load: export {} not found", specification.entry_point))?;
        let mut plan = Self::prepare(specification, CodePtr::from_ptr(entry as *const c_void));
        plan._module = Some(module);
        Ok(plan)
    }

    fn prepare(specification: Specification, entry: CodePtr) -> Self {
        let result = match specification.returns {
            ReturnKind::U32 | ReturnKind::Status => Type::u32(),
            ReturnKind::U64 => Type::u64(),
        };
        let cif = crate::native_call::system_cif(
            specification
                .parameters
                .iter()
                .map(Parameter::ffi_type)
                .collect(),
            result,
        );
        Self {
            specification,
            cif,
            entry,
            _module: None,
        }
    }

    pub fn specification(&self) -> &Specification {
        &self.specification
    }

    pub fn invoke(&self, values: &[Value]) -> Result<CallResult> {
        let parameters = &self.specification.parameters;
        if values.len() != parameters.iter().filter(|p| p.is_input()).count() {
            return Err("win32.value: argument count does not match the plan".into());
        }
        if parameters == &[Parameter::ConsumeHkey {}] {
            let Value::Resource(resource) = &values[0] else {
                return Err("win32.value: consuming calls require a managed HKEY resource".into());
            };
            let value = resource.close_using(|bits| {
                let pointer = bits as *mut c_void;
                unsafe { self.cif.call::<u32>(self.entry, &[Arg::new(&pointer)]) }
            })?;
            return Ok(CallResult {
                value: u64::from(value),
                outputs: Vec::new(),
            });
        }

        let mut slots = (0..parameters.len())
            .map(|_| Slot::default())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let mut inputs = vec![None; parameters.len()];
        let mut leases = Vec::new();
        let mut next = 0;
        for (index, parameter) in parameters.iter().enumerate() {
            if !parameter.is_input() {
                continue;
            }
            let value = &values[next];
            inputs[index] = Some(value);
            next += 1;
            let slot = &mut slots[index];
            match (parameter, value) {
                (Parameter::U32 {}, Value::U32(value)) => slot.u32 = *value,
                (Parameter::Utf16 { nullable: true }, Value::Utf16(None)) => {}
                (Parameter::Utf16 { .. }, Value::Utf16(Some(value))) => {
                    if value.contains('\0') || value.len() > MAX_BUFFER_BYTES / 2 {
                        return Err("win32.value: UTF-16 input is too large or contains NUL".into());
                    }
                    slot.string
                        .try_reserve_exact(value.encode_utf16().count() + 1)
                        .map_err(|_| "win32.value: unable to allocate UTF-16 storage")?;
                    slot.string.extend(value.encode_utf16().chain(Some(0)));
                    slot.pointer = slot.string.as_mut_ptr().cast();
                }
                (
                    Parameter::BorrowHkey {
                        reject_performance_data,
                    },
                    value,
                ) => {
                    let bits = match value {
                        Value::Hkey(handle) => handle.0,
                        Value::Resource(resource) => {
                            let lease = resource.lease()?;
                            let bits = lease.bits;
                            leases.push(lease);
                            bits
                        }
                        _ => return Err("win32.value: expected a typed HKEY input".into()),
                    };
                    if *reject_performance_data && Handle(bits).performance_data() {
                        return Err(
                            "win32.value: HKEY_PERFORMANCE_DATA has an unsupported count contract"
                                .into(),
                        );
                    }
                    slot.pointer = bits as *mut c_void;
                }
                (Parameter::Bytes { .. }, Value::Bytes(bytes)) => {
                    if let Some(bytes) = bytes {
                        if bytes.len() > MAX_BUFFER_BYTES {
                            return Err(
                                "win32.value: byte buffer exceeds the bounded allocation limit"
                                    .into(),
                            );
                        }
                        slot.capacity = bytes.len();
                        // Output-only buffers use private zeroed storage. Even
                        // an empty, non-null view needs a distinct non-null pointer.
                        let length = bytes.len().max(1);
                        slot.bytes
                            .try_reserve_exact(length)
                            .map_err(|_| "win32.value: unable to allocate native byte storage")?;
                        slot.bytes.resize(length, 0);
                        slot.pointer = slot.bytes.as_mut_ptr().cast();
                    }
                }
                _ => {
                    return Err(format!(
                        "win32.value: argument {index} does not match its contract"
                    ));
                }
            }
        }
        for (index, parameter) in parameters.iter().enumerate() {
            match parameter {
                Parameter::OwnHkey { .. } => {
                    slots[index].pointer = (&mut slots[index].handle as *mut usize).cast()
                }
                Parameter::OutU32 {} => {
                    slots[index].pointer = (&mut slots[index].u32 as *mut u32).cast()
                }
                Parameter::ByteCount { buffer_parameter } => {
                    slots[index].u32 = u32::try_from(slots[*buffer_parameter].capacity)
                        .map_err(|_| "win32.value: buffer capacity exceeds u32")?;
                    slots[index].pointer = (&mut slots[index].u32 as *mut u32).cast();
                }
                Parameter::U32 {}
                | Parameter::Utf16 { .. }
                | Parameter::BorrowHkey { .. }
                | Parameter::ConsumeHkey {}
                | Parameter::ReservedNull {}
                | Parameter::Bytes { .. } => {}
            }
        }
        let args = parameters
            .iter()
            .zip(slots.iter())
            .map(|(p, slot)| {
                if matches!(p, Parameter::U32 {}) {
                    Arg::new(&slot.u32)
                } else {
                    Arg::new(&slot.pointer)
                }
            })
            .collect::<Vec<_>>();
        let value = unsafe {
            match self.specification.returns {
                ReturnKind::U64 => self.cif.call::<u64>(self.entry, &args),
                ReturnKind::U32 | ReturnKind::Status => {
                    u64::from(self.cif.call::<u32>(self.entry, &args))
                }
            }
        };
        let success = self.specification.returns != ReturnKind::Status || value == 0;
        let mut outputs = Vec::new();
        for (index, parameter) in parameters.iter().enumerate() {
            let output = match parameter {
                Parameter::OwnHkey { borrowed_from } if success => {
                    let bits = slots[index].handle;
                    let handle = Handle::hkey(bits)?;
                    if bits == slots[*borrowed_from].pointer as usize {
                        match inputs[*borrowed_from].unwrap() {
                            Value::Resource(resource) => Output::Resource(resource.clone()),
                            Value::Hkey(handle) => Output::Hkey(*handle),
                            _ => unreachable!("validated borrowed handle input"),
                        }
                    } else if handle.predefined() {
                        Output::Hkey(handle)
                    } else {
                        Output::Resource(Resource::owned(bits))
                    }
                }
                // The pinned contract makes failed output contents unspecified,
                // not an ownership transfer. Never adopt or close those bits.
                Parameter::OwnHkey { .. } => Output::Null,
                Parameter::OutU32 {} if success => Output::U32(slots[index].u32),
                Parameter::OutU32 {} => Output::Null,
                Parameter::ByteCount { .. } if success || value == 234 => {
                    Output::U32(slots[index].u32)
                }
                Parameter::ByteCount { .. } => Output::Null,
                Parameter::Bytes { count_parameter }
                    if success && !slots[index].pointer.is_null() =>
                {
                    let length = slots[*count_parameter].u32 as usize;
                    if length > slots[index].capacity {
                        return Err(
                            "win32.output: successful byte count exceeds the native buffer capacity"
                                .into(),
                        );
                    }
                    slots[index].bytes.truncate(length);
                    Output::Bytes(std::mem::take(&mut slots[index].bytes))
                }
                Parameter::Bytes { .. } => Output::Null,
                Parameter::U32 {}
                | Parameter::Utf16 { .. }
                | Parameter::BorrowHkey { .. }
                | Parameter::ConsumeHkey {}
                | Parameter::ReservedNull {} => continue,
            };
            outputs.push(output);
        }
        Ok(CallResult { value, outputs })
    }
}

#[derive(Default)]
struct Slot {
    u32: u32,
    handle: usize,
    pointer: *mut c_void,
    string: Vec<u16>,
    bytes: Vec<u8>,
    capacity: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(
        dll: &str,
        entry: &str,
        returns: ReturnKind,
        parameters: Vec<Parameter>,
    ) -> Specification {
        Specification {
            version: 1,
            dll: dll.into(),
            entry_point: entry.into(),
            calling_convention: CallingConvention::System,
            architectures: 7,
            returns,
            parameters,
        }
    }

    fn open_plan() -> CallPlan {
        unsafe {
            CallPlan::bind(spec(
                "ADVAPI32.dll",
                "RegOpenKeyExW",
                ReturnKind::Status,
                vec![
                    Parameter::BorrowHkey {
                        reject_performance_data: true,
                    },
                    Parameter::Utf16 { nullable: true },
                    Parameter::U32 {},
                    Parameter::U32 {},
                    Parameter::OwnHkey { borrowed_from: 0 },
                ],
            ))
            .unwrap()
        }
    }

    fn query_plan() -> Specification {
        spec(
            "ADVAPI32.dll",
            "RegQueryValueExW",
            ReturnKind::Status,
            vec![
                Parameter::BorrowHkey {
                    reject_performance_data: true,
                },
                Parameter::Utf16 { nullable: true },
                Parameter::ReservedNull {},
                Parameter::OutU32 {},
                Parameter::Bytes { count_parameter: 5 },
                Parameter::ByteCount {
                    buffer_parameter: 4,
                },
            ],
        )
    }

    #[test]
    fn win32_traits_and_fail_closed_plans() {
        fn traits<T: Send + Sync>() {}
        traits::<CallPlan>();
        traits::<Resource>();
        traits::<Handle>();
        let mut invalid = query_plan();
        invalid.parameters[4] = Parameter::Bytes {
            count_parameter: 99,
        };
        assert!(invalid.validate().is_err());
        invalid = query_plan();
        invalid.dll = "..\\ADVAPI32.dll".into();
        assert!(invalid.validate().is_err());
        invalid = spec(
            "KERNEL32.dll",
            "CloseHandle",
            ReturnKind::Status,
            vec![Parameter::ConsumeHkey {}],
        );
        assert!(invalid.validate().is_err());
        invalid = spec(
            "ADVAPI32.dll",
            "RegCloseKey",
            ReturnKind::Status,
            vec![Parameter::BorrowHkey {
                reject_performance_data: true,
            }],
        );
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn win32_live_scalars_registry_and_cleanup() {
        for (entry, returns) in [
            ("GetTickCount", ReturnKind::U32),
            ("GetTickCount64", ReturnKind::U64),
        ] {
            let plan =
                unsafe { CallPlan::bind(spec("KERNEL32.dll", entry, returns, vec![])).unwrap() };
            assert!(plan.invoke(&[]).unwrap().value > 0);
            assert!(plan.invoke(&[Value::U32(0)]).is_err());
        }
        let open = open_plan();
        let root = Value::Hkey(Handle::hkey(0x80000002).unwrap());
        let result = open
            .invoke(&[
                root.clone(),
                Value::Utf16(Some(
                    "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion".into(),
                )),
                Value::U32(0),
                Value::U32(1),
            ])
            .unwrap();
        assert_eq!(result.value, 0);
        let Output::Resource(resource) = &result.outputs[0] else {
            panic!("expected owned HKEY")
        };
        let query = unsafe { CallPlan::bind(query_plan()).unwrap() };
        let input = Value::Resource(resource.clone());
        let probe = query
            .invoke(&[
                input.clone(),
                Value::Utf16(Some("ProductName".into())),
                Value::Bytes(None),
            ])
            .unwrap();
        assert_eq!(probe.value, 0);
        let Output::U32(length) = probe.outputs[2] else {
            panic!("missing byte count")
        };
        assert!(length > 1);
        let small = query
            .invoke(&[
                input.clone(),
                Value::Utf16(Some("ProductName".into())),
                Value::Bytes(Some(vec![0; 1])),
            ])
            .unwrap();
        assert_eq!(small.value, 234);
        assert!(matches!(small.outputs[1], Output::Null));
        assert!(matches!(small.outputs[2], Output::U32(n) if n > 1));
        let read = query
            .invoke(&[
                input.clone(),
                Value::Utf16(Some("ProductName".into())),
                Value::Bytes(Some(vec![0; length as usize])),
            ])
            .unwrap();
        assert_eq!(read.value, 0);
        assert!(matches!(&read.outputs[1], Output::Bytes(bytes) if bytes.len() == length as usize));
        let missing = query
            .invoke(&[
                input.clone(),
                Value::Utf16(Some("dynwinrt-missing-contract-test-value".into())),
                Value::Bytes(None),
            ])
            .unwrap();
        assert_ne!(missing.value, 0);
        assert!(
            missing
                .outputs
                .iter()
                .all(|value| matches!(value, Output::Null))
        );
        let close = unsafe {
            CallPlan::bind(spec(
                "ADVAPI32.dll",
                "RegCloseKey",
                ReturnKind::Status,
                vec![Parameter::ConsumeHkey {}],
            ))
            .unwrap()
        };
        assert!(close.invoke(&[root.clone()]).is_err());
        assert_eq!(close.invoke(&[input.clone()]).unwrap().value, 0);
        assert!(resource.closed());
        assert_eq!(close.invoke(&[input.clone()]).unwrap().value, 0);
        assert_eq!(resource.close().unwrap(), 0);
        assert!(
            query
                .invoke(&[input, Value::Utf16(None), Value::Bytes(None)])
                .is_err()
        );
        let refreshed = open
            .invoke(&[root, Value::Utf16(None), Value::U32(0), Value::U32(1)])
            .unwrap();
        assert_eq!(refreshed.value, 0);
        assert!(matches!(refreshed.outputs[0], Output::Hkey(_)));
    }

    #[test]
    fn win32_resource_lease_and_close_share_one_lock() {
        let resource = Resource::owned(123);
        let lease = resource.lease().unwrap();
        let other = resource.clone();
        std::thread::spawn(move || {
            assert!(
                other
                    .close_using(|_| panic!("leased handle must not be closed"))
                    .is_err()
            );
        })
        .join()
        .unwrap();
        drop(lease);
        assert_eq!(resource.close_using(|_| 5).unwrap(), 5);
        assert!(!resource.closed());
        assert_eq!(
            resource
                .close_using(|bits| {
                    assert_eq!(bits, 123);
                    0
                })
                .unwrap(),
            0
        );
        assert!(resource.closed());
        assert!(resource.lease().is_err());
        assert_eq!(
            resource
                .close_using(|_| panic!("must close only once"))
                .unwrap(),
            0
        );
    }

    #[test]
    fn win32_values_fail_before_ffi() {
        let plan = open_plan();
        let arguments = [
            Value::Bytes(Some(vec![0; 8])),
            Value::Utf16(None),
            Value::U32(0),
            Value::U32(1),
        ];
        assert!(plan.invoke(&arguments).unwrap_err().contains("typed HKEY"));
        let plan = unsafe { CallPlan::bind(query_plan()).unwrap() };
        assert!(
            plan.invoke(&[
                Value::Hkey(Handle::hkey(0x80000004).unwrap()),
                Value::Utf16(None),
                Value::Bytes(None)
            ])
            .is_err()
        );
        assert!(
            plan.invoke(&[
                Value::Hkey(Handle::hkey(0x80000001).unwrap()),
                Value::Utf16(Some("bad\0suffix".into())),
                Value::Bytes(None)
            ])
            .is_err()
        );
    }

    unsafe extern "system" fn oversized_output(
        _: *mut c_void,
        _: *const u16,
        reserved: *mut c_void,
        typ: *mut u32,
        data: *mut u8,
        count: *mut u32,
    ) -> u32 {
        assert!(reserved.is_null());
        unsafe {
            assert_eq!(*count, 2);
            *typ = 1;
            *data = 42;
            *count = u32::MAX;
        }
        0
    }

    #[test]
    fn win32_successful_output_rejects_a_count_exceeding_capacity() {
        let plan = CallPlan::prepare(
            query_plan(),
            CodePtr::from_ptr(oversized_output as *const c_void),
        );
        let error = plan
            .invoke(&[
                Value::Hkey(Handle::hkey(0x80000001).unwrap()),
                Value::Utf16(None),
                Value::Bytes(Some(vec![0; 2])),
            ])
            .unwrap_err();
        assert!(error.contains("successful byte count exceeds"));
    }

    unsafe extern "system" fn echo_handle(
        input: *mut c_void,
        _: *const u16,
        _: u32,
        status: u32,
        output: *mut usize,
    ) -> u32 {
        unsafe { *output = input as usize };
        status
    }

    #[test]
    fn win32_alias_and_failure_outputs_never_create_another_owner() {
        let open = open_plan();
        let result = open
            .invoke(&[
                Value::Hkey(Handle::hkey(0x80000001).unwrap()),
                Value::Utf16(Some("Software".into())),
                Value::U32(0),
                Value::U32(1),
            ])
            .unwrap();
        assert_eq!(result.value, 0);
        let Output::Resource(resource) = &result.outputs[0] else {
            panic!("expected owned key")
        };
        let echo = CallPlan::prepare(
            open.specification().clone(),
            CodePtr::from_ptr(echo_handle as *const c_void),
        );
        let alias = echo
            .invoke(&[
                Value::Resource(resource.clone()),
                Value::Utf16(None),
                Value::U32(0),
                Value::U32(0),
            ])
            .unwrap();
        let Output::Resource(other) = &alias.outputs[0] else {
            panic!("expected managed alias")
        };
        assert!(Arc::ptr_eq(&resource.0, &other.0));
        let failed = echo
            .invoke(&[
                Value::Resource(resource.clone()),
                Value::Utf16(None),
                Value::U32(0),
                Value::U32(5),
            ])
            .unwrap();
        assert!(matches!(failed.outputs[0], Output::Null));
        assert!(!resource.closed());
        let before = CLEANUP_COUNT.with(|count| count.get());
        drop(alias);
        assert!(!resource.closed());
        drop(result);
        assert_eq!(CLEANUP_COUNT.with(|count| count.get()), before + 1);
    }
}
