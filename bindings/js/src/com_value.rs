// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! COM-only carrier state. Payload, pointer provenance and input/apartment
//! bookkeeping are independent so conversion never discards a cleanup obligation.

use crate::{
  com, com_input, com_raw, js_storage::CallStorage, winui_dispatcher_loop_exited, DynWinRTValue,
};
use std::mem;
use windows::core::{IUnknown, Interface, GUID};

#[derive(Default)]
pub(crate) struct ComValueState {
  pointer: NativePointerState,
  payload: ComPayload,
  apartment: Option<ComApartmentBinding>,
  inputs: Option<com_input::InputBindings>,
}

#[derive(Default)]
struct NativePointerState {
  owner: Option<NativePointerOwner>,
  provenance: PointerProvenance,
}

#[derive(Default)]
enum ComPayload {
  #[default]
  None,
  NativeStruct(dynwinrt::com::NativeStructValue),
  Buffer(dynwinrt::com::ComBufferValue),
  Automation(AutomationValue),
}

#[allow(dead_code)]
pub(crate) enum NativePointerOwner {
  CoTaskMem(*mut std::ffi::c_void),
  Guid(*mut GUID),
  RawMemory(std::sync::Arc<crate::com_raw::RawAllocation>),
  RawCom(std::sync::Arc<crate::com_raw::RawComReference>),
}

enum AutomationValueKind {
  Bstr(dynwinrt::com::BstrValue),
  NativeUnion(dynwinrt::com::NativeUnionValue),
  Variant(dynwinrt::com::VariantValue),
  SafeArray(dynwinrt::com::SafeArrayValue),
  PropVariant(dynwinrt::com::PropVariantValue),
  DispatchParams(dynwinrt::com::DispatchParamsValue),
  ExcepInfo(dynwinrt::com::ExcepInfoValue),
  StatStg(dynwinrt::com::StatStgValue),
  FormatEtc(dynwinrt::com::FormatEtcValue),
  StgMedium(dynwinrt::com::StgMediumValue),
  AudioFormat(dynwinrt::com::AudioFormatValue),
}

pub(crate) struct AutomationValue {
  owner_thread: std::thread::ThreadId,
  value: Option<AutomationValueKind>,
}

impl AutomationValue {
  pub(crate) fn new(value: dynwinrt::com::Value) -> Self {
    let value = match value {
      dynwinrt::com::Value::Bstr(value) => AutomationValueKind::Bstr(value),
      dynwinrt::com::Value::NativeUnion(value) => AutomationValueKind::NativeUnion(value),
      dynwinrt::com::Value::Variant(value) => AutomationValueKind::Variant(value),
      dynwinrt::com::Value::SafeArray(value) => AutomationValueKind::SafeArray(value),
      dynwinrt::com::Value::PropVariant(value) => AutomationValueKind::PropVariant(value),
      dynwinrt::com::Value::DispatchParams(value) => AutomationValueKind::DispatchParams(value),
      dynwinrt::com::Value::ExcepInfo(value) => AutomationValueKind::ExcepInfo(value),
      dynwinrt::com::Value::StatStg(value) => AutomationValueKind::StatStg(value),
      dynwinrt::com::Value::FormatEtc(value) => AutomationValueKind::FormatEtc(value),
      dynwinrt::com::Value::StgMedium(value) => AutomationValueKind::StgMedium(value),
      dynwinrt::com::Value::AudioFormat(value) => AutomationValueKind::AudioFormat(value),
      _ => unreachable!("AutomationValue requires an automation COM value"),
    };
    Self {
      owner_thread: std::thread::current().id(),
      value: Some(value),
    }
  }

  pub(crate) fn ensure_owner_thread(&self) -> napi::Result<()> {
    if matches!(self.value, Some(AutomationValueKind::Bstr(_)))
      || std::thread::current().id() == self.owner_thread
    {
      Ok(())
    } else {
      Err(napi::Error::from_reason(
        "Apartment-bound COM Automation value used from a different thread",
      ))
    }
  }

