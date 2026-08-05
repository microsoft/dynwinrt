// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * 4-way benchmark: C++ static vs Rust raw N-API vs Rust napi-rs vs Dynamic
 *
 * C++ static:       JS → node-addon-api → C++/WinRT → COM vtable
 * Rust raw N-API:   JS → napi-sys (raw) → windows crate → COM vtable
 * Rust napi-rs:     JS → napi-rs (macro) → windows crate → COM vtable
 * Dynamic:          JS → napi-rs → dynwinrt (Rust + libffi) → COM vtable
 */
import {
  DynWinRtValue, DynWinRtType, DynWinRtMethodSig, DynWinRtStruct,
  WinGuid, roInitialize, RustStaticBench,
  rawGetString, rawGetI32,
} from '../../bindings/js/dist/winrt.js'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const CppBench = require('./static-cpp/build/Release/static_bench.node')
const RustRawBench = require('./static-rust/static_bench_rust.node')

roInitialize(1)

// ======================================================================
// Setup
// ======================================================================

// Dynamic: register interfaces
const factoryIid = WinGuid.parse('44A9796F-723E-4FDF-A218-033E75B0C084')
const iUriFactory = DynWinRtType.registerInterface("IUriRuntimeClassFactory", factoryIid)
    .addMethod("CreateUri", new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

const uriIid = WinGuid.parse('9E365E57-48B2-4160-956F-C7385120BBFC')
const iUri = DynWinRtType.registerInterface("IUriRuntimeClass", uriIid)
    .addMethod("get_AbsoluteUri", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_DisplayUri",  new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Domain",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Extension",   new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Fragment",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Host",        new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Password",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Path",        new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Query",       new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_QueryParsed", new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("get_RawUri",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_SchemeName",  new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_UserName",    new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_Port",        new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
    .addMethod("get_Suspicious",  new DynWinRtMethodSig().addOut(DynWinRtType.boolType()))

const pvStaticsIid = WinGuid.parse('629BDBC8-D932-4FF4-96B9-8D96C5C1E858')
const iPvStatics = DynWinRtType.registerInterface("IPropertyValueStatics", pvStaticsIid)
    .addMethod("CreateEmpty",   new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("CreateUInt8",   new DynWinRtMethodSig().addIn(DynWinRtType.u8()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt16",   new DynWinRtMethodSig().addIn(DynWinRtType.i16()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt16",  new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt32",   new DynWinRtMethodSig().addIn(DynWinRtType.i32()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt32",  new DynWinRtMethodSig().addIn(DynWinRtType.u32()).addOut(DynWinRtType.object()))
    .addMethod("CreateInt64",   new DynWinRtMethodSig().addIn(DynWinRtType.i64()).addOut(DynWinRtType.object()))
    .addMethod("CreateUInt64",  new DynWinRtMethodSig().addIn(DynWinRtType.u64()).addOut(DynWinRtType.object()))
    .addMethod("CreateSingle",  new DynWinRtMethodSig().addIn(DynWinRtType.f32()).addOut(DynWinRtType.object()))
    .addMethod("CreateDouble",  new DynWinRtMethodSig().addIn(DynWinRtType.f64()).addOut(DynWinRtType.object()))
    .addMethod("CreateChar16",  new DynWinRtMethodSig().addIn(DynWinRtType.u16()).addOut(DynWinRtType.object()))
    .addMethod("CreateBoolean", new DynWinRtMethodSig().addIn(DynWinRtType.boolType()).addOut(DynWinRtType.object()))
    .addMethod("CreateString",  new DynWinRtMethodSig().addIn(DynWinRtType.hstring()).addOut(DynWinRtType.object()))

// Pre-create objects
const uriFactory = DynWinRtValue.activationFactory('Windows.Foundation.Uri').cast(factoryIid)
const pvStatics = DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue').cast(pvStaticsIid)
const testUrl = "https://example.com:8080/path?q=1"

const mCreateUri = iUriFactory.methodByName("CreateUri")
const mGetHost   = iUri.methodByName("get_Host")
const mGetPort   = iUri.methodByName("get_Port")
const mGetSusp   = iUri.methodByName("get_Suspicious")
const mGetQP     = iUri.methodByName("get_QueryParsed")
const mPvI32     = iPvStatics.methodByName("CreateInt32")
const mPvF64     = iPvStatics.methodByName("CreateDouble")
const mPvBool    = iPvStatics.methodByName("CreateBoolean")
const mPvStr     = iPvStatics.methodByName("CreateString")

const cppUri     = CppBench.uriCreate(testUrl)
const rustRawUri = RustRawBench.uriCreate(testUrl)
const rustUri    = RustStaticBench.uriCreate(testUrl)
const dynUri     = mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring(testUrl)]).cast(uriIid)

// ======================================================================
// Bench helper
// ======================================================================

function bench(name: string, n: number, fn_: () => void): { name: string; ns: number } {
  // warmup
  for (let i = 0; i < Math.min(n, 1000); i++) fn_()
  const t0 = performance.now()
  for (let i = 0; i < n; i++) fn_()
  const ms = performance.now() - t0
  const ns = (ms * 1e6) / n
  return { name, ns }
}

function fmt(ns: number): string {
  if (ns >= 1e6) return `${(ns / 1e6).toFixed(2)} ms`
  if (ns >= 1000) return `${(ns / 1000).toFixed(2)} µs`
  return `${ns.toFixed(0)} ns`
}

// ======================================================================
// Run
// ======================================================================

const N = 50000

console.log(`\n4-way benchmark (${N} iterations)\n`)
console.log('| Operation | C++ static | Rust raw | Rust napi-rs | Dynamic | Raw/C++ | napi-rs/Raw |')
console.log('|-----------|-----------|----------|-------------|---------|---------|-------------|')

function row(op: string, cppFn: () => void, rawFn: () => void, napiFn: () => void, dynFn: () => void) {
  const cpp  = bench(`${op} C++`,    N, cppFn)
  const raw  = bench(`${op} Raw`,    N, rawFn)
  const napi = bench(`${op} napi`,   N, napiFn)
  const dyn_ = bench(`${op} Dyn`,    N, dynFn)
  console.log(
    `| ${op.padEnd(24)} | ${fmt(cpp.ns).padStart(9)} | ${fmt(raw.ns).padStart(8)} | ${fmt(napi.ns).padStart(11)} | ${fmt(dyn_.ns).padStart(7)} | ${(raw.ns / cpp.ns).toFixed(1).padStart(7)}x | ${(napi.ns / raw.ns).toFixed(1).padStart(11)}x |`
  )
}

row('get_Host → hstring',
  () => CppBench.uriHostFromObj(cppUri),
  () => RustRawBench.uriHostFromObj(rustRawUri),
  () => RustStaticBench.uriHostFromObj(rustUri),
  () => mGetHost.invoke(dynUri, []).toString(),
)

row('get_Port → i32',
  () => CppBench.uriPortFromObj(cppUri),
  () => RustRawBench.uriPortFromObj(rustRawUri),
  () => RustStaticBench.uriPortFromObj(rustUri),
  () => mGetPort.invoke(dynUri, []).toNumber(),
)

row('get_Suspicious → bool',
  () => CppBench.uriSuspiciousFromObj(cppUri),
  () => RustRawBench.uriSuspiciousFromObj(rustRawUri),
  () => RustStaticBench.uriSuspiciousFromObj(rustUri),
  () => mGetSusp.invoke(dynUri, []).toBool(),
)

row('get_QueryParsed → object',
  () => CppBench.uriQueryParsedFromObj(cppUri),
  () => RustRawBench.uriQueryParsedFromObj(rustRawUri),
  () => RustStaticBench.uriQueryParsedFromObj(rustUri),
  () => mGetQP.invoke(dynUri, []),
)

row('CreateUri (hstring)',
  () => CppBench.uriCreate('https://example.com'),
  () => RustRawBench.uriCreate('https://example.com'),
  () => RustStaticBench.uriCreate('https://example.com'),
  () => mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring('https://example.com')]),
)

