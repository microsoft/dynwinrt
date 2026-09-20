// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::{collections::BTreeMap, env, fs, io, path::Path, process::Command};

use sha2::{Digest, Sha256};

type Inputs = BTreeMap<String, Vec<u8>>;

fn relative_name(root: &Path, path: &Path) -> io::Result<String> {
  let relative = path.strip_prefix(root).map_err(io::Error::other)?;
  relative
    .components()
    .map(|component| {
      component
        .as_os_str()
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("Native source path is not valid Unicode"))
    })
    .collect::<io::Result<Vec<_>>>()
    .map(|components| components.join("/"))
}

fn read_input(root: &Path, path: &Path, inputs: &mut Inputs) -> io::Result<()> {
  println!("cargo:rerun-if-changed={}", path.display());
  inputs.insert(relative_name(root, path)?, fs::read(path)?);
  Ok(())
}

fn read_tree(root: &Path, directory: &Path, inputs: &mut Inputs) -> io::Result<()> {
  // Watching the directory also detects newly added/removed native source files.
  println!("cargo:rerun-if-changed={}", directory.display());
  for entry in fs::read_dir(directory)? {
    let entry = entry?;
    let kind = entry.file_type()?;
    if kind.is_dir() {
      read_tree(root, &entry.path(), inputs)?;
    } else if kind.is_file() {
      read_input(root, &entry.path(), inputs)?;
    } else {
      return Err(io::Error::other(format!(
        "Native fingerprint inputs must be regular files/directories: {}",
        entry.path().display()
      )));
    }
  }
  Ok(())
}

fn fingerprint(inputs: &Inputs, configuration: &BTreeMap<String, String>) -> String {
  let mut digest = Sha256::new();
  for (kind, entries) in [
    (
      b"source".as_slice(),
      inputs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect::<Vec<_>>(),
    ),
    (
      b"configuration".as_slice(),
      configuration
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_bytes()))
        .collect::<Vec<_>>(),
    ),
  ] {
    for (name, value) in entries {
      digest.update(kind);
      digest.update((name.len() as u64).to_le_bytes());
      digest.update(name.as_bytes());
      digest.update((value.len() as u64).to_le_bytes());
      digest.update(value);
    }
  }
  format!("{:x}", digest.finalize())
}

