// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use dynwinrt_codegen::{codegen::com, com_metadata};
use std::{fs, path::Path, process::Command};

mod support;

#[test]
fn ambiguous_native_overloads_expose_exact_callable_slots() {
    let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        return;
    };
    for (namespace, interface_name, method_name, camel_name) in [
        (
            "Windows.Win32.Graphics.Direct2D",
            "ID2D1Device1",
            "CreateDeviceContext",
            "createDeviceContext",
        ),
        (
            "Windows.Win32.Graphics.Direct2D",
            "ID2D1Device7",
            "CreateDeviceContext",
            "createDeviceContext",
        ),
        (
            "Windows.Win32.Graphics.DirectWrite",
            "IDWriteFontCollection1",
            "GetFontFamily",
            "getFontFamily",
        ),
        (
            "Windows.Win32.AI.MachineLearning.WinML",
            "IMLOperatorKernelContext",
            "GetOutputTensor",
            "getOutputTensor",
        ),
        (
            "Windows.Win32.Graphics.DirectComposition",
            "IDCompositionDynamicTexture",
            "SetTexture",
            "setTexture",
        ),
    ] {
        let interface =
            com_metadata::parse_com_interface(&winmd, namespace, interface_name).unwrap();
        let output = com::generate_com_interface_files(&interface, &winmd)
            .unwrap_or_else(|error| panic!("{interface_name}: {error}"));
        let slots = interface
            .raw_methods
            .as_ref()
            .unwrap()
            .iter()
            .filter(|method| method.metadata_name == method_name)
            .map(|method| method.vtable_index)
            .collect::<Vec<_>>();
        assert!(
            slots.len() >= 2,
            "{interface_name} must exercise inherited overloads"
        );
        for slot in slots {
            let name = format!("{camel_name}AtSlot{slot}");
            assert!(
                output.dts.contains(&format!("{name}(")),
                "{interface_name}: missing {name}"
            );
            assert!(
                output.js.contains(&format!("{name}(")),
                "{interface_name}: missing {name}"
            );
            assert!(
                output.js.contains(&format!(".method({slot}).invoke")),
                "{interface_name}: lost slot {slot}"
            );
            assert!(
                output
                    .js
                    .contains(&format!(".addMethodAt({slot}, '{method_name}'")),
                "{interface_name}: registration changed"
            );
        }

        assert!(!output.dts.contains(&format!("    {camel_name}(")));
        assert!(!output.js.contains(&format!("    {camel_name}(")));
    }
}

#[test]
fn generated_explicit_overload_declarations_typecheck() {
    let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
        return;
    };
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(r"bindings\js");
    let tsc = package.join(r"node_modules\typescript\bin\tsc");
    if !tsc.is_file() {
        eprintln!("Skipping: repository TypeScript compiler not available");
        return;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output = std::env::temp_dir().join(format!(
        "dynwinrt-com-overloads-{}-{nonce}",
        std::process::id()
    ));
    let generation = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
                .args(["generate", "--winmd", &winmd, "--class-name",
                    "Windows.Win32.Graphics.Direct2D.ID2D1Device1,Windows.Win32.Graphics.DirectWrite.IDWriteFontCollection1",
                    "--output"])
                .arg(&output)
                .output()
                .expect("run overload codegen");
    assert!(
        generation.status.success(),
        "{}",
        String::from_utf8_lossy(&generation.stderr)
    );
    let device = com_metadata::parse_com_interface(
        &winmd,
        "Windows.Win32.Graphics.Direct2D",
        "ID2D1Device1",
    )
    .unwrap();
    let slots = device
        .raw_methods
        .as_ref()
        .unwrap()
        .iter()
        .filter(|method| method.metadata_name == "CreateDeviceContext")
        .map(|method| method.vtable_index)
        .collect::<Vec<_>>();
    let calls = slots
        .iter()
        .map(|slot| format!("device.createDeviceContextAtSlot{slot}(0);"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        output.join("usage.ts"),
        format!(
            "import type {{ ID2D1Device1 }} from './com/index.js';\n\
                 declare const device: ID2D1Device1;\n{calls}\n\
                 // @ts-expect-error ambiguous unsuffixed overload is intentionally absent\n\
                 device.createDeviceContext(0);\n\
                 // @ts-expect-error the native enum input remains typed\n\
                 device.createDeviceContextAtSlot{}('invalid');\n",
            slots[0]
        ),
    )
    .unwrap();
    support::write_com_runtime_stub(&output);
    let config = serde_json::json!({
        "compilerOptions": {
            "target": "ES2022", "module": "Node16", "moduleResolution": "Node16",
            "strict": true, "noEmit": true, "skipLibCheck": false,
            "types": []
        },
        "include": ["globals.d.ts", "usage.ts", "com/**/*.d.ts"]
    });
    fs::write(
        output.join("tsconfig.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let checked = Command::new("node")
        .arg(tsc)
        .args(["--noEmit", "-p"])
        .arg(output.join("tsconfig.json"))
        .output()
        .expect("run overload tsc");
    fs::remove_dir_all(&output).unwrap();
    assert!(
        checked.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
}
