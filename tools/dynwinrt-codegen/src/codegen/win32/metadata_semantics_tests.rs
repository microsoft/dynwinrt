// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::{ir::*, test_support::*, *};
use crate::win32_metadata::*;

#[test]
fn metadata_complete_exports_do_not_require_method_allowlisting() {
    let Some(path) = metadata() else { return };
    for (namespace, name, abi, shape) in [
        (
            "Windows.Win32.System.SystemInformation",
            "GetTickCount",
            AbiType::U32,
            SurfaceType::Number,
        ),
        (
            "Windows.Win32.System.SystemInformation",
            "GetTickCount64",
            AbiType::U64,
            SurfaceType::BigInt,
        ),
    ] {
        let raw = metadata_function(&path, namespace, name);
        assert!(
            crate::win32_contracts::Registry::builtin()
                .unwrap()
                .function_entry(&raw)
                .is_none()
        );
        let projected = project_one(raw);
        assert!(projected.parameters.is_empty() && projected.inputs.is_empty());
        assert_eq!(projected.runtime.return_abi, Some(abi));
        assert_eq!(projected.runtime.success_rule, SuccessRule::Always);
        assert_eq!(projected.runtime.return_cleanup, Cleanup::None);
        assert_eq!(
            projected.runtime.call_contract,
            CallContract::defaults(&projected.runtime.signature_shape(64))
        );
        assert!(matches!(&projected.return_shape,ReturnShape::Direct { typ,.. } if typ == &shape));
    }
    let function = metadata_function(
        &path,
        "Windows.Win32.System.SystemInformation",
        "GetFirmwareType",
    );
    assert!(
        crate::win32_contracts::Registry::builtin()
            .unwrap()
            .function_entry(&function)
            .is_none()
    );
    assert_eq!(function.parameters[0].direction, RawDirection::InOut);
    let projected = project_one(function);
    assert_eq!(projected.runtime.parameters[0].direction, Direction::InOut);
    assert_eq!(projected.runtime.parameters[0].abi, AbiType::I32);
    assert!(
        matches!(&projected.return_shape,ReturnShape::Object { outputs,.. }
        if outputs[0].typ == SurfaceType::Enum("FIRMWARE_TYPE".into()))
    );
}

