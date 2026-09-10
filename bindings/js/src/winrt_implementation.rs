// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
  cell::{Cell, RefCell},
  collections::HashMap,
  ptr,
  rc::Rc,
  sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
  },
};

use dynwinrt::{
  WinRTValue, WinRtImplementation, WinRtImplementationPlan, WinRtInterfaceDefinition,
  WinRtMethodDefinition, WinRtThreadingPolicy,
};
use napi::{
  bindgen_prelude::{FromNapiValue, Function, JavaScriptClassExt, ToNapiValue, Unknown},
  sys, Env, JsValue,
};
use napi_derive::napi;
use windows::core::HRESULT;

use crate::{managed_tsfn::ManagedTsfn, DynWinRTMethodSig, DynWinRTType, DynWinRTValue, WinGUID};

static NEXT_IMPLEMENTATION_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "test-hooks")]
pub(crate) static FINALIZED_CALLBACKS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "test-hooks")]
pub(crate) static ENV_DISCONNECTS: AtomicU64 = AtomicU64::new(0);

thread_local! {
  // N-API owns these controllers on the environment thread. Callback closures
  // never capture them: foreign native destruction only releases a cleanup TSFN.
  static ENVIRONMENTS: RefCell<HashMap<usize, Rc<Environment>>> = RefCell::new(HashMap::new());
}

struct Environment {
  closed: Cell<bool>,
  implementations: RefCell<HashMap<u64, Rc<ImplementationState>>>,
}

struct ImplementationState {
  native: RefCell<WinRtImplementation>,
}

fn environment(env: Env) -> napi::Result<Rc<Environment>> {
  let key = env.raw() as usize;
  if let Some(environment) = ENVIRONMENTS.with(|entries| entries.borrow().get(&key).cloned()) {
    if environment.closed.get() {
      return Err(napi::Error::from_reason(
        "Cannot create a WinRT implementation while the Node environment is closing",
      ));
    }
    return Ok(environment);
  }
  let environment = Rc::new(Environment {
    closed: Cell::new(false),
    implementations: RefCell::new(HashMap::new()),
  });
  // Installed after ManagedTsfn's hook. Node runs cleanup hooks in reverse
  // registration order, so native controls disconnect before TSFN teardown.
  let _hook = env.add_env_cleanup_hook((key, environment.clone()), |(key, environment)| {
    environment.closed.set(true);
    ENVIRONMENTS.with(|entries| entries.borrow_mut().remove(&key));
    let states = environment
      .implementations
      .borrow_mut()
      .drain()
      .map(|(_, state)| state)
      .collect::<Vec<_>>();
    for state in states {
      if let Err(error) = state.native.borrow_mut().dispose() {
        eprintln!("[dynwinrt] WinRT implementation environment cleanup: {error}");
      }
      #[cfg(feature = "test-hooks")]
      ENV_DISCONNECTS.fetch_add(1, Ordering::Relaxed);
    }
  })?;
  ENVIRONMENTS.with(|entries| entries.borrow_mut().insert(key, environment.clone()));
  Ok(environment)
}

fn forget_implementation(env_key: usize, id: u64) {
  let environment = ENVIRONMENTS.with(|entries| entries.borrow().get(&env_key).cloned());
  if let Some(environment) = environment {
    let removed = environment.implementations.borrow_mut().remove(&id);
    drop(removed);
  }
}

fn binding_error(error: windows::core::Error) -> napi::Error {
  napi::Error::from_reason(error.message())
}

pub(crate) fn require_instance<T: JavaScriptClassExt>(
  value: &Unknown<'_>,
  name: &str,
) -> napi::Result<()> {
  // JavaScriptClassExt uses the registered JS name (DynWinRt*). napi-rs's
  // ValidateNapiValue derives instead look up the legacy Rust DynWinRT* name.
  if T::instance_of(&Env::from_raw(value.value().env), value)? {
    Ok(())
  } else {
    Err(napi::Error::from_reason(format!(
      "Expected a {name} instance"
    )))
  }
}

