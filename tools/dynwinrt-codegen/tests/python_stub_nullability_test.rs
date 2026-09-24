// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The stub nullability policy on real Windows SDK metadata. Values received
//! from the projection are non-null in `.pyi` stubs by default, while
//! `IReference<T>`, `Try*` results, `Object` and delegate values keep
//! `| None`. Inputs, implementation protocols and the runtime `.py`
//! annotations are unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

struct Generated(PathBuf);

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Generated {
    fn new(classes: &str) -> Option<Self> {
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
            .join(format!("pn{}", std::process::id()));
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
fn stub_outputs_follow_the_nullability_policy() {
    let Some(generated) = Generated::new(
        "Windows.Storage.StorageFolder,Windows.Web.Http.Headers.HttpContentHeaderCollection,\
         Windows.Foundation.Collections.PropertySet,Windows.Data.Json.JsonObject",
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

    // Method, async, collection and property outputs are non-null by default.
    assert_contains(
        &folder,
        "def create_file_async(self, desired_name: str) -> WinRTCoroutine[StorageFile]: ...",
    );
    assert_contains(
        &folder,
        "def get_files_async(self) -> WinRTCoroutine[Sequence[StorageFile]]: ...",
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

    // IReference<T> values and Object values keep None.
    assert_contains(&headers, "def content_length(self) -> int | None: ...");
    assert_contains(
        &headers,
        "def content_type(self) -> HttpMediaTypeHeaderValue: ...",
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
