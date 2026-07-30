// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Benchmark: JS → C++/WinRT static projection vs JS → dynwinrt (end-to-end)
 *
 * Static path:  JS → node-addon-api → C++/WinRT → COM vtable
 * Dynamic path: JS → napi-rs → dynwinrt (Rust + libffi) → COM vtable
 *
 * Groups:
 *   1. param_count  — 0, 1, 2 input params
 *   2. input_type   — same param count (1 in), different input types
 *   3. return_type  — same param count (0 in), different return types
 *   4. struct_param — struct in-param
 *   5. caching      — cached vs uncached method handle
 *   6. batch        — realistic workload
 *
 * All method handles and objects are pre-created.
 * Run: npx tsx samples/benchmark.ts
 */
import {
  DynWinRtValue,
  DynWinRtType,
  DynWinRtMethodSig,
  DynWinRtStruct,
  WinGuid,
  roInitialize,
} from '../dist/winrt.js'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const CppBench = require('../static-bench/build/Release/static_bench.node')

roInitialize(1)

// ======================================================================
// Setup (not measured)
// ======================================================================

// --- Uri interfaces ---
const factoryIid = WinGuid.parse('44A9796F-723E-4FDF-A218-033E75B0C084')
const iUriFactory = DynWinRtType.registerInterface("IUriRuntimeClassFactory", factoryIid)
    .addMethod("CreateUri", new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))
    .addMethod("CreateWithRelativeUri", new DynWinRtMethodSig()
        .addIn(DynWinRtType.hstring()).addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

const uriIid = WinGuid.parse('9E365E57-48B2-4160-956F-C7385120BBFC')
const iUri = DynWinRtType.registerInterface("IUriRuntimeClass", uriIid)
    .addMethod("get_AbsoluteUri", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_DisplayUri",  new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_RawUri",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_SchemeName",  new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_UserName",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Password",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Host",        new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Domain",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Port",        new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
    .addMethod("get_Path",        new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Query",       new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_QueryParsed", new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("get_Fragment",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Extension",   new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Suspicious",  new DynWinRtMethodSig().addOut(DynWinRtType.boolType()))
    .addMethod("Equals",          new DynWinRtMethodSig().addIn(DynWinRtType.object()).addOut(DynWinRtType.boolType()))
    .addMethod("CombineUri",      new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

// --- PropertyValue interface ---
const pvStaticsIid = WinGuid.parse('629BDBC8-D932-4FF4-96B9-8D96C5C1E858')
const iPvStatics = DynWinRtType.registerInterface("IPropertyValueStatics", pvStaticsIid)
    .addMethod("CreateEmpty",       new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("CreateUInt8",       new DynWinRtMethodSig().addIn(DynWinRtType.u8()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt16",       new DynWinRtMethodSig().addIn(DynWinRtType.i16()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt16",      new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt32",       new DynWinRtMethodSig().addIn(DynWinRtType.i32()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt32",      new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt64",       new DynWinRtMethodSig().addIn(DynWinRtType.i64()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt64",      new DynWinRtMethodSig().addIn(DynWinRtType.u64()).addOut(DynWinRtType.object()))
    .addMethod("CreateSingle",      new DynWinRtMethodSig().addIn(DynWinRtType.f32()).addOut(DynWinRtType.object()))
    .addMethod("CreateDouble",      new DynWinRtMethodSig().addIn(DynWinRtType.f64()).addOut(DynWinRtType.object()))
    .addMethod("CreateChar16",      new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object()))
    .addMethod("CreateBoolean",     new DynWinRtMethodSig().addIn(DynWinRtType.boolType()).addOut(DynWinRtType.object()))
    .addMethod("CreateString",      new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

// --- Geopoint ---
const geoFactoryIid = WinGuid.parse('DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0')
const geoStructType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
    DynWinRtType.f64(), DynWinRtType.f64(), DynWinRtType.f64()
])
const iGeoFactory = DynWinRtType.registerInterface("IGeopointFactory", geoFactoryIid)
    .addMethod("Create", new DynWinRtMethodSig().addIn(geoStructType).addOut(DynWinRtType.object()))

// --- Pre-create factories & objects ---
const uriFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid)
const pvStatics = DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue').cast(pvStaticsIid)
const geoFactory = DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint').cast(geoFactoryIid)

const testUrl = "https://example.com:8080/path?q=1"
const staticUri = CppBench.uriCreate(testUrl)
const createUri = iUriFactory.methodByName("CreateUri")
const dynUri = createUri.invoke(uriFactory, [DynWinRtValue.hstring(testUrl)]).cast(uriIid)

// --- Cache all method handles ---
const mCreateUri = iUriFactory.methodByName("CreateUri")
const mCreateWithRelative = iUriFactory.methodByName("CreateWithRelativeUri")
const mGetHost = iUri.methodByName("get_Host")
const mGetPort = iUri.methodByName("get_Port")
const mGetSuspicious = iUri.methodByName("get_Suspicious")
const mCombineUri = iUri.methodByName("CombineUri")
const mEquals = iUri.methodByName("Equals")
const mPvCreateI32 = iPvStatics.methodByName("CreateInt32")
const mPvCreateF64 = iPvStatics.methodByName("CreateDouble")
const mPvCreateBool = iPvStatics.methodByName("CreateBoolean")
const mPvCreateStr = iPvStatics.methodByName("CreateString")
const mGeoCreate = iGeoFactory.methodByName("Create")

// ======================================================================
// Benchmark helpers
// ======================================================================

const N_FAST = 50_000
const N_SLOW = 10_000
const N_BATCH = 200

function bench(name: string, iterations: number, fn: () => void) {
  for (let i = 0; i < Math.min(iterations, 1000); i++) fn()
  const start = performance.now()
  for (let i = 0; i < iterations; i++) fn()
  const elapsed = performance.now() - start
  return { name, perCall: (elapsed * 1_000_000) / iterations }
}

function fmtTime(ns: number): string {
  if (ns >= 1_000_000) return `${(ns / 1_000_000).toFixed(2)} ms`
  if (ns >= 1000) return `${(ns / 1000).toFixed(2)} µs`
  return `${ns.toFixed(0)} ns`
}

function printGroup(groupName: string, results: { name: string, perCall: number }[]) {
  console.log(`\n--- ${groupName} ---`)
  const maxName = Math.max(...results.map(r => r.name.length))
  for (const r of results) {
    console.log(`  ${r.name.padEnd(maxName)}  ${fmtTime(r.perCall)}`)
  }
}

// ======================================================================
// Benchmarks
// ======================================================================

console.log('=== JS End-to-End Benchmark: C++/WinRT (static) vs dynwinrt (dynamic) ===')
console.log(`Static:  JS → node-addon-api → C++/WinRT → COM`)
console.log(`Dynamic: JS → napi-rs → dynwinrt (Rust + libffi) → COM`)
console.log(`(all method handles and objects pre-created)\n`)

// --- Group 1: Parameter Count ---
// Fixed return type, varying input count

const cachedRel = DynWinRtValue.hstring("/rel")
const cachedBase = DynWinRtValue.hstring("https://example.com")
const cachedPath = DynWinRtValue.hstring("/path")

printGroup('By param count (→ object out)', [
  bench('0 in / static',        N_FAST, () => CppBench.uriHostFromObj(staticUri)),
  bench('0 in / dynamic',       N_FAST, () => mGetHost.invoke(dynUri, []).toString()),
  bench('1 in / static',        N_SLOW, () => CppBench.uriCombine(staticUri, "/rel")),
  bench('1 in / uncached',      N_SLOW, () => mCombineUri.invoke(dynUri, [DynWinRtValue.hstring("/rel")]).toString()),
  bench('1 in / cached',        N_SLOW, () => mCombineUri.invoke(dynUri, [cachedRel]).toString()),
  bench('2 in / static',        N_SLOW, () => CppBench.uriCreateWithRelative("https://example.com", "/path")),
  bench('2 in / uncached',      N_SLOW, () => mCreateWithRelative.invoke(uriFactory, [
    DynWinRtValue.hstring("https://example.com"), DynWinRtValue.hstring("/path")])),
  bench('2 in / cached',        N_SLOW, () => mCreateWithRelative.invoke(uriFactory, [cachedBase, cachedPath])),
])

// --- Group 2: Input Type ---
// Fixed: 1 in → 1 out (object), varying input type

printGroup('By input type (1 in → object out)', [
  bench('i32 / static',        N_SLOW, () => CppBench.pvCreateI32(42)),
  bench('i32 / dynamic',       N_SLOW, () => mPvCreateI32.invoke(pvStatics, [DynWinRtValue.i32(42)])),
  bench('f64 / static',        N_SLOW, () => CppBench.pvCreateF64(3.14)),
  bench('f64 / dynamic',       N_SLOW, () => mPvCreateF64.invoke(pvStatics, [DynWinRtValue.f64(3.14)])),
  bench('bool / static',       N_SLOW, () => CppBench.pvCreateBool(true)),
  bench('bool / dynamic',      N_SLOW, () => mPvCreateBool.invoke(pvStatics, [DynWinRtValue.boolValue(true)])),
  bench('hstring / static',    N_SLOW, () => CppBench.pvCreateString("hello")),
  bench('hstring / dynamic',   N_SLOW, () => mPvCreateStr.invoke(pvStatics, [DynWinRtValue.hstring("hello")])),
  bench('struct 3×f64 / static',  N_SLOW, () => CppBench.geopointCreate(47.6, -122.1, 100.0)),
  bench('struct 3×f64 / dynamic', N_SLOW, () => {
    const s = DynWinRtStruct.create(geoStructType)
    s.setF64(0, 47.6); s.setF64(1, -122.1); s.setF64(2, 100.0)
    mGeoCreate.invoke(geoFactory, [s.toValue()])
  }),
])

// --- Group 3: Arg caching (napi call savings) ---
// Compares: creating DynWinRtValue each call (2 napi) vs pre-cached arg (1 napi)

const cachedI32 = DynWinRtValue.i32(42)
const cachedF64 = DynWinRtValue.f64(3.14)
const cachedBool = DynWinRtValue.boolValue(true)
const cachedStr = DynWinRtValue.hstring("hello")

printGroup('Arg caching: 1 napi (cached) vs 2 napi (uncached)', [
  bench('i32 / cached',     N_SLOW, () => mPvCreateI32.invoke(pvStatics, [cachedI32])),
  bench('i32 / uncached',   N_SLOW, () => mPvCreateI32.invoke(pvStatics, [DynWinRtValue.i32(42)])),
  bench('f64 / cached',     N_SLOW, () => mPvCreateF64.invoke(pvStatics, [cachedF64])),
  bench('f64 / uncached',   N_SLOW, () => mPvCreateF64.invoke(pvStatics, [DynWinRtValue.f64(3.14)])),
  bench('bool / cached',    N_SLOW, () => mPvCreateBool.invoke(pvStatics, [cachedBool])),
  bench('bool / uncached',  N_SLOW, () => mPvCreateBool.invoke(pvStatics, [DynWinRtValue.boolValue(true)])),
  bench('hstring / cached',  N_SLOW, () => mPvCreateStr.invoke(pvStatics, [cachedStr])),
  bench('hstring / uncached',N_SLOW, () => mPvCreateStr.invoke(pvStatics, [DynWinRtValue.hstring("hello")])),
])

// --- Group 4: Return Type ---
// Fixed: 0 in → 1 out getter on pre-created Uri, varying return type

printGroup('By return type (0 in → getter)', [
  bench('i32 / static',     N_FAST, () => CppBench.uriPortFromObj(staticUri)),
  bench('i32 / dynamic',    N_FAST, () => mGetPort.invoke(dynUri, []).toNumber()),
  bench('bool / static',    N_FAST, () => CppBench.uriSuspiciousFromObj(staticUri)),
  bench('bool / dynamic',   N_FAST, () => mGetSuspicious.invoke(dynUri, []).toBool()),
  bench('hstring / static', N_FAST, () => CppBench.uriHostFromObj(staticUri)),
  bench('hstring / dynamic',N_FAST, () => mGetHost.invoke(dynUri, []).toString()),
])

// --- Group 4: Method Handle Caching ---

printGroup('Method handle caching', [
  bench('cached',   N_FAST, () => mGetHost.invoke(dynUri, []).toString()),
  bench('uncached', N_FAST, () => iUri.methodByName("get_Host").invoke(dynUri, []).toString()),
])

// --- Group 5: Batch ---

printGroup(`Batch ${N_BATCH}× create Uri + read Host`, [
  bench('static', 100, () => {
    for (let i = 0; i < N_BATCH; i++) CppBench.uriGetHost(`https://example.com/${i}`)
  }),
  bench('dynamic', 100, () => {
    for (let i = 0; i < N_BATCH; i++) {
      const u = mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring(`https://example.com/${i}`)]).cast(uriIid)
      mGetHost.invoke(u, []).toString()
    }
  }),
])

console.log('\n=== Done ===')