#[test]
fn subsystem_contexts_exemptions_and_managed_startup_follow_native_evidence() {
    let Some(path) = metadata() else { return };
    let mut functions = Vec::new();
    for (namespace, regular, start, stop, subsystem) in [
        (
            "Windows.Win32.Networking.WinSock",
            "WSASetLastError",
            "WSAStartup",
            "WSACleanup",
            Subsystem::Winsock,
        ),
        (
            "Windows.Win32.Graphics.GdiPlus",
            "GdipGetImageDecodersSize",
            "GdiplusStartup",
            "GdiplusShutdown",
            Subsystem::GdiPlus,
        ),
        (
            "Windows.Win32.Media.MediaFoundation",
            "MFGetSystemTime",
            "MFStartup",
            "MFShutdown",
            Subsystem::MediaFoundation,
        ),
    ] {
        let mut raw = crate::win32_metadata::parse_apis(&path, namespace, "Apis").unwrap();
        raw.functions
            .retain(|f| [regular, start, stop].contains(&f.name.as_str()));
        let projected = project_apis(&raw);
        let function = projected
            .projected
            .functions
            .iter()
            .find(|f| f.metadata_name == regular)
            .unwrap();
        assert_eq!(function.subsystem, Some(subsystem));
        for name in [start, stop] {
            let omission = projected
                .omitted
                .iter()
                .find(|o| o.identity.ends_with(&format!("::{name}")))
                .unwrap();
            assert!(omission.reason.contains("lifecycle"));
            assert!(
                !projected
                    .projected
                    .functions
                    .iter()
                    .any(|f| f.metadata_name == name)
            );
        }
        functions.push(
            raw.functions
                .into_iter()
                .find(|f| f.name == regular)
                .unwrap(),
        );
    }
    let exempt = metadata_function(&path, "Windows.Win32.Networking.WinSock", "WSAGetLastError");
    assert!(exempt.supports_last_error);
    assert!(matches!(&exempt.return_type.base, RawBaseType::Named {
        name, kind: RawNamedKind::Enum { underlying: RawScalar::I32, .. }, ..
    } if name == "WSA_ERROR"));
    let projected = project_one(exempt.clone());
    assert_eq!(projected.subsystem, None);
    assert_eq!(projected.runtime.return_abi, Some(AbiType::I32));
    assert_eq!(
        projected.return_shape,
        ReturnShape::Object {
            status: false,
            return_may_be_unavailable: false,
            return_value: Some((SurfaceType::Enum("WSA_ERROR".into()), Conversion::Number)),
            outputs: vec![],
            last_error: true,
        }
    );
    functions.push(exempt);
    let (generated, omitted) = generate_apis_files(&apis(functions), "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const calls=[]
const runtime={DynWin32:{
  initializeWinsock:()=>({kind:'winsock'}),
  initializeGdiPlus:()=>({kind:'gdiplus'}),
  initializeMediaFoundation:()=>({kind:'mediaFoundation'}),
  i32:value=>value,toNumber:Number,toBigint:BigInt,
},DynWin32Function:{bind(spec){return {
  invoke(args){calls.push({spec,args});return {returnValue:7,outputs:[],lastError:19}},
  invokeWithSubsystem(context,kind,args){assert.equal(context.kind,kind);calls.push({spec,args,kind});return {returnValue:7,outputs:[2,32]}}
}}}}
"#,
        r#"
projected.wsaSetLastError(projected.initializeWinsock(),123)
assert.equal(calls.at(-1).kind,'winsock')
assert.deepEqual(Array.from(calls.at(-1).args),[123])
projected.gdipGetImageDecodersSize(projected.initializeGdiPlus())
assert.equal(calls.at(-1).kind,'gdiplus')
assert.equal(projected.mfGetSystemTime(projected.initializeMediaFoundation()),7n)
assert.equal(calls.at(-1).kind,'mediaFoundation')
const error=projected.wsaGetLastError()
assert.equal(error.result,7)
assert.equal(error.lastError,19)
assert.equal(calls.at(-1).kind,undefined)
"#,
    );
}

