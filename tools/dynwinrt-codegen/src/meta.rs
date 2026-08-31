// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use windows_metadata::{HasAttributes, reader};

use crate::types::{EnumMember, TypeKind, TypeMeta, TypeRef};

pub const WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE: &str = "Windows.Foundation.Collections";
pub const PIID_IVECTOR: &str = "913337e9-11a1-4345-a3a2-4e7f956e222d";
pub const PIID_IOBSERVABLE_VECTOR: &str = "5917eb53-50b4-4a0d-b309-65862b3f1dbc";

/// Direction of a method parameter at the ABI level.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamDirection {
    In,
    Out,
    /// FillArray: caller allocates buffer, callee fills it.
    OutFill,
}

/// A single method parameter.
#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub name: String,
    pub typ: TypeMeta,
    pub direction: ParamDirection,
}

/// A method on a WinRT interface.
#[derive(Debug, Clone, Default)]
pub struct MethodMeta {
    pub name: String,
    pub vtable_index: usize,
    pub params: Vec<ParamMeta>,
    pub return_type: Option<TypeMeta>,
    pub is_property_getter: bool,
    pub is_property_setter: bool,
    pub is_event_add: bool,
    pub is_event_remove: bool,
    /// Original CLR method name before any OverloadAttribute rename.
    /// Used for XML doc lookup (`.xml` keys the CLR name, not the renamed name).
    pub raw_name: String,
    /// CLR-style signature key for overload disambiguation in XML doc:
    /// `(Type1,Type2)` or `()` for no-arg methods. Uses raw winmd parameter
    /// order (including out-params), with CLR type names like `System.String`.
    pub raw_signature_key: String,
    /// XML doc summary (populated from sibling .xml).
    pub doc: Option<String>,
    /// XML `<deprecated>` text.
    pub deprecated: Option<String>,
    /// Per-parameter doc, keyed by raw param name.
    pub param_docs: std::collections::HashMap<String, String>,
    /// XML `<returns>` text.
    pub returns_doc: Option<String>,
}

/// A WinRT interface with its methods.
#[derive(Debug, Clone, Default)]
pub struct InterfaceMeta {
    pub name: String,
    pub namespace: String,
    pub iid: String,
    /// Direct WinRT interface bases, excluding IInspectable/IUnknown.
    pub base_interfaces: Vec<TypeMeta>,
    pub methods: Vec<MethodMeta>,
    /// For parameterized interfaces: the PIID (generic IID before instantiation).
    pub generic_piid: Option<String>,
    /// For parameterized interfaces: the type arguments used to instantiate.
    pub generic_args: Vec<TypeMeta>,
    /// XML doc summary (populated from sibling .xml).
    pub doc: Option<String>,
    /// XML `<deprecated>` text.
    pub deprecated: Option<String>,
}

fn same_interface_identity(left: &InterfaceMeta, right: &InterfaceMeta) -> bool {
    if left.namespace != right.namespace {
        return false;
    }
    match (&left.generic_piid, &right.generic_piid) {
        (Some(left_piid), Some(right_piid)) => {
            left_piid == right_piid && left.generic_args == right.generic_args
        }
        (None, None) if !left.iid.is_empty() || !right.iid.is_empty() => left.iid == right.iid,
        (None, None) => left.name == right.name,
        _ => false,
    }
}

fn push_unique_interface(interfaces: &mut Vec<InterfaceMeta>, interface: InterfaceMeta) {
    if !interfaces
        .iter()
        .any(|existing| same_interface_identity(existing, &interface))
    {
        interfaces.push(interface);
    }
}

/// How an interface relates to a RuntimeClass.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceRole {
    Default,
    Factory,
    Static,
    Other,
}

/// The WinMD activation metadata that defines a runtime-class constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructorKind {
    DefaultActivation,
    FactoryActivation,
    PublicComposition,
    ProtectedComposition,
}

/// A constructor declared by ActivatableAttribute or ComposableAttribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructorMeta {
    pub kind: ConstructorKind,
    pub factory_interface: Option<TypeRef>,
}

impl ConstructorMeta {
    pub fn is_public(&self) -> bool {
        self.kind != ConstructorKind::ProtectedComposition
    }
}

/// A WinRT RuntimeClass with all its interfaces.
#[derive(Debug, Clone, Default)]
pub struct ClassMeta {
    pub name: String,
    pub namespace: String,
    pub full_name: String,
    /// Direct WinRT runtime-class base, excluding System.Object.
    pub base_class: Option<TypeRef>,
    pub default_interface: Option<InterfaceMeta>,
    /// Supplemental interfaces, including parameterized and versioned interfaces.
    pub required_interfaces: Vec<InterfaceMeta>,
    /// Native virtual interfaces marked OverridableAttribute. These describe
    /// implementation callbacks, not callable projected instance members.
    pub overridable_interfaces: Vec<InterfaceMeta>,
    pub factory_interfaces: Vec<InterfaceMeta>,
    pub static_interfaces: Vec<InterfaceMeta>,
    pub has_default_constructor: bool,
    pub constructors: Vec<ConstructorMeta>,
    /// The runtime class declares MarshalingBehavior(Agile).
    pub is_agile: bool,
    /// XML doc summary (populated from sibling .xml).
    pub doc: Option<String>,
    /// XML `<deprecated>` text.
    pub deprecated: Option<String>,
}

impl ClassMeta {
    /// Iterate over every interface implemented or exposed by the class.
    pub fn all_interfaces(&self) -> impl Iterator<Item = &InterfaceMeta> {
        self.default_interface
            .iter()
            .chain(self.factory_interfaces.iter())
            .chain(self.static_interfaces.iter())
            .chain(self.required_interfaces.iter())
    }

    /// Whether authoritative WinMD metadata declares parameterless activation.
    pub fn has_default_activation(&self) -> bool {
        self.constructors
            .iter()
            .any(|constructor| constructor.kind == ConstructorKind::DefaultActivation)
    }

    /// Whether an interface is named by public activation/composition metadata.
    pub fn is_public_constructor_factory(&self, interface: &InterfaceMeta) -> bool {
        self.constructors.iter().any(|constructor| {
            constructor.is_public()
                && constructor
                    .factory_interface
                    .as_ref()
                    .is_some_and(|reference| {
                        reference.namespace == interface.namespace
                            && reference.name == interface.name
                    })
        })
    }
}

/// List all unique namespaces found in the given winmd files.
pub fn list_namespaces(winmd_paths: &str) -> Vec<String> {
    let index = match load_index(winmd_paths) {
        Some(idx) => idx,
        None => return Vec::new(),
    };
    let mut namespaces: HashSet<String> = HashSet::new();
    for def in index.all() {
        let ns = def.namespace();
        if !ns.is_empty() {
            namespaces.insert(ns.to_string());
        }
    }
    let mut sorted: Vec<String> = namespaces.into_iter().collect();
    sorted.sort();
    sorted
}

/// Parse a WinMD file and extract metadata for a single RuntimeClass.
/// Accepts multiple winmd paths separated by ';'.
pub fn parse_class(winmd_paths: &str, namespace: &str, name: &str) -> Option<ClassMeta> {
    let index = load_index(winmd_paths)?;
    parse_class_from_index(&index, namespace, name)
}

/// Parse all RuntimeClasses in a given namespace.
pub fn parse_namespace(winmd_paths: &str, namespace: &str) -> Vec<ClassMeta> {
    let index = match load_index(winmd_paths) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut classes = Vec::new();
    for def in index.all() {
        if def.namespace() != namespace {
            continue;
        }
        // Skip CLR projection types (e.g. "<CLR>AdaptiveTextStyle") — .NET interop internals
        if def.name().starts_with('<') {
            continue;
        }
        let extends = match def.extends() {
            Some(e) => e,
            None => continue,
        };
        // A WinRT runtime class either extends System.Object directly or extends
        // another WinRT class (WinUI XAML classes: Button -> ButtonBase ->
        // ContentControl -> ... -> UIElement -> DependencyObject). Filter out
        // structs/enums/interfaces (which extend System.ValueType, System.Enum,
        // or nothing) but keep any class that extends a real WinRT type.
        let extends_system_object = extends.namespace() == "System" && extends.name() == "Object";
        let extends_winrt_class = !matches!(
            (extends.namespace(), extends.name()),
            ("System", "Object")
                | ("System", "ValueType")
                | ("System", "Enum")
                | ("System", "Delegate")
                | ("System", "MulticastDelegate")
                | ("System", _)
        );
        if !extends_system_object && !extends_winrt_class {
            continue;
        }

        if let Some(class) = parse_class_from_index(&index, namespace, def.name()) {
            classes.push(class);
        }
    }
    classes
}

/// Parse public (non-exclusive) interfaces in a namespace.
/// Exclusive interfaces (prefixed with I and paired with a RuntimeClass) are skipped
/// since they are implementation details. We only generate public-facing interfaces.
pub fn parse_interfaces(winmd_paths: &str, namespace: &str) -> Vec<InterfaceMeta> {
    let index = match load_index(winmd_paths) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut interfaces = Vec::new();
    for def in index.all() {
        if def.namespace() != namespace {
            continue;
        }
        // Skip CLR projection types
        if def.name().starts_with('<') {
            continue;
        }
        // Interfaces have no extends (or extend nothing)
        if def.extends().is_some() {
            continue;
        }
        // Skip generic interface definitions (they have generic params)
        if def.generic_params().next().is_some() {
            continue;
        }
        // Check it's actually an interface by looking for GuidAttribute
        let iid = extract_iid(&def);
        if iid.is_empty() {
            continue;
        }
        // Skip exclusive interfaces (marked with ExclusiveTo attribute)
        if def.has_attribute("ExclusiveToAttribute") {
            continue;
        }
        if let Some(iface) = parse_interface(&index, namespace, def.name()) {
            interfaces.push(iface);
        }
    }
    interfaces
}

/// Parse enums in a namespace.
pub fn parse_enums(winmd_paths: &str, namespace: &str) -> Vec<TypeMeta> {
    let index = match load_index(winmd_paths) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let mut enums = Vec::new();
    for def in index.all() {
        if def.namespace() != namespace {
            continue;
        }
        // Skip CLR projection types
        if def.name().starts_with('<') {
            continue;
        }
        if let Some(extends) = def.extends() {
            if extends.namespace() == "System" && extends.name() == "Enum" {
                enums.push(parse_enum_def(&def));
            }
        }
    }
    enums
}

