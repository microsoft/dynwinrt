// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The stub nullability policy on real Windows SDK metadata. Values received
//! from the projection are non-null in `.pyi` stubs by default, while
//! `IReference<T>`, `Try*` results, members documented to return null,
//! `Object` and delegate values keep `| None`. Collection elements follow the
//! native nullable-reference contract. Inputs, implementation protocols and
//! the runtime `.py` annotations are unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use dynwinrt_codegen::codegen::{python, python_stub};
use dynwinrt_codegen::meta::{InterfaceMeta, MethodMeta};
use dynwinrt_codegen::types::TypeMeta;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

struct Generated(PathBuf);

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Generated {
    fn new(label: &str, classes: &str) -> Option<Self> {
        if !Path::new(WINDOWS_WINMD).is_file() {
            eprintln!("Skipping: Windows.winmd not found");
            return None;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join(format!("pn{label}{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let output = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--winmd",
                WINDOWS_WINMD,
                "--class-name",
                classes,
            ])
            .args(["--lang", "py", "--output"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Some(Self(root))
    }

    fn module(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    }
}

fn assert_contains(text: &str, expected: &str) {
    assert!(text.contains(expected), "missing `{expected}`");
}

#[test]
fn ordinary_reference_array_stub_elements_are_nullable() {
    let widget = TypeMeta::RuntimeClass {
        namespace: "Contoso".into(),
        name: "Widget".into(),
        default_interface: None,
    };
    let widget_interface = TypeMeta::Interface {
        namespace: "Contoso".into(),
        name: "IWidget".into(),
        iid: "11111111-1111-1111-1111-111111111111".into(),
    };
    let handler = TypeMeta::Delegate {
        namespace: "Contoso".into(),
        name: "WidgetHandler".into(),
        iid: "22222222-2222-2222-2222-222222222222".into(),
    };
    let reference = TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
        args: vec![TypeMeta::U32],
    };
    let view = TypeMeta::Parameterized {
        namespace: "Windows.Foundation.Collections".into(),
        name: "IVectorView`1".into(),
        piid: "bbe1fa4c-b0e3-4583-baef-1f1b2e483e56".into(),
        args: vec![widget.clone()],
    };
    let method = |name: &str, typ: TypeMeta, slot: usize| MethodMeta {
        name: name.into(),
        raw_name: name.into(),
        vtable_index: slot,
        return_type: Some(typ),
        ..Default::default()
    };
    let source = InterfaceMeta {
        namespace: "Contoso".into(),
        name: "IArraySource".into(),
        iid: "33333333-3333-3333-3333-333333333333".into(),
        methods: vec![
            method("GetWidgets", TypeMeta::Array(Box::new(widget.clone())), 6),
            method(
                "GetInterfaces",
                TypeMeta::Array(Box::new(widget_interface.clone())),
                7,
            ),
            method("GetObjects", TypeMeta::Array(Box::new(TypeMeta::Object)), 8),
            method(
                "GetDelegates",
                TypeMeta::Array(Box::new(handler.clone())),
                9,
            ),
            method(
                "GetReferences",
                TypeMeta::Array(Box::new(reference.clone())),
                10,
            ),
            method("GetViews", TypeMeta::Array(Box::new(view.clone())), 11),
            method(
                "GetWidgetsAsync",
                TypeMeta::AsyncOperation(Box::new(TypeMeta::Array(Box::new(widget.clone())))),
                12,
            ),
            method(
                "GetStrings",
                TypeMeta::Array(Box::new(TypeMeta::String)),
                13,
            ),
            method("GetGuids", TypeMeta::Array(Box::new(TypeMeta::Guid)), 14),
            method("GetNumbers", TypeMeta::Array(Box::new(TypeMeta::I32)), 15),
            method("GetBytes", TypeMeta::Array(Box::new(TypeMeta::U8)), 16),
        ],
        ..Default::default()
    };
    let context = python::PythonProjectionContext::standalone([
        source.type_identity(),
        widget.type_identity(),
        widget_interface.type_identity(),
        handler.type_identity(),
        reference.type_identity(),
        view.type_identity(),
    ])
    .unwrap();
    let stub = python_stub::generate_interface_stub(&context, &source);

    for expected in [
        "def get_widgets(self) -> list[Widget | None]: ...",
        "def get_interfaces(self) -> list[IWidget | None]: ...",
        "def get_objects(self) -> list[DynWinRTValue | None]: ...",
        "def get_delegates(self) -> list[DynWinRTValue | None]: ...",
        "def get_references(self) -> list[IReference_UInt32 | None]: ...",
        "def get_views(self) -> list[IVectorView_Widget | None]: ...",
        "def get_widgets_async(self) -> WinRTCoroutine[list[Widget | None]]: ...",
        "def get_strings(self) -> list[str]: ...",
        "def get_guids(self) -> list[UUID]: ...",
        "def get_numbers(self) -> list[int]: ...",
        "def get_bytes(self) -> bytes: ...",
    ] {
        assert_contains(&stub, expected);
    }
    assert!(
        !stub.contains("list[Widget | None] | None"),
        "array values must remain non-null"
    );
}

#[test]
fn stub_outputs_follow_the_nullability_policy() {
    let Some(generated) = Generated::new(
        "out",
        "Windows.Storage.StorageFolder,Windows.Web.Http.Headers.HttpContentHeaderCollection,\
         Windows.Foundation.Collections.PropertySet,Windows.Data.Json.JsonObject,\
         Windows.Devices.Sensors.Accelerometer,Windows.Devices.Sensors.Compass,\
         Windows.System.DispatcherQueue,Windows.Data.Xml.Dom.XmlDocument",
    ) else {
        return;
    };
    let folder = generated.module("windows__storage__storage_folder.pyi");
    let folder_py = generated.module("windows__storage__storage_folder.py");
    let folder_view = generated.module("windows__storage__i_storage_folder.pyi");
    let headers =
        generated.module("windows__web__http__headers__http_content_header_collection.pyi");
    let properties = generated.module("windows__foundation__collections__property_set.pyi");
    let json = generated.module("windows__data__json__json_object.pyi");
    let accelerometer = generated.module("windows__devices__sensors__accelerometer.pyi");
    let compass = generated.module("windows__devices__sensors__compass.pyi");
    let dispatcher = generated.module("windows__system__dispatcher_queue.pyi");
    let xml = generated.module("windows__data__xml__dom__xml_document.pyi");

    // Method, async, collection and property outputs are non-null by default.
    assert_contains(
        &folder,
        "def create_file_async(self, desired_name: str) -> WinRTCoroutine[StorageFile]: ...",
    );
    assert_contains(
        &folder,
        "def get_files_async(self) -> WinRTCoroutine[Sequence[StorageFile | None]]: ...",
    );
    assert_contains(
        &folder,
        "def get_folder_from_path_async(path: str) -> WinRTCoroutine[StorageFolder]: ...",
    );
    assert_contains(
        &folder,
        "def properties(self) -> StorageItemContentProperties: ...",
    );
    assert_contains(
        &json,
        "def get_named_object(self, name: str) -> JsonObject: ...",
    );
    assert_contains(&json, "def parse(input: str) -> JsonObject: ...");

    // Try* results keep None on their result, async result and out values.
    assert_contains(
        &folder,
        "def try_get_item_async(self, name: str) -> WinRTCoroutine[IStorageItem | None]: ...",
    );
    assert_contains(
        &json,
        "def try_parse(input: str) -> tuple[JsonObject | None, bool]: ...",
    );

    // Members the Windows SDK documentation says can return null keep None,
    // including through overloads, interfaces and async results.
    assert_contains(
        &accelerometer,
        "def get_default() -> Accelerometer | None: ...",
    );
    assert_contains(
        &accelerometer,
        "def get_default_with_accelerometer_reading_type(reading_type: 'AccelerometerReadingType') -> Accelerometer | None: ...",
    );
    assert_contains(&compass, "def get_default() -> Compass | None: ...");
    assert_contains(
        &dispatcher,
        "def get_for_current_thread() -> DispatcherQueue | None: ...",
    );
    assert_contains(
        &dispatcher,
        "def create_timer(self) -> DispatcherQueueTimer: ...",
    );
    assert_contains(
        &folder,
        "def get_parent_async(self) -> WinRTCoroutine[StorageFolder | None]: ...",
    );
    assert_contains(
        &xml,
        "def select_single_node(self, xpath: str) -> IXmlNode | None: ...",
    );

    // IReference<T> values and Object values keep None, and so do properties
    // whose documentation says a null value means "absent".
    assert_contains(&headers, "def content_length(self) -> int | None: ...");
    assert_contains(
        &headers,
        "def content_type(self) -> HttpMediaTypeHeaderValue | None: ...",
    );
    assert_contains(
        &properties,
        "def lookup(self, key: str) -> DynWinRTValue | None: ...",
    );
    assert_contains(
        &properties,
        "def __getitem__(self, key: str) -> DynWinRTValue | None: ...",
    );

    // Inputs are unchanged.
    assert_contains(
        &headers,
        "def content_length(self, value: int | None | IReference_UInt64) -> None: ...",
    );
    assert_contains(
        &folder,
        "def create_folder_query(self, query_options: 'QueryOptionsLike') -> StorageFolderQueryResult: ...",
    );
    assert_contains(
        &properties,
        "def __setitem__(self, key: str, value: DynWinRTValue | _DynWinRTObject | None) -> None: ...",
    );

    // Implementation protocols keep their obligations.
    assert_contains(&folder_view, "class IStorageFolderHandlers(Protocol):");
    assert_contains(
        &folder_view,
        "def create_file_async_overload_default_options(self, desired_name: str) -> DynWinRTValue | None: ...",
    );

    // Runtime annotations stay pessimistic.
    assert_contains(
        &folder_py,
        "def get_folder_from_path_async(path: str) -> WinRTCoroutine[StorageFolder | None]:",
    );
    assert_contains(
        &folder_py,
        "def properties(self) -> StorageItemContentProperties | None:",
    );
}

#[test]
fn sensor_current_reading_docs_control_nullability_without_family_false_positives() {
    let Some(generated) = Generated::new(
        "sensorreadings",
        "Windows.Devices.Sensors.Accelerometer,Windows.Devices.Sensors.Compass,\
         Windows.Devices.Sensors.Gyrometer,Windows.Devices.Sensors.Inclinometer,\
         Windows.Devices.Sensors.LightSensor,Windows.Devices.Sensors.OrientationSensor,\
         Windows.Devices.Sensors.Custom.CustomSensor,Windows.Devices.Sensors.Altimeter,\
         Windows.Devices.Sensors.Barometer,Windows.Devices.Sensors.HumanPresenceSensor,\
         Windows.Devices.Sensors.Magnetometer,Windows.Devices.Sensors.ProximitySensor,\
         Windows.Devices.Sensors.ActivitySensor,Windows.Devices.Sensors.HingeAngleSensor,\
         Windows.Devices.Sensors.Pedometer",
    ) else {
        return;
    };

    for (module, signature) in [
        (
            "windows__devices__sensors__accelerometer.pyi",
            "def get_current_reading(self) -> AccelerometerReading | None: ...",
        ),
        (
            "windows__devices__sensors__compass.pyi",
            "def get_current_reading(self) -> CompassReading | None: ...",
        ),
        (
            "windows__devices__sensors__custom__custom_sensor.pyi",
            "def get_current_reading(self) -> CustomSensorReading | None: ...",
        ),
        (
            "windows__devices__sensors__gyrometer.pyi",
            "def get_current_reading(self) -> GyrometerReading | None: ...",
        ),
        (
            "windows__devices__sensors__inclinometer.pyi",
            "def get_current_reading(self) -> InclinometerReading | None: ...",
        ),
        (
            "windows__devices__sensors__light_sensor.pyi",
            "def get_current_reading(self) -> LightSensorReading | None: ...",
        ),
        (
            "windows__devices__sensors__orientation_sensor.pyi",
            "def get_current_reading(self) -> OrientationSensorReading | None: ...",
        ),
    ] {
        assert_contains(&generated.module(module), signature);
    }

    // The other GetCurrentReading variants at the pinned docs revision do
    // not contain the required-null-check wording and remain non-null.
    for (module, signature) in [
        (
            "windows__devices__sensors__activity_sensor.pyi",
            "def get_current_reading_async(self) -> WinRTCoroutine[ActivitySensorReading]: ...",
        ),
        (
            "windows__devices__sensors__altimeter.pyi",
            "def get_current_reading(self) -> AltimeterReading: ...",
        ),
        (
            "windows__devices__sensors__barometer.pyi",
            "def get_current_reading(self) -> BarometerReading: ...",
        ),
        (
            "windows__devices__sensors__hinge_angle_sensor.pyi",
            "def get_current_reading_async(self) -> WinRTCoroutine[HingeAngleReading]: ...",
        ),
        (
            "windows__devices__sensors__human_presence_sensor.pyi",
            "def get_current_reading(self) -> HumanPresenceSensorReading: ...",
        ),
        (
            "windows__devices__sensors__magnetometer.pyi",
            "def get_current_reading(self) -> MagnetometerReading: ...",
        ),
        (
            "windows__devices__sensors__proximity_sensor.pyi",
            "def get_current_reading(self) -> ProximitySensorReading: ...",
        ),
    ] {
        assert_contains(&generated.module(module), signature);
    }
    assert_contains(
        &generated.module("windows__devices__sensors__pedometer.pyi"),
        "def get_current_readings(self) -> Mapping['PedometerStepKind', PedometerReading | None]: ...",
    );
}

#[test]
fn reference_collection_elements_are_nullable_regardless_of_provenance() {
    let Some(generated) = Generated::new(
        "elements",
        "Windows.Storage.StorageFolder,Windows.Data.Json.JsonObject,\
         Windows.ApplicationModel.Resources.Core.ResourceMap,Windows.Media.Playback.MediaPlaybackList,\
         Windows.Foundation.Collections.StringMap",
    ) else {
        return;
    };
    let files =
        generated.module("windows__foundation__collections__i_vector_view_storage_file.pyi");
    let array = generated.module("windows__data__json__json_array.pyi");
    let object = generated.module("windows__data__json__json_object.pyi");
    let resources =
        generated.module("windows__application_model__resources__core__resource_map.pyi");
    let playlist = generated.module("windows__media__playback__media_playback_list.pyi");
    let observable = generated
        .module("windows__foundation__collections__i_observable_vector_media_playback_item.pyi");
    let string_map_view =
        generated.module("windows__foundation__collections__i_map_view_string_string.pyi");
    let string_map_view_py =
        generated.module("windows__foundation__collections__i_map_view_string_string.py");

    // Collection interfaces carry no provenance. A view or iterator obtained
    // from a mutable collection can expose a null slot, so view item
    // positions conservatively keep None too.
    assert_contains(
        &files,
        "def get_at(self, index: int) -> StorageFile | None: ...",
    );
    assert_contains(
        &files,
        "def __getitem__(self, index: int) -> StorageFile | None: ...",
    );
    assert_contains(
        &resources,
        "class ResourceMap(_ResourceMapIdentity, Mapping[str, NamedResource | None], _DynWinRTRuntimeClass):",
    );
    assert_contains(
        &resources,
        "def lookup(self, key: str) -> NamedResource | None: ...",
    );

    // Mutable collection elements, item positions and element-reading
    // members keep None for the same reason.
    assert_contains(
        &array,
        "class JsonArray(_JsonArrayIdentity, MutableSequence[IJsonValue | None], _DynWinRTRuntimeClass):",
    );
    assert_contains(
        &array,
        "def get_at(self, index: int) -> IJsonValue | None: ...",
    );
    assert_contains(
        &array,
        "def __getitem__(self, index: int) -> IJsonValue | None: ...",
    );
    assert_contains(
        &array,
        "def get_object_at(self, index: int) -> JsonObject: ...",
    );
    assert_contains(
        &object,
        "class JsonObject(_JsonObjectIdentity, MutableMapping[str, IJsonValue | None], _DynWinRTRuntimeClass):",
    );
    assert_contains(
        &object,
        "def lookup(self, key: str) -> IJsonValue | None: ...",
    );
    assert_contains(
        &object,
        "def get_named_array(self, name: str) -> JsonArray: ...",
    );
    assert_contains(
        &playlist,
        "def items(self) -> MutableSequence[MediaPlaybackItem | None]: ...",
    );
    assert_contains(&observable, "MutableSequence[MediaPlaybackItem | None]):");
    assert_contains(
        &observable,
        "def __getitem__(self, index: int) -> MediaPlaybackItem | None: ...",
    );
    // Split succeeds with two optional view pointers; no other IMapView
    // result is widened by this native contract.
    let split = "def split(self) -> tuple[Mapping[str, str] | None, Mapping[str, str] | None]: ...";
    assert_contains(&string_map_view, split);
    assert_contains(&string_map_view, "def lookup(self, key: str) -> str: ...");
    assert_contains(&string_map_view_py, split.trim_end_matches(" ..."));
}
