// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::mem::size_of;
use std::path::Path;

use windows::ApplicationModel::PackageVersion;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FreeLibrary};
use windows::Win32::Storage::Packaging::Appx::{
    GetCurrentPackageInfo2, PACKAGE_FILTER_DIRECT, PACKAGE_FILTER_DYNAMIC, PACKAGE_INFO,
    PackagePathType_Install,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::PCSTR;
use windows::core::PCWSTR;
use windows_core::{Error, HRESULT, HSTRING};

pub struct WinAppSdkContext;

const E_FAIL: HRESULT = HRESULT(0x80004005_u32 as i32);
const E_INVALIDARG: HRESULT = HRESULT(0x80070057_u32 as i32);
const WINAPPSDK_BOOTSTRAP_DLL_PATH_ENV: &str = "WINAPPSDK_BOOTSTRAP_DLL_PATH";
const WINAPPSDK_RUNTIME_PACKAGE_PREFIX: &str = "Microsoft.WindowsAppRuntime.";

#[derive(Debug, Clone)]
pub struct WinAppSdkBootstrapOptions {
    pub major_version: u32,
    pub minor_version: u32,
    pub build_version: u32,
    pub revision_version: u32,
    pub bootstrap_dll_path: Option<String>,
}

pub fn initialize_winappsdk(major: u32, minor: u32) -> crate::result::Result<WinAppSdkContext> {
    let options = WinAppSdkBootstrapOptions {
        major_version: major,
        minor_version: minor,
        build_version: 0,
        revision_version: 0,
        bootstrap_dll_path: None,
    };
    initialize(options).map_err(|e| e.into())
}

pub fn initialize(options: WinAppSdkBootstrapOptions) -> windows::core::Result<WinAppSdkContext> {
    let (major_minor_version, min_version) = bootstrap_version(&options)?;
    let dll_path = options
        .bootstrap_dll_path
        .or_else(|| std::env::var(WINAPPSDK_BOOTSTRAP_DLL_PATH_ENV).ok())
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            Error::new(
                E_INVALIDARG,
                format!(
                    "Windows App SDK bootstrap DLL path is required; set {WINAPPSDK_BOOTSTRAP_DLL_PATH_ENV} or provide bootstrap_dll_path"
                ),
            )
        })?;
    let dll_path = HSTRING::from(dll_path);

    let dp = PCWSTR::from_raw(dll_path.as_ptr());
    let module = unsafe { LoadLibraryW(dp) }?;
    let proc = unsafe {
        GetProcAddress(
            module,
            PCSTR::from_raw(b"MddBootstrapInitialize2\0".as_ptr()),
        )
    };
    let Some(proc) = proc else {
        unsafe {
            let _ = FreeLibrary(module);
        }
        return Err(Error::new(
            E_FAIL,
            "MddBootstrapInitialize2 was not found in the Windows App SDK bootstrap DLL",
        ));
    };

    let init: MddBootstrapInitialize2 = unsafe { std::mem::transmute(proc) };
    let hr = unsafe { init(major_minor_version, PCWSTR::null(), min_version, 0) };
    if let Err(error) = hr.ok() {
        unsafe {
            let _ = FreeLibrary(module);
        }
        return Err(error);
    }

    // The package graph remains active for the process lifetime, so the bootstrap DLL
    // must remain loaded after this function returns.
    Ok(WinAppSdkContext {})
}

impl WinAppSdkContext {
    pub fn resource_pri_path(&self) -> windows::core::Result<String> {
        let (buffer, count) = current_package_graph()?;
        let package_info = buffer.as_ptr().cast::<PACKAGE_INFO>();

        for index in 0..count {
            let info = unsafe { package_info.add(index).read_unaligned() };
            let package_full_name_ptr = info.packageFullName;
            let package_full_name = unsafe { package_full_name_ptr.to_string() }?;
            if !package_full_name.starts_with(WINAPPSDK_RUNTIME_PACKAGE_PREFIX) {
                continue;
            }

            let package_path_ptr = info.path;
            let package_path = unsafe { package_path_ptr.to_string() }?;
            let resource_pri = Path::new(&package_path).join("resources.pri");
            if resource_pri.is_file() {
                return Ok(resource_pri.to_string_lossy().into_owned());
            }
        }

        Err(Error::new(
            E_FAIL,
            "The initialized Windows App SDK runtime package was not found in the current package graph",
        ))
    }
}