pub fn generate() -> io::Result<()> {
  let manifest = env::var_os("CARGO_MANIFEST_DIR")
    .ok_or_else(|| io::Error::other("CARGO_MANIFEST_DIR is required"))?;
  let root = Path::new(&manifest).join("..").join("..").canonicalize()?;
  let mut inputs = Inputs::new();
  for name in ["Cargo.toml", "Cargo.lock"] {
    read_input(&root, &root.join(name), &mut inputs)?;
  }
  for components in [
    ["bindings", "js"],
    ["crates", "dynwinrt"],
    ["crates", "dynwinrt-com-contracts"],
    ["crates", "dynwinrt-win32-contracts"],
  ] {
    let directory = root.join(components[0]).join(components[1]);
    read_input(&root, &directory.join("Cargo.toml"), &mut inputs)?;
    let build_script = directory.join("build.rs");
    if build_script.try_exists()? {
      read_input(&root, &build_script, &mut inputs)?;
    }
    read_tree(&root, &directory.join("src"), &mut inputs)?;
  }
  let binding = root.join("bindings").join("js");
  read_tree(&root, &binding.join("build_support"), &mut inputs)?;
  read_tree(&root, &binding.join("__test__").join("native"), &mut inputs)?;

  let mut configuration = BTreeMap::new();
  for (name, value) in env::vars() {
    if name.starts_with("CARGO_CFG_")
      || name.starts_with("CARGO_FEATURE_")
      || [
        "TARGET",
        "PROFILE",
        "OPT_LEVEL",
        "DEBUG",
        "CARGO_ENCODED_RUSTFLAGS",
      ]
      .contains(&name.as_str())
    {
      let root_name = root.to_string_lossy();
      let portable_root = root_name
        .strip_prefix(r"\\?\")
        .unwrap_or(root_name.as_ref());
      let value = value
        .replace(root_name.as_ref(), "<workspace>")
        .replace(portable_root, "<workspace>")
        .replace(&portable_root.replace('\\', "/"), "<workspace>");
      configuration.insert(name, value);
    }
  }
  let rustc = env::var_os("RUSTC").ok_or_else(|| io::Error::other("RUSTC is required"))?;
  let version = Command::new(rustc).arg("-vV").output()?;
  if !version.status.success() {
    return Err(io::Error::other(format!(
      "rustc -vV failed: {}",
      String::from_utf8_lossy(&version.stderr)
    )));
  }
  configuration.insert(
    "rustc-version".into(),
    String::from_utf8(version.stdout).map_err(io::Error::other)?,
  );
  let salt = format!(
    "microsoft.dynwinrt.native/{}",
    fingerprint(&inputs, &configuration)
  );
  let generated = format!(
    "macro_rules! native_class {{ ($item:item) => {{ #[napi_derive::napi(type_tag = {salt:?})] $item }}; }}\n"
  );
  let output = env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("OUT_DIR is required"))?;
  fs::write(Path::new(&output).join("native_class_tag.rs"), generated)?;
  println!("cargo:rustc-env=DYNWINRT_NATIVE_CLASS_SALT={salt}");
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  struct SourceTree(std::path::PathBuf);

  impl SourceTree {
    fn new() -> Self {
      static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
      loop {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = env::temp_dir().join(format!("dynwinrt-branding-{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
          Ok(()) => return Self(path),
          Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
          Err(error) => panic!("Unable to create native branding test tree: {error}"),
        }
      }
    }
  }

  impl Drop for SourceTree {
    fn drop(&mut self) {
      fs::remove_dir_all(&self.0).expect("Remove the native branding test's own source tree");
    }
  }

  #[test]
  fn source_order_and_workspace_location_do_not_change_the_fingerprint() {
    let first = Inputs::from([
      ("src/b.rs".into(), b"second".to_vec()),
      ("src/a.rs".into(), b"first".to_vec()),
    ]);
    let second = Inputs::from([
      ("src/a.rs".into(), b"first".to_vec()),
      ("src/b.rs".into(), b"second".to_vec()),
    ]);
    assert_eq!(
      fingerprint(&first, &BTreeMap::new()),
      fingerprint(&second, &BTreeMap::new())
    );
    assert_eq!(
      relative_name(Path::new(r"C:\one"), Path::new(r"C:\one\src\a.rs")).unwrap(),
      relative_name(Path::new(r"D:\two"), Path::new(r"D:\two\src\a.rs")).unwrap(),
    );
  }

  #[test]
  fn copied_source_trees_match_and_edits_or_new_files_invalidate_the_brand() {
    let first = SourceTree::new();
    let second = SourceTree::new();
    let collect = |root: &Path| {
      let mut inputs = Inputs::new();
      read_tree(root, &root.join("src"), &mut inputs).unwrap();
      fingerprint(&inputs, &BTreeMap::new())
    };
    for root in [&first.0, &second.0] {
      fs::create_dir(root.join("src")).unwrap();
      fs::write(root.join("src").join("value.rs"), "struct Value(u32);").unwrap();
    }
    let original = collect(&first.0);
    assert_eq!(original, collect(&second.0));
    fs::write(second.0.join("src").join("value.rs"), "struct Value(u64);").unwrap();
    assert_ne!(original, collect(&second.0));
    fs::write(second.0.join("src").join("value.rs"), "struct Value(u32);").unwrap();
    fs::write(second.0.join("src").join("extra.rs"), "struct Extra;").unwrap();
    assert_ne!(original, collect(&second.0));
  }

  #[test]
  fn source_dependency_target_feature_and_compiler_changes_change_the_fingerprint() {
    let inputs = Inputs::from([("src/value.rs".into(), b"layout".to_vec())]);
    let configuration = BTreeMap::from([
      ("TARGET".into(), "aarch64-pc-windows-msvc".into()),
      ("rustc-version".into(), "rustc 1".into()),
    ]);
    let original = fingerprint(&inputs, &configuration);
    for (name, value) in [
      ("src/value.rs", "new layout"),
      ("Cargo.lock", "new dependency"),
      ("src/new.rs", "new class"),
    ] {
      let mut changed = inputs.clone();
      changed.insert(name.into(), value.as_bytes().to_vec());
      assert_ne!(original, fingerprint(&changed, &configuration));
    }
    for (name, value) in [
      ("TARGET", "x86_64-pc-windows-msvc"),
      ("CARGO_FEATURE_TEST_HOOKS", "1"),
      ("rustc-version", "rustc 2"),
      ("CARGO_ENCODED_RUSTFLAGS", "-Zrandomize-layout"),
    ] {
      let mut changed = configuration.clone();
      changed.insert(name.into(), value.into());
      assert_ne!(original, fingerprint(&inputs, &changed));
    }
  }
}