row('PV.CreateInt32 (i32)',
  () => CppBench.pvCreateI32(42),
  () => RustRawBench.pvCreateI32(42),
  () => RustStaticBench.pvCreateI32(42),
  () => mPvI32.invoke(pvStatics, [DynWinRtValue.i32(42)]),
)

row('PV.CreateDouble (f64)',
  () => CppBench.pvCreateF64(3.14),
  () => RustRawBench.pvCreateF64(3.14),
  () => RustStaticBench.pvCreateF64(3.14),
  () => mPvF64.invoke(pvStatics, [DynWinRtValue.f64(3.14)]),
)

row('PV.CreateBoolean (bool)',
  () => CppBench.pvCreateBool(true),
  () => RustRawBench.pvCreateBool(true),
  () => RustStaticBench.pvCreateBool(true),
  () => mPvBool.invoke(pvStatics, [DynWinRtValue.boolValue(true)]),
)

row('PV.CreateString (hstring)',
  () => CppBench.pvCreateString('hello'),
  () => RustRawBench.pvCreateString('hello'),
  () => RustStaticBench.pvCreateString('hello'),
  () => mPvStr.invoke(pvStatics, [DynWinRtValue.hstring('hello')]),
)

const geoStructType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
  DynWinRtType.f64(), DynWinRtType.f64(), DynWinRtType.f64()
])
const geoFactoryIid = WinGuid.parse('DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0')
const iGeoFactory = DynWinRtType.registerInterface("IGeopointFactory", geoFactoryIid)
    .addMethod("Create", new DynWinRtMethodSig().addIn(geoStructType).addOut(DynWinRtType.object()))
