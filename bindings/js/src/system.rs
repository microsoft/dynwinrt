// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use napi_derive::napi;

#[napi]
pub fn has_package_identity() -> bool {
  use windows::ApplicationModel::AppInfo;
  match AppInfo::Current() {
    Ok(_) => true,
    Err(_) => false,
  }
}

#[napi]
pub fn get_computer_name() -> napi::Result<String> {
  #[cfg(target_os = "windows")]
  {
    use windows::core::PWSTR;
    use windows::Win32::System::WindowsProgramming::GetComputerNameW;

    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;

    unsafe {
      if GetComputerNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
        let name = String::from_utf16_lossy(&buffer[..size as usize]);
        Ok(name)
      } else {
        Err(napi::Error::from_reason("Failed to get computer name"))
      }
    }
  }

  #[cfg(not(target_os = "windows"))]
  {
    Err(napi::Error::from_reason(
      "This function is only available on Windows",
    ))
  }
}

#[napi]
pub fn get_windows_directory() -> napi::Result<String> {
  #[cfg(target_os = "windows")]
  {
    use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = [0u16; 260]; // MAX_PATH

    unsafe {
      let len = GetWindowsDirectoryW(Some(&mut buffer));
      if len > 0 {
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Ok(path)
      } else {
        Err(napi::Error::from_reason("Failed to get Windows directory"))
      }
    }
  }

  #[cfg(not(target_os = "windows"))]
  {
    Err(napi::Error::from_reason(
      "This function is only available on Windows",
    ))
  }
}
