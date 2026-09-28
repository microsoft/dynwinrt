// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::exceptions::asyncio::CancelledError as PyCancelledError;
use pyo3::exceptions::{PyIndexError, PyOSError, PyRuntimeError};
use pyo3::prelude::*;
use windows::Win32::Foundation::CO_E_NOTINITIALIZED;
use windows::core::HRESULT;

#[cfg(test)]
pub(crate) static UNRAISABLE_HOOK_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guidance appended to the Windows description of specific HRESULTs.
const HRESULT_HINTS: &[(HRESULT, &str)] = &[(
    CO_E_NOTINITIALIZED,
    "WinRT is not initialized on this thread; use `with dynwinrt.RoApartment():` \
     (or call `dynwinrt.ro_initialize(dynwinrt.RO_INIT_MULTITHREADED)`) before calling \
     WinRT APIs.",
)];

fn hresult_hint(code: HRESULT) -> Option<&'static str> {
    HRESULT_HINTS
        .iter()
        .find_map(|&(hinted, hint)| (hinted == code).then_some(hint))
}

const RELEASED_REASON: &str = "has been released (its projected_lifetime_scope() exited, or \
     release_projected() / DynWinRTValue.release() was called) and can no longer be used.";

/// A call on a value after `release()`, including release by its lifetime scope.
pub(crate) fn released_receiver_error() -> PyErr {
    PyRuntimeError::new_err(format!("This WinRT object {RELEASED_REASON}"))
}

/// Where a value was handed to native code, with a 0-based index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputSlot {
    Argument(usize),
    Element(usize),
    Key(usize),
    Value(usize),
    Field(usize),
    /// A value a Python callback returned to its native caller.
    Output(usize),
}

impl std::fmt::Display for InputSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (slot, index) = match *self {
            Self::Argument(index) => ("argument", index),
            Self::Element(index) => ("element", index),
            Self::Key(index) => ("key", index),
            Self::Value(index) => ("value", index),
            Self::Field(index) => ("field", index),
            Self::Output(index) => ("output", index),
        };
        write!(f, "{slot} {index}")
    }
}

/// A released value passed in `slot` of `operation`.
pub(crate) fn released_input_error(operation: &str, slot: InputSlot) -> PyErr {
    PyRuntimeError::new_err(format!(
        "This WinRT object ({slot} of {operation}) {RELEASED_REASON}"
    ))
}

/// A live value of `kind` used where `operation` needs a WinRT object.
pub(crate) fn non_object_receiver_error(operation: &str, kind: &str) -> PyErr {
    PyRuntimeError::new_err(format!("{operation} requires an Object value, got {kind}"))
}

pub(crate) fn map_windows_error(error: windows::core::Error) -> PyErr {
    windows_error(error, None)
}

pub(crate) fn map_windows_error_with_context(error: windows::core::Error, context: &str) -> PyErr {
    windows_error(error, Some(context))
}

fn windows_error(error: windows::core::Error, context: Option<&str>) -> PyErr {
    let description = match context {
        Some(context) => format!("{context}: {}", error.message()),
        None => error.message(),
    };
    // A hint explains how to fix the failure; it never changes the error.
    let message = match hresult_hint(error.code()) {
        Some(hint) => match description.trim_end() {
            "" => hint.to_owned(),
            text => format!("{text} {hint}"),
        },
        None => description,
    };
    // Match PyWinRT's OSError shape and preserve the signed HRESULT in winerror.
    PyOSError::new_err((0, message, Option::<String>::None, error.code().0))
}

pub(crate) fn map_dynwinrt_error(error: dynwinrt::Error) -> PyErr {
    match error {
        dynwinrt::Error::WindowsError(error) => map_windows_error(error),
        dynwinrt::Error::Canceled => {
            PyCancelledError::new_err("WinRT async operation was canceled")
        }
        dynwinrt::Error::IndexOutOfBounds { index, len } => {
            PyIndexError::new_err(format!("Index {index} out of bounds (len {len})"))
        }
        other => PyRuntimeError::new_err(other.message()),
    }
}

pub(crate) fn map_dynwinrt_error_with_context(error: dynwinrt::Error, context: &str) -> PyErr {
    match error {
        dynwinrt::Error::WindowsError(error) => map_windows_error_with_context(error, context),
        dynwinrt::Error::Canceled => {
            PyCancelledError::new_err("WinRT async operation was canceled")
        }
        other => PyRuntimeError::new_err(format!("{context}: {}", other.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::E_POINTER;

    fn os_error_fields(py: Python<'_>, error: PyErr) -> (i32, i32, String) {
        let value = error.value(py);
        assert!(value.is_instance_of::<PyOSError>());
        (
            value.getattr("winerror").unwrap().extract().unwrap(),
            value.getattr("errno").unwrap().extract().unwrap(),
            value.getattr("strerror").unwrap().extract().unwrap(),
        )
    }

    #[test]
    fn hinted_hresults_keep_their_os_error_and_append_guidance() {
        Python::initialize();
        Python::attach(|py| {
            let hint = hresult_hint(CO_E_NOTINITIALIZED).expect("CO_E_NOTINITIALIZED hint");
            let not_initialized = windows::core::Error::from_hresult(CO_E_NOTINITIALIZED);
            let (winerror, errno, message) =
                os_error_fields(py, map_windows_error(not_initialized.clone()));
            assert_eq!(winerror, CO_E_NOTINITIALIZED.0);
            assert_eq!(errno, 22);
            assert!(message.ends_with(hint), "{message}");
            assert_ne!(message, hint, "the Windows description must remain");

            let (_, _, message) = os_error_fields(
                py,
                map_dynwinrt_error_with_context(
                    dynwinrt::Error::WindowsError(not_initialized),
                    "activation failed",
                ),
            );
            assert!(message.starts_with("activation failed: "), "{message}");
            assert!(message.ends_with(hint), "{message}");

            assert_eq!(hresult_hint(E_POINTER), None);
            let (winerror, _, message) = os_error_fields(
                py,
                map_dynwinrt_error(dynwinrt::Error::WindowsError(E_POINTER.into())),
            );
            assert_eq!(winerror, E_POINTER.0);
            assert!(!message.contains("RoApartment"), "{message}");
        });
    }
}
