// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use pyo3::{exceptions::PyRuntimeError, prelude::*};
use windows::core::HSTRING;

use crate::errors::map_windows_error;

#[pyfunction]
pub(crate) fn _test_winui_owned_references() -> usize {
    crate::runtime::WINUI_MODULES.owned_reference_count()
}

#[pyfunction]
pub(crate) fn _test_winui_module_paths() -> PyResult<Vec<String>> {
    crate::runtime::WINUI_MODULES
        .retained_module_paths()
        .map(|paths| {
            paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect()
        })
        .map_err(map_windows_error)
}

#[pyfunction]
pub(crate) fn _test_winui_with_mta_runtime(
    py: Python<'_>,
    callback: Py<PyAny>,
) -> PyResult<Vec<(bool, u32)>> {
    let start = crate::async_runtime::MTA_THREAD_EVENTS
        .lock()
        .unwrap()
        .len();
    let runtime = crate::async_runtime::runtime_builder()
        .worker_threads(2)
        .build()
        .map_err(|error| {
            PyRuntimeError::new_err(format!("test runtime creation failed: {error}"))
        })?;

    fn use_winrt() -> windows::core::Result<bool> {
        let uri = windows::Foundation::Uri::CreateUri(&HSTRING::from("https://example.com/"))?;
        Ok(uri.Host()? == "example.com")
    }

    let async_job = runtime.spawn(async { use_winrt() });
    let blocking_job = runtime.spawn_blocking(use_winrt);
    let prepared = py.detach(|| {
        runtime.block_on(async {
            let async_result = async_job.await;
            let blocking_result = blocking_job.await;
            for result in [async_result, blocking_result] {
                if !result
                    .map_err(|error| PyRuntimeError::new_err(format!("test job failed: {error}")))?
                    .map_err(map_windows_error)?
                {
                    return Err(PyRuntimeError::new_err(
                        "test worker returned an invalid URI",
                    ));
                }
            }
            Ok(())
        })
    });
    let result = prepared.and_then(|()| callback.call0(py));
    // Use the production apartment hooks but a fixture-owned runtime so its
    // actual async and blocking threads can all be stopped, not timeout-leaked.
    py.detach(|| drop(runtime));
    result?;
    Ok(crate::async_runtime::MTA_THREAD_EVENTS.lock().unwrap()[start..].to_vec())
}
