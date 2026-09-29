// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use dynwinrt_codegen::meta::{
    self, ClassMeta, InterfaceMeta, MethodMeta, ParamDirection, ParamMeta,
};
use dynwinrt_codegen::types::{TypeMeta, TypeMeta::AsyncOperation};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

fn storage_file() -> TypeMeta {
    TypeMeta::RuntimeClass {
        namespace: "Windows.Storage".into(),
        name: "StorageFile".into(),
        default_interface: None,
    }
}

fn method(name: &str, index: usize, params: Vec<ParamMeta>) -> MethodMeta {
    MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index: index,
        params,
        return_type: Some(AsyncOperation(Box::new(storage_file()))),
        ..Default::default()
    }
}

#[test]
fn default_option_method_is_one_python_overload_group_with_legacy_alias() {
    let interface = InterfaceMeta {
        name: "IStorageFolder".into(),
        namespace: "Windows.Storage".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            method(
                "CreateFileAsync",
                6,
                vec![
                    ParamMeta {
                        name: "desiredName".into(),
                        typ: TypeMeta::String,
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "options".into(),
                        typ: TypeMeta::Enum {
                            namespace: "Windows.Storage".into(),
                            name: "CreationCollisionOption".into(),
                            underlying: Box::new(TypeMeta::I32),
                            members: Vec::new(),
                            is_flags: false,
                            doc: None,
                            deprecated: None,
                        },
                        direction: ParamDirection::In,
                    },
                ],
            ),
            method(
                "CreateFileAsyncOverloadDefaultOptions",
                7,
                vec![ParamMeta {
                    name: "desiredName".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                }],
            ),
        ],
        ..Default::default()
    };
    let known = HashSet::from([
        "CreationCollisionOption".into(),
        "IStorageFolder".into(),
        "StorageFile".into(),
    ]);

    let runtime = common::generate_interface(&interface, &known, &HashSet::new());
    let stub = common::generate_interface_stub(&interface, &known, &HashSet::new());

    assert_eq!(
        runtime.matches("def create_file_async(self, *args").count(),
        1
    );
    assert!(runtime.contains("def _create_file_async_6("), "{runtime}");
    assert!(runtime.contains("def _create_file_async_7("), "{runtime}");
    assert!(
        runtime.contains("create_file_async_overload_default_options = create_file_async"),
        "{runtime}"
    );
    assert_eq!(stub.matches("def create_file_async(").count(), 2, "{stub}");
    assert_eq!(stub.matches("@overload").count(), 2, "{stub}");
    assert_eq!(
        stub.matches("def create_file_async_overload_default_options(")
            .count(),
        1,
        "{stub}"
    );
}