/// Collect all type references from a class (both same-namespace and cross-namespace).
/// Excludes the class itself.
pub fn collect_imports(class: &ClassMeta) -> HashSet<(String, String)> {
    let mut imports: HashSet<(String, String)> = HashSet::new();
    let class_name = &class.name;

    fn visit_type(typ: &TypeMeta, class_name: &str, imports: &mut HashSet<(String, String)>) {
        let mut named = Vec::new();
        let mut _param = Vec::new();
        visit_type_refs(typ, &mut named, &mut _param);
        for r in named {
            if r.name != class_name {
                imports.insert((r.namespace, r.name));
            }
        }
    }

    for iface in class.all_interfaces() {
        for m in &iface.methods {
            for p in &m.params {
                visit_type(&p.typ, class_name, &mut imports);
            }
            if let Some(ref rt) = m.return_type {
                visit_type(rt, class_name, &mut imports);
            }
        }
    }

    imports
}

/// Resolved dependency types that need to be generated.
pub struct ResolvedDeps {
    pub classes: Vec<ClassMeta>,
    pub interfaces: Vec<InterfaceMeta>,
    pub enums: Vec<TypeMeta>,
}

/// Resolve all referenced types that don't have generated files yet.
/// Uses fixpoint iteration to recursively discover transitive dependencies.
pub fn resolve_dependencies(
    winmd_paths: &str,
    classes: &[ClassMeta],
    existing_interfaces: &[InterfaceMeta],
    existing_enums: &[TypeMeta],
) -> ResolvedDeps {
    resolve_dependencies_impl(
        winmd_paths,
        classes,
        existing_interfaces,
        existing_enums,
        false,
        true,
    )
}

/// Resolve dependencies needed by Python's structural class and interface stubs.
pub fn resolve_python_dependencies(
    winmd_paths: &str,
    classes: &[ClassMeta],
    existing_interfaces: &[InterfaceMeta],
    existing_enums: &[TypeMeta],
) -> ResolvedDeps {
    resolve_dependencies_impl(
        winmd_paths,
        classes,
        existing_interfaces,
        existing_enums,
        true,
        false,
    )
}

fn resolve_dependencies_impl(
    winmd_paths: &str,
    classes: &[ClassMeta],
    existing_interfaces: &[InterfaceMeta],
    existing_enums: &[TypeMeta],
    include_inheritance: bool,
    fully_qualified_identity: bool,
) -> ResolvedDeps {
    let index = match load_index(winmd_paths) {
        Some(idx) => idx,
        None => {
            return ResolvedDeps {
                classes: vec![],
                interfaces: vec![],
                enums: vec![],
            };
        }
    };

    // Track all known type names (already generated or discovered)
    let mut known: HashSet<String> = HashSet::new();
    for c in classes {
        known.insert(c.name.clone());
    }
    for i in existing_interfaces {
        known.insert(i.name.clone());
    }
    for e in existing_enums {
        if let TypeMeta::Enum { name, .. } = e {
            known.insert(name.clone());
        }
    }
    let mut known_named = HashSet::new();
    known_named.extend(classes.iter().map(|class| TypeRef {
        namespace: class.namespace.clone(),
        name: class.name.clone(),
        kind: TypeKind::Class,
    }));
    known_named.extend(existing_interfaces.iter().map(|interface| TypeRef {
        namespace: interface.namespace.clone(),
        name: interface.name.clone(),
        kind: TypeKind::Interface,
    }));
    known_named.extend(existing_enums.iter().filter_map(|typ| {
        let TypeMeta::Enum {
            namespace, name, ..
        } = typ
        else {
            return None;
        };
        Some(TypeRef {
            namespace: namespace.clone(),
            name: name.clone(),
            kind: TypeKind::Enum,
        })
    }));
    let mut known_parameterized = existing_interfaces
        .iter()
        .filter_map(|interface| {
            interface.generic_piid.as_ref().map(|piid| {
                let suffix = make_parameterized_name("", &interface.generic_args);
                let generic_name = interface
                    .name
                    .strip_suffix(&suffix)
                    .unwrap_or(&interface.name);
                parameterized_dependency_key(
                    &interface.namespace,
                    generic_name,
                    piid,
                    &interface.generic_args,
                )
            })
        })
        .collect::<HashSet<_>>();

    let mut dep_classes: Vec<ClassMeta> = Vec::new();
    let mut dep_interfaces: Vec<InterfaceMeta> = Vec::new();
    let mut dep_enums: Vec<TypeMeta> = Vec::new();

    // Seed the worklist from initial types
    let mut worklist: Vec<TypeRef> = Vec::new();
    let mut param_worklist: Vec<TypeMeta> = Vec::new();
    collect_all_refs_from_classes(
        classes,
        &known,
        &mut worklist,
        &mut param_worklist,
        include_inheritance,
    );
    collect_all_refs_from_interfaces(
        existing_interfaces,
        &known,
        &mut worklist,
        &mut param_worklist,
        include_inheritance,
    );

    // Fixpoint: keep resolving until no new types are discovered
    loop {
        let has_work = !worklist.is_empty() || !param_worklist.is_empty();
        if !has_work {
            break;
        }

        let batch: Vec<_> = worklist.drain(..).collect();
        let param_batch: Vec<_> = param_worklist.drain(..).collect();
        let mut new_classes = Vec::new();
        let mut new_interfaces = Vec::new();

        for r in &batch {
            if fully_qualified_identity {
                if !known_named.insert(r.clone()) {
                    continue;
                }
            } else if known.contains(&r.name) {
                continue;
            }
            known.insert(r.name.clone());

            match r.kind {
                TypeKind::Interface => {
                    if let Some(iface) = parse_interface(&index, &r.namespace, &r.name) {
                        new_interfaces.push(iface);
                    } else {
                        eprintln!(
                            "warning: interface {}.{} not found in loaded winmd files",
                            r.namespace, r.name
                        );
                    }
                }
                TypeKind::Class => {
                    if let Some(class) = parse_class_from_index(&index, &r.namespace, &r.name) {
                        new_classes.push(class);
                    } else {
                        eprintln!(
                            "warning: class {}.{} not found in loaded winmd files",
                            r.namespace, r.name
                        );
                    }
                }
                TypeKind::Enum => {
                    if r.name.starts_with('<') {
                        continue;
                    } // skip CLR projection types
                    if let Some(def) = index.get(&r.namespace, &r.name).next() {
                        dep_enums.push(parse_enum_def(&def));
                    } else {
                        eprintln!(
                            "warning: enum {}.{} not found in loaded winmd files",
                            r.namespace, r.name
                        );
                    }
                }
            }
        }

        // Resolve parameterized interfaces (e.g. IVector<String>)
        for param_type in &param_batch {
            if let TypeMeta::Parameterized {
                namespace,
                name,
                piid,
                args,
            } = param_type
            {
                let resolved_args = args
                    .iter()
                    .map(|argument| {
                        map_winmd_type_with_generics(
                            &type_meta_to_winmd_type(argument),
                            &index,
                            &[],
                        )
                    })
                    .collect::<Vec<_>>();
                if resolved_args
                    .iter()
                    .any(|argument| !is_resolved_generic_arg(argument))
                {
                    continue;
                }
                let concrete_name = make_parameterized_name(name, &resolved_args);
                if fully_qualified_identity {
                    let identity =
                        parameterized_dependency_key(namespace, name, piid, &resolved_args);
                    if !known_parameterized.insert(identity) {
                        continue;
                    }
                } else if known.contains(&concrete_name) {
                    continue;
                }
                known.insert(concrete_name.clone());

                if let Some(iface) = parse_parameterized_interface(
                    &index,
                    namespace,
                    name,
                    &concrete_name,
                    piid,
                    &resolved_args,
                ) {
                    new_interfaces.push(iface);
                } else {
                    eprintln!(
                        "warning: parameterized interface {}.{} (as {}) not found in loaded winmd files",
                        namespace, name, concrete_name
                    );
                }
            }
        }

        // Discover new references from the newly resolved types
        collect_all_refs_from_classes(
            &new_classes,
            &known,
            &mut worklist,
            &mut param_worklist,
            include_inheritance,
        );
        collect_all_refs_from_interfaces(
            &new_interfaces,
            &known,
            &mut worklist,
            &mut param_worklist,
            include_inheritance,
        );

        dep_classes.extend(new_classes);
        dep_interfaces.extend(new_interfaces);
    }

    ResolvedDeps {
        classes: dep_classes,
        interfaces: dep_interfaces,
        enums: dep_enums,
    }
}

/// Visit a TypeMeta tree and collect both named type references and parameterized types.
fn visit_type_refs(typ: &TypeMeta, named: &mut Vec<TypeRef>, parameterized: &mut Vec<TypeMeta>) {
    match typ {
        TypeMeta::Interface {
            namespace, name, ..
        } => {
            named.push(TypeRef {
                namespace: namespace.clone(),
                name: name.clone(),
                kind: TypeKind::Interface,
            });
        }
        TypeMeta::RuntimeClass {
            namespace, name, ..
        } => {
            named.push(TypeRef {
                namespace: namespace.clone(),
                name: name.clone(),
                kind: TypeKind::Class,
            });
        }
        TypeMeta::Enum {
            namespace, name, ..
        } => {
            named.push(TypeRef {
                namespace: namespace.clone(),
                name: name.clone(),
                kind: TypeKind::Enum,
            });
        }
        TypeMeta::AsyncOperation(inner) | TypeMeta::AsyncActionWithProgress(inner) => {
            visit_type_refs(inner, named, parameterized);
        }
        TypeMeta::AsyncOperationWithProgress(r, p) => {
            visit_type_refs(r, named, parameterized);
            visit_type_refs(p, named, parameterized);
        }
        TypeMeta::Struct { fields, .. } => {
            for f in fields {
                visit_type_refs(&f.typ, named, parameterized);
            }
        }
        TypeMeta::Array(inner) => {
            visit_type_refs(inner, named, parameterized);
        }
        TypeMeta::Parameterized { args, .. } => {
            parameterized.push(typ.clone());
            for arg in args {
                visit_type_refs(arg, named, parameterized);
            }
        }
        _ => {}
    }
}

