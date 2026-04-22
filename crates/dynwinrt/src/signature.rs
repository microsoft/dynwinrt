// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use libffi::middle::Cif;
use std::sync::Arc;
use windows::core::{GUID, HSTRING, Interface};

use crate::{call, metadata_table::{TypeHandle, TypeKind, MetadataTable}, value::WinRTValue};

/// How a parameter is passed at the ABI level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    In,
    Out,
    /// FillArray: caller allocates buffer, callee fills it.
    /// ABI expands to 3 params: (u32 capacity, T* items, u32* actual_count).
    OutFillArray,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub typ: TypeHandle,
    pub value_index: usize,
    pub kind: ParamKind,
}

impl Parameter {
    pub fn is_out(&self) -> bool {
        matches!(self.kind, ParamKind::Out | ParamKind::OutFillArray)
    }

    pub fn is_fill_array(&self) -> bool {
        self.kind == ParamKind::OutFillArray
    }
}

#[derive(Debug, Clone)]
pub struct MethodSignature {
    out_count: usize,
    parameters: Vec<Parameter>,
    return_type: TypeHandle,
    #[allow(dead_code)]
    is_opaque: bool,
    #[allow(dead_code)]
    table: Arc<MetadataTable>,
}

impl MethodSignature {
    pub fn new(table: &Arc<MetadataTable>) -> Self {
        MethodSignature {
            out_count: 0,
            parameters: Vec::new(),
            return_type: table.hresult(),
            is_opaque: false,
            table: Arc::clone(table),
        }
    }

    pub fn new_with_registry(table: &Arc<MetadataTable>) -> Self {
        Self::new(table)
    }

    pub fn add_in(mut self, typ: TypeHandle) -> Self {
        self.parameters.push(Parameter {
            kind: ParamKind::In,
            typ,
            value_index: self.parameters.len() - self.out_count,
        });
        self
    }

    pub fn add_out(mut self, typ: TypeHandle) -> Self {
        self.parameters.push(Parameter {
            kind: ParamKind::Out,
            typ,
            value_index: self.out_count,
        });
        self.out_count += 1;
        self
    }

    /// Add a FillArray out parameter: caller allocates buffer, callee fills it.
    /// ABI expands to (u32 capacity, T* items, u32* actual_count).
    pub fn add_out_fill(mut self, typ: TypeHandle) -> Self {
        self.parameters.push(Parameter {
            kind: ParamKind::OutFillArray,
            typ,
            value_index: self.out_count,
        });
        self.out_count += 1;
        self
    }

