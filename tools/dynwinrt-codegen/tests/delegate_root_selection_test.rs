// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dynwinrt_codegen::meta;
use dynwinrt_codegen::types::{TypeIdentityKind, TypeMeta};
use windows_metadata::{
    GenericParamAttributes, HasAttributes, MethodAttributes, MethodCallAttributes,
    MethodImplAttributes, ParamAttributes, Signature, Type, TypeAttributes, Value, reader, writer,
};

const NAMESPACE: &str = "Windows.System.Threading";
const DEFAULT_WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = repo().join("target").join(format!(
            "dr{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn windows_winmd() -> Option<PathBuf> {
    let path = std::env::var_os("DYNWINRT_WINDOWS_WINMD")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WINDOWS_WINMD));
    if path.is_file() {
        return Some(path);
    }
    assert!(
        std::env::var("DYNWINRT_REQUIRE_TSC").as_deref() != Ok("1"),
        "Windows metadata is missing at {}",
        path.display()
    );
    eprintln!("Skipping stock delegate checks: Windows metadata unavailable");
    None
}

fn stock_metadata_paths(winmd: &Path) -> String {
    let mut inputs = vec![winmd.to_str().unwrap().to_owned()];
    if !meta::list_namespaces(&inputs[0])
        .iter()
        .any(|namespace| namespace == "Windows" || namespace.starts_with("Windows."))
    {
        // Keep a versioned Facade's matching contract; an unversioned Facade needs
        // the CLI's versioned-SDK discovery. Both retain the original input.
        let matching_native = winmd
            .parent()
            .filter(|parent| {
                parent
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Facade"))
            })
            .and_then(Path::parent)
            .filter(|parent| {
                parent
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("10."))
            })
            .map(|version| version.join("Windows.winmd"))
            .filter(|path| path.is_file());
        let (native, selection) = if let Some(native) = matching_native {
            (native, "matching versioned SDK")
        } else {
            (native_sdk_winmd(), "CLI versioned-SDK fallback")
        };
        eprintln!(
            "Supplementing {} with native SDK contracts from {} ({selection})",
            winmd.display(),
            native.display()
        );
        inputs.push(native.to_str().unwrap().to_owned());
    }
    meta::expand_winmd_paths(&inputs.join(";"))
}

fn native_sdk_winmd() -> PathBuf {
    // Same version discovery as main.rs::find_windows_sdk_winmd, not CLR Facade sorting.
    let base = Path::new(DEFAULT_WINDOWS_WINMD)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let mut versions = fs::read_dir(base)
        .expect("discover installed native SDK metadata")
        .map(|entry| entry.expect("read SDK metadata directory").path())
        .filter(|path| path.is_dir())
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("10."))
        .collect::<Vec<_>>();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|version| base.join(version).join("Windows.winmd"))
        .find(|path| path.is_file())
        .expect("installed native Windows.winmd required for delegate contracts")
}

