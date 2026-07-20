// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const VS_NODE: &str = r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\MSBuild\Microsoft\VisualStudio\NodeJs\node.exe";

#[test]
fn projected_lifetime_scope_is_inactive_until_created() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let node = if Path::new(VS_NODE).exists() {
        PathBuf::from(VS_NODE)
    } else {
        PathBuf::from("node")
    };
    let output =
        std::env::temp_dir().join(format!("dynwinrt-codegen-lifetime-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    let status = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--output",
        ])
        .arg(&output)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(status.success());

    let script = r#"
let weakRefCount = 0;
global.WeakRef = class {
  constructor(value) { weakRefCount += 1; this.value = value; }
  deref() { return this.value; }
};
const lifetime = require(process.argv[1]);
const unscoped = { release() { throw new Error('unscoped release'); } };
lifetime.trackProjectedValue(unscoped, 'Unscoped');
if (weakRefCount !== 0) throw new Error(`unscoped WeakRef count: ${weakRefCount}`);

const outer = lifetime.createProjectedLifetimeScope();
const first = { released: 0, release() { this.released += 1; } };
lifetime.trackProjectedValue(first, 'First');
if (weakRefCount !== 1) throw new Error(`scoped WeakRef count: ${weakRefCount}`);

const inner = lifetime.createProjectedLifetimeScope();
const second = { released: 0, release() { this.released += 1; } };
lifetime.trackProjectedValue(second, 'Second');
let lifoRejected = false;
try { outer.dispose(); } catch { lifoRejected = true; }
if (!lifoRejected) throw new Error('out-of-order scope disposal was accepted');
inner.dispose();
outer.dispose();
if (first.released !== 1 || second.released !== 1) throw new Error('values were not released once');
if (!outer.disposed || !inner.disposed) throw new Error('scopes were not marked disposed');

const retryScope = lifetime.createProjectedLifetimeScope();
const stable = { released: 0, release() { this.released += 1; } };
const flaky = {
  attempts: 0,
  release() {
    this.attempts += 1;
    if (this.attempts === 1) throw new Error('retry release');
  },
};
lifetime.trackProjectedValue(flaky, 'Flaky');
lifetime.trackProjectedValue(stable, 'Stable');
let releaseRejected = false;
try { retryScope.dispose(); } catch { releaseRejected = true; }
if (!releaseRejected) throw new Error('failed release was accepted');
if (retryScope.disposed) throw new Error('failed scope was marked disposed');
if (stable.released !== 1 || flaky.attempts !== 1) throw new Error('first release pass was incorrect');
retryScope.dispose();
if (!retryScope.disposed) throw new Error('retried scope was not disposed');
if (stable.released !== 1 || flaky.attempts !== 2) throw new Error('retry did not retain only failed values');
"#;
    let result = Command::new(node)
        .args(["-e", script])
        .arg(output.join("lifetime.js"))
        .output()
        .expect("run lifetime scope test");
    let _ = fs::remove_dir_all(&output);
    assert!(
        result.status.success(),
        "lifetime scope script failed:\n{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
