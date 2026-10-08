// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dynwinrt_codegen::codegen::python::{PythonProjectionContext, PythonTypeIdentity};
use dynwinrt_codegen::types::TypeIdentityKind;

const LEGACY_MAX_PATH: usize = 260; // Includes the terminating NUL.

pub(crate) fn dry_run_source_files(
    context: &PythonProjectionContext,
    implementations: &[PythonTypeIdentity],
    public_types: &[PythonTypeIdentity],
    pyi: bool,
) -> Vec<PathBuf> {
    fn add_module(files: &mut BTreeSet<PathBuf>, module: &Path, pyi: bool) {
        files.insert(module.with_extension("py"));
        if pyi {
            files.insert(module.with_extension("pyi"));
        }
    }

    let mut files = BTreeSet::new();
    for module in ["__init__", "_runtime"] {
        add_module(&mut files, Path::new(module), pyi);
    }
    if pyi {
        files.extend(["_typing.pyi", "_implementation_types.pyi"].map(PathBuf::from));
    }
    let is_emitted = |identity: &&PythonTypeIdentity| {
        !(identity.kind() == Some(TypeIdentityKind::Struct)
            && identity.definition_name() == Some("HResult"))
    };
    for identity in implementations.iter().filter(is_emitted) {
        add_module(
            &mut files,
            Path::new(&context.implementation_module(identity)),
            pyi,
        );
    }
    for identity in public_types.iter().filter(is_emitted) {
        if identity.namespace().is_none_or(str::is_empty) {
            continue;
        }
        let facade = context
            .public_qualified_module(identity)
            .split('.')
            .collect::<PathBuf>();
        add_module(&mut files, &facade, pyi);
        let mut parent = facade.parent();
        while let Some(directory) = parent.filter(|path| !path.as_os_str().is_empty()) {
            add_module(&mut files, &directory.join("__init__"), pyi);
            parent = directory.parent();
        }
    }
    files.into_iter().collect()
}

pub(crate) fn warn(output_dir: &Path, source_files: &[PathBuf]) -> Result<(), String> {
    if let Some(message) = warning(output_dir, source_files)? {
        eprintln!("{message}");
    }
    Ok(())
}

fn warning(output_dir: &Path, source_files: &[PathBuf]) -> Result<Option<String>, String> {
    if !cfg!(windows) {
        return Ok(None);
    }
    // Do not canonicalize: a junction/subst path is the consumer's useful short path.
    let output_dir = absolute_consumer_path(output_dir)?;
    let longest = source_files
        .iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "py" || extension == "pyi")
        })
        .map(|relative| {
            let path = output_dir.join(relative);
            let length = utf16_length(&path);
            (path, length)
        })
        .max_by(|left, right| (left.1, &left.0).cmp(&(right.1, &right.0)));
    Ok(longest.and_then(|(path, length)| warning_for_longest(&path, length, true)))
}

fn warning_for_longest(path: &Path, length: usize, windows: bool) -> Option<String> {
    if !windows || length < LEGACY_MAX_PATH {
        return None;
    }
    let excess = length - (LEGACY_MAX_PATH - 1);
    Some(format!(
        "warning: longest Python output path is {length} UTF-16 code units \
         (legacy Windows maximum: 259, excluding NUL): '{}'.\n  \
         Python import, mypy or pip may fail even if generation succeeds. \
         Shorten --output or the checkout by at least {excess} code units \
         (e.g. C:\\g\\bindings); run generation and consumers through the same short path. \
         Build/install staging and venv paths may need additional headroom.",
        path.display(),
    ))
}