/// Collect all type references from methods: both named and parameterized.
fn collect_all_refs_from_methods(
    methods: &[MethodMeta],
    _known: &HashSet<String>,
    named_out: &mut Vec<TypeRef>,
    param_out: &mut Vec<TypeMeta>,
) {
    let mut named = Vec::new();
    let mut parameterized = Vec::new();
    for m in methods {
        for p in &m.params {
            visit_type_refs(&p.typ, &mut named, &mut parameterized);
        }
        if let Some(ref rt) = m.return_type {
            visit_type_refs(rt, &mut named, &mut parameterized);
        }
    }
    named_out.extend(named);
    param_out.extend(parameterized);
}

fn collect_interface_base_ref(
    base: &TypeMeta,
    _known: &HashSet<String>,
    named_out: &mut Vec<TypeRef>,
    param_out: &mut Vec<TypeMeta>,
) {
    let mut named = Vec::new();
    let mut parameterized = Vec::new();
    visit_type_refs(base, &mut named, &mut parameterized);
    named_out.extend(named);
    param_out.extend(parameterized);
}

fn type_dependency_key(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "bool".into(),
        TypeMeta::I8 => "i8".into(),
        TypeMeta::U8 => "u8".into(),
        TypeMeta::I16 => "i16".into(),
        TypeMeta::U16 => "u16".into(),
        TypeMeta::I32 => "i32".into(),
        TypeMeta::U32 => "u32".into(),
        TypeMeta::I64 => "i64".into(),
        TypeMeta::U64 => "u64".into(),
        TypeMeta::F32 => "f32".into(),
        TypeMeta::F64 => "f64".into(),
        TypeMeta::Char16 => "char16".into(),
        TypeMeta::String => "string".into(),
        TypeMeta::Guid => "guid".into(),
        TypeMeta::Object => "object".into(),
        TypeMeta::Interface {
            namespace, name, ..
        } => format!("interface:{namespace}.{name}"),
        TypeMeta::RuntimeClass {
            namespace, name, ..
        } => format!("class:{namespace}.{name}"),
        TypeMeta::Delegate {
            namespace, name, ..
        } => format!("delegate:{namespace}.{name}"),
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => parameterized_dependency_key(namespace, name, piid, args),
        TypeMeta::Array(inner) => format!("array:{}", type_dependency_key(inner)),
        TypeMeta::Struct {
            namespace, name, ..
        } => format!("struct:{namespace}.{name}"),
        TypeMeta::Enum {
            namespace, name, ..
        } => format!("enum:{namespace}.{name}"),
        TypeMeta::AsyncAction => "async-action".into(),
        TypeMeta::AsyncActionWithProgress(progress) => {
            format!("async-action-progress:{}", type_dependency_key(progress))
        }
        TypeMeta::AsyncOperation(result) => {
            format!("async-operation:{}", type_dependency_key(result))
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => format!(
            "async-operation-progress:{},{}",
            type_dependency_key(result),
            type_dependency_key(progress)
        ),
    }
}

