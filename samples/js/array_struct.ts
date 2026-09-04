// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Test: Array and Struct bindings for dynwinrt
 *
 * Tests:
 * 1. PropertyValue.CreateInt32Array  — PassArray (JS → WinRT)
 * 2. PropertyValue.GetInt32Array     — ReceiveArray (WinRT → JS), read via toI32Vec()
 * 3. Geopoint.Create(BasicGeoposition) — Struct in-param
 * 4. Geopoint.Position               — Struct out-param
 */

import {
  DynWinRtValue,
  DynWinRtType,
  DynWinRtMethodSig,
  DynWinRtArray,
  DynWinRtStruct,
  WinGuid,
  roInitialize,
} from '../../bindings/js/dist/winrt.js'

// Initialize WinRT (MTA)
roInitialize(1)

function registerMethodAt(
  name: string,
  iid: WinGuid,
  vtableIndex: number,
  methodName: string,
  signature: DynWinRtMethodSig,
) {
  const iface = DynWinRtType.registerInterface(name, iid)
  for (let index = 6; index < vtableIndex; index++) {
    iface.addMethod(`Reserved${index}`, new DynWinRtMethodSig())
  }
  return iface.addMethod(methodName, signature)
}

// ======================================================================
// 1. PassArray: PropertyValue.CreateInt32Array
// ======================================================================
function testPassArray() {
  console.log('\n--- Test: PassArray (CreateInt32Array) ---')

  // IPropertyValueStatics: {629BDBC8-D932-4FF4-96B9-8D96C5C1E858}
  const staticsIid = WinGuid.parse('629BDBC8-D932-4FF4-96B9-8D96C5C1E858')

  const factory = DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue')
  const statics = factory.cast(staticsIid)
  const propertyValueStatics = registerMethodAt(
    'IPropertyValueStatics',
    staticsIid,
    29,
    'CreateInt32Array',
    new DynWinRtMethodSig().addIn(DynWinRtType.arrayType(DynWinRtType.i32())).addOut(DynWinRtType.object()),
  )

  const arr = DynWinRtArray.fromI32Values([10, 20, 30])
  const result = propertyValueStatics.method(29).invoke(statics, [arr.toValue()])
  console.log('  CreateInt32Array result:', result.toString())

  // Verify by reading back: cast to IPropertyValue, call GetInt32Array
  const ipvIid = WinGuid.parse('4BD682DD-7554-40E9-9A9B-82654EDE7E62')
  const ipv = result.cast(ipvIid)
  const propertyValue = registerMethodAt(
    'IPropertyValue',
    ipvIid,
    29,
    'GetInt32Array',
    new DynWinRtMethodSig().addOut(DynWinRtType.arrayType(DynWinRtType.i32())),
  )
  const readback = propertyValue.method(29).invoke(ipv, [])

  const arrayResult = readback.asArray()
  const ints = arrayResult.toI32Vec()
  console.log('  Read back:', ints)
  console.assert(ints.length === 3, `Expected 3 elements, got ${ints.length}`)
  console.assert(ints[0] === 10, `Expected ints[0]=10, got ${ints[0]}`)
  console.assert(ints[1] === 20, `Expected ints[1]=20, got ${ints[1]}`)
  console.assert(ints[2] === 30, `Expected ints[2]=30, got ${ints[2]}`)
  console.log('  PASS')
}

// ======================================================================
// 2. ReceiveArray: PropertyValue.GetInt32Array
// ======================================================================
function testReceiveArray() {
  console.log('\n--- Test: ReceiveArray (GetInt32Array) ---')

  // Create a PropertyValue with known data via CreateInt32Array
  const staticsIid = WinGuid.parse('629BDBC8-D932-4FF4-96B9-8D96C5C1E858')
  const factory = DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue')
  const statics = factory.cast(staticsIid)
  const propertyValueStatics = registerMethodAt(
    'IPropertyValueStatics',
    staticsIid,
    29,
    'CreateInt32Array',
    new DynWinRtMethodSig().addIn(DynWinRtType.arrayType(DynWinRtType.i32())).addOut(DynWinRtType.object()),
  )

  const arr = DynWinRtArray.fromI32Values([100, 200, 300, 400, 500])
  const pv = propertyValueStatics.method(29).invoke(statics, [arr.toValue()])

  // Cast to IPropertyValue and call GetInt32Array
  const ipvIid = WinGuid.parse('4BD682DD-7554-40E9-9A9B-82654EDE7E62')
  const ipv = pv.cast(ipvIid)
  const propertyValue = registerMethodAt(
    'IPropertyValue',
    ipvIid,
    29,
    'GetInt32Array',
    new DynWinRtMethodSig().addOut(DynWinRtType.arrayType(DynWinRtType.i32())),
  )
  const result = propertyValue.method(29).invoke(ipv, [])

  const array = result.asArray()
  console.log('  Array length:', array.len())
  console.assert(array.len() === 5, `Expected 5, got ${array.len()}`)

  // Fast path: toI32Vec
  const vec = array.toI32Vec()
  console.log('  toI32Vec:', vec)
  console.assert(vec[0] === 100)
  console.assert(vec[4] === 500)

  // Generic path: per-element get
  const v2 = array.get(2)
  console.log('  get(2):', v2.toNumber())
  console.assert(v2.toNumber() === 300)

  console.log('  PASS')
}