fn absolute_consumer_path(path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let normalized = {
        use std::ffi::OsString;
        use std::path::{Component, Prefix};

        let mut components = path.components();
        let prefix = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::VerbatimDisk(drive) => {
                    Some(OsString::from(format!("{}:", char::from(drive))))
                }
                Prefix::VerbatimUNC(server, share) => {
                    let mut prefix = OsString::from(r"\\");
                    prefix.push(server);
                    prefix.push(r"\");
                    prefix.push(share);
                    Some(prefix)
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(prefix) = prefix {
            let mut normalized = PathBuf::from(prefix);
            normalized.extend(components.map(|component| component.as_os_str()));
            normalized
        } else {
            path.to_path_buf()
        }
    };
    #[cfg(not(windows))]
    let normalized = path.to_path_buf();

    std::path::absolute(normalized).map_err(|error| {
        format!(
            "Failed to resolve Python output path '{}': {error}",
            path.display()
        )
    })
}

fn utf16_length(path: &Path) -> usize {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::path::{Component, Prefix};

        let prefix_length = match path.components().next() {
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Verbatim(_)) => 4,
            _ => 0,
        };
        path.as_os_str().encode_wide().count() - prefix_length
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().encode_utf16().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dynwinrt_codegen::types::TypeIdentity;

    #[test]
    fn legacy_budget_boundaries_and_non_windows_policy() {
        for (length, risky) in [(259, false), (260, true), (261, true)] {
            let path = PathBuf::from(format!(r"C:\g\{}.py", "x".repeat(length - 8)));
            assert_eq!(utf16_length(&path), length);
            let message = warning_for_longest(&path, length, true);
            assert_eq!(message.is_some(), risky);
            if let Some(message) = message {
                assert!(message.contains(&format!("{length} UTF-16 code units")));
                assert!(message.contains(&format!("at least {} code units", length - 259)));
                assert!(message.contains("--output"));
            }
            assert!(warning_for_longest(&path, length, false).is_none());
        }
    }

    #[test]
    fn dry_run_paths_include_facades_indexes_and_only_requested_stubs() {
        let identity = TypeIdentity::named(TypeIdentityKind::Class, "Windows.Foundation", "Uri");
        let context = PythonProjectionContext::packaged([identity.clone()]).unwrap();
        let identities = [identity];
        let runtime = dry_run_source_files(&context, &identities, &identities, false);
        assert!(runtime.contains(&PathBuf::from("windows__foundation__uri.py")));
        assert!(runtime.contains(&Path::new("windows").join("foundation").join("uri.py")));
        assert!(runtime.contains(&Path::new("windows").join("foundation").join("__init__.py")));
        assert!(runtime.iter().all(|path| path.extension().unwrap() == "py"));
        let typed = dry_run_source_files(&context, &identities, &identities, true);
        assert!(typed.contains(&PathBuf::from("_implementation_types.pyi")));
        for path in runtime {
            assert!(typed.contains(&path));
            assert!(typed.contains(&path.with_extension("pyi")));
        }
    }

    #[cfg(windows)]
    #[test]
    fn normalized_drive_unc_and_extended_paths_have_the_same_budget() {
        for (input, expected) in [
            (r"C:\g\.\deep\..\bindings", r"C:\g\bindings"),
            (r"\\?\C:\g\.\deep\..\bindings", r"C:\g\bindings"),
            (
                r"\\server\share\deep\..\bindings",
                r"\\server\share\bindings",
            ),
            (
                r"\\?\UNC\server\share\deep\..\bindings",
                r"\\server\share\bindings",
            ),
        ] {
            let path = absolute_consumer_path(Path::new(input)).unwrap();
            assert_eq!(path, Path::new(expected));
            assert_eq!(utf16_length(&path), expected.encode_utf16().count());
        }
    }

    #[cfg(windows)]
    #[test]
    fn stubs_can_cross_the_budget_when_runtime_sources_do_not() {
        let runtime = PathBuf::from(format!("{}.py", "x".repeat(251)));
        assert!(
            warning(Path::new(r"C:\g"), &[runtime.clone()])
                .unwrap()
                .is_none()
        );
        let stub = runtime.with_extension("pyi");
        let message = warning(Path::new(r"\\?\C:\g"), &[runtime, stub])
            .unwrap()
            .unwrap();
        assert!(message.contains("260 UTF-16 code units"));
        assert!(message.contains(".pyi'"));
        assert!(!message.contains(r"\\?\"));
    }

    #[cfg(windows)]
    #[test]
    fn public_facades_can_be_longer_than_implementation_modules() {
        let namespace = format!(
            "Contoso.{}.{}.{}",
            "a".repeat(90),
            "b".repeat(90),
            "c".repeat(90),
        );
        let identity = TypeIdentity::named(TypeIdentityKind::Class, namespace, "Widget");
        let context = PythonProjectionContext::packaged([identity.clone()]).unwrap();
        let implementation =
            PathBuf::from(format!("{}.pyi", context.implementation_module(&identity)));
        assert!(
            warning(Path::new(r"C:\g"), &[implementation])
                .unwrap()
                .is_none()
        );
        let identities = [identity];
        let files = dry_run_source_files(&context, &identities, &identities, true);
        let message = warning(Path::new(r"C:\g"), &files).unwrap().unwrap();
        assert!(message.contains(r"C:\g\contoso\"));
        assert!(message.contains(".pyi'"));
    }

    #[cfg(windows)]
    #[test]
    fn relative_paths_and_non_ascii_are_measured_in_utf16() {
        let relative = Path::new(r"parent\..\generated");
        let absolute = std::env::current_dir().unwrap().join("generated");
        assert_eq!(absolute_consumer_path(relative).unwrap(), absolute);
        let module = PathBuf::from(format!("{}.py", "\u{1f642}".repeat(126)));
        let path = absolute.join(&module);
        let message = warning(relative, &[module]).unwrap().unwrap();
        assert!(message.contains(&path.display().to_string()));
        assert!(message.contains(&format!("{} UTF-16 code units", utf16_length(&path))));
        assert_ne!(utf16_length(&path), path.to_string_lossy().chars().count());
    }

    #[test]
    fn ordinary_outputs_and_non_python_files_do_not_warn() {
        assert!(
            warning(Path::new(r"C:\g"), &[PathBuf::from("uri.py")])
                .unwrap()
                .is_none()
        );
        assert!(
            warning(
                Path::new("generated"),
                &[PathBuf::from(format!("{}.js", "x".repeat(300)))]
            )
            .unwrap()
            .is_none()
        );
        if !cfg!(windows) {
            assert!(
                warning(
                    Path::new("generated"),
                    &[PathBuf::from(format!("{}.py", "x".repeat(300)))]
                )
                .unwrap()
                .is_none()
            );
        }
    }
}
