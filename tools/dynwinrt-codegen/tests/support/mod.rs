// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{fs, path::Path};

// Declaration tests run before the native addon is built in coverage CI.
pub fn write_com_runtime_stub(output: &Path) {
    fs::write(
        output.join("globals.d.ts"),
        "declare class Buffer extends Uint8Array {}\n",
    )
    .expect("write Buffer stub");
    let package = output
        .join("node_modules")
        .join("@microsoft")
        .join("dynwinrt");
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
}
