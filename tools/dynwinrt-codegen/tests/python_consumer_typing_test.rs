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
        ],
    );
    let classes = [
        resource,
        derived,
        class("Unrelated", unrelated.clone()),
        class("Content", content.clone()),
    ];
    let interfaces = [item, unrelated, closable, content];
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
"#,
        &[],
    );
    typecheck(
        &fixture,
        &["first", "second"],
        r#"from dynwinrt import DynWinRTValue
from first.contoso__i_item import IItem
from first.contoso__i_unrelated import IUnrelated
from first.contoso__resource import Resource, ResourceLike
from first.contoso__unrelated import Unrelated
from first.contoso__content import Content
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
        ],
    );
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
fn native_object_inputs_keep_projection_factories_and_context_lifetimes() {
    let available = Command::new(python())
        .args(["-c", "from dynwinrt import DynWinRTImplementationHandle"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        assert_ne!(
            std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref(),
            Ok("1"),
            "DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME=1 but the matching runtime is unavailable"
        );
        eprintln!("Skipping native consumers: set DYNWINRT_TEST_PYTHON to the matching runtime.");
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
