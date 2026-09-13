// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Caller-owned aggregate memory and independently owned native result fields.

use std::{
    collections::BTreeSet,
    ffi::c_void,
    sync::{Arc, Mutex, MutexGuard},
};

use super::{
    Cleanup, MAX_NATIVE_AGGREGATE_SIZE, ResultPolicy, Type, Value, invalid_argument, result_cleanup,
};
use crate::{
    abi::AbiValue,
    result::{Error, Result},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateResultField {
    pub name: String,
    pub offset: usize,
    pub typ: Type,
    pub cleanup: Cleanup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeAggregatePointerLayout {
    identity: String,
    size: usize,
    alignment: usize,
    fields: Vec<AggregateResultField>,
}

impl NativeAggregatePointerLayout {
    pub fn new(
        identity: String,
        size: usize,
        alignment: usize,
        fields: Vec<AggregateResultField>,
    ) -> Result<Arc<Self>> {
        if identity.is_empty()
            || size == 0
            || size > MAX_NATIVE_AGGREGATE_SIZE
            || alignment == 0
            || alignment > 8
            || !alignment.is_power_of_two()
            || size % alignment != 0
            || fields.len() > dynwinrt_win32_contracts::MAX_PARAMETERS
        {
            return Err(invalid_argument("Invalid native aggregate pointer layout"));
        }
        let mut names = BTreeSet::new();
        let mut ranges = Vec::new();
        for field in &fields {
            let length = field_size(field.typ);
            let end = field
                .offset
                .checked_add(length)
                .filter(|end| *end <= size)
                .ok_or_else(|| invalid_argument("Aggregate result field exceeds caller storage"))?;
            if field.name.is_empty()
                || !names.insert(field.name.clone())
                || ranges
                    .iter()
                    .any(|(start, prior_end)| field.offset < *prior_end && *start < end)
                || (field.cleanup.owns_resource()
                    && !matches!(field.typ, Type::Handle | Type::Pointer))
            {
                return Err(invalid_argument(
                    "Invalid or overlapping aggregate result field",
                ));
            }
            ranges.push((field.offset, end));
        }
        Ok(Arc::new(Self {
            identity,
            size,
            alignment,
            fields,
        }))
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn alignment(&self) -> usize {
        self.alignment
    }
    pub fn fields(&self) -> &[AggregateResultField] {
        &self.fields
    }
}

#[derive(Debug)]
pub struct NativeAggregateBuffer {
    layout: Arc<NativeAggregatePointerLayout>,
    pointer: usize,
    state: Mutex<AggregateBufferState>,
}

pub struct NativeAggregateLease<'a> {
    _state: MutexGuard<'a, AggregateBufferState>,
}

#[derive(Debug)]
pub(super) struct AggregateBufferState {
    pub(super) words: Vec<u64>,
    pub(super) results: Vec<Option<Value>>,
    pub(super) raw_cleanup: Vec<Cleanup>,
    pub(super) policies: Vec<ResultPolicy>,
    pub(super) succeeded: Option<bool>,
    pub(super) native_recorded: bool,
}

impl NativeAggregateBuffer {
    pub fn new(
        layout: Arc<NativeAggregatePointerLayout>,
        bytes: Option<&[u8]>,
    ) -> Result<Arc<Self>> {
        let mut words = super::try_zeroed_words(layout.size)?;
        if let Some(bytes) = bytes {
            if bytes.len() != layout.size {
                return Err(invalid_argument(
                    "Native aggregate initialization has the wrong size",
                ));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    words.as_mut_ptr().cast::<u8>(),
                    bytes.len(),
                );
            }
        }
        let pointer = words.as_mut_ptr() as usize;
        let count = layout.fields.len();
        Ok(Arc::new(Self {
            layout,
            pointer,
            state: Mutex::new(AggregateBufferState {
                words,
                results: vec![None; count],
                raw_cleanup: vec![Cleanup::None; count],
                policies: vec![ResultPolicy::Undefined {}; count],
                succeeded: None,
                native_recorded: false,
            }),
        }))
    }

    pub fn layout(&self) -> &Arc<NativeAggregatePointerLayout> {
        &self.layout
    }
    pub fn pointer(&self) -> *mut c_void {
        self.pointer as *mut c_void
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, AggregateBufferState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn lease(&self) -> NativeAggregateLease<'_> {
        NativeAggregateLease {
            _state: self.lock(),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        let state = self.lock();
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.layout.size)
            .map_err(|_| super::out_of_memory("native aggregate bytes"))?;
        let raw = unsafe {
            std::slice::from_raw_parts(state.words.as_ptr().cast::<u8>(), self.layout.size)
        };
        bytes.extend_from_slice(raw);
        Ok(bytes)
    }

    pub fn read<const N: usize>(&self, offset: usize) -> Result<[u8; N]> {
        self.check_range(offset, N)?;
        let state = self.lock();
        let mut bytes = [0; N];
        unsafe {
            std::ptr::copy_nonoverlapping(
                state.words.as_ptr().cast::<u8>().add(offset),
                bytes.as_mut_ptr(),
                N,
            );
        }
        Ok(bytes)
    }

    pub fn write(&self, offset: usize, bytes: &[u8]) -> Result<()> {
        self.check_range(offset, bytes.len())?;
        let mut state = self.lock();
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                state.words.as_mut_ptr().cast::<u8>().add(offset),
                bytes.len(),
            );
        }
        Ok(())
    }

    fn check_range(&self, offset: usize, size: usize) -> Result<()> {
        if offset
            .checked_add(size)
            .is_none_or(|end| end > self.layout.size)
        {
            return Err(invalid_argument(
                "Native aggregate field exceeds caller storage",
            ));
        }
        Ok(())
    }

    pub fn prepare(&self) -> Result<()> {
        self.lock().prepare(&self.layout)
    }

    pub fn require_success(&self) -> Result<()> {
        match self.lock().succeeded {
            Some(true) => Ok(()),
            Some(false) => Err(invalid_argument(
                "native aggregate outputs are unavailable because the native call failed",
            )),
            None => Err(invalid_argument(
                "native aggregate outputs are unavailable before a successful native call",
            )),
        }
    }

    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.layout
            .fields
            .iter()
            .position(|field| field.name == name)
    }

    pub fn field_value(&self, index: usize) -> Result<Value> {
        let state = self.lock();
        let value = state
            .results
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| unavailable_field(state.succeeded))?;
        if matches!(value, Value::Unavailable | Value::Discarded) {
            return Err(unavailable_field(state.succeeded));
        }
        Ok(value.clone())
    }

    pub fn take_field(&self, index: usize) -> Result<Value> {
        let mut state = self.lock();
        let field = self
            .layout
            .fields
            .get(index)
            .ok_or_else(|| invalid_argument("Unknown native aggregate result field"))?;
        let value = state.results[index]
            .as_ref()
            .ok_or_else(|| unavailable_field(state.succeeded))?;
        if matches!(value, Value::Unavailable | Value::Discarded) {
            return Err(unavailable_field(state.succeeded));
        }
        let result = state.results[index]
            .replace(Value::Handle(0))
            .expect("checked result");
        state.zero(field);
        Ok(result)
    }

    /// Compatibility for manually described calls. Generated contracts register
    /// results inside CallPlan and cannot have that ownership erased by JS.
    pub fn mark_legacy(&self, succeeded: bool) -> Result<()> {
        let mut state = self.lock();
        if state.native_recorded || state.succeeded == Some(true) {
            return if state.succeeded == Some(succeeded) {
                Ok(())
            } else {
                Err(invalid_argument(
                    "Cannot overwrite native aggregate result ownership",
                ))
            };
        }
        state.succeeded = Some(succeeded);
        for (index, field) in self.layout.fields.iter().enumerate() {
            state.policies[index] = if succeeded {
                let ownership = if field.cleanup.owns_resource() {
                    super::ResultOwnership::Owned {
                        cleanup: field.cleanup.into(),
                    }
                } else if matches!(
                    field.typ,
                    Type::Handle | Type::Pointer | Type::FunctionPointer
                ) {
                    super::ResultOwnership::Borrowed {}
                } else {
                    super::ResultOwnership::Value {}
                };
                ResultPolicy::delivered(ownership)
            } else {
                ResultPolicy::Undefined {}
            };
            state.raw_cleanup[index] = result_cleanup(state.policies[index]);
        }
        for (index, field) in self.layout.fields.iter().enumerate() {
            if !succeeded {
                state.results[index] = Some(Value::Unavailable);
                continue;
            }
            let raw = state.read_abi(field);
            let value = if field.cleanup.owns_resource() {
                let AbiValue::Pointer(pointer) = raw else {
                    unreachable!("validated owned field");
                };
                if pointer.is_null() {
                    Value::Handle(0)
                } else {
                    Value::Resource(unsafe {
                        super::OwnedResource::adopt(pointer as usize, field.cleanup)
                    }?)
                }
            } else {
                super::decode_plain_value(field.typ, raw)?
            };
            state.raw_cleanup[index] = Cleanup::None;
            state.results[index] = Some(value);
        }
        Ok(())
    }
}

