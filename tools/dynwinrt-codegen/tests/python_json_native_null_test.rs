// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const JSON_CLASSES: &str =
    "Windows.Data.Json.JsonArray,Windows.Data.Json.JsonObject,Windows.Data.Json.JsonValue";
static NEXT: AtomicU64 = AtomicU64::new(0);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = repo_root().join(r"bindings\py\.venv\Scripts\python.exe");
            if venv.is_file() {
                venv
            } else {
                PathBuf::from("python")
            }
        })
}

struct Generated {
    root: PathBuf,
    package: String,
}

impl Generated {
    fn new() -> Option<Self> {
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping JSON SDK regression: Windows.winmd not found");
            return None;
        }
        let package = format!(
            "json_native_null_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let root = repo_root().join("target").join(&package);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--winmd",
                WINDOWS_WINMD,
                "--class-name",
                JSON_CLASSES,
                "--lang",
                "py",
                "--output",
            ])
            .arg(&root)
            .output()
            .expect("generate stock JSON bindings");
        assert_success(output);
        Some(Self { root, package })
    }

    fn module(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).expect(name)
    }

    fn python(&self, script: &str) -> Output {
        Command::new(python())
            .args(["-B", "-c", &script.replace("JSON_PACKAGE", &self.package)])
            .env("PYTHONPATH", self.root.parent().unwrap())
            .output()
            .expect("execute isolated Python consumer")
    }

    fn typing_environment(&self, command: &mut Command) {
        command
            .current_dir(self.root.parent().unwrap())
            .env(
                "MYPYPATH",
                std::env::join_paths([
                    repo_root().join(r"bindings\py"),
                    self.root.parent().unwrap().to_path_buf(),
                ])
                .expect("MYPYPATH"),
            )
            .env("PYTHONPATH", self.root.parent().unwrap());
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_success(output: Output) {
    assert!(
        output.status.success(),
        "exit {:?}\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stock_json_generation_preserves_receiver_dependent_contract() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let array = generated.module("windows__data__json__json_array.py");
    let object = generated.module("windows__data__json__json_object.py");
    let array_stub = generated.module("windows__data__json__json_array.pyi");
    let object_stub = generated.module("windows__data__json__json_object.pyi");

    for code in [&array, &object] {
        assert!(
            code.contains("_dynwinrt_non_null_collection_contract ="),
            "{code}"
        );
        assert!(
            code.contains("if not obj._matches_runtime_class("),
            "{code}"
        );
        assert!(
            code.contains("._validate_non_null_collection_input("),
            "{code}"
        );
    }
    assert!(
        array.contains("def replace_all(")
            && array.contains(
                "self._collection_obj._validate_non_null_collection_input(_dynwinrt_array("
            )
    );
    assert!(
        object.contains("def set_named_value(")
            && object.contains("self._obj._validate_non_null_collection_input(")
    );
    assert!(
        array_stub.contains("class JsonArray(_JsonArrayIdentity, MutableSequence['IJsonValue']")
            && array_stub.contains("def get_at(self, index: int) -> 'IJsonValue': ...")
            && array_stub
                .contains("def replace_all(self, items: DynWinRTArray | Sequence['IJsonValue'])")
            && array_stub.contains("class IVector_IJsonValue(MutableSequence[IJsonValue | None])"),
        "{array_stub}"
    );
    assert!(
        object_stub
            .contains("class JsonObject(_JsonObjectIdentity, MutableMapping[str, 'IJsonValue']")
            && object_stub.contains("def lookup(self, key: str) -> 'IJsonValue': ...")
            && object_stub.contains("def insert(self, key: str, value: 'IJsonValue')")
            && object_stub
                .contains("class IMap_String_IJsonValue(MutableMapping[str, IJsonValue | None])"),
        "{object_stub}"
    );
}

#[test]
fn stock_json_mutators_fail_before_native_mutation_but_custom_generics_keep_null() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let available = Command::new(python())
        .args([
            "-c",
            "from dynwinrt import DynWinRTInterfacePlan, DynWinRTValue; assert hasattr(DynWinRTValue, '_validate_non_null_collection_input')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "the JSON native regression requires the matching Python binding"
    );
    if !available {
        eprintln!("Skipping JSON native regression: matching Python binding not installed");
        return;
    }
    let script = r#"
import operator
from dynwinrt import DynWinRTArray, DynWinRTType, DynWinRTValue, RoApartment, release_projected
from JSON_PACKAGE.windows__data__json__json_array import JsonArray, IVector_IJsonValue, IID_IJsonValue
from JSON_PACKAGE.windows__data__json__json_object import JsonObject, IMap_String_IJsonValue
from JSON_PACKAGE.windows__data__json__json_value import JsonValue

def rejected_without_mutation(receiver, mutation):
    before = receiver.stringify()
    try:
        mutation()
    except TypeError as error:
        assert ('requires a non-null IJsonValue' in str(error)
                or 'map key cannot be None' in str(error)), error
    else:
        raise AssertionError('native null was accepted by a stock JSON collection')
    assert receiver.stringify() == before, (before, receiver.stringify())

with RoApartment():
    element = DynWinRTType.interface(IID_IJsonValue)
    native_null = DynWinRTValue.null_value()
    array_of_null = DynWinRTArray.from_values([native_null], element)
    json_null = JsonValue.create_null_value()
    assert not json_null._obj.is_null()
    assert json_null.stringify() == 'null'

    array = JsonArray.parse('[1]')
    view = array.as_interface(IVector_IJsonValue)
    for mutation in (
        lambda: array.append(None),
        lambda: array.append(native_null),
        lambda: array.insert(0, None),
        lambda: array.insert_at(0, None),
        lambda: array.set_at(0, None),
        lambda: operator.setitem(array, 0, None),
        lambda: operator.setitem(array, slice(None), [json_null, None]),
        lambda: array.replace_all([None]),
        lambda: array.replace_all([json_null, None]),
        lambda: array.replace_all(array_of_null),
        lambda: array.replace_all(array_of_null.to_value()),
        lambda: array.extend([json_null, None]),
        lambda: operator.iadd(array, [json_null, None]),
        lambda: view.append(None),
        lambda: view.set_at(0, None),
        lambda: view.replace_all(array_of_null),
        lambda: view.extend([json_null, None]),
    ):
        rejected_without_mutation(array, mutation)

    obj = JsonObject.parse('{"base":1}')
    map_view = obj.as_interface(IMap_String_IJsonValue)
    for mutation in (
        lambda: obj.insert('bad', None),
        lambda: operator.setitem(obj, 'bad', native_null),
        lambda: obj.set_named_value('bad', native_null),
        lambda: obj.update({'good': json_null, 'bad': None}),
        lambda: obj.update([('good', json_null), ('bad', native_null)]),
        lambda: obj.update([('good', json_null), (None, json_null)]),
        lambda: obj.setdefault('bad'),
        lambda: map_view.insert('bad', None),
        lambda: operator.setitem(map_view, 'bad', None),
        lambda: map_view.update({'good': json_null, 'bad': None}),
    ):
        rejected_without_mutation(obj, mutation)

    array.append(json_null)
    obj['valid'] = json_null
    assert array[-1].stringify() == 'null'
    assert obj['valid'].stringify() == 'null'
    activated_array = JsonArray.create()
    activated_object = JsonObject.create()
    activated_array.append(json_null)
    activated_object['valid'] = json_null
    assert activated_array[0].stringify() == activated_object['valid'].stringify() == 'null'

    generic_vector = IVector_IJsonValue.from_value(
        DynWinRTValue.create_vector([native_null], element)
    )
    assert generic_vector[0] is None
    generic_vector.append(None)
    generic_vector.replace_all(array_of_null)
    generic_vector.extend([None])
    assert list(generic_vector) == [None, None]
    try:
        JsonArray(generic_vector._obj)
    except TypeError as error:
        assert 'Expected a native Windows.Data.Json.JsonArray' in str(error)
    else:
        raise AssertionError('custom vector projected as stock JsonArray')

    generic_map = IMap_String_IJsonValue.from_value(
        DynWinRTValue.create_map(
            [DynWinRTValue.from_hstring('original')],
            [native_null],
            DynWinRTType.hstring(),
            element,
        )
    )
    generic_map.update({'next': None})
    assert generic_map['original'] is None and generic_map['next'] is None
    try:
        JsonObject(generic_map._obj)
    except TypeError as error:
        assert 'Expected a native Windows.Data.Json.JsonObject' in str(error)
    else:
        raise AssertionError('custom map projected as stock JsonObject')

    for wrapper in (
        generic_map, generic_vector, activated_object, activated_array,
        map_view, obj, view, array, json_null,
    ):
        release_projected(wrapper)
    native_null.release()
"#;
    assert_success(generated.python(script));
}

const VALID_CONSUMER: &str = r#"
from JSON_PACKAGE.windows.data.json import JsonArray, JsonObject, JsonValue
from JSON_PACKAGE.windows__data__json__json_array import IVector_IJsonValue
from JSON_PACKAGE.windows__data__json__json_object import IMap_String_IJsonValue

array: JsonArray = JsonArray.parse('[]')
obj: JsonObject = JsonObject.parse('{}')
value: JsonValue = JsonValue.create_null_value()
array.append(value)
array.extend([value])
array[0] = value
array[:] = [value]
array.replace_all([value])
obj['valid'] = value
obj.update({'valid': value})
obj.setdefault('valid', value)
vector: IVector_IJsonValue = array.as_interface(IVector_IJsonValue)
vector.append(None)
map_view: IMap_String_IJsonValue = obj.as_interface(IMap_String_IJsonValue)
map_view.insert('native-null', None)
"#;

const INVALID_CONSUMER: &str = r#"
from JSON_PACKAGE.windows.data.json import JsonArray, JsonObject, JsonValue
array: JsonArray = JsonArray.parse('[]')
obj: JsonObject = JsonObject.parse('{}')
value: JsonValue = JsonValue.create_null_value()
array.append(None)
array.insert(0, None)
array.replace_all([None])
array[:] = [None]
array.extend([value, None])
obj.insert('bad', None)
obj['bad'] = None
obj.update({'good': value, 'bad': None})
obj.setdefault('bad')
"#;

#[test]
fn stock_json_stubs_reject_null_with_strict_typecheckers() {
    let Some(generated) = Generated::new() else {
        return;
    };
    let available = Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "strict JSON typing test requires mypy"
    );
    if !available {
        eprintln!("Skipping JSON typing test: mypy not installed");
        return;
    }
    for (source, expected_errors) in [(VALID_CONSUMER, 0), (INVALID_CONSUMER, 9)] {
        let mut command = Command::new(python());
        command.args([
            "-m",
            "mypy",
            "--strict",
            "--no-incremental",
            "--no-pretty",
            "--cache-dir",
        ]);
        command.arg(generated.root.join("mypy-cache"));
        command.args(["-c", &source.replace("JSON_PACKAGE", &generated.package)]);
        generated.typing_environment(&mut command);
        let output = command.output().expect("run mypy");
        let diagnostics = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            diagnostics.matches(": error:").count(),
            expected_errors,
            "{diagnostics}"
        );
        assert_eq!(
            output.status.success(),
            expected_errors == 0,
            "{diagnostics}"
        );
    }

    if let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") {
        for (file, source, expected_errors) in [
            ("valid.py", VALID_CONSUMER, 0),
            ("invalid.py", INVALID_CONSUMER, 9),
        ] {
            let path = generated.root.join(file);
            fs::write(
                &path,
                format!(
                    "# pyright: strict\n{}",
                    source.replace("JSON_PACKAGE", &generated.package)
                ),
            )
            .unwrap();
            let mut command = Command::new(&pyright);
            command.args(["--pythonpath"]).arg(python()).arg(&path);
            generated.typing_environment(&mut command);
            let output = command.output().expect("run pyright");
            let diagnostics = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let error_lines = diagnostics
                .lines()
                .filter(|line| line.contains(" - error: "))
                .collect::<Vec<_>>();
            if expected_errors == 0 {
                assert!(error_lines.is_empty(), "{diagnostics}");
            } else {
                let mut locations = error_lines
                    .iter()
                    .filter_map(|line| line.split("invalid.py:").nth(1))
                    .filter_map(|location| location.split(':').next()?.parse::<usize>().ok())
                    .collect::<Vec<_>>();
                locations.sort_unstable();
                locations.dedup();
                assert_eq!(locations, (7..=15).collect::<Vec<_>>(), "{diagnostics}");
            }
            assert_eq!(
                output.status.success(),
                expected_errors == 0,
                "{diagnostics}"
            );
        }
    }
}