fn parameterized_dependency_key(
    namespace: &str,
    name: &str,
    piid: &str,
    args: &[TypeMeta],
) -> String {
    format!(
        "generic:{namespace}.{name}:{piid}<{}>",
        args.iter()
            .map(type_dependency_key)
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Generate a concrete name for a parameterized interface, e.g. "IVector_String", "IMap_String_Object".
pub fn make_parameterized_name(generic_name: &str, args: &[TypeMeta]) -> String {
    let base = generic_name.split('`').next().unwrap_or(generic_name);
    let arg_names: Vec<String> = args.iter().map(|a| type_meta_short_name(a)).collect();
    format!("{}_{}", base, arg_names.join("_"))
}

fn type_meta_short_name(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Bool => "Boolean".to_string(),
        TypeMeta::I8 => "Int8".to_string(),
        TypeMeta::U8 => "UInt8".to_string(),
        TypeMeta::I16 => "Int16".to_string(),
        TypeMeta::U16 => "UInt16".to_string(),
        TypeMeta::I32 => "Int32".to_string(),
        TypeMeta::U32 => "UInt32".to_string(),
        TypeMeta::I64 => "Int64".to_string(),
        TypeMeta::U64 => "UInt64".to_string(),
        TypeMeta::F32 => "Single".to_string(),
        TypeMeta::F64 => "Double".to_string(),
        TypeMeta::String => "String".to_string(),
        TypeMeta::Char16 => "Char16".to_string(),
        TypeMeta::Guid => "Guid".to_string(),
        TypeMeta::Object => "Object".to_string(),
        TypeMeta::RuntimeClass { name, .. }
        | TypeMeta::Interface { name, .. }
        | TypeMeta::Enum { name, .. }
        | TypeMeta::Struct { name, .. } => name.clone(),
        TypeMeta::Parameterized { name, args, .. } => make_parameterized_name(name, args),
        _ => "Unknown".to_string(),
    }
}

/// Collect all refs from a list of classes (iterates all interface methods).
fn collect_all_refs_from_classes(
    classes: &[ClassMeta],
    known: &HashSet<String>,
    named_out: &mut Vec<TypeRef>,
    param_out: &mut Vec<TypeMeta>,
    include_class_bases: bool,
) {
    for c in classes {
        if include_class_bases && let Some(base) = &c.base_class {
            named_out.push(base.clone());
        }
        for iface in c.all_interfaces() {
            collect_all_refs_from_methods(&iface.methods, known, named_out, param_out);
        }
        // Required interfaces themselves may need to be resolved
        for iface in &c.required_interfaces {
            if iface.generic_piid.is_none() && !iface.name.is_empty() {
                named_out.push(TypeRef {
                    namespace: iface.namespace.clone(),
                    name: iface.name.clone(),
                    kind: TypeKind::Interface,
                });
            }
        }
    }
}

/// Collect all refs from a list of standalone interfaces.
fn collect_all_refs_from_interfaces(
    interfaces: &[InterfaceMeta],
    known: &HashSet<String>,
    named_out: &mut Vec<TypeRef>,
    param_out: &mut Vec<TypeMeta>,
    include_interface_bases: bool,
) {
    for i in interfaces {
        collect_all_refs_from_methods(&i.methods, known, named_out, param_out);
        if include_interface_bases {
            for base in &i.base_interfaces {
                collect_interface_base_ref(base, known, named_out, param_out);
            }
        }
        if i.generic_piid.as_deref() == Some(PIID_IOBSERVABLE_VECTOR) && i.generic_args.len() == 1 {
            let vector = TypeMeta::Parameterized {
                namespace: WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE.into(),
                name: "IVector".into(),
                piid: PIID_IVECTOR.into(),
                args: i.generic_args.clone(),
            };
            param_out.push(vector);
        }
    }
}

// --- Internal helpers ---

/// Well-known PIIDs for generic interfaces whose GuidAttribute can't be read via extract_iid.
/// Expand winmd paths by discovering sibling .winmd files in the same directories.
/// Returns a new `;`-separated path string with siblings appended.
pub fn expand_winmd_paths(winmd_paths: &str) -> String {
    let explicit: Vec<&str> = winmd_paths.split(';').filter(|s| !s.is_empty()).collect();

    let mut all_paths: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Add explicit paths first
    for p in &explicit {
        let canonical = Path::new(p)
            .canonicalize()
            .map(|c| c.to_string_lossy().to_string())
            .unwrap_or_else(|_| p.to_string());
        if seen.insert(canonical.clone()) {
            all_paths.push(p.to_string());
        }
    }

    // Scan sibling .winmd files in the same directories
    let mut scanned_dirs: HashSet<String> = HashSet::new();
    for p in &explicit {
        if let Some(parent) = Path::new(p).parent() {
            let dir_key = parent
                .canonicalize()
                .map(|c| c.to_string_lossy().to_string())
                .unwrap_or_else(|_| parent.to_string_lossy().to_string());
            if !scanned_dirs.insert(dir_key) {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(parent) {
                let mut siblings = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .map_or(false, |ext| ext.eq_ignore_ascii_case("winmd"))
                    })
                    .collect::<Vec<_>>();
                siblings.sort_by(|left, right| {
                    let left = left.to_string_lossy();
                    let right = right.to_string_lossy();
                    left.to_ascii_lowercase()
                        .cmp(&right.to_ascii_lowercase())
                        .then_with(|| left.cmp(&right))
                });
                for path in siblings {
                    let canonical = path
                        .canonicalize()
                        .map(|c| c.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());
                    if seen.insert(canonical) {
                        eprintln!("Auto-discovered sibling winmd: {}", path.display());
                        all_paths.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    all_paths.join(";")
}

pub(crate) fn load_index(winmd_paths: &str) -> Option<reader::Index> {
    let paths: Vec<&str> = winmd_paths.split(';').filter(|s| !s.is_empty()).collect();
    if paths.is_empty() {
        eprintln!("warning: no winmd paths provided");
        return None;
    }
    if paths.len() == 1 {
        let result = reader::Index::read(paths[0]);
        if result.is_none() {
            eprintln!("warning: failed to read winmd file: {}", paths[0]);
        }
        result
    } else {
        let mut files = Vec::new();
        for path in &paths {
            if let Some(f) = reader::File::read(path) {
                files.push(f);
            } else {
                eprintln!("warning: failed to read winmd file: {}", path);
            }
        }
        if files.is_empty() {
            eprintln!(
                "warning: none of the {} winmd files could be loaded",
                paths.len()
            );
            None
        } else {
            Some(reader::Index::new(files))
        }
    }
}

fn parse_class_from_index(index: &reader::Index, namespace: &str, name: &str) -> Option<ClassMeta> {
    let def = index.get(namespace, name).next()?;
    let full_name = format!("{}.{}", namespace, name);
    let base_class = def.extends().and_then(|base| {
        let namespace = base.namespace().to_string();
        let name = base.name().to_string();
        (!(namespace == "System" && name == "Object")).then_some(TypeRef {
            namespace,
            name,
            kind: TypeKind::Class,
        })
    });

    let mut default_interface = None;
    let mut factory_interfaces = Vec::new();
    let mut static_interfaces = Vec::new();
    let mut generic_required_interfaces = Vec::new();
    let mut overridable_interfaces = Vec::new();
    let mut has_default_constructor = false;
    let mut constructors = Vec::new();
    let mut is_agile = false;

    // 1. Find default interface and collect all required interfaces
    let mut required_iface_names: Vec<(String, String)> = Vec::new();
    for iface_impl in def.interface_impls() {
        let iface_ty = iface_impl.interface(&[]);
        let (iface_ns, iface_name) = match &iface_ty {
            windows_metadata::Type::Name(tn) => (tn.namespace.clone(), tn.name.clone()),
            _ => continue,
        };

        if iface_impl.has_attribute("OverridableAttribute") {
            if let Some(iface_meta) = parse_interface_type(index, &iface_ty)
                && !overridable_interfaces
                    .iter()
                    .any(|existing| same_interface_identity(existing, &iface_meta))
            {
                overridable_interfaces.push(iface_meta);
            }
            continue;
        }
        if iface_impl.has_attribute("DefaultAttribute") {
            if let Some(iface_meta) = parse_interface_type(index, &iface_ty) {
                default_interface = Some(iface_meta);
            }
        } else if matches!(
            &iface_ty,
            windows_metadata::Type::Name(type_name) if !type_name.generics.is_empty()
        ) {
            if let Some(iface_meta) = parse_interface_type(index, &iface_ty) {
                push_unique_interface(&mut generic_required_interfaces, iface_meta);
            }
        } else {
            // Non-default required interface (e.g. ILanguageModel2, versioned interfaces)
            required_iface_names.push((iface_ns, iface_name));
        }
    }

    // 1a. Walk ancestor classes and inherit their interfaces. In WinRT the runtime
    // reaches parent-class members via QI on the child instance — but each parent's
    // interface_impls are only listed on the parent's TypeDef, not repeated on the
    // child. So we walk def.extends() ourselves and flatten every
    // ancestor interface onto this class's required_interfaces. Without this,
    // Button.background (from Control), ScrollViewer.content (from ContentControl),
    // and every other inherited member is silently absent from the wrapper class.
    let mut ancestor_key: Option<(String, String)> = def
        .extends()
        .map(|e| (e.namespace().to_string(), e.name().to_string()));
    while let Some((ext_ns, ext_name)) = ancestor_key.take() {
        if ext_ns == "System" && ext_name == "Object" {
            break;
        }
        let parent_def = match index.get(&ext_ns, &ext_name).next() {
            Some(d) => d,
            None => break,
        };
        for iface_impl in parent_def.interface_impls() {
            let iface_ty = iface_impl.interface(&[]);
            if iface_impl.has_attribute("OverridableAttribute") {
                if let Some(iface_meta) = parse_interface_type(index, &iface_ty)
                    && !overridable_interfaces
                        .iter()
                        .any(|existing| same_interface_identity(existing, &iface_meta))
                {
                    overridable_interfaces.push(iface_meta);
                }
                continue;
            }
            if let windows_metadata::Type::Name(tn) = &iface_ty {
                if !tn.generics.is_empty() {
                    // Do not fall back to the open generic definition: a failed
                    // resolution would register the wrong IID and method types.
                    if let Some(iface_meta) = parse_interface_type(index, &iface_ty)
                        && !default_interface
                            .as_ref()
                            .is_some_and(|default| same_interface_identity(default, &iface_meta))
                    {
                        push_unique_interface(&mut generic_required_interfaces, iface_meta);
                    }
                    continue;
                }
                let key = (tn.namespace.clone(), tn.name.clone());
                if default_interface.as_ref().is_some_and(|default| {
                    default.generic_piid.is_none()
                        && default.namespace == key.0
                        && default.name == key.1
                }) {
                    continue;
                }
                if required_iface_names.iter().any(|k| k == &key) {
                    continue;
                }
                required_iface_names.push(key);
            }
        }
        ancestor_key = parent_def
            .extends()
            .map(|e| (e.namespace().to_string(), e.name().to_string()));
    }

    // 1b. Parse required interfaces (e.g. ILanguageModel2, versioned interfaces)
    // These contain instance methods accessible on the class, but on separate COM interfaces.
    let mut required_interfaces = Vec::new();
    for (ns, iname) in &required_iface_names {
        if let Some(req_iface) = parse_interface(index, ns, iname) {
            if !req_iface.methods.is_empty() {
                push_unique_interface(&mut required_interfaces, req_iface);
            }
        }
    }
    for interface in generic_required_interfaces {
        if !default_interface
            .as_ref()
            .is_some_and(|default| same_interface_identity(default, &interface))
        {
            push_unique_interface(&mut required_interfaces, interface);
        }
    }

    // 2. Find factory/static/default-constructor from class-level attributes
    for attr in def.attributes() {
        let attr_name = attr.ctor().parent().name().to_string();
        let values = attr.value();

        if attr_name == "ActivatableAttribute" {
            match values.first() {
                Some((_, windows_metadata::Value::Utf8(iface_full_name))) => {
                    // Factory interface specified
                    if let Some((ns, n)) = split_full_name(iface_full_name) {
                        if let Some(iface_meta) = parse_interface(index, ns, n) {
                            factory_interfaces.push(iface_meta);
                            push_unique_constructor(
                                &mut constructors,
                                ConstructorMeta {
                                    kind: ConstructorKind::FactoryActivation,
                                    factory_interface: Some(TypeRef {
                                        namespace: ns.to_string(),
                                        name: n.to_string(),
                                        kind: TypeKind::Interface,
                                    }),
                                },
                            );
                        }
                    }
                }
                Some((_, windows_metadata::Value::U32(_)))
                | Some((_, windows_metadata::Value::I32(_))) => {
                    // No factory interface — this is a default (parameterless) constructor
                    has_default_constructor = true;
                    push_unique_constructor(
                        &mut constructors,
                        ConstructorMeta {
                            kind: ConstructorKind::DefaultActivation,
                            factory_interface: None,
                        },
                    );
                }
                // Unknown or malformed activation metadata must fail closed.
                // Inferring a default constructor here makes system-returned
                // classes user-constructible.
                _ => {}
            }
        } else if attr_name == "MarshalingBehaviorAttribute" {
            is_agile = values.first().is_some_and(|(_, value)| {
                matches!(
                    value,
                    windows_metadata::Value::I32(2) | windows_metadata::Value::U32(2)
                )
            });
        } else if attr_name == "StaticAttribute" {
            if let Some((_, windows_metadata::Value::Utf8(iface_full_name))) = values.first() {
                if let Some((ns, n)) = split_full_name(iface_full_name) {
                    if let Some(iface_meta) = parse_interface(index, ns, n) {
                        static_interfaces.push(iface_meta);
                    }
                }
            }
        } else if attr_name == "ComposableAttribute" {
            // Composable: project the exclusive factory as a regular factory.
            if let Some((_, windows_metadata::Value::Utf8(iface_full_name))) = values.first() {
                if let Some((ns, n)) = split_full_name(iface_full_name) {
                    if let Some(iface_meta) = parse_interface(index, ns, n) {
                        factory_interfaces.push(iface_meta);
                        push_unique_constructor(
                            &mut constructors,
                            ConstructorMeta {
                                kind: composable_constructor_kind(
                                    values.get(1).map(|(_, value)| value),
                                ),
                                factory_interface: Some(TypeRef {
                                    namespace: ns.to_string(),
                                    name: n.to_string(),
                                    kind: TypeKind::Interface,
                                }),
                            },
                        );
                    }
                }
            }
        }
    }

    Some(ClassMeta {
        name: name.to_string(),
        namespace: namespace.to_string(),
        full_name,
        base_class,
        default_interface,
        required_interfaces,
        overridable_interfaces,
        factory_interfaces,
        static_interfaces,
        has_default_constructor,
        constructors,
        is_agile,
        doc: None,
        deprecated: None,
    })
}

fn push_unique_constructor(constructors: &mut Vec<ConstructorMeta>, constructor: ConstructorMeta) {
    if !constructors.contains(&constructor) {
        constructors.push(constructor);
    }
}

fn composable_constructor_kind(value: Option<&windows_metadata::Value>) -> ConstructorKind {
    let value = match value {
        Some(windows_metadata::Value::I32(value)) => Some(*value),
        Some(windows_metadata::Value::U32(value)) => i32::try_from(*value).ok(),
        Some(windows_metadata::Value::AttributeEnum(_, value)) => Some(*value),
        _ => None,
    };

    match value {
        Some(2) => ConstructorKind::PublicComposition,
        _ => ConstructorKind::ProtectedComposition,
    }
}

fn split_full_name(full_name: &str) -> Option<(&str, &str)> {
    let dot_pos = full_name.rfind('.')?;
    Some((&full_name[..dot_pos], &full_name[dot_pos + 1..]))
}

fn parse_interface(index: &reader::Index, namespace: &str, name: &str) -> Option<InterfaceMeta> {
    let def = index.get(namespace, name).next()?;
    let iid = extract_iid(&def);
    parse_interface_methods(index, &def, name, namespace, &iid, &[])
}

fn parse_interface_type(
    index: &reader::Index,
    interface_type: &windows_metadata::Type,
) -> Option<InterfaceMeta> {
    let windows_metadata::Type::Name(type_name) = interface_type else {
        return None;
    };
    if type_name.generics.is_empty() {
        return parse_interface(index, &type_name.namespace, &type_name.name);
    }
    if type_name.generics.iter().any(contains_open_generic) {
        return None;
    }

    let TypeMeta::Parameterized {
        namespace,
        name,
        piid,
        args,
    } = resolve_named_type(
        &type_name.namespace,
        &type_name.name,
        &type_name.generics,
        index,
        &[],
    )
    else {
        return None;
    };
    if piid.is_empty() || args.iter().any(|arg| !is_resolved_generic_arg(arg)) {
        return None;
    }
    let concrete_name = make_parameterized_name(&name, &args);
    parse_parameterized_interface(index, &namespace, &name, &concrete_name, &piid, &args)
}

fn contains_open_generic(typ: &windows_metadata::Type) -> bool {
    match typ {
        windows_metadata::Type::Generic(_) => true,
        windows_metadata::Type::Name(name) => name.generics.iter().any(contains_open_generic),
        windows_metadata::Type::Array(inner)
        | windows_metadata::Type::ArrayRef(inner)
        | windows_metadata::Type::ConstRef(inner)
        | windows_metadata::Type::PtrMut(inner, _)
        | windows_metadata::Type::PtrConst(inner, _)
        | windows_metadata::Type::ArrayFixed(inner, _) => contains_open_generic(inner),
        _ => false,
    }
}

fn is_resolved_generic_arg(typ: &TypeMeta) -> bool {
    match typ {
        TypeMeta::Interface { iid, .. } | TypeMeta::Delegate { iid, .. } => !iid.is_empty(),
        TypeMeta::RuntimeClass {
            default_interface, ..
        } => default_interface
            .as_deref()
            .is_some_and(is_resolved_generic_arg),
        TypeMeta::Parameterized { piid, args, .. } => {
            !piid.is_empty() && args.iter().all(is_resolved_generic_arg)
        }
        TypeMeta::Array(inner)
        | TypeMeta::AsyncOperation(inner)
        | TypeMeta::AsyncActionWithProgress(inner) => is_resolved_generic_arg(inner),
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            is_resolved_generic_arg(result) && is_resolved_generic_arg(progress)
        }
        TypeMeta::Struct { fields, .. } => fields
            .iter()
            .all(|field| is_resolved_generic_arg(&field.typ)),
        _ => true,
    }
}

/// Parse a parameterized interface definition (e.g. IVector`1) from winmd,
/// substituting generic type parameters with concrete types.
/// Returns an InterfaceMeta with a mangled name like "IVector_String".
fn parse_parameterized_interface(
    index: &reader::Index,
    namespace: &str,
    generic_name: &str,
    concrete_name: &str,
    piid: &str,
    generic_args: &[TypeMeta],
) -> Option<InterfaceMeta> {
    let trimmed_name = generic_name.split('`').next().unwrap_or(generic_name);
    let def = index.get(namespace, trimmed_name).next()?;
    parse_interface_methods(index, &def, concrete_name, namespace, piid, generic_args)
}

fn interface_meta_type_identity(typ: &TypeMeta) -> Option<(String, String)> {
    match typ {
        TypeMeta::Interface {
            namespace, name, ..
        } => Some((namespace.clone(), name.clone())),
        TypeMeta::Parameterized {
            namespace,
            name,
            args,
            ..
        } => Some((namespace.clone(), make_parameterized_name(name, args))),
        _ => None,
    }
}

fn parse_interface_meta_type(index: &reader::Index, typ: &TypeMeta) -> Option<InterfaceMeta> {
    match typ {
        TypeMeta::Interface {
            namespace, name, ..
        } => parse_interface(index, namespace, name),
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => {
            let concrete_name = make_parameterized_name(name, args);
            parse_parameterized_interface(index, namespace, name, &concrete_name, piid, args)
        }
        _ => None,
    }
}

fn collect_interface_base_closure(
    index: &reader::Index,
    interface: &InterfaceMeta,
    identities: &mut HashSet<(String, String)>,
) {
    for base in &interface.base_interfaces {
        let Some(identity) = interface_meta_type_identity(base) else {
            continue;
        };
        if identities.insert(identity)
            && let Some(base_interface) = parse_interface_meta_type(index, base)
        {
            collect_interface_base_closure(index, &base_interface, identities);
        }
    }
}

/// Core interface parsing: extract methods from a TypeDef, optionally substituting generics.
fn parse_interface_methods(
    index: &reader::Index,
    def: &reader::TypeDef,
    output_name: &str,
    namespace: &str,
    iid: &str,
    generic_args: &[TypeMeta],
) -> Option<InterfaceMeta> {
    let winmd_generics: Vec<windows_metadata::Type> =
        generic_args.iter().map(type_meta_to_winmd_type).collect();

    let mut methods = Vec::new();
    for (i, method) in def.methods().enumerate() {
        let vtable_index = 6 + i;
        let sig = method.signature(&winmd_generics);

        let raw_name = method.name().to_string();
        let overload_name = method.find_attribute("OverloadAttribute").and_then(|a| {
            a.value().into_iter().next().and_then(|(_, v)| match v {
                windows_metadata::Value::Utf8(s) => Some(s),
                _ => None,
            })
        });
        let method_name = overload_name.unwrap_or_else(|| raw_name.clone());

        let mut params = Vec::new();
        let param_defs: Vec<_> = method.params().filter(|p| p.sequence() > 0).collect();
        let mut clr_sig_types: Vec<String> = Vec::new();
        for (j, param_def) in param_defs.iter().enumerate() {
            if j < sig.types.len() {
                clr_sig_types.push(clr_type_name(&sig.types[j]));
                let typ = map_winmd_type_with_generics(&sig.types[j], index, generic_args);
                let is_out = param_def
                    .flags()
                    .contains(windows_metadata::ParamAttributes::Out);
                let direction = if is_out {
                    if matches!(sig.types[j], windows_metadata::Type::Array(_)) {
                        // [out] Array = FillArray (caller allocates buffer, callee fills)
                        ParamDirection::OutFill
                    } else {
                        ParamDirection::Out
                    }
                } else {
                    ParamDirection::In
                };
                params.push(ParamMeta {
                    name: param_def.name().to_string(),
                    typ,
                    direction,
                });
            }
        }

        let return_type = if sig.return_type == windows_metadata::Type::Void {
            None
        } else {
            Some(map_winmd_type_with_generics(
                &sig.return_type,
                index,
                generic_args,
            ))
        };

        let raw_signature_key = if clr_sig_types.is_empty() {
            "()".to_string()
        } else {
            format!("({})", clr_sig_types.join(","))
        };

        methods.push(MethodMeta {
            name: method_name.clone(),
            vtable_index,
            params,
            return_type,
            is_property_getter: method_name.starts_with("get_"),
            is_property_setter: method_name.starts_with("put_"),
            is_event_add: method_name.starts_with("add_"),
            is_event_remove: method_name.starts_with("remove_"),
            raw_name,
            raw_signature_key,
            doc: None,
            deprecated: None,
            param_docs: std::collections::HashMap::new(),
            returns_doc: None,
        });
    }

    let (generic_piid, generic_args_vec) = if !generic_args.is_empty() {
        (Some(iid.to_string()), generic_args.to_vec())
    } else {
        (None, Vec::new())
    };
    let mut base_interfaces = Vec::new();
    for interface_impl in def.interface_impls() {
        let interface_type = interface_impl.interface(&winmd_generics);
        let reference = match map_winmd_type_with_generics(&interface_type, index, generic_args) {
            typ @ TypeMeta::Interface { .. } | typ @ TypeMeta::Parameterized { .. } => Some(typ),
            _ => None,
        };
        if let Some(reference) = reference {
            let name = type_meta_short_name(&reference);
            if name != output_name
                && name != "IInspectable"
                && name != "IUnknown"
                && !base_interfaces.contains(&reference)
            {
                base_interfaces.push(reference);
            }
        }
    }
    let closures = base_interfaces
        .iter()
        .map(|base| {
            let mut identities = HashSet::new();
            if let Some(interface) = parse_interface_meta_type(index, base) {
                collect_interface_base_closure(index, &interface, &mut identities);
            }
            identities
        })
        .collect::<Vec<_>>();
    base_interfaces = base_interfaces
        .into_iter()
        .enumerate()
        .filter(|(index, base)| {
            let identity = interface_meta_type_identity(base);
            !closures.iter().enumerate().any(|(other, closure)| {
                other != *index && identity.as_ref().is_some_and(|id| closure.contains(id))
            })
        })
        .map(|(_, base)| base)
        .collect();
    Some(InterfaceMeta {
        name: output_name.to_string(),
        namespace: namespace.to_string(),
        iid: iid.to_string(),
        base_interfaces,
        methods,
        generic_piid,
        generic_args: generic_args_vec,
        doc: None,
        deprecated: None,
    })
}

/// Produce a .NET-style CLR type name for XML doc signature keys.
/// Examples: `System.Int32`, `System.String`, `Windows.Foundation.Uri`,
/// `System.String[]`, `Windows.Foundation.Collections.IVector`1<System.String>`.
fn clr_type_name(ty: &windows_metadata::Type) -> String {
    match ty {
        windows_metadata::Type::Bool => "System.Boolean".into(),
        windows_metadata::Type::Char => "System.Char".into(),
        windows_metadata::Type::I8 => "System.SByte".into(),
        windows_metadata::Type::U8 => "System.Byte".into(),
        windows_metadata::Type::I16 => "System.Int16".into(),
        windows_metadata::Type::U16 => "System.UInt16".into(),
        windows_metadata::Type::I32 => "System.Int32".into(),
        windows_metadata::Type::U32 => "System.UInt32".into(),
        windows_metadata::Type::I64 => "System.Int64".into(),
        windows_metadata::Type::U64 => "System.UInt64".into(),
        windows_metadata::Type::F32 => "System.Single".into(),
        windows_metadata::Type::F64 => "System.Double".into(),
        windows_metadata::Type::String => "System.String".into(),
        windows_metadata::Type::Object => "System.Object".into(),
        windows_metadata::Type::Array(inner) => format!("{}[]", clr_type_name(inner)),
        windows_metadata::Type::Name(tn) => {
            if tn.namespace == "System" && tn.name == "Guid" {
                "System.Guid".into()
            } else if tn.namespace.is_empty() {
                tn.name.to_string()
            } else {
                format!("{}.{}", tn.namespace, tn.name)
            }
        }
        _ => "System.Object".into(),
    }
}

/// Convert TypeMeta back to windows_metadata::Type (for passing to method.signature()).
fn type_meta_to_winmd_type(typ: &TypeMeta) -> windows_metadata::Type {
    match typ {
        TypeMeta::Bool => windows_metadata::Type::Bool,
        TypeMeta::I8 => windows_metadata::Type::I8,
        TypeMeta::U8 => windows_metadata::Type::U8,
        TypeMeta::I16 => windows_metadata::Type::I16,
        TypeMeta::U16 => windows_metadata::Type::U16,
        TypeMeta::I32 => windows_metadata::Type::I32,
        TypeMeta::U32 => windows_metadata::Type::U32,
        TypeMeta::I64 => windows_metadata::Type::I64,
        TypeMeta::U64 => windows_metadata::Type::U64,
        TypeMeta::F32 => windows_metadata::Type::F32,
        TypeMeta::F64 => windows_metadata::Type::F64,
        TypeMeta::String => windows_metadata::Type::String,
        TypeMeta::Char16 => windows_metadata::Type::Char,
        TypeMeta::Guid => windows_metadata::Type::named("System", "Guid"),
        TypeMeta::Object => windows_metadata::Type::Object,
        TypeMeta::RuntimeClass {
            namespace, name, ..
        }
        | TypeMeta::Interface {
            namespace, name, ..
        }
        | TypeMeta::Enum {
            namespace, name, ..
        }
        | TypeMeta::Struct {
            namespace, name, ..
        } => windows_metadata::Type::named(namespace, name),
        TypeMeta::Parameterized {
            namespace,
            name,
            args,
            ..
        } => windows_metadata::Type::Name(windows_metadata::TypeName {
            namespace: namespace.clone(),
            name: name.clone(),
            generics: args.iter().map(type_meta_to_winmd_type).collect(),
        }),
        TypeMeta::Array(inner) => {
            windows_metadata::Type::Array(Box::new(type_meta_to_winmd_type(inner)))
        }
        TypeMeta::AsyncAction => {
            windows_metadata::Type::named("Windows.Foundation", "IAsyncAction")
        }
        TypeMeta::AsyncOperation(inner) => {
            windows_metadata::Type::Name(windows_metadata::TypeName {
                namespace: "Windows.Foundation".to_string(),
                name: "IAsyncOperation`1".to_string(),
                generics: vec![type_meta_to_winmd_type(inner)],
            })
        }
        TypeMeta::AsyncActionWithProgress(progress) => {
            windows_metadata::Type::Name(windows_metadata::TypeName {
                namespace: "Windows.Foundation".to_string(),
                name: "IAsyncActionWithProgress`1".to_string(),
                generics: vec![type_meta_to_winmd_type(progress)],
            })
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            windows_metadata::Type::Name(windows_metadata::TypeName {
                namespace: "Windows.Foundation".to_string(),
                name: "IAsyncOperationWithProgress`2".to_string(),
                generics: vec![
                    type_meta_to_winmd_type(result),
                    type_meta_to_winmd_type(progress),
                ],
            })
        }
        TypeMeta::Delegate {
            namespace, name, ..
        } => windows_metadata::Type::named(namespace, name),
    }
}

pub(crate) fn extract_iid(def: &reader::TypeDef) -> String {
    if let Some(attr) = def.find_attribute("GuidAttribute") {
        let args: Vec<(String, windows_metadata::Value)> = attr.value();
        if args.len() >= 11 {
            let a = extract_u32(&args[0].1);
            let b = extract_u16(&args[1].1);
            let c = extract_u16(&args[2].1);
            let d = extract_u8(&args[3].1);
            let e = extract_u8(&args[4].1);
            let f = extract_u8(&args[5].1);
            let g = extract_u8(&args[6].1);
            let h = extract_u8(&args[7].1);
            let i = extract_u8(&args[8].1);
            let j = extract_u8(&args[9].1);
            let k = extract_u8(&args[10].1);
            return format!(
                "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                a, b, c, d, e, f, g, h, i, j, k
            );
        }
    }
    String::new()
}

fn extract_u32(val: &windows_metadata::Value) -> u32 {
    match val {
        windows_metadata::Value::U32(v) => *v,
        _ => 0,
    }
}

fn extract_u16(val: &windows_metadata::Value) -> u16 {
    match val {
        windows_metadata::Value::U16(v) => *v,
        _ => 0,
    }
}

fn extract_u8(val: &windows_metadata::Value) -> u8 {
    match val {
        windows_metadata::Value::U8(v) => *v,
        _ => 0,
    }
}

fn find_default_interface_type(def: &reader::TypeDef, index: &reader::Index) -> Option<TypeMeta> {
    for iface_impl in def.interface_impls() {
        if !iface_impl.has_attribute("DefaultAttribute") {
            continue;
        }
        let iface_ty = iface_impl.interface(&[]);
        if let windows_metadata::Type::Name(tn) = &iface_ty {
            return Some(resolve_named_type(
                &tn.namespace,
                &tn.name,
                &tn.generics,
                index,
                &[],
            ));
        }
    }
    None
}

fn parse_enum_def(def: &reader::TypeDef) -> TypeMeta {
    let mut members = Vec::new();
    for field in def.fields() {
        let name = field.name().to_string();
        if name == "value__" {
            continue; // Skip the underlying value field
        }
        // Enum fields have constant values
        if let Some(constant) = field.constant() {
            let value = match constant.value() {
                windows_metadata::Value::I32(v) => v,
                windows_metadata::Value::U32(v) => v as i32,
                _ => 0,
            };
            members.push(EnumMember {
                name,
                value,
                doc: None,
            });
        }
    }
    TypeMeta::Enum {
        namespace: def.namespace().to_string(),
        name: def.name().to_string(),
        underlying: Box::new(TypeMeta::I32),
        members,
        is_flags: def.has_attribute("FlagsAttribute"),
        doc: None,
        deprecated: None,
    }
}

fn map_winmd_type(ty: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    map_winmd_type_with_generics(ty, index, &[])
}

pub(crate) fn map_winmd_type_with_generics(
    ty: &windows_metadata::Type,
    index: &reader::Index,
    generic_args: &[TypeMeta],
) -> TypeMeta {
    use windows_metadata::Type;
    match ty {
        Type::Void => TypeMeta::Object,
        Type::Bool => TypeMeta::Bool,
        Type::I8 => TypeMeta::I8,
        Type::U8 => TypeMeta::U8,
        Type::I16 => TypeMeta::I16,
        Type::U16 => TypeMeta::U16,
        Type::I32 => TypeMeta::I32,
        Type::U32 => TypeMeta::U32,
        Type::I64 => TypeMeta::I64,
        Type::U64 => TypeMeta::U64,
        Type::F32 => TypeMeta::F32,
        Type::F64 => TypeMeta::F64,
        Type::Char => TypeMeta::Char16,
        Type::String => TypeMeta::String,
        Type::Object => TypeMeta::Object,

        Type::Generic(n) => {
            if (*n as usize) < generic_args.len() {
                generic_args[*n as usize].clone()
            } else {
                TypeMeta::Object
            }
        }

        Type::Name(tn) => {
            resolve_named_type(&tn.namespace, &tn.name, &tn.generics, index, generic_args)
        }

        Type::Array(inner) | Type::ArrayRef(inner) => TypeMeta::Array(Box::new(
            map_winmd_type_with_generics(inner, index, generic_args),
        )),

        _ => TypeMeta::Object,
    }
}

fn resolve_named_type(
    namespace: &str,
    name: &str,
    generics: &[windows_metadata::Type],
    index: &reader::Index,
    outer_generic_args: &[TypeMeta],
) -> TypeMeta {
    // System.Guid — not in Windows.winmd, handle as primitive
    if namespace == "System" && name == "Guid" {
        return TypeMeta::Guid;
    }

    // Well-known async types
    if namespace == "Windows.Foundation" {
        match name {
            "IAsyncAction" => return TypeMeta::AsyncAction,
            "IAsyncOperation`1" if generics.len() == 1 => {
                return TypeMeta::AsyncOperation(Box::new(map_winmd_type_with_generics(
                    &generics[0],
                    index,
                    outer_generic_args,
                )));
            }
            "IAsyncActionWithProgress`1" if generics.len() == 1 => {
                return TypeMeta::AsyncActionWithProgress(Box::new(map_winmd_type_with_generics(
                    &generics[0],
                    index,
                    outer_generic_args,
                )));
            }
            "IAsyncOperationWithProgress`2" if generics.len() == 2 => {
                return TypeMeta::AsyncOperationWithProgress(
                    Box::new(map_winmd_type_with_generics(
                        &generics[0],
                        index,
                        outer_generic_args,
                    )),
                    Box::new(map_winmd_type_with_generics(
                        &generics[1],
                        index,
                        outer_generic_args,
                    )),
                );
            }
            _ => {}
        }
    }

    // Parameterized interface (generics non-empty)
    if !generics.is_empty() {
        // Strip arity suffix (e.g. IVectorView`1 -> IVectorView) for winmd lookup
        let lookup_name = name.split('`').next().unwrap_or(name);
        let piid = match index.get(namespace, lookup_name).next() {
            Some(d) => extract_iid(&d),
            None => {
                eprintln!(
                    "warning: parameterized type {}.{} not found in loaded winmd files (cannot resolve PIID)",
                    namespace, name
                );
                String::new()
            }
        };
        let args = generics
            .iter()
            .map(|generic| map_winmd_type_with_generics(generic, index, outer_generic_args))
            .collect();
        return TypeMeta::Parameterized {
            namespace: namespace.to_string(),
            name: name.to_string(),
            piid,
            args,
        };
    }

    let def = match index.get(namespace, name).next() {
        Some(d) => d,
        None => {
            eprintln!(
                "warning: type {}.{} not found in loaded winmd files (using empty IID)",
                namespace, name
            );
            return TypeMeta::Interface {
                namespace: namespace.to_string(),
                name: name.to_string(),
                iid: String::new(),
            };
        }
    };

    if let Some(extends) = def.extends() {
        if extends.namespace() == "System" && extends.name() == "ValueType" {
            let fields = def
                .fields()
                .map(|f| crate::types::FieldMeta {
                    name: f.name().to_string(),
                    typ: map_winmd_type(&f.ty(), index),
                })
                .collect();
            return TypeMeta::Struct {
                namespace: namespace.to_string(),
                name: name.to_string(),
                fields,
            };
        }
        if extends.namespace() == "System" && extends.name() == "Enum" {
            return parse_enum_def(&def);
        }
        // Any WinRT class extends System.Object directly, or extends another
        // WinRT class (WinUI XAML: Button -> ButtonBase -> ... -> DependencyObject).
        // Only structs/enums/delegates hit the branches above; everything else
        // with an Extends is a runtime class.
        let is_delegate = matches!(
            (extends.namespace(), extends.name()),
            ("System", "Delegate") | ("System", "MulticastDelegate")
        );
        if !is_delegate {
            let default_interface = find_default_interface_type(&def, index);
            if let Some(default_type) = default_interface.as_ref() {
                if default_type.is_async() {
                    return default_type.clone();
                }
            }
            return TypeMeta::RuntimeClass {
                namespace: namespace.to_string(),
                name: name.to_string(),
                default_interface: default_interface.map(Box::new),
            };
        }
    }

    let iid = extract_iid(&def);
    TypeMeta::Interface {
        namespace: namespace.to_string(),
        name: name.to_string(),
        iid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_parameterized_name_single_arg() {
        let name = make_parameterized_name("IVector`1", &[TypeMeta::String]);
        assert_eq!(name, "IVector_String");
    }

    #[test]
    fn make_parameterized_name_two_args() {
        let name = make_parameterized_name("IMap`2", &[TypeMeta::String, TypeMeta::Object]);
        assert_eq!(name, "IMap_String_Object");
    }

    #[test]
    fn make_parameterized_name_nested() {
        let inner = TypeMeta::Parameterized {
            namespace: "C".into(),
            name: "IVector`1".into(),
            piid: "".into(),
            args: vec![TypeMeta::I32],
        };
        let name = make_parameterized_name("IIterable`1", &[inner]);
        assert_eq!(name, "IIterable_IVector_Int32");
    }

    #[test]
    fn instantiated_interface_identity_includes_generic_arguments() {
        let interface = |arg| InterfaceMeta {
            name: "ISameShortName".into(),
            namespace: "N".into(),
            iid: "generic-piid".into(),
            generic_piid: Some("generic-piid".into()),
            generic_args: vec![arg],
            ..Default::default()
        };
        let mut interfaces = Vec::new();
        push_unique_interface(&mut interfaces, interface(TypeMeta::U32));
        push_unique_interface(&mut interfaces, interface(TypeMeta::U64));
        push_unique_interface(&mut interfaces, interface(TypeMeta::U32));
        assert_eq!(interfaces.len(), 2);
    }

    #[test]
    fn parameterized_dependency_identity_includes_argument_namespace() {
        let key = |namespace: &str| {
            parameterized_dependency_key(
                WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
                "IVector",
                PIID_IVECTOR,
                &[TypeMeta::RuntimeClass {
                    namespace: namespace.into(),
                    name: "PointerPoint".into(),
                    default_interface: None,
                }],
            )
        };

        assert_ne!(key("Windows.UI.Input"), key("Microsoft.UI.Input"));
    }

    #[test]
    fn class_all_interfaces_iterates_all() {
        let mk_iface = |n: &str| InterfaceMeta {
            name: n.into(),
            namespace: "N".into(),
            iid: "".into(),
            methods: vec![],
            generic_piid: None,
            generic_args: vec![],
            ..Default::default()
        };
        let class = ClassMeta {
            name: "C".into(),
            namespace: "N".into(),
            full_name: "N.C".into(),
            default_interface: Some(mk_iface("IDef")),
            factory_interfaces: vec![mk_iface("IFact")],
            static_interfaces: vec![mk_iface("IStat")],
            required_interfaces: vec![mk_iface("IReq")],
            has_default_constructor: false,
            ..Default::default()
        };
        let names: Vec<&str> = class.all_interfaces().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["IDef", "IFact", "IStat", "IReq"]);
    }

    #[test]
    fn class_all_interfaces_handles_no_default() {
        let class = ClassMeta {
            name: "C".into(),
            namespace: "N".into(),
            full_name: "N.C".into(),
            default_interface: None,
            factory_interfaces: vec![],
            static_interfaces: vec![],
            required_interfaces: vec![],
            has_default_constructor: false,
            ..Default::default()
        };
        assert_eq!(class.all_interfaces().count(), 0);
    }

    #[test]
    fn expand_winmd_paths_empty() {
        assert_eq!(expand_winmd_paths(""), "");
    }

    #[test]
    fn split_full_name_works() {
        assert_eq!(
            split_full_name("Windows.Foundation.Uri"),
            Some(("Windows.Foundation", "Uri"))
        );
        assert_eq!(split_full_name("NoNamespace"), None);
    }

    const WINDOWS_WINMD: &str =
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

    #[test]
    fn test_parse_uri_class() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Foundation", "Uri").unwrap();
        assert_eq!(class.name, "Uri");
        assert_eq!(class.namespace, "Windows.Foundation");
        assert!(class.default_interface.is_some());
        assert!(!class.factory_interfaces.is_empty());
        assert!(class.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::FactoryActivation
                && constructor
                    .factory_interface
                    .as_ref()
                    .is_some_and(|interface| interface.name == "IUriRuntimeClassFactory")
        }));
    }

    #[test]
    fn test_uri_vtable_indices() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Foundation", "Uri").unwrap();
        let default_iface = class.default_interface.as_ref().unwrap();
        let scheme = default_iface
            .methods
            .iter()
            .find(|m| m.name == "get_SchemeName")
            .unwrap();
        assert!(scheme.is_property_getter);
        let port = default_iface
            .methods
            .iter()
            .find(|m| m.name == "get_Port")
            .unwrap();
        assert_eq!(port.return_type, Some(TypeMeta::I32));
    }

    #[test]
    fn test_uri_iid_not_empty() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Foundation", "Uri").unwrap();
        let default_iface = class.default_interface.as_ref().unwrap();
        assert!(!default_iface.iid.is_empty());
    }

    #[test]
    fn test_raw_name_and_signature_key_populated() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Foundation", "Uri").unwrap();
        let factory = class
            .factory_interfaces
            .iter()
            .find(|i| i.name == "IUriRuntimeClassFactory")
            .expect("Uri factory interface");
        // CreateUri is overloaded; the two-arg form is renamed by OverloadAttribute.
        let create = factory
            .methods
            .iter()
            .find(|m| m.raw_name == "CreateUri" && m.params.len() == 1)
            .unwrap();
        assert_eq!(create.raw_name, "CreateUri");
        assert_eq!(create.raw_signature_key, "(System.String)");
        // Return type is out param in winmd, so in-params alone determines the key.
        let create2 = factory
            .methods
            .iter()
            .find(|m| m.raw_name == "CreateWithRelativeUri")
            .unwrap();
        assert_eq!(create2.raw_name, "CreateWithRelativeUri");
        assert_eq!(create2.raw_signature_key, "(System.String,System.String)");

        // Zero-arg methods -> "()"
        let default = class.default_interface.as_ref().unwrap();
        let get_host = default
            .methods
            .iter()
            .find(|m| m.raw_name == "get_Host")
            .unwrap();
        assert_eq!(get_host.raw_signature_key, "()");
    }

    #[test]
    fn test_httpclient_has_default_constructor() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Web.Http", "HttpClient").unwrap();
        assert!(
            class.has_default_constructor,
            "HttpClient should have a default constructor"
        );
        assert!(class.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::DefaultActivation
                && constructor.factory_interface.is_none()
        }));
    }

    #[test]
    fn test_system_returned_class_has_no_constructor() {
        let class = parse_class(WINDOWS_WINMD, "Windows.System", "User").unwrap();
        assert!(class.constructors.is_empty());
        assert!(!class.static_interfaces.is_empty());
        assert!(
            !class.required_interfaces.is_empty(),
            "versioned User interfaces must remain available"
        );

        let known_types = HashSet::from(["User".to_string()]);
        let py = crate::codegen::winrt::python::generate_class(
            &class,
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
        );
        let pyi = crate::codegen::python_stub::generate_class_stub(
            &class,
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(py.contains("def _from_native(cls, obj: DynWinRTValue):"));
        assert!(py.contains("User cannot be constructed directly"));
        assert!(pyi.contains("def __init__(self, _not_constructible: NoReturn) -> None: ..."));
    }

    #[test]
    fn test_composition_visibility_values() {
        assert_eq!(
            composable_constructor_kind(Some(&windows_metadata::Value::I32(2))),
            ConstructorKind::PublicComposition
        );
        assert_eq!(
            composable_constructor_kind(Some(&windows_metadata::Value::I32(1))),
            ConstructorKind::ProtectedComposition
        );
        assert_eq!(
            composable_constructor_kind(None),
            ConstructorKind::ProtectedComposition
        );
    }

    #[test]
    fn test_winui_composition_visibility_when_metadata_is_available() {
        let Some(winui_winmd) = find_winui_winmd() else {
            eprintln!("WinUI metadata not installed; skipping composition metadata test");
            return;
        };
        let winmd_paths = format!("{};{}", WINDOWS_WINMD, winui_winmd.display());

        let stack_panel =
            parse_class(&winmd_paths, "Microsoft.UI.Xaml.Controls", "StackPanel").unwrap();
        assert!(stack_panel.is_agile);
        assert!(stack_panel.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::PublicComposition && constructor.is_public()
        }));
        let stack_factory = stack_panel
            .factory_interfaces
            .iter()
            .find(|interface| interface.name == "IStackPanelFactory")
            .unwrap();
        let create = stack_factory
            .methods
            .iter()
            .find(|method| method.name == "CreateInstance")
            .unwrap();
        assert_eq!(
            create
                .params
                .iter()
                .map(|param| (param.name.as_str(), &param.direction))
                .collect::<Vec<_>>(),
            vec![
                ("baseInterface", &ParamDirection::In),
                ("innerInterface", &ParamDirection::Out),
            ]
        );
        assert!(matches!(
            create.return_type,
            Some(TypeMeta::RuntimeClass { ref name, .. }) if name == "StackPanel"
        ));
        assert!(
            stack_panel
                .overridable_interfaces
                .iter()
                .any(|interface| interface.name == "IUIElementOverrides")
        );
        let framework_overrides = stack_panel
            .overridable_interfaces
            .iter()
            .find(|interface| interface.name == "IFrameworkElementOverrides")
            .unwrap();
        assert_eq!(
            framework_overrides.iid,
            "ffc6fd98-f38c-5904-9ce4-97a3427cf4ba"
        );
        assert_eq!(
            framework_overrides
                .methods
                .iter()
                .map(|method| (method.name.as_str(), method.vtable_index))
                .collect::<Vec<_>>(),
            vec![
                ("MeasureOverride", 6),
                ("ArrangeOverride", 7),
                ("OnApplyTemplate", 8),
                ("GoToElementStateCore", 9),
            ]
        );
        let on_apply_template = &framework_overrides.methods[2];
        assert!(on_apply_template.params.is_empty());
        assert!(on_apply_template.return_type.is_none());
        assert_eq!(
            stack_panel
                .overridable_interfaces
                .iter()
                .find(|interface| interface.name == "IUIElementOverrides")
                .unwrap()
                .iid,
            "9034f41e-ab7b-59e7-8168-50de6b689dde"
        );

        let automation_peer = parse_class(
            &winmd_paths,
            "Microsoft.UI.Xaml.Automation.Peers",
            "AutomationPeer",
        )
        .unwrap();
        assert!(automation_peer.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::ProtectedComposition && !constructor.is_public()
        }));
        assert!(
            automation_peer
                .overridable_interfaces
                .iter()
                .any(|interface| interface.name == "IAutomationPeerOverrides")
        );
        let known_types = HashSet::from(["AutomationPeer".to_string()]);
        let py = crate::codegen::winrt::python::generate_class(
            &automation_peer,
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
        );
        let pyi = crate::codegen::python_stub::generate_class_stub(
            &automation_peer,
            &known_types,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(py.contains("AutomationPeer cannot be constructed directly"));
        assert!(py.contains("def _from_native(cls, obj: DynWinRTValue):"));
        assert!(pyi.contains("def __init__(self, _not_constructible: NoReturn) -> None: ..."));
    }

    #[test]
    fn test_winui_parameterized_default_interface_when_metadata_is_available() {
        let Some(winui_winmd) = find_winui_winmd() else {
            eprintln!("WinUI metadata not installed; skipping collection metadata test");
            return;
        };
        let winmd_paths = format!("{};{}", WINDOWS_WINMD, winui_winmd.display());
        let collection = parse_class(
            &winmd_paths,
            "Microsoft.UI.Xaml.Controls",
            "RowDefinitionCollection",
        )
        .unwrap();
        let interface = collection.default_interface.as_ref().unwrap();
        assert_eq!(interface.name, "IVector_RowDefinition");
        assert!(interface.generic_piid.is_some());
        assert!(
            interface
                .methods
                .iter()
                .any(|method| method.name == "Append")
        );

        let ui_elements = parse_class(
            &winmd_paths,
            "Microsoft.UI.Xaml.Controls",
            "UIElementCollection",
        )
        .unwrap();
        let default_interface = ui_elements.default_interface.as_ref().unwrap();
        assert_eq!(default_interface.name, "IVector_UIElement");
        assert!(default_interface.generic_piid.is_some());
        assert!(
            ui_elements
                .required_interfaces
                .iter()
                .any(|interface| interface.name == "IUIElementCollection")
        );
    }

    fn find_winui_winmd() -> Option<std::path::PathBuf> {
        if let Some(path) = std::env::var_os("DYNWINRT_WINUI_WINMD") {
            let path = std::path::PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }

        let packages = std::path::PathBuf::from(std::env::var_os("USERPROFILE")?)
            .join(".winapp")
            .join("packages");
        let mut candidates: Vec<_> = std::fs::read_dir(packages)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Microsoft.WindowsAppSDK.WinUI.")
            })
            .map(|entry| {
                entry
                    .path()
                    .join("metadata")
                    .join("Microsoft.UI.Xaml.winmd")
            })
            .filter(|path| path.is_file())
            .collect();
        candidates.sort();
        candidates.pop()
    }

    #[test]
    fn test_httpclient_overloads_disambiguated() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Web.Http", "HttpClient").unwrap();
        let default_iface = class.default_interface.as_ref().unwrap();
        let names: Vec<&str> = default_iface
            .methods
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        // Should have GetWithOptionAsync, not duplicate GetAsync
        assert!(names.contains(&"GetWithOptionAsync"));
        assert!(names.contains(&"SendRequestWithOptionAsync"));
    }

    #[test]
    fn test_datawriter_store_async_maps_to_async_operation() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Storage.Streams", "DataWriter").unwrap();
        let default_iface = class.default_interface.as_ref().unwrap();
        let store_async = default_iface
            .methods
            .iter()
            .find(|m| m.name == "StoreAsync")
            .unwrap();
        assert_eq!(
            store_async.return_type,
            Some(TypeMeta::AsyncOperation(Box::new(TypeMeta::U32)))
        );
    }

    #[test]
    fn test_datareader_load_async_maps_to_async_operation() {
        let class = parse_class(WINDOWS_WINMD, "Windows.Storage.Streams", "DataReader").unwrap();
        let default_iface = class.default_interface.as_ref().unwrap();
        let load_async = default_iface
            .methods
            .iter()
            .find(|m| m.name == "LoadAsync")
            .unwrap();
        assert_eq!(
            load_async.return_type,
            Some(TypeMeta::AsyncOperation(Box::new(TypeMeta::U32)))
        );
    }
}

