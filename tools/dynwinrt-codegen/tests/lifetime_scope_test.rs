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
global.WeakRef = class {
  constructor() { throw new Error('Projection scopes must retain strong references.'); }
};
const lifetime = require(process.argv[1]);
const unscoped = { release() { throw new Error('unscoped release'); } };
lifetime.trackProjectedValue(unscoped, 'Unscoped');

const raw = { released: 0, release() { this.released += 1; } };
const projectedRaw = { released: 0, release() { this.released += 1; } };
const RuntimeClass = {
  _fromNativeBorrowed(value) {
    if (value !== raw) throw new Error('projectAs passed the wrong raw value');
    return { _obj: projectedRaw };
  },
};
const projected = lifetime.projectAs(raw, RuntimeClass);
if (raw.released !== 0) throw new Error('projectAs consumed the raw value');
lifetime.releaseProjected(projected);
lifetime.releaseProjected(projected);
if (projectedRaw.released !== 2) throw new Error('releaseProjected was not repeatable');

const transferredRaw = { released: 0, release() { this.released += 1; } };
const transferred = lifetime.projectAs(transferredRaw, {
  _fromNativeBorrowed(value) { return { _obj: value }; },
});
if (transferredRaw.released !== 0) throw new Error('projectAs released a transferred raw value');
lifetime.releaseProjected(transferred);
if (transferredRaw.released !== 1) throw new Error('releaseProjected did not release a transferred value');

const existingRaw = { released: 0, release() { this.released += 1; } };
const existing = { _obj: existingRaw };
const reprojected = lifetime.projectAs(existing, {
  _fromNativeBorrowed(value) { return { _obj: value }; },
});
if (reprojected._obj !== existingRaw || existingRaw.released !== 0) {
  throw new Error('projectAs consumed an existing wrapper');
}

const failedRaw = { released: 0, release() { this.released += 1; } };
let projectionRejected = false;
try {
  lifetime.projectAs(failedRaw, {
    _fromNativeBorrowed() { throw new Error('projection failed'); },
  });
} catch (error) {
  projectionRejected = error.message === 'projection failed';
}
if (!projectionRejected || failedRaw.released !== 0) {
  throw new Error('projectAs consumed a failed raw projection');
}

const invalidTargetRaw = { released: 0, release() { this.released += 1; } };
let invalidTargetRejected = false;
try { lifetime.projectAs(invalidTargetRaw, {}); }
catch (error) { invalidTargetRejected = error instanceof TypeError; }
if (!invalidTargetRejected || invalidTargetRaw.released !== 0) {
  throw new Error('projectAs consumed an invalid target value');
}

let invalidReleaseRejected = false;
try { lifetime.releaseProjected({}); }
catch (error) { invalidReleaseRejected = error instanceof TypeError; }
if (!invalidReleaseRejected) throw new Error('releaseProjected accepted an invalid wrapper');

const outer = lifetime.createProjectedLifetimeScope();
const castSource = {
  released: 0,
  release() { this.released += 1; },
  cast(iid) {
    if (iid !== 'iid') throw new Error('wrong iid');
    return castResult;
  },
};
const castResult = { released: 0, release() { this.released += 1; } };
const castValue = lifetime.castProjectedValue(castSource, 'iid', 'Cast');
if (castValue !== castResult || castSource.released !== 1) {
  throw new Error('castProjectedValue did not consume the source value');
}
const borrowedCastSource = {
  released: 0,
  release() { this.released += 1; },
  cast() { return castResult; },
};
if (lifetime.castProjectedValueBorrowed(borrowedCastSource, 'iid', 'BorrowedCast') !== castResult) {
  throw new Error('castProjectedValueBorrowed returned the wrong value');
}
if (borrowedCastSource.released !== 0) {
  throw new Error('castProjectedValueBorrowed consumed the source value');
}
const first = { released: 0, release() { this.released += 1; } };
lifetime.trackProjectedValue(first, 'First');
const firstWrapper = { _obj: first };
lifetime.releaseProjected(firstWrapper);

const inner = lifetime.createProjectedLifetimeScope();
const second = { released: 0, release() { this.released += 1; } };
lifetime.trackProjectedValue(second, 'Second');
let lifoRejected = false;
try { outer.dispose(); } catch { lifoRejected = true; }
if (!lifoRejected) throw new Error('out-of-order scope disposal was accepted');
inner.dispose();
outer.dispose();
if (first.released !== 1 || second.released !== 1 || castResult.released !== 1) {
  throw new Error('values were not released once');
}
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
