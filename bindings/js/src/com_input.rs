// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Apartment-local lifetime pins for managed COM inputs, including containers.

use std::{
  mem::ManuallyDrop,
  rc::Rc,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard, OnceLock,
  },
  thread::ThreadId,
};

use super::{ComApartmentBinding, DynWinRTValue};

fn type_has_interfaces(typ: &dynwinrt::TypeHandle) -> bool {
  typ.kind().is_com_pointer()
    || typ.kind() == dynwinrt::TypeKind::ArrayOfIUnknown
    || (typ.is_array() && type_has_interfaces(&typ.array_element_type()))
    || (matches!(typ.kind(), dynwinrt::TypeKind::Struct(_))
      && (0..typ.field_count()).any(|index| type_has_interfaces(&typ.field_type(index))))
}

pub(super) fn winrt_has_interfaces(value: &dynwinrt::WinRTValue) -> bool {
  match value {
    dynwinrt::WinRTValue::Object(_)
    | dynwinrt::WinRTValue::Async(_)
    | dynwinrt::WinRTValue::ArrayOfIUnknown(_) => true,
    dynwinrt::WinRTValue::Array(value) => type_has_interfaces(&value.element_type),
    dynwinrt::WinRTValue::Struct(value) => type_has_interfaces(&value.type_handle()),
    _ => false,
  }
}

struct State {
  owner: Arc<OnceLock<ThreadId>>,
  bindings: ManuallyDrop<Vec<ComApartmentBinding>>,
  children: Vec<InputBindings>,
  initialized: bool,
  error: Option<String>,
}

// Before claiming an owner, this state contains only thread-safe bookkeeping.
// Afterwards, every access to its apartment-local bindings checks that owner
// while holding the mutex. Foreign destruction never touches their Rc counts.
unsafe impl Send for State {}

impl Drop for State {
  fn drop(&mut self) {
    if self.owner.get().is_none()
      || (self.owner.get() == Some(&std::thread::current().id())
        && !super::winui_dispatcher_loop_exited())
    {
      unsafe { ManuallyDrop::drop(&mut self.bindings) };
    }
  }
}

struct SharedState {
  value: Mutex<State>,
  contains_interfaces: AtomicBool,
}

pub(super) struct InputBindings {
  // Pointer-free owner tokens also reach pre-existing arrays/struct snapshots.
  // An unclaimed token imposes no affinity on an ordinary WinRT carrier.
  apartment_tokens: Vec<Arc<OnceLock<ThreadId>>>,
  state: Arc<SharedState>,
}

impl InputBindings {
  pub(super) fn deferred() -> Self {
    Self::with_children(Vec::new(), None)
  }

  fn with_children(children: Vec<InputBindings>, error: Option<String>) -> Self {
    let owner = Arc::new(OnceLock::new());
    let mut apartment_tokens = vec![owner.clone()];
    for child in &children {
      apartment_tokens.extend(child.apartment_tokens.iter().cloned());
    }
    Self {
      apartment_tokens,
      state: Arc::new(SharedState {
        contains_interfaces: AtomicBool::new(error.is_some()),
        value: Mutex::new(State {
          owner,
          bindings: ManuallyDrop::new(Vec::new()),
          children,
          initialized: false,
          error,
        }),
      }),
    }
  }

  pub(super) fn capture(value: &dynwinrt::com::Value) -> Self {
    let result = Self::deferred();
    if let Err(error) = result.initialize(value) {
      return Self::with_children(vec![result], Some(error.reason.clone()));
    }
    result
  }

  pub(super) fn is_owner(&self) -> bool {
    let current = std::thread::current().id();
    self
      .apartment_tokens
      .iter()
      .all(|token| token.get().is_none_or(|owner| *owner == current))
  }

  pub(super) fn is_apartment_bound(&self) -> bool {
    self
      .apartment_tokens
      .iter()
      .any(|token| token.get().is_some())
  }

  pub(super) fn contains_interfaces(&self) -> bool {
    self.state.contains_interfaces.load(Ordering::Relaxed)
  }