#[test]
fn overlapped_read_write_require_exact_native_roles_and_emit_the_correct_async_helper() {
    let Some(path) = metadata() else { return };
    let mut raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.Storage.FileSystem", "Apis")
            .unwrap();
    raw.functions
        .retain(|f| ["ReadFile", "WriteFile"].contains(&f.name.as_str()));
    for function in &raw.functions {
        assert_eq!(function.calling_convention, RawCallingConvention::System);
        assert!(!function.variadic);
        assert!(function.supports_last_error);
        assert_eq!(
            function.return_type.base,
            RawBaseType::Scalar(RawScalar::Bool32)
        );
        assert_eq!(function.parameters.len(), 5);
        let read = function.name == "ReadFile";
        assert_eq!(
            function
                .parameters
                .iter()
                .map(|p| p.direction)
                .collect::<Vec<_>>(),
            [
                RawDirection::In,
                if read {
                    RawDirection::Out
                } else {
                    RawDirection::In
                },
                RawDirection::In,
                RawDirection::Out,
                RawDirection::InOut,
            ]
        );
        assert_eq!(
            function.parameters[1].buffer.as_ref().unwrap().size,
            RawBufferSize::ByteCountParam(2)
        );
        assert_eq!(
            function.parameters[2].typ.base,
            RawBaseType::Scalar(RawScalar::U32)
        );
        assert_eq!(function.parameters[3].typ.pointer_depth, 1);
        assert!(function.parameters[4].nullable);
        assert!(
            matches!(&function.parameters[4].typ.base,RawBaseType::Named { namespace,name,kind:RawNamedKind::NativeStruct { .. } }
            if namespace == "Windows.Win32.System.IO" && name == "OVERLAPPED")
        );
        let mut changed = function.clone();
        changed.parameters[1].buffer.as_mut().unwrap().size = RawBufferSize::ByteCountParam(3);
        let rejected = project_apis(&apis(vec![changed]));
        assert!(rejected.projected.functions.is_empty());
        assert!(rejected.projected.async_functions.is_empty());
        assert!(rejected.omitted[0].reason.contains("drift"));
    }
    let projected = project_apis(&raw);
    assert!(projected.omitted.is_empty());
    assert!(projected.projected.functions.is_empty());
    assert_eq!(
        projected.projected.async_functions,
        vec![
            ProjectedAsyncFunction {
                js_name: "readFileAsync".into(),
                kind: AsyncIoKind::Read
            },
            ProjectedAsyncFunction {
                js_name: "writeFileAsync".into(),
                kind: AsyncIoKind::Write
            },
        ]
    );
    let (generated, omitted) = generate_apis_files(&raw, "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const calls=[]
const begin=kind=>(file,buffer,offset)=>({
  cancel(){calls.push({kind:'cancel'})},
  start(callback){calls.push({kind,file,buffer,offset});callback(null,4)}
})
const runtime={DynWin32:{beginReadFile:begin('read'),beginWriteFile:begin('write')},DynWin32Function:{}}
"#,
        r#"
;(async()=>{
  const file={}
  const buffer=Buffer.alloc(8)
  assert.equal(await projected.readFileAsync(file,buffer),4)
  assert.equal(calls.at(-1).kind,'read')
  assert.equal(calls.at(-1).file,file)
  assert.equal(calls.at(-1).buffer,buffer)
  assert.equal(calls.at(-1).offset,0n)
  assert.equal(await projected.writeFileAsync(file,buffer,12n),4)
  assert.equal(calls.at(-1).kind,'write')
  assert.equal(calls.at(-1).offset,12n)
  const before=calls.length
  await assert.rejects(projected.readFileAsync(file,buffer,0n,{}),/AbortSignal/)
  await assert.rejects(projected.readFileAsync(file,buffer,0n,{
    aborted:true,addEventListener(){},removeEventListener(){}
  }),error=>error.name==='AbortError')
  assert.equal(calls.length,before)
})().catch(error=>{console.error(error);process.exitCode=1})
"#,
    );
}