/// One complete method contract, including every output in ABI parameter order.
#[napi(object, object_to_js = false)]
pub struct DynWinRtImplementationMethod<'env> {
  pub name: String,
  pub vtable_index: f64,
  #[napi(ts_type = "DynWinRtMethodSig")]
  pub signature: Unknown<'env>,
}

/// A standalone WinRT interface descriptor. Creation never registers a COM class
/// or changes the outbound WinRT method registry.
#[napi]
pub struct DynWinRtInterfacePlan(WinRtInterfaceDefinition);

#[napi]
impl DynWinRtInterfacePlan {
  #[napi(factory, strict)]
  pub fn create(
    name: String,
    #[napi(ts_arg_type = "DynWinRtType")] interface_type: Unknown<'_>,
    methods: Vec<DynWinRtImplementationMethod<'_>>,
    #[napi(ts_arg_type = "WinGuid[]")] required_iids: Option<Vec<Unknown<'_>>>,
  ) -> napi::Result<Self> {
    require_instance::<DynWinRTType>(&interface_type, "DynWinRtType")?;
    let interface_type = unsafe {
      <&DynWinRTType>::from_napi_value(interface_type.value().env, interface_type.raw())
    }?;
    let methods = methods
      .into_iter()
      .map(|method| {
        let env = method.signature.value().env;
        let raw = method.signature.raw();
        require_instance::<DynWinRTMethodSig>(&method.signature, "DynWinRtMethodSig")?;
        let signature = unsafe { <&DynWinRTMethodSig>::from_napi_value(env, raw) }?;
        Ok(WinRtMethodDefinition {
          name: method.name,
          vtable_index: crate::js_u32(method.vtable_index, "vtableIndex")? as usize,
          signature: signature.0.clone(),
        })
      })
      .collect::<napi::Result<Vec<_>>>()?;
    let required_iids = required_iids
      .unwrap_or_default()
      .into_iter()
      .map(|iid| {
        let env = iid.value().env;
        let raw = iid.raw();
        require_instance::<WinGUID>(&iid, "WinGuid")?;
        Ok(unsafe { <&WinGUID>::from_napi_value(env, raw) }?.0)
      })
      .collect::<napi::Result<Vec<_>>>()?;
    let definition = WinRtInterfaceDefinition {
      name,
      interface_type: interface_type.type_handle(),
      required_iids,
      methods,
    };
    WinRtImplementationPlan::validate_interface(&definition).map_err(binding_error)?;
    Ok(Self(definition))
  }
}

/// Owns one reference to a synchronous, non-agile WinRT implementation.
///
/// release()/GC drops this owner's reference only. Native references keep the
/// callback alive. dispose() disconnects every view and breaks callback cycles.
/// Call dispose() explicitly when a handler captures its owner or a native view;
/// those cross-runtime cycles are not visible to JavaScript's garbage collector.
#[napi]
pub struct DynWinRtImplementation {
  state: Rc<ImplementationState>,
}

impl Drop for DynWinRtImplementation {
  fn drop(&mut self) {
    self.state.native.borrow_mut().release();
  }
}