fn metadata_index(paths: &str) -> reader::Index {
    reader::Index::new(
        paths
            .split(';')
            .map(|path| reader::File::read(path).expect("read expanded metadata input"))
            .collect(),
    )
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn generate(
    metadata: &Path,
    output: &Path,
    namespace: &str,
    roots: Option<&str>,
    language: &str,
    no_pyi: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command
        .args(["generate", "--winmd"])
        .arg(metadata)
        .args(["--namespace", namespace, "--lang", language, "--output"])
        .arg(output);
    if let Some(roots) = roots {
        command.args(["--class-name", roots]);
    }
    if no_pyi {
        command.arg("--no-pyi");
    }
    command.output().unwrap()
}

fn generate_ok(metadata: &Path, output: &Path, roots: Option<&str>, language: &str, no_pyi: bool) {
    let result = generate(metadata, output, NAMESPACE, roots, language, no_pyi);
    assert!(result.status.success(), "{}", diagnostics(&result));
}

fn module_file(output: &Path, name: &str, language: &str, extension: &str) -> PathBuf {
    if language == "js" {
        output
            .join("windows")
            .join("system")
            .join("threading")
            .join(format!("{name}.{extension}"))
    } else {
        let module = dynwinrt_codegen::codegen::python::to_snake_case_filename(name);
        output.join(format!("windows__system__threading__{module}.{extension}"))
    }
}

fn assert_delegate(output: &Path, name: &str, language: &str, no_pyi: bool) {
    let extension = if language == "js" { "js" } else { "py" };
    let source = fs::read_to_string(module_file(output, name, language, extension)).unwrap();
    assert!(source.contains(&format!("IID_{name} = ")), "{source}");
    assert!(
        source.contains(&format!("{name}_PARAM_TYPES = [")),
        "{source}"
    );
    assert!(!source.contains(&format!("class {name}")), "{source}");
    assert!(!source.contains("Placeholder"), "{source}");
    if language == "js" {
        assert!(
            source.contains(&format!("exports.IID_{name} = IID_{name};")),
            "{source}"
        );
        let declaration = fs::read_to_string(module_file(output, name, language, "d.ts")).unwrap();
        assert!(
            declaration.contains(&format!("export type {name} = ")),
            "{declaration}"
        );
    } else if !no_pyi {
        let stub = fs::read_to_string(module_file(output, name, language, "pyi")).unwrap();
        assert!(stub.contains(&format!("IID_{name}: WinGUID")), "{stub}");
        assert!(
            stub.contains(&format!("{name}_PARAM_TYPES: list[DynWinRTType]")),
            "{stub}"
        );
    } else {
        assert!(!module_file(output, name, language, "pyi").exists());
        assert!(!output.join("py.typed").exists());
    }
}

fn assert_same_modules(
    expected: &Path,
    actual: &Path,
    names: &[&str],
    language: &str,
    no_pyi: bool,
) {
    let extensions = match (language, no_pyi) {
        ("js", _) => &["js", "d.ts"][..],
        ("py", false) => &["py", "pyi"][..],
        ("py", true) => &["py"][..],
        _ => unreachable!(),
    };
    for name in names {
        for extension in extensions {
            let expected = module_file(expected, name, language, extension);
            let actual = module_file(actual, name, language, extension);
            assert_eq!(
                fs::read(&expected).unwrap(),
                fs::read(&actual).unwrap(),
                "{} differs from {}",
                actual.display(),
                expected.display()
            );
        }
    }
}

#[test]
fn stock_delegate_roots_match_automatic_dependencies_in_both_languages() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    for (class, delegate) in [
        ("ThreadPool", "WorkItemHandler"),
        ("ThreadPoolTimer", "TimerElapsedHandler"),
    ] {
        for (language, no_pyi) in [("js", false), ("py", false), ("py", true)] {
            let fixture = Fixture::new();
            let automatic = fixture.0.join("automatic");
            generate_ok(&winmd, &automatic, Some(class), language, no_pyi);
            assert_delegate(&automatic, delegate, language, no_pyi);
            for roots in [
                delegate.to_string(),
                format!("{class},{delegate}"),
                format!("{delegate},{class}"),
            ] {
                let explicit = fixture.0.join("explicit");
                generate_ok(&winmd, &explicit, Some(&roots), language, no_pyi);
                assert_delegate(&explicit, delegate, language, no_pyi);
                assert_same_modules(&automatic, &explicit, &[delegate], language, no_pyi);
                if roots.contains(class) {
                    assert_same_modules(&automatic, &explicit, &[class], language, no_pyi);
                }
            }

            let incremental = fixture.0.join("incremental");
            for roots in [
                delegate.to_string(),
                class.to_string(),
                format!("{class},{delegate}"),
                format!("{delegate},{class}"),
                format!("{delegate},{class}"),
            ] {
                generate_ok(&winmd, &incremental, Some(&roots), language, no_pyi);
                assert_delegate(&incremental, delegate, language, no_pyi);
            }
            assert_same_modules(
                &automatic,
                &incremental,
                &[class, delegate],
                language,
                no_pyi,
            );

            generate_ok(&winmd, &automatic, Some(delegate), language, no_pyi);
            assert_delegate(&automatic, delegate, language, no_pyi);
            assert_same_modules(
                &incremental,
                &automatic,
                &[class, delegate],
                language,
                no_pyi,
            );

            let namespace = fixture.0.join("namespace");
            generate_ok(&winmd, &namespace, None, language, no_pyi);
            assert_delegate(&namespace, delegate, language, no_pyi);
            assert_same_modules(&incremental, &namespace, &[delegate], language, no_pyi);
        }
    }
}

