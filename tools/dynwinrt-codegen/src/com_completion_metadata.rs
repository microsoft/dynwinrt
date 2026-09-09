// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Raw metadata for the bounded DLL-export / native-completion relationship.

use super::*;

pub const AUDIO_NAMESPACE: &str = "Windows.Win32.Media.Audio";
pub const AUDIO_EXPORT: &str = "ActivateAudioInterfaceAsync";
pub const HANDLER_NAME: &str = "IActivateAudioInterfaceCompletionHandler";
pub const OPERATION_NAME: &str = "IActivateAudioInterfaceAsyncOperation";

#[derive(Debug, Clone)]
pub struct RawCompletionExport {
    pub namespace: String,
    pub container: String,
    pub name: String,
    pub dll: String,
    pub entry_point: String,
    pub calling_convention: String,
    pub is_static: bool,
    pub params: Vec<RawComParam>,
    pub return_type: RawComType,
    pub semantic_hresult: bool,
}

impl RawCompletionExport {
    pub fn shape(&self) -> String {
        let params = self
            .params
            .iter()
            .map(|param| {
                let mut typ = String::new();
                push_raw_type_shape(&mut typ, &param.typ);
                format!(
                    "{}:{:?}:{}:{}:{typ}",
                    param.name, param.direction, param.optional, param.const_attribute,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut result = String::new();
        push_raw_type_shape(&mut result, &self.return_type);
        format!(
            "{}.{}!{}:{}:{}:{}({params})->{result}:semantic={}",
            self.namespace,
            self.container,
            self.entry_point,
            self.dll,
            self.calling_convention,
            self.is_static,
            self.semantic_hresult,
        )
    }
}

#[derive(Debug, Clone)]
pub struct AudioCompletionFacts {
    pub start: RawCompletionExport,
    pub handler: ComInterfaceMeta,
    pub operation: ComInterfaceMeta,
    pub targets: Vec<ComInterfaceMeta>,
}

pub fn parse_audio_completion(winmd_paths: &str) -> Result<AudioCompletionFacts, String> {
    let index = crate::meta::load_index(winmd_paths)
        .ok_or("Cannot load metadata for native audio completion")?;
    let definitions = index.get(AUDIO_NAMESPACE, "Apis").collect::<Vec<_>>();
    let candidates = definitions
        .iter()
        .flat_map(|def| def.methods())
        .filter(|method| method.name() == AUDIO_EXPORT)
        .collect::<Vec<_>>();
    let [method] = candidates.as_slice() else {
        return Err("Expected exactly one Audio.Apis.ActivateAudioInterfaceAsync export".into());
    };
    let import = method
        .impl_map()
        .ok_or("Audio activation is not a DLL export")?;
    let signature = method.signature(&[]);
    let parameters = method
        .params()
        .filter(|param| param.sequence() > 0)
        .collect::<Vec<_>>();
    if parameters.len() != signature.types.len() {
        return Err("Incomplete audio activation parameter metadata".into());
    }
    let params = parameters
        .iter()
        .zip(&signature.types)
        .map(|(param, typ)| {
            if param.has_attribute("NativeArrayInfoAttribute")
                || param.has_attribute("FreeWithAttribute")
            {
                return Err("Unexpected array or allocator on audio activation input".into());
            }
            Ok(RawComParam {
                name: param.name().into(),
                typ: map_raw_com_type(typ, &index),
                direction: raw_param_direction(param.flags()),
                optional: param
                    .flags()
                    .contains(windows_metadata::ParamAttributes::Optional),
                const_attribute: param.has_attribute("ConstAttribute"),
                native_array: None,
                string_pointer_array: None,
                free_with: None,
                safe_array_evidence: None,
                exact_interface_output: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let interface = |name: &str, namespace: &str| {
        parse_com_interface_from_index(&index, namespace, name)
            .ok_or_else(|| format!("Missing native completion interface {namespace}.{name}"))
    };
    Ok(AudioCompletionFacts {
        start: RawCompletionExport {
            namespace: AUDIO_NAMESPACE.into(),
            container: "Apis".into(),
            name: method.name().into(),
            dll: import.import_scope().name().into(),
            entry_point: import.import_name().into(),
            calling_convention: method.calling_convention().into(),
            is_static: signature.flags == windows_metadata::MethodCallAttributes::default(),
            params,
            return_type: map_raw_com_type(&signature.return_type, &index),
            semantic_hresult: method.has_attribute("CanReturnMultipleSuccessValuesAttribute"),
        },
        handler: interface(HANDLER_NAME, AUDIO_NAMESPACE)?,
        operation: interface(OPERATION_NAME, AUDIO_NAMESPACE)?,
        targets: vec![
            interface("IAudioClient", AUDIO_NAMESPACE)?,
            interface(
                "IAudioEndpointVolume",
                "Windows.Win32.Media.Audio.Endpoints",
            )?,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_completion_metadata_shape() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let facts = parse_audio_completion(&winmd).unwrap();
        assert!(facts.start.is_static);
        assert_eq!(facts.start.params.len(), 5);
        assert_eq!(facts.start.calling_convention, "system");
        assert_eq!(facts.start.dll, "MMDevAPI.dll");
        assert_eq!(facts.handler.base_chain, ["IUnknown"]);
        assert_eq!(facts.operation.base_chain, ["IUnknown"]);
        assert_eq!(facts.handler.raw_methods.as_ref().unwrap().len(), 1);
        assert_eq!(facts.operation.raw_methods.as_ref().unwrap().len(), 1);
        assert_eq!(facts.targets.len(), 2);
    }
}