#[test]
fn newly_dispatched_methods_end_with_their_guard_free_legacy_tier() {
    let mode = TypeMeta::Enum {
        namespace: "Contoso".into(),
        name: "Mode".into(),
        underlying: Box::new(TypeMeta::I32),
        members: Vec::new(),
        is_flags: false,
        doc: None,
        deprecated: None,
    };
    let iterable = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IIterable`1".into(),
        piid: "faa585ea-6214-4217-afda-7f46de5869b3".into(),
        args: vec![TypeMeta::String],
    };
    let param = |name: &str, typ| ParamMeta {
        name: name.into(),
        typ,
        direction: ParamDirection::In,
    };
    let overload = |name: &str, raw_name: &str, index: usize, params: Vec<ParamMeta>| MethodMeta {
        name: name.into(),
        raw_name: raw_name.into(),
        vtable_index: index,
        params,
        ..Default::default()
    };
    let interface = InterfaceMeta {
        name: "IWidget".into(),
        namespace: "Contoso".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
        methods: vec![
            overload(
                "OpenAsync",
                "OpenAsync",
                6,
                vec![param("mode", mode.clone())],
            ),
            overload(
                "OpenWithOptionsAsync",
                "OpenAsync",
                7,
                vec![param("mode", mode.clone()), param("options", mode.clone())],
            ),
            overload(
                "FindAsync",
                "FindAsync",
                8,
                vec![param("id", TypeMeta::Guid)],
            ),
            overload(
                "FindWithOptionsAsync",
                "FindAsync",
                9,
                vec![
                    param("id", TypeMeta::Guid),
                    param("options", TypeMeta::String),
                ],
            ),
            overload(
                "CountAsync",
                "CountAsync",
                10,
                vec![param("count", TypeMeta::I32)],
            ),
            overload(
                "CountWithOptionsAsync",
                "CountAsync",
                11,
                vec![
                    param("count", TypeMeta::I32),
                    param("options", TypeMeta::Bool),
                ],
            ),
            overload(
                "LoadAsync",
                "LoadAsync",
                12,
                vec![param("items", iterable.clone())],
            ),
            overload(
                "LoadWithOptionsAsync",
                "LoadAsync",
                13,
                vec![param("items", iterable), param("options", TypeMeta::Bool)],
            ),
        ],
        ..Default::default()
    };
    let known = HashSet::from(["IWidget".into(), "Mode".into()]);
    let runtime = common::generate_interface(&interface, &known, &HashSet::new());
    let wrapper = &runtime[runtime.rfind("\nclass IWidget:").unwrap()..];

    for (name, parameter, private, conversion) in [
        ("open_async", "mode", "_open_async_6", "int(mode)"),
        ("find_async", "id", "_find_async_8", "_dynwinrt_guid(id)"),
        (
            "count_async",
            "count",
            "_count_async_10",
            "DynWinRTValue.from_i32(count)",
        ),
        (
            "load_async",
            "items",
            "_load_async_12",
            "_dynwinrt_vector(items",
        ),
    ] {
        let body = member_body(wrapper, name);
        let tier = format!(
            "return _dynwinrt_legacy_call(self.{private}, ('{parameter}',), args, kwargs, '{name}')"
        );
        assert!(
            body.contains(&tier),
            "{name} lacks its final legacy tier:\n{body}"
        );
        assert!(
            member_body(wrapper, private).contains(conversion),
            "{private} lost its permissive conversion:\n{runtime}"
        );
    }
}

#[test]
fn real_storage_folder_default_options_method_is_normalized() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    let class = meta::parse_class(WINDOWS_WINMD, "Windows.Storage", "StorageFolder")
        .expect("StorageFolder metadata");
    let known = HashSet::from([
        "CreationCollisionOption".into(),
        "StorageFile".into(),
        "StorageFolder".into(),
    ]);

    let runtime = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let stub = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());

    assert!(
        runtime.contains("def create_file_async(self, *args"),
        "{runtime}"
    );
    assert!(
        runtime.contains("create_file_async_overload_default_options = create_file_async"),
        "{runtime}"
    );
    assert_eq!(stub.matches("def create_file_async(").count(), 2, "{stub}");
}

/// Port of the generator's `to_snake_case` for the ABI names used below.
fn to_snake_case(name: &str) -> String {
    let characters = name.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (index, &character) in characters.iter().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                let previous = characters[index - 1];
                let next_lower = characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_lowercase());
                if previous.is_lowercase()
                    || previous.is_ascii_digit()
                    || (next_lower && previous.is_uppercase())
                {
                    result.push('_');
                }
            }
            result.extend(character.to_lowercase());
        } else {
            result.push(character);
        }
    }
    let tokens = result
        .trim_start_matches('_')
        .split('_')
        .collect::<Vec<_>>();
    let mut merged = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index] == "u"
            && tokens
                .get(index + 1)
                .is_some_and(|next| ["int8", "int16", "int32", "int64"].contains(next))
        {
            merged.push(format!("u{}", tokens[index + 1]));
            index += 2;
        } else {
            merged.push(tokens[index].to_string());
            index += 1;
        }
    }
    let result = merged.join("_");
    if ["from", "import", "global", "print", "lambda", "pass", "del"].contains(&result.as_str()) {
        format!("{result}_")
    } else {
        result
    }
}

/// Public members of `class_name` and its `Like` protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Member {
    definitions: usize,
    overloads: usize,
    assigned: bool,
}

fn class_members(code: &str, class_name: &str) -> BTreeMap<String, Member> {
    let owners = [class_name.to_string(), format!("{class_name}Like")];
    let mut members = BTreeMap::<String, Member>::new();
    let mut inside = false;
    let mut decorated = false;
    for line in code.lines() {
        if let Some(header) = line.strip_prefix("class ") {
            let name = header
                .split(|character| character == '(' || character == ':')
                .next()
                .unwrap_or_default();
            inside = owners.iter().any(|owner| owner == name);
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') {
            inside = false;
        }
        if !inside {
            continue;
        }
        let Some(member) = line.strip_prefix("    ") else {
            continue;
        };
        if member == "@overload" {
            decorated = true;
            continue;
        }
        if member.starts_with('@') {
            continue;
        }
        if let Some(definition) = member.strip_prefix("def ") {
            let name = definition.split('(').next().unwrap_or_default().to_string();
            let entry = members.entry(name).or_default();
            entry.definitions += 1;
            entry.overloads += usize::from(decorated);
        } else if let Some((name, _)) = member.split_once(" = ")
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            members.entry(name.to_string()).or_default().assigned = true;
        }
        decorated = false;
    }
    members.retain(|name, _| !name.starts_with('_'));
    members
}

fn single_definition() -> Member {
    Member {
        definitions: 1,
        overloads: 0,
        assigned: false,
    }
}

fn real_class(namespace: &str, name: &str) -> Option<(ClassMeta, String, String)> {
    let class = meta::parse_class(WINDOWS_WINMD, namespace, name)?;
    let deps = meta::resolve_python_dependencies(WINDOWS_WINMD, &[class.clone()], &[], &[]);
    let mut known = HashSet::from([class.name.clone()]);
    known.extend(deps.classes.iter().map(|class| class.name.clone()));
    known.extend(
        deps.interfaces
            .iter()
            .map(|interface| interface.name.clone()),
    );
    known.extend(deps.enums.iter().filter_map(|typ| match typ {
        TypeMeta::Enum { name, .. } => Some(name.clone()),
        _ => None,
    }));
    let runtime = common::generate_class(&class, &known, &HashSet::new(), &HashSet::new());
    let stub = common::generate_class_stub(&class, &known, &HashSet::new(), &HashSet::new());
    Some((class, runtime, stub))
}

/// Every public method name emitted before CLR-name grouping: the snake-case ABI
/// names, which were either dispatcher names or their compatibility aliases.
fn previous_method_names(class: &ClassMeta) -> BTreeSet<String> {
    class
        .factory_interfaces
        .iter()
        .chain(class.static_interfaces.iter())
        .chain(class.default_interface.iter())
        .chain(class.required_interfaces.iter())
        .filter(|interface| interface.iid != "30d5a829-7fa4-4026-83bb-d75bae4ea99e")
        .flat_map(|interface| interface.methods.iter())
        .filter(|method| {
            !method.is_property_getter
                && !method.is_property_setter
                && !method.is_event_add
                && !method.is_event_remove
        })
        .map(|method| to_snake_case(&method.name))
        .collect()
}

const REPRESENTATIVE_CLASSES: &[(&str, &str)] = &[
    ("Windows.Storage", "StorageFile"),
    ("Windows.Storage", "StorageFolder"),
    ("Windows.UI.Notifications", "ToastNotifier"),
    ("Windows.UI.Notifications", "TileUpdateManagerForUser"),
    ("Windows.Globalization.NumberFormatting", "DecimalFormatter"),
    ("Windows.System", "Launcher"),
    ("Windows.Web.Http", "HttpClient"),
    ("Windows.Globalization", "Calendar"),
    ("Windows.Data.Xml.Dom", "XmlDocument"),
    ("Windows.Storage.Streams", "RandomAccessStream"),
    ("Windows.Storage.Streams", "DataWriter"),
    ("Windows.Networking.Sockets", "StreamSocket"),
    ("Windows.Networking.Sockets", "StreamWebSocket"),
    ("Windows.UI.Composition.Interactions", "InteractionTracker"),
    ("Windows.UI.Xaml", "PropertyMetadata"),
    ("Windows.Devices.Enumeration", "DeviceInformation"),
];

#[test]
fn real_classes_keep_every_previous_public_method_name() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    for (namespace, name) in REPRESENTATIVE_CLASSES {
        let (class, runtime, stub) = real_class(namespace, name).expect("class metadata");
        let runtime_members = class_members(&runtime, name);
        let stub_members = class_members(&stub, name);
        for previous in previous_method_names(&class) {
            assert!(
                runtime_members.contains_key(&previous),
                "{namespace}.{name}.py lost `{previous}`:\n{runtime}"
            );
            assert!(
                stub_members.contains_key(&previous),
                "{namespace}.{name}.pyi lost `{previous}`:\n{stub}"
            );
        }
    }
}

#[test]
fn real_classes_project_overloads_under_documented_clr_names() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    // (class, documented name, overloads, previous names kept as aliases)
    let expectations: &[(&str, &str, &str, usize, &[&str])] = &[
        (
            "Windows.Storage",
            "StorageFile",
            "copy_async",
            3,
            &[
                "copy_overload",
                "copy_overload_default_options",
                "copy_overload_default_name_and_options",
            ],
        ),
        (
            "Windows.Storage",
            "StorageFile",
            "move_async",
            3,
            &[
                "move_overload",
                "move_overload_default_options",
                "move_overload_default_name_and_options",
            ],
        ),
        (
            "Windows.UI.Notifications",
            "ToastNotifier",
            "update",
            2,
            &["update_with_tag", "update_with_tag_and_group"],
        ),
        (
            "Windows.Globalization.NumberFormatting",
            "DecimalFormatter",
            "format",
            2,
            &[],
        ),
        (
            "Windows.System",
            "Launcher",
            "launch_file_async",
            2,
            &["launch_file_with_options_async"],
        ),
        (
            "Windows.System",
            "Launcher",
            "launch_uri_async",
            3,
            &[
                "launch_uri_with_options_async",
                "launch_uri_with_data_async",
            ],
        ),
        (
            "Windows.Web.Http",
            "HttpClient",
            "get_async",
            2,
            &["get_with_option_async"],
        ),
        (
            "Windows.Globalization",
            "Calendar",
            "month_as_string",
            2,
            &["month_as_full_string"],
        ),
        (
            "Windows.Data.Xml.Dom",
            "XmlDocument",
            "load_xml",
            2,
            &["load_xml_with_settings"],
        ),
        (
            "Windows.Storage.Streams",
            "RandomAccessStream",
            "copy_async",
            2,
            &["copy_size_async"],
        ),
    ];
    let mut generated = BTreeMap::new();
    for (namespace, name, documented, overloads, aliases) in expectations {
        let (_, runtime, stub) = generated
            .entry((*namespace, *name))
            .or_insert_with(|| real_class(namespace, name).expect("class metadata"));
        let runtime_members = class_members(runtime, name);
        let stub_members = class_members(stub, name);
        assert_eq!(
            runtime_members.get(*documented),
            Some(&single_definition()),
            "{name}.{documented} must be one runtime dispatcher:\n{runtime}"
        );
        assert!(
            runtime.contains(&format!("def {documented}(self, *args, **kwargs):"))
                || runtime.contains(&format!("def {documented}(*args, **kwargs):")),
            "{name}.{documented} must dispatch overloads:\n{runtime}"
        );
        assert_eq!(
            stub_members.get(*documented),
            Some(&Member {
                definitions: *overloads,
                overloads: *overloads,
                assigned: false,
            }),
            "{name}.{documented} must declare {overloads} overloads:\n{stub}"
        );
        for alias in *aliases {
            assert_eq!(
                runtime_members.get(*alias).map(|member| member.assigned),
                Some(true),
                "{name}.{alias} must stay available as an alias:\n{runtime}"
            );
            assert!(
                stub_members.contains_key(*alias),
                "{name}.{alias} must stay typed:\n{stub}"
            );
        }
    }

    // The runtime dispatcher keeps all three ABI overloads, trying Int64 before
    // UInt64 (larger values) and Double.
    let (_, runtime, _) =
        &generated[&("Windows.Globalization.NumberFormatting", "DecimalFormatter")];
    let dispatcher = &runtime[runtime
        .find("    def format(self, *args, **kwargs):")
        .unwrap()..];
    let order = ["self._format_6(", "self._format_7(", "self._format_8("].map(|call| {
        dispatcher
            .find(call)
            .unwrap_or_else(|| panic!("{call}:\n{runtime}"))
    });
    assert!(order.is_sorted(), "{runtime}");
    assert!(runtime.contains("from_i64(value)") && runtime.contains("from_u64(value)"));

    // INumberFormatter2's FormatInt/FormatUInt/FormatDouble are real methods, not
    // aliases of the INumberFormatter.Format overloads.
    let (_, runtime, _) =
        &generated[&("Windows.Globalization.NumberFormatting", "DecimalFormatter")];
    let members = class_members(runtime, "DecimalFormatter");
    for method in ["format_int", "format_u_int", "format_double"] {
        assert_eq!(members.get(method), Some(&single_definition()), "{runtime}");
    }
    let format_int = member_body(runtime, "format_int");
    assert!(
        format_int.find("self._format_6(").unwrap()
            < format_int.find("self._format_int_6(").unwrap(),
        "the compatibility dispatcher must try the exact interface method that format_int used before CLR grouping:\n{runtime}"
    );
    assert_eq!(
        runtime.matches("    def _format_6(").count(),
        1,
        "the canonical implementation should be defined once:\n{runtime}"
    );
    assert!(
        runtime.contains("return _INumberFormatter2.method(6)"),
        "{runtime}"
    );
}

#[test]
fn real_collisions_keep_previous_python_names() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    // IWebSocket.CloseWithStatus is a `Close` overload, but `close()` is the
    // generated IClosable member of every WebSocket runtime class.
    let (_, runtime, stub) =
        real_class("Windows.Networking.Sockets", "StreamWebSocket").expect("class metadata");
    let members = class_members(&runtime, "StreamWebSocket");
    assert_eq!(
        members.get("close_with_status"),
        Some(&single_definition()),
        "{runtime}"
    );
    assert!(runtime.contains("    def close(self):\n"), "{runtime}");
    assert!(
        stub.contains("def close_with_status(self, code: int, reason: str)"),
        "{stub}"
    );

    // `CreateTileUpdaterForApplication()` is an overload name of
    // `CreateTileUpdaterForApplicationForUser`, so the documented
    // `CreateTileUpdaterForApplication(String)` cannot take over that name.
    let (_, runtime, stub) =
        real_class("Windows.UI.Notifications", "TileUpdateManagerForUser").expect("class metadata");
    let members = class_members(&runtime, "TileUpdateManagerForUser");
    assert_eq!(
        members.get("create_tile_updater_for_application_for_user"),
        Some(&single_definition()),
        "{runtime}"
    );
    assert!(
        runtime.contains(
            "    create_tile_updater_for_application = create_tile_updater_for_application_for_user\n"
        ),
        "{runtime}"
    );
    assert_eq!(
        members.get("create_tile_updater_for_application_with_id"),
        Some(&single_definition()),
        "{runtime}"
    );
    assert!(
        stub.contains("def create_tile_updater_for_application(self) ->"),
        "{stub}"
    );
}

/// The body of `def {name}(` in `code`, up to the next member.
fn member_body<'a>(code: &'a str, name: &str) -> &'a str {
    let start = code
        .find(&format!("    def {name}("))
        .unwrap_or_else(|| panic!("missing `{name}`:\n{code}"));
    let rest = &code[start..];
    let end = rest[1..]
        .find("\n    def ")
        .or_else(|| rest[1..].find("\n    @"))
        .map_or(rest.len(), |end| end + 1);
    &rest[..end]
}

#[test]
fn real_former_standalone_names_keep_calling_their_own_overload() {
    if !Path::new(WINDOWS_WINMD).exists() {
        eprintln!("Skipping: Windows.winmd not found");
        return;
    }
    // INumberFormatter.Format(UInt64) was `format_u_int`. Through `format`,
    // format_u_int(5) would reach Format(Int64); the alias must keep UInt64.
    let interfaces =
        meta::parse_interfaces(WINDOWS_WINMD, "Windows.Globalization.NumberFormatting");
    let formatter = interfaces
        .iter()
        .find(|interface| interface.name == "INumberFormatter")
        .expect("INumberFormatter metadata");
    let unsigned = formatter
        .methods
        .iter()
        .find(|method| method.name == "FormatUInt")
        .expect("FormatUInt");
    let runtime = common::generate_interface(formatter, &HashSet::new(), &HashSet::new());
    let target = format!("_format_{}", unsigned.vtable_index);
    assert!(
        runtime.contains(&format!("\n    format_u_int = {target}\n")),
        "{runtime}"
    );
    let body = member_body(&runtime, &target);
    assert!(
        body.contains(&format!(
            "_INumberFormatter.method({})",
            unsigned.vtable_index
        )) && body.contains("DynWinRTValue.from_u64(value)"),
        "{body}"
    );
    let stub = common::generate_interface_stub(formatter, &HashSet::new(), &HashSet::new());
    assert!(
        stub.contains("    def format_u_int(self, value: int) -> str"),
        "{stub}"
    );

    // PropertyMetadata.Create(Object) was `create_with_default_value`. Through
    // `create`, a projected object would reach Create(CreateDefaultValueCallback).
    let (class, runtime, stub) =
        real_class("Windows.UI.Xaml", "PropertyMetadata").expect("class metadata");
    let with_default = class
        .static_interfaces
        .iter()
        .flat_map(|interface| interface.methods.iter())
        .find(|method| method.name == "CreateWithDefaultValue")
        .expect("CreateWithDefaultValue");
    let target = format!("_create_{}", with_default.vtable_index);
    assert!(
        runtime.contains(&format!("\n    create_with_default_value = {target}\n")),
        "{runtime}"
    );
    let body = member_body(&runtime, &target);
    assert!(
        body.contains(&format!(
            "_IPropertyMetadataStatics.method({})",
            with_default.vtable_index
        )) && body.contains("getattr(default_value, '_obj', default_value)"),
        "{body}"
    );
    assert_eq!(
        class_members(&stub, "PropertyMetadata").get("create_with_default_value"),
        Some(&single_definition()),
        "{stub}"
    );
}

static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join(format!(
                "ovl{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn python() -> std::path::PathBuf {
    std::env::var_os("DYNWINRT_TEST_PYTHON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("python"))
}

fn has_mypy() -> bool {
    let available = std::process::Command::new(python())
        .args(["-m", "mypy", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
        "DYNWINRT_REQUIRE_MYPY=1 but mypy is unavailable",
    );
    available
}

#[test]
fn real_merged_overload_stubs_pass_strict_mypy() {
    if !Path::new(WINDOWS_WINMD).exists() || !has_mypy() {
        eprintln!("Skipping: Windows.winmd or mypy unavailable");
        return;
    }
    let fixture = Fixture::new();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args(["generate", "--winmd", WINDOWS_WINMD, "--class-name"])
        .arg(
            "Windows.Globalization.NumberFormatting.DecimalFormatter,Windows.Storage.StorageFile,\
             Windows.System.Launcher,Windows.Data.Xml.Dom.XmlDocument,\
             Windows.Globalization.Calendar,Windows.Storage.Streams.RandomAccessStream,\
             Windows.Storage.Streams.InMemoryRandomAccessStream,Windows.Storage.Streams.DataWriter",
        )
        .args(["--lang", "py", "--output"])
        .arg(fixture.0.join("sdk"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(
        fixture.0.join("consumer.py"),
        r#"from typing import assert_type
from sdk.windows.data.xml.dom import XmlDocument, XmlLoadSettings
from sdk.windows.foundation import Uri
from sdk.windows.globalization import Calendar
from sdk.windows.globalization.number_formatting import DecimalFormatter
from sdk.windows.storage import NameCollisionOption, StorageFile, StorageFolder
from sdk.windows.storage.streams import DataWriter, InMemoryRandomAccessStream, RandomAccessStream
from sdk.windows.system import Launcher

def formatting(formatter: DecimalFormatter) -> None:
    assert_type(formatter.format(5), str)
    assert_type(formatter.format(2.5), str)
    assert_type(formatter.format_int(5), str)
    assert_type(formatter.format_u_int(5), str)

async def storage(file: StorageFile, folder: StorageFolder) -> None:
    assert_type(await file.copy_async(folder), StorageFile | None)
    assert_type(await file.copy_async(folder, "name.txt"), StorageFile | None)
    option = NameCollisionOption.ReplaceExisting
    assert_type(await file.copy_async(folder, "name.txt", option), StorageFile | None)
    assert_type(await file.copy_overload(folder, "name.txt", option), StorageFile | None)
    await file.move_async(folder)

async def launching(file: StorageFile, uri: Uri) -> None:
    assert_type(await Launcher.launch_file_async(file), bool)
    assert_type(await Launcher.launch_uri_async(uri), bool)

def xml(document: XmlDocument, settings: XmlLoadSettings) -> None:
    document.load_xml("<root />")
    document.load_xml("<root />", settings)
    document.load_xml_with_settings("<root />", settings)

def calendar(value: Calendar) -> None:
    assert_type(value.month_as_string(), str)
    assert_type(value.month_as_string(3), str)
    assert_type(value.month_as_full_string(), str)

async def streams(source: InMemoryRandomAccessStream, target: InMemoryRandomAccessStream) -> None:
    assert_type(await RandomAccessStream.copy_async(source, target), int)
    assert_type(await RandomAccessStream.copy_async(source, target, 4), int)
    assert_type(await RandomAccessStream.copy_size_async(source, target, 4), int)
    DataWriter(source)
"#,
    )
    .unwrap();
    let output = std::process::Command::new(python())
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
            "sdk",
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
        .unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !text.contains("overload-cannot-match") && !text.contains("overload-overlap"),
        "{text}"
    );
    assert!(output.status.success(), "{text}");
}
