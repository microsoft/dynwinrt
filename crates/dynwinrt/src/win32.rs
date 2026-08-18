// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Immutable call plans for flat Win32 exports.
//!
//! This module deliberately has its own ABI vocabulary. WinRT and Classic COM
//! types do not participate in flat export planning.

use core::ffi::c_void;
#[cfg(not(all(windows, target_pointer_width = "32")))]
use std::collections::HashMap;
#[cfg(not(all(windows, target_pointer_width = "32")))]
use std::ffi::CString;
#[cfg(not(all(windows, target_pointer_width = "32")))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use libffi::middle::Type as FfiType;
use libffi::middle::{Arg, Cif, Ret, arg};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, GetLastError, HANDLE, HLOCAL, HMODULE, LocalFree,
};
use windows::Win32::Security::Credentials::CredFree;
#[cfg(not(all(windows, target_pointer_width = "32")))]
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::Registry::{HKEY, RegCloseKey};
use windows::Win32::System::Services::{CloseServiceHandle, SC_HANDLE};
use windows_core::HRESULT;
#[cfg(not(all(windows, target_pointer_width = "32")))]
use windows_core::{HSTRING, PCSTR};

use crate::abi::{AbiType, AbiValue};
#[cfg(not(all(windows, target_pointer_width = "32")))]
use crate::native_call::system_cif;
use crate::result::{Error, Result};

const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
#[cfg(all(windows, target_pointer_width = "32"))]
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
pub const MAX_NATIVE_AGGREGATE_SIZE: usize = 16 * 1024 * 1024;

windows_link::link!("kernel32.dll" "system" "GlobalFree" fn global_free_raw(hmem: *mut c_void) -> *mut c_void);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Bool32,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Pointer,
    FunctionPointer,
    Handle,
}

#[derive(Debug, Clone)]
pub struct NativeAggregateLayout {
    identity: String,
    size: usize,
    alignment: usize,
    ffi_type: FfiType,
}

impl PartialEq for NativeAggregateLayout {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.size == other.size
            && self.alignment == other.alignment
    }
}

impl Eq for NativeAggregateLayout {}

impl NativeAggregateLayout {
    pub fn new(
        identity: impl Into<String>,
        size: usize,
        alignment: usize,
        ffi_type: FfiType,
    ) -> Result<Arc<Self>> {
        let identity = identity.into();
        if identity.trim().is_empty()
            || size == 0
            || alignment == 0
            || !alignment.is_power_of_two()
            || size % alignment != 0
            || alignment > 8
            || size > MAX_NATIVE_AGGREGATE_SIZE
        {
            return Err(invalid_argument("invalid native aggregate call layout"));
        }
        Ok(Arc::new(Self {
            identity,
            size,
            alignment,
            ffi_type,
        }))
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub const fn size(&self) -> usize {
        self.size
    }

    fn libffi_type(&self) -> FfiType {
        self.ffi_type.clone()
    }
}

impl Type {
    fn abi_type(self) -> AbiType {
        match self {
            Self::Bool32 | Self::I32 => AbiType::I32,
            Self::I8 => AbiType::I8,
            Self::U8 => AbiType::U8,
            Self::I16 => AbiType::I16,
            Self::U16 => AbiType::U16,
            Self::U32 => AbiType::U32,
            Self::I64 => AbiType::I64,
            Self::U64 => AbiType::U64,
            Self::F32 => AbiType::F32,
            Self::F64 => AbiType::F64,
            Self::Pointer | Self::FunctionPointer | Self::Handle => AbiType::Ptr,
        }
    }

    #[cfg(not(all(windows, target_pointer_width = "32")))]
    fn is_pointer_like(self) -> bool {
        matches!(self, Self::Pointer | Self::FunctionPointer | Self::Handle)
    }

    fn default_abi_value(self) -> AbiValue {
        self.abi_type().default_value()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    InOut,
}

impl Direction {
    #[cfg(not(all(windows, target_pointer_width = "32")))]
    fn is_input(self) -> bool {
        matches!(self, Self::In | Self::InOut)
    }

    fn is_output(self) -> bool {
        matches!(self, Self::Out | Self::InOut)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup {
    None,
    CloseHandle,
    RegCloseKey,
    LocalFree,
    GlobalFree,
    FreeLibrary,
    CloseServiceHandle,
    CoTaskMemFree,
    CredFree,
}

impl Cleanup {
    fn owns_resource(self) -> bool {
        self != Self::None
    }

    unsafe fn run(self, value: usize) -> windows_core::Result<()> {
        if value == 0 || self == Self::None {
            return Ok(());
        }
        match self {
            Self::None => Ok(()),
            Self::CloseHandle => unsafe { CloseHandle(HANDLE(value as *mut c_void)) },
            Self::RegCloseKey => {
                let status = unsafe { RegCloseKey(HKEY(value as *mut c_void)) };
                if status.0 == 0 {
                    Ok(())
                } else {
                    Err(windows_core::Error::from_hresult(hresult_from_win32(
                        status.0 as u32,
                    )))
                }
            }
            Self::LocalFree => {
                let remaining = unsafe { LocalFree(Some(HLOCAL(value as *mut c_void))) };
                if remaining.is_invalid() {
                    Ok(())
                } else {
                    Err(windows_core::Error::from_hresult(hresult_from_win32(
                        unsafe { GetLastError().0 },
                    )))
                }
            }
            Self::GlobalFree => {
                let remaining = unsafe { global_free_raw(value as *mut c_void) };
                if remaining.is_null() {
                    Ok(())
                } else {
                    Err(windows_core::Error::from_hresult(hresult_from_win32(
                        unsafe { GetLastError().0 },
                    )))
                }
            }
            Self::FreeLibrary => unsafe { FreeLibrary(HMODULE(value as *mut c_void)) },
            Self::CloseServiceHandle => unsafe {
                CloseServiceHandle(SC_HANDLE(value as *mut c_void))
            },
            Self::CoTaskMemFree => {
                unsafe {
                    windows::Win32::System::Com::CoTaskMemFree(Some(value as *const c_void));
                }
                Ok(())
            }
            Self::CredFree => {
                unsafe { CredFree(value as *const c_void) };
                Ok(())
            }
        }
    }
}

/// Releases a native resource whose cleanup contract was validated by a
/// semantic projection and whose ownership has not been adopted elsewhere.
pub unsafe fn cleanup_owned_resource(value: usize, cleanup: Cleanup) -> windows_core::Result<()> {
    unsafe { cleanup.run(value) }
}

fn hresult_from_win32(code: u32) -> HRESULT {
    if code == 0 {
        HRESULT(0)
    } else {
        HRESULT((0x80070000u32 | (code & 0xffff)) as i32)
    }
}

#[derive(Debug)]
pub struct OwnedResource {
    value: Mutex<usize>,
    cleanup: Cleanup,
    async_leases: AtomicUsize,
    active_async_io: AtomicUsize,
}

pub struct OwnedResourceLease<'a> {
    value: std::sync::MutexGuard<'a, usize>,
}

pub struct OwnedResourceAsyncLease {
    resource: Arc<OwnedResource>,
    value: usize,
    active: bool,
}

impl OwnedResourceAsyncLease {
    pub fn raw(&self) -> usize {
        self.value
    }