#[test]
fn native_builders_encode_size_retention_and_success_gated_handle_fields() {
    let Some(path) = metadata() else { return };
    let mut raw =
        crate::win32_metadata::parse_apis(&path, "Windows.Win32.System.Threading", "Apis").unwrap();
    raw.functions
        .retain(|f| ["CreateProcessA", "CreateProcessW"].contains(&f.name.as_str()));
    let projection = project_apis(&raw);
    assert!(projection.omitted.is_empty(), "{:?}", projection.omitted);
    let builders = &projection.projected.native_builders;
    let security = builders
        .iter()
        .find(|b| b.layout_name == "SECURITY_ATTRIBUTES")
        .unwrap();
    assert_eq!(security.js_name, "SecurityAttributes");
    assert_eq!(security.size_field.as_deref(), Some("nLength"));
    assert_eq!(
        security.fields,
        vec![
            ProjectedNativeBuilderField {
                native_name: "lpSecurityDescriptor".into(),
                surface_name: "securityDescriptor".into(),
                kind: NativeBuilderFieldKind::DataPointer { nullable: true },
                optional: true
            },
            ProjectedNativeBuilderField {
                native_name: "bInheritHandle".into(),
                surface_name: "inheritHandle".into(),
                kind: NativeBuilderFieldKind::Boolean,
                optional: true
            },
        ]
    );
    assert!(security.outputs.is_empty());
    for (native, name) in [
        ("STARTUPINFOA", "StartupInfoA"),
        ("STARTUPINFOW", "StartupInfoW"),
    ] {
        let builder = builders.iter().find(|b| b.layout_name == native).unwrap();
        assert_eq!(builder.js_name, name);
        assert_eq!(builder.size_field.as_deref(), Some("cb"));
        assert!(builder.fields.is_empty() && builder.outputs.is_empty());
    }
    let process = builders
        .iter()
        .find(|b| b.layout_name == "PROCESS_INFORMATION")
        .unwrap();
    assert_eq!(process.size_field, None);
    assert!(process.fields.is_empty());
    assert_eq!(
        process.outputs,
        vec![
            ProjectedNativeOutputField {
                native_name: "hProcess".into(),
                surface_name: "process".into(),
                kind: NativeOutputFieldKind::Resource {
                    cleanup: Cleanup::CloseHandle
                }
            },
            ProjectedNativeOutputField {
                native_name: "hThread".into(),
                surface_name: "thread".into(),
                kind: NativeOutputFieldKind::Resource {
                    cleanup: Cleanup::CloseHandle
                }
            },
            ProjectedNativeOutputField {
                native_name: "dwProcessId".into(),
                surface_name: "processId".into(),
                kind: NativeOutputFieldKind::U32
            },
            ProjectedNativeOutputField {
                native_name: "dwThreadId".into(),
                surface_name: "threadId".into(),
                kind: NativeOutputFieldKind::U32
            },
        ]
    );
    for function in &projection.projected.functions {
        assert_eq!(function.runtime.success_rule, SuccessRule::ReturnNonZero);
        assert!(function.runtime.capture_last_error);
        let original = raw
            .functions
            .iter()
            .find(|f| f.name == function.metadata_name)
            .unwrap();
        let native = semantic(original);
        assert_eq!(original.parameters[1].direction, RawDirection::InOut);
        assert_eq!(native.parameters[1].direction, Direction::In);
        for (layout_name, x86_size, x64_size) in [
            ("SECURITY_ATTRIBUTES", 12, 24),
            ("PROCESS_INFORMATION", 16, 24),
            (
                if function.metadata_name.ends_with('W') {
                    "STARTUPINFOW"
                } else {
                    "STARTUPINFOA"
                },
                68,
                104,
            ),
        ] {
            let native = function
                .inputs
                .iter()
                .find_map(|input| match input {
                    InputExpression::NativeAggregate {
                        layout,
                        by_value: false,
                        ..
                    } if layout.name == layout_name => Some(layout),
                    _ => None,
                })
                .unwrap();
            assert_eq!(native.x86.size, x86_size);
            assert_eq!(native.x64.size, x64_size);
            assert_eq!(native.arm64.size, x64_size);
        }
    }
    let (generated, omitted) = generate_apis_files(&raw, "@test/runtime/win32");
    assert!(omitted.is_empty());
    run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const log=[]
const aggregate={
  createNativeStruct(descriptor){const d=JSON.parse(descriptor);return {length:d.x64.size,fields:{},name:d.name}},
  setNativeStructU32(value,descriptor,field,bits){value.fields[field]=bits},
  setNativeStructBool32(value,descriptor,field,flag){value.fields[field]=flag},
  setNativeStructPointer(value,descriptor,field,pointer){value.fields[field]=pointer},
  getNativeStructU32(value,descriptor,field){return value.fields[field]},
  takeNativeStructResource(value,descriptor,field,cleanup){return {field,cleanup}},
}
const safe={dataPointer:(value,nullable)=>({value,nullable})}
const runtime={DynWin32:new Proxy(safe,{get(target,key){return key in target?target[key]:aggregate[key]}}),DynWin32Function:{}}
"#,
        r#"
const descriptor=Buffer.from([1,2,3])
const security=projected.createSecurityAttributes({securityDescriptor:descriptor,inheritHandle:true})
assert.equal(security.fields.nLength,24)
assert.equal(security.fields.lpSecurityDescriptor.value,descriptor)
assert.equal(security.fields.lpSecurityDescriptor.nullable,true)
assert.equal(security.fields.bInheritHandle,true)
for(const create of [projected.createStartupInfoA,projected.createStartupInfoW]) {
  const value=create()
  assert.deepEqual(Object.keys(value.fields),['cb'])
  assert.equal(value.fields.cb,104)
}
const info=projected.createProcessInformation()
info.fields.dwProcessId=31
info.fields.dwThreadId=47
assert.equal(projected.getProcessInformationProcessId(info),31)
assert.equal(projected.getProcessInformationThreadId(info),47)
assert.equal(projected.takeProcessInformationProcess(info).field,'hProcess')
assert.equal(projected.takeProcessInformationThread(info).field,'hThread')
assert.equal(projected.takeProcessInformationThread(info).cleanup,'closeHandle')
"#,
    );
}