#[cfg(test)]
mod iface_tests {
    use std::collections::HashSet;

    use crate::types::TypeMeta;
    use windows_metadata::reader;
    const WINDOWS_WINMD: &str =
        r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

    #[test]
    fn debug_ihttpcontent() {
        let index = reader::Index::read(WINDOWS_WINMD).unwrap();
        let def = index.expect("Windows.Web.Http", "IHttpContent");
        // skip category
        println!("IID: {}", super::extract_iid(&def));
        for (i, m) in def.methods().enumerate() {
            println!("  [{}] {}", i, m.name());
        }
    }

    #[test]
    fn debug_ivectorview_lookup() {
        let index = reader::Index::read(WINDOWS_WINMD).unwrap();
        // winmd stores parameterized types without arity suffix
        let without_arity = index
            .get("Windows.Foundation.Collections", "IVectorView")
            .next();
        assert!(
            without_arity.is_some(),
            "IVectorView should be found in Windows.winmd"
        );
        // with arity suffix is NOT found
        let with_arity = index
            .get("Windows.Foundation.Collections", "IVectorView`1")
            .next();
        assert!(
            with_arity.is_none(),
            "IVectorView`1 should NOT be found (winmd uses name without arity)"
        );
    }

    #[test]
    fn all_well_known_piids_found_in_winmd() {
        let index = reader::Index::read(WINDOWS_WINMD).unwrap();
        let cases = [
            ("Windows.Foundation", "IReference"),
            ("Windows.Foundation", "TypedEventHandler"),
            ("Windows.Foundation", "EventHandler"),
            ("Windows.Foundation.Collections", "IIterable"),
            ("Windows.Foundation.Collections", "IIterator"),
            ("Windows.Foundation.Collections", "IVectorView"),
            ("Windows.Foundation.Collections", "IVector"),
            ("Windows.Foundation.Collections", "IMapView"),
            ("Windows.Foundation.Collections", "IMap"),
            ("Windows.Foundation.Collections", "IKeyValuePair"),
            ("Windows.Foundation.Collections", "IObservableVector"),
            ("Windows.Foundation.Collections", "IObservableMap"),
        ];
        for (ns, name) in &cases {
            let def = index.get(ns, name).next();
            assert!(
                def.is_some(),
                "{}.{} should be found in Windows.winmd",
                ns,
                name
            );
            let iid = super::extract_iid(&def.unwrap());
            assert!(
                !iid.is_empty(),
                "{}.{} should have a GuidAttribute (PIID)",
                ns,
                name
            );
        }
    }

