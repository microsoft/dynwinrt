// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use dynwinrt_codegen::types::{FieldMeta, TypeMeta};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "python-released-implementation-{}-{}",
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

fn python() -> PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let venv = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
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

fn runtime_available() -> bool {
    let available = Command::new(python())
        .args([
            "-c",
            "import dynwinrt; assert hasattr(dynwinrt.DynWinRTValue, 'is_released')",
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_IMPLEMENTATION_RUNTIME").as_deref() != Ok("1"),
        "the released implementation probe requires the current Python binding"
    );
    if !available {
        eprintln!("Skipping the released implementation probe; set DYNWINRT_TEST_PYTHON.");
    }
    available
}

/// `Contoso.ISource` returns a struct with an Object field, and an Object
/// through an out parameter plus the result.
fn source() -> (InterfaceMeta, TypeMeta) {
    let holder = TypeMeta::Struct {
        namespace: "Contoso".into(),
        name: "Holder".into(),
        fields: vec![FieldMeta {
            name: "Item".into(),
            typ: TypeMeta::Object,
        }],
    };
    let interface = InterfaceMeta {
        namespace: "Contoso".into(),
        name: "ISource".into(),
        iid: "3d1f0b7e-5c2a-4e8b-9f6d-1a2b3c4d5e6f".into(),
        methods: vec![
            MethodMeta {
                name: "GetHolder".into(),
                vtable_index: 6,
                return_type: Some(holder.clone()),
                ..Default::default()
            },
            MethodMeta {
                name: "GetPair".into(),
                vtable_index: 7,
                params: vec![ParamMeta {
                    name: "first".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                }],
                return_type: Some(TypeMeta::Object),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (interface, holder)
}

#[test]
fn generated_implementation_results_reject_released_references() {
    let (interface, holder) = source();
    let context = python::PythonProjectionContext::packaged([
        interface.type_identity(),
        holder.type_identity(),
    ])
    .unwrap();
    let source_module = context.implementation_module_for_interface(&interface);
    let holder_module = context.implementation_module_for_type(&holder);
    let generated = python::generate_interface(&context, &interface);
    // Reference results, including struct fields, share the helper that keeps
    // a released value released instead of substituting a fresh null.
    assert!(
        generated.contains("s.set_object(0, _implementation_reference(value.item, "),
        "{generated}"
    );
    let runtime = python::generate_runtime_support_module();
    assert!(
        runtime.contains("    return raw if raw.is_null() else raw.cast(iid)\n"),
        "{runtime}"
    );
    assert!(
        !runtime.contains("null_value() if raw.is_null()"),
        "{runtime}"
    );
    if !runtime_available() {
        return;
    }

    let fixture = Fixture::new();
    let package = fixture.0.join("pyviews");
    fs::create_dir_all(&package).unwrap();
    for (name, source) in [
        ("__init__.py".to_string(), String::new()),
        ("_runtime.py".to_string(), runtime),
        (
            "_runtime.pyi".to_string(),
            python_stub::generate_runtime_support_stub(),
        ),
        (format!("{source_module}.py"), generated),
        (
            format!("{holder_module}.py"),
            python::generate_struct(&context, &holder).unwrap(),
        ),
    ] {
        fs::write(package.join(name), source).unwrap();
    }
    fs::write(
        fixture.0.join("probe.py"),
        format!(
            r#"
import importlib
import sys
import dynwinrt as dw

ISource = importlib.import_module("pyviews.{source_module}").ISource
Holder = importlib.import_module("pyviews.{holder_module}").Holder
PYTHON_EXCEPTION = -1594998779
RELEASED = (
    "has been released (its projected_lifetime_scope() exited, or release_projected() / "
    "DynWinRTValue.release() was called) and can no longer be used."
)
state = {{"item": None, "pair": (None, None)}}


class Handlers:
    def get_holder(self):
        return Holder(item=state["item"])

    def get_pair(self):
        first, result = state["pair"]
        return {{"first": first, "result": result}}


sys.unraisablehook = lambda _args: None
with dw.RoApartment(), dw.projected_lifetime_scope():
    live = dw.DynWinRTValue.activation_factory("Windows.Foundation.Uri")

    def released():
        value = live.cast(dw.WinGUID.parse("af86e2e0-b12d-4c6a-9c5a-d7aa65101e90"))
        value.release()
        return value

    with ISource.implement(Handlers()) as impl:
        view = impl.value
        # Real nulls and live values still cross every result path. An Object
        # struct field reads back as a raw value, so a null one is is_null().
        assert view.get_holder().item.is_null()
        state["item"] = live
        assert not view.get_holder().item.is_null()
        state["pair"] = (None, live)
        view.get_pair()
        state["pair"] = (live, None)
        view.get_pair()
        for call, update, slot in (
            (view.get_holder, {{"item": released()}}, "field 0 of DynWinRTStruct.set_object()"),
            (view.get_pair, {{"item": None, "pair": (released(), None)}}, "output 0 of implementation callback"),
            (view.get_pair, {{"pair": (None, released())}}, "output 1 of implementation callback"),
        ):
            state.update(update)
            try:
                call()
            except OSError as error:
                assert error.winerror == PYTHON_EXCEPTION, error
            else:
                raise AssertionError(f"{{slot}}: a released reference was returned")
            message = impl.take_error()
            assert f"This WinRT object ({{slot}}) {{RELEASED}}" in message, message
    live.release()
print("released-implementation-ok")
"#
        ),
    )
    .unwrap();
    let output = Command::new(python())
        .args(["-B", "probe.py"])
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("released-implementation-ok"),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