const geoFactory = DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint').cast(geoFactoryIid)
const mGeoCreate = iGeoFactory.methodByName("Create")

row('Geopoint (struct 3×f64)',
  () => CppBench.geopointCreate(47.6, -122.1, 100.0),
  () => RustRawBench.geopointCreate(47.6, -122.1, 100.0),
  () => RustStaticBench.geopointCreate(47.6, -122.1, 100.0),
  () => {
    const s = DynWinRtStruct.create(geoStructType)
    s.setF64(0, 47.6); s.setF64(1, -122.1); s.setF64(2, 100.0)
    mGeoCreate.invoke(geoFactory, [s.toValue()])
  },
)

// --- Fast path comparison ---
console.log('\n\n--- Fast path (getString/getI32) vs standard (invoke + toString/toNumber) ---\n')
console.log('| Operation | Standard | Fast path | Speedup |')
console.log('|-----------|----------|-----------|---------|')

function row2(op: string, stdFn: () => void, fastFn: () => void) {
  const std_ = bench(`${op} std`, N, stdFn)
  const fast = bench(`${op} fast`, N, fastFn)
  console.log(
    `| ${op.padEnd(28)} | ${fmt(std_.ns).padStart(8)} | ${fmt(fast.ns).padStart(9)} | ${(std_.ns / fast.ns).toFixed(1).padStart(7)}x |`
  )
}

row2('get_Host → string',
  () => mGetHost.invoke(dynUri, []).toString(),
  () => mGetHost.getString(dynUri),
)

row2('get_Host → string (fn)',
  () => mGetHost.getString(dynUri),
  () => rawGetString(mGetHost, dynUri),
)

row2('get_Port → i32',
  () => mGetPort.invoke(dynUri, []).toNumber(),
  () => mGetPort.getI32(dynUri),
)

row2('get_Port → i32 (fn)',
  () => mGetPort.getI32(dynUri),
  () => rawGetI32(mGetPort, dynUri),
)

row2('CreateUri (hstring→obj)',
  () => mCreateUri.invoke(uriFactory, [DynWinRtValue.hstring('https://example.com')]),
  () => mCreateUri.invokeHstring(uriFactory, 'https://example.com'),
)

row2('PV.CreateInt32 (i32→obj)',
  () => mPvI32.invoke(pvStatics, [DynWinRtValue.i32(42)]),
  () => mPvI32.invokeI32(pvStatics, 42),
)

// --- Raw napi-sys + dynwinrt (theoretical minimum) ---
console.log('\n\n--- Raw napi-sys + dynwinrt vs napi-rs fast path ---\n')

const dynGetters = RustRawBench.dynSetupUriGetters('9E365E57-48B2-4160-956F-C7385120BBFC')

console.log('| Operation | napi-rs fast | Raw napi-sys + dynwinrt | C++ static | Speedup |')
console.log('|-----------|-------------|------------------------|-----------|---------|')

function row3(op: string, napiFn: () => void, rawFn: () => void, cppFn: () => void) {
  const napi_ = bench(`${op} napi`, N, napiFn)
  const raw_  = bench(`${op} raw`,  N, rawFn)
  const cpp_  = bench(`${op} cpp`,  N, cppFn)
  console.log(
    `| ${op.padEnd(24)} | ${fmt(napi_.ns).padStart(11)} | ${fmt(raw_.ns).padStart(22)} | ${fmt(cpp_.ns).padStart(9)} | ${(napi_.ns / raw_.ns).toFixed(1).padStart(7)}x |`
  )
}

row3('get_Host → string',
  () => mGetHost.getString(dynUri),
  () => RustRawBench.dynGetString(dynGetters.getHost),
  () => CppBench.uriHostFromObj(cppUri),
)

row3('get_Port → i32',
  () => mGetPort.getI32(dynUri),
  () => RustRawBench.dynGetI32(dynGetters.getPort),
  () => CppBench.uriPortFromObj(cppUri),
)

console.log('\nDone.')
