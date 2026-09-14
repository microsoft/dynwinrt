// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#![deny(clippy::all)]
#![allow(clippy::missing_safety_doc)]

mod com;
pub use com::{
  initialize_com, DynCom, DynComDispatchInvokeResult, DynComDispatchParams, DynComExcepInfo,
  DynComInterface, DynComMethodHandle, DynComMethodSig, DynComNativeStruct,
  DynComNativeStructArray, DynComNativeUnion, DynComPropVariant, DynComSafeArray,
  DynComSafeArrayBound, DynComType, DynComUnsafe, DynComUnsafeInterface, DynComVariant,
};
mod com_raw;
pub use com_raw::{
  DynComRaw, DynComRawCleanup, DynComRawMemory, DynComRawOwnedComPointer, DynComRawPointer,
  DynComRawStructLayout, DynComRawUnionLayout,
};
mod async_promise;
mod com_borrowed;
mod com_completion;
#[cfg(feature = "test-hooks")]
mod com_completion_test_hooks;
mod com_input;
#[cfg(feature = "test-hooks")]
mod generated_unsafe_test_hooks;
mod js_storage;
mod managed_tsfn;
mod scheduled_start;
mod win32;
mod win32_subsystem;
pub use win32::{
  DynWin32, DynWin32CallResult, DynWin32Function, DynWin32FunctionSpec, DynWin32NativeStruct,
  DynWin32ParameterSpec, DynWin32Resource, DynWin32Unsafe, DynWin32Value,
};
pub use win32_subsystem::DynWin32SubsystemContext;
mod winrt_delegate_method;
pub use winrt_delegate_method::DynWinRtDelegateMethod;
mod winrt_implementation;
pub use winrt_implementation::{
  DynWinRtImplementation, DynWinRtImplementationMethod, DynWinRtInterfacePlan,
};
#[cfg(feature = "test-hooks")]
mod tsfn_test_hooks;
#[cfg(feature = "test-hooks")]
mod winrt_implementation_test_hooks;

mod initialization;
pub use initialization::{get_winappsdk_resource_pri_path, init_winappsdk, ro_initialize};
pub(crate) use initialization::{set_winui_dispatcher_loop_active, winui_dispatcher_loop_exited};
mod js_numbers;
mod property_value;
pub use property_value::unbox_object;
mod winrt_types;
pub(crate) use winrt_types::TABLE;
pub use winrt_types::{DynWinRTType, WinGUID};
mod winrt_methods;
pub use winrt_methods::{raw_get_i32, raw_get_string, DynWinRTMethodHandle, DynWinRTMethodSig};
mod value;
pub(crate) use value::com_value;
pub use value::DynWinRTValue;
mod winrt_array;
pub use winrt_array::DynWinRTArray;
mod winrt_struct;
pub use winrt_struct::DynWinRTStruct;
mod system;
pub use system::{get_computer_name, get_windows_directory, has_package_identity};
mod benchmarks;
pub use benchmarks::{RustStaticBench, StaticObj, StaticUri};
mod direct_callback;
pub(crate) use direct_callback::{
  create_direct_callback_resources, invoke_direct_js_callback, napi_status, DirectJsCallback,
};
mod winrt_delegate;
pub use winrt_delegate::DynWinRtDelegate;
mod winrt_element_factory;
pub use winrt_element_factory::DynWinRtElementFactory;
#[cfg(test)]
mod js_boundary_tests;