  pub(crate) fn to_com_value(&self) -> napi::Result<dynwinrt::com::Value> {
    self.ensure_owner_thread()?;
    let value = self
      .value
      .as_ref()
      .ok_or_else(|| napi::Error::from_reason("COM Automation value has been consumed"))?;
    Ok(match value {
      AutomationValueKind::Bstr(value) => dynwinrt::com::Value::Bstr(value.clone()),
      AutomationValueKind::NativeUnion(value) => dynwinrt::com::Value::NativeUnion(value.clone()),
      AutomationValueKind::Variant(value) => dynwinrt::com::Value::Variant(value.clone()),
      AutomationValueKind::SafeArray(value) => dynwinrt::com::Value::SafeArray(value.clone()),
      AutomationValueKind::PropVariant(value) => dynwinrt::com::Value::PropVariant(value.clone()),
      AutomationValueKind::DispatchParams(value) => {
        dynwinrt::com::Value::DispatchParams(value.clone())
      }
      AutomationValueKind::ExcepInfo(value) => dynwinrt::com::Value::ExcepInfo(value.clone()),
      AutomationValueKind::StatStg(value) => dynwinrt::com::Value::StatStg(value.clone()),
      AutomationValueKind::FormatEtc(value) => dynwinrt::com::Value::FormatEtc(value.clone()),
      AutomationValueKind::StgMedium(value) => dynwinrt::com::Value::StgMedium(value.clone()),
      AutomationValueKind::AudioFormat(value) => dynwinrt::com::Value::AudioFormat(value.clone()),
    })
  }

  pub(crate) fn take_variant(&mut self) -> napi::Result<dynwinrt::com::VariantValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::Variant(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM VARIANT"))
      }
    }
  }

  pub(crate) fn take_safe_array(&mut self) -> napi::Result<dynwinrt::com::SafeArrayValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::SafeArray(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM SAFEARRAY"))
      }
    }
  }

  pub(crate) fn take_prop_variant(&mut self) -> napi::Result<dynwinrt::com::PropVariantValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::PropVariant(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM PROPVARIANT"))
      }
    }
  }

  pub(crate) fn take_excep_info(&mut self) -> napi::Result<dynwinrt::com::ExcepInfoValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::ExcepInfo(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM EXCEPINFO"))
      }
    }
  }

  pub(crate) fn take_stat_stg(&mut self) -> napi::Result<dynwinrt::com::StatStgValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::StatStg(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM STATSTG"))
      }
    }
  }

  pub(crate) fn take_format_etc(&mut self) -> napi::Result<dynwinrt::com::FormatEtcValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::FormatEtc(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM FORMATETC"))
      }
    }
  }

  pub(crate) fn take_stg_medium(&mut self) -> napi::Result<dynwinrt::com::StgMediumValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::StgMedium(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not COM STGMEDIUM"))
      }
    }
  }

  pub(crate) fn take_audio_format(&mut self) -> napi::Result<dynwinrt::com::AudioFormatValue> {
    self.ensure_owner_thread()?;
    match self.value.take() {
      Some(AutomationValueKind::AudioFormat(value)) => Ok(value),
      value => {
        self.value = value;
        Err(napi::Error::from_reason("Value is not a COM WAVEFORMATEX"))
      }
    }
  }

  pub(crate) fn leak_for_shutdown(&mut self) {
    if let Some(value) = self.value.take() {
      std::mem::forget(value);
    }
  }
}

impl Drop for AutomationValue {
  fn drop(&mut self) {
    if !matches!(self.value, Some(AutomationValueKind::Bstr(_)))
      && (std::thread::current().id() != self.owner_thread || crate::winui_dispatcher_loop_exited())
    {
      self.leak_for_shutdown();
    }
  }
}

// Safety: access is rejected off the creating apartment. Wrong-thread and
// post-WinUI destruction drops leak the value instead of invoking native
// cleanup on an invalid apartment.
unsafe impl Send for AutomationValue {}
unsafe impl Sync for AutomationValue {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerProvenance {
  #[default]
  None,
  Borrowed,
  DetachedCom,
  UnclassifiedOutput,
  ComOutput,
  CoTaskMemOutput,
  BstrOutput,
  OwnedHandleOutput(dynwinrt::com::OwnedHandleCleanup),
}

pub(crate) struct NativeInvocationLeases {
  _raw_memory: Vec<crate::com_raw::RawInvocationLease>,
  _raw_com: Vec<crate::com_raw::RawComInvocationLease>,
}

impl NativePointerOwner {
  pub(crate) fn validate(&self) -> napi::Result<()> {
    match self {
      Self::RawMemory(allocation) => allocation.validate_live(),
      Self::RawCom(reference) => reference.validate_live(),
      Self::CoTaskMem(_) | Self::Guid(_) => Ok(()),
    }
  }
}

impl Drop for NativePointerOwner {
  fn drop(&mut self) {
    match self {
      Self::CoTaskMem(ptr) => {
        if !ptr.is_null() {
          unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(*ptr)) };
          *ptr = std::ptr::null_mut();
        }
      }
      Self::Guid(ptr) => {
        if !ptr.is_null() {
          drop(unsafe { Box::from_raw(*ptr) });
          *ptr = std::ptr::null_mut();
        }
      }
      _ => {}
    }
  }
}

