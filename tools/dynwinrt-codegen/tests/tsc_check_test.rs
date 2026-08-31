// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Verify that generated `.d.ts` files are valid TypeScript declarations by
//! running `tsc --noEmit` against them. This catches declaration-level bugs
//! like missing imports, invalid syntax, or type reference errors that would
//! break IntelliSense and type-checking for consumers.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

#[test]
fn generated_dts_passes_tsc_no_emit() {
    let require_tsc = std::env::var("DYNWINRT_REQUIRE_TSC").as_deref() == Ok("1");
    let windows_winmd = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WINDOWS_WINMD));
    if !windows_winmd.is_file() {
        assert!(
            !require_tsc,
            "DYNWINRT_REQUIRE_TSC=1 but Windows metadata is missing at {}",
            windows_winmd.display()
        );
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }

    let tsc = std::env::var_os("DYNWINRT_TSC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../bindings/js/node_modules/typescript/bin/tsc")
        });
    if !tsc.is_file() {
        assert!(
            !require_tsc,
            "DYNWINRT_REQUIRE_TSC=1 but TypeScript compiler is missing at {}",
            tsc.display()
        );
        eprintln!(
            "Skipping: TypeScript compiler is missing at {}",
            tsc.display()
        );
        return;
    }
    let tsc_check = Command::new("node").arg(&tsc).arg("--version").output();
    match tsc_check {
        Ok(output) if output.status.success() => {}
        result => {
            assert!(
                !require_tsc,
                "DYNWINRT_REQUIRE_TSC=1 but node could not run {}: {result:?}",
                tsc.display()
            );
            eprintln!("Skipping: node could not run {}", tsc.display());
            return;
        }
    }

    let exe = env!("CARGO_BIN_EXE_dynwinrt-codegen");
    let tmp =
        std::env::temp_dir().join(format!("dynwinrt-codegen-tsc-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create tmp dir");

    // Generate Uri (covers class, interface, struct, enum)
    let status = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Foundation",
            "--class-name",
            "Uri",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .arg("--winmd")
        .arg(&windows_winmd)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(status.success(), "codegen failed (Uri): {:?}", status);

    // Generate StorageFile (covers async methods, delegate params, required interfaces)
    let status2 = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.Storage",
            "--class-name",
            "StorageFile",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .arg("--winmd")
        .arg(&windows_winmd)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(
        status2.success(),
        "codegen failed (StorageFile): {:?}",
        status2
    );

    // Generate User to cover a system-returned class with no public constructor.
    let status3 = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.System",
            "--class-name",
            "User",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .arg("--winmd")
        .arg(&windows_winmd)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(status3.success(), "codegen failed (User): {:?}", status3);

    let status4 = Command::new(exe)
        .args([
            "generate",
            "--namespace",
            "Windows.ApplicationModel.Contacts",
            "--class-name",
            "ContactDate",
            "--lang",
            "js",
            "--output",
        ])
        .arg(&tmp)
        .arg("--winmd")
        .arg(&windows_winmd)
        .status()
        .expect("spawn dynwinrt-codegen");
    assert!(
        status4.success(),
        "codegen failed (ContactDate): {:?}",
        status4
    );

    fs::write(
        tmp.join("constructor-usage.ts"),
        r#"import { Uri } from "./windows/foundation/Uri.js";
import { User } from "./windows/system/User.js";
import { ContactDate } from "./windows/application-model/contacts/ContactDate.js";
import { IReference_UInt32 } from "./windows/foundation/IReference_UInt32.js";

new Uri("https://example.com");
new Uri("https://example.com/base/", "child");
// @ts-expect-error Uri has no zero-argument activation.
new Uri();
// @ts-expect-error User instances can only be returned by the system.
new User();

const contactDate = ContactDate.create();
const day: number | null = contactDate.day;
contactDate.day = 17;
contactDate.day = null;
declare const legacyDay: IReference_UInt32;
contactDate.day = legacyDay;
// @ts-expect-error Nullable UInt32 properties do not accept strings.
contactDate.day = "17";
"#,
    )
    .expect("write constructor usage");

    // Write a minimal tsconfig.json for tsc --noEmit
    let tsconfig = tmp.join("tsconfig.json");
    fs::write(
        &tsconfig,
        r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "strict": false,
    "noEmit": true,
    "skipLibCheck": false,
    "types": []
  },
  "include": ["*.d.ts", "*.ts"]
}"#,
    )
    .expect("write tsconfig");

    // Write a stub for @microsoft/dynwinrt types (the real package isn't installed here)
    let node_modules = tmp.join("node_modules").join("@microsoft").join("dynwinrt");
    fs::create_dir_all(&node_modules).expect("create node_modules stub");
    fs::write(
        node_modules.join("package.json"),
        r#"{"name":"@microsoft/dynwinrt","version":"0.0.0","types":"index.d.ts"}"#,
    )
    .expect("write stub package.json");
    fs::write(node_modules.join("index.d.ts"), r#"
export declare class DynWinRtType {
    static registerInterface(name: string, iid: WinGuid): DynWinRtType;
    addMethod(name: string, sig: DynWinRtMethodSig): DynWinRtType;
    static parameterized(iid: WinGuid, args: DynWinRtType[]): DynWinRtType;
    iid(): WinGuid;
    static runtimeClass(name: string, defaultInterfaceType: DynWinRtType): DynWinRtType;
    static hstring(): DynWinRtType;
    static i32(): DynWinRtType;
    static u32(): DynWinRtType;
    static i64(): DynWinRtType;
    static u64(): DynWinRtType;
    static f32(): DynWinRtType;
    static f64(): DynWinRtType;
    static bool(): DynWinRtType;
    static object(): DynWinRtType;
    static enumType(name: string, names: string[], values: number[]): DynWinRtType;
    static structType(name: string, fields: DynWinRtType[]): DynWinRtType;
    static iAsyncOperation(inner: DynWinRtType): DynWinRtType;
    static iAsyncAction(): DynWinRtType;
    [key: string]: any;
}
export declare class DynWinRtMethodSig {
    addIn(t: DynWinRtType): DynWinRtMethodSig;
    addOut(t: DynWinRtType): DynWinRtMethodSig;
    addOutFill(t: DynWinRtType): DynWinRtMethodSig;
    [key: string]: any;
}
export declare class DynWinRtValue {
    toNumber(): number;
    toI64(): number;
    toI64Bigint(): bigint;
    toU64Bigint(): bigint;
    toF64(): number;
    toBool(): boolean;
    toString(): string;
    asArray(): DynWinRtArray;
    asStruct(): DynWinRtStruct;
    cast(iid: WinGuid): DynWinRtValue;
    toPromise(): Promise<DynWinRtValue>;
    cancel(): void;
    onProgress(cb: (v: DynWinRtValue) => void): void;
    static activationFactory(name: string): DynWinRtValue;
    static hstring(s: string): DynWinRtValue;
    static fromI32(n: number): DynWinRtValue;
    static fromBool(b: boolean): DynWinRtValue;
    static createVector(items: DynWinRtValue[], elemType: DynWinRtType): DynWinRtValue;
    static createMap(keys: DynWinRtValue[], values: DynWinRtValue[], keyType: DynWinRtType, valueType: DynWinRtType): DynWinRtValue;
    [key: string]: any;
}
export declare class DynWinRtArray {
    getAt(index: number): DynWinRtValue;
    length: number;
    [key: string]: any;
}
export declare class DynWinRtStruct {
    getField(index: number): DynWinRtValue;
    static create(type_: DynWinRtType): DynWinRtStruct;
    setField(index: number, value: DynWinRtValue): void;
    toValue(): DynWinRtValue;
    [key: string]: any;
}
export declare class DynWinRtDelegate {
    static create(iid: WinGuid, paramTypes: DynWinRtType[], callback: (...args: any[]) => void): DynWinRtDelegate;
    toValue(): DynWinRtValue;
    [key: string]: any;
}
export declare class WinGuid {
    static parse(s: string): WinGuid;
    [key: string]: any;
}
"#).expect("write stub index.d.ts");

    let tsc_output = Command::new("node")
        .arg(&tsc)
        .args(["--noEmit", "-p", tsconfig.to_str().unwrap()])
        .current_dir(&tmp)
        .output()
        .expect("spawn tsc");

    let stdout = String::from_utf8_lossy(&tsc_output.stdout);
    let stderr = String::from_utf8_lossy(&tsc_output.stderr);

    let _ = fs::remove_dir_all(&tmp);

    assert!(
        tsc_output.status.success(),
        "tsc --noEmit failed on generated .d.ts files!\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}
