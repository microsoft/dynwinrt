// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;
use std::path::Path;

use windows_metadata::{HasAttributes, reader};

use crate::types::{EnumMember, TypeKind, TypeMeta, TypeRef};

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
    pub default_interface: Option<InterfaceMeta>,
    /// Supplemental interfaces, including parameterized and versioned interfaces.
    pub required_interfaces: Vec<InterfaceMeta>,
    pub factory_interfaces: Vec<InterfaceMeta>,
    pub static_interfaces: Vec<InterfaceMeta>,
    pub has_default_constructor: bool,
    pub constructors: Vec<ConstructorMeta>,
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

/// Look up a WinRT runtime class in `winmd_paths` by simple (unqualified) name
/// and return `(full_namespace, default_interface_name, default_interface_iid)`.
///
/// Used by the classic-COM `*Interop` codegen to auto-resolve the target type
/// of a `GetForWindow(HWND, REFIID, out void**)` method: the interface prefix
/// `I` and suffix `Interop` are stripped, and this helper finds the runtime
/// class of that name in `Windows.winmd` (or any provided WinRT metadata).
///
/// Returns `None` when the winmd is unreadable, when no such class exists,
/// when the class isn't marked `[WindowsRuntime]`, or when it has no default
/// interface with a resolvable IID.
pub fn find_runtime_class_default_iid(
    winmd_paths: &str,
    simple_name: &str,
) -> Option<(String, String, String)> {
    let index = load_index(winmd_paths)?;
    // Collect *all* runtime classes with this simple name so we can detect
    // cross-namespace collisions (e.g. two runtime classes both called
    // `SomeThing` in different namespaces). Returning the first match blindly
    // would silently drive interop codegen with the wrong default-interface
    // IID → wrappers that call `GetForWindow(riid=…, ppv)` for a different
    // interface than the caller expects.
    let mut found: Option<(String, String, String)> = None;
    let mut collisions: Vec<(String, String, String)> = Vec::new();
    for def in index.all() {
        if def.name() != simple_name {
            continue;
        }
        // A WinRT runtime class extends System.Object AND carries the
        // WindowsRuntime flag on its type. Interfaces extend nothing;
        // classes extend Object/etc. We filter to actual runtime classes.
        if !def.flags().contains(windows_metadata::TypeAttributes::WindowsRuntime) {
            continue;
        }
        // Must be a class (not interface/enum/struct).
        if def.flags().contains(windows_metadata::TypeAttributes::Interface) {
            continue;
        }
        let namespace = def.namespace().to_string();
        // Look for the default interface via DefaultAttribute.
        for iface_impl in def.interface_impls() {
            if !iface_impl.has_attribute("DefaultAttribute") {
                continue;
            }
            let iface_ty = iface_impl.interface(&[]);
            let windows_metadata::Type::Name(tn) = &iface_ty else { continue };
            // Resolve concrete (non-generic) interface's IID from its TypeDef.
            if !tn.generics.is_empty() {
                // Skip generic default interfaces — interop projections don't
                // hit them in practice, and the parameterized IID would need
                // separate computation.
                continue;
            }
            let Some(iface_def) = index.get(&tn.namespace, &tn.name).next() else {
                // Unreadable/missing TypeDef for this DefaultAttribute impl
                // — skip *this* candidate rather than aborting the whole
                // lookup. Other matching runtime classes (or other
                // DefaultAttribute impls on the same class) can still resolve
                // successfully.
                continue;
            };
            let iid = extract_iid(&iface_def);
            if iid.is_empty() {
                continue;
            }
            let candidate = (namespace.clone(), tn.name.clone(), iid);
            match &found {
                None => found = Some(candidate),
                Some(prev) if prev == &candidate => {
                    // Exact duplicate — same namespace + same IID means the
                    // same TypeDef, harmless.
                }
                Some(_) => collisions.push(candidate),
            }
            break; // stop looking at this class's other interface_impls
        }
    }
    if !collisions.is_empty() {
        let mut all = vec![found.clone().unwrap()];
        all.extend(collisions);
        eprintln!(
            "warning: find_runtime_class_default_iid({}): multiple runtime classes with this simple name resolve to distinct default IIDs — refusing to guess. Candidates: {:?}",
            simple_name, all
        );
        return None;
    }
    found
}

