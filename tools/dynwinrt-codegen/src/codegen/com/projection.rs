// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use crate::com_metadata::{ComInterfaceMeta, MethodMeta, ParamDirection, ParamMeta};
use crate::types::TypeMeta;

use super::naming::camel_case;
use super::type_mapping::is_hresult;

#[derive(Debug, Clone)]
pub(super) struct InteropMethod {
    pub(super) camel: String,
    pub(super) vtable_index: usize,
    pub(super) natural_params: Option<Vec<ParamMeta>>,
    pub(super) plain: Option<MethodMeta>,
    pub(super) _doc: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct InteropInfo {
    pub(super) methods: Vec<InteropMethod>,
    pub(super) class_name: String,
    pub(super) class_namespace: String,
    pub(super) target_iid: String,
}

pub(super) fn method_is_interop_shape(m: &MethodMeta) -> Option<Vec<ParamMeta>> {
    match &m.return_type {
        Some(t) if is_hresult(t) => {}
        _ => return None,
    }
    if m.params.len() < 2 {
        return None;
    }
    let last_idx = m.params.len() - 1;
    let out_param = &m.params[last_idx];
    if out_param.direction != ParamDirection::Out || !matches!(out_param.typ, TypeMeta::Object) {
        return None;
    }
    if m.params[..last_idx]
        .iter()
        .any(|param| param.direction != ParamDirection::In)
    {
        return None;
    }
    let riid = &m.params[last_idx - 1];
    let is_riid = match &riid.typ {
        TypeMeta::Guid => true,
        TypeMeta::Object => {
            let name = riid.name.to_ascii_lowercase();
            name == "riid" || name == "iid"
        }
        _ => false,
    };
    is_riid.then(|| m.params[..last_idx - 1].to_vec())
}

pub(super) fn detect_interop(
    meta: &ComInterfaceMeta,
    winmd_paths: &str,
) -> Result<Option<InteropInfo>, String> {
    let iface = &meta.interface;
    if !iface.name.ends_with("Interop") || iface.methods.is_empty() {
        return Ok(None);
    }

    let mut has_interop_method = false;
    let methods = iface
        .methods
        .iter()
        .map(|method| match method_is_interop_shape(method) {
            Some(natural_params) if method.name == "GetForWindow" => {
                has_interop_method = true;
                InteropMethod {
                    camel: camel_case(&method.name),
                    vtable_index: method.vtable_index,
                    natural_params: Some(natural_params),
                    plain: None,
                    _doc: method.doc.clone(),
                }
            }
            _ => InteropMethod {
                camel: camel_case(&method.name),
                vtable_index: method.vtable_index,
                natural_params: None,
                plain: Some(method.clone()),
                _doc: method.doc.clone(),
            },
        })
        .collect();
    if !has_interop_method {
        return Ok(None);
    }

    let stripped_i = iface.name.strip_prefix('I').unwrap_or(&iface.name);
    let class_name = stripped_i
        .strip_suffix("Interop")
        .unwrap_or(stripped_i)
        .to_string();
    let (class_namespace, target_iid) = match resolve_projected_default_iid(
        winmd_paths,
        &class_name,
    ) {
        Some((namespace, _interface_name, iid)) => (namespace, iid),
        None => {
            return Err(format!(
                "Classic-COM interop generator: cannot resolve default IID for the projected \
                     WinRT runtime class `{class_name}` (derived from `{}`). \
                     Neither the winmds passed to the generator ({winmd_paths:?}) nor the newest installed \
                     `C:\\Program Files (x86)\\Windows Kits\\10\\UnionMetadata\\<version>\\Windows.winmd` \
                     contains a WinRT runtime class of that name with a resolvable default interface. \
                     Pass the correct Windows.winmd via --ref or install a recent Windows SDK.",
                iface.name
            ));
        }
    };

    Ok(Some(InteropInfo {
        methods,
        class_name,
        class_namespace,
        target_iid,
    }))
}

fn resolve_projected_default_iid(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interop_shape_rejects_non_refiid_trailing_object() {
        let method = MethodMeta {
            params: vec![
                ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "result".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(TypeMeta::Struct {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HRESULT".into(),
                fields: Vec::new(),
            }),
            ..Default::default()
        };

        assert!(method_is_interop_shape(&method).is_none());
    }

    #[test]
    fn non_get_for_window_interop_keeps_caller_iid() {
        let method = MethodMeta {
            name: "CreateSessionForWindow".into(),
            params: vec![
                ParamMeta {
                    name: "window".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "riid".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::In,
                },
                ParamMeta {
                    name: "result".into(),
                    typ: TypeMeta::Object,
                    direction: ParamDirection::Out,
                },
            ],
            return_type: Some(TypeMeta::Struct {
                namespace: "Windows.Win32.Foundation".into(),
                name: "HRESULT".into(),
                fields: Vec::new(),
            }),
            ..Default::default()
        };
        let meta = ComInterfaceMeta {
            interface: crate::com_metadata::InterfaceMeta {
                name: "IUserActivityInterop".into(),
                namespace: "Windows.Win32.System.WinRT".into(),
                iid: "00000000-0000-0000-0000-000000000000".into(),
                methods: vec![method],
                ..Default::default()
            },
            base_offset: 3,
            is_iunknown_rooted: true,
            base_chain: vec!["IUnknown".into()],
            coclass_clsid: None,
            coclass_name: None,
            own_methods_start: 3,
            referenced_enums: Vec::new(),
        };

        assert!(detect_interop(&meta, "").unwrap().is_none());
    }
}
