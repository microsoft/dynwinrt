// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{fs, path::PathBuf, process::Command};

use dynwinrt_codegen::codegen::winrt::javascript::parameterized_name;
use dynwinrt_codegen::types::TypeMeta;
use windows_metadata::{
    MethodAttributes, MethodCallAttributes, MethodImplAttributes, ParamAttributes, Signature, Type,
    TypeAttributes, TypeName, Value, writer,
};

const COLLECTIONS: &str = "Windows.Foundation.Collections";
const VECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
const OBSERVABLE_VECTOR: &str = "5917eb53-50b4-4a0d-b309-65862b3f1dbc";
const MAP: &str = "3c2925fe-8519-45c1-aa79-197b6718c1c1";

fn closed(namespace: &str, name: &str, generics: Vec<Type>) -> Type {
    Type::Name(TypeName {
        namespace: namespace.into(),
        name: name.into(),
        generics,
    })
}

fn fixture() -> Vec<u8> {
    let mut file = writer::File::new("UnionArrayDeclarations");
    let interface = file.TypeDef(
        "Tests",
        "IProbe",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![
                Type::U32,
                Type::U16,
                Type::U16,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
                Type::U8,
            ],
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        writer::HasAttribute::TypeDef(interface),
        writer::AttributeType::MemberRef(constructor),
        &[
            Value::U32(0x86270e31),
            Value::U16(0x9c04),
            Value::U16(0x4fc2),
            Value::U8(0x9c),
            Value::U8(0xaf),
            Value::U8(1),
            Value::U8(2),
            Value::U8(3),
            Value::U8(4),
            Value::U8(5),
            Value::U8(6),
        ]
        .into_iter()
        .map(|value| (String::new(), value))
        .collect::<Vec<_>>(),
    );
    let reference = closed("Windows.Foundation", "IReference`1", vec![Type::U32]);
    for (name, typ) in [
        (
            "Vector",
            closed(COLLECTIONS, "IVector`1", vec![reference.clone()]),
        ),
        (
            "View",
            closed(COLLECTIONS, "IVectorView`1", vec![reference.clone()]),
        ),
        (
            "Iterable",
            closed(COLLECTIONS, "IIterable`1", vec![reference.clone()]),
        ),
        (
            "Observable",
            closed(COLLECTIONS, "IObservableVector`1", vec![reference.clone()]),
        ),
        (
            "Map",
            closed(COLLECTIONS, "IMap`2", vec![Type::String, reference.clone()]),
        ),
        (
            "Keys",
            closed(COLLECTIONS, "IMap`2", vec![reference.clone(), Type::U32]),
        ),
        ("Array", Type::Array(Box::new(reference))),
        (
            "Strings",
            closed(COLLECTIONS, "IVector`1", vec![Type::String]),
        ),
        ("Numbers", closed(COLLECTIONS, "IVector`1", vec![Type::U32])),
        ("Bytes", Type::Array(Box::new(Type::U8))),
    ] {
        file.MethodDef(
            &format!("Take{name}"),
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Void,
                types: vec![typ],
            },
            MethodAttributes::Public
                | MethodAttributes::Abstract
                | MethodAttributes::Virtual
                | MethodAttributes::NewSlot,
            MethodImplAttributes::default(),
        );
        file.Param("value", 1, ParamAttributes::In);
    }
    file.into_stream()
}

