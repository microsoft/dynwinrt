// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Exact metadata relationship and projection for the first native one-shot
//! consumer. DLL exports are not represented as fake COM vtable methods.

use crate::com_metadata::{ComInterfaceMeta, RawComMethod, completion::*, raw_method_shape};

use super::{
    ComGeneratedOutput,
    ir::{
        NativeCompletionHandler, NativeCompletionResult, NativeCompletionResultParameter,
        NativeCompletionStartParameter, ProjectedNativeCompletion,
    },
};

const HANDLER_IID: &str = "41d949ab-9862-444a-80f6-c261334da5eb";
const OPERATION_IID: &str = "72a22d78-cde4-431d-b8cc-843a71199b6d";
const TARGETS: [(&str, &str, &str); 2] = [
    (
        AUDIO_NAMESPACE,
        "IAudioClient",
        "1cb9ad4c-dbfa-4c32-b178-c2f568a703b2",
    ),
    (
        "Windows.Win32.Media.Audio.Endpoints",
        "IAudioEndpointVolume",
        "5cdf2c82-841e-4546-9722-0cf74078229a",
    ),
];
const START_SHAPE: &str = "Windows.Win32.Media.Audio.Apis!ActivateAudioInterfaceAsync:MMDevAPI.dll:system:true(deviceInterfacePath:In:false:true:Windows.Win32.Foundation.PWSTR[Struct]/ptr0/Unspecified/underlying=char16/ptr1/Mutable,riid:In:false:true:System.Guid[Unknown]/ptr1/Mutable,activationParams:In:true:false:Windows.Win32.System.Com.StructuredStorage.PROPVARIANT[Struct]/ptr1/Mutable,completionHandler:In:false:false:Windows.Win32.Media.Audio.IActivateAudioInterfaceCompletionHandler[Interface]{41d949ab-9862-444a-80f6-c261334da5eb}/ptr0/Unspecified,activationOperation:Out:false:false:Windows.Win32.Media.Audio.IActivateAudioInterfaceAsyncOperation[Interface]{72a22d78-cde4-431d-b8cc-843a71199b6d}/ptr1/Mutable)->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:semantic=false";
const HANDLER_SHAPE: &str = "ActivateCompleted@3(activateOperation:in:required:noconstattr:Windows.Win32.Media.Audio.IActivateAudioInterfaceAsyncOperation[Interface]{72a22d78-cde4-431d-b8cc-843a71199b6d}/ptr0/Unspecified)->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:plain_hresult:not_enumerator_next";
const RESULT_SHAPE: &str = "GetActivateResult@3(activateResult:out:required:noconstattr:Windows.Win32.Foundation.HRESULT[Struct]/ptr1/Mutable/underlying=i32/ptr0/Unspecified,activatedInterface:out:required:noconstattr:Windows.Win32.System.Com.IUnknown[Interface]{00000000-0000-0000-c000-000000000046}/ptr1/Mutable)->Windows.Win32.Foundation.HRESULT[Struct]/ptr0/Unspecified/underlying=i32/ptr0/Unspecified:plain_hresult:not_enumerator_next";

fn validate_interface<'a>(
    interface: &'a ComInterfaceMeta,
    name: &str,
    iid: &str,
    shape: &str,
) -> Result<&'a RawComMethod, String> {
    let raw = interface.raw_methods.as_deref().unwrap_or_default();
    if interface.interface.namespace != AUDIO_NAMESPACE
        || interface.interface.name != name
        || interface.interface.iid != iid
        || !interface.is_iunknown_rooted
        || interface.base_offset != 3
        || interface.own_methods_start != 3
        || interface.base_chain != ["IUnknown"]
        || !interface.base_iids.is_empty()
        || interface.interface.generic_piid.is_some()
        || !interface.interface.generic_args.is_empty()
        || interface.interface.methods.len() != 1
        || raw.len() != 1
    {
        return Err(format!(
            "Native completion interface identity/inheritance drift: {name}"
        ));
    }
    let raw = &raw[0];
    if raw.declaring_namespace != AUDIO_NAMESPACE
        || raw.declaring_interface != name
        || raw.declaring_iid != iid
        || raw_method_shape(raw) != shape
        || raw.exact_contract.is_some()
        || raw.exact_interface_output_call.is_some()
        || !raw.interface_replacement_contracts.is_empty()
        || !raw.output_ownership_contracts.is_empty()
        || !raw.exact_null_input_contracts.is_empty()
        || !raw.exact_parameter_direction_contracts.is_empty()
        || raw.safe_array_contract_error.is_some()
        || raw.params.iter().any(|param| {
            param.exact_interface_output.is_some() || param.safe_array_evidence.is_some()
        })
    {
        return Err(format!("Native completion method contract drift: {name}"));
    }
    Ok(raw)
}

