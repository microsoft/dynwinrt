// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::TypeMeta;

const COLLECTIONS: &str = "Windows.Foundation.Collections";
const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

fn collections_type(name: &str, piid: &str, args: Vec<TypeMeta>) -> TypeMeta {
    TypeMeta::Parameterized {
        namespace: COLLECTIONS.into(),
        name: name.into(),
        piid: piid.into(),
        args,
    }
}

fn map_changed_handler(key: TypeMeta, value: TypeMeta) -> TypeMeta {
    collections_type(
        "MapChangedEventHandler`2",
        "179517f3-94ee-41f8-bddc-768a895544f3",
        vec![key, value],
    )
}

fn vector_changed_handler(element: TypeMeta) -> TypeMeta {
    collections_type(
        "VectorChangedEventHandler`1",
        "0c051752-9fbf-4c70-aa0c-0e4c82d9a761",
        vec![element],
    )
}

fn event_methods(event: &str, handler: TypeMeta) -> Vec<MethodMeta> {
    vec![
        MethodMeta {
            name: format!("add_{event}"),
            raw_name: format!("add_{event}"),
            vtable_index: 6,
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: handler,
                direction: ParamDirection::In,
            }],
            is_event_add: true,
            ..Default::default()
        },
        MethodMeta {
            name: format!("remove_{event}"),
            raw_name: format!("remove_{event}"),
            vtable_index: 7,
            params: vec![ParamMeta {
                name: "token".into(),
                typ: TypeMeta::I64,
                direction: ParamDirection::In,
            }],
            is_event_remove: true,
            ..Default::default()
        },
    ]
}

/// `IObservableMap<K, V>` whose `MapChanged` delegate carries the
/// `Invoke(IObservableMap<K, V> sender, IMapChangedEventArgs<K> event)` metadata.
fn observable_map(key: TypeMeta, value: TypeMeta, name: &str) -> InterfaceMeta {
    let handler = map_changed_handler(key.clone(), value.clone());
    let mut interface = InterfaceMeta {
        name: name.into(),
        namespace: COLLECTIONS.into(),
        iid: "65df2bf5-bf39-41b5-aebc-5a9d865e472b".into(),
        generic_piid: Some("65df2bf5-bf39-41b5-aebc-5a9d865e472b".into()),
        generic_args: vec![key.clone(), value.clone()],
        methods: event_methods("MapChanged", handler.clone()),
        ..Default::default()
    };
    interface
        .implementation_metadata
        .delegates
        .push(common::delegate_invoke(
            handler,
            &[
                (
                    "sender",
                    collections_type(
                        "IObservableMap`2",
                        "65df2bf5-bf39-41b5-aebc-5a9d865e472b",
                        vec![key.clone(), value],
                    ),
                ),
                (
                    "event",
                    collections_type(
                        "IMapChangedEventArgs`1",
                        "9939f4df-050a-4c0f-aa60-77075f9c4777",
                        vec![key],
                    ),
                ),
            ],
        ));
    interface
}

/// `IObservableVector<T>` whose `VectorChanged` delegate carries the
/// `Invoke(IObservableVector<T> sender, IVectorChangedEventArgs event)` metadata.
fn observable_vector(element: TypeMeta, name: &str) -> InterfaceMeta {
    let handler = vector_changed_handler(element.clone());
    let mut interface = InterfaceMeta {
        name: name.into(),
        namespace: COLLECTIONS.into(),
        iid: "5917eb53-50b4-4a0d-b309-65862b3f1dbc".into(),
        generic_piid: Some("5917eb53-50b4-4a0d-b309-65862b3f1dbc".into()),
        generic_args: vec![element.clone()],
        methods: event_methods("VectorChanged", handler.clone()),
        ..Default::default()
    };
    interface
        .implementation_metadata
        .delegates
        .push(common::delegate_invoke(
            handler,
            &[
                (
                    "sender",
                    collections_type(
                        "IObservableVector`1",
                        "5917eb53-50b4-4a0d-b309-65862b3f1dbc",
                        vec![element],
                    ),
                ),
                (
                    "event",
                    TypeMeta::Interface {
                        namespace: COLLECTIONS.into(),
                        name: "IVectorChangedEventArgs".into(),
                        iid: "575933df-34fe-4480-af15-07691f3d5d9b".into(),
                    },
                ),
            ],
        ));
    interface
}