  pub(super) fn ensure_owner(&self) -> napi::Result<()> {
    if !self.is_owner() {
      return Err(napi::Error::from_reason(
        "Managed COM input used from a different apartment thread",
      ));
    }
    Ok(())
  }

  pub(super) fn pin(&self) -> napi::Result<Self> {
    self.ensure_owner()?;
    Ok(Self {
      apartment_tokens: self.apartment_tokens.clone(),
      state: self.state.clone(),
    })
  }

  fn locked(&self) -> napi::Result<MutexGuard<'_, State>> {
    let state = self
      .state
      .value
      .lock()
      .map_err(|_| napi::Error::from_reason("Managed COM input bookkeeping lock is poisoned"))?;
    self.ensure_owner()?;
    Ok(state)
  }

  pub(super) fn bind_current_apartment(&self) -> napi::Result<()> {
    let children = {
      let state = self.locked()?;
      if let Some(error) = &state.error {
        return Err(napi::Error::from_reason(error.clone()));
      }
      state.owner.get_or_init(|| std::thread::current().id());
      self.ensure_owner()?;
      state
        .children
        .iter()
        .map(Self::pin)
        .collect::<napi::Result<Vec<_>>>()?
    };
    for child in children {
      child.bind_current_apartment()?;
    }
    Ok(())
  }

  // WinRT construction only copies private bookkeeping; it neither queries
  // interfaces nor imposes COM lifecycle rules on the WinRT API.
  pub(super) fn inherit(values: &[&DynWinRTValue]) -> Self {
    Self::with_children(
      values
        .iter()
        .map(|value| {
          value
            .7
            .as_ref()
            .map_or_else(Self::deferred, Self::copy_bookkeeping)
        })
        .collect(),
      None,
    )
  }

  pub(super) fn fields(values: &[Option<InputBindings>]) -> Self {
    Self::with_children(
      values
        .iter()
        .map(|value| {
          value
            .as_ref()
            .map_or_else(Self::deferred, Self::copy_bookkeeping)
        })
        .collect(),
      None,
    )
  }

  pub(super) fn copy_bookkeeping(&self) -> Self {
    match self.pin() {
      Ok(pin) => pin,
      Err(error) => self.failed_copy(error),
    }
  }

  fn failed_copy(&self, error: napi::Error) -> Self {
    let mut result = Self::with_children(Vec::new(), Some(error.reason.clone()));
    result
      .apartment_tokens
      .extend(self.apartment_tokens.iter().cloned());
    result
  }

  pub(super) fn element_bookkeeping(&self, index: usize) -> Self {
    // Preserve element positions, rather than retaining other elements' empty
    // identity slots after the container's native references have been dropped.
    match self.locked() {
      Ok(state) => state
        .children
        .get(index)
        .map_or_else(Self::deferred, Self::copy_bookkeeping),
      Err(error) => self.failed_copy(error),
    }
  }

  pub(super) fn attach(&self, binding: &ComApartmentBinding) -> napi::Result<()> {
    if binding.owner_thread != std::thread::current().id() {
      return Err(napi::Error::from_reason(
        "Managed COM binding belongs to a different apartment thread",
      ));
    }
    self.bind_current_apartment()?;
    let mut state = self.locked()?;
    if !state
      .bindings
      .iter()
      .any(|existing| Rc::ptr_eq(&existing.context, &binding.context))
    {
      state.bindings.push(binding.clone());
    }
    state.initialized = true;
    self
      .state
      .contains_interfaces
      .store(true, Ordering::Relaxed);
    Ok(())
  }

  fn initialize(&self, value: &dynwinrt::com::Value) -> napi::Result<()> {
    if let dynwinrt::com::Value::WinRt(value) = value {
      return self.initialize_winrt(value);
    }
    self.bind_current_apartment()?;
    let mut bindings = Vec::new();
    let result = value.visit_interfaces(&mut |object| {
      let binding = ComApartmentBinding::for_object(object);
      binding.refresh_context(object)?;
      bindings.push(binding);
      Ok(())
    });
    let mut state = self.locked()?;
    state.bindings.extend(bindings);
    state.initialized = true;
    if let Err(error) = result {
      state.error = Some(error.message());
    }
    self.state.contains_interfaces.store(
      !state.bindings.is_empty() || state.error.is_some(),
      Ordering::Relaxed,
    );
    Ok(())
  }

  fn initialize_winrt(&self, value: &dynwinrt::WinRTValue) -> napi::Result<()> {
    self.bind_current_apartment()?;
    let previous_children = {
      let state = self.locked()?;
      state
        .children
        .iter()
        .map(Self::pin)
        .collect::<napi::Result<Vec<_>>>()?
    };
    let mut bindings = Vec::new();
    let mut children = Vec::new();
    let mut initialize_child = |index: usize, value: dynwinrt::WinRTValue| -> napi::Result<()> {
      let child = previous_children
        .get(index)
        .map(Self::pin)
        .transpose()?
        .unwrap_or_else(Self::deferred);
      let initialized = child.locked()?.initialized;
      if !initialized {
        child.initialize_winrt(&value)?;
      }
      children.push(child);
      Ok(())
    };
    let result = if let dynwinrt::WinRTValue::Array(array) = value {
      (0..array.len()).try_for_each(|index| {
        let value = array
          .try_get(index)
          .map_err(|error| napi::Error::from_reason(error.message()))?;
        initialize_child(index, value)
      })
    } else if let dynwinrt::WinRTValue::Struct(value) = value {
      let typ = value.type_handle();
      (0..typ.field_count()).try_for_each(|index| {
        let field = typ.field_type(index);
        let value = if field.kind().is_com_pointer() {
          value
            .get_field_object(index)
            .map_err(|error| napi::Error::from_reason(error.message()))?
            .map_or(dynwinrt::WinRTValue::Null, dynwinrt::WinRTValue::Object)
        } else if matches!(field.kind(), dynwinrt::TypeKind::Struct(_)) {
          dynwinrt::WinRTValue::Struct(
            value
              .get_field_struct_checked(index)
              .map_err(|error| napi::Error::from_reason(error.message()))?,
          )
        } else {
          dynwinrt::WinRTValue::Null
        };
        initialize_child(index, value)
      })
    } else {
      dynwinrt::com::visit_winrt_interfaces(value, &mut |object| {
        let binding = ComApartmentBinding::for_object(object);
        binding.refresh_context(object)?;
        bindings.push(binding);
        Ok(())
      })
      .map_err(|error| napi::Error::from_reason(error.message()))
    };
    let mut state = self.locked()?;
    state.bindings.extend(bindings);
    let old_children = std::mem::replace(&mut state.children, children);
    state.initialized = true;
    if let Err(error) = result {
      state.error = Some(error.reason.clone());
    }
    self.state.contains_interfaces.store(
      !state.bindings.is_empty()
        || state.error.is_some()
        || state.children.iter().any(Self::contains_interfaces),
      Ordering::Relaxed,
    );
    drop(state);
    drop(old_children);
    Ok(())
  }

  pub(super) fn admit_winrt(&self, value: &dynwinrt::WinRTValue) -> napi::Result<()> {
    self.ensure_idle()?;
    let initialized = self.locked()?.initialized;
    if !initialized {
      // The COM boundary, not normal WinRT construction, resolves identities.
      self.initialize_winrt(value)?;
    }
    self.ensure_idle()
  }

  pub(super) fn ensure_idle(&self) -> napi::Result<()> {
    let (children, bindings) = {
      let state = self.locked()?;
      if let Some(error) = &state.error {
        return Err(napi::Error::from_reason(error.clone()));
      }
      (
        state
          .children
          .iter()
          .map(Self::pin)
          .collect::<napi::Result<Vec<_>>>()?,
        state.bindings.to_vec(),
      )
    };
    for child in children {
      child.ensure_idle()?;
    }
    for binding in bindings {
      binding.ensure_idle()?;
    }
    Ok(())
  }

  #[cfg(test)]
  pub(super) fn state_alive_probe(&self) -> impl Fn() -> bool + Send + Sync + 'static {
    let weak = Arc::downgrade(&self.state);
    move || weak.strong_count() != 0
  }
}
