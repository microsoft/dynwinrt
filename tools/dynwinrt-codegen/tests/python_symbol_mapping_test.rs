// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{
    self, ClassMeta, ConstructorKind, ConstructorMeta, InterfaceMeta, MethodMeta, ParamDirection,
    ParamMeta,
};
use dynwinrt_codegen::types::{
    EnumMember, TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta, TypeRef,
};
use windows_metadata::{
    FieldAttributes, GenericParamAttributes, MethodAttributes, MethodCallAttributes,
    MethodImplAttributes, ParamAttributes, Signature, Type, TypeAttributes, TypeName, Value,
    writer,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "python-symbols-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    fn package(&self) -> PathBuf {
        let package = self.0.join("pyviews");
        fs::create_dir_all(&package).unwrap();
        package
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn root() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap();
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = root()
                .join("bindings")
                .join("py")
                .join(".venv")
                .join("Scripts")
                .join("python.exe");
            if venv.is_file() {
                venv
            } else {
                PathBuf::from("python")
            }
        })
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn available(arguments: &[&str], description: &str) -> bool {
    let available = Command::new(python())
        .args(arguments)
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "{description} is required by DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME=1",
    );
    if !available {
        eprintln!(
            "Skipping {description}; set DYNWINRT_TEST_PYTHON to the prepared E2E interpreter."
        );
    }
    available
}

fn typecheck(directory: &Path, consumer: &str) {
    check_consumer(directory, consumer, false);
}

fn strict_typecheck(directory: &Path, consumer: &str) {
    check_consumer(directory, consumer, true);
}

fn check_consumer(directory: &Path, consumer: &str, strict: bool) {
    if !available(
        &["-m", "mypy", "--version"],
        "whole-package Python typechecking",
    ) {
        return;
    }
    fs::write(directory.join("consumer.py"), consumer).unwrap();
    // Explicitly check every generated module, not only the public consumer.
    success(
        Command::new(python())
            .args([
                "-m",
                "mypy",
                "--no-incremental",
                "--follow-imports=normal",
                "pyviews",
                "consumer.py",
            ])
            .args(strict.then_some("--strict"))
            .current_dir(directory)
            .env("MYPYPATH", root().join("bindings").join("py"))
            .output()
            .unwrap(),
    );
}

fn reject_consumer(directory: &Path, consumer: &str, codes: &[&str]) {
    if !available(
        &["-m", "mypy", "--version"],
        "strict negative consumer typechecking",
    ) {
        return;
    }
    fs::write(directory.join("negative.py"), consumer).unwrap();
    let output = Command::new(python())
        .args([
            "-m",
            "mypy",
            "--strict",
            "--no-incremental",
            "--follow-imports=normal",
            "--show-error-codes",
            "--no-pretty",
            "pyviews",
            "negative.py",
        ])
        .current_dir(directory)
        .env("MYPYPATH", root().join("bindings").join("py"))
        .output()
        .unwrap();
    let diagnostics = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{diagnostics}");
    let errors = diagnostics
        .lines()
        .filter(|line| line.contains(": error:"))
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), codes.len(), "{diagnostics}");
    for (error, code) in errors.iter().zip(codes) {
        assert!(error.starts_with("negative.py:"), "{diagnostics}");
        assert!(error.ends_with(code), "{diagnostics}");
    }
}

fn runtime(directory: &Path, source: &str) {
    if available(
        &[
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt, 'DynWinRTImplementationHandle')",
        ],
        "native Python projection operations",
    ) {
        fs::write(directory.join("runtime_probe.py"), source).unwrap();
        success(
            Command::new(python())
                .args(["-B", "runtime_probe.py"])
                .current_dir(directory)
                .output()
                .unwrap(),
        );
    }
}

fn module(package: &Path, name: &str, source: String, stub: String) {
    fs::write(package.join(format!("{name}.py")), source).unwrap();
    fs::write(package.join(format!("{name}.pyi")), stub).unwrap();
}

fn support(package: &Path, interfaces: &[String]) {
    for (name, source) in [
        ("__init__.py", String::new()),
        ("_runtime.py", python::generate_runtime_support_module()),
        ("_runtime.pyi", python_stub::generate_runtime_support_stub()),
        ("_typing.pyi", python_stub::generate_typing_support_module()),
        (
            "_implementation_types.pyi",
            python_stub::generate_implementation_pair_types(interfaces),
        ),
    ] {
        fs::write(package.join(name), source).unwrap();
    }
}

fn class_type(namespace: &str, name: &str) -> TypeMeta {
    TypeMeta::RuntimeClass {
        namespace: namespace.into(),
        name: name.into(),
        default_interface: None,
    }
}

fn echo(name: &str, typ: TypeMeta, index: usize) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index: index,
        params: vec![ParamMeta {
            name: "value".into(),
            typ: typ.clone(),
            direction: ParamDirection::In,
        }],
        return_type: Some(typ),
        ..Default::default()
    }
}