fn string_object_map() -> InterfaceMeta {
    InterfaceMeta {
        name: "IMap_String_Object".into(),
        namespace: COLLECTIONS.into(),
        iid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
        generic_piid: Some("3c2925fe-8519-45c1-aa79-197b6718c1c1".into()),
        generic_args: vec![TypeMeta::String, TypeMeta::Object],
        methods: vec![MethodMeta {
            name: "get_Size".into(),
            raw_name: "get_Size".into(),
            vtable_index: 7,
            return_type: Some(TypeMeta::U32),
            is_property_getter: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn map_known_types() -> HashSet<String> {
    HashSet::from([
        "IObservableMap_String_Object".into(),
        "IMap_String_Object".into(),
        "IMapChangedEventArgs_String".into(),
    ])
}

fn map_delegates() -> HashSet<String> {
    HashSet::from(["MapChangedEventHandler_String_Object".into()])
}

const MAP_CALLBACK: &str =
    "Callable[['IObservableMap_String_Object', 'IMapChangedEventArgs_String'], object]";

#[test]
fn observable_map_projects_python_mutable_mapping_and_typed_events() {
    let interface = observable_map(
        TypeMeta::String,
        TypeMeta::Object,
        "IObservableMap_String_Object",
    );

    let py = common::generate_interface(&interface, &map_known_types(), &map_delegates());
    let map_base = "_dynwinrt_symbol('i_map_string_object', 'IMap_String_Object')";
    assert!(
        py.contains(&format!("class IObservableMap_String_Object({map_base}):")),
        "{py}"
    );
    assert!(
        py.contains(&format!("        {map_base}._set_native(self, obj)\n")),
        "{py}"
    );
    assert!(
        py.contains("self._observable_obj = obj.cast(IID_IObservableMap_String_Object)"),
        "{py}"
    );
    for helper in ["on", "subscribe", "once"] {
        assert!(
            py.contains(&format!(
                "def {helper}_map_changed(self, callback: {MAP_CALLBACK}):"
            )),
            "{py}"
        );
    }
    assert!(
        py.contains(
            "_wrapped = (lambda callback=callback: (lambda __sender__, __event__: callback(\
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('i_observable_map_string_object', 'IObservableMap_String_Object')(value))(__sender__), \
             (lambda value: None if value.is_null() else \
             _dynwinrt_symbol('i_map_changed_event_args_string', 'IMapChangedEventArgs_String')(value))(__event__))))()"
        ),
        "{py}"
    );
    assert!(!py.contains("_wrapped = callback\n"), "{py}");
    assert!(
        py.contains("_IObservableMap_String_Object.method(6).invoke(self._observable_obj"),
        "{py}"
    );
    assert!(
        py.contains("_IObservableMap_String_Object.method(7).invoke(self._observable_obj"),
        "{py}"
    );
    assert!(
        py.contains("import IMapChangedEventArgs_String  # noqa: F401"),
        "{py}"
    );

    let pyi = common::generate_interface_stub(&interface, &map_known_types(), &map_delegates());
    assert!(
        pyi.contains(
            "class IObservableMap_String_Object(_IObservableMap_String_ObjectIdentity, MutableMapping[str, DynWinRTValue | None]):"
        ),
        "{pyi}"
    );
    assert!(
        pyi.contains("    def __init__(self, obj: DynWinRTValue) -> None: ..."),
        "{pyi}"
    );
    assert!(pyi.contains("    def __len__(self) -> int: ..."), "{pyi}");
    assert!(
        pyi.contains("    def __getitem__(self, key: str) -> DynWinRTValue | None: ..."),
        "{pyi}"
    );
    assert!(
        pyi.contains("    def __delitem__(self, key: str) -> None: ..."),
        "{pyi}"
    );
    assert!(
        pyi.contains(&format!(
            "def on_map_changed(self, callback: {MAP_CALLBACK}) -> 'DynWinRTValue': ..."
        )),
        "{pyi}"
    );
    for helper in ["subscribe", "once"] {
        assert!(
            pyi.contains(&format!(
                "def {helper}_map_changed(self, callback: {MAP_CALLBACK}) -> Callable[[], None]: ..."
            )),
            "{pyi}"
        );
    }
    assert_eq!(
        pyi.matches("import IMapChangedEventArgs_String  # noqa: F401")
            .count(),
        1,
        "{pyi}"
    );
    assert!(!pyi.contains("Callable[..., object]"), "{pyi}");
}

fn runtime_class(name: &str, required_interfaces: Vec<InterfaceMeta>) -> ClassMeta {
    ClassMeta {
        name: name.into(),
        namespace: COLLECTIONS.into(),
        full_name: format!("{COLLECTIONS}.{name}"),
        default_interface: Some(InterfaceMeta {
            name: format!("I{name}"),
            namespace: COLLECTIONS.into(),
            iid: "8a43ed9f-f4e6-4421-acf9-1dab2986820c".into(),
            ..Default::default()
        }),
        required_interfaces,
        is_referenced_as_value: true,
        ..Default::default()
    }
}

#[test]
fn runtime_class_map_changed_events_project_observable_sender_and_arguments() {
    let class = runtime_class(
        "PropertySet",
        vec![
            observable_map(
                TypeMeta::String,
                TypeMeta::Object,
                "IObservableMap_String_Object",
            ),
            string_object_map(),
        ],
    );

    let py = common::generate_class(
        &class,
        &map_known_types(),
        &map_delegates(),
        &HashSet::new(),
    );
    for helper in ["on", "subscribe", "once"] {
        assert!(
            py.contains(&format!(
                "def {helper}_map_changed(self, callback: {MAP_CALLBACK}):"
            )),
            "{py}"
        );
    }
    assert!(
        py.contains(
            "_dynwinrt_symbol('i_observable_map_string_object', 'IObservableMap_String_Object')(value))(__sender__)"
        ),
        "{py}"
    );
    assert!(
        py.contains(
            "_dynwinrt_symbol('i_map_changed_event_args_string', 'IMapChangedEventArgs_String')(value))(__event__)"
        ),
        "{py}"
    );
    assert!(
        py.contains(
            "_IObservableMap_String_Object.method(6).invoke(self._obj.cast(IID_IObservableMap_String_Object)"
        ),
        "{py}"
    );
    let type_checking = py
        .split_once("if TYPE_CHECKING:\n")
        .map(|(_, rest)| rest.split_once("\n\n").map_or(rest, |(block, _)| block))
        .unwrap_or_else(|| panic!("missing TYPE_CHECKING imports:\n{py}"));
    for imported in [
        "import IObservableMap_String_Object  # noqa: F401",
        "import IMapChangedEventArgs_String  # noqa: F401",
    ] {
        assert!(type_checking.contains(imported), "{py}");
    }
    // The projected sender comes from the standalone observable-map module,
    // so the class module no longer embeds a protocol-less duplicate.
    assert!(!py.contains("\nclass IObservableMap_String_Object"), "{py}");
    assert!(
        py.contains("\nclass IMap_String_Object(_WinRTMutableMappingMixin):"),
        "{py}"
    );

    let pyi = common::generate_class_stub(
        &class,
        &map_known_types(),
        &map_delegates(),
        &HashSet::new(),
    );
    for imported in [
        "import IObservableMap_String_Object  # noqa: F401\n",
        "import IMapChangedEventArgs_String  # noqa: F401\n",
    ] {
        assert_eq!(pyi.matches(imported).count(), 1, "{pyi}");
    }
    assert!(
        !pyi.contains("\nclass IObservableMap_String_Object"),
        "{pyi}"
    );
    assert!(
        pyi.contains(&format!(
            "def on_map_changed(self, callback: {MAP_CALLBACK}) -> 'DynWinRTValue': ..."
        )),
        "{pyi}"
    );
    assert_eq!(
        pyi.matches(&format!(
            "def subscribe_map_changed(self, callback: {MAP_CALLBACK}) -> Callable[[], None]: ..."
        ))
        .count(),
        2,
        "the Like protocol and the class both expose the typed helper:\n{pyi}"
    );
    assert!(!pyi.contains("Callable[..., object]"), "{pyi}");
}

#[test]
fn runtime_class_vector_changed_events_import_observable_sender_and_arguments() {
    let class = runtime_class(
        "StringCollection",
        vec![
            observable_vector(TypeMeta::String, "IObservableVector_String"),
            InterfaceMeta {
                name: "IVector_String".into(),
                namespace: COLLECTIONS.into(),
                iid: "913337e9-11a1-4345-a3a2-4e7f956e222d".into(),
                generic_piid: Some("913337e9-11a1-4345-a3a2-4e7f956e222d".into()),
                generic_args: vec![TypeMeta::String],
                ..Default::default()
            },
        ],
    );
    let known_types = HashSet::from([
        "IObservableVector_String".into(),
        "IVector_String".into(),
        "IVectorChangedEventArgs".into(),
    ]);
    let delegates = HashSet::from(["VectorChangedEventHandler_String".into()]);
    let callback = "Callable[['IObservableVector_String', 'IVectorChangedEventArgs'], object]";

    let py = common::generate_class(&class, &known_types, &delegates, &HashSet::new());
    for helper in ["on", "subscribe", "once"] {
        assert!(
            py.contains(&format!(
                "def {helper}_vector_changed(self, callback: {callback}):"
            )),
            "{py}"
        );
    }
    assert!(
        py.contains(
            "_dynwinrt_symbol('i_observable_vector_string', 'IObservableVector_String')(value))(__sender__)"
        ),
        "{py}"
    );
    assert!(
        py.contains(
            "_dynwinrt_symbol('windows__foundation__collections__i_vector_changed_event_args', 'IVectorChangedEventArgs')(value))(__event__)"
        ),
        "{py}"
    );
    for imported in [
        "import IObservableVector_String  # noqa: F401",
        "IVectorChangedEventArgs  # noqa: F401",
    ] {
        assert!(py.contains(imported), "{py}");
    }
    assert!(!py.contains("\nclass IObservableVector_String"), "{py}");

    let pyi = common::generate_class_stub(&class, &known_types, &delegates, &HashSet::new());
    for imported in [
        "import IObservableVector_String  # noqa: F401\n",
        "IVectorChangedEventArgs  # noqa: F401\n",
    ] {
        assert_eq!(pyi.matches(imported).count(), 1, "{pyi}");
    }
    assert!(
        pyi.contains(&format!(
            "def once_vector_changed(self, callback: {callback}) -> Callable[[], None]: ..."
        )),
        "{pyi}"
    );
    assert!(!pyi.contains("\nclass IObservableVector_String"), "{pyi}");
    assert!(!pyi.contains("Callable[..., object]"), "{pyi}");
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Output(PathBuf);

impl Output {
    fn generate(classes: &str) -> Option<Self> {
        let winmd = Path::new(WINDOWS_WINMD);
        if !winmd.is_file() {
            eprintln!("Skipping Windows.winmd observable map checks: SDK metadata unavailable.");
            return None;
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!(
                "om{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(winmd)
            .args(["--class-name", classes, "--lang", "py", "--output"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Some(Self(path))
    }

    fn read(&self, file: &str) -> String {
        fs::read_to_string(self.0.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn windows_observable_maps_type_and_project_map_changed_handlers() {
    let Some(output) = Output::generate(
        "Windows.Foundation.Collections.PropertySet,Windows.Foundation.Collections.StringMap",
    ) else {
        return;
    };

    for (class, value) in [("property_set", "Object"), ("string_map", "String")] {
        let callback = format!(
            "Callable[['IObservableMap_String_{value}', 'IMapChangedEventArgs_String'], object]"
        );
        let observable_module = format!(
            "windows__foundation__collections__i_observable_map_string_{}",
            value.to_lowercase()
        );
        let sender = format!(
            "(lambda value: None if value.is_null() else _dynwinrt_symbol('{observable_module}', 'IObservableMap_String_{value}')(value))(__sender__)"
        );
        let args = "(lambda value: None if value.is_null() else _dynwinrt_symbol('windows__foundation__collections__i_map_changed_event_args_string', 'IMapChangedEventArgs_String')(value))(__event__)";

        let class_py = output.read(&format!("windows__foundation__collections__{class}.py"));
        let interface_py = output.read(&format!("{observable_module}.py"));
        for py in [&class_py, &interface_py] {
            for helper in ["on", "subscribe", "once"] {
                assert!(
                    py.contains(&format!(
                        "def {helper}_map_changed(self, callback: {callback}):"
                    )),
                    "{py}"
                );
            }
            assert!(py.contains(&sender), "{py}");
            assert!(py.contains(args), "{py}");
            assert!(!py.contains("_wrapped = callback\n"), "{py}");
        }
        assert!(
            interface_py.contains(&format!(
                "class IObservableMap_String_{value}(_dynwinrt_symbol('windows__foundation__collections__i_map_string_{}', 'IMap_String_{value}')):",
                value.to_lowercase()
            )),
            "{interface_py}"
        );
        assert!(
            class_py.contains(&format!(
                "import IObservableMap_String_{value}  # noqa: F401"
            )),
            "{class_py}"
        );

        let class_pyi = output.read(&format!("windows__foundation__collections__{class}.pyi"));
        let interface_pyi = output.read(&format!("{observable_module}.pyi"));
        for pyi in [&class_pyi, &interface_pyi] {
            assert!(
                pyi.contains(&format!(
                    "def on_map_changed(self, callback: {callback}) -> 'DynWinRTValue': ..."
                )),
                "{pyi}"
            );
            for helper in ["subscribe", "once"] {
                assert!(
                    pyi.contains(&format!(
                        "def {helper}_map_changed(self, callback: {callback}) -> Callable[[], None]: ..."
                    )),
                    "{pyi}"
                );
            }
            assert!(!pyi.contains("Callable[..., object]"), "{pyi}");
        }
        let python_value = if value == "Object" {
            "DynWinRTValue | None"
        } else {
            "str"
        };
        assert!(
            interface_pyi.contains(&format!(
                "class IObservableMap_String_{value}(_IObservableMap_String_{value}Identity, MutableMapping[str, {python_value}]):"
            )),
            "{interface_pyi}"
        );
        assert!(
            interface_pyi.contains(&format!(
                "def __getitem__(self, key: str) -> {python_value}: ..."
            )),
            "{interface_pyi}"
        );
    }
}

#[test]
fn windows_observable_map_returns_emit_their_mutable_map_base() {
    let Some(output) = Output::generate("Windows.ApplicationModel.Resources.Core.ResourceContext")
    else {
        return;
    };
    let observable =
        output.read("windows__foundation__collections__i_observable_map_string_string.py");
    assert!(
        observable.contains(
            "class IObservableMap_String_String(_dynwinrt_symbol('windows__foundation__collections__i_map_string_string', 'IMap_String_String')):"
        ),
        "{observable}"
    );
    let map = output.read("windows__foundation__collections__i_map_string_string.py");
    assert!(
        map.contains("class IMap_String_String(_WinRTMutableMappingMixin):"),
        "{map}"
    );
}