#[derive(Clone)]
pub(crate) struct ComApartmentBinding {
  owner_thread: std::thread::ThreadId,
  context: ComContextSlot,
  identity_error: Option<String>,
}

type ComContextSlot = std::rc::Rc<std::cell::RefCell<Option<dynwinrt::com::borrowed::Identity>>>;

thread_local! {
  // Empty slots are pinned by their managed carriers' owned interface refs.
  // Populated slots additionally own the core canonical-identity sidecar.
  // Sharing slots at carrier creation keeps pre-existing independent aliases
  // from forgetting provenance or poison when the first copier is released.
  static COM_CONTEXT_SLOTS: std::cell::RefCell<std::collections::HashMap<usize,
    std::rc::Weak<std::cell::RefCell<Option<dynwinrt::com::borrowed::Identity>>>>> =
    std::cell::RefCell::new(std::collections::HashMap::new());
}

impl ComApartmentBinding {
  pub(crate) fn is_owner(&self) -> bool {
    self.owner_thread == std::thread::current().id()
  }

  pub(crate) fn shares_identity_slot(&self, other: &Self) -> bool {
    std::rc::Rc::ptr_eq(&self.context, &other.context)
  }

  pub(crate) fn for_object(object: &IUnknown) -> Self {
    let owner_thread = std::thread::current().id();
    let mut pointer = std::ptr::null_mut();
    let hr = unsafe { object.query(&IUnknown::IID, &mut pointer) };
    if hr.is_err() || pointer.is_null() {
      return Self {
        owner_thread,
        context: Default::default(),
        identity_error: Some(format!(
          "COM canonical identity is unavailable (HRESULT 0x{:08X}, null={})",
          hr.0 as u32,
          pointer.is_null()
        )),
      };
    }
    let canonical = unsafe { IUnknown::from_raw(pointer) };
    let key = canonical.as_raw().addr();
    let context = COM_CONTEXT_SLOTS.with(|slots| {
      let mut slots = slots.borrow_mut();
      if let Some(slot) = slots.get(&key).and_then(std::rc::Weak::upgrade) {
        return slot;
      }
      slots.retain(|_, slot| slot.strong_count() != 0);
      let slot: ComContextSlot = Default::default();
      slots.insert(key, std::rc::Rc::downgrade(&slot));
      slot
    });
    Self {
      owner_thread,
      context,
      identity_error: None,
    }
  }

  pub(crate) fn ensure_identity(&self) -> napi::Result<()> {
    if let Some(error) = &self.identity_error {
      return Err(napi::Error::from_reason(error.clone()));
    }
    Ok(())
  }

  pub(crate) fn refresh_context(&self, object: &IUnknown) -> dynwinrt::Result<()> {
    if self.owner_thread != std::thread::current().id() {
      return Err(dynwinrt::Error::WindowsError(windows::core::Error::new(
        windows::core::HRESULT(0x80070057u32 as i32),
        "Classic COM object used from a different apartment thread",
      )));
    }
    if self.context.borrow().is_none() {
      let context = dynwinrt::com::borrowed::Identity::lookup(object)?;
      *self.context.borrow_mut() = context;
    }
    Ok(())
  }

  pub(crate) fn ensure_idle(&self) -> napi::Result<()> {
    if self.owner_thread != std::thread::current().id() {
      return Err(napi::Error::from_reason(
        "Classic COM object used from a different apartment thread",
      ));
    }
    self.ensure_identity()?;
    if let Some(context) = self.context.borrow().as_ref() {
      context
        .ensure_idle()
        .map_err(|error| napi::Error::from_reason(error.message()))?;
    }
    Ok(())
  }
}