    pub fn mark_active(&mut self) {
        if !self.active {
            self.resource.active_async_io.fetch_add(1, Ordering::AcqRel);
            self.active = true;
        }
    }

    pub fn mark_inactive(&mut self) {
        if self.active {
            self.resource.active_async_io.fetch_sub(1, Ordering::AcqRel);
            self.active = false;
        }
    }
}

impl Drop for OwnedResourceAsyncLease {
    fn drop(&mut self) {
        self.mark_inactive();
        self.resource.async_leases.fetch_sub(1, Ordering::AcqRel);
    }
}

impl OwnedResourceLease<'_> {
    pub fn raw(&self) -> usize {
        *self.value
    }
}

impl OwnedResource {
    fn new(value: usize, cleanup: Cleanup) -> Self {
        debug_assert!(value != 0);
        debug_assert!(cleanup.owns_resource());
        Self {
            value: Mutex::new(value),
            cleanup,
            async_leases: AtomicUsize::new(0),
            active_async_io: AtomicUsize::new(0),
        }
    }

    /// Adopts a native resource whose exact ownership and cleanup were
    /// validated by the flat Win32 semantic projection.
    pub unsafe fn adopt(value: usize, cleanup: Cleanup) -> Result<Arc<Self>> {
        if value == 0 || !cleanup.owns_resource() {
            return Err(invalid_argument(
                "owned Win32 resource requires a nonzero value and cleanup",
            ));
        }
        Ok(Arc::new(Self::new(value, cleanup)))
    }

    pub fn raw(&self) -> usize {
        *self.value.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn is_closed(&self) -> bool {
        self.raw() == 0
    }

    pub fn cleanup(&self) -> Cleanup {
        self.cleanup
    }

    pub fn has_async_leases(&self) -> bool {
        self.async_leases.load(Ordering::Acquire) != 0
    }

    pub fn has_active_async_io(&self) -> bool {
        self.active_async_io.load(Ordering::Acquire) != 0
    }

    pub fn lease(&self, expected_cleanup: Cleanup) -> Result<OwnedResourceLease<'_>> {
        if self.cleanup != expected_cleanup {
            return Err(invalid_argument(
                "managed Win32 resource cleanup kind does not match",
            ));
        }
        let value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if *value == 0 {
            return Err(invalid_argument("cannot lease a closed Win32 resource"));
        }
        Ok(OwnedResourceLease { value })
    }

    fn lock_for_call(&self, consumes_resource: bool) -> Result<std::sync::MutexGuard<'_, usize>> {
        let value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if consumes_resource && self.has_async_leases() {
            return Err(invalid_argument(
                "cannot consume a Win32 resource while asynchronous I/O is pending",
            ));
        }
        Ok(value)
    }

    pub fn async_lease(
        self: &Arc<Self>,
        expected_cleanup: Cleanup,
    ) -> Result<OwnedResourceAsyncLease> {
        if self.cleanup != expected_cleanup {
            return Err(invalid_argument(
                "managed Win32 resource cleanup kind does not match",
            ));
        }
        let value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if *value == 0 {
            return Err(invalid_argument("cannot lease a closed Win32 resource"));
        }
        self.async_leases.fetch_add(1, Ordering::AcqRel);
        Ok(OwnedResourceAsyncLease {
            resource: Arc::clone(self),
            value: *value,
            active: false,
        })
    }