fn class(namespace: &str, name: &str, methods: Vec<MethodMeta>, id: u32) -> ClassMeta {
    ClassMeta {
        namespace: namespace.into(),
        name: name.into(),
        full_name: format!("{namespace}.{name}"),
        default_interface: Some(InterfaceMeta {
            namespace: namespace.into(),
            name: format!("I{name}"),
            iid: format!("{id:08x}-6281-4900-b782-040302010910"),
            methods,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn class_identity(class: &ClassMeta) -> TypeIdentity {
    class_type(&class.namespace, &class.name).type_identity()
}

#[test]
fn python_class_self_and_base_markers_use_declarations_in_both_layouts() {
    for packaged in [false, true] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let mut classes = Vec::new();
        for (namespace, foreign, id) in
            [("Alpha", "Beta", 0x51931001), ("Beta", "Alpha", 0x51931010)]
        {
            classes.push(class(namespace, "Base", vec![], id));
            let mut widget = class(
                namespace,
                "Widget",
                vec![
                    echo("EchoSelf", class_type(namespace, "Widget"), 6),
                    echo("EchoForeign", class_type(foreign, "Widget"), 7),
                ],
                id + 1,
            );
            widget.base_class = Some(TypeRef {
                namespace: namespace.into(),
                name: "Base".into(),
                kind: TypeKind::Class,
            });
            classes.push(widget);
            let mut derived = class(namespace, "Derived", vec![], id + 2);
            derived.base_class = Some(TypeRef {
                namespace: foreign.into(),
                name: "Base".into(),
                kind: TypeKind::Class,
            });
            classes.push(derived);
        }
        let context = python::PythonProjectionContext::new(
            classes
                .iter()
                .map(class_identity)
                .chain(classes.iter().filter_map(|class| {
                    class
                        .default_interface
                        .as_ref()
                        .map(InterfaceMeta::type_identity)
                })),
            packaged,
        )
        .unwrap();
        let mut consumer = "from typing import assert_type\n".to_string();
        for class in &classes {
            let identity = class_identity(class);
            let name = context.projected_name(&identity);
            let module_name = context.implementation_module(&identity);
            let source = python::generate_class(&context, class, &Default::default());
            let stub = python_stub::generate_class_stub(&context, class, &Default::default());
            for text in [&source, &stub] {
                assert!(
                    !text.contains(&format!("from .{module_name} import ")),
                    "{text}"
                );
                assert!(text.contains(&format!("class {name}")), "{text}");
            }
            if class.name == "Widget" {
                let foreign = if class.namespace == "Alpha" {
                    "Beta"
                } else {
                    "Alpha"
                };
                assert_ne!(
                    context.reference_name(&identity),
                    name,
                    "fixture must require a local self binding"
                );
                assert!(
                    stub.contains("def echo_self(self, value: 'WidgetLike') -> Widget | None:"),
                    "{stub}"
                );
                assert!(
                    !source.contains(&context.reference_name(&identity)),
                    "{source}"
                );
                let foreign_module =
                    context.implementation_module(&class_type(foreign, "Widget").type_identity());
                assert!(
                    stub.contains(&format!("from .{foreign_module} import Widget as ")),
                    "{stub}"
                );
            }
            if let Some(base) = &class.base_class {
                let identity = class_type(&base.namespace, &base.name).type_identity();
                let declaration = context.projected_name(&identity);
                let reference = context.reference_name(&identity);
                assert_ne!(declaration, reference);
                assert!(
                    stub.contains(&format!(
                        "from .{} import _{declaration}Identity as _{reference}Identity",
                        context.implementation_module(&identity),
                    )),
                    "{stub}"
                );
                assert!(
                    stub.contains(&format!(
                        "class _{name}Identity(_{reference}Identity, Protocol):"
                    )),
                    "{stub}"
                );
            }
            consumer.push_str(&format!(
                "from pyviews.{module_name} import {name} as {namespace}{name}, {name}Like as {namespace}{name}Like\n",
                namespace = class.namespace,
            ));
            module(&package, &module_name, source, stub);
        }
        consumer.push_str(
            r#"
def alpha(value: AlphaWidget, peer: BetaWidget, child: AlphaDerived) -> None:
    own: AlphaBaseLike = value
    foreign: BetaBaseLike = child
    assert_type(value.echo_self(value), AlphaWidget | None)
    assert_type(value.echo_foreign(peer), BetaWidget | None)
def beta(value: BetaWidget, peer: AlphaWidget, child: BetaDerived) -> None:
    own: BetaBaseLike = value
    foreign: AlphaBaseLike = child
    assert_type(value.echo_self(value), BetaWidget | None)
    assert_type(value.echo_foreign(peer), AlphaWidget | None)
"#,
        );
        support(&package, &[]);
        typecheck(&fixture.0, &consumer);
        reject_consumer(
            &fixture.0,
            &format!(
                r#"{consumer}
def reject_wrong_identity(value: AlphaWidget, peer: BetaWidget) -> None:
    value.echo_self(peer)
    wrong_base: AlphaBaseLike = peer
    result = value.echo_self(value)
    if result is not None:
        result.no_such_member()
"#
            ),
            &["[arg-type]", "[assignment]", "[attr-defined]"],
        );
    }
}

#[test]
fn python_named_class_closed_generic_collision_uses_allocated_declaration() {
    for packaged in [false, true] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let namespace = "Classes";
        let named_type = class_type(namespace, "IBucket_String");
        let generic_type = TypeMeta::Parameterized {
            namespace: namespace.into(),
            name: "IBucket`1".into(),
            piid: "51931300-6281-4900-b782-040302010910".into(),
            args: vec![TypeMeta::String],
        };
        let mut named = class(
            namespace,
            "IBucket_String",
            vec![
                echo("EchoSelf", named_type.clone(), 6),
                echo("EchoGeneric", generic_type.clone(), 7),
            ],
            0x51931301,
        );
        let factory = InterfaceMeta {
            namespace: namespace.into(),
            name: "IBucketFactory".into(),
            iid: "51931302-6281-4900-b782-040302010910".into(),
            methods: vec![MethodMeta {
                name: "CreateNamed".into(),
                raw_name: "CreateNamed".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::In,
                }],
                return_type: Some(named_type.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        named.constructors = vec![
            ConstructorMeta {
                kind: ConstructorKind::DefaultActivation,
                factory_interface: None,
            },
            ConstructorMeta {
                kind: ConstructorKind::FactoryActivation,
                factory_interface: Some(TypeRef {
                    namespace: namespace.into(),
                    name: factory.name.clone(),
                    kind: TypeKind::Interface,
                }),
            },
        ];
        named.has_default_constructor = true;
        named.factory_interfaces.push(factory);
        named.static_interfaces.push(InterfaceMeta {
            namespace: namespace.into(),
            name: "IBucketStatics".into(),
            iid: "51931303-6281-4900-b782-040302010910".into(),
            methods: vec![MethodMeta {
                name: "GetCurrent".into(),
                raw_name: "GetCurrent".into(),
                vtable_index: 6,
                return_type: Some(named_type.clone()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let mut derived = class(namespace, "Derived", vec![], 0x51931304);
        derived.base_class = Some(TypeRef {
            namespace: namespace.into(),
            name: "IBucket_String".into(),
            kind: TypeKind::Class,
        });
        derived
            .required_interfaces
            .push(named.default_interface.as_ref().unwrap().clone());
        let generic = InterfaceMeta {
            namespace: namespace.into(),
            name: "IBucket_String".into(),
            generic_name: Some("IBucket`1".into()),
            generic_piid: Some("51931300-6281-4900-b782-040302010910".into()),
            generic_args: vec![TypeMeta::String],
            methods: vec![
                echo("EchoSelf", generic_type.clone(), 6),
                echo("EchoNamed", named_type.clone(), 7),
            ],
            ..Default::default()
        };
        assert_ne!(named_type.type_identity(), generic.type_identity());
        let context = python::PythonProjectionContext::new(
            [
                named_type.type_identity(),
                generic.type_identity(),
                class_identity(&derived),
            ]
            .into_iter()
            .chain(named.all_interfaces().map(InterfaceMeta::type_identity))
            .chain(derived.all_interfaces().map(InterfaceMeta::type_identity)),
            packaged,
        )
        .unwrap();
        let named_declaration = context.projected_name_for_type(&named_type);
        let generic_declaration = context.projected_name_for_interface(&generic);
        let named_module = context.implementation_module_for_type(&named_type);
        let generic_module = context.implementation_module_for_interface(&generic);
        let derived_module = context.implementation_module(&class_identity(&derived));
        assert_ne!(named_declaration, "IBucket_String");
        assert_ne!(named_declaration, generic_declaration);
        let source = python::generate_class(&context, &named, &Default::default());
        let stub = python_stub::generate_class_stub(&context, &named, &Default::default());
        for text in [&source, &stub] {
            assert!(
                text.contains(&format!("class {named_declaration}")),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "from .{generic_module} import {generic_declaration}"
                )),
                "{text}",
            );
            assert!(
                !text.contains(&format!("from .{named_module} import ")),
                "{text}"
            );
        }
        assert!(
            source.contains("activation_factory('Classes.IBucket_String')"),
            "the allocated Python declaration must not change the native activation name:\n{source}",
        );
        assert!(
            !source.contains(&format!(
                "activation_factory('Classes.{named_declaration}')"
            )),
            "{source}",
        );
        assert!(
            stub.contains(&format!("class _{named_declaration}Identity("))
                && stub.contains(&format!("class {named_declaration}Like(")),
            "{stub}",
        );
        module(&package, &named_module, source, stub);
        let derived_stub =
            python_stub::generate_class_stub(&context, &derived, &Default::default());
        assert!(
            derived_stub.contains(&format!(
                "from .{named_module} import _{named_declaration}Identity"
            )),
            "{derived_stub}",
        );
        module(
            &package,
            &derived_module,
            python::generate_class(&context, &derived, &Default::default()),
            derived_stub,
        );
        let source = python::generate_interface(&context, &generic);
        let stub = python_stub::generate_interface_stub(&context, &generic);
        for text in [&source, &stub] {
            assert!(
                text.contains(&format!("from .{named_module} import {named_declaration}")),
                "a named peer is not the closed generic's self type:\n{text}",
            );
            assert!(
                !text.contains(&format!("from .{generic_module} import ")),
                "{text}"
            );
        }
        module(&package, &generic_module, source, stub);
        support(&package, &[]);
        let imports = format!(
            "from pyviews.{named_module} import {named_declaration} as Named, {named_declaration}Like as NamedLike\n\
             from pyviews.{generic_module} import {generic_declaration} as Bucket\n\
             from pyviews.{derived_module} import Derived\n",
        );
        typecheck(
            &fixture.0,
            &format!(
                r#"{imports}
from typing import assert_type
assert_type(Named(), Named)
assert_type(Named(7), Named)
assert_type(Named.get_current(), Named | None)
def check(named: Named, generic: Bucket, derived: Derived) -> None:
    base: NamedLike = derived
    assert_type(named.echo_self(named), Named | None)
    assert_type(named.echo_generic(generic), Bucket | None)
    assert_type(generic.echo_self(generic), Bucket | None)
    assert_type(generic.echo_named(named), Named | None)
"#,
            ),
        );
        runtime(
            &fixture.0,
            &format!(
                r#"
from pyviews.{named_module} import {named_declaration} as Named
from pyviews.{generic_module} import {generic_declaration} as Bucket
assert Named is not Bucket
assert Named.__name__ == "{named_declaration}"
assert Bucket.__name__ == "{generic_declaration}"
"#,
            ),
        );
    }
}

fn enumeration(namespace: &str, name: &str) -> TypeMeta {
    TypeMeta::Enum {
        namespace: namespace.into(),
        name: name.into(),
        underlying: Box::new(TypeMeta::I32),
        members: vec![EnumMember {
            name: "Unknown".into(),
            value: 0,
            doc: None,
        }],
        is_flags: false,
        doc: None,
        deprecated: None,
    }
}

#[test]
fn python_element_factory_and_dispatcher_helpers_resolve_colliding_names() {
    for packaged in [false, true] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let xaml = "Microsoft.UI.Xaml";
        let dispatching = "Microsoft.UI.Dispatching";
        let mut classes = Vec::new();
        for namespace in [xaml, "Shadow"] {
            for name in [
                "ElementFactoryGetArgs",
                "ElementFactoryRecycleArgs",
                "UIElement",
            ] {
                classes.push(class(
                    namespace,
                    name,
                    vec![],
                    0x51931100 + classes.len() as u32,
                ));
            }
        }
        let factory = InterfaceMeta {
            namespace: xaml.into(),
            name: "IElementFactory".into(),
            iid: "75faba47-2cf2-54ae-91e6-0581556fddaa".into(),
            methods: vec![
                MethodMeta {
                    name: "GetElement".into(),
                    raw_name: "GetElement".into(),
                    vtable_index: 6,
                    params: vec![ParamMeta {
                        name: "args".into(),
                        typ: class_type(xaml, "ElementFactoryGetArgs"),
                        direction: ParamDirection::In,
                    }],
                    return_type: Some(class_type(xaml, "UIElement")),
                    ..Default::default()
                },
                MethodMeta {
                    name: "RecycleElement".into(),
                    raw_name: "RecycleElement".into(),
                    vtable_index: 7,
                    params: vec![ParamMeta {
                        name: "args".into(),
                        typ: class_type(xaml, "ElementFactoryRecycleArgs"),
                        direction: ParamDirection::In,
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let priorities = [
            enumeration(dispatching, "DispatcherQueuePriority"),
            enumeration("Shadow", "DispatcherQueuePriority"),
        ];
        let queue = class(
            dispatching,
            "DispatcherQueue",
            vec![echo("EchoPriority", priorities[0].clone(), 6)],
            0x51931110,
        );
        classes.push(queue.clone());
        let context = python::PythonProjectionContext::new(
            classes
                .iter()
                .map(class_identity)
                .chain(classes.iter().filter_map(|class| {
                    class
                        .default_interface
                        .as_ref()
                        .map(InterfaceMeta::type_identity)
                }))
                .chain(priorities.iter().map(TypeMeta::type_identity))
                .chain([factory.type_identity()]),
            packaged,
        )
        .unwrap();
        let stub = python_stub::generate_interface_stub(&context, &factory);
        let get_args = context.reference_name_for_type(&class_type(xaml, "ElementFactoryGetArgs"));
        let recycle_args =
            context.reference_name_for_type(&class_type(xaml, "ElementFactoryRecycleArgs"));
        let element = context.reference_name_for_type(&class_type(xaml, "UIElement"));
        assert_ne!(get_args, "ElementFactoryGetArgs");
        assert!(
            stub.contains(&format!(
                "get_element: Callable[['{get_args}'], '{element}']"
            )),
            "{stub}"
        );
        assert!(
            stub.contains(&format!(
                "recycle_element: Callable[['{recycle_args}'], object]"
            )),
            "{stub}"
        );
        let factory_source = python::generate_interface(&context, &factory);
        for name in ["ElementFactoryGetArgs", "ElementFactoryRecycleArgs"] {
            let typ = class_type(xaml, name);
            assert!(
                factory_source.contains(&format!(
                    "'{}', '{}'",
                    context.implementation_module_for_type(&typ),
                    context.projected_name_for_type(&typ),
                )),
                "runtime lookup must target the declaring module, not a consumer alias:\n{factory_source}",
            );
        }
        let ui_element_interface = classes
            .iter()
            .find(|class| class.namespace == xaml && class.name == "UIElement")
            .unwrap()
            .default_interface
            .as_ref()
            .unwrap();
        let ui_element_iid = format!(
            "IID_{}",
            context.reference_name(&ui_element_interface.type_identity()),
        );
        assert_ne!(
            ui_element_iid, "IID_IUIElement",
            "fixture must qualify the interface IID"
        );
        assert!(
            factory_source.contains(&format!(
                "'{}', '{ui_element_iid}'",
                context.implementation_module_for_type(&class_type(xaml, "UIElement")),
            )),
            "the extension must load the IID exported by the class module:\n{factory_source}",
        );
        module(
            &package,
            &context.implementation_module_for_interface(&factory),
            factory_source,
            stub,
        );
        for class in &classes {
            let source = python::generate_class(&context, class, &Default::default());
            let stub = python_stub::generate_class_stub(&context, class, &Default::default());
            if class.namespace == xaml && class.name == "UIElement" {
                assert!(
                    source.contains(&format!("{ui_element_iid} = WinGUID.parse(")),
                    "{source}"
                );
            }
            if class.name == "DispatcherQueue" {
                let priority = context.reference_name_for_type(&priorities[0]);
                assert_ne!(priority, "DispatcherQueuePriority");
                assert!(stub.contains(&format!("priority: '{priority}'")), "{stub}");
            }
            module(
                &package,
                &context.implementation_module(&class_identity(class)),
                source,
                stub,
            );
        }
        for typ in &priorities {
            module(
                &package,
                &context.implementation_module_for_type(typ),
                python::generate_enum(&context, typ).unwrap(),
                python_stub::generate_enum_stub(&context, typ).unwrap(),
            );
        }
        support(&package, &[]);
        typecheck(
            &fixture.0,
            &format!(
                r#"
from typing import assert_type
from pyviews.{factory_module} import IElementFactory
from pyviews.{get_module} import ElementFactoryGetArgs
from pyviews.{recycle_module} import ElementFactoryRecycleArgs
from pyviews.{element_module} import UIElement
from pyviews.{queue_module} import DispatcherQueue
from pyviews.{priority_module} import DispatcherQueuePriority
def element(args: ElementFactoryGetArgs) -> UIElement:
    raise NotImplementedError
def recycle(args: ElementFactoryRecycleArgs) -> None:
    pass
assert_type(IElementFactory.create(element, recycle), IElementFactory)
async def schedule(queue: DispatcherQueue) -> None:
    assert_type(await queue.enqueue_with_priority_async(DispatcherQueuePriority.Unknown, lambda: 7), int)
"#,
                factory_module = context.implementation_module_for_interface(&factory),
                get_module = context
                    .implementation_module_for_type(&class_type(xaml, "ElementFactoryGetArgs")),
                recycle_module = context
                    .implementation_module_for_type(&class_type(xaml, "ElementFactoryRecycleArgs")),
                element_module =
                    context.implementation_module_for_type(&class_type(xaml, "UIElement")),
                queue_module = context.implementation_module(&class_identity(&queue)),
                priority_module = context.implementation_module_for_type(&priorities[0]),
            ),
        );
        runtime(
            &fixture.0,
            &format!(
                r#"
import dynwinrt as dw
from pyviews.{factory_module} import IElementFactory
def unused(args):
    raise AssertionError("factory creation must not invoke callbacks")
with dw.RoApartment(1):
    factory = IElementFactory.create(unused, unused)
    try:
        assert type(factory) is IElementFactory
    finally:
        factory.release_callbacks()
        dw.release_projected(factory)
"#,
                factory_module = context.implementation_module_for_interface(&factory),
            ),
        );
    }
}

fn iid(file: &mut writer::File, definition: writer::TypeDef, id: u32) {
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
        writer::HasAttribute::TypeDef(definition),
        writer::AttributeType::MemberRef(constructor),
        &[
            Value::U32(id),
            Value::U16(0x6281),
            Value::U16(0x4900),
            Value::U8(0xb7),
            Value::U8(0x82),
            Value::U8(4),
            Value::U8(3),
            Value::U8(2),
            Value::U8(1),
            Value::U8(9),
            Value::U8(0x10),
        ]
        .into_iter()
        .map(|value| (String::new(), value))
        .collect::<Vec<_>>(),
    );
}

fn interface(file: &mut writer::File, namespace: &str, name: &str, id: u32) -> writer::TypeDef {
    let definition = file.TypeDef(
        namespace,
        name,
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public
            | TypeAttributes::Interface
            | TypeAttributes::Abstract
            | TypeAttributes::WindowsRuntime,
    );
    iid(file, definition, id);
    definition
}

fn structure(file: &mut writer::File, namespace: &str, name: &str, fields: &[(&str, Type)]) {
    let base = file.TypeRef("System", "ValueType");
    file.TypeDef(
        namespace,
        name,
        writer::TypeDefOrRef::TypeRef(base),
        TypeAttributes::Public
            | TypeAttributes::Sealed
            | TypeAttributes::SequentialLayout
            | TypeAttributes::WindowsRuntime,
    );
    for (name, typ) in fields {
        file.Field(name, typ, FieldAttributes::Public);
    }
}

fn method(
    file: &mut writer::File,
    name: &str,
    result: Type,
    params: &[(&str, Type, ParamAttributes)],
) {
    file.MethodDef(
        name,
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: result,
            types: params.iter().map(|(_, typ, _)| typ.clone()).collect(),
        },
        MethodAttributes::Public
            | MethodAttributes::Abstract
            | MethodAttributes::Virtual
            | MethodAttributes::NewSlot,
        MethodImplAttributes::default(),
    );
    for (index, (name, _, flags)) in params.iter().enumerate() {
        file.Param(name, index as u16 + 1, *flags);
    }
}

fn runtime_class(file: &mut writer::File, name: &str, default: &str) {
    let object = file.TypeRef("System", "Object");
    let definition = file.TypeDef(
        "Audit",
        name,
        writer::TypeDefOrRef::TypeRef(object),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    let implementation = file.InterfaceImpl(definition, &Type::named("Audit", default));
    let attribute = file.TypeRef("Windows.Foundation.Metadata", "DefaultAttribute");
    let constructor = file.MemberRef(
        ".ctor",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::Void,
            types: vec![],
        },
        writer::MemberRefParent::TypeRef(attribute),
    );
    file.Attribute(
        writer::HasAttribute::InterfaceImpl(implementation),
        writer::AttributeType::MemberRef(constructor),
        &[],
    );
}

fn class_companion_metadata(path: &Path, peer_name: &str) {
    let mut file = writer::File::new("PythonClassCompanions");
    let peer_interface = format!("I{peer_name}");
    for (index, (name, target)) in [("IWidget", peer_name), (peer_interface.as_str(), "Widget")]
        .into_iter()
        .enumerate()
    {
        interface(&mut file, "Audit", name, 0x51931500 + index as u32);
        let target = Type::named("Audit", target);
        method(
            &mut file,
            "Echo",
            target.clone(),
            &[("value", target, ParamAttributes::In)],
        );
    }
    interface(&mut file, "Audit", "IUse", 0x51931502);
    for (name, target) in [("EchoOwner", "Widget"), ("EchoPeer", peer_name)] {
        let target = Type::named("Audit", target);
        method(
            &mut file,
            name,
            target.clone(),
            &[("value", target, ParamAttributes::In)],
        );
    }
    runtime_class(&mut file, "Widget", "IWidget");
    runtime_class(&mut file, peer_name, &peer_interface);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

fn metadata(path: &Path) {
    let mut file = writer::File::new("PythonSymbolMapping");
    let alpha = Type::named("Alpha", "URLValue");
    let beta = Type::named("Beta", "UrlValue");
    let pair = Type::named("Containers", "NormalizedPair");
    structure(&mut file, "Alpha", "URLValue", &[("Value", Type::I32)]);
    structure(&mut file, "Beta", "UrlValue", &[("Other", Type::I64)]);
    structure(
        &mut file,
        "Containers",
        "NormalizedPair",
        &[("First", alpha.clone()), ("Second", beta.clone())],
    );
    interface(&mut file, "Audit", "IAlpha", 0x51931205);
    method(
        &mut file,
        "EchoAlpha",
        alpha.clone(),
        &[("value", alpha.clone(), ParamAttributes::In)],
    );
    let callback_base = file.TypeRef("System", "MulticastDelegate");
    let callback = file.TypeDef(
        "Audit",
        "PairCallback",
        writer::TypeDefOrRef::TypeRef(callback_base),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    iid(&mut file, callback, 0x51931200);
    method(&mut file, ".ctor", Type::Void, &[]);
    method(
        &mut file,
        "Invoke",
        Type::Void,
        &[
            ("first", alpha.clone(), ParamAttributes::In),
            ("second", beta.clone(), ParamAttributes::In),
        ],
    );
    interface(&mut file, "Audit", "IHelpers", 0x51931201);
    for (name, typ) in [
        ("EchoAlpha", alpha.clone()),
        ("EchoBeta", beta.clone()),
        ("EchoPair", pair.clone()),
        ("EchoArray", Type::Array(Box::new(alpha.clone()))),
    ] {
        method(
            &mut file,
            name,
            typ.clone(),
            &[("value", typ, ParamAttributes::In)],
        );
    }
    method(
        &mut file,
        "Split",
        alpha,
        &[("second", beta, ParamAttributes::Out)],
    );
    method(
        &mut file,
        "UseCallback",
        pair,
        &[(
            "callback",
            Type::named("Audit", "PairCallback"),
            ParamAttributes::In,
        )],
    );

    let generic = interface(&mut file, "Enums", "IBucket`1", 0x51931202);
    file.GenericParam(
        "T",
        writer::TypeOrMethodDef::TypeDef(generic),
        0,
        GenericParamAttributes::default(),
    );
    method(&mut file, "Ping", Type::Void, &[]);
    let base = file.TypeRef("System", "Enum");
    file.TypeDef(
        "Enums",
        "IBucket_String",
        writer::TypeDefOrRef::TypeRef(base),
        TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
    );
    file.Field(
        "value__",
        &Type::I32,
        FieldAttributes::Public | FieldAttributes::SpecialName | FieldAttributes::RTSpecialName,
    );
    let kind = Type::named("Enums", "IBucket_String");
    let member = file.Field(
        "Unknown",
        &kind,
        FieldAttributes::Public
            | FieldAttributes::Static
            | FieldAttributes::Literal
            | FieldAttributes::HasDefault,
    );
    file.Constant(writer::HasConstant::Field(member), &Value::I32(0));
    let bucket = Type::Name(TypeName {
        namespace: "Enums".into(),
        name: "IBucket`1".into(),
        generics: vec![Type::String],
    });
    interface(&mut file, "Enums", "IUse", 0x51931203);
    method(&mut file, "GetBucket", bucket.clone(), &[]);
    method(
        &mut file,
        "EchoKind",
        kind.clone(),
        &[("value", kind.clone(), ParamAttributes::In)],
    );
    method(
        &mut file,
        "EchoKinds",
        Type::Array(Box::new(kind.clone())),
        &[(
            "value",
            Type::Array(Box::new(kind.clone())),
            ParamAttributes::In,
        )],
    );
    interface(&mut file, "Consumers", "IForeign", 0x51931204);
    method(&mut file, "GetBucket", bucket, &[]);
    method(
        &mut file,
        "EchoKind",
        kind.clone(),
        &[("value", kind, ParamAttributes::In)],
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
}

fn helper_interface(path: &Path) -> InterfaceMeta {
    meta::parse_interfaces(path.to_str().unwrap(), "Audit")
        .into_iter()
        .find(|interface| interface.name == "IHelpers")
        .unwrap()
}

fn structures(interface: &InterfaceMeta) -> Vec<TypeMeta> {
    interface
        .methods
        .iter()
        .take(3)
        .map(|method| method.return_type.clone().unwrap())
        .collect()
}

fn write_structs(
    package: &Path,
    context: &python::PythonProjectionContext,
    structures: &[TypeMeta],
) {
    for typ in structures {
        module(
            package,
            &context.implementation_module_for_type(typ),
            python::generate_struct(context, typ).unwrap(),
            python_stub::generate_struct_stub(context, typ).unwrap(),
        );
    }
}

fn imported_symbol(source: &str, module: &str, declaration: &str) -> String {
    let prefix = format!("from .{module} import ");
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix(&prefix))
        .flat_map(|imports| imports.split('#').next().unwrap().split(','))
        .find_map(|import| {
            let words = import.split_whitespace().collect::<Vec<_>>();
            (words.first().copied() == Some(declaration)).then(|| words.last().unwrap().to_string())
        })
        .unwrap_or_else(|| panic!("missing {declaration} from {module}:\n{source}"))
}

fn registration_metadata(path: &Path, initial_collision: bool) -> (InterfaceMeta, Vec<TypeMeta>) {
    let mut file = writer::File::new("PythonRegistrationSymbols");
    structure(&mut file, "Alpha", "URLValue", &[("Value", Type::I32)]);
    structure(&mut file, "Beta", "UrlValue", &[("Other", Type::I64)]);
    let name = if initial_collision {
        "pack_url_value"
    } else {
        "pack_beta_url_value_struct"
    };
    interface(&mut file, "Audit", name, 0x51931500);
    let alpha = Type::named("Alpha", "URLValue");
    method(
        &mut file,
        "EchoAlpha",
        alpha.clone(),
        &[("value", alpha, ParamAttributes::In)],
    );
    if !initial_collision {
        let beta = Type::named("Beta", "UrlValue");
        method(
            &mut file,
            "EchoBeta",
            beta.clone(),
            &[("value", beta, ParamAttributes::In)],
        );
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
    let interface = meta::parse_interfaces(path.to_str().unwrap(), "Audit")
        .pop()
        .unwrap();
    let structures = interface
        .methods
        .iter()
        .map(|method| method.return_type.clone().unwrap())
        .collect();
    (interface, structures)
}

fn unused_registration_identities() -> impl Iterator<Item = TypeIdentity> {
    ["unpack_url_value", "URLValue_TYPE", "IActivationFactory"]
        .into_iter()
        .map(|name| TypeIdentity::named(TypeIdentityKind::Interface, "Unrelated", name))
}

fn registration_bindings(source: &str, expected: &[&str]) -> BTreeMap<String, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut registrations = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(symbol) = line.strip_suffix(" = DynWinRTType.register_interface(") else {
            continue;
        };
        assert!(!symbol.starts_with(char::is_whitespace), "{line}");
        let arguments = lines[index + 1].trim();
        let quote = arguments.chars().next().unwrap();
        assert!(matches!(quote, '\'' | '"'), "{arguments}");
        let name = arguments[1..].split(quote).next().unwrap();
        assert!(
            registrations
                .insert(name.to_string(), symbol.to_string())
                .is_none(),
            "duplicate registration for {name}:\n{source}"
        );
        let assignments = lines
            .iter()
            .filter(|line| {
                line.starts_with(&format!("{symbol} = "))
                    || line.starts_with(&format!("def {symbol}("))
                    || line.starts_with(&format!("class {symbol}:"))
                    || line.starts_with(&format!("class {symbol}("))
            })
            .count();
        let imports = lines
            .iter()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("from ")?
                    .split_once(" import ")
                    .map(|(_, names)| names)
            })
            .flat_map(|names| names.split('#').next().unwrap().split(','))
            .filter(|name| name.split_whitespace().last() == Some(symbol))
            .count();
        assert_eq!(
            assignments + imports,
            1,
            "native registration {symbol} must not share its binding with any helper or type:\n{source}"
        );
        assert!(
            source.contains(&format!("{symbol}.method(")),
            "generated calls must reference their allocated registration {symbol}:\n{source}"
        );
    }
    assert_eq!(
        registrations
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        expected.iter().copied().collect(),
        "reserve exactly the registrations this module actually emits:\n{source}",
    );
    registrations
}

fn registration_struct_modules(
    package: &Path,
    context: &python::PythonProjectionContext,
    structures: &[TypeMeta],
) {
    let isolated = python::PythonProjectionContext::new(
        structures.iter().map(TypeMeta::type_identity),
        context.is_packaged(),
    )
    .unwrap();
    for structure in structures {
        let source = python::generate_struct(context, structure).unwrap();
        let stub = python_stub::generate_struct_stub(context, structure).unwrap();
        assert_eq!(
            source,
            python::generate_struct(&isolated, structure).unwrap(),
            "non-emitted interface registrations must not alter an owning struct's public API"
        );
        assert_eq!(
            stub,
            python_stub::generate_struct_stub(&isolated, structure).unwrap()
        );
        module(
            package,
            &context.implementation_module_for_type(structure),
            source,
            stub,
        );
    }
}

#[test]
fn python_native_registration_interface_symbols_do_not_shadow_struct_helpers() {
    for packaged in [true, false] {
        for initial_collision in [true, false] {
            let fixture = Fixture::new();
            let package = fixture.package();
            let (interface, structures) = registration_metadata(
                &fixture.0.join("metadata").join("Input.winmd"),
                initial_collision,
            );
            let identities = structures
                .iter()
                .map(TypeMeta::type_identity)
                .chain([interface.type_identity()])
                .collect::<Vec<_>>();
            let context =
                python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
            let reordered =
                python::PythonProjectionContext::new(identities.iter().rev().cloned(), packaged)
                    .unwrap();
            let unrelated = python::PythonProjectionContext::new(
                identities
                    .into_iter()
                    .chain(unused_registration_identities()),
                packaged,
            )
            .unwrap();
            let source = python::generate_interface(&context, &interface);
            let stub = python_stub::generate_interface_stub(&context, &interface);
            for alternative in [&reordered, &unrelated] {
                assert_eq!(source, python::generate_interface(alternative, &interface));
                assert_eq!(
                    stub,
                    python_stub::generate_interface_stub(alternative, &interface)
                );
            }
            let registrations = registration_bindings(&source, &[&interface.name]);
            let owner_module = context.implementation_module_for_interface(&interface);
            module(&package, &owner_module, source, stub);
            registration_struct_modules(&package, &unrelated, &structures);
            support(
                &package,
                if packaged {
                    std::slice::from_ref(&owner_module)
                } else {
                    &[]
                },
            );
            let alpha_module = context.implementation_module_for_type(&structures[0]);
            let payload_module = if packaged {
                &alpha_module
            } else {
                &owner_module
            };
            let mut imports = format!(
                "from pyviews.{owner_module} import {name} as Projection\n\
                 from pyviews.{payload_module} import URLValue as Alpha\n",
                name = context.projected_name_for_interface(&interface),
            );
            let mut beta_check = String::new();
            let mut beta_runtime = String::new();
            let mut beta_public = String::new();
            if let Some(beta) = structures.get(1) {
                let beta_module = context.implementation_module_for_type(beta);
                let payload_module = if packaged {
                    &beta_module
                } else {
                    &owner_module
                };
                imports.push_str(&format!(
                    "from pyviews.{payload_module} import UrlValue as Beta\n"
                ));
                beta_check.push_str("    assert_type(value.echo_beta(Beta(2**40)), Beta)\n");
                beta_runtime
                    .push_str("    assert view.echo_beta(Beta(2**40)) == Beta(2**40 + 1)\n");
                beta_public = format!(
                    "from pyviews.{beta_module} import UrlValue as CanonicalBeta, pack_url_value as pack_beta, unpack_url_value as unpack_beta\n\
                     assert unpack_beta(pack_beta(CanonicalBeta(2**40)).to_value()) == CanonicalBeta(2**40)\n"
                );
            }
            typecheck(
                &fixture.0,
                &format!(
                    r#"{imports}
from typing import assert_type
def check(value: Projection) -> None:
    assert_type(value.echo_alpha(Alpha(17)), Alpha)
{beta_check}
"#
                ),
            );
            reject_consumer(
                &fixture.0,
                &format!(
                    r#"{imports}
def reject(value: Projection) -> None:
    value.echo_alpha(17)
    wrong: str = value.echo_alpha(Alpha(17))
"#
                ),
                &["[arg-type]", "[assignment]"],
            );
            runtime(
                &fixture.0,
                &format!(
                    r#"{imports}
import importlib
import struct
import typing
import dynwinrt as dw
from pyviews.{alpha_module} import URLValue as CanonicalAlpha, URLValue_TYPE, pack_url_value as pack_alpha, unpack_url_value as unpack_alpha
assert struct.calcsize("P") == 8
assert isinstance(URLValue_TYPE, dw.DynWinRTType)
assert typing.get_type_hints(pack_alpha)["v"] is CanonicalAlpha
assert typing.get_type_hints(unpack_alpha)["return"] is CanonicalAlpha
assert unpack_alpha(pack_alpha(CanonicalAlpha(17)).to_value()) == CanonicalAlpha(17)
{beta_public}
owner = importlib.import_module("pyviews.{owner_module}")
registrations = {registrations}
for symbol in registrations.values():
    assert isinstance(getattr(owner, symbol), dw.DynWinRTType)
class Handler:
    def __init__(self): self.calls = []
    def echo_alpha(self, value):
        self.calls.append(("alpha", value.value))
        return type(value)(value.value + 1)
    def echo_beta(self, value):
        self.calls.append(("beta", value.other))
        return type(value)(value.other + 1)
handler = Handler()
with dw.RoApartment(1), Projection.implement(handler) as implementation:
    view = implementation.value
    assert view.echo_alpha(Alpha(17)) == Alpha(18)
{beta_runtime}
assert handler.calls == {expected_calls}
for symbol in registrations.values():
    assert isinstance(getattr(owner, symbol), dw.DynWinRTType)
"#,
                    registrations = serde_json::to_string(&registrations).unwrap(),
                    expected_calls = if initial_collision {
                        "[('alpha', 17)]"
                    } else {
                        "[('alpha', 17), ('beta', 2**40)]"
                    },
                ),
            );
        }
    }
}

#[test]
fn python_native_registration_class_routes_reserve_all_emitted_interfaces() {
    for packaged in [true, false] {
        for initial_collision in [true, false] {
            for shared_required in [true, false] {
                let fixture = Fixture::new();
                let package = fixture.package();
                let (default, structures) = registration_metadata(
                    &fixture.0.join("metadata").join("Input.winmd"),
                    initial_collision,
                );
                let mut required = default.clone();
                required.name = "unpack_alpha_url_value_struct".into();
                required.iid = "51931501-6281-4900-b782-040302010910".into();
                for method in &mut required.methods {
                    method.name = format!("Required{}", method.name);
                    method.raw_name = method.name.clone();
                }
                let mut statics = default.clone();
                statics.name = "unpack_beta_url_value_struct".into();
                statics.iid = "51931502-6281-4900-b782-040302010910".into();
                for method in &mut statics.methods {
                    method.name = format!("Static{}", method.name);
                    method.raw_name = method.name.clone();
                }
                let factory = InterfaceMeta {
                    namespace: "Audit".into(),
                    name: "pack_alpha_url_value_struct".into(),
                    iid: "51931503-6281-4900-b782-040302010910".into(),
                    methods: vec![MethodMeta {
                        name: "CreateWrapper".into(),
                        raw_name: "CreateWrapper".into(),
                        vtable_index: 6,
                        params: structures
                            .iter()
                            .enumerate()
                            .map(|(index, typ)| ParamMeta {
                                name: format!("value{index}"),
                                typ: typ.clone(),
                                direction: ParamDirection::In,
                            })
                            .collect(),
                        return_type: Some(class_type("Audit", "Wrapper")),
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                let wrapper = ClassMeta {
                    namespace: "Audit".into(),
                    name: "Wrapper".into(),
                    full_name: "Audit.Wrapper".into(),
                    default_interface: Some(default.clone()),
                    required_interfaces: vec![required.clone()],
                    static_interfaces: vec![statics.clone()],
                    factory_interfaces: vec![factory.clone()],
                    constructors: vec![ConstructorMeta {
                        kind: ConstructorKind::FactoryActivation,
                        factory_interface: Some(TypeRef {
                            namespace: factory.namespace.clone(),
                            name: factory.name.clone(),
                            kind: TypeKind::Interface,
                        }),
                    }],
                    ..Default::default()
                };
                let identities = structures
                    .iter()
                    .map(TypeMeta::type_identity)
                    .chain(wrapper.all_interfaces().map(InterfaceMeta::type_identity))
                    .chain([class_identity(&wrapper)])
                    .collect::<Vec<_>>();
                let context =
                    python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
                let reordered = python::PythonProjectionContext::new(
                    identities.iter().rev().cloned(),
                    packaged,
                )
                .unwrap();
                let unrelated = python::PythonProjectionContext::new(
                    identities
                        .into_iter()
                        .chain(unused_registration_identities()),
                    packaged,
                )
                .unwrap();
                let shared_iids = if shared_required {
                    HashSet::from([required.iid.clone()])
                } else {
                    HashSet::new()
                };
                let source = python::generate_class(&context, &wrapper, &shared_iids);
                let stub = python_stub::generate_class_stub(&context, &wrapper, &shared_iids);
                let required_module = context.implementation_module_for_interface(&required);
                let required_symbol = context.reference_name(&required.type_identity());
                if shared_required {
                    assert_eq!(
                        imported_symbol(
                            &source,
                            &required_module,
                            &context.projected_name_for_interface(&required)
                        ),
                        required_symbol,
                    );
                    assert!(
                        !source.contains(&format!("\nclass {required_symbol}:")),
                        "{source}"
                    );
                } else {
                    assert!(
                        source.contains(&format!("\nclass {required_symbol}:")),
                        "{source}"
                    );
                    assert!(
                        stub.contains(&format!("\nclass {required_symbol}:")),
                        "{stub}"
                    );
                    assert!(
                        !source.contains(&format!("from .{required_module} import ")),
                        "{source}"
                    );
                }
                for alternative in [&reordered, &unrelated] {
                    assert_eq!(
                        source,
                        python::generate_class(alternative, &wrapper, &shared_iids)
                    );
                    assert_eq!(
                        stub,
                        python_stub::generate_class_stub(alternative, &wrapper, &shared_iids)
                    );
                }
                let expected = wrapper
                    .all_interfaces()
                    .map(|interface| interface.name.as_str())
                    .collect::<Vec<_>>();
                let registrations = registration_bindings(&source, &expected);
                assert!(!registrations.contains_key("IActivationFactory"));
                for interface in wrapper.all_interfaces() {
                    let iid = format!("IID_{}", context.reference_name(&interface.type_identity()));
                    assert!(
                        source.contains(&format!("\"{}\", {iid})", interface.name)),
                        "{source}"
                    );
                    assert!(source.contains(&interface.iid), "{source}");
                }
                assert!(
                    source.contains("activation_factory('Audit.Wrapper')"),
                    "{source}"
                );
                let wrapper_module = context.implementation_module(&class_identity(&wrapper));
                module(&package, &wrapper_module, source, stub);
                for interface in [&default, &required] {
                    module(
                        &package,
                        &context.implementation_module_for_interface(interface),
                        python::generate_interface(&context, interface),
                        python_stub::generate_interface_stub(&context, interface),
                    );
                }
                registration_struct_modules(&package, &unrelated, &structures);
                let implementation_modules = [&default, &required]
                    .map(|interface| context.implementation_module_for_interface(interface));
                support(
                    &package,
                    if packaged {
                        &implementation_modules
                    } else {
                        &[]
                    },
                );
                let alpha_module = context.implementation_module_for_type(&structures[0]);
                let payload_module = if packaged {
                    &alpha_module
                } else {
                    &wrapper_module
                };
                let mut imports = format!(
                    "from pyviews.{wrapper_module} import Wrapper\n\
                 from pyviews.{payload_module} import URLValue as Alpha\n",
                );
                let mut beta_check = String::new();
                let mut beta_runtime = String::new();
                let mut beta_constructor = String::new();
                if let Some(beta) = structures.get(1) {
                    let beta_module = context.implementation_module_for_type(beta);
                    let payload_module = if packaged {
                        &beta_module
                    } else {
                        &wrapper_module
                    };
                    imports.push_str(&format!(
                        "from pyviews.{payload_module} import UrlValue as Beta\n"
                    ));
                    beta_check = "    assert_type(value.echo_beta(Beta(2**40)), Beta)\n    assert_type(value.required_echo_beta(Beta(2**40)), Beta)\n    assert_type(Wrapper.static_echo_beta(Beta(2**40)), Beta)\n".into();
                    beta_runtime = "        assert value.echo_beta(Beta(2**40)) == Beta(2**40 + 1)\n        assert value.required_echo_beta(Beta(2**40)) == Beta(2**40 + 2)\n".into();
                    beta_constructor = ", Beta(2**40)".into();
                }
                typecheck(
                    &fixture.0,
                    &format!(
                        r#"{imports}
from typing import assert_type
def check(value: Wrapper) -> None:
    assert_type(value.echo_alpha(Alpha(17)), Alpha)
    assert_type(value.required_echo_alpha(Alpha(17)), Alpha)
    assert_type(Wrapper.static_echo_alpha(Alpha(17)), Alpha)
    assert_type(Wrapper(Alpha(17){beta_constructor}), Wrapper)
{beta_check}
"#
                    ),
                );
                reject_consumer(
                    &fixture.0,
                    &format!(
                        r#"{imports}
def reject(value: Wrapper) -> None:
    value.echo_alpha("wrong")
    value.required_echo_alpha("wrong")
"#
                    ),
                    &["[arg-type]", "[arg-type]"],
                );
                runtime(
                    &fixture.0,
                    &format!(
                        r#"{imports}
import importlib
import struct
import dynwinrt as dw
from pyviews.{default_module} import {default_name} as DefaultProjection
from pyviews.{required_module} import {required_name} as RequiredProjection
assert struct.calcsize("P") == 8
owner = importlib.import_module("pyviews.{wrapper_module}")
registrations = {registrations}
for symbol in registrations.values():
    assert isinstance(getattr(owner, symbol), dw.DynWinRTType)
class DefaultHandler:
    def __init__(self): self.calls = []
    def echo_alpha(self, value):
        self.calls.append(("alpha", value.value))
        return type(value)(value.value + 1)
    def echo_beta(self, value):
        self.calls.append(("beta", value.other))
        return type(value)(value.other + 1)
class RequiredHandler:
    def __init__(self): self.calls = []
    def required_echo_alpha(self, value):
        self.calls.append(("alpha", value.value))
        return type(value)(value.value + 2)
    def required_echo_beta(self, value):
        self.calls.append(("beta", value.other))
        return type(value)(value.other + 2)
default, required = DefaultHandler(), RequiredHandler()
with dw.RoApartment(1), DefaultProjection.implement(default, interfaces=[(RequiredProjection, required)]) as implementation:
    value = Wrapper._from_native(implementation.value._obj)
    try:
        assert value.echo_alpha(Alpha(17)) == Alpha(18)
        assert value.required_echo_alpha(Alpha(17)) == Alpha(19)
{beta_runtime}
    finally:
        dw.release_projected(value)
assert default.calls == {expected_calls}
assert required.calls == {expected_calls}
for symbol in registrations.values():
    assert isinstance(getattr(owner, symbol), dw.DynWinRTType)
"#,
                        default_module = implementation_modules[0],
                        required_module = implementation_modules[1],
                        default_name = context.projected_name_for_interface(&default),
                        required_name = context.projected_name_for_interface(&required),
                        registrations = serde_json::to_string(&registrations).unwrap(),
                        expected_calls = if initial_collision {
                            "[('alpha', 17)]"
                        } else {
                            "[('alpha', 17), ('beta', 2**40)]"
                        },
                    ),
                );
            }
        }
    }
}

#[test]
fn python_native_registration_activation_factory_reserves_only_when_emitted() {
    for packaged in [true, false] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let kind = enumeration("Kinds", "_IActivationFactory");
        let mut owner = class(
            "Audit",
            "ActivationOwner",
            vec![echo("EchoKind", kind.clone(), 6)],
            0x51931510,
        );
        owner.has_default_constructor = true;
        let identities = [
            class_identity(&owner),
            owner.default_interface.as_ref().unwrap().type_identity(),
            kind.type_identity(),
        ];
        let context = python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
        let unrelated = python::PythonProjectionContext::new(
            identities
                .into_iter()
                .chain(unused_registration_identities()),
            packaged,
        )
        .unwrap();
        let kind_module = context.implementation_module_for_type(&kind);
        let inactive = python::generate_class(&context, &owner, &Default::default());
        let inactive_stub = python_stub::generate_class_stub(&context, &owner, &Default::default());
        assert_eq!(
            inactive,
            python::generate_class(&unrelated, &owner, &Default::default())
        );
        assert_eq!(
            inactive_stub,
            python_stub::generate_class_stub(&unrelated, &owner, &Default::default())
        );
        assert_eq!(
            imported_symbol(&inactive, &kind_module, "_IActivationFactory"),
            "_IActivationFactory"
        );
        assert_eq!(
            imported_symbol(&inactive_stub, &kind_module, "_IActivationFactory"),
            "_IActivationFactory"
        );
        registration_bindings(&inactive, &["IActivationOwner"]);
        owner.constructors.push(ConstructorMeta {
            kind: ConstructorKind::DefaultActivation,
            factory_interface: None,
        });
        let source = python::generate_class(&context, &owner, &Default::default());
        let stub = python_stub::generate_class_stub(&context, &owner, &Default::default());
        let registrations =
            registration_bindings(&source, &["IActivationOwner", "IActivationFactory"]);
        let kind_symbol = imported_symbol(&source, &kind_module, "_IActivationFactory");
        assert_ne!(registrations["IActivationFactory"], kind_symbol);
        assert!(
            source.contains("00000035-0000-0000-c000-000000000046"),
            "{source}"
        );
        let owner_module = context.implementation_module(&class_identity(&owner));
        module(&package, &owner_module, source, stub);
        module(
            &package,
            &kind_module,
            python::generate_enum(&context, &kind).unwrap(),
            python_stub::generate_enum_stub(&context, &kind).unwrap(),
        );
        support(&package, &[]);
        typecheck(
            &fixture.0,
            &format!(
                r#"
from typing import assert_type
from pyviews.{owner_module} import ActivationOwner
from pyviews.{kind_module} import _IActivationFactory as Kind
def check(value: ActivationOwner) -> None:
    assert_type(value.echo_kind(Kind.Unknown), Kind)
    assert_type(ActivationOwner(), ActivationOwner)
"#
            ),
        );
        runtime(
            &fixture.0,
            &format!(
                r#"
import importlib
import dynwinrt as dw
from pyviews.{kind_module} import _IActivationFactory as Kind
owner = importlib.import_module("pyviews.{owner_module}")
for symbol in {registrations}.values():
    assert isinstance(getattr(owner, symbol), dw.DynWinRTType)
assert Kind.Unknown.value == 0
"#,
                registrations = serde_json::to_string(&registrations).unwrap()
            ),
        );
    }
}

fn cross_role_metadata(path: &Path, enum_fields: bool) -> (InterfaceMeta, [TypeMeta; 3]) {
    let mut file = writer::File::new("PythonCrossRoleSymbols");
    let mut fields = vec![("Value", Type::I32)];
    if enum_fields {
        fields.extend([
            ("PackKind", Type::named("Kinds", "pack_url_value")),
            ("TypeKind", Type::named("Kinds", "URLValue_TYPE")),
        ]);
    }
    structure(&mut file, "Alpha", "URLValue", &fields);
    for name in ["pack_url_value", "URLValue_TYPE"] {
        let base = file.TypeRef("System", "Enum");
        file.TypeDef(
            "Kinds",
            name,
            writer::TypeDefOrRef::TypeRef(base),
            TypeAttributes::Public | TypeAttributes::Sealed | TypeAttributes::WindowsRuntime,
        );
        file.Field(
            "value__",
            &Type::I32,
            FieldAttributes::Public | FieldAttributes::SpecialName | FieldAttributes::RTSpecialName,
        );
        let member = file.Field(
            "Unknown",
            &Type::named("Kinds", name),
            FieldAttributes::Public
                | FieldAttributes::Static
                | FieldAttributes::Literal
                | FieldAttributes::HasDefault,
        );
        file.Constant(writer::HasConstant::Field(member), &Value::I32(0));
    }
    interface(&mut file, "Audit", "IUse", 0x51931400);
    for (name, typ) in [
        ("Echo", Type::named("Alpha", "URLValue")),
        ("EchoPackKind", Type::named("Kinds", "pack_url_value")),
        ("EchoTypeKind", Type::named("Kinds", "URLValue_TYPE")),
    ] {
        method(
            &mut file,
            name,
            typ.clone(),
            &[("value", typ, ParamAttributes::In)],
        );
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, file.into_stream()).unwrap();
    let interface = meta::parse_interfaces(path.to_str().unwrap(), "Audit")
        .pop()
        .unwrap();
    let types = std::array::from_fn(|index| interface.methods[index].return_type.clone().unwrap());
    (interface, types)
}

fn cross_role_modules(
    package: &Path,
    context: &python::PythonProjectionContext,
    types: &[TypeMeta; 3],
) {
    write_structs(package, context, &types[..1]);
    for enumeration in &types[1..] {
        module(
            package,
            &context.implementation_module_for_type(enumeration),
            python::generate_enum(context, enumeration).unwrap(),
            python_stub::generate_enum_stub(context, enumeration).unwrap(),
        );
    }
}

#[test]
fn python_cross_role_imported_enums_do_not_shadow_struct_helpers() {
    for packaged in [true, false] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let (interface, types) =
            cross_role_metadata(&fixture.0.join("metadata").join("Input.winmd"), false);
        let identities = types
            .iter()
            .map(TypeMeta::type_identity)
            .chain([interface.type_identity()])
            .collect::<Vec<_>>();
        let baseline = python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
        let context = python::PythonProjectionContext::new(
            identities
                .into_iter()
                .chain([enumeration("Unrelated", "unpack_url_value").type_identity()]),
            packaged,
        )
        .unwrap();
        let owner_module = context.implementation_module_for_interface(&interface);
        let struct_module = context.implementation_module_for_type(&types[0]);
        let pack_kind_module = context.implementation_module_for_type(&types[1]);
        let type_kind_module = context.implementation_module_for_type(&types[2]);
        let source = python::generate_interface(&context, &interface);
        let stub = python_stub::generate_interface_stub(&context, &interface);
        assert_eq!(
            source,
            python::generate_interface(&baseline, &interface),
            "a non-imported similarly named type must not change module symbols"
        );
        assert_eq!(
            stub,
            python_stub::generate_interface_stub(&baseline, &interface)
        );
        let pack_kind_alias = imported_symbol(&source, &pack_kind_module, "pack_url_value");
        let type_kind_alias = imported_symbol(&source, &type_kind_module, "URLValue_TYPE");
        if packaged {
            assert_eq!(
                imported_symbol(&source, &struct_module, "unpack_url_value"),
                "unpack_url_value"
            );
        } else {
            assert!(source.contains("def unpack_url_value("), "{source}");
        }
        module(&package, &owner_module, source, stub);
        cross_role_modules(&package, &context, &types);
        support(
            &package,
            if packaged {
                std::slice::from_ref(&owner_module)
            } else {
                &[]
            },
        );
        let payload_module = if packaged {
            &struct_module
        } else {
            &owner_module
        };
        let imports = format!(
            "from pyviews.{owner_module} import IUse\n\
             from pyviews.{payload_module} import URLValue\n\
             from pyviews.{pack_kind_module} import pack_url_value as PackKind\n\
             from pyviews.{type_kind_module} import URLValue_TYPE as TypeKind\n",
        );
        typecheck(
            &fixture.0,
            &format!(
                r#"{imports}
from typing import assert_type
def check(value: IUse) -> None:
    assert_type(value.echo(URLValue(17)), URLValue)
    assert_type(value.echo_pack_kind(PackKind.Unknown), PackKind)
    assert_type(value.echo_type_kind(TypeKind.Unknown), TypeKind)
"#
            ),
        );
        reject_consumer(
            &fixture.0,
            &format!(
                r#"{imports}
def reject(value: IUse) -> None:
    value.echo(PackKind.Unknown)
    value.echo_pack_kind(URLValue(17))
    wrong: TypeKind = value.echo_pack_kind(PackKind.Unknown)
"#
            ),
            &["[arg-type]", "[arg-type]", "[assignment]"],
        );
        runtime(
            &fixture.0,
            &format!(
                r#"{imports}
import typing
import dynwinrt as dw
from pyviews.{struct_module} import URLValue as Canonical, pack_url_value, unpack_url_value, URLValue_TYPE
assert isinstance(URLValue_TYPE, dw.DynWinRTType)
assert unpack_url_value(pack_url_value(Canonical(17)).to_value()) == Canonical(17)
assert typing.get_type_hints(pack_url_value)["v"] is Canonical
assert typing.get_type_hints(unpack_url_value)["return"] is Canonical
foreign = {{"{pack_kind_alias}": PackKind, "{type_kind_alias}": TypeKind}}
assert typing.get_type_hints(IUse.echo)["return"] is URLValue
assert typing.get_type_hints(IUse.echo_pack_kind, localns=foreign)["return"] is PackKind
assert typing.get_type_hints(IUse.echo_type_kind, localns=foreign)["return"] is TypeKind
class Handler:
    def echo(self, value): return value
    def echo_pack_kind(self, value): return value
    def echo_type_kind(self, value): return value
with dw.RoApartment(1), IUse.implement(Handler()) as implementation:
    assert implementation.value.echo(URLValue(17)) == URLValue(17)
    assert implementation.value.echo_pack_kind(PackKind.Unknown) is PackKind.Unknown
    assert implementation.value.echo_type_kind(TypeKind.Unknown) is TypeKind.Unknown
"#
            ),
        );
    }
}

#[test]
fn python_cross_role_class_owners_keep_self_bindings_and_qualified_helpers() {
    for packaged in [true, false] {
        for (index, name) in ["pack_url_value", "URLValue_TYPE"].into_iter().enumerate() {
            let fixture = Fixture::new();
            let package = fixture.package();
            let (_, types) =
                cross_role_metadata(&fixture.0.join("metadata").join("Input.winmd"), false);
            let owner = class(
                "Owners",
                name,
                vec![
                    echo("EchoSelf", class_type("Owners", name), 6),
                    echo("EchoPayload", types[0].clone(), 7),
                    MethodMeta {
                        name: "GetSelf".into(),
                        raw_name: "GetSelf".into(),
                        vtable_index: 8,
                        return_type: Some(class_type("Owners", name)),
                        ..Default::default()
                    },
                ],
                0x51931410 + index as u32,
            );
            let identities = [
                class_identity(&owner),
                types[0].type_identity(),
                owner.default_interface.as_ref().unwrap().type_identity(),
            ];
            let baseline =
                python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
            let context = python::PythonProjectionContext::new(
                identities
                    .into_iter()
                    .chain([enumeration("Unrelated", "unpack_url_value").type_identity()]),
                packaged,
            )
            .unwrap();
            let owner_module = context.implementation_module(&class_identity(&owner));
            let struct_module = context.implementation_module_for_type(&types[0]);
            let source = python::generate_class(&context, &owner, &Default::default());
            let stub = python_stub::generate_class_stub(&context, &owner, &Default::default());
            assert_eq!(
                source,
                python::generate_class(&baseline, &owner, &Default::default())
            );
            assert_eq!(
                stub,
                python_stub::generate_class_stub(&baseline, &owner, &Default::default())
            );
            assert!(source.contains(&format!("class {name}:")), "{source}");
            module(&package, &owner_module, source, stub);
            write_structs(&package, &context, &types[..1]);
            support(&package, &[]);
            let payload_module = if packaged {
                &struct_module
            } else {
                &owner_module
            };
            let imports = format!(
                "from pyviews.{owner_module} import {name} as Owner\n\
                 from pyviews.{payload_module} import URLValue\n",
            );
            typecheck(
                &fixture.0,
                &format!(
                    r#"{imports}
from typing import assert_type
def check(value: Owner) -> None:
    assert_type(value.echo_self(value), Owner | None)
    assert_type(value.get_self(), Owner | None)
    assert_type(value.echo_payload(URLValue(17)), URLValue)
"#
                ),
            );
            reject_consumer(
                &fixture.0,
                &format!(
                    r#"{imports}
def reject(value: Owner, payload: URLValue) -> None:
    value.echo_self(payload)
    value.echo_payload(value)
"#
                ),
                &["[arg-type]", "[arg-type]"],
            );
            runtime(
                &fixture.0,
                &format!(
                    r#"{imports}
import typing
from pyviews.{struct_module} import URLValue as Canonical, pack_url_value, unpack_url_value
assert isinstance(Owner, type)
assert Owner.__name__ == "{name}"
assert typing.get_type_hints(Owner.get_self)["return"] == Owner | None
assert typing.get_type_hints(Owner.echo_payload)["return"] is URLValue
assert typing.get_type_hints(pack_url_value)["v"] is Canonical
assert typing.get_type_hints(unpack_url_value)["return"] is Canonical
assert unpack_url_value(pack_url_value(Canonical(17)).to_value()) == Canonical(17)
"#
                ),
            );
        }
    }
}

#[test]
fn python_cross_role_owning_struct_preserves_public_helpers_and_aliases_field_enums() {
    for packaged in [true, false] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let (interface, types) =
            cross_role_metadata(&fixture.0.join("metadata").join("Input.winmd"), true);
        let identities = types
            .iter()
            .map(TypeMeta::type_identity)
            .chain([interface.type_identity()])
            .collect::<Vec<_>>();
        let baseline = python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
        let context = python::PythonProjectionContext::new(
            identities
                .into_iter()
                .chain([enumeration("Unrelated", "unpack_url_value").type_identity()]),
            packaged,
        )
        .unwrap();
        let struct_module = context.implementation_module_for_type(&types[0]);
        let pack_kind_module = context.implementation_module_for_type(&types[1]);
        let type_kind_module = context.implementation_module_for_type(&types[2]);
        let source = python::generate_struct(&context, &types[0]).unwrap();
        let stub = python_stub::generate_struct_stub(&context, &types[0]).unwrap();
        assert_eq!(
            source,
            python::generate_struct(&baseline, &types[0]).unwrap()
        );
        assert_eq!(
            stub,
            python_stub::generate_struct_stub(&baseline, &types[0]).unwrap()
        );
        for text in [&source, &stub] {
            assert!(
                text.contains("def pack_url_value(") && text.contains("def unpack_url_value("),
                "{text}"
            );
            assert_ne!(
                imported_symbol(text, &pack_kind_module, "pack_url_value"),
                "pack_url_value",
                "the owning struct's public pack helper must win over a foreign field type"
            );
            assert_ne!(
                imported_symbol(text, &type_kind_module, "URLValue_TYPE"),
                "URLValue_TYPE",
                "the owning struct's public type constant must win over a foreign field type"
            );
        }
        let pack_kind_alias = imported_symbol(&source, &pack_kind_module, "pack_url_value");
        let type_kind_alias = imported_symbol(&source, &type_kind_module, "URLValue_TYPE");
        cross_role_modules(&package, &context, &types);
        support(&package, &[]);
        let imports = format!(
            "from pyviews.{struct_module} import URLValue, pack_url_value, unpack_url_value, URLValue_TYPE\n\
             from pyviews.{pack_kind_module} import pack_url_value as PackKind\n\
             from pyviews.{type_kind_module} import URLValue_TYPE as TypeKind\n",
        );
        typecheck(
            &fixture.0,
            &format!(
                r#"{imports}
from typing import assert_type
from dynwinrt import DynWinRTType
assert_type(URLValue_TYPE, DynWinRTType)
assert_type(URLValue().pack_kind, PackKind)
assert_type(URLValue().type_kind, TypeKind)
assert_type(unpack_url_value(pack_url_value(URLValue()).to_value()), URLValue)
"#
            ),
        );
        reject_consumer(
            &fixture.0,
            &format!(
                r#"{imports}
def reject(value: URLValue) -> None:
    pack_url_value(PackKind.Unknown)
    value.pack_kind = TypeKind.Unknown
    value.type_kind = PackKind.Unknown
"#
            ),
            &["[arg-type]", "[assignment]", "[assignment]"],
        );
        runtime(
            &fixture.0,
            &format!(
                r#"{imports}
import typing
import dynwinrt as dw
foreign = {{"{pack_kind_alias}": PackKind, "{type_kind_alias}": TypeKind}}
hints = typing.get_type_hints(URLValue.__init__, localns=foreign)
assert hints["pack_kind"] is PackKind
assert hints["type_kind"] is TypeKind
assert typing.get_type_hints(pack_url_value)["v"] is URLValue
assert typing.get_type_hints(unpack_url_value)["return"] is URLValue
assert isinstance(URLValue_TYPE, dw.DynWinRTType)
default = URLValue()
assert type(default.pack_kind) is PackKind
assert type(default.type_kind) is TypeKind
value = URLValue(17, PackKind.Unknown, TypeKind.Unknown)
result = unpack_url_value(pack_url_value(value).to_value())
assert result == value
assert type(result.pack_kind) is PackKind
assert type(result.type_kind) is TypeKind
"#
            ),
        );
    }
}

#[test]
fn python_normalized_struct_helpers_round_trip_nested_fields_arrays_and_delegates() {
    for packaged in [false, true] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let winmd = fixture.0.join("metadata").join("Input.winmd");
        metadata(&winmd);
        let interface = helper_interface(&winmd);
        let structures = structures(&interface);
        let callback = interface
            .implementation_metadata
            .delegates
            .iter()
            .find(|delegate| delegate.typ.type_identity().definition_name() == Some("PairCallback"))
            .expect("metadata must retain the callback contract");
        let context = python::PythonProjectionContext::new(
            structures.iter().map(TypeMeta::type_identity).chain([
                interface.type_identity(),
                callback
                    .typ
                    .type_identity()
                    .with_kind(TypeIdentityKind::Delegate),
            ]),
            packaged,
        )
        .unwrap();
        let callback_meta = InterfaceMeta {
            namespace: "Audit".into(),
            name: "PairCallback".into(),
            iid: match &callback.typ {
                TypeMeta::Delegate { iid, .. } | TypeMeta::Interface { iid, .. } => iid.clone(),
                _ => unreachable!(),
            },
            methods: vec![
                MethodMeta {
                    name: ".ctor".into(),
                    ..Default::default()
                },
                callback.invoke.clone(),
            ],
            ..Default::default()
        };
        for iface in [&interface, &callback_meta] {
            module(
                &package,
                &context.implementation_module_for_interface(iface),
                python::generate_interface(&context, iface),
                python_stub::generate_interface_stub(&context, iface),
            );
        }
        write_structs(&package, &context, &structures);
        let owner_module = context.implementation_module_for_interface(&interface);
        let source = fs::read_to_string(package.join(format!("{owner_module}.py"))).unwrap();
        if packaged {
            for declaration in ["_pack_url_value", "_unpack_url_value"] {
                let alpha = imported_symbol(
                    &source,
                    &context.implementation_module_for_type(&structures[0]),
                    declaration,
                );
                let beta = imported_symbol(
                    &source,
                    &context.implementation_module_for_type(&structures[1]),
                    declaration,
                );
                assert_ne!(
                    alpha, beta,
                    "different canonical structs must not bind the same helper"
                );
            }
        } else {
            let functions = source
                .lines()
                .filter_map(|line| line.strip_prefix("def "))
                .filter_map(|line| line.split_once('(').map(|(name, _)| name))
                .collect::<Vec<_>>();
            assert_eq!(
                functions.len(),
                functions.iter().collect::<BTreeSet<_>>().len(),
                "inline normalized helpers must not overwrite each other:\n{source}"
            );
        }
        support(
            &package,
            if packaged {
                std::slice::from_ref(&owner_module)
            } else {
                &[]
            },
        );
        let helper_records = python::implementation_helper_records(&context, &interface);
        let helper_name = |key| {
            helper_records
                .iter()
                .find(|record| record.key == key)
                .unwrap()
                .name
                .clone()
        };
        let alpha_module = if packaged {
            context.implementation_module_for_type(&structures[0])
        } else {
            owner_module.clone()
        };
        let beta_module = if packaged {
            context.implementation_module_for_type(&structures[1])
        } else {
            owner_module.clone()
        };
        let pair_module = if packaged {
            context.implementation_module_for_type(&structures[2])
        } else {
            owner_module.clone()
        };
        let imports = format!(
            "from pyviews.{owner_module} import IHelpers, {handlers} as Handlers, {callback} as Callback, {split} as SplitResult\n\
             from pyviews.{alpha_module} import URLValue\n\
             from pyviews.{beta_module} import UrlValue\n\
             from pyviews.{pair_module} import NormalizedPair\n",
            handlers = helper_name("handlers"),
            callback = helper_name("delegate:0"),
            split = helper_name(&format!(
                "method-result:{}",
                interface
                    .methods
                    .iter()
                    .find(|method| method.name == "Split")
                    .unwrap()
                    .vtable_index
            )),
        );
        typecheck(
            &fixture.0,
            &format!(
                r#"{imports}
from typing import assert_type
class Handler:
    def echo_alpha(self, value: URLValue) -> URLValue: return value
    def echo_beta(self, value: UrlValue) -> UrlValue: return value
    def echo_pair(self, value: NormalizedPair) -> NormalizedPair: return value
    def echo_array(self, value: list[URLValue]) -> list[URLValue]: return value
    def split(self) -> SplitResult: return {{"second": UrlValue(2**40), "result": URLValue(17)}}
    def use_callback(self, callback: Callback | None) -> NormalizedPair:
        assert callback is not None
        callback(URLValue(17), UrlValue(2**40))
        return NormalizedPair(URLValue(17), UrlValue(2**40))
handlers: Handlers = Handler()
assert_type(NormalizedPair().first, URLValue)
assert_type(NormalizedPair().second, UrlValue)
def check(value: IHelpers) -> None:
    assert_type(value.echo_alpha(URLValue()), URLValue)
    assert_type(value.echo_beta(UrlValue()), UrlValue)
    assert_type(value.echo_array([URLValue()]), list[URLValue])
    assert_type(value.echo_pair(NormalizedPair()).second, UrlValue)
"#
            ),
        );
        reject_consumer(
            &fixture.0,
            &format!(
                "{imports}\ndef reject_wrong_struct(value: IHelpers) -> None:\n    value.echo_alpha(UrlValue())\n"
            ),
            &["[arg-type]"],
        );
        runtime(
            &fixture.0,
            &format!(
                r#"{imports}
import dynwinrt as dw
from pyviews.{alpha_standalone} import URLValue as Alpha, pack_url_value as pack_alpha, unpack_url_value as unpack_alpha
from pyviews.{beta_standalone} import UrlValue as Beta, pack_url_value as pack_beta, unpack_url_value as unpack_beta
from pyviews.{pair_standalone} import NormalizedPair as Pair, pack_normalized_pair, unpack_normalized_pair
assert unpack_alpha(pack_alpha(Alpha(17)).to_value()) == Alpha(17)
assert unpack_beta(pack_beta(Beta(2**40)).to_value()) == Beta(2**40)
pair = Pair(Alpha(17), Beta(2**40))
assert unpack_normalized_pair(pack_normalized_pair(pair).to_value()) == pair
assert type(Pair().first) is Alpha
assert type(Pair().second) is Beta
assert type(NormalizedPair().first) is URLValue
assert type(NormalizedPair().second) is UrlValue
class Handler:
    def echo_alpha(self, value): return value
    def echo_beta(self, value): return value
    def echo_pair(self, value): return value
    def echo_array(self, value): return value
    def split(self): return {{"second": UrlValue(2**40), "result": URLValue(17)}}
    def use_callback(self, callback):
        assert callback is not None
        callback(URLValue(17), UrlValue(2**40))
        return NormalizedPair(URLValue(17), UrlValue(2**40))
with dw.RoApartment(1), IHelpers.implement(Handler()) as implementation:
    view = implementation.value
    assert view.echo_alpha(URLValue(17)) == URLValue(17)
    assert view.echo_beta(UrlValue(2**40)) == UrlValue(2**40)
    assert view.echo_pair(NormalizedPair(URLValue(17), UrlValue(2**40))) == NormalizedPair(URLValue(17), UrlValue(2**40))
    assert view.echo_array([URLValue(17), URLValue(-3)]) == [URLValue(17), URLValue(-3)]
    result = view.split()
    assert result == (UrlValue(2**40), URLValue(17)), repr(result)
    # Forward delegates receive native values; use the public struct projections.
    seen = []
    callback_result = view.use_callback(lambda first, second: seen.append((unpack_alpha(first).value, unpack_beta(second).other)))
    assert seen == [(17, 2**40)], seen
    assert callback_result == NormalizedPair(URLValue(17), UrlValue(2**40)), repr(callback_result)
"#,
                alpha_standalone = context.implementation_module_for_type(&structures[0]),
                beta_standalone = context.implementation_module_for_type(&structures[1]),
                pair_standalone = context.implementation_module_for_type(&structures[2]),
            ),
        );
        assert!(
            source.contains("Alpha.URLValue") && source.contains("Beta.UrlValue"),
            "{source}"
        );
    }
}

#[test]
fn python_enum_closed_generic_collision_preserves_projection_and_native_identity() {
    for packaged in [false, true] {
        let fixture = Fixture::new();
        let package = fixture.package();
        let winmd = fixture.0.join("metadata").join("Input.winmd");
        metadata(&winmd);
        let enumeration = meta::parse_enums(winmd.to_str().unwrap(), "Enums")
            .pop()
            .unwrap();
        let local = meta::parse_interfaces(winmd.to_str().unwrap(), "Enums")
            .into_iter()
            .find(|interface| interface.name == "IUse")
            .unwrap();
        let foreign = meta::parse_interfaces(winmd.to_str().unwrap(), "Consumers")
            .pop()
            .unwrap();
        let generic_type = local.methods[0].return_type.clone().unwrap();
        let TypeMeta::Parameterized {
            piid, args, name, ..
        } = &generic_type
        else {
            panic!("closed generic return")
        };
        let generic = InterfaceMeta {
            namespace: "Enums".into(),
            name: "IBucket_String".into(),
            generic_name: Some(name.clone()),
            generic_piid: Some(piid.clone()),
            generic_args: args.clone(),
            methods: vec![MethodMeta {
                name: "Ping".into(),
                raw_name: "Ping".into(),
                vtable_index: 6,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_ne!(enumeration.type_identity(), generic.type_identity());
        let context = python::PythonProjectionContext::new(
            [
                enumeration.type_identity(),
                local.type_identity(),
                foreign.type_identity(),
                generic.type_identity(),
            ],
            packaged,
        )
        .unwrap();
        let enum_name = context.projected_name_for_type(&enumeration);
        let generic_name = context.projected_name_for_interface(&generic);
        assert_ne!(
            enum_name, "IBucket_String",
            "named enum must yield to the closed generic declaration"
        );
        assert_ne!(enum_name, generic_name);
        let enum_module = context.implementation_module_for_type(&enumeration);
        for source in [
            python::generate_enum(&context, &enumeration).unwrap(),
            python_stub::generate_enum_stub(&context, &enumeration).unwrap(),
        ] {
            assert!(
                source.contains(&format!("class {enum_name}(IntEnum):")),
                "{source}"
            );
        }
        module(
            &package,
            &enum_module,
            python::generate_enum(&context, &enumeration).unwrap(),
            python_stub::generate_enum_stub(&context, &enumeration).unwrap(),
        );
        for interface in [&local, &foreign, &generic] {
            let source = python::generate_interface(&context, interface);
            let stub = python_stub::generate_interface_stub(&context, interface);
            if interface.generic_piid.is_none() {
                for text in [&source, &stub] {
                    assert!(
                        text.contains(&format!("from .{enum_module} import {enum_name}")),
                        "{text}"
                    );
                }
                assert!(
                    source.contains("DynWinRTType.enum_type('Enums.IBucket_String'"),
                    "{source}"
                );
                assert!(
                    !source.contains(&format!("DynWinRTType.enum_type('Enums.{enum_name}'")),
                    "{source}"
                );
                assert!(
                    !source.contains(&format!("'{enum_module}', 'IBucket_String'")),
                    "runtime lookups must use the projected enum declaration, including arrays:\n{source}",
                );
            }
            module(
                &package,
                &context.implementation_module_for_interface(interface),
                source,
                stub,
            );
        }
        let owners = [&local, &foreign]
            .map(|interface| context.implementation_module_for_interface(interface));
        support(&package, if packaged { &owners } else { &[] });
        let interfaces = [local.clone(), foreign.clone(), generic.clone()];
        let reversed = interfaces.iter().rev().cloned().collect::<Vec<_>>();
        let enums = std::slice::from_ref(&enumeration);
        let index = python::generate_index(&context, &[], &interfaces, enums);
        let index_stub = python_stub::generate_index_stub(&context, &[], &interfaces, enums);
        assert_eq!(
            index,
            python::generate_index(&context, &[], &reversed, enums)
        );
        assert_eq!(
            index_stub,
            python_stub::generate_index_stub(&context, &[], &reversed, enums)
        );
        for text in [&index, &index_stub] {
            assert_eq!(imported_symbol(text, &enum_module, &enum_name), enum_name);
            assert_eq!(
                imported_symbol(
                    text,
                    &context.implementation_module_for_interface(&generic),
                    &generic_name
                ),
                generic_name,
                "the named enum must not suppress its same-spelling closed generic in a direct index",
            );
        }
        for (text, reordered) in [
            (
                python::generate_public_index(&context, &[], &interfaces, enums),
                python::generate_public_index(&context, &[], &reversed, enums),
            ),
            (
                python_stub::generate_public_index_stub(&context, &[], &interfaces, enums),
                python_stub::generate_public_index_stub(&context, &[], &reversed, enums),
            ),
        ] {
            assert_eq!(text, reordered);
            for (identity, name) in [
                (enumeration.type_identity(), &enum_name),
                (generic.type_identity(), &generic_name),
            ] {
                assert_eq!(
                    imported_symbol(&text, &context.public_qualified_module(&identity), name),
                    *name,
                    "public indexes must retain both canonical identities",
                );
            }
        }
        module(&package, "__init__", index, index_stub);
        let imports = format!(
            "from pyviews.{enum_module} import {enum_name} as Kind\n\
             from pyviews.{generic_module} import {generic_name} as Bucket\n\
             from pyviews.{local_module} import IUse\n\
             from pyviews.{foreign_module} import IForeign\n\
             from pyviews import {enum_name} as RootKind, {generic_name} as RootBucket\n",
            generic_module = context.implementation_module_for_interface(&generic),
            local_module = owners[0],
            foreign_module = owners[1],
        );
        typecheck(
            &fixture.0,
            &format!(
                r#"{imports}
from typing import assert_type
assert_type(RootKind(0), Kind)
def check(local: IUse, foreign: IForeign) -> None:
    assert_type(local.get_bucket(), Bucket | None)
    assert_type(local.get_bucket(), RootBucket | None)
    assert_type(foreign.get_bucket(), Bucket | None)
    assert_type(local.echo_kind(Kind.Unknown), Kind)
    assert_type(foreign.echo_kind(Kind.Unknown), Kind)
    assert_type(local.echo_kinds([Kind.Unknown]), list[Kind])
"#
            ),
        );
        reject_consumer(
            &fixture.0,
            &format!(
                "{imports}\ndef reject_wrong_enum(value: IUse) -> None:\n    value.echo_kind(0)\n"
            ),
            &["[arg-type]"],
        );
        runtime(
            &fixture.0,
            &format!(
                r#"{imports}
import dynwinrt as dw
from enum import IntEnum
assert RootKind is Kind
assert RootBucket is Bucket
assert issubclass(Kind, IntEnum)
assert not issubclass(Bucket, IntEnum)
class Handler:
    def get_bucket(self): return None
    def echo_kind(self, value):
        assert type(value) is Kind
        return value
    def echo_kinds(self, value): return value
with dw.RoApartment(1):
    for projection in (IUse, IForeign):
        with projection.implement(Handler()) as implementation:
            assert implementation.value.echo_kind(Kind.Unknown) is Kind.Unknown
            assert implementation.value.get_bucket() is None
    with IUse.implement(Handler()) as implementation:
        kinds = implementation.value.echo_kinds([Kind.Unknown])
        assert kinds == [Kind.Unknown]
        assert all(type(value) is Kind for value in kinds)
"#
            ),
        );
    }
}

fn generate(winmd: &Path, package: &Path, selected: &str) {
    success(
        Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args(["generate", "--winmd"])
            .arg(winmd)
            .args(["--output"])
            .arg(package)
            .args(["--lang", "py", "--class-name", selected])
            .output()
            .unwrap(),
    );
}

fn python_files(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("py" | "pyi")
            ) {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(directory, directory, &mut files);
    files
}

#[test]
fn python_class_companion_aliases_preserve_cli_and_standalone_contracts() {
    for peer in ["WidgetLike", "WidgetPeer"] {
        for packaged in [true, false] {
            let fixture = Fixture::new();
            let winmd = fixture.0.join("metadata").join("Input.winmd");
            class_companion_metadata(&winmd, peer);
            let package = fixture.package();
            let classes = meta::parse_namespace(winmd.to_str().unwrap(), "Audit");
            let interfaces = meta::parse_interfaces(winmd.to_str().unwrap(), "Audit");
            assert_eq!(classes.len(), 2);
            assert!(
                classes
                    .iter()
                    .all(|class| class.default_interface.is_some())
            );
            let identities = classes
                .iter()
                .map(class_identity)
                .chain(interfaces.iter().map(InterfaceMeta::type_identity))
                .collect::<Vec<_>>();
            let context =
                python::PythonProjectionContext::new(identities.clone(), packaged).unwrap();
            let reordered = python::PythonProjectionContext::new(
                identities.into_iter().rev().chain([
                    class_type("Unrelated", "WidgetLikeLike").type_identity(),
                    class_type("Unrelated", "_WidgetIdentity").type_identity(),
                    class_type("Unrelated", "Audit_WidgetLike_class").type_identity(),
                ]),
                packaged,
            )
            .unwrap();
            if packaged {
                generate(
                    &winmd,
                    &package,
                    &format!("Audit.Widget,Audit.{peer},Audit.IWidget,Audit.I{peer},Audit.IUse"),
                );
                let reversed = fixture.0.join("reversed").join("pyviews");
                generate(
                    &winmd,
                    &reversed,
                    &format!("Audit.IUse,Audit.I{peer},Audit.IWidget,Audit.{peer},Audit.Widget"),
                );
                assert_eq!(python_files(&package), python_files(&reversed));
            }
            for class in &classes {
                let source = python::generate_class(&context, class, &Default::default());
                let stub = python_stub::generate_class_stub(&context, class, &Default::default());
                assert_eq!(
                    source,
                    python::generate_class(&reordered, class, &Default::default())
                );
                assert_eq!(
                    stub,
                    python_stub::generate_class_stub(&reordered, class, &Default::default())
                );
                if !packaged {
                    module(
                        &package,
                        &context.implementation_module(&class_identity(class)),
                        source,
                        stub,
                    );
                }
            }
            for interface in &interfaces {
                let source = python::generate_interface(&context, interface);
                let stub = python_stub::generate_interface_stub(&context, interface);
                assert_eq!(source, python::generate_interface(&reordered, interface));
                assert_eq!(
                    stub,
                    python_stub::generate_interface_stub(&reordered, interface)
                );
                if !packaged {
                    module(
                        &package,
                        &context.implementation_module_for_interface(interface),
                        source,
                        stub,
                    );
                }
            }
            if !packaged {
                support(&package, &[]);
            }
            let owner_module =
                context.implementation_module_for_type(&class_type("Audit", "Widget"));
            let peer_module = context.implementation_module_for_type(&class_type("Audit", peer));
            let owner_source =
                fs::read_to_string(package.join(format!("{owner_module}.py"))).unwrap();
            let owner_stub =
                fs::read_to_string(package.join(format!("{owner_module}.pyi"))).unwrap();
            let peer_stub = fs::read_to_string(package.join(format!("{peer_module}.pyi"))).unwrap();
            let peer_alias = imported_symbol(&owner_source, &peer_module, peer);
            assert_eq!(peer_alias, imported_symbol(&owner_stub, &peer_module, peer));
            assert_eq!(peer_alias == peer, peer == "WidgetPeer");
            assert_eq!(
                imported_symbol(&owner_stub, &peer_module, &format!("{peer}Like")),
                format!("{peer}Like"),
                "the foreign Like role must not follow its renamed Type role"
            );
            assert_eq!(
                imported_symbol(&peer_stub, &owner_module, "Widget"),
                "Widget"
            );
            assert_eq!(
                imported_symbol(&peer_stub, &owner_module, "WidgetLike") == "WidgetLike",
                peer == "WidgetPeer",
                "the imported Like must not shadow its consuming class"
            );
            assert!(owner_stub.contains("class WidgetLike(_WidgetIdentity, Protocol):"));
            assert!(owner_stub.contains("class Widget("));
            let imports = format!(
                "from pyviews.{owner_module} import Widget\n\
                 from pyviews.{peer_module} import {peer} as Peer\n\
                 from pyviews.audit__i_use import IUse\n",
            );
            strict_typecheck(
                &fixture.0,
                &format!(
                    r#"{imports}
from typing import assert_type
def check(owner: Widget, peer: Peer, use: IUse) -> None:
    assert_type(owner.echo(peer), Peer | None)
    assert_type(peer.echo(owner), Widget | None)
    assert_type(use.echo_owner(owner), Widget | None)
    assert_type(use.echo_peer(peer), Peer | None)
"#,
                ),
            );
            reject_consumer(
                &fixture.0,
                &format!(
                    r#"{imports}
def reject(owner: Widget, peer: Peer, use: IUse) -> None:
    owner.echo(owner)
    peer.echo(peer)
    use.echo_owner(peer)
    use.echo_peer(owner)
    result = owner.echo(peer)
    if result is not None:
        result.no_such_member()
"#,
                ),
                &[
                    "[arg-type]",
                    "[arg-type]",
                    "[arg-type]",
                    "[arg-type]",
                    "[attr-defined]",
                ],
            );
            let public_imports = if packaged {
                format!(
                    "from pyviews import Widget as RootOwner, {peer} as RootPeer\n\
                     from pyviews.audit.widget import Widget as FacadeOwner\n\
                     from pyviews.audit.{} import {peer} as FacadePeer\n\
                     assert RootOwner is FacadeOwner is Widget\n\
                     assert RootPeer is FacadePeer is Peer\n",
                    python::to_snake_case_filename(peer),
                )
            } else {
                String::new()
            };
            runtime(
                &fixture.0,
                &format!(
                    r#"{imports}
{public_imports}
import typing
import dynwinrt as dw
from pyviews.audit__i_widget import IWidget
from pyviews.audit__i_{peer_file} import I{peer} as IPeer
assert Widget.__name__ == "Widget"
assert Peer.__name__ == "{peer}"
hints = typing.get_type_hints(Widget.echo, localns={{
    "{peer_alias}": Peer, "{peer}Like": Peer,
}})
assert hints["return"] == Peer | None
assert hints["value"] is Peer
class Handler:
    def echo(self, value):
        return value
with dw.RoApartment(1):
    with IWidget.implement(Handler()) as owner_impl, IPeer.implement(Handler()) as peer_impl:
        owner = Widget(owner_impl.value._obj)
        peer = Peer(peer_impl.value._obj)
        try:
            echoed_peer = owner.echo(peer)
            echoed_owner = peer.echo(owner)
            assert type(echoed_peer) is Peer
            assert type(echoed_owner) is Widget
            dw.release_projected(echoed_peer)
            dw.release_projected(echoed_owner)
        finally:
            dw.release_projected(owner)
            dw.release_projected(peer)
"#,
                    peer_file = python::to_snake_case_filename(peer),
                ),
            );
        }
    }
}

#[test]
fn python_companion_aliases_cover_inherited_identities_and_required_views() {
    for packaged in [false, true] {
        for shared in [false, true] {
            let fixture = Fixture::new();
            let package = fixture.package();
            let kind = enumeration("Kinds", "_Base_Widget_classIdentity");
            let base = class("Base", "Widget", vec![], 0x51931600);
            let mut owner = class(
                "Audit",
                "Widget",
                vec![echo("EchoKind", kind.clone(), 6)],
                0x51931601,
            );
            owner.base_class = Some(TypeRef {
                namespace: base.namespace.clone(),
                name: base.name.clone(),
                kind: TypeKind::Class,
            });
            let required = InterfaceMeta {
                namespace: "Views".into(),
                name: "WidgetLike".into(),
                iid: "51931602-6281-4900-b782-040302010910".into(),
                methods: vec![echo("EchoNumber", TypeMeta::I32, 6)],
                ..Default::default()
            };
            owner.required_interfaces.push(required.clone());
            let context = python::PythonProjectionContext::new(
                [
                    class_identity(&base),
                    class_identity(&owner),
                    kind.type_identity(),
                    required.type_identity(),
                ],
                packaged,
            )
            .unwrap();
            let shared_iids = if shared {
                HashSet::from([required.iid.clone()])
            } else {
                HashSet::new()
            };
            let owner_module = context.implementation_module(&class_identity(&owner));
            let base_module = context.implementation_module(&class_identity(&base));
            let required_module = context.implementation_module_for_interface(&required);
            let kind_module = context.implementation_module_for_type(&kind);
            let source = python::generate_class(&context, &owner, &shared_iids);
            let stub = python_stub::generate_class_stub(&context, &owner, &shared_iids);
            let base_identity = imported_symbol(&stub, &base_module, "_WidgetIdentity");
            assert_ne!(base_identity, "_Base_Widget_classIdentity");
            assert!(
                stub.contains(&format!(
                    "class _WidgetIdentity({base_identity}, Protocol):"
                )),
                "{stub}"
            );
            let view = "Views_WidgetLike_interface";
            for text in [&source, &stub] {
                assert!(text.contains(view), "{text}");
                if shared {
                    assert_eq!(imported_symbol(text, &required_module, "WidgetLike"), view);
                } else {
                    assert!(text.contains(&format!("class {view}:")), "{text}");
                }
                assert_ne!(
                    imported_symbol(text, &kind_module, "_Base_Widget_classIdentity"),
                    "_Base_Widget_classIdentity"
                );
            }
            assert!(stub.contains("class WidgetLike(_WidgetIdentity, Protocol):"));
            module(&package, &owner_module, source, stub);
            module(
                &package,
                &base_module,
                python::generate_class(&context, &base, &Default::default()),
                python_stub::generate_class_stub(&context, &base, &Default::default()),
            );
            module(
                &package,
                &required_module,
                python::generate_interface(&context, &required),
                python_stub::generate_interface_stub(&context, &required),
            );
            module(
                &package,
                &kind_module,
                python::generate_enum(&context, &kind).unwrap(),
                python_stub::generate_enum_stub(&context, &kind).unwrap(),
            );
            support(&package, &[]);
            let imports = format!(
                "from pyviews.{owner_module} import Widget, WidgetLike\n\
                 from pyviews.{base_module} import Widget as Base, WidgetLike as BaseLike\n\
                 from pyviews.{kind_module} import _Base_Widget_classIdentity as Kind\n",
            );
            strict_typecheck(
                &fixture.0,
                &format!(
                    r#"{imports}
from typing import assert_type
def check(child: Widget) -> None:
    inherited: BaseLike = child
    own: WidgetLike = child
    assert_type(child.echo_kind(Kind.Unknown), Kind)
    assert_type(child.echo_number(17), int)
"#,
                ),
            );
            reject_consumer(
                &fixture.0,
                &format!(
                    r#"{imports}
def reject(child: Widget, base: Base) -> None:
    wrong: WidgetLike = base
    child.echo_kind(0)
    child.echo_number(Kind.Unknown.name)
"#,
                ),
                &["[assignment]", "[arg-type]", "[arg-type]"],
            );
            if !shared {
                runtime(
                    &fixture.0,
                    &format!(
                        r#"
from pyviews.{owner_module} import Widget, {view}
from dynwinrt import WinGUID
assert Widget.__name__ == "Widget"
assert {view}.__name__ == "{view}"
assert Widget is not {view}
assert {view}._dynwinrt_interface_iid.to_string() == WinGUID.parse("{iid}").to_string()
"#,
                        iid = required.iid,
                    ),
                );
            }
        }
    }
}

#[test]
fn python_symbol_collisions_incremental_package_matches_clean_generation() {
    let fixture = Fixture::new();
    let winmd = fixture.0.join("metadata").join("Input.winmd");
    metadata(&winmd);
    let incremental = fixture.0.join("incremental").join("pyviews");
    let clean = fixture.0.join("clean").join("pyviews");
    generate(&winmd, &incremental, "Audit.IAlpha,Enums.IUse");
    let alpha_before = fs::read(incremental.join("alpha__url_value.py")).unwrap();
    generate(&winmd, &incremental, "Audit.IHelpers,Consumers.IForeign");
    generate(
        &winmd,
        &clean,
        "Audit.IAlpha,Audit.IHelpers,Enums.IUse,Consumers.IForeign",
    );
    assert_eq!(
        alpha_before,
        fs::read(incremental.join("alpha__url_value.py")).unwrap()
    );
    let incremental_files = python_files(&incremental);
    let clean_files = python_files(&clean);
    assert_eq!(
        incremental_files.keys().collect::<BTreeSet<_>>(),
        clean_files.keys().collect::<BTreeSet<_>>()
    );
    for (name, expected) in clean_files {
        assert_eq!(
            incremental_files[&name],
            expected,
            "incremental mismatch: {}",
            name.display()
        );
    }
    let consumer = r#"
from typing import assert_type
from pyviews.alpha.url_value import URLValue, pack_url_value, unpack_url_value
from pyviews.beta.url_value import UrlValue
from pyviews.containers.normalized_pair import NormalizedPair
from pyviews.audit.i_helpers import IHelpers
assert_type(unpack_url_value(pack_url_value(URLValue(17)).to_value()), URLValue)
assert_type(NormalizedPair().first, URLValue)
assert_type(NormalizedPair().second, UrlValue)
def check(value: IHelpers) -> None:
    assert_type(value.echo_alpha(URLValue()), URLValue)
    assert_type(value.echo_beta(UrlValue()), UrlValue)
"#;
    typecheck(incremental.parent().unwrap(), consumer);
    typecheck(clean.parent().unwrap(), consumer);
    runtime(
        incremental.parent().unwrap(),
        r#"
from pyviews.alpha.url_value import URLValue, pack_url_value as pack_alpha, unpack_url_value as unpack_alpha
from pyviews.beta.url_value import UrlValue, pack_url_value as pack_beta, unpack_url_value as unpack_beta
from pyviews.containers.normalized_pair import NormalizedPair, pack_normalized_pair, unpack_normalized_pair
assert unpack_alpha(pack_alpha(URLValue(17)).to_value()) == URLValue(17)
assert unpack_beta(pack_beta(UrlValue(2**40)).to_value()) == UrlValue(2**40)
pair = NormalizedPair(URLValue(17), UrlValue(2**40))
assert unpack_normalized_pair(pack_normalized_pair(pair).to_value()) == pair
"#,
    );
}