impl AggregateBufferState {
    pub(super) fn prepare(&mut self, layout: &NativeAggregatePointerLayout) -> Result<()> {
        let mut first_error = self.cleanup_raw(layout).err();
        for (index, field) in layout.fields.iter().enumerate() {
            if matches!(
                self.policies[index],
                ResultPolicy::Defined {
                    ownership: super::ResultOwnership::Owned { .. },
                    ..
                }
            ) {
                if let Some(Value::Resource(resource)) = &self.results[index] {
                    if let Err(error) = resource.close() {
                        first_error.get_or_insert(Error::WindowsError(error));
                        continue;
                    }
                }
            }
            if self.raw_cleanup[index].owns_resource() {
                continue;
            }
            self.results[index] = None;
            self.zero(field);
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.policies.fill(ResultPolicy::Undefined {});
        self.succeeded = Some(false);
        self.native_recorded = false;
        Ok(())
    }

    pub(super) fn read_abi(&self, field: &AggregateResultField) -> AbiValue {
        let pointer = unsafe { self.words.as_ptr().cast::<u8>().add(field.offset) };
        unsafe {
            match field.typ {
                Type::Bool32 | Type::I32 => AbiValue::I32(pointer.cast::<i32>().read_unaligned()),
                Type::I8 => AbiValue::I8(pointer.cast::<i8>().read_unaligned()),
                Type::U8 => AbiValue::U8(pointer.read()),
                Type::I16 => AbiValue::I16(pointer.cast::<i16>().read_unaligned()),
                Type::U16 => AbiValue::U16(pointer.cast::<u16>().read_unaligned()),
                Type::U32 => AbiValue::U32(pointer.cast::<u32>().read_unaligned()),
                Type::I64 => AbiValue::I64(pointer.cast::<i64>().read_unaligned()),
                Type::U64 => AbiValue::U64(pointer.cast::<u64>().read_unaligned()),
                Type::F32 => AbiValue::F32(pointer.cast::<f32>().read_unaligned()),
                Type::F64 => AbiValue::F64(pointer.cast::<f64>().read_unaligned()),
                Type::Handle | Type::Pointer | Type::FunctionPointer => {
                    AbiValue::Pointer(pointer.cast::<usize>().read_unaligned() as *mut c_void)
                }
            }
        }
    }

    fn zero(&mut self, field: &AggregateResultField) {
        unsafe {
            std::ptr::write_bytes(
                self.words.as_mut_ptr().cast::<u8>().add(field.offset),
                0,
                field_size(field.typ),
            );
        }
    }

    fn cleanup_raw(&mut self, layout: &NativeAggregatePointerLayout) -> Result<()> {
        let mut first_error = None;
        for (index, field) in layout.fields.iter().enumerate() {
            let cleanup = self.raw_cleanup[index];
            if cleanup.owns_resource() {
                let AbiValue::Pointer(pointer) = self.read_abi(field) else {
                    unreachable!("validated owned field");
                };
                if let Err(error) = unsafe { cleanup.run(pointer as usize) } {
                    first_error.get_or_insert(Error::WindowsError(error));
                } else {
                    self.raw_cleanup[index] = Cleanup::None;
                    self.zero(field);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for NativeAggregateBuffer {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = state.cleanup_raw(&self.layout) {
            eprintln!(
                "[dynwinrt] native aggregate field cleanup failed: {}",
                error.message()
            );
        }
    }
}

pub(super) fn field_size(typ: Type) -> usize {
    match typ {
        Type::I8 | Type::U8 => 1,
        Type::I16 | Type::U16 => 2,
        Type::Bool32 | Type::I32 | Type::U32 | Type::F32 => 4,
        Type::I64 | Type::U64 | Type::F64 => 8,
        Type::Pointer | Type::FunctionPointer | Type::Handle => size_of::<usize>(),
    }
}

fn unavailable_field(succeeded: Option<bool>) -> Error {
    invalid_argument(match succeeded {
        None => "native aggregate outputs are unavailable before a successful native call",
        Some(false) => "native aggregate outputs are unavailable because the native call failed",
        Some(true) => {
            "native aggregate outputs are unavailable because the native field is not deliverable"
        }
    })
}