// ======================================================================
// 3. Struct in-param: Geopoint.Create(BasicGeoposition)
// ======================================================================
function testStructIn() {
  console.log('\n--- Test: Struct In-Param (Geopoint.Create) ---')

  // BasicGeoposition = { f64 Latitude, f64 Longitude, f64 Altitude }
  const geoposType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
    DynWinRtType.f64(),
    DynWinRtType.f64(),
    DynWinRtType.f64(),
  ])

  const pos = DynWinRtStruct.create(geoposType)
  pos.setF64(0, 47.643) // Latitude
  pos.setF64(1, -122.131) // Longitude
  pos.setF64(2, 100.0) // Altitude
  console.log('  Created struct: lat=%d lon=%d alt=%d', pos.getF64(0), pos.getF64(1), pos.getF64(2))

  // Get IGeopointFactory
  const factory = DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint')

  // IGeopointFactory: {DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0}
  const factoryIid = WinGuid.parse('DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0')
  const gpFactory = factory.cast(factoryIid)
  const geopointFactory = DynWinRtType.registerInterface('IGeopointFactory', factoryIid).addMethod(
    'Create',
    new DynWinRtMethodSig().addIn(geoposType).addOut(DynWinRtType.object()),
  )

  // IGeopointFactory.Create(BasicGeoposition) at vtable index 6
  const geopoint = geopointFactory.method(6).invoke(gpFactory, [pos.toValue()])
  console.log('  Geopoint created:', geopoint.toString())
  console.log('  PASS')
}

// ======================================================================
// 4. Struct out-param: Geopoint.Position
// ======================================================================
function testStructOut() {
  console.log('\n--- Test: Struct Out-Param (Geopoint.Position) ---')

  // First create a Geopoint
  const geoposType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
    DynWinRtType.f64(),
    DynWinRtType.f64(),
    DynWinRtType.f64(),
  ])

  const pos = DynWinRtStruct.create(geoposType)
  pos.setF64(0, 47.643)
  pos.setF64(1, -122.131)
  pos.setF64(2, 100.0)

  const factory = DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint')
  const factoryIid = WinGuid.parse('DB6B8D33-76BD-4E30-8AF7-A844DC37B7A0')
  const gpFactory = factory.cast(factoryIid)
  const geopointFactory = DynWinRtType.registerInterface('IGeopointFactory', factoryIid).addMethod(
    'Create',
    new DynWinRtMethodSig().addIn(geoposType).addOut(DynWinRtType.object()),
  )
  const geopoint = geopointFactory.method(6).invoke(gpFactory, [pos.toValue()])

  // IGeopoint: {6BFA00EB-E56E-49BB-9CAF-CBAA78A8BCEF} — .Position at vtable 6
  const igeopointIid = WinGuid.parse('6BFA00EB-E56E-49BB-9CAF-CBAA78A8BCEF')
  const igeopoint = geopoint.cast(igeopointIid)
  const geopointInterface = DynWinRtType.registerInterface('IGeopoint', igeopointIid).addMethod(
    'get_Position',
    new DynWinRtMethodSig().addOut(geoposType),
  )
  const posResult = geopointInterface.method(6).invoke(igeopoint, [])

  const s = posResult.asStruct()
  const lat = s.getF64(0)
  const lon = s.getF64(1)
  const alt = s.getF64(2)
  console.log('  Position: lat=%d lon=%d alt=%d', lat, lon, alt)
  console.assert(Math.abs(lat - 47.643) < 1e-6, `Latitude mismatch: ${lat}`)
  console.assert(Math.abs(lon - -122.131) < 1e-6, `Longitude mismatch: ${lon}`)
  console.assert(Math.abs(alt - 100.0) < 1e-6, `Altitude mismatch: ${alt}`)
  console.log('  PASS')
}

// ======================================================================
// Run all tests
// ======================================================================
console.log('=== Array & Struct Binding Tests ===')
testPassArray()
testReceiveArray()
testStructIn()
testStructOut()
console.log('\n=== All tests passed ===')
