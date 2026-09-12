// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;

#[test]
fn consuming_module_wrapper_preserves_result_order_and_validates_resource_inputs() {
    let Some(path) = test_support::metadata() else {
        return;
    };
    let raw = test_support::metadata_function(&path, "Windows.Win32.Foundation", "FreeLibrary");
    let (generated, omitted) =
        generate_apis_files(&test_support::apis(vec![raw]), "@test/runtime/win32");
    assert!(omitted.is_empty());
    test_support::run_js(
        &generated,
        r#"
const assert=require('node:assert/strict')
const moduleResource={}
const calls=[]
let succeeded=true
const runtime={DynWin32:{
  resource(value,cleanup) {
    assert.equal(cleanup,'freeLibrary')
    if(value!==moduleResource) throw new TypeError('managed module resource required')
    return value
  },
  toBoolean:value=>value!==0
},DynWin32Function:{bind(spec) {
  assert.equal(spec.dll,'KERNEL32.dll')
  assert.equal(spec.entryPoint,'FreeLibrary')
  assert.equal(spec.returnType,'bool32')
  assert.equal(spec.returnCleanup,'none')
  assert.equal(spec.successRule,'nonzero')
  assert.equal(spec.captureLastError,true)
  assert.deepEqual(JSON.parse(spec.callContractDescriptor),{
    version:2,
    results:[{
      target:{kind:'return'},
      onSuccess:{kind:'defined',ownership:{kind:'value'},delivery:'deliver'},
      onFailure:{kind:'defined',ownership:{kind:'value'},delivery:'deliver'},
      overrides:[]
    }],
    resourceEffects:[]
  })
  assert.equal(spec.parameters[0].type,'handle')
  assert.equal(spec.parameters[0].direction,'in')
  assert.equal(spec.parameters[0].consumesResource,true)
  assert.equal(spec.parameters[0].resourceCleanup,'freeLibrary')
  return {invoke(args) {
    calls.push(args)
    let reads=0
    return {returnValue:succeeded?1:0,lastError:succeeded?0:5,
      get outputs(){assert.equal(++reads,1);return []}}
  }}
}}}
"#,
        r#"
for(const state of [true,false]) {
  succeeded=state
  const result=projected.freeLibrary(moduleResource)
  assert.deepEqual(Object.keys(result),['result','lastError'])
  assert.equal(result.result,state)
  assert.equal(result.lastError,state?0:5)
  assert.equal(calls.at(-1)[0],moduleResource)
}
const before=calls.length
assert.throws(()=>projected.freeLibrary(123n),/managed module/)
assert.throws(()=>projected.freeLibrary({value:123n}),/managed module/)
assert.equal(calls.length,before)
"#,
    );
}

