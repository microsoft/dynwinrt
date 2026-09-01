#![allow(dead_code)]

use std::collections::HashSet;

use dynwinrt_codegen::codegen::python::{self, PythonProjectionContext};
use dynwinrt_codegen::codegen::python_stub;
use dynwinrt_codegen::meta::{ClassMeta, InterfaceMeta, MethodMeta};
use dynwinrt_codegen::types::{TypeIdentity, TypeIdentityKind, TypeMeta};

fn compatibility_name(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Interface { name, .. }
        | TypeMeta::Delegate { name, .. }
        | TypeMeta::Struct { name, .. }
        | TypeMeta::Enum { name, .. } => name.clone(),
        TypeMeta::Parameterized { name, args, .. } => {
            dynwinrt_codegen::meta::make_parameterized_name(name, args)
        }
        _ => String::new(),
    }
}

fn collect_type(
    typ: &TypeMeta,
    known: &HashSet<String>,
    delegates: &HashSet<String>,
    identities: &mut HashSet<TypeIdentity>,
) {
    let compatibility_name = compatibility_name(typ);
    let raw_identity = typ.type_identity();
    let full_name = raw_identity
        .namespace()
        .zip(raw_identity.definition_name())
        .map(|(namespace, name)| format!("{namespace}.{name}"));
    if known.contains(&compatibility_name)
        || full_name.as_ref().is_some_and(|name| known.contains(name))
        || delegates.contains(&compatibility_name)
        || matches!(typ, TypeMeta::Delegate { .. })
    {
        let identity = if delegates.contains(&compatibility_name) {
            raw_identity.with_kind(TypeIdentityKind::Delegate)
        } else {
            raw_identity
        };
        identities.insert(identity);
    }

    match typ {
        TypeMeta::Parameterized { args, .. } => {
            for argument in args {
                collect_type(argument, known, delegates, identities);
            }
        }
        TypeMeta::Array(inner)
        | TypeMeta::AsyncActionWithProgress(inner)
        | TypeMeta::AsyncOperation(inner) => collect_type(inner, known, delegates, identities),
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            collect_type(result, known, delegates, identities);
            collect_type(progress, known, delegates, identities);
        }
        TypeMeta::RuntimeClass {
            default_interface: Some(interface),
            ..
        } => collect_type(interface, known, delegates, identities),
        TypeMeta::Struct { fields, .. } => {
            for field in fields {
                collect_type(&field.typ, known, delegates, identities);
            }
        }
        _ => {}
    }
}

fn collect_methods(
    methods: &[MethodMeta],
    known: &HashSet<String>,
    delegates: &HashSet<String>,
    identities: &mut HashSet<TypeIdentity>,
) {
    for method in methods {
        for parameter in &method.params {
            collect_type(&parameter.typ, known, delegates, identities);
        }
        if let Some(return_type) = &method.return_type {
            collect_type(return_type, known, delegates, identities);
        }
    }
}

fn context(
    classes: &[&ClassMeta],
    interfaces: &[&InterfaceMeta],
    known: &HashSet<String>,
    delegates: &HashSet<String>,
    packaged: bool,
) -> PythonProjectionContext {
    let mut identities = HashSet::new();
    for full_name in known.iter().filter(|name| name.contains('.')) {
        if let Some((namespace, name)) = full_name.rsplit_once('.') {
            identities.insert(TypeIdentity::named(
                TypeIdentityKind::Class,
                namespace,
                name,
            ));
        }
    }
    for class in classes {
        identities.insert(TypeIdentity::named(
            TypeIdentityKind::Class,
            class.namespace.clone(),
            class.name.clone(),
        ));
        for interface in class
            .all_interfaces()
            .chain(class.overridable_interfaces.iter())
        {
            identities.insert(interface.type_identity());
            collect_methods(&interface.methods, known, delegates, &mut identities);
        }
    }
    for interface in interfaces {
        identities.insert(interface.type_identity());
        collect_methods(&interface.methods, known, delegates, &mut identities);
    }
    PythonProjectionContext::new(identities, packaged).unwrap()
}

pub fn projection_context(
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
    known: &HashSet<String>,
    delegates: &HashSet<String>,
) -> PythonProjectionContext {
    context(
        &classes.iter().collect::<Vec<_>>(),
        &interfaces.iter().collect::<Vec<_>>(),
        known,
        delegates,
        false,
    )
}

pub fn packaged_projection_context(
    classes: &[ClassMeta],
    interfaces: &[InterfaceMeta],
    known: &HashSet<String>,
    delegates: &HashSet<String>,
) -> PythonProjectionContext {
    context(
        &classes.iter().collect::<Vec<_>>(),
        &interfaces.iter().collect::<Vec<_>>(),
        known,
        delegates,
        true,
    )
}

pub fn generate_class(
    class: &ClassMeta,
    known: &HashSet<String>,
    delegates: &HashSet<String>,
    shared_iids: &HashSet<String>,
) -> String {
    let context = context(&[class], &[], known, delegates, false);
    python::generate_class(&context, class, shared_iids)
}

pub fn generate_class_stub(
    class: &ClassMeta,
    known: &HashSet<String>,
    delegates: &HashSet<String>,
    shared_iids: &HashSet<String>,
) -> String {
    let context = context(&[class], &[], known, delegates, false);
    python_stub::generate_class_stub(&context, class, shared_iids)
}

pub fn generate_interface(
    interface: &InterfaceMeta,
    known: &HashSet<String>,
    delegates: &HashSet<String>,
) -> String {
    let context = context(&[], &[interface], known, delegates, false);
    python::generate_interface(&context, interface)
}

pub fn generate_interface_stub(
    interface: &InterfaceMeta,
    known: &HashSet<String>,
    delegates: &HashSet<String>,
) -> String {
    let context = context(&[], &[interface], known, delegates, false);
    python_stub::generate_interface_stub(&context, interface)
}