#[test]
fn sdk_backed_union_arrays_pass_strict_tsc_and_reject_scalar_containers() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let required = std::env::var("DYNWINRT_REQUIRE_TSC").as_deref() == Ok("1");
    let windows_winmd = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
            )
        });
    let tsc = std::env::var_os("DYNWINRT_TSC")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../bindings/js/node_modules/typescript/bin/tsc"));
    for path in [&windows_winmd, &tsc] {
        if !path.is_file() {
            assert!(
                !required,
                "Required declaration test dependency is missing: {}",
                path.display()
            );
            eprintln!(
                "Skipping union array declaration test: missing {}",
                path.display()
            );
            return;
        }
    }
    let version = Command::new("node").arg(&tsc).arg("--version").output();
    match version {
        Ok(output) if output.status.success() => {}
        result => {
            assert!(
                !required,
                "Required TypeScript compiler could not run: {result:?}"
            );
            eprintln!("Skipping union array declaration test: node could not run tsc");
            return;
        }
    }

    let directory = manifest
        .join("target")
        .join(format!("union-arrays-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let input = directory.join("Input.winmd");
    fs::write(&input, fixture()).unwrap();
    let generated = directory.join("generated");
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--namespace",
            "Tests",
            "--lang",
            "js",
            "--winmd",
        ])
        .arg(format!("{};{}", input.display(), windows_winmd.display()))
        .arg("--output")
        .arg(&generated)
        .output()
        .expect("run freshly built codegen");
    assert!(output.status.success(), "codegen failed: {output:?}");

    // No native addon is needed: declarations use only these opaque runtime types.
    let runtime = directory.join("node_modules/@microsoft/dynwinrt");
    fs::create_dir_all(&runtime).unwrap();
    fs::write(runtime.join("package.json"), r#"{"types":"index.d.ts"}"#).unwrap();
    fs::write(
        runtime.join("index.d.ts"),
        r#"
export declare class WinGuid { private readonly brand: unknown; }
export declare class DynWinRtType { private readonly brand: unknown; }
export declare class DynWinRtValue { private readonly brand: unknown; }
export declare class DynWinRtStruct { private readonly brand: unknown; }
export declare class DynWinRtImplementation { private readonly brand: unknown; }
export declare class DynWinRtImplementationHandle<T> { readonly value: T; }
export interface DynWinRtImplementationDescriptor { readonly plan: DynWinRtType; }
export interface DynWinRtImplementationType {
    implementation(handlers: never): DynWinRtImplementationDescriptor;
}
export interface DynWinRtImplementationOptions<T extends readonly DynWinRtImplementationType[]> {
    readonly interfaces: T;
}
"#,
    )
    .unwrap();
    let reference = TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
        args: vec![TypeMeta::U32],
    };
    let imports = [
        ("Vector", "IVector", VECTOR, vec![reference.clone()]),
        (
            "Observable",
            "IObservableVector",
            OBSERVABLE_VECTOR,
            vec![reference.clone()],
        ),
        (
            "Values",
            "IMap",
            MAP,
            vec![TypeMeta::String, reference.clone()],
        ),
        ("Keys", "IMap", MAP, vec![reference, TypeMeta::U32]),
        ("Strings", "IVector", VECTOR, vec![TypeMeta::String]),
        ("Numbers", "IVector", VECTOR, vec![TypeMeta::U32]),
    ]
    .into_iter()
    .map(|(alias, name, piid, args)| {
        format!(
            "{} as {alias}",
            parameterized_name(COLLECTIONS, name, piid, &args)
        )
    })
    .collect::<Vec<_>>()
    .join(", ");
    let prefix = format!(
        "import {{ IProbe, IReference_UInt32, {imports} }} from './generated/index.js';\n\
         declare const probe: IProbe;\n\
         declare const boxed: IReference_UInt32;\n\
         declare const vector: Vector;\n"
    );
    let valid = format!(
        r#"{prefix}
const native = [17, null];
const boxes = [boxed];
Vector.create(native);
Vector.create(boxes);
Vector.create([17, null, boxed]);
Observable.create(native);
Observable.create(boxes);
Values.create(['value', 'absent'], native);
Values.create(['value'], boxes);
Keys.create(native, [17, 0]);
Keys.create(boxes, [17]);
probe.takeArray(native);
probe.takeArray(boxes);
probe.takeVector(native);
probe.takeVector(vector);
probe.takeView(native);
probe.takeView(vector.getView());
probe.takeIterable(native);
vector.replaceAll(boxes);
vector.getMany(0, boxes);
const materialized: (number | null | IReference_UInt32)[] = vector.toArray();
Strings.create(['value']);
Numbers.create([17]);
probe.takeStrings(['value']);
probe.takeNumbers([17]);
probe.takeBytes(new Uint8Array([17]));
probe.takeBytes([17]);
"#
    );
    fs::write(directory.join("valid.ts"), valid).unwrap();
    let invalid = [
        ("Vector.create(17);", "2345"),
        ("Vector.create(null);", "2345"),
        ("Observable.create(17);", "2345"),
        ("Observable.create(null);", "2345"),
        ("Values.create(['value'], 17);", "2345"),
        ("Values.create(['value'], null);", "2345"),
        ("Keys.create(17, [17]);", "2345"),
        ("Keys.create(null, [17]);", "2345"),
        ("probe.takeArray(17);", "2345"),
        ("probe.takeArray(null);", "2345"),
        ("probe.takeVector(17);", "2345"),
        ("probe.takeVector(null);", "2345"),
        ("probe.takeView(17);", "2345"),
        ("probe.takeIterable(null);", "2345"),
        ("vector.replaceAll(17);", "2345"),
        ("vector.getMany(0, null);", "2345"),
        ("Vector.create([[17]]);", "2322"),
        ("Vector.create(['17']);", "2322"),
        ("Vector.create({ value: 17 });", "2561"),
        ("Strings.create('value');", "2345"),
        ("Numbers.create(17);", "2345"),
    ];
    fs::write(
        directory.join("invalid.ts"),
        format!(
            "{prefix}{}\n",
            invalid
                .iter()
                .map(|(line, _)| *line)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let compile = |file| {
        Command::new("node")
            .arg(&tsc)
            .args([
                "--noEmit",
                "--strict",
                "--skipLibCheck",
                "false",
                "--target",
                "ES2022",
                "--module",
                "Node16",
                "--moduleResolution",
                "Node16",
                "--pretty",
                "false",
                file,
            ])
            .current_dir(&directory)
            .output()
            .expect("run TypeScript")
    };
    let positive = compile("valid.ts");
    let negative = compile("invalid.ts");
    let diagnostics = String::from_utf8_lossy(&negative.stdout);
    let regex = regex::Regex::new(r"^invalid\.ts\((\d+),\d+\): error TS(\d+):").unwrap();
    let actual = diagnostics
        .lines()
        .map(|line| {
            let captures = regex
                .captures(line)
                .unwrap_or_else(|| panic!("Unexpected diagnostic: {line}"));
            (
                captures[1].parse::<usize>().unwrap(),
                captures[2].to_string(),
            )
        })
        .collect::<Vec<_>>();
    let expected = invalid
        .iter()
        .enumerate()
        .map(|(index, (_, code))| (prefix.lines().count() + index + 1, code.to_string()))
        .collect::<Vec<_>>();
    assert!(
        positive.status.success() && negative.status.code() == Some(2) && actual == expected,
        "Strict valid-array check: {positive:?}\nInvalid-container diagnostics:\n{diagnostics}\n\
         Expected: {expected:?}\nActual: {actual:?}\nFixture: {}",
        directory.display()
    );
    fs::remove_dir_all(directory).unwrap();
}