/// Discover the NEWEST installed Windows SDK `Windows.winmd` by enumerating the
/// versioned directories under `C:\Program Files (x86)\Windows Kits\10\UnionMetadata`
/// and picking the highest version that actually contains a readable file.
///
/// Used as a portable fallback by the classic-COM interop code generator when
/// the winmds explicitly loaded for generation don't contain the projected
/// WinRT runtime class. Returns `None` when no SDK is installed.
pub fn discover_newest_windows_winmd() -> Option<String> {
    let base = std::path::Path::new(r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata");
    if !base.exists() {
        return None;
    }
    let mut versions: Vec<String> = std::fs::read_dir(base)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("10."))
        .collect();
    // Sort by dotted-version tuple so `10.0.26100.0` beats `10.0.19041.0`.
    versions.sort_by(|a, b| {
        let pa: Vec<u64> = a.split('.').filter_map(|s| s.parse().ok()).collect();
        let pb: Vec<u64> = b.split('.').filter_map(|s| s.parse().ok()).collect();
        pa.cmp(&pb)
    });
    for version in versions.iter().rev() {
        let winmd_path = base.join(version).join("Windows.winmd");
        if winmd_path.exists() {
            return Some(winmd_path.to_string_lossy().to_string());
        }
    }
    None
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

    let mut dep_classes: Vec<ClassMeta> = Vec::new();
    let mut dep_interfaces: Vec<InterfaceMeta> = Vec::new();
    let mut dep_enums: Vec<TypeMeta> = Vec::new();

    // Seed the worklist from initial types
    let mut worklist: Vec<TypeRef> = Vec::new();
    let mut param_worklist: Vec<TypeMeta> = Vec::new();
    collect_all_refs_from_classes(classes, &known, &mut worklist, &mut param_worklist);
    collect_all_refs_from_interfaces(
        existing_interfaces,
        &known,
        &mut worklist,
        &mut param_worklist,
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
            if known.contains(&r.name) {
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
                let concrete_name = make_parameterized_name(name, args);
                if known.contains(&concrete_name) {
                    continue;
                }
                known.insert(concrete_name.clone());

                if let Some(iface) = parse_parameterized_interface(
                    &index,
                    namespace,
                    name,
                    &concrete_name,
                    piid,
                    args,
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
        collect_all_refs_from_classes(&new_classes, &known, &mut worklist, &mut param_worklist);
        collect_all_refs_from_interfaces(
            &new_interfaces,
            &known,
            &mut worklist,
            &mut param_worklist,
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
    known: &HashSet<String>,
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
    for r in named {
        if !known.contains(&r.name) {
            named_out.push(r);
        }
    }
    for r in parameterized {
        if let TypeMeta::Parameterized { name, args, .. } = &r {
            let concrete = make_parameterized_name(name, args);
            if !known.contains(&concrete) {
                param_out.push(r);
            }
        }
    }
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
) {
    for c in classes {
        for iface in c.all_interfaces() {
            collect_all_refs_from_methods(&iface.methods, known, named_out, param_out);
        }
        // Required interfaces themselves may need to be resolved
        for iface in &c.required_interfaces {
            if iface.generic_piid.is_none()
                && !iface.name.is_empty()
                && !known.contains(&iface.name)
            {
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
) {
    for i in interfaces {
        collect_all_refs_from_methods(&i.methods, known, named_out, param_out);
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
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .extension()
                        .map_or(false, |ext| ext.eq_ignore_ascii_case("winmd"))
                    {
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
    }

    all_paths.join(";")
}

fn load_index(winmd_paths: &str) -> Option<reader::Index> {
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

    let mut default_interface = None;
    let mut factory_interfaces = Vec::new();
    let mut static_interfaces = Vec::new();
    let mut generic_required_interfaces = Vec::new();
    let mut has_default_constructor = false;
    let mut constructors = Vec::new();

    // 1. Find default interface and collect all required interfaces
    let mut required_iface_names: Vec<(String, String)> = Vec::new();
    for iface_impl in def.interface_impls() {
        let iface_ty = iface_impl.interface(&[]);
        let (iface_ns, iface_name) = match &iface_ty {
            windows_metadata::Type::Name(tn) => (tn.namespace.clone(), tn.name.clone()),
            _ => continue,
        };

        if iface_impl.has_attribute("DefaultAttribute") {
            if let Some(iface_meta) = parse_interface_type(index, &iface_ty) {
                default_interface = Some(iface_meta);
            }
        } else if matches!(
            &iface_ty,
            windows_metadata::Type::Name(type_name) if !type_name.generics.is_empty()
        ) {
            if let Some(iface_meta) = parse_interface_type(index, &iface_ty) {
                generic_required_interfaces.push(iface_meta);
            }
        } else {
            // Non-default required interface (e.g. ILanguageModel2, versioned interfaces)
            required_iface_names.push((iface_ns, iface_name));
        }
    }

    // 1a. Walk ancestor classes and inherit their interfaces. In WinRT the runtime
    // reaches parent-class members via QI on the child instance — but each parent's
    // interface_impls are only listed on the parent's TypeDef, not repeated on the
    // child. So we walk def.extends() ourselves and flatten every non-generic
    // ancestor interface onto this class's required_interfaces. Without this,
    // Button.background (from Control), ScrollViewer.content (from ContentControl),
    // and every other inherited member is silently absent from the wrapper class.
    let default_iface_key = default_interface
        .as_ref()
        .map(|d| (d.namespace.clone(), d.name.clone()));
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
            if let windows_metadata::Type::Name(tn) = &iface_ty {
                // Skip generic instantiations — parse_interface can't substitute
                // args here, and their flavor-specific files (IVector_T.js etc.)
                // are already emitted separately for explicit casts.
                if !tn.generics.is_empty() {
                    continue;
                }
                let key = (tn.namespace.clone(), tn.name.clone());
                if default_iface_key.as_ref() == Some(&key) {
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
                required_interfaces.push(req_iface);
            }
        }
    }
    required_interfaces.extend(generic_required_interfaces);

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
                _ => {
                    has_default_constructor = true;
                    push_unique_constructor(
                        &mut constructors,
                        ConstructorMeta {
                            kind: ConstructorKind::DefaultActivation,
                            factory_interface: None,
                        },
                    );
                }
            }
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
        default_interface,
        required_interfaces,
        factory_interfaces,
        static_interfaces,
        has_default_constructor,
        constructors,
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
    parse_interface_methods(index, &def, name, namespace, &iid, &[], 6)
}

// ==========================================================================
// Classic-COM (option A) support
// ==========================================================================

/// Rich metadata for a classic-COM interface discovered by walking the
/// `interface_impls()` chain. The `interface.methods` list is the *flattened*
/// method set (own + all inherited, excluding IUnknown's QI/AddRef/Release)
/// with absolute vtable indices — so the codegen renderer never has to think
/// about inheritance again.
///
/// This is entirely separate from the WinRT `parse_class`/`parse_interface`
/// path so we do not risk regressing IInspectable-based generation.
#[derive(Debug, Clone)]
pub struct ComInterfaceMeta {
    /// Flattened interface with own + inherited methods, absolute vtable indices.
    pub interface: InterfaceMeta,
    /// The vtable index of the first user method in the flattened list:
    /// - `3` for any IUnknown-rooted interface (QI/AddRef/Release occupy 0..2).
    /// - `6` for any IInspectable-rooted interface (WinRT projection layout).
    pub base_offset: usize,
    /// `true` iff the inheritance chain terminates at IUnknown.
    /// `false` iff it terminates at IInspectable (WinRT-projected classic COM).
    pub is_iunknown_rooted: bool,
    /// Ordered list of base names from immediate parent up to the root
    /// (e.g. `["ITaskbarList2", "ITaskbarList", "IUnknown"]`).
    pub base_chain: Vec<String>,
    /// If a Win32 coclass matches this interface, the coclass GUID (=CLSID).
    pub coclass_clsid: Option<String>,
    /// The name of the discovered coclass, e.g. `"TaskbarList"`.
    pub coclass_name: Option<String>,
    /// The absolute vtable slot of this leaf interface's first *own* method
    /// (i.e. the number of methods contributed by all bases plus the root
    /// offset). Renderer helper — not core metadata.
    pub own_methods_start: usize,
    /// Enum types referenced by this interface's methods (directly resolved
    /// during metadata parsing so codegen can emit them without a second
    /// resolve_dependencies pass over the whole namespace).
    pub referenced_enums: Vec<TypeMeta>,
}

/// Parse a classic-COM interface (IUnknown-rooted) by name, walking the
/// `interface_impls()` chain to compute absolute vtable slots and flatten
/// inherited methods.
///
/// Returns `None` if the type isn't found. Unlike `parse_interface`, this
/// function also handles interfaces that inherit from other classic-COM
/// interfaces via `interface_impls()` (the Windows.Win32 winmd doesn't
/// use `[NativeInheritance]` attributes — it uses actual InterfaceImpl rows).
pub fn parse_com_interface(
    winmd_paths: &str,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let index = load_index(winmd_paths)?;
    parse_com_interface_from_index(&index, namespace, name)
}

fn parse_com_interface_from_index(
    index: &reader::Index,
    namespace: &str,
    name: &str,
) -> Option<ComInterfaceMeta> {
    let def = index.get(namespace, name).next()?;

    // Guard: refuse to treat non-interface TypeDefs (WinRT runtime classes,
    // enums, structs, delegates) as classic-COM interfaces. Without this,
    // routing a name that happens to resolve to e.g. a `*Interop` runtime
    // class through this path would walk its `interface_impls()` and produce
    // a bogus flattened method list. Callers see `None` and can fall through
    // to the correct WinRT code path in `main.rs`.
    if !def
        .flags()
        .contains(windows_metadata::TypeAttributes::Interface)
    {
        return None;
    }

    // Walk the interface_impls chain: for each base, collect its own method
    // count, and stop at IUnknown or IInspectable. Traverse from the leaf up
    // so we can compute cumulative offsets.
    let mut base_chain: Vec<(String, String, usize)> = Vec::new(); // (ns, name, own_method_count)
    let mut cur_ns = namespace.to_string();
    let mut cur_name = name.to_string();
    let mut is_iunknown_rooted = false;
    // Explicit-termination flag: set only when the walk reaches a well-known
    // COM/WinRT root (IUnknown or IInspectable). If we exit the loop without
    // this being set — malformed/incomplete winmd, missing `interface_impls`,
    // or a depth-limit overrun — the offset-3 vs. offset-6 decision below
    // would be guesswork. In that case we return None rather than emit code
    // with silently-wrong vtable slots.
    let mut terminated_at_known_root = false;

    // Walk up to 32 levels deep as a safety limit (real chains are 3-4 deep).
    for _ in 0..32 {
        let cur_def = match index.get(&cur_ns, &cur_name).next() {
            Some(d) => d,
            None => break,
        };
        // Find the (single) base via interface_impls.
        let base_ii = cur_def.interface_impls().next();
        let base_type = base_ii.map(|ii| ii.interface(&[]));
        let base = match base_type {
            Some(windows_metadata::Type::Name(tn)) => (tn.namespace.clone(), tn.name.clone()),
            _ => break,
        };
        // Terminate at IUnknown or IInspectable.
        if base.1 == "IUnknown" {
            is_iunknown_rooted = true;
            terminated_at_known_root = true;
            base_chain.push(("Windows.Win32.System.Com".to_string(), "IUnknown".to_string(), 0));
            break;
        }
        if base.1 == "IInspectable" {
            terminated_at_known_root = true;
            base_chain.push(("Windows.Foundation".to_string(), "IInspectable".to_string(), 0));
            break;
        }
        // Otherwise this base is a real classic-COM interface — count its methods.
        let base_def = match index.get(&base.0, &base.1).next() {
            Some(d) => d,
            None => break,
        };
        let own_count = base_def.methods().count();
        base_chain.push((base.0.clone(), base.1.clone(), own_count));
        cur_ns = base.0;
        cur_name = base.1;
    }

    // Refuse to guess a root offset when the walk didn't terminate cleanly:
    // an unknown-shape base chain would produce wrong absolute vtable slots
    // and therefore wrong method dispatch. Callers see `None` and can log /
    // surface a clearer error than a silent mis-generation.
    if !terminated_at_known_root {
        eprintln!(
            "warning: base-chain walk for {}.{} did not terminate at IUnknown or IInspectable — refusing to guess vtable root offset",
            namespace, name
        );
        return None;
    }

    // Compute root offset (3 for IUnknown, 6 for IInspectable) and the
    // absolute vtable slot at which THIS leaf interface's own methods start.
    let root_offset = if is_iunknown_rooted { 3 } else { 6 };
    let intermediate_methods: usize = base_chain
        .iter()
        .filter(|(_, name, _)| name != "IUnknown" && name != "IInspectable")
        .map(|(_, _, c)| *c)
        .sum();
    let own_methods_start = root_offset + intermediate_methods;

    // Build a flattened method list: iterate the chain top-down (from root
    // toward the leaf, i.e. reverse `base_chain`), assigning consecutive
    // vtable slots. Base interfaces contribute their own methods first.
    //
    // Vtable layout: [IUnknown 0..2] [base_N 3..] [base_{N-1} ...] ... [leaf's own].
    let mut methods: Vec<MethodMeta> = Vec::new();

    let mut slot_cursor = root_offset;
    // Reverse: iterate from the outermost base (closest to IUnknown) down
    // toward the immediate parent.
    let mut chain_top_down: Vec<&(String, String, usize)> = base_chain.iter().rev().collect();
    // Filter out the root (IUnknown/IInspectable, which contribute 0 own methods to the vtable
    // *from the user-visible perspective* — their slots are already counted in `root_offset`).
    chain_top_down.retain(|(_, n, _)| n != "IUnknown" && n != "IInspectable");

    for (base_ns, base_name, _own_count) in chain_top_down {
        match parse_interface_with_offset(index, base_ns, base_name, slot_cursor) {
            Some(base_iface) => {
                slot_cursor += base_iface.methods.len();
                methods.extend(base_iface.methods);
            }
            None => {
                // Fail loud: if we can't parse a base interface's methods,
                // the flattened method list would be missing entries and the
                // leaf's absolute vtable indices would be wrong. In release
                // the `debug_assert_eq!` below is compiled out, so we'd
                // silently emit wrappers that dispatch to the wrong COM
                // methods. Return None so callers surface a clear error.
                eprintln!(
                    "warning: could not parse base classic-COM interface {}.{} — refusing to emit {}.{} with a truncated vtable",
                    base_ns, base_name, namespace, name
                );
                return None;
            }
        }
    }
    // Assert the invariant that we lined up correctly.
    debug_assert_eq!(slot_cursor, own_methods_start,
        "vtable cursor {} != computed own_methods_start {}", slot_cursor, own_methods_start);

    // Now the leaf's own methods
    let iid = extract_iid(&def);
    let own = parse_interface_methods(index, &def, name, namespace, &iid, &[], slot_cursor)?;
    methods.extend(own.methods);

    // Build a mostly-standard InterfaceMeta wrapping the flattened method list.
    let interface = InterfaceMeta {
        name: name.to_string(),
        namespace: namespace.to_string(),
        iid: iid.clone(),
        methods,
        generic_piid: None,
        generic_args: Vec::new(),
        doc: None,
        deprecated: None,
    };

    // Discover coclass CLSID. Heuristic: strip leading `I` from the interface
    // name, then strip trailing digits (e.g. `ITaskbarList3` → `TaskbarList3`
    // → `TaskbarList`). Return the first coclass matching either variant that
    // has a GuidAttribute AND `extends System.ValueType`.
    let mut candidates: Vec<String> = Vec::new();
    if let Some(stripped) = name.strip_prefix('I') {
        candidates.push(stripped.to_string());
        // Also try trimming trailing digits: TaskbarList3 → TaskbarList
        let trimmed: String = stripped
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .to_string();
        if trimmed != stripped {
            candidates.push(trimmed);
        }
    }
    let mut coclass_clsid: Option<String> = None;
    let mut coclass_name: Option<String> = None;
    for cand in &candidates {
        if let Some(cc_def) = index.get(namespace, cand).next() {
            let ext = cc_def.extends();
            let is_coclass_shape = matches!(
                ext.map(|e| (e.namespace().to_string(), e.name().to_string())),
                Some((ref ns, ref n)) if ns == "System" && n == "ValueType"
            );
            if !is_coclass_shape {
                continue;
            }
            let cc_iid = extract_iid(&cc_def);
            if !cc_iid.is_empty() {
                coclass_clsid = Some(cc_iid);
                coclass_name = Some(cand.clone());
                break;
            }
        }
    }

    // Collect enum types referenced in methods' parameters (direct only).
    let mut referenced_enums: Vec<TypeMeta> = Vec::new();
    let mut seen_enum_names: HashSet<String> = HashSet::new();
    for m in &interface.methods {
        for p in &m.params {
            if let TypeMeta::Enum { .. } = &p.typ {
                if let TypeMeta::Enum { name: en, .. } = &p.typ {
                    if seen_enum_names.insert(en.clone()) {
                        referenced_enums.push(p.typ.clone());
                    }
                }
            }
        }
    }

    Some(ComInterfaceMeta {
        interface,
        base_offset: root_offset,
        is_iunknown_rooted,
        base_chain: base_chain.into_iter().map(|(_, n, _)| n).collect(),
        coclass_clsid,
        coclass_name,
        own_methods_start,
        referenced_enums,
    })
}

// ---------------------------------------------------------------------------
// Flat-Win32 [DllImport] method discovery
// ---------------------------------------------------------------------------

/// A single flat-Win32 export parameter with its ABI shape preserved.
///
/// Unlike WinRT `ParamMeta`, this keeps raw pointer types (`PtrMut`/`PtrConst`)
/// distinct from opaque handles so the flat emitter can project pointer-based
/// out-params (e.g. `PHKEY`) as JS return values.
#[derive(Debug, Clone)]
pub struct FlatParamMeta {
    pub name: String,
    pub abi: FlatAbiType,
    pub direction: FlatDirection,
}

/// Direction of a flat-Win32 parameter, computed from `ParamAttributes`
/// (`In=0x01`, `Out=0x02`; a pointer that's both is `InOut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatDirection {
    In,
    Out,
    InOut,
}

/// A restricted ABI type space for flat-Win32 exports.
///
/// This is intentionally SEPARATE from `TypeMeta`: `map_winmd_type_with_generics`
/// collapses pointer types (`PtrMut`, `PtrConst`) to `TypeMeta::Object`, losing
/// the pointee direction we need to project out-params. Flat marshalling also
/// treats Win32 typedef wrappers (HKEY, PWSTR, LSTATUS, WIN32_ERROR) as first-
/// class shapes so the emitter can pick a natural JS surface (string, bigint,
/// enum-number) per shape.
#[derive(Debug, Clone, PartialEq)]
pub enum FlatAbiType {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    /// Wide-character UCS-2 code unit.
    Char16,
    /// Opaque pointer of any pointee type (raw `void*`).
    Ptr,
    /// A pointer with a KNOWN pointee ABI type. Used for out/inout scalar
    /// slots we can project (e.g. `PtrMut(HKEY)` → out HKEY value;
    /// `PtrMut(U32)` [InOut] → in-out DWORD).
    PtrTo(Box<FlatAbiType>),
    /// PWSTR / PCWSTR / LPCWSTR: pointer to a UTF-16 string. The flat
    /// emitter models these as *read-only* string inputs: the wrapper
    /// builds a NUL-terminated UTF-16 `Buffer` on demand from a
    /// `string | null` argument. This is correct for `PCWSTR` / `LPCWSTR`
    /// (Win32's const-form pointer-to-CH); for the mutable `PWSTR` form
    /// used as an OUT/INOUT string buffer, this projection would be too
    /// narrow (the caller would need a pre-sized `Buffer` — that case
    /// falls through the ``[out]``/``[in,out]`` param classification in
    /// `flat.rs` and is currently marshalled via ``pointer(<user buf>)``
    /// rather than via the string-input path).
    PWStr,
    /// PSTR / PCSTR / LPCSTR: pointer to an 8-bit / ANSI / UTF-8 string.
    /// Same read-only string-input projection as `PWStr` above; the
    /// mutable `PSTR` output form flows through the `Buffer` marshalling
    /// path in `flat.rs`.
    PStr,
    /// A Win32 opaque handle struct (single `Value` field with a pointer
    /// or integer shape). Natural surface is `bigint | number` — see the
    /// handle typedef doc in `codegen/flat.rs`. `Buffer` is intentionally
    /// NOT a valid input shape because `DynWinRtValue.pointer(Buffer)`
    /// uses the buffer's own base address rather than the pointer bits
    /// contained in it, which would be misinterpreted as a pointer to
    /// the handle (an address-of-address) instead of the handle itself.
    Handle {
        namespace: String,
        name: String,
    },
    /// Win32 BOOL — 32-bit integer at the ABI, `boolean` on the surface.
    Bool32,
    /// A named `[Flags]` or plain enum from the winmd. `underlying` is the
    /// storage type (usually `U32`). The surface projects as `number`.
    Enum {
        namespace: String,
        name: String,
        underlying: Box<FlatAbiType>,
        members: Vec<crate::types::EnumMember>,
    },
    /// Anything we cannot classify precisely. Emitted as an opaque pointer at
    /// the ABI; the surface will require the caller to pass a `Buffer|bigint`.
    Unknown,
}

/// A single flat-Win32 export from an `Apis`-class static method.
#[derive(Debug, Clone)]
pub struct FlatMethodMeta {
    /// PascalCase name of the method in the winmd (e.g. `RegOpenKeyExW`).
    pub name: String,
    /// DLL name from the `[DllImport]` module ref (e.g. `ADVAPI32.dll`).
    pub dll: String,
    /// Entry-point name from `ImplMap.import_name` — usually identical to
    /// `name`, but can differ for aliased exports.
    pub entry_point: String,
    /// Return type at the ABI.
    pub return_type: FlatAbiType,
    /// Ordered parameters, with `[in]` / `[out]` / `[in,out]` direction
    /// recovered from `ParamAttributes`.
    pub params: Vec<FlatParamMeta>,
    /// True when the return type is a known Win32 status typedef (HRESULT,
    /// NTSTATUS, LSTATUS) or a WIN32_ERROR-family enum. Callers should
    /// project the return as a numeric `.status` field so consumers can
    /// branch on ERROR_SUCCESS / ERROR_FILE_NOT_FOUND / etc. FALSE for
    /// plain I32/U32 returns (e.g. `GetCurrentProcessId -> u32`,
    /// `MulDiv -> i32`) — those are real integer values and must be
    /// projected as `.result` rather than mis-labelled as status codes.
    pub return_is_status: bool,
}

/// A container class whose static methods are all `[DllImport]` exports —
/// the `Apis` class pattern used throughout `Windows.Win32.winmd`.
#[derive(Debug, Clone)]
pub struct FlatApisMeta {
    pub namespace: String,
    pub class_name: String,
    pub methods: Vec<FlatMethodMeta>,
    /// Distinct enum types referenced by any parameter or return type. The
    /// generator emits a per-enum sibling `.js`/`.d.ts` for each one.
    pub referenced_enums: Vec<TypeMeta>,
}

/// Parse a flat-Win32 `Apis`-shaped class (a container of `[DllImport]` static
/// methods) from the winmd. Returns `None` when the class does not exist,
/// when it has no DllImport methods (i.e. it's actually a WinRT class), or
/// when it fails to parse.
pub fn parse_flat_apis(
    winmd_paths: &str,
    namespace: &str,
    class_name: &str,
) -> Option<FlatApisMeta> {
    let index = load_index(winmd_paths)?;
    parse_flat_apis_from_index(&index, namespace, class_name)
}

/// True when the RAW winmd return type is a known Win32 status typedef —
/// HRESULT / NTSTATUS / LSTATUS in `Windows.Win32.Foundation`. Preserves
/// typedef intent that would otherwise be lost by `map_flat_type` collapsing
/// them all to `FlatAbiType::I32`, so the emitter can distinguish real
/// status codes (project as `.status`) from integer-return APIs like
/// `MulDiv` or `GetCurrentProcessId` (project as `.result`).
fn is_status_return_type(ty: &windows_metadata::Type) -> bool {
    use windows_metadata::Type;
    match ty {
        Type::Name(tn) => {
            tn.namespace == "Windows.Win32.Foundation"
                && matches!(tn.name.as_ref(), "HRESULT" | "NTSTATUS" | "LSTATUS")
        }
        _ => false,
    }
}

/// True when the mapped `FlatAbiType` is a WIN32_ERROR-family enum whose
/// underlying storage is a 32-bit integer. The Win32 winmd exposes many
/// error/status typedefs as `[Flags]`-style enums (e.g. `WIN32_ERROR`,
/// `NTSTATUS`-like enums whose name ends with `STATUS`) — those still count
/// as status codes for return-value projection.
fn is_status_return_enum(t: &FlatAbiType) -> bool {
    if let FlatAbiType::Enum {
        name, underlying, ..
    } = t
    {
        (name == "WIN32_ERROR" || name.ends_with("STATUS"))
            && matches!(**underlying, FlatAbiType::U32 | FlatAbiType::I32)
    } else {
        false
    }
}

fn parse_flat_apis_from_index(
    index: &reader::Index,
    namespace: &str,
    class_name: &str,
) -> Option<FlatApisMeta> {
    let def = index.get(namespace, class_name).next()?;

    // Determine target platform-pointer size. The Win32 winmd's PtrMut carries
    // an explicit size for fixed-size pointers, but its `usize` is only ever 1
    // for `void*`-shaped values. We always compile on 64-bit here so pointer
    // width = 8 bytes.

    let mut methods: Vec<FlatMethodMeta> = Vec::new();
    let mut referenced_enums: Vec<TypeMeta> = Vec::new();
    // Deduplicate referenced enums by (namespace, name) to avoid silently
    // dropping a distinct type with the same simple name from a different
    // namespace (e.g. `SomeNs.WIN32_ERROR` vs `Windows.Win32.Foundation
    // .WIN32_ERROR`). Keying by `name` alone would keep only the first-
    // seen variant and emit incorrect sibling files.
    let mut seen_enum_keys: HashSet<(String, String)> = HashSet::new();

    for m in def.methods() {
        let Some(imap) = m.impl_map() else {
            // Not a [DllImport] method — skip. (An Apis class may also have
            // constructor stubs; we intentionally ignore those.)
            continue;
        };
        // Skip .ctor (unlikely on Apis, but future-proof).
        if m.name() == ".ctor" || m.name() == ".cctor" {
            continue;
        }
        let dll = imap.import_scope().name().to_string();
        let entry_point = imap.import_name().to_string();

        let sig = m.signature(&[]);
        let return_type = map_flat_type(&sig.return_type, index, &mut |e| {
            collect_enum(e, &mut seen_enum_keys, &mut referenced_enums)
        });
        // Preserve typedef intent from the raw return Type: only project as
        // a `.status` numeric field when the return is a known Win32 status
        // typedef (HRESULT/NTSTATUS/LSTATUS) OR a WIN32_ERROR-family enum
        // after mapping. A plain I32/U32 return (e.g. `GetCurrentProcessId`,
        // `MulDiv`) is a real value, NOT a status code, and must project as
        // `{ result: number }` — see `render_method_js`.
        let return_is_status =
            is_status_return_type(&sig.return_type) || is_status_return_enum(&return_type);

        let param_defs: Vec<_> = m.params().filter(|p| p.sequence() > 0).collect();
        // Fail-loud on parameter/signature divergence. Silently truncating
        // to the shorter list would emit a wrapper with a fabricated
        // argument list, and a mismatched flat call is UB. Skip the whole
        // method (with a stderr warning) instead — the codegen surface then
        // simply lacks this export, which is far safer than a wrapper that
        // corrupts the callee's stack.
        if param_defs.len() != sig.types.len() {
            eprintln!(
                "warning: skipping {}.{}.{} — param count ({}) differs from signature type count ({}); metadata is inconsistent",
                namespace,
                class_name,
                m.name(),
                param_defs.len(),
                sig.types.len(),
            );
            continue;
        }
        let mut params: Vec<FlatParamMeta> = Vec::with_capacity(param_defs.len());
        for (i, pd) in param_defs.iter().enumerate() {
            let ty = &sig.types[i];
            let abi = map_flat_type(ty, index, &mut |e| {
                collect_enum(e, &mut seen_enum_keys, &mut referenced_enums)
            });
            let flags = pd.flags();
            let is_in = flags.contains(windows_metadata::ParamAttributes::In);
            let is_out = flags.contains(windows_metadata::ParamAttributes::Out);
            let direction = match (is_in, is_out) {
                (_, true) if is_in => FlatDirection::InOut,
                (_, true) => FlatDirection::Out,
                _ => FlatDirection::In,
            };
            params.push(FlatParamMeta {
                name: pd.name().to_string(),
                abi,
                direction,
            });
        }

        methods.push(FlatMethodMeta {
            name: m.name().to_string(),
            dll,
            entry_point,
            return_type,
            params,
            return_is_status,
        });
    }

    if methods.is_empty() {
        return None;
    }
    // Stable order: winmd row order is arbitrary. Sort by name so snapshots
    // are deterministic across metadata rewrites.
    methods.sort_by(|a, b| a.name.cmp(&b.name));
    referenced_enums.sort_by(|a, b| match (a, b) {
        (TypeMeta::Enum { name: an, .. }, TypeMeta::Enum { name: bn, .. }) => an.cmp(bn),
        _ => std::cmp::Ordering::Equal,
    });

    Some(FlatApisMeta {
        namespace: namespace.to_string(),
        class_name: class_name.to_string(),
        methods,
        referenced_enums,
    })
}

fn collect_enum(
    en: TypeMeta,
    seen: &mut HashSet<(String, String)>,
    sink: &mut Vec<TypeMeta>,
) {
    if let TypeMeta::Enum { namespace, name, .. } = &en {
        if seen.insert((namespace.clone(), name.clone())) {
            sink.push(en);
        }
    }
}

/// Map a `windows_metadata::Type` to a `FlatAbiType`, following `Windows.Win32`
/// typedef conventions (single-field structs with `NativeTypedefAttribute`
/// wrapping a primitive → the underlying primitive OR a Handle/String flavour
/// depending on the pointee).
fn map_flat_type(
    ty: &windows_metadata::Type,
    index: &reader::Index,
    enum_sink: &mut dyn FnMut(TypeMeta),
) -> FlatAbiType {
    use windows_metadata::Type;
    match ty {
        Type::Void => FlatAbiType::Void,
        Type::Bool => FlatAbiType::Bool,
        Type::Char => FlatAbiType::Char16,
        Type::I8 => FlatAbiType::I8,
        Type::U8 => FlatAbiType::U8,
        Type::I16 => FlatAbiType::I16,
        Type::U16 => FlatAbiType::U16,
        Type::I32 => FlatAbiType::I32,
        Type::U32 => FlatAbiType::U32,
        Type::I64 => FlatAbiType::I64,
        Type::U64 => FlatAbiType::U64,
        Type::F32 => FlatAbiType::F32,
        Type::F64 => FlatAbiType::F64,
        Type::PtrMut(inner, _) | Type::PtrConst(inner, _) => {
            // A pointer to `Void` is opaque; any other pointer keeps the
            // pointee so out-params can be projected.
            match inner.as_ref() {
                Type::Void => FlatAbiType::Ptr,
                _ => {
                    let pointee = map_flat_type(inner, index, enum_sink);
                    FlatAbiType::PtrTo(Box::new(pointee))
                }
            }
        }
        Type::Name(tn) => resolve_named_flat_type(&tn.namespace, &tn.name, index, enum_sink),
        // Anything else (Array, ConstRef, generics, …) is not a valid flat
        // ABI shape in practice — surface as unknown pointer.
        _ => FlatAbiType::Unknown,
    }
}

fn resolve_named_flat_type(
    namespace: &str,
    name: &str,
    index: &reader::Index,
    enum_sink: &mut dyn FnMut(TypeMeta),
) -> FlatAbiType {
    // Handle well-known Win32 typedef wrappers directly by name so we don't
    // depend on TypeDef lookup succeeding for well-known types.
    if namespace == "Windows.Win32.Foundation" {
        match name {
            "PWSTR" | "PCWSTR" => return FlatAbiType::PWStr,
            "PSTR" | "PCSTR" => return FlatAbiType::PStr,
            // BSTR is a length-prefixed, SysAllocString-owned COM string —
            // NOT a NUL-terminated PWSTR/PCWSTR. Marshalling as PWStr would
            // silently drop the 4-byte length prefix and can crash callees
            // that use SysStringLen. Treat as an opaque pointer so callers
            // must supply a properly-allocated BSTR (or generation fails
            // loudly with an unsupported-arg error at call time) instead
            // of silently mis-marshalling.
            "BSTR" => return FlatAbiType::Unknown,
            "BOOL" => return FlatAbiType::Bool32,
            "BOOLEAN" => return FlatAbiType::U8,
            "HRESULT" => return FlatAbiType::I32,
            "NTSTATUS" => return FlatAbiType::I32,
            // LSTATUS is a plain Int32 typedef in the win32 metadata, but
            // if a future metadata revision ever exposed it as a
            // `struct { Value: I32 }` (like Handle typedefs) the TypeDef
            // path below would classify it as a Handle — which routes
            // returns through the `'Ptr'` retKind and would mis-marshal
            // the status code as a pointer. Also route it through I32
            // explicitly so it stays consistent with is_status_return_type
            // in this module (which treats LSTATUS as a status typedef).
            "LSTATUS" => return FlatAbiType::I32,
            _ => {}
        }
    }
    let Some(def) = index.get(namespace, name).next() else {
        return FlatAbiType::Unknown;
    };
    let Some(ext) = def.extends() else {
        return FlatAbiType::Unknown;
    };
    // Enum: extends System.Enum.
    if ext.namespace() == "System" && ext.name() == "Enum" {
        let en = parse_enum_def(&def);
        if let TypeMeta::Enum {
            underlying,
            members,
            ..
        } = &en
        {
            let underlying_flat = match underlying.as_ref() {
                TypeMeta::U32 => FlatAbiType::U32,
                TypeMeta::I32 => FlatAbiType::I32,
                TypeMeta::U16 => FlatAbiType::U16,
                TypeMeta::I16 => FlatAbiType::I16,
                TypeMeta::U8 => FlatAbiType::U8,
                TypeMeta::I8 => FlatAbiType::I8,
                TypeMeta::U64 => FlatAbiType::U64,
                TypeMeta::I64 => FlatAbiType::I64,
                _ => FlatAbiType::I32,
            };
            let result = FlatAbiType::Enum {
                namespace: namespace.to_string(),
                name: name.to_string(),
                underlying: Box::new(underlying_flat),
                members: members.clone(),
            };
            enum_sink(en);
            return result;
        }
    }
    // Struct: extends System.ValueType. Handle-like typedefs are single-field
    // wrappers named `{ Value: T }` — we treat these as opaque handles.
    if ext.namespace() == "System" && ext.name() == "ValueType" {
        let fields: Vec<(String, windows_metadata::Type)> = def
            .fields()
            .map(|f| (f.name().to_string(), f.ty()))
            .collect();
        if fields.len() == 1 && fields[0].0 == "Value" {
            match &fields[0].1 {
                windows_metadata::Type::PtrMut(inner, _)
                | windows_metadata::Type::PtrConst(inner, _) => {
                    // Pointer typedef (HANDLE-like). If the pointee is Char/U8
                    // this is a string handle — project as PWStr/PStr; else
                    // treat as an opaque handle for natural marshalling.
                    return match inner.as_ref() {
                        windows_metadata::Type::Char => FlatAbiType::PWStr,
                        windows_metadata::Type::U8 => FlatAbiType::PStr,
                        _ => FlatAbiType::Handle {
                            namespace: namespace.to_string(),
                            name: name.to_string(),
                        },
                    };
                }
                windows_metadata::Type::I32 => {
                    // `{ Value: I32 }` typedefs are integer handles (BOOL is
                    // handled by name above; other examples: HRESULT.). Treat
                    // as `i32` at the ABI to avoid surfacing them as pointer.
                    if is_hresult_named(namespace, name) {
                        return FlatAbiType::I32;
                    }
                    return FlatAbiType::Handle {
                        namespace: namespace.to_string(),
                        name: name.to_string(),
                    };
                }
                windows_metadata::Type::U32 => {
                    return FlatAbiType::U32;
                }
                _ => {}
            }
        }
        // Multi-field struct — fall through to unknown (opaque pointer at ABI).
        return FlatAbiType::Unknown;
    }
    FlatAbiType::Unknown
}

fn is_hresult_named(ns: &str, name: &str) -> bool {
    ns == "Windows.Win32.Foundation" && name == "HRESULT"
}


/// Parse an interface's OWN methods (no inheritance flattening) with a caller-
/// supplied base offset. Used by `parse_com_interface_from_index` to lay out
/// base-class methods at the correct absolute vtable slots.
fn parse_interface_with_offset(
    index: &reader::Index,
    namespace: &str,
    name: &str,
    base_offset: usize,
) -> Option<InterfaceMeta> {
    let def = index.get(namespace, name).next()?;
    let iid = extract_iid(&def);
    parse_interface_methods(index, &def, name, namespace, &iid, &[], base_offset)
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
    )
    else {
        return None;
    };
    let concrete_name = make_parameterized_name(&name, &args);
    parse_parameterized_interface(index, &namespace, &name, &concrete_name, &piid, &args)
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
    parse_interface_methods(index, &def, concrete_name, namespace, piid, generic_args, 6)
}

/// Core interface parsing: extract methods from a TypeDef, optionally substituting generics.
///
/// `base_offset` is the vtable index of the first user method:
/// - `6` for WinRT (IInspectable-rooted: QI/AddRef/Release + GetIids/GetRuntimeClassName/GetTrustLevel).
/// - `3` for classic-COM IUnknown-rooted interfaces (QI/AddRef/Release only).
/// - Or any absolute offset for a base-aware slot in a chained classic-COM interface.
fn parse_interface_methods(
    index: &reader::Index,
    def: &reader::TypeDef,
    output_name: &str,
    namespace: &str,
    iid: &str,
    generic_args: &[TypeMeta],
    base_offset: usize,
) -> Option<InterfaceMeta> {
    let winmd_generics: Vec<windows_metadata::Type> =
        generic_args.iter().map(type_meta_to_winmd_type).collect();

    let mut methods = Vec::new();
    for (i, method) in def.methods().enumerate() {
        let vtable_index = base_offset + i;
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
    Some(InterfaceMeta {
        name: output_name.to_string(),
        namespace: namespace.to_string(),
        iid: iid.to_string(),
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
        _ => windows_metadata::Type::Object,
    }
}

fn extract_iid(def: &reader::TypeDef) -> String {
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
        doc: None,
        deprecated: None,
    }
}

fn map_winmd_type(ty: &windows_metadata::Type, index: &reader::Index) -> TypeMeta {
    map_winmd_type_with_generics(ty, index, &[])
}

fn map_winmd_type_with_generics(
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

        Type::Name(tn) => resolve_named_type(&tn.namespace, &tn.name, &tn.generics, index),

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
                return TypeMeta::AsyncOperation(Box::new(map_winmd_type(&generics[0], index)));
            }
            "IAsyncActionWithProgress`1" if generics.len() == 1 => {
                return TypeMeta::AsyncActionWithProgress(Box::new(map_winmd_type(
                    &generics[0],
                    index,
                )));
            }
            "IAsyncOperationWithProgress`2" if generics.len() == 2 => {
                return TypeMeta::AsyncOperationWithProgress(
                    Box::new(map_winmd_type(&generics[0], index)),
                    Box::new(map_winmd_type(&generics[1], index)),
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
        let args = generics.iter().map(|g| map_winmd_type(g, index)).collect();
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
        assert!(stack_panel.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::PublicComposition && constructor.is_public()
        }));

        let automation_peer = parse_class(
            &winmd_paths,
            "Microsoft.UI.Xaml.Automation.Peers",
            "AutomationPeer",
        )
        .unwrap();
        assert!(automation_peer.constructors.iter().any(|constructor| {
            constructor.kind == ConstructorKind::ProtectedComposition && !constructor.is_public()
        }));
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
}