impl DynWinRTValue {
  pub(crate) fn new(value: dynwinrt::WinRTValue) -> Self {
    let inputs = com_input::winrt_has_interfaces(&value).then(com_input::InputBindings::deferred);
    Self {
      winrt: value,
      storage: None,
      com: ComValueState {
        inputs,
        ..Default::default()
      },
    }
  }

  pub(crate) fn winrt(&self) -> &dynwinrt::WinRTValue {
    &self.winrt
  }

  pub(crate) fn winrt_mut(&mut self) -> &mut dynwinrt::WinRTValue {
    &mut self.winrt
  }

  pub(crate) fn input_bindings(&self) -> Option<&com_input::InputBindings> {
    self.com.inputs.as_ref()
  }

  pub(crate) fn set_input_bindings(&mut self, inputs: Option<com_input::InputBindings>) {
    self.com.inputs = inputs;
  }

  pub(crate) fn com_binding(&self) -> Option<&ComApartmentBinding> {
    self.com.apartment.as_ref()
  }

  pub(crate) fn take_com_binding(&mut self) -> Option<ComApartmentBinding> {
    self.com.apartment.take()
  }

  pub(crate) fn set_com_binding(&mut self, binding: Option<ComApartmentBinding>) {
    self.com.apartment = binding;
  }

  pub(crate) fn copy_com_bookkeeping_from(&mut self, source: &Self) {
    self.com.apartment = source.com.apartment.clone();
    self.com.inputs = source
      .com
      .inputs
      .as_ref()
      .map(com_input::InputBindings::copy_bookkeeping);
  }

  pub(crate) fn pointer_owner(&self) -> Option<&NativePointerOwner> {
    self.com.pointer.owner.as_ref()
  }

  pub(crate) fn call_storage(&self) -> Option<&CallStorage> {
    self.storage.as_ref()
  }

  pub(crate) fn has_pointer_owner(&self) -> bool {
    self.storage.is_some() || self.com.pointer.owner.is_some()
  }

  pub(crate) fn pointer_provenance(&self) -> PointerProvenance {
    self.com.pointer.provenance
  }

  pub(crate) fn clear_pointer_provenance(&mut self) {
    self.com.pointer.provenance = PointerProvenance::None;
  }

  pub(crate) fn validate_call_storage(&self) -> napi::Result<()> {
    if let Some(storage) = self.call_storage() {
      storage.validate()?;
    }
    Ok(())
  }

  pub(crate) fn validate_pointer_owner(&self) -> napi::Result<()> {
    self.validate_call_storage()?;
    if let Some(owner) = self.pointer_owner() {
      owner.validate()?;
    }
    Ok(())
  }

  pub(crate) fn native_struct(&self) -> Option<&dynwinrt::com::NativeStructValue> {
    match &self.com.payload {
      ComPayload::NativeStruct(value) => Some(value),
      _ => None,
    }
  }

  pub(crate) fn com_buffer(&self) -> Option<&dynwinrt::com::ComBufferValue> {
    match &self.com.payload {
      ComPayload::Buffer(value) => Some(value),
      _ => None,
    }
  }

  pub(crate) fn take_com_buffer(&mut self) -> Option<dynwinrt::com::ComBufferValue> {
    match mem::take(&mut self.com.payload) {
      ComPayload::Buffer(value) => Some(value),
      other => {
        self.com.payload = other;
        None
      }
    }
  }

  pub(crate) fn automation(&self) -> Option<&AutomationValue> {
    match &self.com.payload {
      ComPayload::Automation(value) => Some(value),
      _ => None,
    }
  }

  pub(crate) fn automation_mut(&mut self) -> Option<&mut AutomationValue> {
    match &mut self.com.payload {
      ComPayload::Automation(value) => Some(value),
      _ => None,
    }
  }

  pub(crate) fn clear_automation(&mut self) {
    if matches!(self.com.payload, ComPayload::Automation(_)) {
      self.com.payload = ComPayload::None;
    }
  }

  pub(crate) fn has_com_payload(&self) -> bool {
    !matches!(self.com.payload, ComPayload::None)
  }

  pub(crate) fn release_value(&mut self) -> napi::Result<()> {
    if let Some(inputs) = &self.com.inputs {
      if inputs.is_apartment_bound() {
        inputs.ensure_owner()?;
      }
    }
    if self.com.apartment.is_some() {
      self.ensure_com_apartment()?;
    }
    self.release_native_pointer_output()?;
    self.winrt = dynwinrt::WinRTValue::Null;
    self.storage = None;
    self.com.pointer.owner = None;
    self.com.pointer.provenance = PointerProvenance::None;
    self.com.payload = ComPayload::None;
    self.com.apartment = None;
    self.com.inputs = None;
    Ok(())
  }

