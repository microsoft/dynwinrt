// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use windows::core::{GUID, IInspectable, IUnknown, Interface};

use crate::errors::{map_dynwinrt_error, map_windows_error};
use crate::runtime::{DynWinRTMethodSig, DynWinRTValue, WinGUID};

type DelegateCall =
    dyn Fn(&IUnknown, &[dynwinrt::WinRTValue]) -> windows::core::Result<Vec<dynwinrt::WinRTValue>>;

struct PreparedDelegateCall(Box<DelegateCall>);

// Constructed only below, capturing a fully built WinRT Method: immutable
// MethodInfo and a prepared libffi CIF, with no Python or target references.
// ffi_call reads that CIF; argument/output storage is local to each invocation.
// Its owned metadata/CIF allocations can also be destroyed on any thread.
unsafe impl Send for PreparedDelegateCall {}
unsafe impl Sync for PreparedDelegateCall {}

#[pyclass(frozen, module = "dynwinrt")]
pub struct DynWinRTDelegateMethod {
    iid: GUID,
    call: PreparedDelegateCall,
}

#[pymethods]
impl DynWinRTDelegateMethod {
    /// Prepare a metadata-described WinRT delegate Invoke signature once.
    #[staticmethod]
    pub(crate) fn create(iid: &WinGUID, signature: &DynWinRTMethodSig) -> PyResult<Self> {
        if iid.0 == GUID::zeroed() || iid.0 == IUnknown::IID || iid.0 == IInspectable::IID {
            return Err(PyTypeError::new_err(
                "delegate invocation requires a metadata-described delegate IID",
            ));
        }
        let method = signature.0.clone().build(3);
        Ok(Self {
            iid: iid.0,
            call: PreparedDelegateCall(Box::new(move |object, args| {
                method.call_dynamic(object.as_raw(), args)
            })),
        })
    }

    /// Return all outputs in signature order, or [] for a void Invoke method.
    pub(crate) fn invoke(
        &self,
        value: &Bound<'_, DynWinRTValue>,
        args: Vec<DynWinRTValue>,
    ) -> PyResult<Vec<DynWinRTValue>> {
        // Keep native pins, not a Python value borrow, across reentrant Invoke.
        let value = value.try_borrow()?.0.clone();
        let delegate = value.cast(&self.iid).map_err(map_dynwinrt_error)?;
        let dynwinrt::WinRTValue::Object(object) = &delegate else {
            return Err(PyTypeError::new_err(
                "delegate invocation requires a managed WinRT delegate value",
            ));
        };
        let args = args.into_iter().map(|arg| arg.0).collect::<Vec<_>>();
        (self.call.0)(object, &args)
            .map(|outputs| outputs.into_iter().map(DynWinRTValue).collect())
            .map_err(map_windows_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_delegate_method_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DynWinRTDelegateMethod>();
    }
}
