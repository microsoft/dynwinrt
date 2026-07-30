// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

pub(super) fn resolve_projected_default_iid(
    winmd_paths: &str,
    simple_class_name: &str,
) -> Option<(String, String, String)> {
    if !winmd_paths.is_empty() {
        if let Some(result) =
            crate::com_metadata::find_runtime_class_default_iid(winmd_paths, simple_class_name)
        {
            return Some(result);
        }
    }
    let sdk_winmd = crate::com_metadata::discover_newest_windows_winmd()?;
    if winmd_paths
        .split(';')
        .any(|path| path.eq_ignore_ascii_case(&sdk_winmd))
    {
        return None;
    }
    crate::com_metadata::find_runtime_class_default_iid(&sdk_winmd, simple_class_name)
}