  pub(crate) fn bind_current_com_apartment(&mut self) -> napi::Result<()> {
    if !matches!(
      self.winrt,
      dynwinrt::WinRTValue::Object(_) | dynwinrt::WinRTValue::Async(_)
    ) {
      return Err(napi::Error::from_reason(
        "Classic COM apartment binding requires a managed COM object",
      ));
    }
    if let Some(inputs) = &self.com.inputs {
      inputs.bind_current_apartment()?;
    }
    let current = std::thread::current().id();
    if self
      .com
      .apartment
      .as_ref()
      .is_some_and(|binding| binding.owner_thread != current)
    {
      return Err(napi::Error::from_reason(
        "Classic COM object is already bound to a different apartment thread",
      ));
    }
    if self.winrt.as_object().is_none() {
      return Err(napi::Error::from_reason(
        "Classic COM apartment binding requires a managed COM object",
      ));
    }
    if self.com.apartment.is_none() {
      self.com.apartment = Some(ComApartmentBinding::for_object(
        &self.winrt.as_object().expect("checked COM object"),
      ));
    }
    self.existing_com_context()?;
    if let Some(inputs) = &self.com.inputs {
      inputs.attach(self.com.apartment.as_ref().expect("bound COM object"))?;
    }
    Ok(())
  }

  pub(crate) fn ensure_com_apartment(&self) -> napi::Result<()> {
    let current = std::thread::current().id();
    match &self.com.apartment {
      Some(binding) if binding.owner_thread == current => Ok(()),
      Some(_) => Err(napi::Error::from_reason(
        "Classic COM object used from a different apartment thread",
      )),
      None => Err(napi::Error::from_reason(
        "Classic COM object must be apartment-bound before native invocation",
      )),
    }
  }

  pub(crate) fn ensure_existing_com_apartment(&self) -> napi::Result<()> {
    if let Some(inputs) = &self.com.inputs {
      if inputs.is_apartment_bound() {
        inputs.ensure_owner()?;
      }
    }
    if self.com.apartment.is_some() {
      self.ensure_com_apartment()
    } else {
      Ok(())
    }
  }

  pub(crate) fn com_context(&self) -> napi::Result<dynwinrt::com::borrowed::Identity> {
    self.ensure_com_apartment()?;
    let binding = self.com.apartment.as_ref().expect("checked apartment");
    binding.ensure_identity()?;
    if let Some(context) = binding.context.borrow().as_ref() {
      return Ok(context.clone());
    }
    let object = self
      .winrt
      .as_object()
      .ok_or_else(|| napi::Error::from_reason("COM object is released"))?;
    let context = dynwinrt::com::borrowed::Identity::for_object(&object)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    *binding.context.borrow_mut() = Some(context.clone());
    Ok(context)
  }

  pub(crate) fn existing_com_context(
    &self,
  ) -> napi::Result<Option<dynwinrt::com::borrowed::Identity>> {
    self.ensure_com_apartment()?;
    let binding = self.com.apartment.as_ref().expect("checked apartment");
    binding.ensure_identity()?;
    let object = match &self.winrt {
      dynwinrt::WinRTValue::Object(object) => object,
      dynwinrt::WinRTValue::Async(value) => (&value.info).into(),
      _ => return Err(napi::Error::from_reason("COM object is released")),
    };
    binding
      .refresh_context(object)
      .map_err(|error| napi::Error::from_reason(error.message()))?;
    Ok(binding.context.borrow().clone())
  }

  pub(crate) fn ensure_tracked_com_idle(&self) -> napi::Result<()> {
    if let Some(context) = self.existing_com_context()? {
      context
        .ensure_idle()
        .map_err(|error| napi::Error::from_reason(error.message()))?;
    }
    Ok(())
  }