pub fn validate_audio_completion(facts: &AudioCompletionFacts) -> Result<(), String> {
    if facts.start.name != AUDIO_EXPORT
        || facts.start.shape() != START_SHAPE
        || facts.start.params.iter().any(|param| {
            param.native_array.is_some()
                || param.string_pointer_array.is_some()
                || param.free_with.is_some()
                || param.safe_array_evidence.is_some()
                || param.exact_interface_output.is_some()
        })
    {
        return Err(
            "Native completion DLL export contract drift: ActivateAudioInterfaceAsync".into(),
        );
    }
    validate_interface(&facts.handler, HANDLER_NAME, HANDLER_IID, HANDLER_SHAPE)?;
    validate_interface(
        &facts.operation,
        OPERATION_NAME,
        OPERATION_IID,
        RESULT_SHAPE,
    )?;
    if facts.targets.len() != TARGETS.len() {
        return Err("Native audio completion target set drift".into());
    }
    for (interface, (namespace, name, iid)) in facts.targets.iter().zip(TARGETS) {
        if interface.interface.namespace != namespace
            || interface.interface.name != name
            || interface.interface.iid != iid
            || !interface.is_iunknown_rooted
            || interface.raw_methods.is_none()
        {
            return Err(format!(
                "Native audio completion target identity drift: {name}"
            ));
        }
    }
    Ok(())
}

fn project(facts: &AudioCompletionFacts) -> Result<ProjectedNativeCompletion, String> {
    validate_audio_completion(facts)?;
    Ok(ProjectedNativeCompletion {
        namespace: AUDIO_NAMESPACE,
        function_name: "activateAudioInterfaceAsync",
        version: 1,
        kind: "audio-activation",
        library: facts.start.dll.clone(),
        export: facts.start.entry_point.clone(),
        calling_convention: "system",
        has_this: false,
        start_parameters: vec![
            NativeCompletionStartParameter::Utf16Path,
            NativeCompletionStartParameter::RefIid,
            NativeCompletionStartParameter::NullPropVariant,
            NativeCompletionStartParameter::NativeSignal,
            NativeCompletionStartParameter::OwnedOperation,
        ],
        handler: NativeCompletionHandler {
            iid: facts.handler.interface.iid.clone(),
            root: "IUnknown",
            slot: facts.handler.raw_methods.as_ref().unwrap()[0].vtable_index,
            input: "borrowed-operation",
            return_type: "hresult",
        },
        result: NativeCompletionResult {
            iid: facts.operation.interface.iid.clone(),
            root: "IUnknown",
            slot: facts.operation.raw_methods.as_ref().unwrap()[0].vtable_index,
            // GetActivateResult docs supply pointee nullability on inner
            // failure; winmd's required flag describes the receiving cell.
            outputs: vec![
                NativeCompletionResultParameter::Hresult,
                NativeCompletionResultParameter::NullableOwnedInterface,
            ],
            return_type: "hresult",
        },
        allowed_targets: facts
            .targets
            .iter()
            .map(|target| target.interface.iid.clone())
            .collect(),
    })
}

