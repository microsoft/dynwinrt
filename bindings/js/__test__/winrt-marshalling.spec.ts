// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test, { type ExecutionContext } from 'ava'
import {
  DynWinRtArray,
  DynWinRtMethodSig,
  DynWinRtStruct,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  roInitialize,
} from '../dist/winrt.js'

test.before(() => roInitialize(1))

function own(t: ExecutionContext, value: InstanceType<typeof DynWinRtValue>) {
  t.teardown(() => value.release())
  return value
}

function registerMethodAt(
  name: string,
  iid: InstanceType<typeof WinGuid>,
  slot: number,
  methodName: string,
  signature: InstanceType<typeof DynWinRtMethodSig>,
) {
  let type = DynWinRtType.registerInterface(name, iid)
  // Only the exercised method is callable; preceding slots are never dispatched.
  for (let index = 6; index < slot; index++) {
    type = type.addMethod(`Unused${index}`, new DynWinRtMethodSig())
  }
  return type.addMethod(methodName, signature).method(slot)
}

test('round-trip WinRT arrays through PropertyValue', (t) => {
  const staticsIid = WinGuid.parse('629bdbc8-d932-4ff4-96b9-8d96c5c1e858')
  const propertyIid = WinGuid.parse('4bd682dd-7554-40e9-9a9b-82654ede7e62')
  const arrayType = DynWinRtType.arrayType(DynWinRtType.i32())
  const createArray = registerMethodAt(
    'IPropertyValueStaticsArrayTest',
    staticsIid,
    29,
    'CreateInt32Array',
    new DynWinRtMethodSig().addIn(arrayType).addOut(DynWinRtType.object()),
  )
  const getArray = registerMethodAt(
    'IPropertyValueArrayTest',
    propertyIid,
    29,
    'GetInt32Array',
    new DynWinRtMethodSig().addOut(arrayType),
  )
  const activationFactory = own(t, DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue'))
  const factory = own(t, activationFactory.cast(staticsIid))

  for (const expected of [[], [10, 20, 30], [100, 200, 300, 400, 500], [-2147483648, -1, 0, 2147483647]]) {
    const input = own(t, DynWinRtArray.fromI32Values(expected).toValue())
    const boxed = own(t, createArray.invoke(factory, [input]))
    const property = own(t, boxed.cast(propertyIid))
    const returned = own(t, getArray.invoke(property, []))
    const array = returned.asArray()

    t.is(array.len(), expected.length)
    t.deepEqual(array.toI32Vec(), expected)
    t.deepEqual(
      array.toValues().map((value) => value.toNumber()),
      expected,
    )
    expected.forEach((value, index) => t.is(array.get(index).toNumber(), value))
  }
})

test('round-trip WinRT structs through Geopoint', (t) => {
  const positionType = DynWinRtType.structType('Windows.Devices.Geolocation.BasicGeoposition', [
    DynWinRtType.f64(),
    DynWinRtType.f64(),
    DynWinRtType.f64(),
  ])
  const factoryIid = WinGuid.parse('db6b8d33-76bd-4e30-8af7-a844dc37b7a0')
  const geopointIid = WinGuid.parse('6bfa00eb-e56e-49bb-9caf-cbaa78a8bcef')
  const factoryType = DynWinRtType.registerInterface('IGeopointFactoryMarshallingTest', factoryIid).addMethod(
    'Create',
    new DynWinRtMethodSig().addIn(positionType).addOut(DynWinRtType.interface(geopointIid)),
  )
  const geopointType = DynWinRtType.registerInterface('IGeopointMarshallingTest', geopointIid).addMethod(
    'get_Position',
    new DynWinRtMethodSig().addOut(positionType),
  )
  const activationFactory = own(t, DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint'))
  const factory = own(t, activationFactory.cast(factoryIid))

  for (const expected of [
    [47.643, -122.131, 100],
    [-33.86, 151.21, -25],
    [0, 0, 0],
  ]) {
    const input = DynWinRtStruct.create(positionType)
    expected.forEach((value, index) => input.setF64(index, value))
    const inputValue = own(t, input.toValue())
    const point = own(t, factoryType.method(6).invoke(factory, [inputValue]))

    // BasicGeoposition is passed by value, not retained as caller-owned storage.
    expected.forEach((_, index) => input.setF64(index, 0))
    const position = own(t, geopointType.method(6).invoke(point, [])).asStruct()
    t.deepEqual([position.getF64(0), position.getF64(1), position.getF64(2)], expected)
  }
})