#[test]
fn stock_delegate_classification_preserves_winmd_identity_and_invoke_contracts() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    let paths = stock_metadata_paths(&winmd);
    assert_stock_delegate_contracts(&paths);
    let error = meta::parse_delegate(&paths, "Windows.Foundation", "EventHandler").unwrap_err();
    assert!(error.contains("open generic delegate roots"), "{error}");
    let fixture = Fixture::new();
    for language in ["js", "py"] {
        let output = fixture.0.join(language);
        let result = generate(
            Path::new(&paths),
            &output,
            "Windows.Foundation",
            Some("EventHandler"),
            language,
            false,
        );
        assert!(!result.status.success(), "{}", diagnostics(&result));
        assert!(diagnostics(&result).contains("open generic delegate roots"));
        assert!(!output.exists());
    }
}

fn assert_stock_delegate_contracts(path: &str) {
    let index = metadata_index(path);
    for (class, name) in [
        ("ThreadPool", "WorkItemHandler"),
        ("ThreadPoolTimer", "TimerElapsedHandler"),
    ] {
        let definition = index.get(NAMESPACE, name).next().unwrap();
        let base = definition.extends().unwrap();
        assert_eq!(
            (base.namespace(), base.name()),
            ("System", "MulticastDelegate")
        );
        assert!(definition.flags().contains(TypeAttributes::WindowsRuntime));
        assert!(!definition.flags().contains(TypeAttributes::Interface));
        assert!(definition.generic_params().next().is_none());
        assert!(definition.find_attribute("GuidAttribute").is_some());
        assert!(meta::parse_public_interface(path, NAMESPACE, name).is_none());

        let root = meta::parse_delegate(path, NAMESPACE, name)
            .unwrap()
            .unwrap();
        assert_eq!(
            root.type_identity().kind(),
            Some(TypeIdentityKind::Delegate)
        );
        let class = meta::parse_class(path, NAMESPACE, class).unwrap();
        let dependencies = meta::resolve_dependencies(path, &[class], &[], &[]);
        let dependency = dependencies
            .interfaces
            .iter()
            .find(|interface| interface.namespace == NAMESPACE && interface.name == name)
            .unwrap();
        assert_eq!(root.iid, dependency.iid);
        let invoke = root
            .methods
            .iter()
            .find(|method| method.name == "Invoke")
            .unwrap();
        let dependency_invoke = dependency
            .methods
            .iter()
            .find(|method| method.name == "Invoke")
            .unwrap();
        assert!(invoke.return_type.is_none());
        assert_eq!(invoke.params.len(), 1);
        assert_eq!(invoke.params[0].typ, dependency_invoke.params[0].typ);
        assert_eq!(
            invoke.params[0].direction,
            dependency_invoke.params[0].direction
        );
        if name == "WorkItemHandler" {
            assert_eq!(root.iid, "1d1a8b8b-fa66-414f-9cbd-b65fc99d17fa");
            assert_eq!(invoke.params[0].typ, TypeMeta::AsyncAction);
        } else {
            assert!(matches!(
                &invoke.params[0].typ,
                TypeMeta::RuntimeClass { namespace, name, .. }
                    if namespace == NAMESPACE && name == "ThreadPoolTimer"
            ));
        }
        eprintln!("{NAMESPACE}.{name}: {} -> {:?}", root.iid, invoke.params);
    }
    for (namespace, name) in [
        (NAMESPACE, "ThreadPool"),
        (NAMESPACE, "WorkItemPriority"),
        ("Windows.Foundation", "Point"),
        ("Windows.Foundation", "IStringable"),
        ("Windows.Foundation", "IUriRuntimeClass"),
        ("Windows.Foundation", "IReference"),
    ] {
        assert!(
            meta::parse_delegate(path, namespace, name)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn sdk_facade_and_native_contract_graphs_match_generated_delegates() {
    let Some(_) = windows_winmd() else {
        return;
    };
    let native = native_sdk_winmd();
    let native_paths = stock_metadata_paths(&native);
    assert_stock_delegate_contracts(&native_paths);
    let fixture = Fixture::new();
    let version = native.parent().unwrap();
    for (index, facade) in [
        version.join("Facade").join("Windows.winmd"),
        version
            .parent()
            .unwrap()
            .join("Facade")
            .join("Windows.winmd"),
    ]
    .iter()
    .enumerate()
    {
        assert!(
            facade.is_file(),
            "SDK Facade control at {}",
            facade.display()
        );
        let raw = reader::Index::read(facade).expect("read SDK CLR Facade control");
        assert!(
            raw.get(NAMESPACE, "WorkItemHandler").next().is_none(),
            "CLR Facade control must require native metadata supplementation"
        );
        let facade_paths = stock_metadata_paths(facade);
        assert_eq!(facade_paths.split(';').next(), facade.to_str());
        assert_stock_delegate_contracts(&facade_paths);
        for language in ["js", "py"] {
            let native_output = fixture.0.join(format!("{language}-native"));
            let facade_output = fixture.0.join(format!("{language}-facade-{index}"));
            let raw_facade_output = fixture.0.join(format!("{language}-raw-facade-{index}"));
            for (input, output) in [
                (Path::new(&native_paths), &native_output),
                (Path::new(&facade_paths), &facade_output),
                (facade.as_path(), &raw_facade_output),
            ] {
                generate_ok(
                    input,
                    output,
                    Some("ThreadPool,WorkItemHandler,ThreadPoolTimer,TimerElapsedHandler"),
                    language,
                    false,
                );
                assert_delegate(output, "WorkItemHandler", language, false);
                assert_delegate(output, "TimerElapsedHandler", language, false);
                assert_same_modules(
                    &native_output,
                    output,
                    &[
                        "ThreadPool",
                        "WorkItemHandler",
                        "ThreadPoolTimer",
                        "TimerElapsedHandler",
                    ],
                    language,
                    false,
                );
            }
        }
    }
}

fn guid(file: &mut writer::File, definition: writer::TypeDef, values: Vec<Value>) {
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "GuidAttribute");
    let types = values
        .iter()
        .map(|value| match value {
            Value::U32(_) => Type::U32,
            Value::U16(_) => Type::U16,
            Value::U8(_) => Type::U8,
            Value::I32(_) => Type::I32,
            Value::Utf8(_) => Type::String,
            _ => unreachable!(),
        })
        .collect();
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types,
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        writer::HasAttribute::TypeDef(definition),
        writer::AttributeType::MemberRef(constructor),
        &values
            .into_iter()
            .map(|value| (String::new(), value))
            .collect::<Vec<_>>(),
    );
}

fn delegate_fixture() -> Vec<u8> {
    let mut file = writer::File::new("DelegateRoots");
    let base = file.TypeRef("System", "MulticastDelegate");
    let object = file.TypeRef("System", "Object");
    for name in [
        "ValidHandler",
        "ClrHandler",
        "PrivateHandler",
        "NestedHandler",
        "InterfaceHandler",
        "MissingGuid",
        "ShortGuid",
        "WrongGuidTypes",
        "MissingCtor",
        "MissingInvoke",
        "DuplicateInvoke",
        "StaticInvoke",
        "MissingParam",
        "InOutParam",
        "NativePointer",
        "PreserveSig",
        "GenericHandler",
        "Pretender",
    ] {
        let visibility = match name {
            "PrivateHandler" => TypeAttributes::default(),
            _ => TypeAttributes::Public,
        };
        let flags = visibility | TypeAttributes::Sealed;
        let flags = if name == "NestedHandler" {
            // This includes nested visibility bits absent from the writer's named constants.
            !TypeAttributes::Interface
        } else if name == "InterfaceHandler" {
            flags | TypeAttributes::Interface
        } else {
            flags
        };
        let definition = file.TypeDef(
            "Tests.DelegateRoots",
            name,
            writer::TypeDefOrRef::TypeRef(if name == "Pretender" { object } else { base }),
            if name == "ClrHandler" {
                flags
            } else {
                flags | TypeAttributes::WindowsRuntime
            },
        );
        if name == "GenericHandler" {
            file.GenericParam(
                "T",
                writer::TypeOrMethodDef::TypeDef(definition),
                0,
                GenericParamAttributes::default(),
            );
        }
        if name != "MissingGuid" {
            let mut values = vec![Value::U32(1), Value::U16(2), Value::U16(3)];
            values.extend((4..12).map(Value::U8));
            if name == "ShortGuid" {
                values = vec![Value::Utf8("not-a-guid".into())];
            } else if name == "WrongGuidTypes" {
                values[0] = Value::I32(1);
            }
            guid(&mut file, definition, values);
        }
        if name != "MissingCtor" {
            file.MethodDef(
                ".ctor",
                &Signature {
                    flags: MethodCallAttributes::HASTHIS,
                    return_type: Type::Void,
                    types: vec![],
                },
                MethodAttributes::Public | MethodAttributes::SpecialName,
                MethodImplAttributes::default(),
            );
        }
        if name == "MissingInvoke" {
            continue;
        }
        for _ in 0..if name == "DuplicateInvoke" { 2 } else { 1 } {
            file.MethodDef(
                "Invoke",
                &Signature {
                    flags: if name == "StaticInvoke" {
                        MethodCallAttributes::default()
                    } else {
                        MethodCallAttributes::HASTHIS
                    },
                    return_type: Type::Void,
                    types: vec![if name == "NativePointer" {
                        Type::PtrMut(Box::new(Type::I32), 1)
                    } else {
                        Type::I32
                    }],
                },
                MethodAttributes::Public | MethodAttributes::Virtual | MethodAttributes::NewSlot,
                if name == "PreserveSig" {
                    MethodImplAttributes::PreserveSig
                } else {
                    MethodImplAttributes::default()
                },
            );
            if name != "MissingParam" {
                file.Param(
                    "value",
                    1,
                    if name == "InOutParam" {
                        ParamAttributes::In | ParamAttributes::Out
                    } else {
                        ParamAttributes::In
                    },
                );
            }
        }
    }
    file.into_stream()
}

#[test]
fn standalone_namespace_delegates_are_emitted_without_referencing_classes() {
    let fixture = Fixture::new();
    let mut file = writer::File::new("StandaloneDelegate");
    let base = file.TypeRef("System", "MulticastDelegate");
    let definition = file.TypeDef(
        "Tests.StandaloneDelegate",
        "Handler",
        writer::TypeDefOrRef::TypeRef(base),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let mut values = vec![Value::U32(1), Value::U16(2), Value::U16(3)];
    values.extend((4..12).map(Value::U8));
    guid(&mut file, definition, values);
    for name in [".ctor", "Invoke"] {
        file.MethodDef(
            name,
            &Signature {
                flags: MethodCallAttributes::HASTHIS,
                return_type: Type::Void,
                types: vec![],
            },
            MethodAttributes::Public,
            MethodImplAttributes::default(),
        );
    }
    let metadata = fixture.0.join("Standalone.winmd");
    fs::write(&metadata, file.into_stream()).unwrap();
    for language in ["js", "py"] {
        let output = fixture.0.join(language);
        let result = generate(
            &metadata,
            &output,
            "Tests.StandaloneDelegate",
            None,
            language,
            false,
        );
        assert!(result.status.success(), "{}", diagnostics(&result));
        let module = if language == "js" {
            output
                .join("tests")
                .join("standalone-delegate")
                .join("Handler.js")
        } else {
            output.join("tests__standalone_delegate__handler.py")
        };
        let source = fs::read_to_string(module).unwrap();
        assert!(source.contains("IID_Handler = "), "{source}");
        assert!(source.contains("Handler_PARAM_TYPES = []"), "{source}");
        assert!(!source.contains("class Handler"), "{source}");
    }
}

#[test]
fn malformed_or_non_winrt_delegate_roots_fail_before_class_fallback() {
    let fixture = Fixture::new();
    let metadata = fixture.0.join("DelegateRoots.winmd");
    fs::write(&metadata, delegate_fixture()).unwrap();
    let path = metadata.to_str().unwrap();
    let namespace = "Tests.DelegateRoots";
    let valid = meta::parse_delegate(path, namespace, "ValidHandler")
        .unwrap()
        .unwrap();
    assert!(valid.is_delegate());
    assert_eq!(
        valid
            .methods
            .iter()
            .find(|method| method.name == "Invoke")
            .unwrap()
            .params[0]
            .typ,
        TypeMeta::I32
    );
    assert!(
        meta::parse_delegate(path, namespace, "Pretender")
            .unwrap()
            .is_none()
    );
    assert!(
        meta::parse_delegate(path, namespace, "DoesNotExist")
            .unwrap()
            .is_none()
    );
    for (name, reason) in [
        ("ClrHandler", "not a public Windows Runtime delegate"),
        ("PrivateHandler", "not a public Windows Runtime delegate"),
        ("NestedHandler", "not a public Windows Runtime delegate"),
        ("InterfaceHandler", "not a public Windows Runtime delegate"),
        ("MissingGuid", "missing GuidAttribute"),
        ("ShortGuid", "malformed GuidAttribute"),
        ("WrongGuidTypes", "malformed GuidAttribute"),
        (
            "MissingCtor",
            "requires .ctor and exactly one Invoke contract",
        ),
        (
            "MissingInvoke",
            "requires .ctor and exactly one Invoke contract",
        ),
        (
            "DuplicateInvoke",
            "requires .ctor and exactly one Invoke contract",
        ),
        ("StaticInvoke", "unsupported native calling convention"),
        ("MissingParam", "incomplete delegate parameter contracts"),
        ("InOutParam", "unsupported direction contract"),
        ("NativePointer", "unsupported native type PtrMut"),
        ("PreserveSig", "unsupported native calling convention"),
        ("GenericHandler", "open generic delegate roots"),
    ] {
        let error = meta::parse_delegate(path, namespace, name).unwrap_err();
        assert!(error.contains(reason), "{name}: {error}");
        for language in ["js", "py"] {
            let output = fixture.0.join(format!("{name}-{language}"));
            let result = generate(&metadata, &output, namespace, Some(name), language, false);
            assert!(!result.status.success(), "{name}: {}", diagnostics(&result));
            assert!(
                diagnostics(&result).contains(reason),
                "{}",
                diagnostics(&result)
            );
            assert!(
                !output.exists(),
                "invalid root committed {}",
                output.display()
            );
        }
    }
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"))
}

fn js_runtime_package(output: &Path, runtime: &Path) {
    let package = output
        .join("node_modules")
        .join("@microsoft")
        .join("dynwinrt");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        serde_json::to_vec(&serde_json::json!({
            "name": "@microsoft/dynwinrt",
            "main": runtime.join("dist").join("winrt.js"),
            "types": runtime.join("dist").join("winrt.d.ts")
        }))
        .unwrap(),
    )
    .unwrap();
}

fn bounded_native(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("Native delegate child timed out: {}", diagnostics(&output));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    let evidence = String::from_utf8_lossy(&output.stdout);
    assert!(evidence.contains("\"callbacks\""), "{evidence}");
    eprintln!("{evidence}");
    output
}

#[test]
fn explicitly_selected_js_delegate_runs_a_real_thread_pool_callback_cjs_and_esm() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    let runtime = std::env::var_os("DYNWINRT_TEST_JS_RUNTIME")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("bindings").join("js"));
    if !runtime.join("dist").join("winrt.js").is_file() {
        eprintln!("Skipping native JS delegate checks: matching production binding is not built");
        return;
    }
    let node = std::env::var_os("DYNWINRT_TEST_NODE").unwrap_or_else(|| "node".into());
    for module in ["cjs", "esm"] {
        let fixture = Fixture::new();
        js_runtime_package(&fixture.0, &runtime);
        let output = fixture.0.join("generated");
        generate_ok(
            &winmd,
            &output,
            Some("ThreadPool,WorkItemHandler"),
            "js",
            false,
        );
        bounded_native(
            Command::new(&node)
                .arg("--expose-gc")
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests")
                        .join("fixtures")
                        .join("delegate_root_native.mjs"),
                )
                .arg(&runtime)
                .arg(&output)
                .arg(module),
        );
    }
}

