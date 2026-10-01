// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use dynwinrt_codegen::codegen::{project, projected::ProjectedFile, render_dts, render_js};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

fn method(name: &str, slot: usize, action: bool, parameter: bool) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index: slot,
        params: if parameter {
            vec![ParamMeta {
                name: "value".into(),
                typ: TypeMeta::U32,
                direction: ParamDirection::In,
            }]
        } else {
            Vec::new()
        },
        return_type: Some(if action {
            TypeMeta::AsyncActionWithProgress(Box::new(TypeMeta::U32))
        } else {
            TypeMeta::AsyncOperationWithProgress(Box::new(TypeMeta::U32), Box::new(TypeMeta::U32))
        }),
        ..Default::default()
    }
}

fn progress_probe() -> ProjectedFile {
    let class = ClassMeta {
        name: "ProgressProbe".into(),
        namespace: "Tests".into(),
        full_name: "Tests.ProgressProbe".into(),
        default_interface: Some(InterfaceMeta {
            name: "IProgressProbe".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
            methods: vec![
                method("RunOperation", 6, false, false),
                method("RunAction", 7, true, false),
                method("RunOverloadedOperation", 8, false, false),
                method("RunOverloadedOperation", 9, false, true),
                method("RunOverloadedAction", 10, true, false),
                method("RunOverloadedAction", 11, true, true),
                method("RunCrossOperation", 12, false, false),
                method("RunCrossAction", 13, true, false),
            ],
            ..Default::default()
        }),
        required_interfaces: vec![InterfaceMeta {
            name: "IProgressProbe2".into(),
            iid: "22222222-2222-2222-2222-222222222222".into(),
            methods: vec![
                method("RunCrossOperation", 6, false, true),
                method("RunCrossAction", 7, true, true),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    project::project_class(
        &Default::default(),
        &class,
        &HashSet::from(["ProgressProbe".into()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

#[test]
fn generated_progress_promises_register_native_completion_once() {
    let projected = progress_probe();
    let source = format!(
        "const generatedSource = {};\n{}",
        serde_json::to_string(&render_js::render(&projected)).unwrap(),
        include_str!("fixtures/javascript_progress_promise.cjs"),
    );
    let mut child = Command::new("node")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node is required for generated progress Promise execution tests");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn generated_progress_promise_declarations_pass_strict_tsc() {
    let tsc = std::env::var_os("DYNWINRT_TSC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(r"..\..\bindings\js\node_modules\typescript\bin\tsc")
        });
    if !tsc.is_file() {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_TSC").as_deref(),
            Ok("1"),
            "the generated progress Promise declarations require TypeScript"
        );
        eprintln!("Skipping: repository TypeScript compiler is unavailable");
        return;
    }
    let directory = std::env::temp_dir().join(format!(
        "dynwinrt-progress-promise-types-{}",
        std::process::id()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("ProgressProbe.d.ts"),
        render_dts::render(&progress_probe()),
    )
    .unwrap();
    let runtime = directory
        .join("node_modules")
        .join("@microsoft")
        .join("dynwinrt");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(
        runtime.join("package.json"),
        r#"{"name":"@microsoft/dynwinrt","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        runtime.join("index.d.ts"),
        "export declare class WinGuid {}\nexport declare class DynWinRtValue {}\nexport declare class DynWinRtType {}\n",
    )
    .unwrap();
    fs::write(
        directory.join("consumer.ts"),
        r#"import { ProgressProbe } from './ProgressProbe.js';
declare const probe: ProgressProbe;
declare const signal: AbortSignal;

async function consume() {
    const operation = probe.runOperation(signal);
    const number: number = await operation;
    const promise: Promise<number> = operation.toPromise();
    const chained: typeof operation = operation.progress(value => { const n: number = value; });
    const together: number[] = await Promise.all([operation, promise, chained.toPromise()]);
    const action: Promise<void> = probe.runAction(signal).progress(value => { const n: number = value; }).toPromise();
    const nothing: void = await action;
    const short: Promise<number> = probe.runOverloadedOperation(signal).toPromise();
    const long: Promise<number> = probe.runOverloadedOperation(7, signal).toPromise();
    const shortAction: Promise<void> = probe.runOverloadedAction(signal).toPromise();
    const longAction: Promise<void> = probe.runOverloadedAction(7, signal).toPromise();
    const cross: Promise<number> = probe.runCrossOperation(7, signal).toPromise();
    const crossAction: Promise<void> = probe.runCrossAction(7, signal).toPromise();
    operation.cancel();
    // @ts-expect-error Progress is projected to a number, not a raw native value.
    operation.progress((value: string) => {});
    // @ts-expect-error Action-with-progress resolves to void.
    const invalid: Promise<number> = probe.runAction().toPromise();
}
"#,
    )
    .unwrap();
    let output = Command::new("node")
        .arg(tsc)
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "ES2022",
            "--module",
            "Node16",
            "--moduleResolution",
            "Node16",
            "consumer.ts",
        ])
        .current_dir(&directory)
        .output()
        .expect("run TypeScript on generated progress Promise declarations");
    fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
