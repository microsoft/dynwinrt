// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta, TypeRef};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!(
                "pc{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python"))
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn has_mypy() -> bool {
    let available = Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "DYNWINRT_REQUIRE_MYPY=1 but mypy is unavailable",
    );
    if !available {
        eprintln!("Skipping consumer checks: mypy unavailable; set DYNWINRT_TEST_PYTHON.");
    }
    available
}

fn has_implementation_runtime() -> bool {
    let available = Command::new(python())
        .args(["-c", "from dynwinrt import DynWinRTImplementationHandle"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME=1 but the matching runtime is unavailable"
    );
    if !available {
        eprintln!("Skipping native consumers: set DYNWINRT_TEST_PYTHON to the matching runtime.");
    }
    available
}

fn typecheck(fixture: &Fixture, packages: &[&str], consumer: &str, errors: &[&str]) {
    fs::write(fixture.0.join("consumer.py"), consumer).unwrap();
    let installed = std::env::var("DYNWINRT_TEST_INSTALLED_RUNTIME").as_deref() == Ok("1");
    for source_stubs in if installed {
        &[true, false][..]
    } else {
        &[true][..]
    } {
        let mut command = Command::new(python());
        command
            .args([
                "-B",
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--follow-imports=normal",
                "--no-pretty",
                "--show-error-codes",
                "--cache-dir",
                ".mypy_cache",
            ])
            .args(packages)
            .arg("consumer.py")
            .current_dir(&fixture.0);
        if *source_stubs {
            command.env(
                "MYPYPATH",
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("bindings")
                    .join("py"),
            );
        } else {
            command.env_remove("MYPYPATH");
        }
        let output = command.output().unwrap();
        let text = diagnostics(&output);
        if errors.is_empty() {
            assert!(output.status.success(), "{text}");
        } else {
            assert_eq!(output.status.code(), Some(1), "{text}");
            let actual = text
                .lines()
                .filter(|line| line.contains(": error:"))
                .collect::<Vec<_>>();
            assert_eq!(actual.len(), errors.len(), "{text}");
            for (line, code) in actual.iter().zip(errors) {
                assert!(line.starts_with("consumer.py:"), "{text}");
                assert!(line.ends_with(code), "{text}");
            }
        }
    }
}

fn pyright_typecheck(fixture: &Fixture, consumer: &str, expected_errors: usize) {
    let Some(pyright) = std::env::var_os("DYNWINRT_PYRIGHT") else {
        return;
    };
    fs::write(
        fixture.0.join("pyright_consumer.py"),
        format!("# pyright: strict\n{consumer}"),
    )
    .unwrap();
    let output = Command::new(pyright)
        .args(["--pythonpath"])
        .arg(python())
        .arg("pyright_consumer.py")
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    let text = diagnostics(&output);
    let actual = text
        .lines()
        .filter(|line| line.contains(" - error: "))
        .count();
    assert_eq!(actual, expected_errors, "{text}");
    assert_eq!(output.status.success(), expected_errors == 0, "{text}");
}

fn parameter(typ: TypeMeta) -> ParamMeta {
    ParamMeta {
        name: "value".into(),
        typ,
        direction: ParamDirection::In,
    }
}

fn interface(name: &str, id: u32, methods: Vec<MethodMeta>) -> InterfaceMeta {
    InterfaceMeta {
        namespace: "Contoso".into(),
        name: name.into(),
        iid: format!("{id:08x}-1111-1111-1111-111111111111"),
        methods,
        ..Default::default()
    }
}

fn class(name: &str, interface: InterfaceMeta) -> ClassMeta {
    ClassMeta {
        namespace: "Contoso".into(),
        name: name.into(),
        full_name: format!("Contoso.{name}"),
        default_interface: Some(interface),
        is_referenced_as_value: true,
        ..Default::default()
    }
}

fn generate_fixture(fixture: &Fixture, package: &str) {
    let named = vec![MethodMeta {
        name: "get_Name".into(),
        vtable_index: 6,
        is_property_getter: true,
        return_type: Some(TypeMeta::String),
        ..Default::default()
    }];
    let item = interface("IItem", 1, named.clone());
    let unrelated = interface("IUnrelated", 2, named);
    let closable = InterfaceMeta {
        namespace: "Windows.Foundation".into(),
        name: "IClosable".into(),
        iid: "30d5a829-7fa4-4026-83bb-d75bae4ea99e".into(),
        methods: vec![MethodMeta {
            name: "Close".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut resource = class("Resource", item.clone());
    resource.required_interfaces.push(closable.clone());
    let mut derived = class("DerivedResource", item.clone());
    derived.required_interfaces.push(closable.clone());
    derived.base_class = Some(TypeRef {
        namespace: "Contoso".into(),
        name: "Resource".into(),
        kind: TypeKind::Class,
    });
    let content = interface(
        "IContent",
        3,
        vec![
            MethodMeta {
                name: "get_Content".into(),
                vtable_index: 6,
                is_property_getter: true,
                return_type: Some(TypeMeta::Object),
                ..Default::default()
            },
            MethodMeta {
                name: "put_Content".into(),
                vtable_index: 7,
                is_property_setter: true,
                params: vec![parameter(TypeMeta::Object)],
                ..Default::default()
            },
            MethodMeta {
                name: "SetObject".into(),
                vtable_index: 8,
                params: vec![parameter(TypeMeta::Object)],
                ..Default::default()
            },
            MethodMeta {
                name: "SetObjects".into(),
                vtable_index: 9,
                params: vec![parameter(TypeMeta::Array(Box::new(TypeMeta::Object)))],
                ..Default::default()
            },
            MethodMeta {
                name: "GetResources".into(),
                raw_name: "GetResources".into(),
                vtable_index: 10,
                return_type: Some(TypeMeta::Array(Box::new(TypeMeta::RuntimeClass {
                    namespace: "Contoso".into(),
                    name: "Resource".into(),
                    default_interface: Some(Box::new(TypeMeta::Interface {
                        namespace: "Contoso".into(),
                        name: "IItem".into(),
                        iid: "00000001-1111-1111-1111-111111111111".into(),
                    })),
                }))),
                ..Default::default()
            },
            MethodMeta {
                name: "GetCounts".into(),
                raw_name: "GetCounts".into(),
                vtable_index: 11,
                return_type: Some(TypeMeta::Array(Box::new(TypeMeta::I32))),
                ..Default::default()
            },
            MethodMeta {
                name: "GetItems".into(),
                raw_name: "GetItems".into(),
                vtable_index: 12,
                return_type: Some(TypeMeta::Array(Box::new(TypeMeta::Interface {
                    namespace: "Contoso".into(),
                    name: "IItem".into(),
                    iid: "00000001-1111-1111-1111-111111111111".into(),
                }))),
                ..Default::default()
            },
        ],
    );
    let classes = [
        resource,
        derived,
        class("Unrelated", unrelated.clone()),
        class("Content", content.clone()),
    ];
    let resource_type = TypeMeta::RuntimeClass {
        namespace: "Contoso".into(),
        name: "Resource".into(),
        default_interface: None,
    };
    let mut interfaces = vec![item, unrelated, closable, content];
    for (name, definition, piid, args) in [
        (
            "IVector_Object",
            "IVector`1",
            "913337e9-11a1-4345-a3a2-4e7f956e222d",
            vec![TypeMeta::Object],
        ),
        (
            "IVector_Resource",
            "IVector`1",
            "913337e9-11a1-4345-a3a2-4e7f956e222d",
            vec![resource_type.clone()],
        ),
        (
            "IMap_Object_Object",
            "IMap`2",
            "3c2925fe-8519-45c1-aa79-197b6718c1c1",
            vec![TypeMeta::Object, TypeMeta::Object],
        ),
        (
            "IMap_String_Object",
            "IMap`2",
            "3c2925fe-8519-45c1-aa79-197b6718c1c1",
            vec![TypeMeta::String, TypeMeta::Object],
        ),
        (
            "IMap_Resource_Resource",
            "IMap`2",
            "3c2925fe-8519-45c1-aa79-197b6718c1c1",
            vec![resource_type.clone(), resource_type],
        ),
        (
            "IMap_Guid_Object",
            "IMap`2",
            "3c2925fe-8519-45c1-aa79-197b6718c1c1",
            vec![TypeMeta::Guid, TypeMeta::Object],
        ),
    ] {
        let methods = if definition == "IVector`1" {
            vec![
                MethodMeta {
                    name: "GetAt".into(),
                    vtable_index: 6,
                    params: vec![ParamMeta {
                        name: "index".into(),
                        typ: TypeMeta::U32,
                        direction: ParamDirection::In,
                    }],
                    return_type: Some(args[0].clone()),
                    ..Default::default()
                },
                MethodMeta {
                    name: "Append".into(),
                    vtable_index: 13,
                    params: vec![parameter(args[0].clone())],
                    ..Default::default()
                },
            ]
        } else if definition == "IMap`2" {
            vec![MethodMeta {
                name: "Insert".into(),
                raw_name: "Insert".into(),
                vtable_index: 10,
                params: vec![
                    ParamMeta {
                        name: "key".into(),
                        typ: args[0].clone(),
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "value".into(),
                        typ: args[1].clone(),
                        direction: ParamDirection::In,
                    },
                ],
                return_type: Some(TypeMeta::Bool),
                collection_inputs: vec![
                    (0, dynwinrt_codegen::meta::CollectionInputRole::Key),
                    (1, dynwinrt_codegen::meta::CollectionInputRole::Value),
                ],
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        interfaces.push(InterfaceMeta {
            namespace: "Windows.Foundation.Collections".into(),
            name: name.into(),
            generic_name: Some(definition.into()),
            generic_piid: Some(piid.into()),
            generic_args: args,
            methods,
            ..Default::default()
        });
    }
    let identities = classes
        .iter()
        .map(|class| TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name))
        .chain(interfaces.iter().map(InterfaceMeta::type_identity));
    let context = python::PythonProjectionContext::new(identities, true).unwrap();
    let directory = fixture.0.join(package);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("__init__.py"), "").unwrap();
    fs::write(directory.join("__init__.pyi"), "").unwrap();
    fs::write(
        directory.join("_runtime.py"),
        python::generate_runtime_support_module(),
    )
    .unwrap();
    fs::write(
        directory.join("_runtime.pyi"),
        python_stub::generate_runtime_support_stub(),
    )
    .unwrap();
    fs::write(
        directory.join("_typing.pyi"),
        python_stub::generate_typing_support_module(),
    )
    .unwrap();
    fs::write(
        directory.join("_implementation_types.pyi"),
        python_stub::generate_implementation_pair_types(&[]),
    )
    .unwrap();
    let shared = interfaces.iter().map(|iface| iface.iid.clone()).collect();
    for iface in interfaces {
        let module = context.implementation_module(&iface.type_identity());
        fs::write(
            directory.join(format!("{module}.py")),
            python::generate_interface(&context, &iface),
        )
        .unwrap();
        fs::write(
            directory.join(format!("{module}.pyi")),
            python_stub::generate_interface_stub(&context, &iface),
        )
        .unwrap();
    }
    for class in classes {
        let identity = TypeIdentity::named(TypeIdentityKind::Class, &class.namespace, &class.name);
        let module = context.implementation_module(&identity);
        fs::write(
            directory.join(format!("{module}.py")),
            python::generate_class(&context, &class, &shared),
        )
        .unwrap();
        fs::write(
            directory.join(format!("{module}.pyi")),
            python_stub::generate_class_stub(&context, &class, &shared),
        )
        .unwrap();
    }
}

#[test]
fn strict_consumers_separate_instances_factories_and_native_object_inputs() {
    if !has_mypy() {
        return;
    }
    let fixture = Fixture::new();
    generate_fixture(&fixture, "first");
    generate_fixture(&fixture, "second");
    typecheck(
        &fixture,
        &["first", "second"],
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue, _DynWinRTProjector
from first.contoso__i_item import IItem
from first.contoso__resource import Resource, ResourceLike
from first.contoso__derived_resource import DerivedResource
from first.contoso__content import Content
from first.windows__foundation__collections__i_map_object_object import IMap_Object_Object
from first.windows__foundation__collections__i_vector_object import IVector_Object
from first.windows__foundation__collections__i_vector_resource import IVector_Resource
from second.contoso__i_item import IItem as OtherItem
from second.contoso__derived_resource import DerivedResource as OtherDerived

def use_item(value: IItem) -> str:
    return value.name

def use_base(value: ResourceLike) -> str:
    with value as entered:
        assert_type(entered, ResourceLike)
        return entered.name

def valid(raw: DynWinRTValue, resource: Resource, derived: DerivedResource,
          other: OtherDerived, item: OtherItem, content: Content) -> None:
    use_item(resource)
    use_item(derived)
    use_item(other)
    use_item(item)
    use_base(derived)
    use_base(other)
    with derived as entered:
        assert_type(entered, DerivedResource)
    projector: _DynWinRTProjector[IItem] = IItem
    assert_type(projector.from_value(raw), IItem)
    assert_type(IItem.from_value(raw), IItem)
    assert_type(resource.as_interface(IItem), IItem)
    assert_type(resource.as_interface(OtherItem), OtherItem)
    assert_type(content.content, DynWinRTValue | None)
    content.content = resource
    content.content = other
    content.content = item
    content.content = raw
    content.set_object(derived)
    content.set_objects([resource, other, item, raw])

def collections(resource: Resource, derived: OtherDerived, raw: DynWinRTValue,
                vector: IVector_Object, resources: IVector_Resource,
                mapping: IMap_Object_Object) -> None:
    vector[0] = resource
    vector[1:2] = [resource, derived, raw]
    vector.insert(0, derived)
    vector.append(derived)
    resources[0] = derived
    resources[1:2] = [resource, derived]
    resources.insert(0, derived)
    resources.append(derived)
    mapping[resource] = derived
    assert_type(vector[0], DynWinRTValue | None)
    assert_type(vector[:], list[DynWinRTValue | None])
    assert_type(resources[0], Resource | None)
    assert_type(mapping[resource], DynWinRTValue | None)
    del mapping[resource]
"#,
        &[],
    );
    typecheck(
        &fixture,
        &["first", "second"],
        r#"from dynwinrt import DynWinRTValue, DynWinRtDelegate
from first.contoso__i_item import IItem
from first.contoso__i_unrelated import IUnrelated
from first.contoso__resource import Resource, ResourceLike
from first.contoso__unrelated import Unrelated
from first.contoso__content import Content
from first.windows__foundation__collections__i_map_object_object import IMap_Object_Object
from first.windows__foundation__collections__i_vector_object import IVector_Object
from first.windows__foundation__collections__i_vector_resource import IVector_Resource
from second.contoso__unrelated import Unrelated as OtherUnrelated

class WrongObject:
    _obj: int = 42

def use_item(value: IItem) -> None:
    pass

def use_base(value: ResourceLike) -> None:
    pass

def invalid(raw: DynWinRTValue, resource: Resource, unrelated: Unrelated,
            other: OtherUnrelated, interface: IUnrelated, content: Content) -> None:
    use_item(unrelated)
    use_item(other)
    use_item(interface)
    use_base(unrelated)
    resource.as_interface(Resource)
    content.content = object()
    content.content = "not boxed"
    content.content = None
    content.content = Resource
    content.set_object(WrongObject())
    content.set_objects([42])

def invalid_collections(resource: Resource, unrelated: Unrelated,
                       vector: IVector_Object, resources: IVector_Resource,
                       mapping: IMap_Object_Object) -> None:
    vector[0] = object()
    vector[0] = [resource]
    vector[:] = resource
    vector[:] = [42]
    resources[0] = unrelated
    resources.append(unrelated)
    mapping[WrongObject()] = resource
    mapping[resource] = object()
"#,
        &[
            "[arg-type]",
            "[arg-type]",
            "[arg-type]",
            "[arg-type]",
            "[arg-type]",
            "[assignment]",
            "[assignment]",
            "[assignment]",
            "[assignment]",
            "[arg-type]",
            "[list-item]",
            "[call-overload]",
            "[call-overload]",
            "[call-overload]",
            "[list-item]",
            "[call-overload]",
            "[arg-type]",
            "[index]",
            "[assignment]",
        ],
    );
    typecheck(
        &fixture,
        &["first"],
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from first.contoso__resource import Resource
from first.windows__foundation__collections__i_map_object_object import IMap_Object_Object
from first.windows__foundation__collections__i_map_resource_resource import IMap_Resource_Resource

def nullable_reference_keys(
    objects: IMap_Object_Object, resources: IMap_Resource_Resource
) -> None:
    objects[None] = None
    assert_type(objects[None], DynWinRTValue | None)
    resources[None] = None
    resources.update({None: None})
    assert_type(resources.setdefault(None, None), Resource | None)
    assert_type(resources[None], Resource | None)
"#,
        &[],
    );
    pyright_typecheck(
        &fixture,
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from first.contoso__resource import Resource
from first.windows__foundation__collections__i_map_object_object import IMap_Object_Object
from first.windows__foundation__collections__i_map_resource_resource import IMap_Resource_Resource

def nullable_reference_keys(
    objects: IMap_Object_Object, resources: IMap_Resource_Resource
) -> None:
    objects[None] = None
    assert_type(objects[None], DynWinRTValue | None)
    resources[None] = None
    resources.update({None: None})
    assert_type(resources.setdefault(None, None), Resource | None)
    assert_type(resources[None], Resource | None)
"#,
        0,
    );
    typecheck(
        &fixture,
        &["first"],
        r#"from first.windows__foundation__collections__i_map_guid_object import IMap_Guid_Object
from first.windows__foundation__collections__i_map_string_object import IMap_String_Object
from uuid import UUID

def invalid_key(guid: IMap_Guid_Object, string: IMap_String_Object) -> None:
    guid[None] = None
    guid.get(None)
    guid.update({None: None})
    guid.setdefault(None, None)
    string[None] = None
    string.get(None)
    string.update({None: None})
    string.setdefault(None, None)
"#,
        &[
            "[index]",
            "[call-overload]",
            "[dict-item]",
            "[call-overload]",
            "[index]",
            "[call-overload]",
            "[dict-item]",
            "[call-overload]",
        ],
    );
    pyright_typecheck(
        &fixture,
        r#"from first.windows__foundation__collections__i_map_guid_object import IMap_Guid_Object
from first.windows__foundation__collections__i_map_string_object import IMap_String_Object

def invalid_key(guid: IMap_Guid_Object, string: IMap_String_Object) -> None:
    guid[None] = None
    guid.get(None)
    guid.update({None: None})
    guid.setdefault(None, None)
    string[None] = None
    string.get(None)
    string.update({None: None})
    string.setdefault(None, None)
"#,
        12,
    );
}

#[test]
fn reference_array_results_preserve_null_elements() {
    if !has_mypy() {
        return;
    }
    let fixture = Fixture::new();
    generate_fixture(&fixture, "views");
    let source = fs::read_to_string(fixture.0.join("views").join("contoso__content.py")).unwrap();
    assert!(source.contains("def get_resources(self) -> list[Resource | None]:"));
    assert!(source.contains("_dynwinrt_wrap_values('contoso__resource', 'Resource'"));
    assert!(source.contains("def get_items(self) -> list[IItem | None]:"));
    assert!(source.contains("_dynwinrt_wrap_values('contoso__i_item', 'IItem'"));
    assert!(source.contains("def get_counts(self) -> list[int]:"));
    let valid = r#"from typing import assert_type
from views.contoso__content import Content
from views.contoso__i_item import IItem
from views.contoso__resource import Resource

def consume(content: Content) -> list[str]:
    resources = content.get_resources()
    assert_type(resources, list[Resource | None])
    assert_type(content.get_items(), list[IItem | None])
    assert_type(content.get_counts(), list[int])
    return [resource.name for resource in resources if resource is not None]
"#;
    typecheck(&fixture, &["views"], valid, &[]);
    pyright_typecheck(&fixture, valid, 0);

    let invalid = r#"from views.contoso__content import Content

def consume(content: Content) -> str:
    return content.get_resources()[0].name
"#;
    typecheck(&fixture, &["views"], invalid, &["[union-attr]"]);
    pyright_typecheck(&fixture, invalid, 1);

    if has_implementation_runtime() {
        let script = r#"from dynwinrt import RoApartment, project_as, projected_lifetime_scope
from views.contoso__content import Content
from views.contoso__i_content import IContent

class ContentHandlers:
    def get_content(self):
        return None

    def set_content(self, _value):
        pass

    def set_object(self, _value):
        pass

    def set_objects(self, _value):
        pass

    def get_resources(self):
        return [None]

    def get_counts(self):
        return [1, 2]

    def get_items(self):
        return [None]

with RoApartment(1), projected_lifetime_scope():
    with IContent.implement(ContentHandlers()) as owner:
        content = project_as(owner.value, Content)
        resources = content.get_resources()
        assert isinstance(resources, list)
        assert resources == [None]
        items = content.get_items()
        assert isinstance(items, list)
        assert items == [None]
        counts = content.get_counts()
        assert isinstance(counts, list)
        assert counts == [1, 2]
print("nullable-reference-array-native-ok", flush=True)
"#;
        fs::write(fixture.0.join("reference_array_runtime.py"), script).unwrap();
        let output = Command::new(python())
            .args(["-B", "reference_array_runtime.py"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("nullable-reference-array-native-ok"),
            "{}",
            diagnostics(&output)
        );
    }
}

#[test]
fn real_windows_consumers_accept_file_stream_content_and_composition_instances() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping SDK consumers: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Storage.FileIO,Windows.Storage.StorageFile,\
             Windows.Media.Playback.MediaPlayer,Windows.Media.SpeechSynthesis.SpeechSynthesisStream,\
             Windows.UI.Xaml.Controls.Button,Windows.UI.Xaml.Controls.TextBlock,\
             Windows.UI.Composition.ExpressionAnimation,Windows.UI.Composition.ContainerVisual",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    typecheck(
        &fixture,
        &["sdk"],
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from sdk.windows.storage import FileIO, StorageFile
from sdk.windows.media.playback import MediaPlayer
from sdk.windows.media.speech_synthesis import SpeechSynthesisStream
from sdk.windows.storage.streams import Buffer, IBuffer
from sdk.windows.ui.xaml.controls import Button, TextBlock
from sdk.windows.ui.composition import ContainerVisual, ExpressionAnimation

async def file_io(file: StorageFile) -> str:
    await FileIO.write_text_async(file, "hello")
    await FileIO.append_text_async(file, " world")
    return await FileIO.read_text_async(file)

def stream_source(player: MediaPlayer, stream: SpeechSynthesisStream) -> None:
    player.set_stream_source(stream)

def buffer_view(buffer: Buffer, data: bytes) -> IBuffer:
    assert_type(IBuffer.from_bytes(data), IBuffer)
    assert_type(buffer.as_interface(IBuffer), IBuffer)
    return buffer

def content(button: Button, text: TextBlock) -> None:
    assert_type(button.content, DynWinRTValue | None)
    button.content = text

def reference(animation: ExpressionAnimation, visual: ContainerVisual) -> None:
    animation.set_reference_parameter("target", visual)
    with visual as entered:
        assert_type(entered, ContainerVisual)
"#,
        &[],
    );
}

#[test]
fn interface_factory_preserves_subclass_types_and_runtime_identity() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping SDK factories: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Storage.Streams.Buffer",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    let prelude = r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from sdk.windows.storage.streams import Buffer, IBuffer

class TaggedBuffer(IBuffer):
    def tag(self) -> str:
        return "tagged"

class SpecializedBuffer(TaggedBuffer):
    pass
"#;
    typecheck(
        &fixture,
        &["sdk"],
        &format!(
            r#"{prelude}
from dynwinrt import _DynWinRTProjector

def valid(raw: DynWinRTValue, buffer: Buffer, factory: type[TaggedBuffer]) -> None:
    assert_type(IBuffer.from_value(raw), IBuffer)
    assert_type(TaggedBuffer.from_value(raw), TaggedBuffer)
    assert_type(SpecializedBuffer.from_value(raw), SpecializedBuffer)
    assert_type(factory.from_value(raw), TaggedBuffer)
    assert_type(TaggedBuffer.from_value(raw).tag(), str)
    assert_type(buffer.as_interface(IBuffer), IBuffer)
    assert_type(buffer.as_interface(TaggedBuffer), TaggedBuffer)
    assert_type(buffer.as_interface(SpecializedBuffer), SpecializedBuffer)
    projector: _DynWinRTProjector[TaggedBuffer] = TaggedBuffer
    assert_type(projector.from_value(raw), TaggedBuffer)
    assert_type(TaggedBuffer.from_bytes(b"static factory"), IBuffer)
"#
        ),
        &[],
    );
    typecheck(
        &fixture,
        &["sdk"],
        &format!(
            r#"{prelude}
def invalid(raw: DynWinRTValue, buffer: Buffer) -> None:
    TaggedBuffer.from_value(object())
    tagged: TaggedBuffer = IBuffer.from_value(raw)
    specialized: SpecializedBuffer = TaggedBuffer.from_value(raw)
    IBuffer.from_value(raw).tag()
    static_result: TaggedBuffer = TaggedBuffer.from_bytes(b"base result")
    buffer.as_interface(Buffer)
"#
        ),
        &[
            "[arg-type]",
            "[assignment]",
            "[assignment]",
            "[attr-defined]",
            "[assignment]",
            "[arg-type]",
        ],
    );
    if has_implementation_runtime() {
        fs::write(
            fixture.0.join("factory_runtime.py"),
            format!(
                r#"{prelude}
from dynwinrt import RoApartment, projected_lifetime_scope

with RoApartment(1), projected_lifetime_scope():
    buffer = Buffer.from_bytes(b"subclass factory")
    base = IBuffer.from_value(buffer._obj)
    tagged = TaggedBuffer.from_value(buffer._obj)
    specialized = SpecializedBuffer.from_value(buffer._obj)
    assert type(base) is IBuffer
    assert type(tagged) is TaggedBuffer
    assert type(specialized) is SpecializedBuffer
    assert tagged.tag() == "tagged"
    assert specialized.to_bytes() == b"subclass factory"
    assert TaggedBuffer.from_value(tagged._obj) is tagged
    assert buffer.as_interface(TaggedBuffer) is tagged
    assert buffer.as_interface(SpecializedBuffer) is specialized
    assert type(TaggedBuffer.from_bytes(b"static factory")) is IBuffer
print("subclass-factory-native-ok", flush=True)
"#
            ),
        )
        .unwrap();
        let output = Command::new(python())
            .args(["-B", "factory_runtime.py"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("subclass-factory-native-ok"),
            "{}",
            diagnostics(&output)
        );
    }
}

#[test]
fn collection_subscripts_accept_projected_inputs_and_keep_raw_outputs() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping SDK collections: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Foundation.Collections.PropertySet,Windows.Foundation.Uri",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    let imports = r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from sdk.windows.foundation import Uri
from sdk.windows.foundation.collections import IMap_String_Object, PropertySet
from sdk.windows__foundation__collections__property_set import (
    IMap_String_Object as EmbeddedMap, PropertySetLike,
)
"#;
    typecheck(
        &fixture,
        &["sdk"],
        &format!(
            r#"{imports}
def valid(uri: Uri, raw: DynWinRTValue, properties: PropertySet,
          like: PropertySetLike, mapping: IMap_String_Object,
          embedded: EmbeddedMap) -> None:
    properties["uri"] = uri
    properties["raw"] = raw
    like["uri"] = uri
    mapping["uri"] = uri
    embedded["uri"] = uri
    assert_type(properties["uri"], DynWinRTValue | None)
    assert_type(like["uri"], DynWinRTValue | None)
    assert_type(mapping["uri"], DynWinRTValue | None)
    assert_type(embedded["uri"], DynWinRTValue | None)
    del properties["uri"]
    del mapping["uri"]
"#
        ),
        &[],
    );
    typecheck(
        &fixture,
        &["sdk"],
        &format!(
            r#"{imports}
class WrongObject:
    _obj: int = 42

def invalid(uri: Uri, properties: PropertySet, mapping: IMap_String_Object) -> None:
    properties["plain"] = object()
    properties["string"] = "unboxed"
    properties["wrong"] = WrongObject()
    mapping["number"] = 42
    mapping["class"] = Uri
    mapping[42] = uri
    result: Uri = properties["uri"]
"#
        ),
        &[
            "[assignment]",
            "[assignment]",
            "[assignment]",
            "[assignment]",
            "[assignment]",
            "[index]",
            "[assignment]",
        ],
    );
    if has_implementation_runtime() {
        fs::write(
            fixture.0.join("collections_runtime.py"),
            r#"from dynwinrt import DynWinRTValue, RoApartment, projected_lifetime_scope
from sdk.windows.foundation import Uri
from sdk.windows.foundation.collections import IMap_String_Object, PropertySet

with RoApartment(1), projected_lifetime_scope():
    uri = Uri("https://example.com/collection-input")
    properties = PropertySet()
    mapping = IMap_String_Object.create({"uri": uri})
    for collection in (properties, mapping):
        collection["uri"] = uri
        value = collection["uri"]
        assert isinstance(value, DynWinRTValue)
        assert value.identity_raw() == uri._obj.identity_raw()
        value.release()
        collection["null"] = DynWinRTValue.null_value()
        assert collection["null"] is None
        collection.insert("raw", uri._obj)
        value = collection["raw"]
        assert isinstance(value, DynWinRTValue)
        assert value.identity_raw() == uri._obj.identity_raw()
        value.release()
        del collection["uri"]
        assert not collection.has_key("uri")
print("collection-subscript-native-ok", flush=True)
"#,
        )
        .unwrap();
        let output = Command::new(python())
            .args(["-B", "collections_runtime.py"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("collection-subscript-native-ok"),
            "{}",
            diagnostics(&output)
        );
    }
}

#[test]
fn thread_pool_abi_names_keep_precise_callable_and_native_inputs() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping ThreadPool typing: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.System.Threading.ThreadPool",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    typecheck(
        &fixture,
        &["sdk"],
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue, DynWinRtDelegate, WinRTCoroutine
from sdk.windows.system.threading import ThreadPool, WorkItemOptions, WorkItemPriority

def supported(native: DynWinRtDelegate, raw: DynWinRTValue) -> None:
    first: WinRTCoroutine[None] = ThreadPool.run_async(
        lambda action: assert_type(action, DynWinRTValue)
    )
    second: WinRTCoroutine[None] = ThreadPool.run_with_priority_async(
        lambda action: assert_type(action, DynWinRTValue), WorkItemPriority.Normal
    )
    third: WinRTCoroutine[None] = ThreadPool.run_with_priority_and_options_async(
        lambda action: assert_type(action, DynWinRTValue),
        WorkItemPriority.Normal, WorkItemOptions.TimeSliced,
    )
    ThreadPool.run_async(native)
    ThreadPool.run_with_priority_async(raw, WorkItemPriority.Normal)
    ThreadPool.run_with_priority_and_options_async(
        native, WorkItemPriority.Normal, WorkItemOptions.TimeSliced
    )
    _ = first, second, third
"#,
        &[],
    );
    typecheck(
        &fixture,
        &["sdk"],
        r#"from dynwinrt import DynWinRtDelegate
from sdk.windows.system.threading import ThreadPool, WorkItemOptions, WorkItemPriority

def invalid(native: DynWinRtDelegate) -> None:
    ThreadPool.run_async(native, WorkItemPriority.Normal)
    ThreadPool.run_async(native, WorkItemPriority.Normal, WorkItemOptions.TimeSliced)
    ThreadPool.run_async(handler=native, priority=WorkItemPriority.Normal)
    ThreadPool.run_async(
        handler=native, priority=WorkItemPriority.Normal, options=WorkItemOptions.TimeSliced
    )
"#,
        &[
            "[call-arg]",
            "[call-arg]",
            "[call-arg]",
            "[call-arg]",
            "[call-arg]",
        ],
    );
}

#[test]
fn map_changed_handlers_receive_typed_observable_maps_and_arguments() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping SDK map events: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Foundation.Collections.PropertySet,Windows.Foundation.Collections.StringMap,\
             Windows.Foundation.PropertyValue",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    let valid = r#"from typing import assert_type
from dynwinrt import DynWinRTValue, DynWinRtDelegate
from sdk.windows.foundation.collections import (
    CollectionChange, IMapChangedEventArgs_String, IObservableMap_String_Object,
    IObservableMap_String_String, PropertySet, StringMap,
)

def typed(
    properties: PropertySet,
    strings: StringMap,
    native: DynWinRtDelegate,
    raw: DynWinRTValue,
) -> None:
    def on_properties(
        sender: IObservableMap_String_Object, args: IMapChangedEventArgs_String
    ) -> None:
        assert_type(sender, IObservableMap_String_Object)
        assert_type(args, IMapChangedEventArgs_String)
        assert_type(len(sender), int)
        assert_type(sender[args.key], DynWinRTValue | None)
        assert_type(next(iter(sender.values())), DynWinRTValue | None)
        assert_type(args.collection_change, CollectionChange)

    assert_type(properties.lookup("key"), DynWinRTValue | None)
    properties.subscribe_map_changed(on_properties)
    properties.once_map_changed(
        lambda sender, args: assert_type(sender, IObservableMap_String_Object)
    )
    token = properties.on_map_changed(
        lambda sender, args: assert_type(args, IMapChangedEventArgs_String)
    )
    properties.off_map_changed(token)
    properties.off_map_changed(properties.on_map_changed(native))
    properties.subscribe_map_changed(raw)()
    strings.subscribe_map_changed(
        lambda sender, args: assert_type(sender, IObservableMap_String_String)
    )
    strings.subscribe_map_changed(lambda sender, args: assert_type(sender[args.key], str))
"#;
    typecheck(&fixture, &["sdk"], valid, &[]);
    pyright_typecheck(&fixture, valid, 0);

    let invalid = r#"from dynwinrt import DynWinRtDelegate
from sdk.windows.foundation.collections import PropertySet, StringMap

def wrong_sender(sender: int, args: object) -> None: ...
def wrong_args(sender: object, args: str) -> None: ...

def untyped(
    properties: PropertySet,
    strings: StringMap,
    native: DynWinRtDelegate,
) -> None:
    properties.subscribe_map_changed(wrong_sender)
    strings.once_map_changed(wrong_args)
    properties.on_map_changed(lambda sender, args: args.index)
    strings.subscribe_map_changed(lambda sender, args: sender[0])
    strings.once_map_changed(native)
"#;
    typecheck(
        &fixture,
        &["sdk"],
        invalid,
        &[
            "[arg-type]",
            "[arg-type]",
            "[attr-defined]",
            "[index]",
            "[arg-type]",
        ],
    );
    pyright_typecheck(&fixture, invalid, 7);

    if has_implementation_runtime() {
        fs::write(
            fixture.0.join("map_events_runtime.py"),
            r#"from collections.abc import Callable
from typing import get_type_hints

from dynwinrt import (
    DynWinRTValue, DynWinRtDelegate, RoApartment, projected_lifetime_scope,
)
from sdk.windows.foundation import PropertyValue
from sdk.windows.foundation.collections import (
    CollectionChange, IMapChangedEventArgs_String, IObservableMap_String_Object,
    IObservableMap_String_String, PropertySet, StringMap,
)

callback_annotation = (
    Callable[..., object] | DynWinRTValue | DynWinRtDelegate
)
assert get_type_hints(PropertySet.on_map_changed)["callback"] == callback_annotation
assert get_type_hints(PropertySet.subscribe_map_changed)["callback"] == callback_annotation
assert get_type_hints(PropertySet.once_map_changed)["callback"] == Callable[..., object]

with RoApartment(1), projected_lifetime_scope():
    properties = PropertySet()
    changes = []

    def on_properties(sender, args):
        assert isinstance(sender, IObservableMap_String_Object), type(sender)
        assert isinstance(args, IMapChangedEventArgs_String), type(args)
        value = sender[args.key]
        looked_up = sender.lookup(args.key)
        assert (looked_up is None) == (value is None)
        if value is not None:
            assert looked_up.identity_raw() == value.identity_raw()
            looked_up.release()
        listed = list(sender.values())
        assert (listed[-1] is None) == (value is None)
        changes.append((
            type(sender).__name__,
            type(args).__name__,
            type(value).__name__ if value is not None else None,
            value is None,
        ))
        if listed[-1] is not None:
            listed[-1].release()
        if value is not None:
            value.release()

    unsubscribe = properties.subscribe_map_changed(on_properties)
    properties["null"] = None
    properties["null"] = PropertyValue.create_int32(1)
    unsubscribe()
    assert changes[0] == (
        "IObservableMap_String_Object",
        "IMapChangedEventArgs_String",
        None,
        True,
    ), changes
    assert changes[1][0:2] == (
        "IObservableMap_String_Object",
        "IMapChangedEventArgs_String",
    ), changes
    assert changes[1][2:] == ("DynWinRTValue", False), changes

    strings = StringMap()
    string_changes = []
    stop = strings.subscribe_map_changed(
        lambda sender, args: string_changes.append(
            (type(sender).__name__, type(args).__name__, sender[args.key])
        )
    )
    strings["k"] = "value"
    stop()
    assert string_changes == [
        ("IObservableMap_String_String", "IMapChangedEventArgs_String", "value")
    ], string_changes

print("map-changed-native-ok", changes, string_changes, flush=True)
"#,
        )
        .unwrap();
        let output = Command::new(python())
            .args(["-B", "map_events_runtime.py"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("map-changed-native-ok"), "{stdout}");
        assert!(
            stdout.contains(
                "('IObservableMap_String_Object', 'IMapChangedEventArgs_String', None, True)"
            ),
            "{stdout}"
        );
        assert!(stdout.contains("'DynWinRTValue', False"), "{stdout}");
    }
}

#[test]
fn natural_sdk_consumers_guard_only_nullable_results() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping natural SDK consumers: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Foundation.Uri,Windows.Foundation.Collections.PropertySet,\
             Windows.Data.Json.JsonObject,Windows.Globalization.Calendar,\
             Windows.Security.Cryptography.CryptographicBuffer,\
             Windows.Security.Cryptography.Core.HashAlgorithmProvider,\
             Windows.Storage.StorageFolder,Windows.Storage.FileIO,\
             Windows.Storage.Streams.DataReader,Windows.Storage.Streams.DataWriter,\
             Windows.Storage.Streams.InMemoryRandomAccessStream,\
             Windows.Devices.Sensors.Accelerometer",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
    let imports = r#"from collections.abc import Sequence
from typing import assert_type
from dynwinrt import DynWinRTValue, WinRTCoroutine
from sdk.windows.data.json import IJsonValue, JsonObject
from sdk.windows.devices.sensors import Accelerometer
from sdk.windows.foundation import Uri
from sdk.windows.foundation.collections import PropertySet
from sdk.windows.globalization import Calendar
from sdk.windows.security.cryptography import CryptographicBuffer
from sdk.windows.security.cryptography.core import HashAlgorithmProvider
from sdk.windows.storage import CreationCollisionOption, FileIO, IStorageItem, StorageFile, StorageFolder
from sdk.windows.storage.streams import DataReader, DataWriter, InMemoryRandomAccessStream
"#;
    typecheck(
        &fixture,
        &["sdk"],
        &format!(
            r#"{imports}
def uri_demo() -> str:
    uri = Uri("https://example.com/a/b?x=1&y=two")
    query = {{
        entry.name: entry.value
        for entry in uri.query_parsed
        if entry is not None
    }}
    return uri.combine_uri("c/d").absolute_uri + str(query)

def json_demo() -> list[str]:
    parsed = JsonObject.parse('{{"tags": ["a", "b"]}}')
    assert_type(JsonObject.try_parse("{{}}"), tuple[JsonObject | None, bool])
    tags = parsed.get_named_array("tags")
    assert_type(tags[0], IJsonValue | None)
    return [value.get_string() for value in tags if value is not None]

def sensor_demo() -> float | None:
    accelerometer = Accelerometer.get_default()
    if accelerometer is None:
        return None
    reading = accelerometer.get_current_reading()
    return None if reading is None else reading.acceleration_x

def calendar_demo(calendar: Calendar) -> str:
    languages: Sequence[str] = calendar.languages
    return languages[0]

def crypto_demo(data: bytes) -> str:
    buffer = CryptographicBuffer.create_from_byte_array(data)
    digest = HashAlgorithmProvider.open_algorithm("SHA256").hash_data(buffer)
    return CryptographicBuffer.encode_to_hex_string(digest)

def object_values(properties: PropertySet) -> DynWinRTValue | None:
    assert_type(properties["count"], DynWinRTValue | None)
    return properties.lookup("count")

async def streams_demo() -> str:
    stream = InMemoryRandomAccessStream()
    writer = DataWriter(stream.get_output_stream_at(0))
    writer.write_string("streamed text")
    written = await writer.store_async()
    reader = DataReader(stream.get_input_stream_at(0))
    return reader.read_string(await reader.load_async(written))

async def storage_demo(path: str) -> list[str]:
    folder = await StorageFolder.get_folder_from_path_async(path)
    file = await folder.create_file_async("notes.txt", CreationCollisionOption.ReplaceExisting)
    await FileIO.write_text_async(file, "first line")
    assert_type(folder.create_file_async("a.txt"), WinRTCoroutine[StorageFile])
    assert_type(folder.try_get_item_async("notes.txt"), WinRTCoroutine[IStorageItem | None])
    assert_type(folder.get_parent_async(), WinRTCoroutine[StorageFolder | None])
    return [item.name for item in await folder.get_files_async() if item is not None]
"#
        ),
        &[],
    );
}

#[test]
fn mutable_collection_mutators_accept_none() {
    let winmd = Path::new(
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
    );
    if !winmd.is_file() || !has_mypy() {
        eprintln!("Skipping mutable collection mutators: Windows.winmd or mypy unavailable.");
        return;
    }
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd"])
        .arg(winmd)
        .args([
            "--class-name",
            "Windows.Storage.StorageLibrary,Windows.Data.Json.JsonObject,\
             Windows.Foundation.Collections.StringMap,Windows.Foundation.Uri,\
             Windows.Foundation.Collections.PropertySet,Windows.UI.Xaml.ResourceDictionary",
            "--lang",
            "py",
            "--output",
        ])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));

    let stringable = TypeMeta::Interface {
        namespace: "Windows.Foundation".into(),
        name: "IStringable".into(),
        iid: "96369f54-8eb6-48f0-abce-c1b211e627c3".into(),
    };
    let stringable_map_type = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IMap`2".into(),
        piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
        args: vec![stringable.clone(), stringable.clone()],
    };
    let stringable_map = InterfaceMeta {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IMap_IStringable_IStringable".into(),
        generic_name: Some("IMap`2".into()),
        generic_piid: Some("3c2925fe-8519-45c1-aa79-197b6718c1c1".into()),
        generic_args: vec![stringable.clone(), stringable.clone()],
        methods: vec![
            MethodMeta {
                name: "Lookup".into(),
                raw_name: "Lookup".into(),
                vtable_index: 6,
                params: vec![parameter(stringable.clone())],
                return_type: Some(stringable.clone()),
                element_access: Some(dynwinrt_codegen::meta::ElementAccess::Mutable),
                collection_inputs: vec![(0, dynwinrt_codegen::meta::CollectionInputRole::Key)],
                ..Default::default()
            },
            MethodMeta {
                name: "get_Size".into(),
                raw_name: "get_Size".into(),
                vtable_index: 7,
                is_property_getter: true,
                return_type: Some(TypeMeta::U32),
                ..Default::default()
            },
            MethodMeta {
                name: "HasKey".into(),
                raw_name: "HasKey".into(),
                vtable_index: 8,
                params: vec![parameter(stringable.clone())],
                return_type: Some(TypeMeta::Bool),
                collection_inputs: vec![(0, dynwinrt_codegen::meta::CollectionInputRole::Key)],
                ..Default::default()
            },
            MethodMeta {
                name: "GetView".into(),
                raw_name: "GetView".into(),
                vtable_index: 9,
                return_type: Some(TypeMeta::Object),
                ..Default::default()
            },
            MethodMeta {
                name: "Insert".into(),
                raw_name: "Insert".into(),
                vtable_index: 10,
                params: vec![
                    ParamMeta {
                        name: "key".into(),
                        typ: stringable.clone(),
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "value".into(),
                        typ: stringable.clone(),
                        direction: ParamDirection::In,
                    },
                ],
                return_type: Some(TypeMeta::Bool),
                collection_inputs: vec![
                    (0, dynwinrt_codegen::meta::CollectionInputRole::Key),
                    (1, dynwinrt_codegen::meta::CollectionInputRole::Value),
                ],
                ..Default::default()
            },
            MethodMeta {
                name: "Remove".into(),
                raw_name: "Remove".into(),
                vtable_index: 11,
                params: vec![parameter(stringable.clone())],
                collection_inputs: vec![(0, dynwinrt_codegen::meta::CollectionInputRole::Key)],
                ..Default::default()
            },
            MethodMeta {
                name: "Clear".into(),
                raw_name: "Clear".into(),
                vtable_index: 12,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let stringable_context = python::PythonProjectionContext::new(
        [
            stringable.type_identity(),
            stringable_map_type.type_identity(),
        ],
        true,
    )
    .unwrap();
    let stringable_module = stringable_context.implementation_module_for_interface(&stringable_map);
    fs::write(
        fixture
            .0
            .join("sdk")
            .join(format!("{stringable_module}.py")),
        python::generate_interface(&stringable_context, &stringable_map),
    )
    .unwrap();
    fs::write(
        fixture
            .0
            .join("sdk")
            .join(format!("{stringable_module}.pyi")),
        python_stub::generate_interface_stub(&stringable_context, &stringable_map),
    )
    .unwrap();
    let object_map_stub = fs::read_to_string(
        fixture
            .0
            .join("sdk")
            .join("windows__foundation__collections__i_map_object_object.pyi"),
    )
    .unwrap();
    assert!(object_map_stub.contains("MutableMapping[DynWinRTValue | None, DynWinRTValue | None]"));
    let pair_stub = fs::read_to_string(
        fixture
            .0
            .join("sdk")
            .join("windows__foundation__collections__i_key_value_pair_object_object.pyi"),
    )
    .unwrap();
    assert!(pair_stub.contains("def key(self) -> DynWinRTValue | None: ..."));
    assert!(pair_stub.contains("def value(self) -> DynWinRTValue | None: ..."));
    // Inherited MutableSequence and MutableMapping mutators take the element
    // type of the collection base, which keeps `| None` for mutable
    // collections, like the generated item setters.
    typecheck(
        &fixture,
        &["sdk"],
        r#"from typing import assert_type
from dynwinrt import DynWinRTValue
from sdk.windows.data.json import IJsonValue, JsonObject
from sdk.windows.foundation import IStringable, Uri
from sdk.windows.foundation.collections import (
    IMap_Object_Object,
    IMap_String_String,
    IObservableVector_StorageFolder,
)
from sdk.windows.storage import StorageFolder
from sdk.windows__foundation__collections__property_set import IMap_String_Object
from sdk.windows__foundation__collections__i_map_i_stringable_i_stringable import (
    IMap_IStringable_IStringable,
)

def vector(folders: IObservableVector_StorageFolder) -> None:
    IObservableVector_StorageFolder.create([None])
    folders.append(None)
    folders.extend([None])
    folders.insert(0, None)
    folders[0] = None
    assert_type(folders[0], StorageFolder | None)

def mapping(values: JsonObject) -> None:
    values.update({"k": None})
    values.setdefault("k", None)
    values["k"] = None
    assert_type(values["k"], IJsonValue | None)

def object_values(values: IMap_String_Object, uri: Uri) -> None:
    values["none"] = None
    values["uri"] = uri
    assert_type(values["none"], DynWinRTValue | None)

def reference_keys(
    objects: IMap_Object_Object, stringable: IMap_IStringable_IStringable
) -> None:
    objects[None] = None
    stringable[None] = None
    assert_type(objects[None], DynWinRTValue | None)
    assert_type(stringable[None], IStringable | None)

def string_keys(values: IMap_String_String) -> None:
    values["key"] = "value"
    values.get("key")
"#,
        &[],
    );

    if has_implementation_runtime() {
        // Nested IReference<T> collection wrappers call the local boxing
        // helper. Generate an otherwise synthetic module to prove recursive
        // helper detection emits it; the runtime script below imports and
        // invokes the emitted helper. A scalar collection is the negative
        // control and must not emit it.
        let reference = TypeMeta::Parameterized {
            namespace: "Windows.Foundation".into(),
            name: "IReference`1".into(),
            piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
            args: vec![TypeMeta::I32],
        };
        let iterable = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IIterable`1".into(),
            piid: "faa585ea-6214-4217-afda-7f46de5869b3".into(),
            args: vec![reference.clone()],
        };
        let mapping = TypeMeta::Parameterized {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IMap`2".into(),
            piid: "3c2925fe-8519-45c1-aa79-197b6718c1c1".into(),
            args: vec![TypeMeta::String, reference.clone()],
        };
        let nested = interface(
            "INestedReferences",
            99,
            vec![
                MethodMeta {
                    name: "SetValues".into(),
                    raw_name: "SetValues".into(),
                    params: vec![parameter(iterable.clone())],
                    ..Default::default()
                },
                MethodMeta {
                    name: "SetMapping".into(),
                    raw_name: "SetMapping".into(),
                    params: vec![parameter(mapping.clone())],
                    ..Default::default()
                },
            ],
        );
        let nested_context = python::PythonProjectionContext::new(
            [
                nested.type_identity(),
                iterable.type_identity(),
                mapping.type_identity(),
                reference.type_identity(),
            ],
            true,
        )
        .unwrap();
        let nested_module = nested_context.implementation_module_for_interface(&nested);
        let nested_source = python::generate_interface(&nested_context, &nested);
        assert!(nested_source.contains("def _dynwinrt_box_reference(value, value_type, wrap):"));
        assert!(nested_source.contains("_dynwinrt_vector(value, lambda item:"));
        assert!(nested_source.contains("_dynwinrt_map(value, lambda item:"));
        assert!(nested_source.contains("_dynwinrt_box_reference("));
        fs::write(
            fixture.0.join("sdk").join(format!("{nested_module}.py")),
            nested_source,
        )
        .unwrap();

        let scalar = interface(
            "IScalarCollection",
            100,
            vec![MethodMeta {
                name: "SetValues".into(),
                raw_name: "SetValues".into(),
                params: vec![parameter(TypeMeta::Parameterized {
                    namespace: "Windows.Foundation.Collections".into(),
                    name: "IIterable`1".into(),
                    piid: "faa585ea-6214-4217-afda-7f46de5869b3".into(),
                    args: vec![TypeMeta::I32],
                })],
                ..Default::default()
            }],
        );
        let scalar_source = python::generate_interface(&nested_context, &scalar);
        assert!(!scalar_source.contains("def _dynwinrt_box_reference(value, value_type, wrap):"));
        assert!(!scalar_source.contains("_dynwinrt_box_reference("));

        let script = r#"from dynwinrt import (
    DynWinRTMethodSig, DynWinRTType, DynWinRTValue, RoApartment, WinGUID,
    projected_lifetime_scope,
)
from sdk.windows.foundation.collections import (
    IIterable_StorageFolder,
    IMap_Object_Object,
    IMap_String_String,
    IObservableVector_StorageFolder,
    IVector_String,
    IVector_StorageFolder,
)
from sdk.windows__foundation__collections__i_map_i_stringable_i_stringable import (
    IMap_IStringable_IStringable,
)
from sdk.windows__data__json__json_object import IID_IJsonValue, IMap_String_IJsonValue
from sdk.__NESTED_MODULE__ import _dynwinrt_box_reference

with RoApartment(1), projected_lifetime_scope():
    boxed = _dynwinrt_box_reference(
        17, DynWinRTType.i32_type(), DynWinRTValue.from_i32
    )
    assert not boxed.is_null()
    boxed.release()
    null = _dynwinrt_box_reference(
        None, DynWinRTType.i32_type(), DynWinRTValue.from_i32
    )
    assert null.is_null()
    null.release()

    for vector_type in (IVector_StorageFolder, IObservableVector_StorageFolder):
        vector = vector_type.create([None])
        assert vector[0] is None
        vector.append(None)
        vector.insert(0, None)
        vector[1] = None
        vector[1:2] = [None, None]
        vector.extend([None])
        assert list(vector) == [None] * len(vector)

        base = vector.as_vector() if hasattr(vector, "as_vector") else vector
        view = base.get_view()
        assert view is not None
        assert view[0] is None
        assert list(view) == [None] * len(view)
        iterator = base.as_interface(IIterable_StorageFolder).first()
        assert iterator is not None
        assert next(iterator) is None

    native = DynWinRTValue.create_map(
        [], [], DynWinRTType.hstring(), DynWinRTType.interface(IID_IJsonValue)
    )
    mapping = IMap_String_IJsonValue.from_value(native)
    mapping.update({"update": None})
    assert mapping.setdefault("default", None) is None
    mapping["index"] = None
    assert mapping["update"] is None
    assert mapping["default"] is None
    assert mapping["index"] is None
    assert list(mapping.values()) == [None, None, None]

    objects = IMap_Object_Object.create({None: None})
    assert None in objects
    assert objects[None] is None
    objects[None] = None
    assert objects.setdefault(None, None) is None
    assert next(iter(objects.items())) == (None, None)
    assert dict(objects) == {None: None}
    assert objects == {None: None}
    try:
        hash(objects)
    except TypeError:
        pass
    else:
        raise AssertionError("mutable maps must remain unhashable")
    object_view = objects.get_view()
    assert object_view is not None
    assert object_view[None] is None
    assert dict(object_view) == {None: None}
    del objects[None]
    assert None not in objects

    stringable = DynWinRTType.interface(
        WinGUID.parse("96369F54-8EB6-48F0-ABCE-C1B211E627C3")
    )
    stringables = IMap_IStringable_IStringable.create({None: None})
    assert None in stringables
    assert stringables[None] is None
    stringables[None] = None
    stringable_pair = DynWinRTType.parameterized(
        WinGUID.parse("02B51929-C1C4-4A7E-8940-0312B5C18500"),
        [stringable, stringable],
    )
    stringable_iterator = DynWinRTType.parameterized(
        WinGUID.parse("6A79E863-4300-459A-9966-CBB660963EE1"),
        [stringable_pair],
    )
    iterable = DynWinRTType.register_interface(
        "IIterable_IStringablePair",
        DynWinRTType.parameterized(
            WinGUID.parse("FAA585EA-6214-4217-AFDA-7F46DE5869B3"),
            [stringable_pair],
        ).iid(),
    ).add_method("First", DynWinRTMethodSig().add_out(stringable_iterator))
    iterator = DynWinRTType.register_interface(
        "IIterator_IStringablePair", stringable_iterator.iid()
    ).add_method("get_Current", DynWinRTMethodSig().add_out(stringable_pair))
    pair = DynWinRTType.register_interface(
        "IKeyValuePair_IStringable_IStringable", stringable_pair.iid()
    ).add_method("get_Key", DynWinRTMethodSig().add_out(stringable)).add_method(
        "get_Value", DynWinRTMethodSig().add_out(stringable)
    )
    iterable_value = stringables._obj.cast(iterable.iid())
    iterator_value = iterable.method(6).invoke(iterable_value, [])
    pair_value = iterator.method(6).invoke(iterator_value, [])
    assert pair.method(6).invoke(pair_value, []).is_null()
    assert pair.method(7).invoke(pair_value, []).is_null()
    del stringables[None]
    assert None not in stringables

    empty_string_map = IMap_String_String.create({})
    string_view = empty_string_map.get_view()
    assert string_view is not None
    assert string_view.split() == (None, None)

    strings = IMap_String_String.create({})
    invalid = (
        ("map key cannot be None", lambda: mapping.__setitem__(None, None)),
        ("map value cannot be None", lambda: strings.__setitem__("key", None)),
        ("map key cannot be None", lambda: IMap_String_String.create({None: "value"})),
        ("map value cannot be None", lambda: IMap_String_String.create({"key": None})),
        ("collection element cannot be None", lambda: IVector_String.create([None])),
        (
            "collection element cannot be None",
            lambda: IVector_String.create([]).append(None),
        ),
    )
    for expected, operation in invalid:
        try:
            operation()
        except TypeError as error:
            assert str(error) == expected
        else:
            raise AssertionError(f"{expected!r} was not raised")
print("nullable-collection-native-ok", flush=True)
"#
        .replace("__NESTED_MODULE__", &nested_module);
        fs::write(fixture.0.join("nullable_collections.py"), script).unwrap();
        let output = Command::new(python())
            .args(["-B", "nullable_collections.py"])
            .current_dir(&fixture.0)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", diagnostics(&output));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("nullable-collection-native-ok"),
            "{}",
            diagnostics(&output)
        );
    }
}