fn current_package_graph() -> windows::core::Result<(Vec<usize>, usize)> {
    // Identityless bootstrap dependencies are visible only through the dynamic filter.
    let flags = PACKAGE_FILTER_DIRECT | PACKAGE_FILTER_DYNAMIC;
    let mut buffer_length = 0;
    let mut count = 0;
    let status = unsafe {
        GetCurrentPackageInfo2(
            flags,
            PackagePathType_Install,
            &mut buffer_length,
            None,
            Some(&mut count),
        )
    };
    if status != ERROR_INSUFFICIENT_BUFFER {
        status.ok()?;
    }

    let word_count = (buffer_length as usize).div_ceil(size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    let status = unsafe {
        GetCurrentPackageInfo2(
            flags,
            PackagePathType_Install,
            &mut buffer_length,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut count),
        )
    };
    status.ok()?;

    Ok((buffer, count as usize))
}

fn bootstrap_version(
    options: &WinAppSdkBootstrapOptions,
) -> windows::core::Result<(u32, PackageVersion)> {
    let major = version_component(options.major_version, "major")?;
    let minor = version_component(options.minor_version, "minor")?;
    let build = version_component(options.build_version, "build")?;
    let revision = version_component(options.revision_version, "revision")?;

    Ok((
        (u32::from(major) << 16) | u32::from(minor),
        PackageVersion {
            Major: major,
            Minor: minor,
            Build: build,
            Revision: revision,
        },
    ))
}

fn version_component(value: u32, name: &str) -> windows::core::Result<u16> {
    u16::try_from(value).map_err(|_| {
        Error::new(
            E_INVALIDARG,
            format!("Windows App SDK {name} version component must be between 0 and 65535"),
        )
    })
}

#[allow(dead_code)]
pub fn find_winappsdk_package(
    major: u32,
    minor: u32,
) -> windows::core::Result<Vec<windows::ApplicationModel::Package>> {
    use windows::Management::Deployment::{PackageManager, PackageTypes};
    use windows_core::HSTRING;

    let manager = PackageManager::new()?;
    let family = format!("Microsoft.WindowsAppRuntime.{major}.{minor}_8wekyb3d8bbwe");
    let packages = manager.FindPackagesByUserSecurityIdPackageFamilyNameWithPackageTypes(
        &HSTRING::default(),
        &HSTRING::from(family),
        PackageTypes::Framework,
    )?;

    let packages: Vec<windows::ApplicationModel::Package> = packages.into_iter().collect();
    Ok(packages)
}

type MddBootstrapInitialize2 =
    unsafe extern "system" fn(u32, PCWSTR, PackageVersion, u32) -> HRESULT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_version_preserves_release_and_minimum_version() {
        let options = WinAppSdkBootstrapOptions {
            major_version: 2,
            minor_version: 2,
            build_version: 3,
            revision_version: 4,
            bootstrap_dll_path: None,
        };

        let (major_minor, minimum) = bootstrap_version(&options).unwrap();

        assert_eq!(major_minor, 0x0002_0002);
        assert_eq!(minimum.Major, 2);
        assert_eq!(minimum.Minor, 2);
        assert_eq!(minimum.Build, 3);
        assert_eq!(minimum.Revision, 4);
    }

    #[test]
    fn bootstrap_version_rejects_components_larger_than_u16() {
        let options = WinAppSdkBootstrapOptions {
            major_version: u16::MAX as u32 + 1,
            minor_version: 0,
            build_version: 0,
            revision_version: 0,
            bootstrap_dll_path: None,
        };

        assert!(bootstrap_version(&options).is_err());
    }

    #[test]
    #[ignore] // Requires WINAPPSDK_BOOTSTRAP_DLL_PATH env variable
    fn test_initialize() {
        let options = WinAppSdkBootstrapOptions {
            major_version: 1,
            minor_version: 8,
            build_version: 0,
            revision_version: 0,
            bootstrap_dll_path: None,
        };
        let context = initialize(options).unwrap();
        assert!(Path::new(&context.resource_pri_path().unwrap()).is_file());
    }
}