    #[test]
    fn unresolved_parameterized_interface_types_fail_closed() {
        let index = reader::Index::read(WINDOWS_WINMD).unwrap();
        let open_vector = windows_metadata::Type::Name(windows_metadata::TypeName {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IVector`1".into(),
            generics: vec![windows_metadata::Type::Generic(0)],
        });
        assert!(super::parse_interface_type(&index, &open_vector).is_none());

        let unresolved_arg = windows_metadata::Type::Name(windows_metadata::TypeName {
            namespace: "Windows.Foundation.Collections".into(),
            name: "IVector`1".into(),
            generics: vec![windows_metadata::Type::named("Missing.Metadata", "Unknown")],
        });
        assert!(super::parse_interface_type(&index, &unresolved_arg).is_none());
    }

    #[test]
    fn observable_vector_discovers_mutable_vector_dependency() {
        let interface = super::InterfaceMeta {
            name: "IObservableVector_ICommandBarElement".into(),
            namespace: "Windows.Foundation.Collections".into(),
            generic_piid: Some(super::PIID_IOBSERVABLE_VECTOR.into()),
            generic_args: vec![TypeMeta::Interface {
                namespace: "Microsoft.UI.Xaml.Controls".into(),
                name: "ICommandBarElement".into(),
                iid: "f8eb20b4-373e-5327-9942-66a1ea21f5f9".into(),
            }],
            ..Default::default()
        };
        let mut named = Vec::new();
        let mut parameterized = Vec::new();

        super::collect_all_refs_from_interfaces(
            &[interface],
            &HashSet::new(),
            &mut named,
            &mut parameterized,
            false,
        );

        assert!(named.is_empty());
        assert_eq!(parameterized.len(), 1);
        assert!(matches!(
            &parameterized[0],
            TypeMeta::Parameterized {
                namespace,
                name,
                piid,
                args,
            } if namespace == super::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE
                && name == "IVector"
                && piid == super::PIID_IVECTOR
                && args == &vec![TypeMeta::Interface {
                    namespace: "Microsoft.UI.Xaml.Controls".into(),
                    name: "ICommandBarElement".into(),
                    iid: "f8eb20b4-373e-5327-9942-66a1ea21f5f9".into(),
                }]
        ));
    }

    #[test]
    fn observable_vector_dependency_is_resolved_for_emission() {
        let observable = super::InterfaceMeta {
            name: "IObservableVector_String".into(),
            namespace: "Windows.Foundation.Collections".into(),
            generic_piid: Some(super::PIID_IOBSERVABLE_VECTOR.into()),
            generic_args: vec![TypeMeta::String],
            ..Default::default()
        };

        let dependencies = super::resolve_dependencies(WINDOWS_WINMD, &[], &[observable], &[]);

        assert!(
            dependencies
                .interfaces
                .iter()
                .any(|interface| interface.name == "IVector_String")
        );
    }
}