  pub(crate) fn retain_com_context(
    &mut self,
    context: dynwinrt::com::borrowed::Identity,
  ) -> napi::Result<()> {
    let object = match &self.winrt {
      dynwinrt::WinRTValue::Object(object) => object,
      dynwinrt::WinRTValue::RawPtr(pointer)
        if self.com.pointer.provenance == PointerProvenance::ComOutput =>
      {
        unsafe { IUnknown::from_raw_borrowed(pointer) }
          .ok_or_else(|| napi::Error::from_reason("COM context output is null"))?
      }
      _ => {
        return Err(napi::Error::from_reason(
          "COM context requires an owned interface output",
        ))
      }
    };
    let binding = ComApartmentBinding::for_object(object);
    *binding.context.borrow_mut() = Some(context);
    self.com.apartment = Some(binding);
    if let Some(inputs) = &self.com.inputs {
      inputs.attach(self.com.apartment.as_ref().unwrap())?;
    }
    self.com.apartment.as_ref().unwrap().ensure_identity()
  }

  pub(crate) fn with_pointer_owner(value: dynwinrt::WinRTValue, owner: NativePointerOwner) -> Self {
    let mut result = Self::with_borrowed_pointer(value);
    result.com.pointer.owner = Some(owner);
    result
  }

  pub(crate) fn with_call_storage(value: dynwinrt::WinRTValue, storage: CallStorage) -> Self {
    let mut result = Self::with_borrowed_pointer(value);
    result.storage = Some(storage);
    result
  }

  pub(crate) fn with_borrowed_pointer(value: dynwinrt::WinRTValue) -> Self {
    Self {
      winrt: value,
      storage: None,
      com: ComValueState {
        pointer: NativePointerState {
          owner: None,
          provenance: PointerProvenance::Borrowed,
        },
        ..Default::default()
      },
    }
  }

  pub(crate) fn with_detached_com_owner(
    value: dynwinrt::WinRTValue,
    owner: std::sync::Arc<com_raw::RawComReference>,
  ) -> Self {
    let mut result = Self::with_pointer_owner(value, NativePointerOwner::RawCom(owner));
    result.com.pointer.provenance = PointerProvenance::DetachedCom;
    result
  }

  pub(crate) fn with_com_buffer(
    buffer: dynwinrt::com::ComBufferValue,
    storage: CallStorage,
  ) -> Self {
    let mut result = Self::with_call_storage(dynwinrt::WinRTValue::Null, storage);
    result.com.payload = ComPayload::Buffer(buffer);
    result
  }

  pub(crate) fn from_com_result(
    value: dynwinrt::WinRTValue,
    output_kind: dynwinrt::com::PointerOutputKind,
  ) -> Self {
    let provenance = if matches!(value, dynwinrt::WinRTValue::RawPtr(_)) {
      match output_kind {
        dynwinrt::com::PointerOutputKind::None | dynwinrt::com::PointerOutputKind::Unclassified => {
          PointerProvenance::UnclassifiedOutput
        }
        dynwinrt::com::PointerOutputKind::Com => PointerProvenance::ComOutput,
        dynwinrt::com::PointerOutputKind::CoTaskMem => PointerProvenance::CoTaskMemOutput,
        dynwinrt::com::PointerOutputKind::Bstr => PointerProvenance::BstrOutput,
        dynwinrt::com::PointerOutputKind::OwnedHandle(cleanup) => {
          PointerProvenance::OwnedHandleOutput(cleanup)
        }
      }
    } else {
      PointerProvenance::None
    };
    let apartment = if let dynwinrt::WinRTValue::Object(object) = &value {
      Some(ComApartmentBinding::for_object(object))
    } else {
      None
    };
    let mut result = Self::new(value);
    if let (Some(inputs), Some(binding)) = (&result.com.inputs, &apartment) {
      inputs
        .attach(binding)
        .expect("native COM output belongs to the current thread");
    }
    result.com.pointer.provenance = provenance;
    result.com.apartment = apartment;
    result
  }

