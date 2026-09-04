// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn generated_com_sink_dts_passes_tsc_no_emit() {
    let winmd = std::env::var("DYNWINRT_WIN32_WINMD")
        .unwrap_or_else(|_| r"C:\s\win32metadata\Windows.Win32.winmd".into());
    if !Path::new(&winmd).exists() {
        eprintln!("Skipping: Windows.Win32.winmd not found");
        return;
    }
    let tsc =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\bindings\js\node_modules\.bin\tsc.cmd");
    let tsc_check = Command::new("cmd")
        .arg("/c")
        .arg(&tsc)
        .arg("--version")
        .output();
    if !matches!(tsc_check, Ok(output) if output.status.success()) {
        eprintln!("Skipping: repository TypeScript compiler not available");
        return;
    }

    let exe = env!("CARGO_BIN_EXE_dynwinrt-codegen");
    let tmp = std::env::temp_dir().join(format!(
        "dynwinrt-codegen-com-sink-tsc-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create COM sink temp dir");
    let status = Command::new(exe)
        .args([
            "generate",
            "--winmd",
            &winmd,
            "--namespace",
            "Windows.Win32.UI.Shell",
            "--class-name",
            "FileOpenDialog,IFileDialogEvents,TaskbarList",
            "--output",
        ])
        .arg(&tmp)
        .status()
        .expect("spawn COM sink codegen");
    assert!(status.success(), "COM sink codegen failed: {status:?}");
    let status = Command::new(exe)
        .args([
            "generate",
            "--winmd",
            &winmd,
            "--class-name",
            "Windows.Win32.System.Ole.IDropTarget",
            "--output",
        ])
        .arg(&tmp)
        .status()
        .expect("spawn IDropTarget codegen");
    assert!(
        status.success(),
        "IDropTarget sink codegen failed: {status:?}"
    );

    let com_dir = tmp.join("com");
    fs::write(
        com_dir.join("sink-usage.ts"),
        r#"import { FileOpenDialog } from "./windows/win32/ui/shell/FileOpenDialog.js";
import {
          DROPEFFECT,
          FDE_OVERWRITE_RESPONSE,
          FDE_SHAREVIOLATION_RESPONSE,
          IFileDialogEvents,
          MODIFIERKEYS_FLAGS,
} from "./index.js";
import type { IFileDialogEventsImplementation } from "./windows/win32/ui/shell/IFileDialogEvents.js";
import {
  createPOINTL,
  IDropTarget,
  type IDropTargetImplementation,
  type POINTL,
} from "./windows/win32/system/ole/IDropTarget.js";

const dialog = new FileOpenDialog();
const eventHandlers: IFileDialogEventsImplementation = {
  onFileOk(fileDialog) {
    fileDialog.isNull();
    return 0;
  },
  onFolderChanging() {},
  onFolderChange() {},
  onSelectionChange() {},
  onShareViolation(fileDialog, item) {
    fileDialog.isNull();
    item.isNull();
    return FDE_SHAREVIOLATION_RESPONSE.FDESVR_REFUSE;
  },
  onTypeChange() {},
  onOverwrite() {
    return FDE_OVERWRITE_RESPONSE.FDEOR_DEFAULT;
  },
};
const events = IFileDialogEvents.implement(eventHandlers);
const cookie = dialog.advise(events.nativeValue);
dialog.unadvise(cookie);
events.release();
dialog.release();

const dropHandlers: IDropTargetImplementation = {
  dragEnter(dataObject, keyState, point, effect) {
    dataObject.isNull();
    const values: [MODIFIERKEYS_FLAGS, POINTL, DROPEFFECT] = [keyState, point, effect];
    return values[2];
  },
  dragOver(keyState, point, effect) {
    const values: [MODIFIERKEYS_FLAGS, POINTL, DROPEFFECT] = [keyState, point, effect];
    return values[2];
  },
  dragLeave() {},
  drop(dataObject, keyState, point, effect) {
    dataObject.isNull();
    const values: [MODIFIERKEYS_FLAGS, POINTL, DROPEFFECT] = [keyState, point, effect];
    return values[2];
  },
};
const dropTarget = IDropTarget.implement(dropHandlers);
declare const point: ReturnType<typeof createPOINTL>;
const effect = dropTarget.dragOver(
  MODIFIERKEYS_FLAGS.MK_CONTROL,
  point,
  DROPEFFECT.DROPEFFECT_COPY,
);
effect satisfies DROPEFFECT;
dropTarget.release();
const composed = IDropTarget.implement(
  dropHandlers,
  IFileDialogEvents.implementation(eventHandlers),
);
const composedEvents = composed.as(IFileDialogEvents);
composedEvents.release();
composed.release();
"#,
    )
    .expect("write COM sink usage");
    fs::write(
        tmp.join("globals.d.ts"),
        "declare class Buffer extends Uint8Array {}\n",
    )
    .expect("write Buffer stub");
    fs::write(
        tmp.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": false,
    "types": []
  },
  "include": ["globals.d.ts", "com/**/*.d.ts", "com/*.ts"]
}"#,
    )
    .expect("write COM sink tsconfig");

    let package = tmp.join("node_modules").join("@microsoft").join("dynwinrt");
    fs::create_dir_all(&package).expect("create COM runtime stub");
    fs::write(
        package.join("package.json"),
        r#"{
  "name": "@microsoft/dynwinrt",
  "version": "0.0.0",
  "exports": {
    "./com": {
      "types": "./com.d.ts"
    }
  }
}"#,
    )
    .expect("write COM runtime package stub");
    fs::write(
        package.join("com.d.ts"),
        r#"export declare class WinGuid {}
export interface DynComImplementation {}
export declare class DynWinRtValue {
  isNull(): boolean;
}
export declare class DynComNativeStruct {}
export declare class DynComNativeStructArray {}
"#,
    )
    .expect("write COM runtime declarations");

    let output = Command::new("cmd")
        .arg("/c")
        .arg(&tsc)
        .args(["--noEmit", "-p"])
        .arg(tmp.join("tsconfig.json"))
        .current_dir(&tmp)
        .output()
        .expect("spawn COM sink tsc");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = fs::remove_dir_all(&tmp);
    assert!(
        output.status.success(),
        "tsc --noEmit failed on generated COM sink declarations!\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
