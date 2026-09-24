// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::exceptions::asyncio::CancelledError as PyCancelledError;
use pyo3::exceptions::{PyIndexError, PyOSError, PyRuntimeError};
use pyo3::prelude::*;

#[cfg(test)]
pub(crate) static UNRAISABLE_HOOK_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RELEASED_REASON: &str = "its projected_lifetime_scope() exited or release_projected() \
     was called. Use the object inside its scope, or don't release it.";

/// A call on a value after `release()`, including release by its lifetime scope.
pub(crate) fn released_receiver_error() -> PyErr {
    PyRuntimeError::new_err(format!(
        "This WinRT object has been released: {RELEASED_REASON}"
    ))
}

/// A released value passed as argument `position` (0-based) of `operation`.
pub(crate) fn released_argument_error(operation: &str, position: usize) -> PyErr {
    PyRuntimeError::new_err(format!(
        "This WinRT object (argument {position} of {operation}) has been released: \
         {RELEASED_REASON}"
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
    let message = match context {
        Some(context) => format!("{context}: {}", error.message()),
        None => error.message(),
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
