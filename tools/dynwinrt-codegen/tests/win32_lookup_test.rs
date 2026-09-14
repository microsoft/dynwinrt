// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use dynwinrt_codegen::win32_metadata::{FlatFunctionIndex, has_flat_functions};
use windows_metadata::{
    MethodAttributes, MethodCallAttributes, MethodImplAttributes, PInvokeAttributes, Signature,
    Type, TypeAttributes, writer,
};

const NAMESPACE: &str = "Windows.Win32.Routing";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dynwinrt-routing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn metadata(path: &Path, imported: bool) {
    let mut file = writer::File::new("Routing");
    file.TypeDef(
        NAMESPACE,
        "Apis",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public | TypeAttributes::Sealed,
    );
    let method = file.MethodDef(
        "GetTickCount",
        &Signature {
            flags: MethodCallAttributes::default(),
            return_type: Type::U32,
            types: Vec::new(),
        },
        MethodAttributes::Public,
        MethodImplAttributes::default(),
    );
    if imported {
        file.ImplMap(
            method,
            PInvokeAttributes::CallConvPlatformapi,
            "GetTickCount",
            "kernel32.dll",
        );
    }
    file.TypeDef(
        NAMESPACE,
        "IExample",
        writer::TypeDefOrRef::default(),
        TypeAttributes::Public | TypeAttributes::Interface | TypeAttributes::Abstract,
    );
    file.MethodDef(
        "Read",
        &Signature {
            flags: MethodCallAttributes::HASTHIS,
            return_type: Type::I32,
            types: Vec::new(),
        },
        MethodAttributes::Public | MethodAttributes::Virtual | MethodAttributes::Abstract,
        MethodImplAttributes::default(),
    );
    fs::write(path, file.into_stream()).unwrap();
}

#[test]
fn routing_is_lazy_and_reuses_one_snapshot_without_reopening_files() {
    let scratch = Scratch::new();
    let path = scratch.0.join("input.winmd");
    let paths = path.to_str().unwrap();
    let mut index = FlatFunctionIndex::new(paths);
    metadata(&path, true);
    assert!(index.has_flat_functions(NAMESPACE, "Apis").unwrap());
    fs::remove_file(&path).unwrap();
    for _ in 0..50 {
        assert!(!index.has_flat_functions(NAMESPACE, "IExample").unwrap());
        assert!(!index.has_flat_functions(NAMESPACE, "Missing").unwrap());
        assert!(index.has_flat_functions(NAMESPACE, "Apis").unwrap());
    }
    assert!(
        FlatFunctionIndex::new(paths)
            .has_flat_functions(NAMESPACE, "Apis")
            .is_err()
    );
    metadata(&path, false);
    assert!(
        !FlatFunctionIndex::new(paths)
            .has_flat_functions(NAMESPACE, "Apis")
            .unwrap()
    );
    assert!(index.has_flat_functions(NAMESPACE, "Apis").unwrap());
}

#[test]
fn routing_checks_all_definitions_in_the_loaded_metadata_set() {
    let scratch = Scratch::new();
    let first = scratch.0.join("first.winmd");
    let second = scratch.0.join("second.winmd");
    metadata(&first, false);
    metadata(&second, true);
    let paths = format!("{};{}", first.display(), second.display());
    let mut index = FlatFunctionIndex::new(&paths);
    assert!(index.has_flat_functions(NAMESPACE, "Apis").unwrap());
    assert!(!index.has_flat_functions(NAMESPACE, "IExample").unwrap());
    assert!(has_flat_functions(&paths, NAMESPACE, "Apis").unwrap());
}

#[test]
fn incomplete_metadata_sets_fail_closed_without_caching_partial_indexes() {
    let scratch = Scratch::new();
    let first = scratch.0.join("first.winmd");
    let second = scratch.0.join("second.winmd");
    metadata(&first, true);
    fs::write(&second, b"not metadata").unwrap();
    let paths = format!("{};{}", first.display(), second.display());
    let mut index = FlatFunctionIndex::new(&paths);
    let error = index.has_flat_functions(NAMESPACE, "Apis").unwrap_err();
    assert!(error.contains("win32.metadata: invalid file"));
    metadata(&second, false);
    assert!(index.has_flat_functions(NAMESPACE, "Apis").unwrap());
    assert!(
        FlatFunctionIndex::new(";")
            .has_flat_functions(NAMESPACE, "Apis")
            .is_err()
    );
}