    pub fn close(&self) -> windows_core::Result<()> {
        let mut value = self.value.lock().unwrap_or_else(|error| error.into_inner());
        if self.has_async_leases() {
            return Err(windows_core::Error::new(
                HRESULT(0x800700AAu32 as i32),
                "cannot close a Win32 resource while asynchronous I/O is pending",
            ));
        }
        unsafe { self.cleanup.run(*value) }?;
        *value = 0;
        Ok(())
    }
}

impl Drop for OwnedResource {
    fn drop(&mut self) {
        let value = *self
            .value
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let _ = unsafe { self.cleanup.run(value) };
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Bool(bool),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Pointer(*mut c_void),
    FunctionPointer(usize),
    Handle(usize),
    Resource(Arc<OwnedResource>),
    Aggregate {
        layout: Arc<NativeAggregateLayout>,
        pointer: *mut c_void,
    },
    OwnedAggregate {
        layout: Arc<NativeAggregateLayout>,
        bytes: Vec<u8>,
    },
    Null,
}

unsafe impl Send for Value {}
unsafe impl Sync for Value {}

impl Value {
    pub fn resource(&self) -> Option<&Arc<OwnedResource>> {
        match self {
            Self::Resource(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    pub typ: Type,
    pub direction: Direction,
    pub nullable: bool,
    pub cleanup: Cleanup,
    pub consumes_resource: bool,
    pub resource_cleanup: Cleanup,
}

impl Parameter {
    pub const fn input(typ: Type, nullable: bool) -> Self {
        Self {
            typ,
            direction: Direction::In,
            nullable,
            cleanup: Cleanup::None,
            consumes_resource: false,
            resource_cleanup: Cleanup::None,
        }
    }

    pub const fn output(typ: Type, cleanup: Cleanup) -> Self {
        Self {
            typ,
            direction: Direction::Out,
            nullable: false,
            cleanup,
            consumes_resource: false,
            resource_cleanup: Cleanup::None,
        }
    }

    pub const fn input_output(typ: Type, nullable: bool, cleanup: Cleanup) -> Self {
        Self {
            typ,
            direction: Direction::InOut,
            nullable,
            cleanup,
            consumes_resource: false,
            resource_cleanup: Cleanup::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessRule {
    Always,
    ReturnZero,
    ReturnNonZero,
    ReturnNonNull,
    HResultSucceeded,
    SignedNonNegative,
    ReturnValidHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallingConvention {
    System,
    Cdecl,
}

#[derive(Debug, Clone)]
pub struct CallPlanSpec {
    pub dll: String,
    pub entry_point: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<Type>,
    pub return_cleanup: Cleanup,
    pub success_rule: SuccessRule,
    pub capture_last_error: bool,
    pub calling_convention: CallingConvention,
    pub parameter_aggregates: Vec<Option<Arc<NativeAggregateLayout>>>,
    pub return_aggregate: Option<Arc<NativeAggregateLayout>>,
}

#[derive(Debug)]
struct PlannedParameter {
    spec: Parameter,
    input_index: Option<usize>,
    output_index: Option<usize>,
}

#[derive(Debug)]
pub struct CallPlan {
    dll: String,
    entry_point: String,
    function: usize,
    parameters: Vec<PlannedParameter>,
    input_count: usize,
    output_count: usize,
    return_type: Option<Type>,
    return_cleanup: Cleanup,
    success_rule: SuccessRule,
    capture_last_error: bool,
    calling_convention: CallingConvention,
    parameter_aggregates: Vec<Option<Arc<NativeAggregateLayout>>>,
    return_aggregate: Option<Arc<NativeAggregateLayout>>,
    cif: Cif,
}

// Safety: plans are completely built before publication and immutable
// afterwards. libffi reads the prepared CIF and type graph during ffi_call.
unsafe impl Send for CallPlan {}
unsafe impl Sync for CallPlan {}

#[derive(Debug)]
pub struct CallResult {
    pub return_value: Option<Value>,
    pub outputs: Vec<Value>,
    pub last_error: Option<u32>,
    pub succeeded: bool,
}

impl CallPlan {
    /// # Safety
    ///
    /// The specification must exactly match the target export's native ABI.
    pub unsafe fn new(spec: CallPlanSpec) -> Result<Arc<Self>> {
        #[cfg(all(windows, target_pointer_width = "32"))]
        {
            let _ = spec;
            return Err(not_implemented(
                "flat Win32 plans currently reject 32-bit targets because metadata calling conventions are not yet projected",
            ));
        }

        #[cfg(not(all(windows, target_pointer_width = "32")))]
        {
            validate_spec(&spec)?;
            let module = get_cached_module(&spec.dll)?;
            let function = proc_address(module, &spec.dll, &spec.entry_point)? as usize;

            let mut input_count = 0;
            let mut output_count = 0;
            let parameters = spec
                .parameters
                .iter()
                .copied()
                .map(|parameter| {
                    let input_index = parameter.direction.is_input().then(|| {
                        let index = input_count;
                        input_count += 1;
                        index
                    });
                    let output_index = parameter.direction.is_output().then(|| {
                        let index = output_count;
                        output_count += 1;
                        index
                    });
                    PlannedParameter {
                        spec: parameter,
                        input_index,
                        output_index,
                    }
                })
                .collect::<Vec<_>>();
            if spec.parameter_aggregates.len() != parameters.len() {
                return Err(invalid_argument(
                    "flat Win32 aggregate descriptor count must match native parameters",
                ));
            }
            let argument_types = parameters
                .iter()
                .enumerate()
                .map(|(index, parameter)| {
                    if parameter.spec.direction.is_output() {
                        FfiType::pointer()
                    } else if let Some(layout) = &spec.parameter_aggregates[index] {
                        layout.libffi_type()
                    } else {
                        parameter.spec.typ.abi_type().libffi_type()
                    }
                })
                .collect::<Vec<_>>();
            let return_ffi_type = spec.return_aggregate.as_ref().map_or_else(
                || {
                    spec.return_type
                        .map_or_else(FfiType::void, |typ| typ.abi_type().libffi_type())
                },
                |layout| layout.libffi_type(),
            );
            let cif = match spec.calling_convention {
                CallingConvention::System => system_cif(argument_types, return_ffi_type),
                CallingConvention::Cdecl => Cif::new(argument_types, return_ffi_type),
            };

            Ok(Arc::new(Self {
                dll: spec.dll,
                entry_point: spec.entry_point,
                function,
                parameters,
                input_count,
                output_count,
                return_type: spec.return_type,
                return_cleanup: spec.return_cleanup,
                success_rule: spec.success_rule,
                capture_last_error: spec.capture_last_error,
                calling_convention: spec.calling_convention,
                parameter_aggregates: spec.parameter_aggregates,
                return_aggregate: spec.return_aggregate,
                cif,
            }))
        }
    }

    pub fn dll(&self) -> &str {
        &self.dll
    }

    pub fn entry_point(&self) -> &str {
        &self.entry_point
    }

    pub fn input_count(&self) -> usize {
        self.input_count
    }

    pub fn output_count(&self) -> usize {
        self.output_count
    }

    pub fn calling_convention(&self) -> CallingConvention {
        self.calling_convention
    }

    /// # Safety
    ///
    /// Every pointer and aggregate argument must remain valid for the complete
    /// native call and satisfy the contract used to construct this plan.
    pub unsafe fn invoke(&self, args: &[Value]) -> Result<CallResult> {
        if args.len() != self.input_count {
            return Err(invalid_argument(&format!(
                "{}!{} expects {} inputs, received {}",
                self.dll,
                self.entry_point,
                self.input_count,
                args.len()
            )));
        }
        for parameter in &self.parameters {
            if !parameter.spec.consumes_resource {
                continue;
            }
            let input_index = parameter
                .input_index
                .expect("consuming parameter is an input");
            let Value::Resource(resource) = &args[input_index] else {
                return Err(invalid_argument(
                    "consuming Win32 parameters require a managed resource object",
                ));
            };
            if resource.cleanup() != parameter.spec.resource_cleanup {
                return Err(invalid_argument(
                    "managed Win32 resource cleanup kind does not match the consuming parameter",
                ));
            }
        }

        let mut resources = args
            .iter()
            .enumerate()
            .filter_map(|(index, value)| match value {
                Value::Resource(resource) => Some((index, resource)),
                _ => None,
            })
            .collect::<Vec<_>>();
        resources.sort_by_key(|(_, resource)| Arc::as_ptr(resource) as usize);
        if resources
            .windows(2)
            .any(|pair| Arc::ptr_eq(pair[0].1, pair[1].1))
        {
            return Err(invalid_argument(
                "the same managed Win32 resource cannot occupy multiple parameters in one call",
            ));
        }
        let mut resource_guard_by_arg = vec![None; args.len()];
        let mut resource_guards = Vec::with_capacity(resources.len());
        for (arg_index, resource) in resources {
            let consumes_resource = self.parameters.iter().any(|parameter| {
                parameter.input_index == Some(arg_index) && parameter.spec.consumes_resource
            });
            let guard = resource.lock_for_call(consumes_resource)?;
            let guard_index = resource_guards.len();
            resource_guard_by_arg[arg_index] = Some(guard_index);
            resource_guards.push(guard);
        }

        let input_storage = self
            .parameters
            .iter()
            .enumerate()
            .map(|(parameter_index, parameter)| {
                if self.parameter_aggregates[parameter_index].is_some() {
                    return Ok(None);
                }
                parameter
                    .input_index
                    .map(|index| {
                        value_to_abi(
                            parameter.spec.typ,
                            &args[index],
                            parameter.spec.nullable,
                            parameter.spec.resource_cleanup,
                            resource_guard_by_arg[index]
                                .map(|guard_index| *resource_guards[guard_index]),
                        )
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        let mut output_storage = self
            .parameters
            .iter()
            .filter(|parameter| parameter.spec.direction.is_output())
            .map(|parameter| {
                if parameter.spec.direction == Direction::InOut {
                    value_to_abi(
                        parameter.spec.typ,
                        &args[parameter.input_index.expect("in/out input")],
                        parameter.spec.nullable,
                        parameter.spec.resource_cleanup,
                        parameter
                            .input_index
                            .and_then(|index| resource_guard_by_arg[index])
                            .map(|guard_index| *resource_guards[guard_index]),
                    )
                } else {
                    Ok(parameter.spec.typ.default_abi_value())
                }
            })
            .collect::<Result<Vec<_>>>()?;
        debug_assert_eq!(output_storage.len(), self.output_count);
        let output_pointers = output_storage
            .iter()
            .map(|value| value.as_out_ptr() as *mut c_void)
            .collect::<Vec<_>>();

        let mut ffi_args = Vec::<Arg<'_>>::with_capacity(self.parameters.len());
        for (index, parameter) in self.parameters.iter().enumerate() {
            if let Some(output_index) = parameter.output_index {
                ffi_args.push(arg(&output_pointers[output_index]));
            } else if let Some(expected) = &self.parameter_aggregates[index] {
                let input_index = parameter.input_index.expect("aggregate input index");
                let Value::Aggregate { layout, pointer } = &args[input_index] else {
                    return Err(invalid_argument(
                        "flat Win32 argument does not match native aggregate",
                    ));
                };
                if layout != expected || pointer.is_null() {
                    return Err(invalid_argument(
                        "flat Win32 native aggregate identity mismatch",
                    ));
                }
                ffi_args.push(Arg::new(unsafe { &*pointer.cast::<u8>() }));
            } else {
                ffi_args.push(abi_arg(
                    input_storage[index]
                        .as_ref()
                        .expect("input parameter has ABI storage"),
                ));
            }
        }

        let mut aggregate_return_words = self
            .return_aggregate
            .as_ref()
            .map(|layout| try_zeroed_words(layout.size()))
            .transpose()?;
        let raw_return = if let (Some(layout), Some(words)) =
            (&self.return_aggregate, aggregate_return_words.as_mut())
        {
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), layout.size())
            };
            unsafe {
                self.cif.call_return_into(
                    libffi::middle::CodePtr(self.function as *mut c_void),
                    &ffi_args,
                    Ret::new(bytes),
                )
            };
            None
        } else {
            unsafe {
                crate::call::call_native_function(
                    &self.cif,
                    self.function as *mut c_void,
                    &ffi_args,
                    self.return_type.map(Type::abi_type),
                )
            }
        };
        let last_error = self.capture_last_error.then(|| unsafe { GetLastError().0 });
        let succeeded = success_matches(self.success_rule, raw_return.as_ref())?;
        if succeeded {
            for parameter in &self.parameters {
                if !parameter.spec.consumes_resource {
                    continue;
                }
                let input_index = parameter
                    .input_index
                    .expect("consuming parameter is an input");
                if let Some(guard_index) = resource_guard_by_arg[input_index] {
                    *resource_guards[guard_index] = 0;
                }
            }
        }

        let return_value = if let (Some(layout), Some(words)) =
            (&self.return_aggregate, aggregate_return_words)
        {
            let source =
                unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), layout.size()) };
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(source.len())
                .map_err(|_| out_of_memory("native aggregate return"))?;
            bytes.extend_from_slice(source);
            Some(Value::OwnedAggregate {
                layout: Arc::clone(layout),
                bytes,
            })
        } else {
            match (self.return_type, raw_return) {
                (Some(typ), Some(value)) => Some(finalize_value(
                    typ,
                    value,
                    self.return_cleanup,
                    succeeded,
                    false,
                )?),
                (None, None) => None,
                _ => {
                    return Err(invalid_argument(
                        "flat Win32 call plan produced an inconsistent return value",
                    ));
                }
            }
        };

        let mut outputs = Vec::with_capacity(self.output_count);
        for parameter in &self.parameters {
            let Some(output_index) = parameter.output_index else {
                continue;
            };
            let raw = std::mem::replace(
                &mut output_storage[output_index],
                parameter.spec.typ.default_abi_value(),
            );
            outputs.push(finalize_value(
                parameter.spec.typ,
                raw,
                parameter.spec.cleanup,
                succeeded,
                true,
            )?);
        }

        Ok(CallResult {
            return_value,
            outputs,
            last_error,
            succeeded,
        })
    }
}

fn try_zeroed_words(byte_length: usize) -> Result<Vec<u64>> {
    let word_length = byte_length.div_ceil(std::mem::size_of::<u64>());
    let mut words = Vec::new();
    words
        .try_reserve_exact(word_length)
        .map_err(|_| out_of_memory("native aggregate storage"))?;
    words.resize(word_length, 0);
    Ok(words)
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn validate_spec(spec: &CallPlanSpec) -> Result<()> {
    if !is_bare_system_module_name(&spec.dll) {
        return Err(invalid_argument(
            "flat Win32 DLL names must be bare .dll or .drv names loaded from System32",
        ));
    }
    if spec.entry_point.is_empty() || spec.entry_point.as_bytes().contains(&0) {
        return Err(invalid_argument(
            "flat Win32 entry point must be a non-empty NUL-free export name",
        ));
    }
    if spec.return_cleanup.owns_resource() && !spec.return_type.is_some_and(Type::is_pointer_like) {
        return Err(invalid_argument(
            "owned flat Win32 returns must be pointer or handle values",
        ));
    }
    if spec.return_type.is_some() && spec.return_aggregate.is_some() {
        return Err(invalid_argument(
            "flat Win32 return cannot be both scalar and aggregate",
        ));
    }
    if spec.return_aggregate.is_some() && spec.return_cleanup != Cleanup::None {
        return Err(invalid_argument(
            "native aggregate returns cannot use pointer cleanup",
        ));
    }
    if spec.success_rule != SuccessRule::Always && spec.return_type.is_none() {
        return Err(invalid_argument(
            "flat Win32 success rules require a direct return value",
        ));
    }
    if matches!(
        spec.success_rule,
        SuccessRule::HResultSucceeded | SuccessRule::SignedNonNegative
    ) && spec.return_type != Some(Type::I32)
    {
        return Err(invalid_argument(
            "signed success rules require an i32 return type",
        ));
    }
    for (index, parameter) in spec.parameters.iter().enumerate() {
        if spec
            .parameter_aggregates
            .get(index)
            .is_some_and(Option::is_some)
            && (parameter.direction != Direction::In || parameter.typ != Type::Pointer)
        {
            return Err(invalid_argument(
                "by-value native aggregates require direct pointer-typed input plans",
            ));
        }
        if parameter.nullable && !parameter.typ.is_pointer_like() {
            return Err(invalid_argument(
                "only pointer and handle inputs may be nullable",
            ));
        }
        if parameter.cleanup.owns_resource()
            && (!parameter.direction.is_output() || !parameter.typ.is_pointer_like())
        {
            return Err(invalid_argument(
                "flat Win32 cleanup applies only to pointer or handle outputs",
            ));
        }
        if parameter.consumes_resource
            && (parameter.direction != Direction::In
                || parameter.typ != Type::Handle
                || !parameter.resource_cleanup.owns_resource())
        {
            return Err(invalid_argument(
                "consuming Win32 parameters must be direct managed handle inputs with exact cleanup",
            ));
        }
        if parameter.resource_cleanup != Cleanup::None
            && (parameter.direction == Direction::Out || parameter.typ != Type::Handle)
        {
            return Err(invalid_argument(
                "managed resource compatibility applies only to handle inputs",
            ));
        }
    }
    Ok(())
}

fn value_to_abi(
    typ: Type,
    value: &Value,
    nullable: bool,
    resource_cleanup: Cleanup,
    resource_bits: Option<usize>,
) -> Result<AbiValue> {
    let mismatch = || invalid_argument(&format!("flat Win32 argument does not match {typ:?}"));
    Ok(match (typ, value) {
        (Type::Bool32, Value::Bool(value)) => AbiValue::I32(i32::from(*value)),
        (Type::I8, Value::I8(value)) => AbiValue::I8(*value),
        (Type::U8, Value::U8(value)) => AbiValue::U8(*value),
        (Type::I16, Value::I16(value)) => AbiValue::I16(*value),
        (Type::U16, Value::U16(value)) => AbiValue::U16(*value),
        (Type::I32, Value::I32(value)) => AbiValue::I32(*value),
        (Type::U32, Value::U32(value)) => AbiValue::U32(*value),
        (Type::I64, Value::I64(value)) => AbiValue::I64(*value),
        (Type::U64, Value::U64(value)) => AbiValue::U64(*value),
        (Type::F32, Value::F32(value)) => AbiValue::F32(*value),
        (Type::F64, Value::F64(value)) => AbiValue::F64(*value),
        (Type::Pointer, Value::Pointer(value)) => AbiValue::Pointer(*value),
        (Type::FunctionPointer, Value::FunctionPointer(value)) => {
            AbiValue::Pointer(*value as *mut c_void)
        }
        (Type::Handle, Value::Handle(value)) => AbiValue::Pointer(*value as *mut c_void),
        (Type::Handle, Value::Resource(resource)) => {
            if resource_cleanup == Cleanup::None || resource.cleanup() != resource_cleanup {
                return Err(invalid_argument(
                    "managed Win32 resource cleanup kind does not match the handle parameter",
                ));
            }
            let raw = resource_bits.ok_or_else(|| {
                invalid_argument("managed Win32 resource has no active call lease")
            })?;
            if raw == 0 {
                return Err(invalid_argument(
                    "cannot pass a closed flat Win32 resource handle",
                ));
            }
            AbiValue::Pointer(raw as *mut c_void)
        }
        (Type::Pointer | Type::FunctionPointer | Type::Handle, Value::Null) if nullable => {
            AbiValue::Pointer(std::ptr::null_mut())
        }
        _ => return Err(mismatch()),
    })
}

fn abi_arg(value: &AbiValue) -> Arg<'_> {
    match value {
        AbiValue::Bool(value) => arg(value),
        AbiValue::I8(value) => arg(value),
        AbiValue::U8(value) => arg(value),
        AbiValue::I16(value) => arg(value),
        AbiValue::U16(value) => arg(value),
        AbiValue::I32(value) => arg(value),
        AbiValue::U32(value) => arg(value),
        AbiValue::I64(value) => arg(value),
        AbiValue::U64(value) => arg(value),
        AbiValue::F32(value) => arg(value),
        AbiValue::F64(value) => arg(value),
        AbiValue::Guid(value) => arg(value),
        AbiValue::Pointer(value) => arg(value),
    }
}

fn finalize_value(
    typ: Type,
    value: AbiValue,
    cleanup: Cleanup,
    succeeded: bool,
    cleanup_on_failure: bool,
) -> Result<Value> {
    if cleanup.owns_resource() {
        let bits = match value {
            AbiValue::Pointer(value) => value as usize,
            _ => {
                return Err(invalid_argument(
                    "owned flat Win32 output was not pointer-shaped",
                ));
            }
        };
        if !succeeded {
            if cleanup_on_failure {
                unsafe { cleanup.run(bits) }.map_err(Error::WindowsError)?;
            }
            return Ok(Value::Handle(0));
        }
        return if bits == 0 {
            Ok(Value::Handle(0))
        } else {
            Ok(Value::Resource(Arc::new(OwnedResource::new(bits, cleanup))))
        };
    }

    Ok(match (typ, value) {
        (Type::Bool32, AbiValue::I32(value)) => Value::Bool(value != 0),
        (Type::I8, AbiValue::I8(value)) => Value::I8(value),
        (Type::U8, AbiValue::U8(value)) => Value::U8(value),
        (Type::I16, AbiValue::I16(value)) => Value::I16(value),
        (Type::U16, AbiValue::U16(value)) => Value::U16(value),
        (Type::I32, AbiValue::I32(value)) => Value::I32(value),
        (Type::U32, AbiValue::U32(value)) => Value::U32(value),
        (Type::I64, AbiValue::I64(value)) => Value::I64(value),
        (Type::U64, AbiValue::U64(value)) => Value::U64(value),
        (Type::F32, AbiValue::F32(value)) => Value::F32(value),
        (Type::F64, AbiValue::F64(value)) => Value::F64(value),
        (Type::Pointer, AbiValue::Pointer(value)) => Value::Pointer(value),
        (Type::FunctionPointer, AbiValue::Pointer(value)) => Value::FunctionPointer(value as usize),
        (Type::Handle, AbiValue::Pointer(value)) => Value::Handle(value as usize),
        _ => {
            return Err(invalid_argument(
                "flat Win32 native result did not match its immutable call plan",
            ));
        }
    })
}

fn success_matches(rule: SuccessRule, value: Option<&AbiValue>) -> Result<bool> {
    let scalar = |value: &AbiValue| -> Option<i128> {
        match value {
            AbiValue::Bool(value) => Some(*value as i128),
            AbiValue::I8(value) => Some(*value as i128),
            AbiValue::U8(value) => Some(*value as i128),
            AbiValue::I16(value) => Some(*value as i128),
            AbiValue::U16(value) => Some(*value as i128),
            AbiValue::I32(value) => Some(*value as i128),
            AbiValue::U32(value) => Some(*value as i128),
            AbiValue::I64(value) => Some(*value as i128),
            AbiValue::U64(value) => Some(*value as i128),
            AbiValue::Pointer(value) => Some(*value as usize as i128),
            AbiValue::F32(_) | AbiValue::F64(_) | AbiValue::Guid(_) => None,
        }
    };
    match rule {
        SuccessRule::Always => Ok(true),
        SuccessRule::ReturnZero => value
            .and_then(scalar)
            .map(|value| value == 0)
            .ok_or_else(|| invalid_argument("ReturnZero requires an integer or pointer return")),
        SuccessRule::ReturnNonZero | SuccessRule::ReturnNonNull => value
            .and_then(scalar)
            .map(|value| value != 0)
            .ok_or_else(|| invalid_argument("nonzero success rule requires a scalar return")),
        SuccessRule::HResultSucceeded => match value {
            Some(AbiValue::I32(value)) => Ok(*value >= 0),
            _ => Err(invalid_argument(
                "HRESULT success rule requires a signed 32-bit return",
            )),
        },
        SuccessRule::SignedNonNegative => match value {
            Some(AbiValue::I32(value)) => Ok(*value >= 0),
            _ => Err(invalid_argument(
                "signed-nonnegative success rule requires a signed 32-bit return",
            )),
        },
        SuccessRule::ReturnValidHandle => match value {
            Some(AbiValue::Pointer(value)) => {
                let bits = *value as usize;
                Ok(bits != 0 && bits != usize::MAX)
            }
            _ => Err(invalid_argument(
                "valid-handle success rule requires a pointer return",
            )),
        },
    }
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
struct CachedModule(HMODULE);

#[cfg(not(all(windows, target_pointer_width = "32")))]
unsafe impl Send for CachedModule {}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn module_cache() -> &'static Mutex<HashMap<String, CachedModule>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedModule>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn get_cached_module(dll: &str) -> Result<HMODULE> {
    if !is_bare_system_module_name(dll) {
        return Err(invalid_argument("invalid System32 module name"));
    }
    let key = dll.to_ascii_lowercase();
    if let Some(module) = module_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .map(|module| module.0)
    {
        return Ok(module);
    }
    let module = unsafe { LoadLibraryExW(&HSTRING::from(dll), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(Error::WindowsError)?;
    let mut cache = module_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    Ok(cache.entry(key).or_insert(CachedModule(module)).0)
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn proc_address(module: HMODULE, dll: &str, entry: &str) -> Result<*mut c_void> {
    let entry = CString::new(entry).map_err(|_| invalid_argument("invalid export name"))?;
    let function = unsafe { GetProcAddress(module, PCSTR(entry.as_ptr().cast())) };
    function
        .map(|function| function as *const () as *mut c_void)
        .ok_or_else(|| {
            Error::WindowsError(windows_core::Error::new(
                HRESULT(0x8007007Fu32 as i32),
                &format!(
                    "Export `{}` was not found in `{dll}`",
                    entry.to_string_lossy()
                ),
            ))
        })
}

#[cfg(not(all(windows, target_pointer_width = "32")))]
fn is_bare_system_module_name(dll: &str) -> bool {
    let lower = dll.to_ascii_lowercase();
    !dll.is_empty()
        && (lower.ends_with(".dll") || lower.ends_with(".drv"))
        && !dll.encode_utf16().any(|unit| unit == 0)
        && !dll
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
        && !matches!(dll, "." | "..")
}

fn invalid_argument(message: &str) -> Error {
    Error::WindowsError(windows_core::Error::new(E_INVALIDARG, message))
}

fn out_of_memory(context: &str) -> Error {
    Error::WindowsError(windows_core::Error::new(
        HRESULT(0x8007000Eu32 as i32),
        format!("Unable to allocate {context}"),
    ))
}

#[cfg(all(windows, target_pointer_width = "32"))]
fn not_implemented(message: &str) -> Error {
    Error::WindowsError(windows_core::Error::new(E_NOTIMPL, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(all(windows, target_pointer_width = "32")))]
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(not(all(windows, target_pointer_width = "32")))]
    fn mul_div_plan() -> Arc<CallPlan> {
        unsafe {
            CallPlan::new(CallPlanSpec {
                dll: "kernel32.dll".into(),
                entry_point: "MulDiv".into(),
                parameters: vec![
                    Parameter::input(Type::I32, false),
                    Parameter::input(Type::I32, false),
                    Parameter::input(Type::I32, false),
                ],
                return_type: Some(Type::I32),
                return_cleanup: Cleanup::None,
                success_rule: SuccessRule::Always,
                capture_last_error: false,
                calling_convention: CallingConvention::System,
                parameter_aggregates: vec![None; 3],
                return_aggregate: None,
            })
        }
        .unwrap()
    }

    #[test]
    fn immutable_plan_invokes_exact_scalar_signature() {
        #[cfg(target_pointer_width = "64")]
        {
            let plan = mul_div_plan();
            let result =
                unsafe { plan.invoke(&[Value::I32(100), Value::I32(3), Value::I32(2)]) }.unwrap();
            assert!(matches!(result.return_value, Some(Value::I32(150))));
            assert!(result.outputs.is_empty());
        }
    }

    #[test]
    fn immutable_plan_rejects_wrong_argument_kind_before_dispatch() {
        #[cfg(target_pointer_width = "64")]
        {
            let plan = mul_div_plan();
            let error = unsafe { plan.invoke(&[Value::U32(100), Value::I32(3), Value::I32(2)]) }
                .unwrap_err();
            assert!(error.message().contains("does not match I32"));
        }
    }

    #[test]
    fn cdecl_plan_invokes_ldap_scalar_export() {
        #[cfg(target_pointer_width = "64")]
        {
            let plan = unsafe {
                CallPlan::new(CallPlanSpec {
                    dll: "wldap32.dll".into(),
                    entry_point: "LdapGetLastError".into(),
                    parameters: vec![],
                    return_type: Some(Type::U32),
                    return_cleanup: Cleanup::None,
                    success_rule: SuccessRule::Always,
                    capture_last_error: false,
                    calling_convention: CallingConvention::Cdecl,
                    parameter_aggregates: vec![],
                    return_aggregate: None,
                })
            }
            .unwrap();
            assert_eq!(plan.calling_convention(), CallingConvention::Cdecl);
            let result = unsafe { plan.invoke(&[]) }.unwrap();
            assert!(matches!(result.return_value, Some(Value::U32(_))));
        }
    }

    #[cfg(not(all(windows, target_pointer_width = "32")))]
    #[test]
    fn system32_policy_rejects_paths() {
        assert!(!is_bare_system_module_name(
            r"C:\Windows\System32\kernel32.dll"
        ));
        assert!(is_bare_system_module_name("kernel32.dll"));
    }

    #[cfg(all(windows, target_pointer_width = "32"))]
    #[test]
    fn x86_plan_binding_fails_before_loading_or_dispatch() {
        let error = unsafe {
            CallPlan::new(CallPlanSpec {
                dll: "kernel32.dll".into(),
                entry_point: "MulDiv".into(),
                parameters: vec![
                    Parameter::input(Type::I32, false),
                    Parameter::input(Type::I32, false),
                    Parameter::input(Type::I32, false),
                ],
                return_type: Some(Type::I32),
                return_cleanup: Cleanup::None,
                success_rule: SuccessRule::Always,
                capture_last_error: false,
                calling_convention: CallingConvention::System,
                parameter_aggregates: vec![None; 3],
                return_aggregate: None,
            })
        }
        .unwrap_err();
        assert!(error.message().contains("reject 32-bit targets"));
    }

    #[test]
    fn call_plan_is_publishable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CallPlan>();
    }

    #[test]
    fn hresult_success_rule_accepts_s_false_and_rejects_failures() {
        assert!(success_matches(SuccessRule::HResultSucceeded, Some(&AbiValue::I32(1))).unwrap());
        assert!(!success_matches(SuccessRule::HResultSucceeded, Some(&AbiValue::I32(-1))).unwrap());
    }

    #[test]
    fn failed_direct_handle_sentinel_is_never_cleaned() {
        let value = finalize_value(
            Type::Handle,
            AbiValue::Pointer(usize::MAX as *mut c_void),
            Cleanup::CloseHandle,
            false,
            false,
        )
        .unwrap();
        assert!(matches!(value, Value::Handle(0)));
    }

    #[test]
    fn registry_handle_output_is_adopted_only_on_success() {
        #[cfg(target_pointer_width = "64")]
        {
            let plan = unsafe {
                CallPlan::new(CallPlanSpec {
                    dll: "advapi32.dll".into(),
                    entry_point: "RegOpenKeyExW".into(),
                    parameters: vec![
                        Parameter::input(Type::Handle, false),
                        Parameter::input(Type::Pointer, false),
                        Parameter::input(Type::U32, false),
                        Parameter::input(Type::U32, false),
                        Parameter::output(Type::Handle, Cleanup::RegCloseKey),
                    ],
                    return_type: Some(Type::I32),
                    return_cleanup: Cleanup::None,
                    success_rule: SuccessRule::ReturnZero,
                    capture_last_error: false,
                    calling_convention: CallingConvention::System,
                    parameter_aggregates: vec![None; 5],
                    return_aggregate: None,
                })
            }
            .unwrap();
            let existing = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
            let opened = unsafe {
                plan.invoke(&[
                    Value::Handle(0x80000002),
                    Value::Pointer(existing.as_ptr() as *mut c_void),
                    Value::U32(0),
                    Value::U32(0x20019),
                ])
            }
            .unwrap();
            assert!(opened.succeeded);
            let resource = opened.outputs[0].resource().unwrap();
            assert_ne!(resource.raw(), 0);
            let wrong_cleanup = unsafe {
                CallPlan::new(CallPlanSpec {
                    dll: "kernel32.dll".into(),
                    entry_point: "CloseHandle".into(),
                    parameters: vec![Parameter {
                        typ: Type::Handle,
                        direction: Direction::In,
                        nullable: false,
                        cleanup: Cleanup::None,
                        consumes_resource: true,
                        resource_cleanup: Cleanup::CloseHandle,
                    }],
                    return_type: Some(Type::Bool32),
                    return_cleanup: Cleanup::None,
                    success_rule: SuccessRule::ReturnNonZero,
                    capture_last_error: true,
                    calling_convention: CallingConvention::System,
                    parameter_aggregates: vec![None],
                    return_aggregate: None,
                })
            }
            .unwrap();
            let error = unsafe { wrong_cleanup.invoke(&[Value::Resource(Arc::clone(resource))]) }
                .unwrap_err();
            assert!(error.message().contains("cleanup kind does not match"));
            assert!(!resource.is_closed());
            let close = unsafe {
                CallPlan::new(CallPlanSpec {
                    dll: "advapi32.dll".into(),
                    entry_point: "RegCloseKey".into(),
                    parameters: vec![Parameter {
                        typ: Type::Handle,
                        direction: Direction::In,
                        nullable: false,
                        cleanup: Cleanup::None,
                        consumes_resource: true,
                        resource_cleanup: Cleanup::RegCloseKey,
                    }],
                    return_type: Some(Type::I32),
                    return_cleanup: Cleanup::None,
                    success_rule: SuccessRule::ReturnZero,
                    capture_last_error: false,
                    calling_convention: CallingConvention::System,
                    parameter_aggregates: vec![None],
                    return_aggregate: None,
                })
            }
            .unwrap();
            let raw_error = unsafe { close.invoke(&[Value::Handle(resource.raw())]) }.unwrap_err();
            assert!(
                raw_error
                    .message()
                    .contains("require a managed resource object")
            );
            assert!(!resource.is_closed());
            let closed = unsafe { close.invoke(&[Value::Resource(Arc::clone(resource))]) }.unwrap();
            assert!(closed.succeeded);
            assert!(resource.is_closed());
            resource.close().unwrap();

            let missing = wide(r"SOFTWARE\DynWinRT\DefinitelyMissing");
            let failed = unsafe {
                plan.invoke(&[
                    Value::Handle(0x80000002),
                    Value::Pointer(missing.as_ptr() as *mut c_void),
                    Value::U32(0),
                    Value::U32(0x20019),
                ])
            }
            .unwrap();
            assert!(!failed.succeeded);
            assert!(matches!(failed.outputs[0], Value::Handle(0)));
        }
    }

    #[test]
    fn native_aggregate_layout_rejects_excessive_sizes() {
        let error = NativeAggregateLayout::new(
            "Tests.Huge",
            MAX_NATIVE_AGGREGATE_SIZE + 8,
            8,
            FfiType::structure(vec![FfiType::u64()]),
        )
        .unwrap_err();
        assert!(error.message().contains("invalid native aggregate"));
    }

    #[test]
    fn global_alloc_resource_uses_global_free_once() {
        use windows::Win32::System::Memory::{GMEM_FIXED, GlobalAlloc};

        let allocation = unsafe { GlobalAlloc(GMEM_FIXED, 32) }.unwrap();
        let resource = OwnedResource::new(allocation.0 as usize, Cleanup::GlobalFree);
        assert!(!resource.is_closed());
        resource.close().unwrap();
        assert!(resource.is_closed());
        resource.close().unwrap();
    }

    #[test]
    fn consuming_call_lock_rejects_an_async_lease() {
        use windows::Win32::System::Memory::{GMEM_FIXED, GlobalAlloc};

        let allocation = unsafe { GlobalAlloc(GMEM_FIXED, 32) }.unwrap();
        let resource =
            unsafe { OwnedResource::adopt(allocation.0 as usize, Cleanup::GlobalFree) }.unwrap();
        let lease = resource.async_lease(Cleanup::GlobalFree).unwrap();
        let error = resource.lock_for_call(true).unwrap_err();
        assert!(error.message().contains("asynchronous I/O is pending"));
        drop(lease);

        let guard = resource.lock_for_call(true).unwrap();
        assert_eq!(*guard, allocation.0 as usize);
        drop(guard);
        resource.close().unwrap();
    }

    #[test]
    fn last_error_is_captured_with_the_native_result() {
        #[cfg(target_pointer_width = "64")]
        {
            let plan = unsafe {
                CallPlan::new(CallPlanSpec {
                    dll: "kernel32.dll".into(),
                    entry_point: "GetModuleHandleW".into(),
                    parameters: vec![Parameter::input(Type::Pointer, false)],
                    return_type: Some(Type::Handle),
                    return_cleanup: Cleanup::None,
                    success_rule: SuccessRule::Always,
                    capture_last_error: true,
                    calling_convention: CallingConvention::System,
                    parameter_aggregates: vec![None],
                    return_aggregate: None,
                })
            }
            .unwrap();
            let missing = wide("dynwinrt-module-that-is-not-loaded.dll");
            let result =
                unsafe { plan.invoke(&[Value::Pointer(missing.as_ptr() as *mut c_void)]) }.unwrap();
            assert!(matches!(result.return_value, Some(Value::Handle(0))));
            assert_eq!(result.last_error, Some(126));
        }
    }
}