    pub fn build(self, index: usize) -> Method {
        use libffi::middle::Type;
        let mut types: Vec<Type> = Vec::with_capacity(self.parameters.len() + 1);
        types.push(Type::pointer()); // com object's this pointer
        for param in &self.parameters {
            if param.is_fill_array() {
                // FillArray: UINT32 capacity, T* items, UINT32* actual_count
                types.push(Type::u32());
                types.push(Type::pointer());
                types.push(Type::pointer());
            } else if param.typ.is_array() {
                if param.is_out() {
                    // ReceiveArray: UINT32* out_length, T** out_data
                    types.push(Type::pointer());
                    types.push(Type::pointer());
                } else {
                    // PassArray: UINT32 length, T* data
                    types.push(Type::u32());
                    types.push(Type::pointer());
                }
            } else if param.is_out() {
                types.push(Type::pointer());
            } else {
                types.push(param.typ.libffi_type());
            }
        }
        let in_count = self.parameters.len() - self.out_count;
        let has_complex_param = self.parameters.iter().any(|p| {
            p.typ.is_array() || p.is_fill_array() || matches!(p.typ.kind(), TypeKind::Struct(_))
        });

        // Check if the single in-param (if any) is a simple non-HString, non-Struct type
        let simple_in = !has_complex_param && in_count == 1 && {
            let in_param = self.parameters.iter().find(|p| !p.is_out()).unwrap();
            !matches!(in_param.typ.kind(), TypeKind::HString)
        };

        // Classify array parameters
        let array_in_count = self.parameters.iter().filter(|p| !p.is_out() && p.typ.is_array()).count();
        let fill_out_count = self.parameters.iter().filter(|p| p.is_fill_array()).count();
        let array_out_count = self.parameters.iter().filter(|p| p.is_out() && p.typ.is_array() && !p.is_fill_array()).count();
        let scalar_in_count = in_count - array_in_count;
        let scalar_out_count = self.out_count - fill_out_count - array_out_count;

        let strategy = if !has_complex_param && in_count == 0 && self.out_count == 1 {
            CallStrategy::Direct0In1Out
        } else if !has_complex_param && in_count == 0 && self.out_count == 0 {
            CallStrategy::Direct0In0Out
        } else if simple_in && self.out_count == 0 {
            CallStrategy::Direct1In0Out
        } else if simple_in && self.out_count == 1 {
            CallStrategy::Direct1In1Out
        // ReceiveArray only: fn(this, *mut u32, *mut *mut c_void) -> HRESULT
        } else if scalar_in_count == 0 && array_in_count == 0 && array_out_count == 1 && fill_out_count == 0 && scalar_out_count == 0 {
            CallStrategy::DirectReceiveArray
        // PassArray + 1 out: fn(this, u32, *const u8, out) -> HRESULT
        } else if scalar_in_count == 0 && array_in_count == 1 && array_out_count == 0 && fill_out_count == 0 && scalar_out_count == 1 {
            CallStrategy::DirectPassArray1Out
        // FillArray only: fn(this, u32, *mut u8, *mut u32) -> HRESULT
        } else if scalar_in_count == 0 && array_in_count == 0 && fill_out_count == 1 && array_out_count == 0 && scalar_out_count == 0 {
            CallStrategy::DirectFillArray
        // 1 scalar in + FillArray: fn(this, val, u32, *mut u8, *mut u32) -> HRESULT
        } else if scalar_in_count == 1 && array_in_count == 0 && fill_out_count == 1 && array_out_count == 0 && scalar_out_count == 0 {
            let in_param = self.parameters.iter().find(|p| !p.is_out() && !p.typ.is_array()).unwrap();
            if !matches!(in_param.typ.kind(), TypeKind::HString | TypeKind::Struct(_)) {
                CallStrategy::Direct1InFillArray
            } else {
                CallStrategy::Libffi(Cif::new(types.into_iter(), self.return_type.abi_type().libffi_type()))
            }
        } else {
            CallStrategy::Libffi(Cif::new(types.into_iter(), self.return_type.abi_type().libffi_type()))
        };

        Method {
            info: MethodInfo {
                index,
                parameters: self.parameters,
                out_count: self.out_count,
            },
            strategy,
        }
    }
}

#[derive(Debug)]
pub struct MethodInfo {
    pub index: usize,
    pub parameters: Vec<Parameter>,
    pub out_count: usize,
}

/// How a Method should be invoked — decided once at build time.
#[derive(Debug)]
enum CallStrategy {
    /// 0 in + 0 out: fn(this) -> HRESULT.
    Direct0In0Out,
    /// 0 in + 1 out (getter): fn(this, out) -> HRESULT.
    Direct0In1Out,
    /// 1 in + 0 out (setter, non-HString): fn(this, val) -> HRESULT.
    Direct1In0Out,
    /// 1 in + 1 out (factory/query, non-HString in): fn(this, val, out) -> HRESULT.
    Direct1In1Out,
    /// ReceiveArray: fn(this, *mut u32, *mut *mut c_void) -> HRESULT.
    DirectReceiveArray,
    /// PassArray + 1 out: fn(this, u32, *const u8, out) -> HRESULT.
    DirectPassArray1Out,
    /// FillArray only: fn(this, u32, *mut u8, *mut u32) -> HRESULT.
    DirectFillArray,
    /// 1 scalar in + FillArray: fn(this, val, u32, *mut u8, *mut u32) -> HRESULT.
    Direct1InFillArray,
    /// General case → libffi via cached Cif.
    Libffi(Cif),
}