  pub(crate) fn from_com_value(
    value: dynwinrt::com::Value,
    output_kind: dynwinrt::com::PointerOutputKind,
  ) -> Self {
    let inputs = com_input::InputBindings::capture(&value);
    let mut result = match value {
      dynwinrt::com::Value::WinRt(value) => Self::from_com_result(value, output_kind),
      dynwinrt::com::Value::NativeStruct(value) => {
        let mut result = Self::new(dynwinrt::WinRTValue::Null);
        result.com.payload = ComPayload::NativeStruct(value);
        result
      }
      dynwinrt::com::Value::Buffer(value) => {
        let mut result = Self::new(dynwinrt::WinRTValue::Null);
        result.com.payload = ComPayload::Buffer(value);
        result
      }
      value @ (dynwinrt::com::Value::Bstr(_)
      | dynwinrt::com::Value::NativeUnion(_)
      | dynwinrt::com::Value::Variant(_)
      | dynwinrt::com::Value::SafeArray(_)
      | dynwinrt::com::Value::PropVariant(_)
      | dynwinrt::com::Value::DispatchParams(_)
      | dynwinrt::com::Value::ExcepInfo(_)
      | dynwinrt::com::Value::StatStg(_)
      | dynwinrt::com::Value::FormatEtc(_)
      | dynwinrt::com::Value::StgMedium(_)
      | dynwinrt::com::Value::AudioFormat(_)) => {
        let mut result = Self::new(dynwinrt::WinRTValue::Null);
        result.com.payload = ComPayload::Automation(AutomationValue::new(value));
        result
      }
    };
    result.com.inputs = inputs.contains_interfaces().then_some(inputs);
    result
  }

  pub(crate) fn check_com_input_state(&self) -> napi::Result<()> {
    if let Some(inputs) = &self.com.inputs {
      inputs.ensure_idle()?;
    }
    if let Some(binding) = &self.com.apartment {
      binding.ensure_idle()?;
    }
    if let Some(automation) = self.automation() {
      automation.ensure_owner_thread()?;
    }
    Ok(())
  }

  pub(crate) fn admit_com_input(&self) -> napi::Result<()> {
    self.check_com_input_state()?;
    self.ensure_existing_com_apartment()?;
    if let Some(inputs) = &self.com.inputs {
      inputs.admit_winrt(&self.winrt)?;
    }
    if self.com.apartment.is_some() {
      self.ensure_tracked_com_idle()?;
    }
    Ok(())
  }

  pub(crate) fn to_com_value(&self) -> napi::Result<dynwinrt::com::Value> {
    self.admit_com_input()?;
    match &self.com.payload {
      ComPayload::Automation(value) => value.to_com_value(),
      ComPayload::Buffer(value) => Ok(dynwinrt::com::Value::Buffer(value.clone())),
      ComPayload::NativeStruct(value) => Ok(dynwinrt::com::Value::NativeStruct(value.clone())),
      ComPayload::None => Ok(dynwinrt::com::Value::WinRt(self.winrt.clone())),
    }
  }

  pub(crate) fn release_native_pointer_output(&mut self) -> napi::Result<()> {
    let ptr = match &self.winrt {
      dynwinrt::WinRTValue::RawPtr(ptr) => *ptr,
      _ => return Ok(()),
    };
    let provenance = self.com.pointer.provenance;
    if let PointerProvenance::OwnedHandleOutput(cleanup) = provenance {
      if !ptr.is_null() {
        com::DynComOwnedHandle::cleanup_address(ptr.addr(), cleanup)?;
      }
      self.winrt = dynwinrt::WinRTValue::Null;
      self.com.pointer.provenance = PointerProvenance::None;
      return Ok(());
    }
    self.winrt = dynwinrt::WinRTValue::Null;
    self.com.pointer.provenance = PointerProvenance::None;
    if ptr.is_null() {
      return Ok(());
    }
    match provenance {
      PointerProvenance::ComOutput => {
        drop(unsafe { IUnknown::from_raw(ptr) });
      }
      PointerProvenance::CoTaskMemOutput => unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(ptr));
      },
      PointerProvenance::BstrOutput => {
        drop(unsafe { windows::core::BSTR::from_raw(ptr.cast()) });
      }
      PointerProvenance::OwnedHandleOutput(_) => unreachable!(),
      PointerProvenance::None
      | PointerProvenance::Borrowed
      | PointerProvenance::DetachedCom
      | PointerProvenance::UnclassifiedOutput => {}
    }
    Ok(())
  }
}

