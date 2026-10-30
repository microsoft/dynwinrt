// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::meta::{
    self, ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use dynwinrt_codegen::types::TypeMeta;
use windows_metadata::{
    GenericParamAttributes, MethodCallAttributes, Signature, Type, TypeAttributes, TypeName, Value,
    writer,
};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";
const ICLOSABLE_IID: &str = "30d5a829-7fa4-4026-83bb-d75bae4ea99e";

#[test]
fn closable_context_manager_never_suppresses_exceptions() {
    let class = ClassMeta {
        name: "Resource".into(),
        namespace: "Contoso".into(),
        full_name: "Contoso.Resource".into(),
        required_interfaces: vec![InterfaceMeta {
            name: "IClosable".into(),
            namespace: "Windows.Foundation".into(),
            iid: "30d5a829-7fa4-4026-83bb-d75bae4ea99e".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let stub = common::generate_class_stub(
        &class,
        &HashSet::from(["IClosable".into(), "Resource".into()]),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(stub.contains("from typing import Literal"), "{stub}");
    assert!(stub.contains("def __enter__(self) -> Self: ..."), "{stub}");
    assert!(
        stub.contains(
            "def __exit__(self, exc_type: object, exc_value: object, traceback: object) -> Literal[False]: ..."
        ),
        "{stub}"
    );
}

fn closable_type() -> TypeMeta {
    TypeMeta::Interface {
        namespace: "Windows.Foundation".into(),
        name: "IClosable".into(),
        iid: ICLOSABLE_IID.into(),
    }
}

fn resource_interface() -> InterfaceMeta {
    InterfaceMeta {
        name: "IResource".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        base_interfaces: vec![closable_type()],
        ..Default::default()
    }
}

fn assert_helpers(interface: &InterfaceMeta, close: bool, context: bool) {
    let known = HashSet::from(["IClosable".into()]);
    let runtime = common::generate_interface(interface, &known, &HashSet::new());
    let stub = common::generate_interface_stub(interface, &known, &HashSet::new());
    assert_eq!(runtime.contains("    def close(self):"), close, "{runtime}");
    for source in [&runtime, &stub] {
        assert_eq!(source.contains("    def __enter__("), context, "{source}");
        assert_eq!(source.contains("    def __exit__("), context, "{source}");
    }
    if context {
        assert!(stub.contains("def __enter__(self) -> Self: ..."), "{stub}");
        assert!(stub.contains("-> Literal[False]: ..."), "{stub}");
        assert!(
            runtime.contains("self.close()\n        return False"),
            "{runtime}"
        );
    }
    if close {
        assert!(runtime.contains("self._closed = False"), "{runtime}");
        assert!(
            runtime.contains(".from_value(self._obj).close()"),
            "{runtime}"
        );
        assert!(stub.contains("def close(self) -> None: ..."), "{stub}");
    }
}

#[test]
fn closable_interfaces_use_metadata_iids_not_names_or_generic_arguments() {
    let mut interface = resource_interface();
    assert_helpers(&interface, true, true);
    interface.base_interfaces.clear();
    interface
        .implementation_metadata
        .required_interfaces
        .push(closable_type());
    assert_helpers(&interface, true, true);

    interface.generic_piid = Some(interface.iid.clone());
    interface.generic_name = Some("IResource".into());
    interface.generic_args = vec![TypeMeta::String];
    assert_helpers(&interface, true, true);
    interface.implementation_metadata.is_generic_definition = true;
    assert_helpers(&interface, false, false);
    interface.implementation_metadata.is_generic_definition = false;
    interface
        .implementation_metadata
        .required_interfaces
        .clear();
    interface.generic_args = vec![closable_type()];
    assert_helpers(&interface, false, false);

    interface.generic_piid = None;
    interface.generic_args.clear();
    interface.name = "IClosable".into();
    assert_helpers(&interface, false, false);
    interface.base_interfaces.push(TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: "IClosable".into(),
        iid: interface.iid.clone(),
    });
    assert_helpers(&interface, false, false);
}

#[test]
fn iclosable_keeps_its_native_close_member() {
    let interface = InterfaceMeta {
        name: "IClosable".into(),
        namespace: "Windows.Foundation".into(),
        iid: ICLOSABLE_IID.to_uppercase(),
        methods: vec![MethodMeta {
            name: "Close".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_helpers(&interface, false, true);
    let runtime = common::generate_interface(&interface, &HashSet::new(), &HashSet::new());
    assert!(
        runtime.contains("_IClosable.method(6).invoke(self._obj, [])"),
        "{runtime}"
    );
    assert!(!runtime.contains("self._closed"), "{runtime}");
}

#[test]
fn closable_helpers_yield_to_native_members_and_aliases() {
    for method in [
        MethodMeta {
            name: "Close".into(),
            params: vec![ParamMeta {
                name: "reason".into(),
                typ: TypeMeta::String,
                direction: ParamDirection::In,
            }],
            ..Default::default()
        },
        MethodMeta {
            name: "get_Close".into(),
            is_property_getter: true,
            return_type: Some(TypeMeta::I32),
            ..Default::default()
        },
    ] {
        let interface = InterfaceMeta {
            methods: vec![MethodMeta {
                vtable_index: 6,
                ..method
            }],
            ..resource_interface()
        };
        assert_helpers(&interface, false, false);
        let runtime = common::generate_interface(&interface, &HashSet::new(), &HashSet::new());
        assert!(runtime.contains("    def close("), "{runtime}");
        assert!(
            runtime.contains("_IResource.method(6).invoke(self._obj,"),
            "{runtime}"
        );
    }
    let interface = InterfaceMeta {
        methods: vec![MethodMeta {
            name: "CloseWithReason".into(),
            raw_name: "Close".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..resource_interface()
    };
    assert_helpers(&interface, false, false);
    for source in [
        common::generate_interface(&interface, &HashSet::new(), &HashSet::new()),
        common::generate_interface_stub(&interface, &HashSet::new(), &HashSet::new()),
    ] {
        assert!(source.contains("    def close("), "{source}");
        assert!(source.contains("close_with_reason"), "{source}");
    }
    let interface = InterfaceMeta {
        methods: vec![MethodMeta {
            name: "Close".into(),
            raw_name: "Shutdown".into(),
            vtable_index: 6,
            ..Default::default()
        }],
        ..resource_interface()
    };
    assert_helpers(&interface, false, false);
    let runtime = common::generate_interface(&interface, &HashSet::new(), &HashSet::new());
    assert!(runtime.contains("    def shutdown("), "{runtime}");
    assert!(runtime.contains("    close = shutdown"), "{runtime}");
}

#[test]
fn real_stream_requirements_include_transitive_closable_without_inheriting_methods() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let interfaces = meta::parse_interfaces(WINDOWS_WINMD, "Windows.Storage.Streams");
    for name in [
        "IInputStream",
        "IOutputStream",
        "IRandomAccessStream",
        "IRandomAccessStreamWithContentType",
    ] {
        let interface = interfaces
            .iter()
            .find(|interface| interface.name == name)
            .unwrap();
        assert_helpers(interface, true, true);
        if name == "IRandomAccessStreamWithContentType" {
            assert!(!interface.base_interfaces.contains(&closable_type()));
            assert!(
                interface
                    .implementation_metadata
                    .required_interfaces
                    .contains(&closable_type())
            );
            let runtime = common::generate_interface(interface, &HashSet::new(), &HashSet::new());
            assert!(!runtime.contains("def seek("), "{runtime}");
        }
    }
    for name in [
        "IBuffer",
        "IContentTypeProvider",
        "IRandomAccessStreamReference",
    ] {
        assert_helpers(
            interfaces
                .iter()
                .find(|interface| interface.name == name)
                .unwrap(),
            false,
            false,
        );
    }
}

#[test]
fn closed_generic_required_interface_closure_is_not_its_type_argument_closure() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let fixture = Fixture::new();
    let mut file = writer::File::new("ClosableGenerics");
    let mut define = |name: &str, id, generic, required: Type| {
        let interface = file.TypeDef(
            "Contoso",
            name,
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
                Value::U32(id),
                Value::U16(1),
                Value::U16(2),
                Value::U8(3),
                Value::U8(4),
                Value::U8(5),
                Value::U8(6),
                Value::U8(7),
                Value::U8(8),
                Value::U8(9),
                Value::U8(10),
            ]
            .into_iter()
            .map(|value| (String::new(), value))
            .collect::<Vec<_>>(),
        );
        if generic {
            file.GenericParam(
                "T",
                writer::TypeOrMethodDef::TypeDef(interface),
                0,
                GenericParamAttributes::default(),
            );
        }
        file.InterfaceImpl(interface, &required);
    };
    let generic = |namespace: &str, name: &str, argument| {
        Type::Name(TypeName {
            namespace: namespace.into(),
            name: name.into(),
            generics: vec![argument],
        })
    };
    define(
        "IResource`1",
        1,
        true,
        Type::named("Windows.Foundation", "IClosable"),
    );
    define(
        "IDerived`1",
        2,
        true,
        generic("Contoso", "IResource`1", Type::Generic(0)),
    );
    define(
        "IRoot",
        3,
        false,
        generic("Contoso", "IDerived`1", Type::String),
    );
    define(
        "IContainer",
        4,
        false,
        generic(
            "Windows.Foundation.Collections",
            "IVector`1",
            Type::named("Windows.Foundation", "IClosable"),
        ),
    );
    let winmd = fixture.0.join("ClosableGenerics.winmd");
    fs::write(&winmd, file.into_stream()).unwrap();
    let inputs = format!("{};{WINDOWS_WINMD}", winmd.display());
    let roots = meta::parse_interfaces(&inputs, "Contoso");
    let root = roots
        .iter()
        .find(|interface| interface.name == "IRoot")
        .unwrap();
    assert_helpers(root, true, true);
    assert!(!root.base_interfaces.contains(&closable_type()));
    let container = roots
        .iter()
        .find(|interface| interface.name == "IContainer")
        .unwrap();
    assert_helpers(container, false, false);
    let dependencies = meta::resolve_python_dependencies(&inputs, &[], &roots, &[]);
    for name in ["IResource_String", "IDerived_String", "IVector_IClosable"] {
        let interface = dependencies
            .interfaces
            .iter()
            .find(|interface| interface.name == name)
            .unwrap_or_else(|| panic!("missing closed dependency {name}"));
        let closable = name != "IVector_IClosable";
        assert_helpers(interface, closable, closable);
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "pcm{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
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
        .unwrap_or_else(|| "python".into())
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn success(output: Output) {
    assert!(output.status.success(), "{}", diagnostics(&output));
}

fn available(import: &str, required: &str) -> bool {
    let available = Command::new(python())
        .args(["-c", import])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var(required).as_deref() != Ok("1"),
        "{required}=1: {import}"
    );
    if !available {
        eprintln!("Skipping Python probe: {import}; set DYNWINRT_TEST_PYTHON.");
    }
    available
}

fn generate_streams(fixture: &Fixture, no_pyi: bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"));
    command.args(["generate", "--lang", "py", "--winmd", WINDOWS_WINMD, "--class-name",
        "Windows.Storage.StorageFile,Windows.Storage.Streams.InMemoryRandomAccessStream,Windows.Storage.Streams.DataWriter,Windows.Storage.Streams.Buffer,Windows.Storage.Streams.IRandomAccessStreamWithContentType"]);
    command.arg("--output").arg(fixture.0.join("views"));
    if no_pyi {
        command.arg("--no-pyi");
    }
    success(command.output().unwrap());
}

#[test]
fn generated_stream_contexts_close_real_native_resources() {
    if !Path::new(WINDOWS_WINMD).exists()
        || !available(
            "from dynwinrt import DynWinRTImplementationHandle",
            "DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME",
        )
    {
        return;
    }
    for no_pyi in [false, true] {
        let fixture = Fixture::new();
        generate_streams(&fixture, no_pyi);
        fs::write(fixture.0.join("probe.py"), NATIVE_PROBE).unwrap();
        success(
            Command::new(python())
                .arg("-B")
                .arg("probe.py")
                .current_dir(&fixture.0)
                .output()
                .unwrap(),
        );
    }
}

const NATIVE_PROBE: &str = r#"
import asyncio
import inspect
from pathlib import Path
from tempfile import TemporaryDirectory
from dynwinrt import RoApartment, projected_lifetime_scope
import views as g

async def expect_closed(action):
    try:
        value = action()
        if inspect.isawaitable(value):
            await value
    except OSError as error:
        assert error.winerror & 0xFFFFFFFF == 0x80000013, error
    else:
        raise AssertionError('native operation should fail with RO_E_CLOSED')

def expect_closed_entry(stream):
    try:
        stream.__enter__()
    except RuntimeError as error:
        assert 'closed WinRT object' in str(error), error
    else:
        raise AssertionError('closed projection can be reentered')

async def main():
    with TemporaryDirectory() as directory:
        path = Path(directory) / 'input.txt'
        path.write_bytes(b'closable')
        file = await g.StorageFile.get_file_from_path_async(str(path))
        stream = await file.open_async(g.FileAccessMode.Read)
        assert type(stream) is g.IRandomAccessStream
        assert stream.size == 8
        stream.close()
        stream.close()
        assert not stream._obj.is_null(), 'Close must not release the projection'
        expect_closed_entry(stream)
        await expect_closed(lambda: stream.size)

        failure = ValueError('application error')
        stream = await file.open_async(g.FileAccessMode.Read)
        try:
            with stream as entered:
                assert entered is stream and stream.size == 8
                raise failure
        except ValueError as error:
            assert error is failure
        else:
            raise AssertionError('context swallowed application exception')
        await expect_closed(lambda: stream.size)

        buffer = g.Buffer.create(8).as_interface(g.IBuffer)
        with await file.open_async(g.FileAccessMode.Read) as stream:
            input_stream = stream.get_input_stream_at(0)
            assert type(input_stream) is g.IInputStream
            with input_stream as entered:
                assert entered is input_stream
                read = await input_stream.read_async(buffer, 8, g.InputStreamOptions.None_)
                assert read.to_bytes() == b'closable'
            input_stream.close()
            expect_closed_entry(input_stream)
            await expect_closed(lambda: input_stream.read_async(buffer, 1, g.InputStreamOptions.None_))

        content = await file.open_read_async()
        assert type(content) is g.IRandomAccessStreamWithContentType
        alias = content.as_interface(g.IRandomAccessStream)
        with content as entered:
            assert entered is content and alias.size == 8
        assert not alias._obj.is_null()
        await expect_closed(lambda: alias.size)

        with g.InMemoryRandomAccessStream.create() as memory:
            output = memory.get_output_stream_at(0)
            assert type(output) is g.IOutputStream
            with output as entered:
                assert entered is output
                assert await output.write_async(buffer) == 8
            output.close()
            await expect_closed(lambda: output.write_async(buffer))
        await expect_closed(lambda: memory.size)
        with g.DataWriter.create_default() as writer:
            writer.write_byte(42)
        writer.close()
        await expect_closed(lambda: writer.write_byte(43))

        memory = g.InMemoryRandomAccessStream.create()
        with memory.as_interface(g.IClosable) as closable:
            assert type(closable) is g.IClosable
            assert memory.size == 0
        await expect_closed(lambda: memory.size)
        assert not hasattr(buffer, 'close') and not hasattr(buffer, '__enter__')
        assert not hasattr(file.as_interface(g.IStorageFile), 'close')

    class Input:
        def read_async(self, buffer, count, options):
            raise NotImplementedError('unused')

    class Close:
        calls = 0
        def close(self):
            self.calls += 1
            if self.calls == 1:
                raise ValueError('retry close')

    handler = Close()
    with g.IInputStream.implement(Input(), interfaces=[(g.IClosable, handler)]) as owner:
        view = owner.value
        try:
            view.close()
        except OSError:
            assert handler.calls == 1
        else:
            raise AssertionError('failed Close was swallowed')
        assert view.__enter__() is view
        view.close()
        view.close()
        assert handler.calls == 2
        assert not owner.is_closed, 'Close must not dispose the implementation owner'
        expect_closed_entry(view)
        direct = view.as_interface(g.IClosable)
        direct.close()
        direct.close()
        assert handler.calls == 4, 'IClosable.Close must retain native repeat-call semantics'

with RoApartment(), projected_lifetime_scope():
    asyncio.run(main())
print('real stream close/context, exceptions, aliases, retry, and existing classes: OK')
"#;

#[test]
fn generated_stream_context_typing_preserves_self_and_non_suppression() {
    if !Path::new(WINDOWS_WINMD).exists() || !available("import mypy", "DYNWINRT_REQUIRE_MYPY") {
        return;
    }
    let fixture = Fixture::new();
    generate_streams(&fixture, false);
    fs::write(fixture.0.join("consumer.py"), r#"
from typing import assert_type
from views import IClosable, IInputStream, IOutputStream, IRandomAccessStream, IRandomAccessStreamWithContentType, InMemoryRandomAccessStream, DataWriter, StorageFile, FileAccessMode

async def read_size(path: str) -> int:
    file = await StorageFile.get_file_from_path_async(path)
    with await file.open_async(FileAccessMode.Read) as stream:
        assert_type(stream, IRandomAccessStream)
        size = stream.size
    return size

def interfaces(input: IInputStream, output: IOutputStream, content: IRandomAccessStreamWithContentType, close: IClosable) -> None:
    with input as i, output as o, content as c, close as closer:
        assert_type(i, IInputStream)
        assert_type(o, IOutputStream)
        assert_type(c, IRandomAccessStreamWithContentType)
        assert_type(closer, IClosable)
        result = 1
    assert_type(result, int)
    input.close()
    output.close()
    content.close()
    close.close()
    with InMemoryRandomAccessStream.create() as memory:
        assert_type(memory, InMemoryRandomAccessStream)
    with DataWriter.create_default() as writer:
        assert_type(writer, DataWriter)
"#).unwrap();
    let typecheck = || {
        Command::new(python())
            .args([
                "-B",
                "-m",
                "mypy",
                "--strict",
                "--no-incremental",
                "--follow-imports=silent",
                "--no-pretty",
                "--show-error-codes",
                "--cache-dir",
                ".mypy_cache",
                "consumer.py",
            ])
            .env(
                "MYPYPATH",
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("bindings")
                    .join("py"),
            )
            .current_dir(&fixture.0)
            .output()
            .unwrap()
    };
    success(typecheck());
    fs::write(
        fixture.0.join("consumer.py"),
        r#"
from views import IBuffer
def unsupported(buffer: IBuffer) -> None:
    buffer.close()
    with buffer:
        pass
"#,
    )
    .unwrap();
    let output = typecheck();
    let text = diagnostics(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    for member in ["close", "__enter__", "__exit__"] {
        assert!(
            text.contains(&format!("has no attribute \"{member}\"")),
            "{text}"
        );
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.contains(": error:"))
            .count(),
        3,
        "{text}"
    );
}
