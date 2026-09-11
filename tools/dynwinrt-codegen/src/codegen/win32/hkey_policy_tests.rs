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
fn hkey_count_validity_and_conditional_cleanup_execute_without_capability_loss() {
    let Some(metadata) = test_support::metadata() else {
        return;
    };
    let mut raw =
        crate::win32_metadata::parse_apis(&metadata, "Windows.Win32.System.Registry", "Apis")
            .unwrap();
    raw.functions.retain(|function| {
        [
            "RegOpenKeyExA",
            "RegOpenKeyExW",
            "RegQueryValueExA",
            "RegQueryValueExW",
        ]
        .contains(&function.name.as_str())
    });
    let (generated, omitted) = generate_apis_files(&raw, "@test/runtime/win32");
    assert!(omitted.is_empty(), "{omitted:?}");
    assert!(
        generated
            .dts
            .contains("key: DynWin32Resource | null | bigint")
    );
    assert!(!generated.js.contains(".value"));
    assert!(generated.dts.contains("dataSize: number | null"));
    assert!(
        !generated
            .js
            .contains("throw new RangeError('HKEY_PERFORMANCE_DATA")
    );
    let setup = r#"
const assert = require('node:assert/strict')
const vm = require('node:vm')
const managed = new WeakMap()
const calls = []
let status = 0
let returned = 0n
let reportedCount = 0
let undefinedCount = false
let resourceConversions = 0
const extent = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), 'byteLength').get
const helpers = {
  handle(value) {
    if (typeof value === 'bigint') return value
    if (typeof value === 'number' && Number.isSafeInteger(value)) return BigInt(value)
    if (value && managed.has(value)) return managed.get(value)
    throw new TypeError('not a native handle')
  },
  toBigint: value => BigInt(value),
  toNumber: value => Number(value),
  toResource(value) { resourceConversions++; return { owned: value } },
  wideString: value => value,
  ansiString: value => value,
  u32: value => value,
  nullPointer: () => null,
  dataPointer: value => value,
  byteLength: value => Reflect.apply(extent, value, []),
  copyBuffer(value, count) {
    assert(Reflect.apply(extent, value, []) >= count)
    return Buffer.from(Uint8Array.prototype.subarray.call(value, 0, count))
  },
}
const runtime = {
  DynWin32: helpers,
  DynWin32Function: {
    bind(spec) {
      return {
        invoke(args) {
          calls.push({ spec, args })
          if (spec.parameters.length === 6) {
            const outputs = [1, reportedCount]
            if (undefinedCount) Object.defineProperty(outputs, 1, {
              get() { throw new Error('undefined native count was read') }
            })
            if (status === 0 && args[3] != null) args[3].fill(0x5a)
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
for (const name of ['regQueryValueExA', 'regQueryValueExW']) {
  const resource = {}
  managed.set(resource, 0xffffffff80000004n)
  Object.defineProperty(resource, 'value', { get() { throw Error('resource.value was read') } })
  for (const key of [0x80000004n, -2147483644n, resource]) {
    status = 0
    reportedCount = 8
    undefinedCount = false
    const data = Buffer.alloc(8)
    let result = projected[name](key, 'Global', data)
    assert.equal(result.status, 0)
    assert.equal(result.dataSize, 8)
    assert(data.every(byte => byte === 0x5a))
    assert.equal(calls.at(-1).args[4], 8)
    status = 234
    undefinedCount = true
    result = projected[name](key, 'Global', null)
    assert.equal(result.status, 234)
    assert.equal(result.dataSize, null)
    assert.equal(calls.at(-1).args[4], 0)
    result = projected[name](key, 'Global', Buffer.alloc(1))
    assert.equal(result.status, 234)
    assert.equal(result.dataSize, null)
    assert.equal(calls.at(-1).args[4], 1)
  }
  undefinedCount = false
  reportedCount = 4096
  let result = projected[name](0x80000002n, 'ProductName', Buffer.alloc(1))
  assert.equal(result.status, 234)
  assert.equal(result.dataSize, 4096)
  status = 0
  result = projected[name](0x80000002n, 'ProductName', null)
  assert.equal(result.dataSize, 4096)
}
assert.equal(calls.length, 22)
for (const [name, empty] of [
  ['regOpenKeyExA', Buffer.from([0])],
  ['regOpenKeyExW', Buffer.from([0, 0])],
]) {
  Object.defineProperty(empty, 'length', { value: 0xffffffff })
  Object.defineProperty(empty, 'byteLength', { value: 0xffffffff })
  for (const subkey of [null, '', empty]) {
    returned = 0xffffffff80000002n
    const conversions = resourceConversions
    const result = projected[name](0x80000002n, subkey, 0, 1)
    assert.equal(result.key, returned)
    assert.equal(calls.at(-1).spec.parameters[4].cleanup, 'none')
    assert.equal(resourceConversions, conversions)
  }
  returned = 123n
  let result = projected[name](0x80000002n, 'Software', 0, 1)
  assert.equal(result.key.owned, 123n)
  assert.equal(calls.at(-1).spec.parameters[4].cleanup, 'regCloseKey')
  result = projected[name](42n, null, 0, 1)
  assert.equal(result.key.owned, 123n)
  assert.equal(calls.at(-1).spec.parameters[4].cleanup, 'regCloseKey')
  status = 5
  returned = 0xbadf00dn
  result = projected[name](0x80000002n, null, 0, 1)
  assert.equal(result.key, null)
  status = 0
}
"#;
    test_support::run_js(&generated, setup, checks);
}