#[test]
fn native_object_inputs_keep_projection_factories_and_context_lifetimes() {
    if !has_implementation_runtime() {
        return;
    }
    let fixture = Fixture::new();
    generate_fixture(&fixture, "views");
    let script = r#"from typing import get_type_hints
from dynwinrt import DynWinRTValue, RoApartment, project_as, projected_lifetime_scope
from views._runtime import _DynWinRTObject
from views.contoso__i_item import IItem
from views.contoso__i_content import IContent
from views.contoso__resource import Resource
from views.contoso__derived_resource import DerivedResource
from views.contoso__content import Content
from views.windows__foundation__i_closable import IClosable

class ItemHandlers:
    closed = 0

    def get_name(self) -> str:
        return "native item"

    def close(self) -> None:
        self.closed += 1

class ContentHandlers:
    def __init__(self) -> None:
        self.value: DynWinRTValue | None = None
        self.items: list[DynWinRTValue | None] = []

    def get_content(self) -> DynWinRTValue | None:
        return self.value

    def set_content(self, value: DynWinRTValue | None) -> None:
        self.value = value

    def set_object(self, value: DynWinRTValue | None) -> None:
        self.value = value

    def set_objects(self, value: list[DynWinRTValue | None]) -> None:
        self.items = value

    def get_resources(self):
        return []

    def get_counts(self):
        return []

    def get_items(self):
        return []

class WrongObject:
    _obj = 42

assert get_type_hints(Content.set_object)["value"] == DynWinRTValue | _DynWinRTObject
assert get_type_hints(Content.content.fset)["value"] == DynWinRTValue | _DynWinRTObject
assert "value" in get_type_hints(Content.set_objects)
item_handlers = ItemHandlers()
content_handlers = ContentHandlers()
with RoApartment(1), projected_lifetime_scope():
    with IItem.implement(item_handlers, IClosable.implementation(item_handlers)) as item_owner:
        with IContent.implement(content_handlers) as content_owner:
            resource = project_as(item_owner.value, Resource)
            content = project_as(content_owner.value, Content)
            interface = resource.as_interface(IItem)
            assert interface.name == "native item"
            assert IItem.from_value(resource._obj).name == "native item"
            with project_as(item_owner.value, DerivedResource) as derived:
                assert derived.__enter__() is derived
                for value in (resource, derived, interface, resource._obj):
                    content.content = value
                    result = content.content
                    assert isinstance(result, DynWinRTValue)
                    assert IItem.from_value(result).name == "native item"
                    content.set_object(value)
                    assert content_handlers.value is not None
                content.set_objects([resource, derived, interface, resource._obj])
                assert len(content_handlers.items) == 4
                assert all(item is not None for item in content_handlers.items)
                content.content = DynWinRTValue.null_value()
                assert content.content is None
            assert item_handlers.closed == 1
            try:
                with resource as entered:
                    assert entered is resource
                    raise ValueError("not suppressed")
            except ValueError as error:
                assert str(error) == "not suppressed"
            else:
                raise AssertionError("__exit__ suppressed an exception")
            assert item_handlers.closed == 2
            for invalid in (object(), "not boxed", None, 42, WrongObject()):
                try:
                    content.set_object(invalid)
                except TypeError:
                    pass
                else:
                    raise AssertionError("invalid Object input was accepted")
            assert item_owner.take_error() is None
            assert content_owner.take_error() is None
"#;
    fs::write(fixture.0.join("runtime.py"), script).unwrap();
    let output = Command::new(python())
        .args(["-B", "runtime.py"])
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", diagnostics(&output));
}