#[derive(Debug)]
pub struct Method {
    info: MethodInfo,
    strategy: CallStrategy,
}

impl Method {
    // --- Fast getter paths: zero Vec/WinRTValue allocation ---

    /// Getter → i32 (0 in, 1 out). Writes directly to stack i32.
    pub fn call_getter_i32(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<i32> {
        let mut out: i32 = 0;
        let hr = call::call_winrt_method_1(self.info.index, obj, &mut out as *mut i32 as *mut std::ffi::c_void);
        hr.ok()?;
        Ok(out)
    }

    /// Getter → bool (0 in, 1 out). Writes directly to stack bool.
    pub fn call_getter_bool(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<bool> {
        let mut out: i32 = 0; // WinRT bool is i32 on ABI
        let hr = call::call_winrt_method_1(self.info.index, obj, &mut out as *mut i32 as *mut std::ffi::c_void);
        hr.ok()?;
        Ok(out != 0)
    }

    /// Getter → HSTRING (0 in, 1 out). Writes directly to stack HSTRING ptr.
    pub fn call_getter_hstring(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<windows_core::HSTRING> {
        // HSTRING is a pointer-sized handle on ABI. Let WinRT write it directly.
        let mut out = windows_core::HSTRING::new();
        let hr = call::call_winrt_method_1(self.info.index, obj, &mut out as *mut windows_core::HSTRING as *mut std::ffi::c_void);
        hr.ok()?;
        Ok(out)
    }

    /// Getter → COM object (0 in, 1 out). Writes directly to stack pointer.
    pub fn call_getter_object(
        &self,
        obj: *mut std::ffi::c_void,
    ) -> windows_core::Result<WinRTValue> {
        let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = call::call_winrt_method_1(self.info.index, obj, &mut out as *mut _ as *mut std::ffi::c_void);
        hr.ok()?;
        if out.is_null() {
            Ok(WinRTValue::Null)
        } else {
            Ok(WinRTValue::Object(unsafe { windows_core::IUnknown::from_raw(out) }))
        }
    }

    pub fn call_dynamic(
        &self,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> windows_core::Result<Vec<WinRTValue>> {
        match &self.strategy {
            CallStrategy::Direct0In0Out => {
                // 0 in + 0 out: fn(this) -> HRESULT
                let hr = call::call_winrt_method_0(self.info.index, obj);
                hr.ok()?;
                Ok(vec![])
            }
            CallStrategy::Direct0In1Out => {
                // 0 in + 1 out: fn(this, out) -> HRESULT
                let param = &self.info.parameters[0];
                let mut out = param.typ.default_winrt_value();
                let hr = call::call_winrt_method_1(self.info.index, obj, out.out_ptr());
                hr.ok()?;
                // COM pointer types use RawPtr(null) as buffer to avoid IUnknown::from_raw(null) UB.
                // After COM writes the pointer, convert via from_out.
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = param.typ.from_out(raw_ptr)
                        .map_err(|e| windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e)))?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::Direct1In0Out => {
                // 1 in + 0 out: fn(this, val) -> HRESULT
                let hr = call::call_1in(self.info.index, obj, &args[0]);
                hr.ok()?;
                Ok(vec![])
            }
            CallStrategy::Direct1In1Out => {
                // 1 in + 1 out: fn(this, val, out) -> HRESULT
                let out_param = self.info.parameters.iter().find(|p| p.is_out()).unwrap();
                let mut out = out_param.typ.default_winrt_value();
                let hr = call::call_1in_1out(self.info.index, obj, &args[0], out.out_ptr());
                hr.ok()?;
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = out_param.typ.from_out(raw_ptr)
                        .map_err(|e| windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e)))?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::DirectReceiveArray => {
                // fn(this, *mut u32, *mut *mut c_void) -> HRESULT
                let param = &self.info.parameters[0];
                let elem_type = param.typ.array_element_type();
                let mut length: u32 = 0;
                let mut data_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void, *mut u32, *mut *mut std::ffi::c_void,
                    ) -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, &mut length, &mut data_ptr)
                };
                if hr.is_err() {
                    // Callee may have allocated a buffer before returning failure.
                    // Wrap in ArrayData to release elements + CoTaskMemFree.
                    if !data_ptr.is_null() {
                        let _ = crate::array::ArrayData::from_cotaskmem(
                            elem_type.clone(), data_ptr, length as usize,
                        );
                    }
                    hr.ok()?;
                }
                let array = if data_ptr.is_null() || length == 0 {
                    if !data_ptr.is_null() {
                        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(data_ptr)); }
                    }
                    crate::array::ArrayData::empty(elem_type)
                } else {
                    crate::array::ArrayData::from_cotaskmem(elem_type, data_ptr, length as usize)
                };
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::DirectPassArray1Out => {
                // fn(this, u32, *const u8, out) -> HRESULT
                let in_param = self.info.parameters.iter().find(|p| !p.is_out()).unwrap();
                let out_param = self.info.parameters.iter().find(|p| p.is_out()).unwrap();
                let array_data = args[in_param.value_index].as_array().unwrap();
                let buffer = array_data.serialize_for_abi();
                let mut out = out_param.typ.default_winrt_value();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void, u32, *const u8, *mut std::ffi::c_void,
                    ) -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, array_data.len() as u32, buffer.as_ptr(), out.out_ptr())
                };
                hr.ok()?;
                if let WinRTValue::RawPtr(raw_ptr) = out {
                    out = out_param.typ.from_out(raw_ptr)
                        .map_err(|e| windows_core::Error::new(windows_core::HRESULT(-1), &format!("{:?}", e)))?;
                }
                out.sanitize_null_object();
                Ok(vec![out])
            }
            CallStrategy::DirectFillArray => {
                // fn(this, u32, *mut u8, *mut u32) -> HRESULT
                // FillArray: caller provides buffer of known capacity, callee fills it.
                let param = &self.info.parameters[0];
                let elem_type = param.typ.array_element_type();
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);

                assert!(
                    !args.is_empty() && args[param.value_index].as_array().is_some(),
                    "DirectFillArray requires a pre-allocated array argument with the desired capacity. \
                     Pass an ArrayData with the expected number of elements."
                );
                let array_data = args[param.value_index].as_array().unwrap();
                let capacity = array_data.len() as u32;
                let total_bytes = capacity as usize * elem_type.element_size();
                let buffer_ptr = unsafe {
                    windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut u8
                };
                assert!(!buffer_ptr.is_null(), "CoTaskMemAlloc failed for FillArray");
                unsafe { std::ptr::write_bytes(buffer_ptr, 0, total_bytes) };
                // Use sentinel to detect if callee actually wrote actual_count.
                // WinRT FillArray contract requires callee to write the count,
                // but some implementations don't — sentinel lets us distinguish
                // "callee wrote 0" from "callee didn't write at all".
                let mut actual_count: u32 = 0;
                let hr: windows_core::HRESULT = unsafe {
                    let method: unsafe extern "system" fn(
                        *mut std::ffi::c_void, u32, *mut u8, *mut u32,
                    ) -> windows_core::HRESULT = std::mem::transmute(fptr);
                    method(obj, capacity, buffer_ptr, &mut actual_count)
                };
                if hr.is_err() {
                    // Callee may have written elements before failing.
                    // Buffer was zero-initialized, so null slots are safe to release.
                    // Use capacity as cleanup length — ArrayData::Drop skips null elements.
                    let _ = crate::array::ArrayData::from_cotaskmem(
                        elem_type.clone(), buffer_ptr as _, capacity as usize,
                    );
                    hr.ok()?;
                }
                // If callee didn't set actual_count (left as 0) and capacity > 0, assume full buffer.
                if actual_count == 0 && capacity > 0 {
                    actual_count = capacity;
                }
                // Clamp to capacity to prevent OOB if callee misbehaves
                actual_count = std::cmp::min(actual_count, capacity);
                let array = crate::array::ArrayData::from_cotaskmem(
                    elem_type, buffer_ptr as _, actual_count as usize,
                );
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::Direct1InFillArray => {
                // fn(this, val, u32, *mut u8, *mut u32) -> HRESULT
                let in_param = self.info.parameters.iter().find(|p| !p.is_out()).unwrap();
                let fill_param = self.info.parameters.iter().find(|p| p.is_fill_array()).unwrap();
                let array_data = args[fill_param.value_index].as_array().unwrap();
                let elem_type = fill_param.typ.array_element_type();
                let capacity = array_data.len() as u32;
                let total_bytes = capacity as usize * elem_type.element_size();
                let buffer_ptr = unsafe {
                    windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut u8
                };
                assert!(!buffer_ptr.is_null(), "CoTaskMemAlloc failed for FillArray");
                unsafe { std::ptr::write_bytes(buffer_ptr, 0, total_bytes) };
                // Use sentinel to detect if callee actually wrote actual_count.
                let mut actual_count: u32 = 0;
                let fptr = call::get_vtable_function_ptr(obj, self.info.index);
                let hr = call::call_fill_array_1in(
                    fptr, obj, &args[in_param.value_index],
                    capacity, buffer_ptr, &mut actual_count,
                );
                if hr.is_err() {
                    // Buffer was zero-initialized; use capacity for cleanup.
                    let _ = crate::array::ArrayData::from_cotaskmem(
                        elem_type.clone(), buffer_ptr as _, capacity as usize,
                    );
                    hr.ok()?;
                }
                // If callee didn't set actual_count (left as 0) and capacity > 0, assume full buffer.
                if actual_count == 0 && capacity > 0 {
                    actual_count = capacity;
                }
                // Clamp to capacity to prevent OOB if callee misbehaves
                actual_count = std::cmp::min(actual_count, capacity);
                let array = crate::array::ArrayData::from_cotaskmem(
                    elem_type, buffer_ptr as _, actual_count as usize,
                );
                Ok(vec![WinRTValue::Array(array)])
            }
            CallStrategy::Libffi(cif) => {
                call::call_winrt_method_dynamic(
                    self.info.index,
                    obj,
                    &self.info.parameters,
                    args,
                    self.info.out_count,
                    cif,
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct InterfaceSignature {
    pub name: String,
    pub iid: windows_core::GUID,
    pub methods: Vec<Method>,
    #[allow(dead_code)]
    table: Arc<MetadataTable>,
}

impl InterfaceSignature {
    pub fn define_interface(name: String, iid: windows_core::GUID, table: &Arc<MetadataTable>) -> Self {
        InterfaceSignature {
            name,
            iid,
            methods: Vec::new(),
            table: Arc::clone(table),
        }
    }

    pub fn define_from_iunknown(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut t = InterfaceSignature::define_interface(name.to_owned(), iid, table);
        t.add_method(MethodSignature::new(table)) // 0 QueryInterface
            .add_method(MethodSignature::new(table)) // 1 AddRef
            .add_method(MethodSignature::new(table)); // 2 Release
        t
    }

    pub fn define_from_iinspectable(name: &str, iid: GUID, table: &Arc<MetadataTable>) -> Self {
        let mut t = Self::define_from_iunknown(name, iid, table);
        t.add_method(MethodSignature::new(table)) // 3 GetIids
            .add_method(MethodSignature::new(table).add_out(table.hstring())) // 4 GetRuntimeClassName
            .add_method(MethodSignature::new(table)); // 5 GetTrustLevel
        t
    }

    pub fn add_method(&mut self, signature: MethodSignature) -> &mut Self {
        let method = signature.build(self.methods.len());
        self.methods.push(method);
        self
    }
}

#[allow(dead_code)]
pub struct RuntimeClassSignature {
    name: HSTRING,
    static_interfaces: Vec<InterfaceSignature>,
    instance_interfaces: Vec<InterfaceSignature>,
}