#[test]
fn explicitly_selected_python_delegate_runs_a_real_thread_pool_callback_with_and_without_stubs() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    if std::env::var_os("DYNWINRT_TEST_PYTHON").is_none() {
        eprintln!(
            "Skipping native Python delegate checks: set DYNWINRT_TEST_PYTHON to the matching wheel"
        );
        return;
    }
    for no_pyi in [false, true] {
        let fixture = Fixture::new();
        let output = fixture.0.join("generated");
        generate_ok(
            &winmd,
            &output,
            Some("ThreadPool,WorkItemHandler"),
            "py",
            no_pyi,
        );
        bounded_native(
            Command::new(python())
                .arg("-B")
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests")
                        .join("fixtures")
                        .join("delegate_root_native.py"),
                )
                .arg(&output),
        );
    }
}

#[test]
fn explicitly_selected_delegate_callables_pass_strict_typescript() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    let tsc = std::env::var_os("DYNWINRT_TSC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo()
                .join("bindings")
                .join("js")
                .join("node_modules")
                .join("typescript")
                .join("bin")
                .join("tsc")
        });
    if !tsc.is_file() {
        assert!(
            std::env::var("DYNWINRT_REQUIRE_TSC").as_deref() != Ok("1"),
            "Missing TypeScript compiler"
        );
        eprintln!("Skipping strict delegate consumer: TypeScript unavailable");
        return;
    }
    let fixture = Fixture::new();
    let output = fixture.0.join("generated");
    generate_ok(
        &winmd,
        &output,
        Some("ThreadPool,WorkItemHandler"),
        "js",
        false,
    );
    let package = fixture
        .0
        .join("node_modules")
        .join("@microsoft")
        .join("dynwinrt");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"@microsoft/dynwinrt","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export declare class DynWinRtType {}\nexport declare class WinGuid {}\nexport declare class DynWinRtValue { release(): void; }\n").unwrap();
    fs::write(fixture.0.join("consumer.ts"), r#"
import { ThreadPool } from './generated/windows/system/threading/ThreadPool.js';
import { WorkItemHandler, IID_WorkItemHandler, WorkItemHandler_PARAM_TYPES } from './generated/windows/system/threading/WorkItemHandler.js';
import { DynWinRtValue, DynWinRtType, WinGuid } from '@microsoft/dynwinrt';
const handler: WorkItemHandler = (operation: DynWinRtValue): void => { operation.release(); };
const operation: Promise<void> = ThreadPool.runAsync(handler);
const iid: WinGuid = IID_WorkItemHandler;
const parameters: DynWinRtType[] = WorkItemHandler_PARAM_TYPES;
// @ts-expect-error A delegate input must be callable.
ThreadPool.runAsync(42);
// @ts-expect-error The callback receives one argument, not two required arguments.
ThreadPool.runAsync((first: DynWinRtValue, second: DynWinRtValue) => {});
// @ts-expect-error A delegate is a callback type, not a runtime class constructor.
new WorkItemHandler();
void [operation, iid, parameters];
"#).unwrap();
    let result = Command::new("node")
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
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", diagnostics(&result));
}

