// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod common;

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use dynwinrt_codegen::codegen::{javascript, project, python, python_stub, render_dts, render_js};
use dynwinrt_codegen::meta::{
    self, ImplementationDelegateMeta, InterfaceImplementationMetadata, InterfaceMeta, MethodMeta,
    ParamDirection, ParamMeta,
};
use dynwinrt_codegen::types::{FieldMeta, TypeMeta};

const WINDOWS_WINMD: &str =
    r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd";

fn generated(iface: &InterfaceMeta) -> [String; 4] {
    let known = HashSet::from([iface.name.clone()]);
    let projected = project::project_interface(
        &Default::default(),
        iface,
        &known,
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let context =
        common::projection_context(&[], std::slice::from_ref(iface), &known, &HashSet::new());
    [
        render_js::render(&projected),
        render_dts::render(&projected),
        python::generate_interface(&context, iface),
        python_stub::generate_interface_stub(&context, iface),
    ]
}

fn fixture() -> InterfaceMeta {
    let delegate = TypeMeta::Delegate {
        namespace: "Tests".into(),
        name: "Callback".into(),
        iid: "3fe67b63-0ad2-4b10-8b25-4f5d7696cb03".into(),
    };
    let fill_delegate = TypeMeta::Delegate {
        namespace: "Tests".into(),
        name: "FillCallback".into(),
        iid: "6df84c34-56fe-47fd-a7d6-2a10e986efc4".into(),
    };
    let reference = TypeMeta::Parameterized {
        namespace: "Windows.Foundation".into(),
        name: "IReference`1".into(),
        piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
        args: vec![TypeMeta::I32],
    };
    let hresult = TypeMeta::Struct {
        namespace: "Windows.Foundation".into(),
        name: "HResult".into(),
        fields: vec![FieldMeta {
            name: "Value".into(),
            typ: TypeMeta::I32,
        }],
    };
    InterfaceMeta {
        namespace: "Tests".into(),
        name: "IContract".into(),
        iid: "119ade03-d016-4c34-8813-4c2c3678dada".into(),
        methods: vec![
            MethodMeta {
                name: "get_Title".into(),
                vtable_index: 6,
                is_property_getter: true,
                return_type: Some(TypeMeta::String),
                ..Default::default()
            },
            MethodMeta {
                name: "put_Title".into(),
                vtable_index: 7,
                is_property_setter: true,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::String,
                    direction: ParamDirection::In,
                }],
                ..Default::default()
            },
            MethodMeta {
                name: "Transform".into(),
                vtable_index: 8,
                params: vec![
                    ParamMeta {
                        name: "label".into(),
                        typ: TypeMeta::String,
                        direction: ParamDirection::Out,
                    },
                    ParamMeta {
                        name: "values".into(),
                        typ: TypeMeta::Array(Box::new(TypeMeta::I32)),
                        direction: ParamDirection::In,
                    },
                    ParamMeta {
                        name: "buffer".into(),
                        typ: TypeMeta::Array(Box::new(TypeMeta::I32)),
                        direction: ParamDirection::OutFill,
                    },
                ],
                return_type: Some(TypeMeta::Bool),
                ..Default::default()
            },
            MethodMeta {
                name: "UseCallback".into(),
                vtable_index: 9,
                params: vec![ParamMeta {
                    name: "callback".into(),
                    typ: delegate.clone(),
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::String),
                ..Default::default()
            },
            MethodMeta {
                name: "GetObject".into(),
                vtable_index: 10,
                return_type: Some(TypeMeta::Object),
                ..Default::default()
            },
            MethodMeta {
                name: "EchoOptional".into(),
                vtable_index: 11,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: reference.clone(),
                    direction: ParamDirection::In,
                }],
                return_type: Some(reference),
                ..Default::default()
            },
            MethodMeta {
                name: "UseContract".into(),
                vtable_index: 12,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: TypeMeta::Interface {
                        namespace: "Tests".into(),
                        name: "IContract".into(),
                        iid: "119ade03-d016-4c34-8813-4c2c3678dada".into(),
                    },
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::Bool),
                ..Default::default()
            },
            MethodMeta {
                name: "UseFillCallback".into(),
                vtable_index: 13,
                params: vec![ParamMeta {
                    name: "callback".into(),
                    typ: fill_delegate.clone(),
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::Bool),
                ..Default::default()
            },
            MethodMeta {
                name: "RoundTripHresult".into(),
                vtable_index: 14,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: hresult.clone(),
                    direction: ParamDirection::In,
                }],
                return_type: Some(hresult.clone()),
                ..Default::default()
            },
            MethodMeta {
                name: "RoundTripHresults".into(),
                vtable_index: 15,
                params: vec![ParamMeta {
                    name: "values".into(),
                    typ: TypeMeta::Array(Box::new(hresult.clone())),
                    direction: ParamDirection::In,
                }],
                return_type: Some(TypeMeta::Array(Box::new(hresult))),
                ..Default::default()
            },
        ],
        implementation_metadata: InterfaceImplementationMetadata {
            delegates: vec![
                ImplementationDelegateMeta {
                    typ: delegate,
                    invoke: MethodMeta {
                        name: "Invoke".into(),
                        vtable_index: 3,
                        params: vec![ParamMeta {
                            name: "value".into(),
                            typ: TypeMeta::I32,
                            direction: ParamDirection::In,
                        }],
                        return_type: Some(TypeMeta::String),
                        ..Default::default()
                    },
                },
                ImplementationDelegateMeta {
                    typ: fill_delegate,
                    invoke: MethodMeta {
                        name: "Invoke".into(),
                        vtable_index: 3,
                        params: vec![ParamMeta {
                            name: "buffer".into(),
                            typ: TypeMeta::Array(Box::new(TypeMeta::I32)),
                            direction: ParamDirection::OutFill,
                        }],
                        return_type: Some(TypeMeta::U32),
                        ..Default::default()
                    },
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn run(program: &str, args: &[&str], input: &str) -> String {
    let mut child = Command::new(program)
        .env("PYTHONUTF8", "1")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start installed language runner");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn implementation_renderers_preserve_abi_order_and_emit_typed_named_results() {
    let [js, dts, py, pyi] = generated(&fixture());
    assert!(js.contains("signature: new DynWinRtMethodSig().addOut(DynWinRtType.hstring()).addIn(DynWinRtType.arrayType(DynWinRtType.i32())).addOutFill(DynWinRtType.arrayType(DynWinRtType.i32())).addOut(DynWinRtType.boolType())"), "{js}");
    assert!(py.contains("DynWinRTImplementationMethod(\"Transform\", 8, DynWinRTMethodSig().add_out(DynWinRTType.hstring()).add_in(DynWinRTType.array_type(DynWinRTType.i32_type())).add_out_fill(DynWinRTType.array_type(DynWinRTType.i32_type())).add_out(DynWinRTType.bool_type()))"), "{py}");
    assert!(dts.contains("transform(values: (number)[], bufferCapacity: number): { label: string; buffer: (number)[]; result: boolean }"), "{dts}");
    assert!(dts.contains("getTitle(): string") && dts.contains("setTitle(value: string): void"));
    assert!(pyi.contains("def transform(self, values: list[int], buffer_capacity: int) -> IContractImplementationTransformResult"), "{pyi}");
    assert!(pyi.contains("class IContractImplementationTransformResult(TypedDict):\n    label: str\n    buffer: list[int]\n    result: bool"));
    assert!(js.contains("bufferCapacity") || dts.contains("bufferCapacity"));
    assert!(
        js.contains("FillArray result must match capacity")
            && python::generate_runtime_support_module()
                .contains("FillArray result must match capacity")
    );
    assert!(dts.contains("private constructor();"));
    assert!(dts.contains("static fromImplementation(owner: DynWinRtImplementation | DynWinRtImplementationHandle<unknown>): IContract;"));
    assert!(
        pyi.contains(
            "def from_implementation(cls, owner: DynWinRTImplementation | DynWinRTImplementationHandle[object]) -> IContract: ..."
        )
    );
    assert!(js.contains("const value = owner.toValue();"));
    assert!(
        js.contains("return this.from(value);\n        } finally {\n            value.release();")
    );
    assert!(py.contains("value = owner.to_value()"));
    assert!(py.contains("native = value.cast(IID_IContract)"));
    assert!(py.contains("cls._set_native(view, native, cache=False)"));
    assert!(py.contains(
        "except BaseException:\n                native.release()\n                raise"
    ));
    assert!(py.contains("finally:\n            value.release()"));
}

#[test]
fn implementation_result_names_are_chosen_once_before_language_naming() {
    let iface = InterfaceMeta {
        namespace: "Tests".into(),
        name: "IResults".into(),
        iid: "119ade03-d016-4c34-8813-4c2c3678dada".into(),
        methods: vec![MethodMeta {
            name: "Read".into(),
            vtable_index: 6,
            params: ["RESULT", "result_"]
                .into_iter()
                .map(|name| ParamMeta {
                    name: name.into(),
                    typ: TypeMeta::I32,
                    direction: ParamDirection::Out,
                })
                .collect(),
            return_type: Some(TypeMeta::Bool),
            ..Default::default()
        }],
        ..Default::default()
    };
    let [js, dts, py, pyi] = generated(&iface);
    assert!(
        dts.contains("{ rESULT: number; result_: number; result__: boolean }"),
        "{dts}"
    );
    assert!(
        pyi.contains("    result: int\n    result_: int\n    result__: bool"),
        "{pyi}"
    );
    assert!(js.contains("__implementationField(result, \"result__\""));
    assert!(py.contains("_implementation_field(result, \"result__\""));
}

#[test]
fn implementation_hresult_values_and_arrays_use_exact_factories() {
    let [js, _, py, _] = generated(&fixture());
    assert!(js.contains("DynWinRtValue.hresult("));
    assert!(js.contains("DynWinRtArray.fromHresultValues("));
    assert!(py.contains("DynWinRTValue.from_hresult("));
    assert!(py.contains("DynWinRTArray.from_values([DynWinRTValue.from_hresult("));
    assert!(
        js.contains("DynWinRtValue.i32("),
        "ordinary I32 values must retain their factory"
    );
    assert!(
        py.contains("DynWinRTValue.from_i32("),
        "ordinary I32 values must retain their factory"
    );
}

#[test]
fn implementation_invalid_contracts_keep_outbound_generation_and_reject_factories() {
    let mut cases = Vec::new();
    let mut generic = fixture();
    generic.generic_piid = Some(generic.iid.clone());
    generic.generic_args = vec![TypeMeta::I32];
    cases.push((generic, "generic interface implementations"));
    let mut incomplete = fixture();
    incomplete.methods[1].vtable_index = 10;
    cases.push((incomplete, "incomplete or non-WinRT vtable"));
    let mut missing_iid = fixture();
    missing_iid.iid.clear();
    cases.push((missing_iid, "IID is unresolved"));
    let mut unknown = fixture();
    unknown
        .implementation_metadata
        .diagnostics
        .push("unknown pointer contract".into());
    cases.push((unknown, "unknown pointer contract"));
    let mut collision = fixture();
    collision.methods[1].name = "GetTitle".into();
    collision.methods[1].is_property_setter = false;
    cases.push((collision, "handler name collision"));
    for (iface, expected) in cases {
        let [js, dts, py, pyi] = generated(&iface);
        for output in [&js, &dts, &py, &pyi] {
            assert!(output.contains(expected), "{expected}\n{output}");
        }
        assert!(js.contains("class IContract") && py.contains("class IContract"));
        assert!(dts.contains("handlers: never") && !pyi.contains("def implement("));
        assert!(
            !js.contains("DynWinRtImplementationHandle.create")
                && !py.contains("DynWinRTImplementationHandle._create")
        );
    }
}

#[test]
fn implementation_stock_winmd_covers_background_strings_closing_properties_events_arrays_and_multiout()
 {
    if !Path::new(WINDOWS_WINMD).is_file() {
        eprintln!("Skipping: Windows.winmd is unavailable");
        return;
    }
    let foundation = meta::parse_interfaces(WINDOWS_WINMD, "Windows.Foundation");
    let background = meta::parse_interfaces(WINDOWS_WINMD, "Windows.ApplicationModel.Background");
    for (interfaces, name, method) in [
        (&foundation, "IStringable", "ToString"),
        (&foundation, "IClosable", "Close"),
        (&background, "IBackgroundTask", "Run"),
    ] {
        let iface = interfaces.iter().find(|iface| iface.name == name).unwrap();
        assert_eq!(iface.methods[0].vtable_index, 6);
        let [js, dts, py, pyi] = generated(iface);
        assert!(
            js.contains("DynWinRtImplementationHandle.create")
                && py.contains("DynWinRTImplementationHandle._create"),
            "{js}\n{py}"
        );
        assert!(js.contains(&format!("name: \"{method}\", vtableIndex: 6")));
        assert!(
            dts.contains(&format!("{name}Handlers")) && pyi.contains(&format!("{name}Handlers"))
        );
    }
    let instance = background
        .iter()
        .find(|iface| iface.name == "IBackgroundTaskInstance")
        .unwrap();
    assert!(!instance.implementation_metadata.delegates.is_empty());
    assert_eq!(
        instance.implementation_metadata.delegates[0]
            .invoke
            .vtable_index,
        3
    );
    let [js, dts, py, pyi] = generated(instance);
    assert!(
        dts.contains("getProgress") && dts.contains("setProgress") && dts.contains("addCanceled")
    );
    assert!(
        pyi.contains("get_progress")
            && pyi.contains("set_progress")
            && pyi.contains("add_canceled")
    );
    assert!(js.contains(".invokeDelegate(") && py.contains(".invoke_delegate("));
    let reader = meta::parse_class(WINDOWS_WINMD, "Windows.Storage.Streams", "DataReader")
        .unwrap()
        .default_interface
        .unwrap();
    assert!(reader.methods.iter().any(|method| {
        method
            .params
            .iter()
            .any(|p| p.direction == ParamDirection::OutFill)
    }));
    let [js, _, py, _] = generated(&reader);
    assert!(
        js.contains("addOutFill") && js.contains("DynWinRtImplementationHandle.create"),
        "{js}"
    );
    assert!(
        py.contains("add_out_fill") && py.contains("DynWinRTImplementationHandle._create"),
        "{py}"
    );
    let properties = foundation
        .iter()
        .find(|iface| iface.name == "IPropertyValue")
        .unwrap();
    let [js, _, py, _] = generated(properties);
    assert!(
        js.contains("DynWinRtArray.fromObjectValues")
            && js.contains("DynWinRtImplementationHandle.create")
    );
    assert!(
        py.contains("DynWinRTArray.from_object_values")
            && py.contains("DynWinRTImplementationHandle._create")
    );
    let transforms =
        meta::parse_class(WINDOWS_WINMD, "Windows.UI.Xaml.Media", "GeneralTransform").unwrap();
    let transform = transforms.default_interface.unwrap();
    assert!(transform.methods.iter().any(|method| {
        method.return_type.is_some()
            && method
                .params
                .iter()
                .any(|param| param.direction == ParamDirection::Out)
    }));
    let [js, dts, py, pyi] = generated(&transform);
    assert!(
        js.contains("DynWinRtImplementationHandle.create")
            && py.contains("DynWinRTImplementationHandle._create")
    );
    assert!(dts.contains("result: boolean") && pyi.contains("result: bool"));
    let streams = meta::parse_interfaces(WINDOWS_WINMD, "Windows.Storage.Streams");
    let random_access = streams
        .iter()
        .find(|interface| interface.name == "IRandomAccessStream")
        .unwrap();
    let requirements = &random_access.implementation_metadata.required_interfaces;
    for name in ["IInputStream", "IOutputStream", "IClosable"] {
        assert!(
            requirements.iter().any(
                |typ| matches!(typ, TypeMeta::Interface { name: actual, .. } if actual == name)
            ),
            "{name}: {requirements:?}"
        );
    }
    assert!(
        !random_access
            .methods
            .iter()
            .any(|method| method.name == "ReadAsync" || method.name == "WriteAsync")
    );
    let [js, dts, py, pyi] = generated(random_access);
    assert!(
        js.contains("DynWinRtImplementationHandle.create")
            && py.contains("DynWinRTImplementationHandle._create")
    );
    assert!(dts.contains("Required QI views") && pyi.contains("Required QI views"));
    for iid in [
        "905a0fe2-bc53-11df-8c49-001e4fc686da",
        "905a0fe6-bc53-11df-8c49-001e4fc686da",
        "30d5a829-7fa4-4026-83bb-d75bae4ea99e",
    ] {
        assert!(js.contains(iid) && py.contains(iid), "{iid}");
    }
}

#[test]
fn implementation_javascript_projection_validates_handlers_results_arrays_and_dispatch() {
    let [js, _, _, _] = generated(&fixture());
    let test = r#"
const assert = require('node:assert/strict');
const payload = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
const input = payload.code;
// Projection-only doubles: native vtables/ownership are covered by the native and E2E suites.
class Value {
    constructor(kind, value) {
        this.kind = kind; this.value = value; this.releases = 0;
        if (kind === 'object' && value?.owner) value.owner.references++;
    }
    toString() { return this.value; } toNumber() { return this.value; } toBool() { return this.value; }
    asArray() { return this.value; } isNull() { return this.value === null; }
    cast(iid) {
        if (this.kind !== 'object') throw new TypeError('not an object');
        if (this.value?.owner?.failQueryInterface) throw new TypeError('QueryInterface failure');
        return new Value(this.kind, this.value);
    }
    release() {
        this.releases++;
        if (this.kind === 'object' && this.value?.owner) this.value.owner.references--;
        this.kind = 'null'; this.value = null;
    }
    invokeDelegate(iid, signature, args) {
        if (iid === '6df84c34-56fe-47fd-a7d6-2a10e986efc4') {
            assert.deepEqual(signature.parameters.map(p => p[0]), ['addOutFill', 'addOut']);
            assert.equal(args.length, 1);
            assert.equal(args[0].kind, 'array');
            assert.deepEqual(args[0].asArray().toValues().map(value => value.value), [0, 0, 0]);
            return [ArrayValue.fromObjectValues([Value.i32(7), Value.i32(7), Value.i32(7)]).toValue(), Value.u32(3)];
        }
        assert.equal(iid, '3fe67b63-0ad2-4b10-8b25-4f5d7696cb03');
        assert.deepEqual(signature.parameters.map(p => p[0]), ['addIn', 'addOut']);
        return [Value.hstring(`delegate:${args[0].value}`)];
    }
}
for (const name of ['i32', 'u32', 'hstring', 'boolValue']) Value[name] = value => new Value(name, value);
Value.hresult = value => {
    if (!Number.isInteger(value) || value < -2147483648 || value > 2147483647) throw new RangeError('HRESULT range');
    return new Value('hresult', value);
};
Value.nullValue = () => new Value('null', null);
Value.boxReference = value => new Value('object', { boxed: value });
const Type = new Proxy({}, { get(_, key) { return (...args) => ({
    key, args, iid() { return args[0]; }, addMethod() { return this; },
    method(slot) { return { invoke(obj, args) {
        if (obj.value.boxed) { assert.equal(slot, 6); return obj.value.boxed; }
        return obj.value.owner.callback(0, slot, args)[0] ?? Value.nullValue();
    } }; },
}); } });
class Signature { constructor() { this.parameters = []; } }
for (const name of ['addIn', 'addOut', 'addOutFill']) Signature.prototype[name] = function(type) { this.parameters.push([name, type]); return this; };
class Plan { static create(name, type, methods, required) { return Object.assign(new Plan(), { name, type, methods, required }); } }
let published = 0;
class Implementation {
    static create(plans, callback) { published++; return (Implementation.last = Object.assign(new Implementation(), { plans, callback, references: 1 })); }
    toValue() { return (this.lastValue = new Value('object', { owner: this })); }
    release() { if (!this.released) { this.references--; this.released = true; } }
    disconnect() { this.isClosed = true; }
}
function releaseProjected(view) { view._obj.release(); }
const ArrayValue = {
    fromObjectValues(values, type) {
        return { toValue: () => new Value('array', { toValues: () => [...values], toI32Vec: () => values.map(value => value.value) }) };
    },
    fromHresultValues(values) { return this.fromObjectValues(values.map(Value.hresult)); },
};
const runtime = { DynWinRtValue: Value, DynWinRtType: Type, DynWinRtMethodSig: Signature, DynWinRtInterfacePlan: Plan, DynWinRtImplementation: Implementation, DynWinRtArray: ArrayValue, WinGuid: { parse: value => value } };
const handleModule = { exports: {} };
new Function('require', 'module', payload.handle)(() => runtime, handleModule);
runtime.DynWinRtImplementationHandle = handleModule.exports.DynWinRtImplementationHandle;
const moduleObject = { exports: {} };
new Function('require', 'module', 'exports', input)((name) => runtime, moduleObject, moduleObject.exports);
const IContract = moduleObject.exports.IContract;
let title = 'start', seen;
const handlers = {
    getTitle() { return title; }, setTitle(value) { title = value; },
    transform(values, bufferCapacity) { seen = [values, bufferCapacity]; return { label: 'ok', buffer: Array(bufferCapacity).fill(values[0] + 1), result: true }; },
    useCallback(callback) { if (callback === null) return 'null'; assert(callback._obj instanceof Value); return callback(7); },
    getObject() { return Value.nullValue(); },
    echoOptional(value) { return value; },
    useContract(value) { return value !== null && value.title === 'public view'; },
    useFillCallback(callback) {
        if (callback === null) return false;
        const result = callback(3);
        assert.deepEqual(result, { buffer: [7, 7, 7], result: 3 });
        return true;
    },
    roundTripHresult(value) { return value; },
    roundTripHresults(values) { return values; },
};
assert.throws(() => IContract.implement({}), /missing synchronous handler/);
assert.throws(() => IContract.implement({ ...handlers, getTitle: async () => 'bad' }), /async handlers/);
let getterCalls = 0;
assert.throws(() => IContract.implement({ ...handlers, get getTitle() { getterCalls++; return () => 'bad'; } }), /accessors/);
assert.equal(getterCalls, 0); assert.equal(published, 0);
const invokeDelegate = Value.prototype.invokeDelegate;
delete Value.prototype.invokeDelegate;
assert.throws(() => IContract.implement(handlers), /requires WinRT delegate invocation/);
Value.prototype.invokeDelegate = invokeDelegate;
const descriptor = IContract.implementation(handlers);
assert.equal(descriptor.dispatch(6, [])[0].value, 'start');
descriptor.dispatch(7, [Value.hstring('changed')]); assert.equal(title, 'changed');
const values = ArrayValue.fromObjectValues([Value.i32(4), Value.i32(5)]).toValue();
const results = descriptor.dispatch(8, [values, Value.u32(3)]);
assert.deepEqual(seen, [[4, 5], 3]);
assert.deepEqual(results.map(v => v.kind), ['hstring', 'array', 'boolValue']);
assert.deepEqual(results[1].asArray().toValues().map(v => v.value), [5, 5, 5]);
assert.equal(descriptor.dispatch(9, [new Value('object', {})])[0].value, 'delegate:7');
assert.equal(descriptor.dispatch(9, [new Value('object', null)])[0].value, 'null');
assert.equal(descriptor.dispatch(10, [])[0].isNull(), true);
assert.equal(descriptor.dispatch(11, [Value.boxReference(Value.i32(8))])[0].value.boxed.value, 8);
assert.equal(descriptor.dispatch(11, [Value.nullValue()])[0].isNull(), true);
assert.equal(descriptor.dispatch(13, [new Value('object', {})])[0].value, true);
const semanticHresult = descriptor.dispatch(14, [Value.hresult(-2147467259)])[0];
assert.equal(semanticHresult.kind, 'hresult');
assert.equal(semanticHresult.value, -2147467259);
const hresultArray = descriptor.dispatch(15, [ArrayValue.fromHresultValues([-2147467259, 0]).toValue()])[0];
assert.deepEqual(hresultArray.asArray().toValues().map(value => [value.kind, value.value]), [['hresult', -2147467259], ['hresult', 0]]);
assert.throws(() => IContract.implementation({ ...handlers, roundTripHresults: () => [2147483648] }).dispatch(15, [hresultArray]), /invalid implementation result/);
assert.throws(() => descriptor.dispatch(3, []), /unknown implementation vtable slot/);
assert.throws(() => descriptor.dispatch(8, []), /argument count/);
assert.throws(() => IContract.implementation({ ...handlers, getTitle: () => ({ then() {} }) }).dispatch(6, []), /thenable/);
assert.throws(() => IContract.implementation({ ...handlers, transform: () => ({ label: 'bad', buffer: [], result: true }) }).dispatch(8, [values, Value.u32(3)]), /match capacity/);
assert.throws(() => IContract.implementation({ ...handlers, transform: () => ({ label: 'bad', buffer: [2147483648], result: true }) }).dispatch(8, [values, Value.u32(1)]), /invalid implementation result/);
assert.throws(() => IContract.implementation({ ...handlers, transform: () => ({ buffer: [1], result: true }) }).dispatch(8, [values, Value.u32(1)]), /missing data field/);
const handle = IContract.implement(handlers);
const owner = Implementation.last;
assert.equal(owner.callback(0, 6, [])[0].value, 'changed');
assert.throws(() => owner.callback(-1, 6, []), /Unknown implementation interface index/);
const view = IContract.fromImplementation(owner);
assert(view instanceof IContract);
assert.equal(owner.lastValue.releases, 1);
assert.equal(owner.references, 2);
assert.equal(view.title, 'changed');
view.title = 'public view';
assert.equal(view.useContract(view), true);
assert.equal(view.roundTripHresult(-2147467259), -2147467259);
assert.deepEqual(view.roundTripHresults([-2147467259, 0]), [-2147467259, 0]);
releaseProjected(view);
assert.equal(owner.references, 1);
assert.equal(owner.callback(0, 6, [])[0].value, 'public view');
owner.failQueryInterface = true;
assert.throws(() => IContract.fromImplementation(owner), /QueryInterface failure/);
assert.equal(owner.lastValue.releases, 1);
assert.equal(owner.references, 1);
owner.failQueryInterface = false;
const originalFrom = IContract.from;
IContract.from = () => { throw new TypeError('projection failure'); };
assert.throws(() => IContract.fromImplementation(owner), /projection failure/);
assert.equal(owner.lastValue.releases, 1);
assert.equal(owner.references, 1);
IContract.from = originalFrom;
const recovered = IContract.fromImplementation(owner);
assert.equal(recovered.title, 'public view');
releaseProjected(recovered);
assert.equal(owner.references, 1);
assert.throws(() => IContract.fromImplementation({}), /requires a DynWinRtImplementation controller/);
console.log('JavaScript projection assertions passed');
"#;
    let output = run(
        "node",
        &["-e", test],
        &serde_json::json!({
            "code": js,
            "handle": include_str!("../../../bindings/js/runtime/winrt-implementation.cjs"),
        })
        .to_string(),
    );
    assert!(output.contains("assertions passed"));
}

#[test]
fn implementation_javascript_handler_lookup_excludes_builtin_prototypes() {
    let sources = ["IStringable", "IUserCallbacks"].map(|name| {
        let mut iface = InterfaceMeta {
            namespace: "Tests".into(),
            name: name.into(),
            iid: "119ade03-d016-4c34-8813-4c2c3678dada".into(),
            methods: vec![MethodMeta {
                name: "ToString".into(),
                vtable_index: 6,
                return_type: Some(TypeMeta::String),
                ..Default::default()
            }],
            ..Default::default()
        };
        if name == "IUserCallbacks" {
            iface.methods.push(MethodMeta {
                name: "Call".into(),
                vtable_index: 7,
                return_type: Some(TypeMeta::String),
                ..Default::default()
            });
        }
        serde_json::json!({ "name": name, "source": generated(&iface)[0] })
    });
    let script = r#"
const assert = require('node:assert/strict');
const sources = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
let plansCreated = 0;
class Signature { addOut() { return this; } }
class Value { static hstring(value) { return value; } }
const runtime = {
    DynWinRtType: { interface: iid => iid, hstring: () => 'hstring' },
    DynWinRtMethodSig: Signature, DynWinRtValue: Value,
    DynWinRtInterfacePlan: { create() { plansCreated++; return {}; } },
    DynWinRtImplementation: class {},
    WinGuid: { parse: iid => iid },
};
for (const { name, source } of sources) {
    const module = { exports: {} };
    new Function('require', 'module', 'exports', source)(() => runtime, module, module.exports);
    const Interface = module.exports[name];
    const before = plansCreated;
    assert.throws(() => Interface.implementation({}), /missing synchronous handler toString/);
    assert.throws(() => Interface.implementation(function () {}), /missing synchronous handler toString/);
    assert.throws(() => Interface.implementation(Object.create(Function.prototype)), /missing synchronous handler toString/);
    assert.equal(plansCreated, before);
    class BaseHandlers {
        toString() { return this.label; }
        call() { return 'user class call'; }
    }
    class DerivedHandlers extends BaseHandlers {
        constructor() { super(); this.label = 'user class string'; }
    }
    const descriptor = Interface.implementation(new DerivedHandlers());
    assert.deepEqual(descriptor.dispatch(6, []), ['user class string']);
    const explicit = function () {};
    explicit.toString = () => 'explicit function handler';
    if (name === 'IUserCallbacks') {
        assert.throws(() => Interface.implementation(explicit), /missing synchronous handler call/);
        explicit.call = () => 'explicit call handler';
        assert.deepEqual(Interface.implementation(explicit).dispatch(7, []), ['explicit call handler']);
    }
    assert.deepEqual(Interface.implementation(explicit).dispatch(6, []), ['explicit function handler']);
}
console.log('prototype validation passed');
"#;
    assert!(
        run(
            "node",
            &["-e", script],
            &serde_json::to_string(&sources).unwrap()
        )
        .contains("prototype validation passed")
    );
}

#[test]
fn implementation_python_projection_validates_handlers_results_arrays_and_dispatch() {
    let mut iface = fixture();
    let zero_delegate = TypeMeta::Delegate {
        namespace: "Tests".into(),
        name: "ZeroCallback".into(),
        iid: "41c64fe4-5f4d-4cf8-8a39-c8e2a9f396a1".into(),
    };
    iface.methods.push(MethodMeta {
        name: "UseZeroCallback".into(),
        vtable_index: 16,
        params: vec![ParamMeta {
            name: "callback".into(),
            typ: zero_delegate.clone(),
            direction: ParamDirection::In,
        }],
        return_type: Some(TypeMeta::String),
        ..Default::default()
    });
    iface
        .implementation_metadata
        .delegates
        .push(ImplementationDelegateMeta {
            typ: zero_delegate,
            invoke: MethodMeta {
                name: "Invoke".into(),
                vtable_index: 3,
                return_type: Some(TypeMeta::String),
                ..Default::default()
            },
        });
    let [_, _, py, pyi] = generated(&iface);
    for source in [&py, &pyi] {
        assert!(source.contains("def __call__(self, value: int, /) -> str: ..."));
        assert!(source.contains("def __call__(self, /) -> str: ..."));
    }
    let test = r#"
import sys, json, types, typing, datetime, uuid, weakref, ast
payload = json.load(sys.stdin)
source = payload['code']
# Projection-only doubles, not native ABI fixtures.
class Value:
    def __init__(self, kind, value):
        self.kind, self.value, self.releases = kind, value, 0
        if kind == 'object' and hasattr(value, 'owner'): value.owner.references += 1
    def to_string(self): return self.value
    def to_number(self): return self.value
    def to_u32(self): return self.value
    def to_bool(self): return self.value
    def as_array(self): return self.value
    def is_null(self): return self.value is None
    def cast(self, iid):
        if self.kind != 'object': raise TypeError('not an object')
        if getattr(getattr(self.value, 'owner', None), 'fail_query_interface', False):
            raise TypeError('QueryInterface failure')
        return Value(self.kind, self.value)
    def release(self):
        self.releases += 1
        if self.kind == 'object' and hasattr(self.value, 'owner'): self.value.owner.references -= 1
        self.kind, self.value = 'null', None
    def invoke_delegate(self, iid, signature, args):
        if iid == '41c64fe4-5f4d-4cf8-8a39-c8e2a9f396a1':
            assert [kind for kind, typ in signature.parameters] == ['Out']
            assert args == []
            return [Value.from_hstring('zero')]
        if iid == '6df84c34-56fe-47fd-a7d6-2a10e986efc4':
            assert [kind for kind, typ in signature.parameters] == ['OutFill', 'Out']
            assert len(args) == 1
            assert args[0].kind == 'array'
            assert [value.value for value in args[0].as_array().to_values()] == [0, 0, 0]
            return [Array([Value.from_i32(7), Value.from_i32(7), Value.from_i32(7)]).to_value(), Value.from_u32(3)]
        assert iid == '3fe67b63-0ad2-4b10-8b25-4f5d7696cb03'
        assert [kind for kind, typ in signature.parameters] == ['In', 'Out']
        return [Value.from_hstring(f'delegate:{args[0].value}')]
for name in ('from_i32', 'from_u32', 'from_hstring', 'from_bool'):
    setattr(Value, name, staticmethod(lambda value, name=name: Value(name, value)))
Value.from_hresult = staticmethod(lambda value: Value('hresult', value))
Value.null_value = staticmethod(lambda: Value('null', None))
Value.box_reference = staticmethod(lambda value, typ: Value('object', types.SimpleNamespace(boxed=value)))
class NativeType:
    def __init__(self, key, args): self.key, self.args = key, args
    def iid(self): return self.args[0]
    def add_method(self, *args): return self
    def method(self, slot):
        def invoke(obj, args):
            if hasattr(obj.value, 'boxed'):
                assert slot == 6
                return obj.value.boxed
            outputs = obj.value.owner.callback(0, slot, args)
            return outputs[0] if outputs else Value.null_value()
        return types.SimpleNamespace(invoke=invoke, get_string=lambda obj: invoke(obj, []).to_string())
class Types(type):
    def __getattr__(cls, name): return lambda *args: NativeType(name, args)
class Type(metaclass=Types): pass
class Signature:
    def __init__(self): self.parameters = []
    def add_in(self, typ): self.parameters.append(('In', typ)); return self
    def add_out(self, typ): self.parameters.append(('Out', typ)); return self
    def add_out_fill(self, typ): self.parameters.append(('OutFill', typ)); return self
class Method:
    def __init__(self, name, slot, signature): self.name, self.slot, self.signature = name, slot, signature
class Plan:
    @staticmethod
    def create(*args): return Plan()
class Descriptor:
    def __init__(self, plan, dispatch): self.plan, self.dispatch = plan, dispatch
class Implementation:
    published = 0
    @staticmethod
    def create(plans, callback):
        Implementation.published += 1
        owner = Implementation()
        owner.plans, owner.callback = plans, callback
        owner.references = 1
        Implementation.last = owner
        return owner
    def to_value(self):
        self.last_value = Value('object', types.SimpleNamespace(owner=self))
        return self.last_value
    def release(self): self.references -= 1
    def disconnect(self): self.is_closed = True
def release_projected(view): view._obj.release()
class Array:
    def __init__(self, values): self.values = values
    @staticmethod
    def from_values(values, typ=None): return Array(values)
    @staticmethod
    def from_object_values(values, typ=None): return Array(values)
    def to_value(self): return Value('array', self)
    def to_values(self): return self.values[:]
    def to_i32_list(self): return [value.value for value in self.values]
runtime = types.ModuleType('generated._runtime')
runtime.__getattr__ = lambda name: None
def from_native(cls, obj, setter):
    instance = object.__new__(cls)
    getattr(instance, setter)(obj)
    return instance
for name, value in dict(
    TYPE_CHECKING=False, Callable=typing.Callable, DynWinRTType=Type, DynWinRTMethodSig=Signature,
    DynWinRTValue=Value, DynWinRTArray=Array, WinGUID=types.SimpleNamespace(parse=lambda value: value),
    _property=property, _weakref_ref=weakref.ref, UUID=uuid.UUID,
    datetime=datetime.datetime, timedelta=datetime.timedelta,
    _dynwinrt_projected_from_native=from_native,
    _dynwinrt_track_projected=lambda value, *args: value,
    _dynwinrt_cache_projected=lambda value: None,
    _dynwinrt_symbol=lambda module, name: namespace[name],
    _dynwinrt_array=lambda values, wrap, element_type, bytes_allowed: Array([wrap(value) for value in values]).to_value(),
).items(): setattr(runtime, name, value)
binding = types.ModuleType('dynwinrt')
for name, value in dict(DynWinRTInterfacePlan=Plan, DynWinRTImplementationMethod=Method,
    DynWinRTImplementation=Implementation, DynWinRTImplementationDescriptor=Descriptor).items(): setattr(binding, name, value)
binding.DynWinRTValue = Value
binding.release_projected = release_projected
exec(payload['handle'], binding.__dict__)
exec('import inspect as _implementation_inspect' + payload['support'].split('import inspect as _implementation_inspect', 1)[1], runtime.__dict__)
package = types.ModuleType('generated'); package.__path__ = []
sys.modules.update({'generated': package, 'generated._runtime': runtime, 'dynwinrt': binding})
namespace = {'__name__': 'generated.contract', '__package__': 'generated'}
exec(compile(source, 'contract.py', 'exec'), namespace)
for declaration in (source, payload['stub']):
    syntax = ast.parse(declaration)
    for name, count in [('IContractImplementationDelegate0', 2), ('IContractImplementationDelegate2', 1)]:
        protocol = next(node for node in ast.walk(syntax) if isinstance(node, ast.ClassDef) and node.name == name)
        call = next(node for node in protocol.body if isinstance(node, ast.FunctionDef) and node.name == '__call__')
        assert len(call.args.posonlyargs) == count and call.args.args == []
Contract = namespace['IContract']
class Handlers:
    def __init__(self): self.title = 'start'; self.seen = None
    def get_title(self): return self.title
    def set_title(self, value): self.title = value
    def transform(self, values, buffer_capacity):
        self.seen = (values, buffer_capacity)
        return {'label': 'ok', 'buffer': [values[0]+1] * buffer_capacity, 'result': True}
    def use_callback(self, callback):
        if callback is None: return 'null'
        assert isinstance(callback._obj, Value)
        raises('unexpected keyword', lambda: callback(value=7))
        raises('unexpected keyword', lambda: callback(7, value=7))
        raises('argument count', lambda: callback())
        raises('argument count', lambda: callback(7, 8))
        return callback(7)
    def get_object(self): return Value.null_value()
    def echo_optional(self, value): return value
    def use_contract(self, value): return value is not None and value.title == 'public view'
    def use_fill_callback(self, callback):
        if callback is None: return False
        result = callback(3)
        assert result == {'buffer': [7, 7, 7], 'result': 3}
        return True
    def round_trip_hresult(self, value): return value
    def round_trip_hresults(self, values): return values
    def use_zero_callback(self, callback):
        raises('unexpected keyword', lambda: callback(unexpected=7))
        raises('argument count', lambda: callback(7))
        return callback()
def raises(fragment, call):
    try: call()
    except (TypeError, ValueError) as error: assert fragment in str(error), str(error)
    else: raise AssertionError('expected rejection: '+fragment)
raises('missing synchronous handler', lambda: Contract.implement(object()))
class AsyncHandlers(Handlers):
    async def get_title(self): return 'bad'
raises('async handlers', lambda: Contract.implement(AsyncHandlers()))
class WrongSignature(Handlers):
    def get_title(self, unexpected): return unexpected
raises('handler signature', lambda: Contract.implement(WrongSignature()))
class PropertyHandlers(Handlers):
    @property
    def get_title(self): raise AssertionError('must not evaluate property')
raises('accessors', lambda: Contract.implement(PropertyHandlers()))
assert Implementation.published == 0
invoke_delegate = Value.invoke_delegate
del Value.invoke_delegate
raises('requires WinRT delegate invocation', lambda: Contract.implement(Handlers()))
Value.invoke_delegate = invoke_delegate
handlers = Handlers(); descriptor = Contract.implementation(handlers)
assert descriptor.dispatch(6, [])[0].value == 'start'
descriptor.dispatch(7, [Value.from_hstring('changed')]); assert handlers.title == 'changed'
values = Array([Value.from_i32(4), Value.from_i32(5)]).to_value()
outputs = descriptor.dispatch(8, [values, Value.from_u32(3)])
assert handlers.seen == ([4, 5], 3)
assert [value.kind for value in outputs] == ['from_hstring', 'array', 'from_bool']
assert [value.value for value in outputs[1].as_array().to_values()] == [5, 5, 5]
assert descriptor.dispatch(9, [Value('object', {})])[0].value == 'delegate:7'
assert descriptor.dispatch(9, [Value('object', None)])[0].value == 'null'
assert descriptor.dispatch(10, [])[0].is_null()
assert descriptor.dispatch(11, [Value.box_reference(Value.from_i32(8), None)])[0].value.boxed.value == 8
assert descriptor.dispatch(11, [Value.null_value()])[0].is_null()
assert descriptor.dispatch(13, [Value('object', {})])[0].value is True
assert descriptor.dispatch(16, [Value('object', {})])[0].value == 'zero'
semantic_hresult = descriptor.dispatch(14, [Value.from_hresult(-2147467259)])[0]
assert semantic_hresult.kind == 'hresult' and semantic_hresult.value == -2147467259
hresult_array = descriptor.dispatch(15, [Array([Value.from_hresult(-2147467259), Value.from_hresult(0)]).to_value()])[0]
assert [(value.kind, value.value) for value in hresult_array.as_array().to_values()] == [('hresult', -2147467259), ('hresult', 0)]
class InvalidHresults(Handlers):
    def round_trip_hresults(self, values): return [2147483648]
raises('invalid implementation result', lambda: Contract.implementation(InvalidHresults()).dispatch(15, [hresult_array]))
raises('unknown implementation vtable slot', lambda: descriptor.dispatch(3, []))
raises('argument count', lambda: descriptor.dispatch(8, []))
async def coroutine(): return 'bad'
class CoroutineResult(Handlers):
    def get_title(self): return coroutine()
raises('synchronously', lambda: Contract.implementation(CoroutineResult()).dispatch(6, []))
class ShortBuffer(Handlers):
    def transform(self, values, capacity): return {'label':'bad','buffer':[],'result':True}
raises('match capacity', lambda: Contract.implementation(ShortBuffer()).dispatch(8,[values,Value.from_u32(3)]))
class LargeInteger(Handlers):
    def transform(self, values, capacity): return {'label':'bad','buffer':[2147483648],'result':True}
raises('invalid implementation result', lambda: Contract.implementation(LargeInteger()).dispatch(8,[values,Value.from_u32(1)]))
handle = Contract.implement(handlers)
owner = Implementation.last
assert owner.callback(0,6,[])[0].value == 'changed'
raises('Unknown implementation interface index', lambda: owner.callback(-1,6,[]))
view = Contract.from_implementation(owner)
assert isinstance(view, Contract)
assert owner.last_value.releases == 1
assert owner.references == 2
assert view.title == 'changed'
view.title = 'public view'
assert view.use_contract(view)
assert view.round_trip_hresult(-2147467259) == -2147467259
assert view.round_trip_hresults([-2147467259, 0]) == [-2147467259, 0]
release_projected(view)
assert owner.references == 1
assert owner.callback(0,6,[])[0].value == 'public view'
owner.fail_query_interface = True
raises('QueryInterface failure', lambda: Contract.from_implementation(owner))
assert owner.last_value.releases == 1
assert owner.references == 1
owner.fail_query_interface = False
class FailingView(Contract):
    def _set_native(self, value, *, cache=True):
        raise TypeError('projection failure')
raises('projection failure', lambda: FailingView.from_implementation(owner))
assert owner.last_value.releases == 1
assert owner.references == 1
recovered = Contract.from_implementation(owner)
assert recovered.title == 'public view'
independent = Contract.from_implementation(owner)
assert independent is not recovered
assert owner.references == 3
release_projected(independent)
assert recovered.title == 'public view'
release_projected(recovered)
assert owner.references == 1
raises('requires a DynWinRTImplementation controller', lambda: Contract.from_implementation(object()))
print('Python projection assertions passed')
"#;
    let output = run(
        "python",
        &["-c", test],
        &serde_json::json!({
            "code": py,
            "stub": pyi,
            "handle": include_str!("../../../bindings/py/python/dynwinrt/_implementation.py"),
            "support": python::generate_runtime_support_module(),
        })
        .to_string(),
    );
    assert!(output.contains("assertions passed"));
}

#[test]
fn implementation_helper_types_are_separate_from_javascript_runtime_exports() {
    let iface = fixture();
    let projected = project::project_interface(
        &Default::default(),
        &iface,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    );
    assert!(
        projected
            .public_type_exports()
            .contains("IContractHandlers")
    );
    assert!(!projected.public_exports().contains("IContractHandlers"));
    let index = javascript::generator::generate_index(&[], &[iface], &[]);
    assert!(index.contains("export type { IContractHandlers, IContractImplementationDelegate0, IContractImplementationDelegate1 }"));
    assert!(
        !javascript::generator::esm_index_to_cjs_lazy(&index).contains("exports.IContractHandlers")
    );
    let [_, _, _, stub] = generated(&fixture());
    assert!(stub.contains("class _IContractImplementationFactory(ABCMeta):"));
    assert!(stub.contains("metaclass=_IContractImplementationFactory"));
    let interface_body = stub.split("class IContract(").nth(1).unwrap();
    assert!(
        !interface_body.contains("def implementation(")
            && !interface_body.contains("def implement(")
    );
    assert!(!interface_body.contains("def from_implementation("));
}

#[test]
fn implementation_cli_emits_background_instance_public_views() {
    if !Path::new(WINDOWS_WINMD).is_file() {
        eprintln!("Skipping: Windows.winmd is unavailable");
        return;
    }
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("cli-public-views-{}", std::process::id()));
    let mut artifacts = Vec::new();
    for language in ["js", "py"] {
        let output = scratch.join(language);
        let generated = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
            .args([
                "generate",
                "--class-name",
                "Windows.ApplicationModel.Background.IBackgroundTaskInstance",
                "--lang",
                language,
                "--winmd",
                WINDOWS_WINMD,
                "--output",
            ])
            .arg(&output)
            .output()
            .expect("run the built CLI executable");
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let base = if language == "js" {
            output
                .join("windows")
                .join("application-model")
                .join("background")
        } else {
            output
        };
        let stem = if language == "js" {
            "IBackgroundTaskInstance"
        } else {
            "windows__application_model__background__i_background_task_instance"
        };
        let declarations = if language == "js" { "d.ts" } else { "pyi" };
        artifacts.push((
            std::fs::read_to_string(base.join(format!("{stem}.{language}"))).unwrap(),
            std::fs::read_to_string(base.join(format!("{stem}.{declarations}"))).unwrap(),
        ));
    }
    std::fs::remove_dir_all(&scratch).unwrap();
    assert!(artifacts[0].0.contains("static fromImplementation(owner)"));
    assert!(
        artifacts[0]
            .0
            .contains("} finally {\n            value.release();")
    );
    assert!(artifacts[0].1.contains("private constructor();"));
    assert!(artifacts[0].1.contains(
        "static fromImplementation(owner: DynWinRtImplementation | DynWinRtImplementationHandle<unknown>): IBackgroundTaskInstance;"
    ));
    assert!(artifacts[1].0.contains(
        "def from_implementation(cls, owner: DynWinRTImplementation | DynWinRTImplementationHandle)"
    ));
    assert!(
        artifacts[1]
            .0
            .contains("finally:\n            value.release()")
    );
    assert!(artifacts[1].1.contains(
        "def from_implementation(cls, owner: DynWinRTImplementation | DynWinRTImplementationHandle[object]) -> IBackgroundTaskInstance: ..."
    ));
}

#[test]
fn implementation_public_python_views_type_check_without_internal_constructors() {
    if !Path::new(WINDOWS_WINMD).is_file() {
        eprintln!("Skipping: Windows.winmd is unavailable");
        return;
    }
    let available = Command::new("python")
        .args(["-m", "mypy", "--version"])
        .output();
    if !available.is_ok_and(|output| output.status.success()) {
        assert!(
            std::env::var("DYNWINRT_REQUIRE_MYPY").as_deref() != Ok("1"),
            "DYNWINRT_REQUIRE_MYPY=1 but the existing mypy runner is unavailable"
        );
        eprintln!("Skipping: mypy is unavailable");
        return;
    }
    let tool_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = tool_root
        .join("target")
        .join(format!("public-views-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let package = scratch.join("views");
    let generated = Command::new(env!("CARGO_BIN_EXE_dynwinrt-codegen"))
        .args([
            "generate",
            "--class-name",
            "Windows.ApplicationModel.Background.IBackgroundTask,Windows.ApplicationModel.Background.IBackgroundTaskInstance,Windows.Foundation.IStringable,Windows.Foundation.IClosable,Windows.Foundation.IMemoryBufferReference",
            "--lang",
            "py",
            "--winmd",
            WINDOWS_WINMD,
            "--output",
        ])
        .arg(&package)
        .output()
        .expect("run the existing generator");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let foundation = package.join("windows").join("foundation");
    let delegate_facade = std::fs::read_dir(&foundation)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension().is_some_and(|extension| extension == "pyi")
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("typed_event_handler_background_task_registration_group")
        })
        .expect("transitive BackgroundTaskRegistrationGroup delegate facade");
    let public_name = delegate_facade
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let relative = delegate_facade
        .strip_prefix(&package)
        .unwrap()
        .with_extension("");
    assert!(
        relative.to_string_lossy().len() <= 120,
        "public namespace path exceeded the flat-module budget: {}",
        relative.display()
    );
    let facade = std::fs::read_to_string(&delegate_facade).unwrap();
    let implementation = facade
        .lines()
        .find_map(|line| {
            line.strip_prefix("from ...")
                .and_then(|line| line.split_once(" import "))
                .map(|(module, _)| module)
        })
        .expect("facade import of its canonical implementation");
    assert_eq!(
        implementation.strip_prefix("windows__foundation__"),
        Some(public_name.as_str())
    );
    assert!(package.join(format!("{implementation}.pyi")).is_file());
    assert!(
        std::fs::read_to_string(foundation.join("__init__.pyi"))
            .unwrap()
            .contains(&format!("from .{public_name} import "))
    );
    // Same-named interfaces in distinct namespaces must remain distinct pairs.
    // The foreign-only method shapes also exercise the generic fallback rather
    // than accidentally matching a member of the primary package's closed union.
    let mut next_iid = 1;
    let mut synthetic = |namespace: &str, name: &str, method: &str, result: Option<TypeMeta>| {
        let iid = format!("8755484b-1a1f-48eb-88e9-{next_iid:012x}");
        next_iid += 1;
        InterfaceMeta {
            namespace: namespace.into(),
            name: name.into(),
            iid,
            methods: vec![MethodMeta {
                name: method.into(),
                vtable_index: 6,
                return_type: result,
                ..Default::default()
            }],
            ..Default::default()
        }
    };
    for (name, interfaces) in [
        (
            "pairs",
            vec![
                synthetic("Root", "ITask", "Run", None),
                synthetic("Left", "IValue", "ToString", Some(TypeMeta::String)),
                synthetic("Right", "IValue", "Close", None),
            ],
        ),
        (
            "foreign",
            vec![
                synthetic("External", "IGreeting", "Greet", Some(TypeMeta::String)),
                synthetic("External", "IShutdown", "Shutdown", None),
            ],
        ),
    ] {
        let directory = scratch.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        let context = python::PythonProjectionContext::packaged(
            interfaces.iter().map(InterfaceMeta::type_identity),
        )
        .unwrap();
        let root_interfaces = interfaces
            .iter()
            .filter(|interface| context.root_name_is_unambiguous(&interface.type_identity()))
            .cloned()
            .collect::<Vec<_>>();
        let mut modules = Vec::new();
        for interface in &interfaces {
            let module = context.implementation_module_for_interface(interface);
            std::fs::write(
                directory.join(format!("{module}.pyi")),
                python_stub::generate_interface_stub(&context, interface),
            )
            .unwrap();
            modules.push(module);
        }
        for (module, content) in [
            (
                "__init__.pyi",
                python_stub::generate_index_stub(&context, &[], &root_interfaces, &[]),
            ),
            ("_typing.pyi", python_stub::generate_typing_support_module()),
            ("_runtime.pyi", python_stub::generate_runtime_support_stub()),
            (
                "_implementation_types.pyi",
                python_stub::generate_implementation_pair_types(&modules),
            ),
        ] {
            std::fs::write(directory.join(module), content).unwrap();
        }
    }
    let consumer = scratch.join("consumer.py");
    std::fs::write(
        &consumer,
        r#"
from views import IStringable, IClosable, IBackgroundTask, IBackgroundTaskInstance
from dynwinrt import DynWinRTImplementationHandle, release_projected
from typing import assert_type
from views import IMemoryBufferReferenceImplementationDelegate0
from pairs.root__i_task import ITask as LocalTask
from pairs.left__i_value import IValue as Left
from pairs.right__i_value import IValue as Right
from foreign import IGreeting, IShutdown

class Strings:
    def to_string(self) -> str:
        return "public view"

class Closing:
    def close(self) -> None:
        pass

def consume_stringable(value: IStringable) -> str:
    return value.to_string()

class Task:
    def run(self, instance: IBackgroundTaskInstance | None) -> None:
        if instance is not None:
            instance.progress = 24

def invoke_native_delegate(callback: IMemoryBufferReferenceImplementationDelegate0) -> None:
    callback(None, None)

def consume_task(value: IBackgroundTask, instance: IBackgroundTaskInstance) -> None:
    value.run(instance)

owner = IStringable.implement(Strings(), IClosable.implementation(Closing()))
view: IStringable = IStringable.from_implementation(owner)
text: str = consume_stringable(view)
release_projected(view)
owner.dispose()
task_owner = IBackgroundTask.implement(Task())
task_view: IBackgroundTask = IBackgroundTask.from_implementation(task_owner)
release_projected(task_view)
task_owner.dispose()
with IBackgroundTask.implement(
    Task(), interfaces=[(IStringable, Strings()), (IClosable, Closing())]
) as implementation:
    assert_type(implementation, DynWinRTImplementationHandle[IBackgroundTask])
    assert_type(implementation.value, IBackgroundTask)
    closer = IClosable.from_implementation(implementation)
    closer.close()
    release_projected(closer)
homogeneous = [(IStringable, Strings())]
assert_type(
    IBackgroundTask.implement(Task(), interfaces=homogeneous),
    DynWinRTImplementationHandle[IBackgroundTask],
)

class LocalHandler:
    def run(self) -> None:
        pass

class Greeting:
    def greet(self) -> str:
        return "foreign"

class Shutdown:
    def shutdown(self) -> None:
        pass

assert_type(
    LocalTask.implement(LocalHandler(), interfaces=[(Left, Strings()), (Right, Closing())]),
    DynWinRTImplementationHandle[LocalTask],
)
assert_type(
    LocalTask.implement(LocalHandler(), interfaces=[(IGreeting, Greeting())]),
    DynWinRTImplementationHandle[LocalTask],
)
assert_type(
    LocalTask.implement(
        LocalHandler(), IGreeting.implementation(Greeting()), IShutdown.implementation(Shutdown())
    ),
    DynWinRTImplementationHandle[LocalTask],
)
"#,
    )
    .unwrap();
    let binding_stubs = tool_root
        .join("..")
        .join("..")
        .join("bindings")
        .join("py")
        .canonicalize()
        .unwrap();
    let check = |file: &str| {
        Command::new("python")
            .args([
                "-m",
                "mypy",
                "--strict",
                "--follow-imports=silent",
                "--no-incremental",
                "--cache-dir",
                "mypy-cache",
                file,
            ])
            .current_dir(&scratch)
            .env("PYTHONUTF8", "1")
            .env(
                "MYPYPATH",
                std::env::join_paths([binding_stubs.clone(), std::path::PathBuf::from(".")])
                    .unwrap(),
            )
            .output()
            .expect("run the existing mypy checker")
    };
    let result = check("consumer.py");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let negative = r#"
from consumer import Task, Strings, Closing
from views import IBackgroundTask, IStringable, IClosable
from views import IMemoryBufferReferenceImplementationDelegate0
from consumer import LocalHandler
from pairs.root__i_task import ITask as LocalTask
from pairs.left__i_value import IValue as Left
from pairs.right__i_value import IValue as Right
from foreign import IGreeting

class WrongReturn:
    def to_string(self) -> int:
        return 1

class WrongSignature:
    def to_string(self, required: int) -> str:
        return str(required)

class MissingMethod:
    pass

IBackgroundTask.implement(Task(), interfaces=[(IStringable, Closing()), (IClosable, Strings())])  # swapped
IBackgroundTask.implement(Task(), interfaces=[(IStringable, WrongReturn()), (IClosable, Closing())])  # return
IBackgroundTask.implement(Task(), interfaces=[(IStringable, WrongSignature()), (IClosable, Closing())])  # signature
IBackgroundTask.implement(Task(), interfaces=[(IStringable, MissingMethod()), (IClosable, Closing())])  # missing
IBackgroundTask.implement(Task(), interfaces=[(IStringable, WrongReturn())])  # homogeneous
LocalTask.implement(LocalHandler(), interfaces=[(Left, Closing()), (Right, Strings())])
LocalTask.implement(LocalHandler(), interfaces=[(IGreeting, Closing())])

def reject_native_delegate_calls(callback: IMemoryBufferReferenceImplementationDelegate0) -> None:
    callback(sender=None, args=None)
    callback(None)
    callback(None, None, None)
    callback(None, args=None)
"#;
    std::fs::write(scratch.join("negative.py"), negative).unwrap();
    let rejected = check("negative.py");
    let diagnostics = String::from_utf8_lossy(&rejected.stdout);
    assert!(!rejected.status.success(), "{diagnostics}");
    for (index, line) in negative.lines().enumerate() {
        if line.starts_with("IBackgroundTask.implement(")
            || line.starts_with("LocalTask.implement(")
            || line.trim_start().starts_with("callback(")
        {
            assert!(
                diagnostics.contains(&format!("negative.py:{}: error:", index + 1)),
                "incorrect pair was accepted: {line}\n{diagnostics}"
            );
        }
    }
    std::fs::remove_dir_all(&scratch).unwrap();
}
