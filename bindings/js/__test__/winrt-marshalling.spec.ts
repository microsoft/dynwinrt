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

test('unsigned enums retain SDK IIDs and high-bit values across native storage', (t) => {
  const flags = DynWinRtType.enumType(
    'Windows.Gaming.Input.GamepadButtons',
    ['High', 'All'],
    [0x80000000, 0xffffffff],
    DynWinRtType.u32(),
  )
  const vector = DynWinRtType.parameterized(WinGuid.parse('913337e9-11a1-4345-a3a2-4e7f956e222d'), [flags])
  const reference = DynWinRtType.parameterized(WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'), [flags])
  t.is(vector.iid().toString().toLowerCase(), '2da67b4c-03d3-57c7-8257-6c30837177ac')
  t.is(reference.iid().toString().toLowerCase(), '9b7e3bfb-85a9-5b98-81b3-1af4a060a6f5')
  t.is(DynWinRtType.getEnumValue('Windows.Gaming.Input.GamepadButtons', 'All'), 0xffffffff)
  const getter = DynWinRtType.registerInterface('EnumContracts.IReferenceFlags', reference.iid())
    .addMethod('get_Value', new DynWinRtMethodSig().addOut(flags))
    .method(6)

  const values = [0, 0x80000000, 0xffffffff].map((value) => own(t, DynWinRtValue.enumValue(flags, value)))
  for (const [index, expected] of [0, 0x80000000, 0xffffffff].entries()) {
    t.is(values[index].toNumber(), expected)
    t.is(values[index].getEnumInt(), expected)
    const boxed = own(t, DynWinRtValue.boxReference(values[index], flags))
    const view = own(t, boxed.cast(reference.iid()))
    t.is(own(t, getter.invoke(view, [])).toNumber(), expected)
    t.throws(() => getter.getI32(view))
  }
  t.is(values[2].getEnumName(), 'All')
  const array = DynWinRtArray.fromObjectValues(values, flags)
  t.deepEqual(array.toU32Vec(), [0, 0x80000000, 0xffffffff])
  t.throws(() => array.toI32Vec())
  const record = DynWinRtStruct.create(DynWinRtType.structType('EnumContracts.FlagRecord', [flags]))
  record.setU32(0, 0xffffffff)
  t.is(record.getU32(0), 0xffffffff)
  t.throws(() => record.getI32(0))

  for (const value of [-1, 2 ** 32, 1.5, NaN, Infinity]) {
    t.throws(() => DynWinRtValue.enumValue(flags, value))
  }
  t.throws(() => DynWinRtValue.enumValue(DynWinRtType.u32(), 1))
  t.throws(() => DynWinRtType.enumType('Windows.Gaming.Input.GamepadButtons'), { message: /backing type/ })
  t.throws(() => DynWinRtType.enumType('EnumContracts.Bad', [], [], DynWinRtType.u64()))
  t.throws(() => DynWinRtType.enumType('EnumContracts.Range', ['Bad'], [-1], DynWinRtType.u32()))
  t.throws(() => DynWinRtType.enumType('EnumContracts.Length', ['Missing'], []))

  const signed = DynWinRtType.enumType('EnumContracts.Signed', ['Negative'], [-1])
  const negative = own(t, DynWinRtValue.enumValue(signed, -1))
  t.is(negative.toNumber(), -1)
  t.is(negative.getEnumInt(), -1)
  t.is(DynWinRtType.getEnumValue('EnumContracts.Signed', 'Negative'), -1)
  t.throws(() => DynWinRtValue.enumValue(signed, 0x80000000))
})

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