#[napi]
impl DynWinRtImplementation {
  #[napi(factory, strict)]
  pub fn create(
    env: Env,
    #[napi(ts_arg_type = "DynWinRtInterfacePlan[]")] interfaces: Vec<Unknown<'_>>,
    #[napi(
      ts_arg_type = "(interfaceIndex: number, vtableIndex: number, args: DynWinRtValue[]) => DynWinRtValue[]"
    )]
    callback: Function<'_, (), Unknown<'_>>,
    runtime_class_name: Option<String>,
  ) -> napi::Result<Self> {
    let definitions = interfaces
      .into_iter()
      .map(|interface| {
        let raw = interface.raw();
        require_instance::<DynWinRtInterfacePlan>(&interface, "DynWinRtInterfacePlan")?;
        Ok(
          unsafe { <&DynWinRtInterfacePlan>::from_napi_value(env.raw(), raw) }?
            .0
            .clone(),
        )
      })
      .collect::<napi::Result<Vec<_>>>()?;
    let plan = WinRtImplementationPlan::new(definitions, WinRtThreadingPolicy::OwnerThread)
      .map_err(binding_error)?;
    reject_async_function(env.raw(), callback.raw())?;

    let id = NEXT_IMPLEMENTATION_ID.fetch_add(1, Ordering::Relaxed);
    let env_key = env.raw() as usize;
    let (callback_ref, async_context, finalize_resources) =
      crate::create_direct_callback_resources(
        env.raw(),
        callback.raw(),
        b"dynwinrt.winrtImplementation",
      )?;
    let finalizer = Box::new(move |env| {
      forget_implementation(env_key, id);
      finalize_resources(env);
      #[cfg(feature = "test-hooks")]
      FINALIZED_CALLBACKS.fetch_add(1, Ordering::Relaxed);
    });
    let tsfn = ManagedTsfn::create(
      env.raw(),
      callback.raw(),
      1,
      true,
      |(), _| Ok(Vec::new()),
      Some(finalizer),
    )?;
    let environment = environment(env)?;
    let direct = crate::DirectJsCallback {
      env: env.raw(),
      callback_ref,
      async_context,
      lifecycle: tsfn.lifecycle(),
    };
    let callback: dynwinrt::WinRtImplementationCallback =
      Arc::new(move |interface_index, vtable_index, values| {
        // This TSFN is only a lifetime/finalizer bridge. Native enforces the
        // owner thread before entry; method dispatch is never queued.
        let _resources = &tsfn;
        invoke_callback(&direct, interface_index, vtable_index, values).map_err(|error| {
          windows::core::Error::new(
            HRESULT(0x80004005u32 as i32),
            format!("WinRT implementation callback failed: {}", error.reason),
          )
        })
      });
    let native = WinRtImplementation::new(plan, callback, runtime_class_name.as_deref())
      .map_err(binding_error)?;
    let state = Rc::new(ImplementationState {
      native: RefCell::new(native),
    });
    environment
      .implementations
      .borrow_mut()
      .insert(id, state.clone());
    Ok(Self { state })
  }

  /// Returns a separately owned canonical IInspectable value.
  #[napi]
  pub fn to_value(&self) -> napi::Result<DynWinRTValue> {
    self
      .state
      .native
      .borrow()
      .to_value()
      .map(DynWinRTValue::new)
      .map_err(binding_error)
  }

  /// Drops only this owner's native reference. Retained views remain callable.
  #[napi]
  pub fn release(&self) {
    self.state.native.borrow_mut().release();
  }

  /// Disconnects callbacks on every view, retaining this owner's reference.
  #[napi]
  pub fn disconnect(&self) -> napi::Result<()> {
    self
      .state
      .native
      .borrow()
      .disconnect()
      .map_err(binding_error)
  }

  /// Disconnects all views, releases the owner, and breaks callback roots.
  /// Repeated calls are harmless; an active callback may finish before cleanup.
  #[napi]
  pub fn dispose(&self) -> napi::Result<()> {
    self
      .state
      .native
      .borrow_mut()
      .dispose()
      .map_err(binding_error)
  }

  #[napi(getter)]
  pub fn is_closed(&self) -> bool {
    self.state.native.borrow().is_closed()
  }

  /// Returns and clears the latest native dispatch/validation error.
  #[napi]
  pub fn take_error(&self) -> Option<String> {
    self.state.native.borrow().take_error()
  }
}