#[test]
fn registry_native_outcomes_preserve_counts_aliases_and_owned_results() {
    let Some(metadata) = test_support::metadata() else {
        return;
    };
    let mut raw =
        crate::win32_metadata::parse_apis(&metadata, "Windows.Win32.System.Registry", "Apis")
            .unwrap();
    raw.functions.retain(|function| {
        [
            "RegOpenKeyA",
            "RegOpenKeyW",
            "RegOpenKeyExA",
            "RegOpenKeyExW",
            "RegQueryValueExA",
            "RegQueryValueExW",
            "RegGetValueA",
            "RegGetValueW",
        ]
        .contains(&function.name.as_str())
    });
    let (generated, omitted) = generate_apis_files(&raw, "@test/runtime/win32");
    assert!(omitted.is_empty(), "{omitted:?}");
    assert!(
        generated
            .dts
            .contains("key: DynWin32Resource | bigint | null")
    );
    assert!(!generated.js.contains(".value"));
    assert!(generated.dts.contains("dataSize: number | null"));
    for retired_helper in [
        "_hkeyBits",
        "_isPredefinedHkey",
        "_emptyNativeString",
        "_borrowedHkeyOutput",
        "_performanceDataCount",
        "PlanBorrowed",
    ] {
        assert!(!generated.js.contains(retired_helper), "{retired_helper}");
    }
    assert!(generated.js.contains("DynWin32.isUnavailable(_outputs[1])"));
    assert!(
        generated
            .js
            .contains("DynWin32.toResourceOrHandle(_outputs[0])")
    );
    let setup = r#"
const assert = require('node:assert/strict')
const managed = new WeakMap()
const calls = []
const bindings = new Map()
const unavailable = Symbol('native unavailable output')
const discarded = Symbol('native discarded output')
let status = 0
let returned = null
let reportedCount = 0
let aliasConversions = 0
const extent = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), 'byteLength').get
const helpers = {
  handle(value) {
    if (typeof value === 'bigint' || (typeof value === 'number' && Number.isSafeInteger(value)) || managed.has(value)) return { input: value }
    throw new TypeError('not a native handle')
  },
  isUnavailable: value => value === unavailable || value === discarded,
  toNumber(value) {
    assert.notEqual(value, unavailable, 'undefined native output must not be converted')
    return Number(value)
  },
  toResource() { throw new Error('alias-capable outputs must use the native outcome conversion') },
  toResourceOrHandle(value) { aliasConversions++; return value },
  wideString: value => value,
  ansiString: value => value,
  u32: value => value,
  nullPointer: () => null,
  dataPointer: value => value,
  alignedDataPointer: value => value,
  byteLength: value => Reflect.apply(extent, value, []),
}
const predefined = [-2147483648, -2147483647, -2147483646, -2147483645, -2147483644, -2147483643, -2147483642, -2147483568, -2147483552]
const runtime = {
  DynWin32: helpers,
  DynWin32Function: {
    bind(spec) {
      bindings.set(spec.entryPoint, (bindings.get(spec.entryPoint) ?? 0) + 1)
      assert.equal(typeof spec.callContractDescriptor, 'string')
      const contract = JSON.parse(spec.callContractDescriptor)
      const query = /^(RegQueryValueEx|RegGetValue)/.test(spec.entryPoint)
      const get = spec.entryPoint.startsWith('RegGetValue')
      const extended = spec.entryPoint.startsWith('RegOpenKeyEx')
      const target = query ? (get ? 6 : 5) : (extended ? 4 : 2)
      const width = spec.entryPoint.endsWith('W') ? 2 : 1
      const inputs = query
        ? [{kind:'bits-in',parameter:0,mask:0xffffffff,values:[0x80000004]}]
        : [...(extended ? [{kind:'handle-in',parameter:0,values:predefined}] : []),
           {kind:'null-or-empty',parameter:1,elementWidth:width}]
      const valuePolicy = {kind:'defined',ownership:{kind:'value'},delivery:'deliver'}
      const valueResult = target => ({target,onSuccess:valuePolicy,onFailure:valuePolicy,overrides:[]})
      const owned = {kind:'owned',cleanup:'reg-close-key'}
      const result = query ? valueResult({kind:'parameter',index:target}) : {
        target:{kind:'parameter',index:target},
        onSuccess:{kind:'defined',ownership:owned,delivery:'deliver'},
        onFailure:{kind:'undefined'},
        overrides:[]
      }
      result.overrides = query
        ? [{when:{inputs,returnValue:234,succeeded:null},policy:{kind:'undefined'}}]
        : [
            {when:{inputs,returnValue:null,succeeded:true},policy:{kind:'defined',ownership:{kind:'alias-input',parameter:0},delivery:'deliver'}},
            {when:{inputs,returnValue:null,succeeded:false},policy:{kind:'undefined'}}
          ]
      assert.deepEqual(contract, {
        version:2,
        results: [
          valueResult({kind:'return'}),
          ...(query ? [valueResult({kind:'parameter',index:get ? 4 : 3})] : []),
          result
        ],
        resourceEffects:[]
      })
      assert.equal(spec.parameters[target].cleanup, query ? 'none' : 'regCloseKey')
      return {
        invoke(args) {
          calls.push({ spec, args })
          if (query) {
            const outputs = [1, reportedCount]
            const data = args[get ? 4 : 3]
            if (status === 0 && data != null) data.fill(0x5a)
            return { returnValue: status, outputs, succeeded: status === 0 }
          }
          return { returnValue: status, outputs: [returned], succeeded: status === 0 }
        }
      }
    }
  }
}
"#;
    let checks = r#"
