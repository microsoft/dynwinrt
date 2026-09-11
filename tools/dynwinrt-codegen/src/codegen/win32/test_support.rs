// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use super::{GeneratedOutput, ir::ProjectedFunction, project_apis};
use crate::win32_metadata::{
    RawApis, RawArchitectures, RawBaseType, RawCallingConvention, RawConstness, RawDirection,
    RawFunction, RawParameter, RawScalar, RawStatusSemantics, RawType,
};

fn resolve_metadata(
    path: Option<String>,
    required: bool,
    is_file: impl FnOnce(&str) -> bool,
) -> Result<Option<String>, String> {
    match path.filter(|path| !path.trim().is_empty()) {
        Some(path) if is_file(&path) => Ok(Some(path)),
        Some(path) => Err(format!(
            "DYNWINRT_WIN32_WINMD is not a readable metadata file: {path}"
        )),
        None if required => {
            Err("DYNWINRT_REQUIRE_WIN32_METADATA=1 requires DYNWINRT_WIN32_WINMD".into())
        }
        None => Ok(None),
    }
}

pub(crate) fn metadata() -> Option<String> {
    resolve_metadata(
        std::env::var("DYNWINRT_WIN32_WINMD").ok(),
        std::env::var("DYNWINRT_REQUIRE_WIN32_METADATA").as_deref() == Ok("1"),
        |path| Path::new(path).is_file(),
    )
    .unwrap_or_else(|reason| panic!("{reason}"))
}

pub(crate) fn metadata_function(path: &str, namespace: &str, name: &str) -> RawFunction {
    crate::win32_metadata::parse_apis(path, namespace, "Apis")
        .unwrap_or_else(|| panic!("missing metadata container {namespace}.Apis"))
        .functions
        .into_iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("missing metadata function {namespace}.Apis::{name}"))
}

pub(crate) fn scalar(kind: RawScalar) -> RawType {
    RawType {
        base: RawBaseType::Scalar(kind),
        pointer_depth: 0,
        constness: RawConstness::Unspecified,
    }
}

pub(crate) fn pointer(mut typ: RawType, depth: u8, constness: RawConstness) -> RawType {
    typ.pointer_depth = depth;
    typ.constness = constness;
    typ
}

pub(crate) fn parameter(name: &str, typ: RawType, direction: RawDirection) -> RawParameter {
    RawParameter {
        name: name.into(),
        typ,
        direction,
        nullable: false,
        reserved: false,
        null_null_terminated: false,
        buffer: None,
        free_with: None,
    }
}

pub(crate) fn synthetic_function(name: &str) -> RawFunction {
    RawFunction {
        namespace: "Tests".into(),
        container: "Apis".into(),
        name: name.into(),
        dll: "kernel32.dll".into(),
        entry_point: name.into(),
        return_type: scalar(RawScalar::U32),
        parameters: Vec::new(),
        return_status: RawStatusSemantics::None,
        return_free_with: None,
        supports_last_error: false,
        calling_convention: RawCallingConvention::System,
        architectures: RawArchitectures {
            x86: true,
            x64: true,
            arm64: true,
        },
        variadic: false,
        evidence: None,
    }
}

pub(crate) fn apis(functions: Vec<RawFunction>) -> RawApis {
    RawApis {
        namespace: "Tests".into(),
        class_name: "Apis".into(),
        functions,
    }
}

pub(crate) fn project_one(function: RawFunction) -> ProjectedFunction {
    let projection = project_apis(&apis(vec![function]));
    assert!(projection.omitted.is_empty(), "{:?}", projection.omitted);
    assert_eq!(projection.projected.functions.len(), 1);
    projection.projected.functions.into_iter().next().unwrap()
}

pub(crate) fn semantic(function: &RawFunction) -> super::ir::FunctionContract {
    super::model::validate_function(function)
        .unwrap_or_else(|reason| panic!("{}: {reason}", function.name))
}

pub(crate) fn run_js(generated: &GeneratedOutput, setup: &str, checks: &str) {
    let script = format!(
        "{setup}\nconst projected = {{}}\nrequire('node:vm').runInNewContext({}, \
         {{ exports: projected, require: () => runtime, Buffer, BigInt }})\n{checks}",
        serde_json::to_string(&generated.js).unwrap(),
    );
    let mut child = Command::new("node")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node.js is required to exercise generated Win32 JavaScript");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn required_metadata_configuration_never_silently_skips() {
    for (path, required, present, expected) in [
        (None, false, false, Ok(None)),
        (None, true, false, Err(())),
        (Some(""), true, false, Err(())),
        (Some("missing.winmd"), false, false, Err(())),
        (Some("missing.winmd"), true, false, Err(())),
        (
            Some("official.winmd"),
            true,
            true,
            Ok(Some("official.winmd".to_owned())),
        ),
    ] {
        let result = resolve_metadata(path.map(str::to_owned), required, |_| present);
        assert_eq!(
            result.map_err(|_| ()),
            expected,
            "{path:?}, required={required}"
        );
    }
}