fn reject_async_function(env: sys::napi_env, callback: sys::napi_value) -> napi::Result<()> {
  let mut constructor = ptr::null_mut();
  crate::napi_status("napi_get_named_property(callback constructor)", unsafe {
    sys::napi_get_named_property(env, callback, c"constructor".as_ptr(), &mut constructor)
  })?;
  let mut typ = sys::ValueType::napi_undefined;
  crate::napi_status("napi_typeof(callback constructor)", unsafe {
    sys::napi_typeof(env, constructor, &mut typ)
  })?;
  if typ == sys::ValueType::napi_function {
    let mut name = ptr::null_mut();
    crate::napi_status(
      "napi_get_named_property(callback constructor name)",
      unsafe { sys::napi_get_named_property(env, constructor, c"name".as_ptr(), &mut name) },
    )?;
    let name = unsafe { String::from_napi_value(env, name) }?;
    if name == "AsyncFunction" || name == "AsyncGeneratorFunction" || name == "GeneratorFunction" {
      return Err(napi::Error::from_reason(
        "WinRT implementation handlers must be synchronous functions, not async/generator functions",
      ));
    }
  }
  Ok(())
}

fn parse_outputs(env: sys::napi_env, result: sys::napi_value) -> napi::Result<Vec<WinRTValue>> {
  let mut promise = false;
  crate::napi_status("napi_is_promise(WinRT result)", unsafe {
    sys::napi_is_promise(env, result, &mut promise)
  })?;
  let mut typ = sys::ValueType::napi_undefined;
  crate::napi_status("napi_typeof(WinRT result)", unsafe {
    sys::napi_typeof(env, result, &mut typ)
  })?;
  if matches!(
    typ,
    sys::ValueType::napi_object | sys::ValueType::napi_function
  ) {
    let mut then = ptr::null_mut();
    crate::napi_status("napi_get_named_property(WinRT result then)", unsafe {
      sys::napi_get_named_property(env, result, c"then".as_ptr(), &mut then)
    })?;
    let mut then_type = sys::ValueType::napi_undefined;
    crate::napi_status("napi_typeof(WinRT result then)", unsafe {
      sys::napi_typeof(env, then, &mut then_type)
    })?;
    promise |= then_type == sys::ValueType::napi_function;
  }
  if promise {
    return Err(napi::Error::from_reason(
      "WinRT implementation handlers must return synchronous DynWinRtValue[] results, not a Promise/thenable",
    ));
  }
  let mut array = false;
  crate::napi_status("napi_is_array(WinRT outputs)", unsafe {
    sys::napi_is_array(env, result, &mut array)
  })?;
  if !array {
    return Err(napi::Error::from_reason(
      "WinRT implementation handlers must return a DynWinRtValue[] (use [] for void)",
    ));
  }
  let mut length = 0;
  crate::napi_status("napi_get_array_length(WinRT outputs)", unsafe {
    sys::napi_get_array_length(env, result, &mut length)
  })?;
  let mut outputs = Vec::with_capacity(length as usize);
  for index in 0..length {
    let mut raw = ptr::null_mut();
    crate::napi_status("napi_get_element(WinRT output)", unsafe {
      sys::napi_get_element(env, result, index, &mut raw)
    })?;
    let value = unsafe { Unknown::from_napi_value(env, raw) }?;
    require_instance::<DynWinRTValue>(&value, "DynWinRtValue").map_err(|error| {
      napi::Error::from_reason(format!(
        "WinRT output {index} must be a DynWinRtValue: {}",
        error.reason
      ))
    })?;
    outputs.push(
      unsafe { <&DynWinRTValue>::from_napi_value(env, raw) }?
        .0
        .clone(),
    );
  }
  Ok(outputs)
}

