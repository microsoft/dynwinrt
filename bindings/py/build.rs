// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{env, fs, io::ErrorKind, path::PathBuf};

fn main() {
    // Mixed maturin packages do not copy the root native stub automatically.
    // Keep it authoritative and preserve the wheel's existing root stub shape.
    println!("cargo:rerun-if-changed=dynwinrt.pyi");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = fs::read(root.join("dynwinrt.pyi")).expect("read the native Python stub");
    // In an sdist maturin places Python sources at the workspace/archive root
    // while retaining this crate at bindings/py.
    let package = [root.clone(), root.join("..").join("..")]
        .into_iter()
        .map(|path| path.join("python").join("dynwinrt"))
        .find(|path| path.join("_implementation.py").is_file())
        .expect("find the packaged Python source directory");
    let destination = package.join("__init__.pyi");
    println!("cargo:rerun-if-changed={}", destination.display());
    let needs_update = match fs::read(&destination) {
        Ok(existing) => existing != source,
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(error) => panic!(
            "read packaged Python stub {}: {error}",
            destination.display()
        ),
    };
    if needs_update {
        fs::write(destination, source).expect("stage the packaged Python stub");
    }
}
