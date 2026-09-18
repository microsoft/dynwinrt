// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[test]
#[cfg(windows)]
fn focused_e2e_uses_shared_build_and_interpreter_preparation() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("e2e")
        .join("e2e_preparation.tests.ps1");
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(script)
        .output()
        .expect("run the existing PowerShell E2E infrastructure");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