fn callback_error(env: sys::napi_env, error: napi::Error) -> napi::Error {
  let mut pending = false;
  if unsafe { sys::napi_is_exception_pending(env, &mut pending) } != sys::Status::napi_ok
    || !pending
  {
    return error;
  }
  let mut exception = ptr::null_mut();
  if unsafe { sys::napi_get_and_clear_last_exception(env, &mut exception) } != sys::Status::napi_ok
  {
    return error;
  }
  let mut message = ptr::null_mut();
  if unsafe { sys::napi_coerce_to_string(env, exception, &mut message) } == sys::Status::napi_ok {
    if let Ok(message) = unsafe { String::from_napi_value(env, message) } {
      return napi::Error::from_reason(message);
    }
  }
  // Even an exception's toString() may throw. Never leak a pending exception
  // through the foreign HRESULT boundary.
  let mut ignored = ptr::null_mut();
  unsafe { sys::napi_get_and_clear_last_exception(env, &mut ignored) };
  napi::Error::from_reason(format!(
    "JavaScript handler threw an exception (message unavailable): {}",
    error.reason
  ))
}

struct CallbackHandleScope {
  env: sys::napi_env,
  scope: Option<sys::napi_handle_scope>,
}

impl CallbackHandleScope {
  fn open(env: sys::napi_env) -> napi::Result<Self> {
    let mut scope = ptr::null_mut();
    crate::napi_status("napi_open_handle_scope(WinRT callback)", unsafe {
      sys::napi_open_handle_scope(env, &mut scope)
    })?;
    Ok(Self {
      env,
      scope: Some(scope),
    })
  }

  fn close(&mut self) -> napi::Result<()> {
    if let Some(scope) = self.scope.take() {
      crate::napi_status("napi_close_handle_scope(WinRT callback)", unsafe {
        sys::napi_close_handle_scope(self.env, scope)
      })?;
    }
    Ok(())
  }
}

impl Drop for CallbackHandleScope {
  fn drop(&mut self) {
    // Native catches Rust panics; close the N-API scope during unwinding too.
    if let Err(error) = self.close() {
      eprintln!("[dynwinrt] WinRT callback handle scope cleanup: {error}");
    }
  }
}

fn invoke_callback(
  direct: &crate::DirectJsCallback,
  interface_index: usize,
  vtable_index: usize,
  values: &[WinRTValue],
) -> napi::Result<Vec<WinRTValue>> {
  if direct.lifecycle.is_closing() {
    return Err(napi::Error::from_reason(
      "WinRT implementation's Node environment is closing",
    ));
  }
  let env = direct.env;
  let mut scope = CallbackHandleScope::open(env)?;
  let result = (|| {
    let mut function = ptr::null_mut();
    crate::napi_status("napi_get_reference_value(WinRT callback)", unsafe {
      sys::napi_get_reference_value(env, direct.callback_ref, &mut function)
    })?;
    let js_values = values
      .iter()
      .cloned()
      .map(DynWinRTValue::new)
      .collect::<Vec<_>>();
    let args = unsafe {
      [
        u32::to_napi_value(env, interface_index as u32)?,
        u32::to_napi_value(env, vtable_index as u32)?,
        Vec::<DynWinRTValue>::to_napi_value(env, js_values)?,
      ]
    };
    let mut receiver = ptr::null_mut();
    crate::napi_status("napi_get_global(WinRT callback)", unsafe {
      sys::napi_get_global(env, &mut receiver)
    })?;
    let mut result = ptr::null_mut();
    crate::napi_status("napi_make_callback(WinRT implementation)", unsafe {
      sys::napi_make_callback(
        env,
        direct.async_context,
        receiver,
        function,
        args.len(),
        args.as_ptr(),
        &mut result,
      )
    })?;
    parse_outputs(env, result)
  })()
  .map_err(|error| callback_error(env, error));
  let close = scope.close();
  match result {
    Err(error) => Err(error),
    Ok(outputs) => close.map(|()| outputs),
  }
}

#[cfg(feature = "test-hooks")]
pub(crate) fn live_control_count() -> u32 {
  ENVIRONMENTS.with(|entries| {
    entries
      .borrow()
      .values()
      .map(|env| env.implementations.borrow().len() as u32)
      .sum()
  })
}