#[test]
fn real_enum_files_preserve_values_and_frozen_runtime_objects_not_debug_fingerprints() {
    let Some(path) = metadata() else { return };
    let raw = metadata_function(&path, "Windows.Win32.System.Registry", "RegQueryValueExW");
    let projected = project_apis(&apis(vec![raw.clone()]));
    assert!(projected.omitted.is_empty());
    let definition = projected
        .projected
        .enums
        .iter()
        .find(|e| e.name == "REG_VALUE_TYPE")
        .unwrap();
    assert_eq!(definition.namespace, "Windows.Win32.System.Registry");
    assert_eq!(definition.underlying, EnumUnderlying::U32);
    for (name, value) in [
        ("REG_NONE", 0),
        ("REG_SZ", 1),
        ("REG_BINARY", 3),
        ("REG_DWORD", 4),
    ] {
        assert_eq!(
            definition
                .members
                .iter()
                .find(|m| m.name == name)
                .unwrap()
                .value,
            value
        );
    }
    let (generated, omitted) = generate_apis_files(&apis(vec![raw]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    let javascript = &generated
        .extra_files
        .iter()
        .find(|(n, _)| n == "REG_VALUE_TYPE.js")
        .unwrap()
        .1;
    let declarations = &generated
        .extra_files
        .iter()
        .find(|(n, _)| n == "REG_VALUE_TYPE.d.ts")
        .unwrap()
        .1;
    assert!(javascript.starts_with("// Generated by dynwinrt-codegen"));
    assert!(declarations.starts_with("// Generated by dynwinrt-codegen"));
    run_js(
        &GeneratedOutput {
            js: javascript.clone(),
            dts: String::new(),
            extra_files: vec![],
        },
        "const assert=require('node:assert/strict'); const runtime={}",
        "assert(Object.isFrozen(projected.REG_VALUE_TYPE)); assert.equal(projected.REG_VALUE_TYPE.REG_NONE,0); assert.equal(projected.REG_VALUE_TYPE.REG_DWORD,4);",
    );
}

#[test]
fn borrowed_module_returns_keep_numeric_handles_and_do_not_acquire_cleanup() {
    let Some(path) = metadata() else { return };
    for name in ["GetModuleHandleA", "GetModuleHandleW"] {
        let raw = metadata_function(&path, "Windows.Win32.System.LibraryLoader", name);
        let native = semantic(&raw);
        assert_eq!(native.return_cleanup, Cleanup::None);
        let projected = project_one(raw);
        assert_eq!(projected.runtime.return_cleanup, Cleanup::None);
        assert_eq!(projected.runtime.return_abi, Some(AbiType::Handle));
        assert_eq!(projected.runtime.success_rule, SuccessRule::Always);
        assert!(matches!(&projected.return_shape,ReturnShape::Object {
            status:false,return_value:Some((SurfaceType::Handle(name),Conversion::BigInt)),outputs,last_error:true,return_may_be_unavailable:false,
        } if name == "HMODULE" && outputs.is_empty()));
    }
}