for (const name of ['regQueryValueExA', 'regQueryValueExW', 'regGetValueA', 'regGetValueW']) {
  const get = name.startsWith('regGetValue')
  const invoke = (key, value, data) => get
    ? projected[name](key, null, value, 0xffff, data)
    : projected[name](key, value, data)
  const resource = {}
  managed.set(resource, 0xffffffff80000004n)
  Object.defineProperty(resource, 'value', { get() { throw Error('resource.value was read') } })
  for (const key of [0x80000004n, -2147483644n, resource]) {
    status = 0
    reportedCount = 8
    const data = Buffer.alloc(8)
    let result = invoke(key, 'Global', data)
    assert.equal(result.status, 0)
    assert.equal(result.dataSize, 8)
    assert(data.every(byte => byte === 0x5a))
    assert.equal(calls.at(-1).args[get ? 5 : 4], 8)
    assert.equal(calls.at(-1).args[0].input, key)
    status = 234
    reportedCount = unavailable
    result = invoke(key, 'Global', null)
    assert.equal(result.status, 234)
    assert.equal(result.dataSize, null)
    assert.equal(calls.at(-1).args[get ? 5 : 4], 0)
    result = invoke(key, 'Global', Buffer.alloc(1))
    assert.equal(result.status, 234)
    assert.equal(result.dataSize, null)
    assert.equal(calls.at(-1).args[get ? 5 : 4], 1)
  }
  reportedCount = 4096
  let result = invoke(0x80000002n, 'ProductName', Buffer.alloc(1))
  assert.equal(result.status, 234)
  assert.equal(result.dataSize, 4096)
  status = 0
  result = invoke(0x80000002n, 'ProductName', null)
  assert.equal(result.dataSize, 4096)
}
assert.equal(calls.length, 44)
for (const [name, empty] of [
  ['regOpenKeyA', Buffer.from([0])],
  ['regOpenKeyW', Buffer.from([0, 0])],
  ['regOpenKeyExA', Buffer.from([0])],
  ['regOpenKeyExW', Buffer.from([0, 0])],
]) {
  const extended = name.startsWith('regOpenKeyEx')
  const invoke = (key, subkey) => extended
    ? projected[name](key, subkey, 0, 1 | 2)
    : projected[name](key, subkey)
  Object.defineProperty(empty, 'length', { value: 0xffffffff })
  Object.defineProperty(empty, 'byteLength', { value: 0xffffffff })
  for (const subkey of [null, '', empty]) {
    for (const canonical of [-2147483646n, 0xffffffff80000002n]) {
      returned = 0xffffffff80000002n
      const conversions = aliasConversions
      const result = invoke(canonical, subkey)
      assert.equal(result.key, returned)
      assert.equal(calls.at(-1).args[0].input, canonical)
      assert.equal(calls.at(-1).spec.parameters[extended ? 4 : 2].cleanup, 'regCloseKey')
      assert.equal(aliasConversions, conversions + 1)
    }
    const fresh = {owned: 0x310n}
    returned = extended ? fresh : 0x80000002n
    assert.equal(invoke(0x80000002n, subkey).key, returned)
  }
  const owned = { owned: 123n }
  returned = owned
  assert.equal(invoke(0x80000002n, 'Software').key, owned)
  returned = extended ? owned : 42n
  assert.equal(invoke(42n, null).key, returned)
  const input = {}
  managed.set(input, 42n)
  Object.defineProperty(input, 'value', { get() { throw Error('managed owner must not be inspected') } })
  returned = extended ? owned : input
  assert.equal(invoke(input, '').key, returned)
  const predefinedInput = {}
  managed.set(predefinedInput, 0xffffffff80000002n)
  returned = predefinedInput
  assert.equal(invoke(predefinedInput, null).key, predefinedInput)
  const zeroExtendedInput = {}
  managed.set(zeroExtendedInput, 0x80000002n)
  returned = extended ? owned : zeroExtendedInput
  assert.equal(invoke(zeroExtendedInput, null).key, returned)
  status = 5
  for (const nonDelivered of [null, unavailable, discarded]) {
    returned = nonDelivered
    assert.equal(invoke(0x80000002n, null).key, null)
    assert.equal(invoke(input, '').key, null)
  }
  status = 0
}
assert.equal(bindings.size, 8)
assert([...bindings.values()].every(count => count === 1))
"#;
    test_support::run_js(&generated, setup, checks);
}