pub fn generate_audio_completion_files(winmd_paths: &str) -> Result<ComGeneratedOutput, String> {
    let facts = parse_audio_completion(winmd_paths)?;
    let projected = project(&facts)?;
    let mut output = super::javascript::render::render_native_completion(&projected)?;
    // Validate and emit every method of each result projection. A partially
    // supported result type must not become a safe entry point.
    let mut extras = std::collections::BTreeMap::new();
    for target in &facts.targets {
        let files = super::generate_com_interface_files(target, winmd_paths)?;
        let module =
            super::canonical_module_path(&target.interface.namespace, &target.interface.name)?;
        extras.insert(format!("{module}.js"), files.js);
        extras.insert(format!("{module}.d.ts"), files.dts);
        for (path, content) in files.extra_files {
            if let Some(existing) = extras.insert(path.clone(), content.clone())
                && existing != content
            {
                return Err(format!("Conflicting native completion dependency: {path}"));
            }
        }
    }
    output.extra_files = extras.into_iter().collect();
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com_metadata::{RawNativeType, RawParamDirection};

    #[test]
    fn native_completion_projection_and_rendering_have_explicit_roles() {
        let plan = ProjectedNativeCompletion {
            namespace: AUDIO_NAMESPACE,
            function_name: "renamedActivation",
            version: 1,
            kind: "audio-activation",
            library: "MMDevAPI.dll".into(),
            export: AUDIO_EXPORT.into(),
            calling_convention: "system",
            has_this: false,
            start_parameters: vec![
                NativeCompletionStartParameter::Utf16Path,
                NativeCompletionStartParameter::RefIid,
                NativeCompletionStartParameter::NullPropVariant,
                NativeCompletionStartParameter::NativeSignal,
                NativeCompletionStartParameter::OwnedOperation,
            ],
            handler: NativeCompletionHandler {
                iid: HANDLER_IID.into(),
                root: "IUnknown",
                slot: 3,
                input: "borrowed-operation",
                return_type: "hresult",
            },
            result: NativeCompletionResult {
                iid: OPERATION_IID.into(),
                root: "IUnknown",
                slot: 3,
                outputs: vec![
                    NativeCompletionResultParameter::Hresult,
                    NativeCompletionResultParameter::NullableOwnedInterface,
                ],
                return_type: "hresult",
            },
            allowed_targets: TARGETS.iter().map(|(_, _, iid)| iid.to_string()).collect(),
        };
        let descriptor = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            descriptor["start_parameters"],
            serde_json::json!([
                "utf16-path",
                "ref-iid",
                "null-prop-variant",
                "native-signal",
                "owned-operation",
            ])
        );
        assert_eq!(
            descriptor["result"]["outputs"],
            serde_json::json!(["hresult", "nullable-owned-interface"])
        );
        assert!(descriptor.get("function_name").is_none());
        let rendered = super::super::javascript::render::render_native_completion(&plan).unwrap();
        assert!(
            rendered
                .js
                .contains("exports.renamedActivation = renamedActivation")
        );
        assert!(rendered.js.contains("arguments.length !== 2"));
        assert!(rendered.dts.contains("renamedActivation<T>"));
        assert!(rendered.dts.contains("Promise<T>"));
    }

    #[test]
    fn exact_winmd_completion_relationship_and_fail_closed_drift() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        let facts = parse_audio_completion(&winmd).unwrap();
        validate_audio_completion(&facts).unwrap();
        let output = generate_audio_completion_files(&winmd).unwrap();
        assert!(output.js.contains("exports.activateAudioInterfaceAsync"));
        assert!(output.js.contains("\\\"has_this\\\":false"));
        assert!(output.js.contains("null-prop-variant"));
        assert!(output.js.contains("nullable-owned-interface"));
        assert!(output.dts.contains("Promise<T>"));
        assert!(!output.js.contains("IAsyncInfo"));
        assert!(!output.js.contains("implement("));
        assert!(
            output
                .extra_files
                .iter()
                .any(|(name, _)| name.ends_with("/IAudioClient.js"))
        );
        assert!(
            output
                .extra_files
                .iter()
                .any(|(name, _)| name.ends_with("/IAudioEndpointVolume.js"))
        );
        let reject = |mutate: fn(&mut AudioCompletionFacts)| {
            let mut changed = facts.clone();
            mutate(&mut changed);
            assert!(validate_audio_completion(&changed).is_err());
        };
        let mutations: &[fn(&mut AudioCompletionFacts)] = &[
            |f: &mut AudioCompletionFacts| f.start.is_static = false,
            |f| f.start.calling_convention = "cdecl".into(),
            |f| f.start.dll = "untrusted.dll".into(),
            |f| f.start.entry_point = "Other".into(),
            |f| f.start.params.swap(0, 1),
            |f| f.start.params[1].typ.pointer_depth = 0,
            |f| f.start.params[2].optional = false,
            |f| f.start.params[2].typ.pointer_depth = 2,
            |f| f.start.params[3].direction = RawParamDirection::Out,
            |f| f.start.params[4].typ.pointer_depth = 0,
            |f| f.start.semantic_hresult = true,
            |f| f.start.return_type.native_type = RawNativeType::Void,
            |f| f.handler.base_chain.push("IForeign".into()),
            |f| f.handler.interface.iid = OPERATION_IID.into(),
            |f| f.handler.raw_methods.as_mut().unwrap()[0].vtable_index = 4,
            |f| f.handler.raw_methods.as_mut().unwrap()[0].params[0].optional = true,
            |f| {
                f.operation.raw_methods.as_mut().unwrap()[0].params[0]
                    .typ
                    .pointer_depth = 0
            },
            |f| f.operation.raw_methods.as_mut().unwrap()[0].params[1].optional = true,
            |f| {
                f.operation.raw_methods.as_mut().unwrap()[0].params[1]
                    .typ
                    .pointer_depth = 2
            },
            |f| {
                f.operation
                    .interface
                    .methods
                    .push(f.operation.interface.methods[0].clone())
            },
            |f| f.targets[0].interface.iid = HANDLER_IID.into(),
        ];
        for mutate in mutations {
            reject(*mutate);
        }
    }
}