impl Drop for DynWinRTValue {
  fn drop(&mut self) {
    if self
      .com
      .inputs
      .as_ref()
      .is_some_and(|inputs| inputs.is_apartment_bound() && !inputs.is_owner())
      || self
        .com
        .apartment
        .as_ref()
        .is_some_and(|binding| binding.owner_thread != std::thread::current().id())
    {
      let value = mem::replace(&mut self.winrt, dynwinrt::WinRTValue::Null);
      mem::forget(value);
      if let Some(binding) = self.com.apartment.take() {
        mem::forget(binding);
      }
      if let Some(value) = self.take_com_buffer() {
        mem::forget(value);
      }
      return;
    }
    // After Application.Start returns, XAML has already torn down its thread
    // state. Leaking late projected COM references is safer than releasing
    // them into a destroyed DXamlCore; normal application teardown must call
    // release()/releaseProjected() before this process-exit fallback is needed.
    if winui_dispatcher_loop_exited() {
      self.com.pointer.provenance = PointerProvenance::None;
      let value = mem::replace(&mut self.winrt, dynwinrt::WinRTValue::Null);
      mem::forget(value);
      if let Some(value) = self.take_com_buffer() {
        mem::forget(value);
      }
      if let Some(value) = self.automation_mut() {
        value.leak_for_shutdown();
      }
      if let Some(binding) = self.com.apartment.take() {
        mem::forget(binding);
      }
    } else {
      let _ = self.release_native_pointer_output();
    }
  }
}

pub(crate) fn collect_invocation_leases(
  args: &[&DynWinRTValue],
) -> napi::Result<NativeInvocationLeases> {
  let mut raw_memory = Vec::new();
  let mut raw_com = Vec::new();
  for arg in args {
    arg.validate_call_storage()?;
    if let Some(owner) = arg.pointer_owner() {
      match owner {
        NativePointerOwner::RawMemory(allocation) => {
          raw_memory.push(allocation.acquire_invocation_lease()?);
        }
        NativePointerOwner::RawCom(reference) => {
          raw_com.push(reference.acquire_invocation_lease()?);
        }
        _ => {
          owner.validate()?;
        }
      }
    }
  }
  Ok(NativeInvocationLeases {
    _raw_memory: raw_memory,
    _raw_com: raw_com,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn winrt_scalar_construction_has_no_com_ownership_or_payload() {
    for native in [
      dynwinrt::WinRTValue::Null,
      dynwinrt::WinRTValue::Bool(true),
      dynwinrt::WinRTValue::U64(u64::MAX),
      dynwinrt::WinRTValue::HString("plain WinRT".into()),
    ] {
      let value = DynWinRTValue::new(native);
      assert!(!value.has_pointer_owner());
      assert!(!value.has_com_payload());
      assert!(value.input_bindings().is_none());
      assert!(value.com_binding().is_none());
      assert_eq!(value.pointer_provenance(), PointerProvenance::None);
    }
  }

  #[test]
  fn taking_a_buffer_payload_does_not_consume_its_independent_backing_owner() {
    let mut bytes = vec![1, 2, 3, 4].into_boxed_slice();
    let pointer = bytes.as_mut_ptr();
    let buffer =
      unsafe { dynwinrt::com::ComBufferValue::borrowed(pointer, bytes.len(), 1, true, true) }
        .unwrap();
    let mut value = DynWinRTValue::with_com_buffer(buffer, CallStorage::Bytes(bytes));
    assert!(value.native_struct().is_none());
    assert!(value.automation().is_none());
    assert!(value.take_com_buffer().is_some());
    assert!(value.take_com_buffer().is_none());
    assert!(value.has_pointer_owner());
    assert_eq!(value.pointer_provenance(), PointerProvenance::Borrowed);
    assert_eq!(
      unsafe { std::slice::from_raw_parts(pointer, 4) },
      [1, 2, 3, 4]
    );
    assert!(
      com::take_native_output_pointer(&mut value, PointerProvenance::ComOutput, "test")
        .unwrap_err()
        .reason
        .contains("owner-backed")
    );
    value.release().unwrap();
    assert!(!value.has_pointer_owner());
    assert_eq!(value.pointer_provenance(), PointerProvenance::None);
  }

  #[test]
  fn failed_payload_conversion_preserves_automation_until_explicit_release() {
    let mut value = DynWinRTValue::from_com_value(
      dynwinrt::com::Value::Bstr(dynwinrt::com::BstrValue::new("retained")),
      dynwinrt::com::PointerOutputKind::None,
    );
    assert!(value.take_com_buffer().is_none());
    assert!(value.automation_mut().unwrap().take_safe_array().is_err());
    assert_eq!(value.to_string(), "retained");
    assert!(!value.is_null());
    value.release().unwrap();
    assert!(value.is_null());
    assert!(!value.has_com_payload());
  }
}