#[test]
fn explicitly_selected_delegate_callables_pass_strict_mypy_and_pyright() {
    let Some(winmd) = windows_winmd() else {
        return;
    };
    let available = Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "Missing mypy"
    );
    if !available {
        eprintln!("Skipping strict delegate consumers: mypy unavailable");
        return;
    }
    let installed_runtime = Command::new(python())
        .args([
            "-I",
            "-c",
            "from dynwinrt import DynWinRTValue, WinRTCoroutine",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        installed_runtime
            || (std::env::var("DYNWINRT_TEST_INSTALLED_RUNTIME").as_deref() != Ok("1")
                && std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1")),
        "The requested installed-runtime typing lane requires the matching wheel"
    );
    let source_stubs = repo().join("bindings").join("py");
    assert!(source_stubs.join("dynwinrt.pyi").is_file());
    let inherited_paths = std::env::var_os("MYPYPATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    for no_pyi in [false, true] {
        let fixture = Fixture::new();
        let incomplete_source = fixture.0.join("binding-source");
        let source_package = incomplete_source.join("dynwinrt");
        fs::create_dir_all(&source_package).unwrap();
        for name in ["__init__.py", "py.typed"] {
            fs::copy(
                source_stubs.join("python").join("dynwinrt").join(name),
                source_package.join(name),
            )
            .unwrap();
        }
        assert!(!source_package.join("__init__.pyi").exists());
        let extra_stubs = fixture.0.join("extra-stubs");
        fs::create_dir_all(&extra_stubs).unwrap();
        fs::write(
            extra_stubs.join("delegate_fixture_extra.pyi"),
            "marker: str\n",
        )
        .unwrap();
        let mut paths = vec![incomplete_source.clone(), extra_stubs.clone()];
        paths.extend(inherited_paths.iter().cloned());
        let output = fixture.0.join("generated");
        generate_ok(
            &winmd,
            &output,
            Some("ThreadPool,WorkItemHandler"),
            "py",
            no_pyi,
        );
        for (negative, expected_errors) in [(false, 0), (true, 1)] {
            let consumer = fixture.0.join("consumer.py");
            fs::write(
                &consumer,
                format!(
                    "# pyright: strict\n\
                 from typing import assert_type\n\
                 from dynwinrt import DynWinRTValue, WinRTCoroutine\n\
                 from delegate_fixture_extra import marker\n\
                 from generated.windows__system__threading__thread_pool import ThreadPool\n\
                 def handler(operation: DynWinRTValue) -> None:\n    operation.release()\n\
                 assert_type(marker, str)\n\
                 assert_type(ThreadPool.run_async(handler), WinRTCoroutine[None])\n{}",
                    if negative {
                        "ThreadPool.run_async(42)\n"
                    } else {
                        ""
                    },
                ),
            )
            .unwrap();
            if !negative {
                let legacy =
                    mypy_consumer(&fixture, &[incomplete_source.clone(), extra_stubs.clone()]);
                let text = diagnostics(&legacy);
                assert_eq!(legacy.status.code(), Some(1), "{text}");
                assert!(
                    text.contains("Module \"dynwinrt\" has no attribute"),
                    "{text}"
                );
            }
            for use_source in if installed_runtime {
                &[true, false][..]
            } else {
                &[true][..]
            } {
                let stub_paths =
                    consumer_stub_paths(use_source.then_some(source_stubs.as_path()), &paths);
                assert!(stub_paths.contains(&extra_stubs));
                assert!(!stub_paths.contains(&incomplete_source));
                eprintln!(
                    "Strict delegate consumer: {}, no_pyi={no_pyi}, negative={negative}",
                    if *use_source {
                        "tracked dynwinrt.pyi"
                    } else {
                        "installed wheel"
                    }
                );
                let result = mypy_consumer(&fixture, &stub_paths);
                let text = diagnostics(&result);
                let errors = text
                    .lines()
                    .filter(|line| line.contains(": error:"))
                    .collect::<Vec<_>>();
                assert_eq!(errors.len(), expected_errors, "{text}");
                assert_eq!(result.status.success(), expected_errors == 0, "{text}");
                for error in errors {
                    assert!(
                        error.starts_with("consumer.py:") && error.ends_with("[arg-type]"),
                        "{text}"
                    );
                }
                if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
                    fs::write(
                        fixture.0.join("pyrightconfig.json"),
                        serde_json::to_vec(&serde_json::json!({ "extraPaths": stub_paths }))
                            .unwrap(),
                    )
                    .unwrap();
                    let result = Command::new(pyright)
                        .arg("--pythonpath")
                        .arg(python())
                        .arg("consumer.py")
                        .env_remove("PYTHONPATH")
                        .current_dir(&fixture.0)
                        .output()
                        .unwrap();
                    let text = diagnostics(&result);
                    let errors = text
                        .lines()
                        .filter(|line| line.contains(" - error: "))
                        .collect::<Vec<_>>();
                    assert_eq!(errors.len(), expected_errors, "{text}");
                    assert_eq!(result.status.success(), expected_errors == 0, "{text}");
                    for error in errors {
                        assert!(error.contains("consumer.py:"), "{text}");
                    }
                    assert_eq!(
                        text.matches("(reportArgumentType)").count(),
                        expected_errors,
                        "{text}"
                    );
                }
            }
        }
    }
}

fn consumer_stub_paths(source_stubs: Option<&Path>, inherited: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = source_stubs
        .map(Path::to_path_buf)
        .into_iter()
        .collect::<Vec<_>>();
    let cwd = std::env::current_dir().unwrap();
    for path in inherited {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };
        // Retain other stub roots, but never let a source package mask wheel typing.
        if !path.join("dynwinrt").is_dir()
            && !path.join("dynwinrt.pyi").is_file()
            && !path.join("dynwinrt.py").is_file()
        {
            paths.push(path);
        }
    }
    paths
}

fn mypy_consumer(fixture: &Fixture, stub_paths: &[PathBuf]) -> Output {
    let mut command = Command::new(python());
    command
        .args([
            "-B",
            "-m",
            "mypy",
            "--strict",
            "--no-incremental",
            "--follow-imports=silent",
            "--no-pretty",
            "--show-error-codes",
        ])
        .arg("consumer.py")
        .env_remove("PYTHONPATH")
        .env_remove("MYPYPATH")
        .current_dir(&fixture.0);
    if !stub_paths.is_empty() {
        command.env("MYPYPATH", std::env::join_paths(stub_paths).unwrap());
    }
    command.output().unwrap()
}
