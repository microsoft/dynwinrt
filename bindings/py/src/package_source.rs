// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::path::PathBuf;

use pyo3::prelude::*;

/// Execute a Python source file shipped next to the native module into the
/// native module's globals.
///
/// Loads from the installed package, not an embedded string or a build-time
/// path, and keeps the native globals (including `__name__`) so the defined
/// types share the native module's public identity.
pub(crate) fn exec_package_source(
    module: &Bound<'_, PyModule>,
    file_name: &str,
    module_name: &str,
) -> PyResult<()> {
    let py = module.py();
    let source_path = module
        .getattr("__spec__")?
        .getattr("origin")?
        .extract::<PathBuf>()?
        .with_file_name(file_name);
    let source_path = py.import("os")?.call_method1("fspath", (source_path,))?;
    let loader = py
        .import("importlib.machinery")?
        .getattr("SourceFileLoader")?
        .call1((module_name, source_path))?;
    let source = loader.call_method1("get_code", (module_name,))?;
    py.import("builtins")?
        .call_method1("exec", (source, module.dict()))?;
    Ok(())
}
