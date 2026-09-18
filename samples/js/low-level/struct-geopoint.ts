// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { DynWinRtStruct, DynWinRtValue, roInitialize } from '@microsoft/dynwinrt'
import { registerGeopoint } from './contracts.ts'
import { runExample } from './example.ts'

process.exitCode = await runExample('struct-geopoint', ({ own }) => {
    roInitialize(1)
    const { factory, point, position } = registerGeopoint()
    const expected = [47.643, -122.131, 100]
    const input = DynWinRtStruct.create(position)
    input.setF64(0, expected[0])
    input.setF64(1, expected[1])
    input.setF64(2, expected[2])

    const activation = own(DynWinRtValue.activationFactory('Windows.Devices.Geolocation.Geopoint'))
    const factoryObject = own(activation.cast(factory.iid()))
    const inputValue = own(input.toValue())
    const created = own(factory.methodByName('Create').invoke(factoryObject, [inputValue]))
    const pointObject = own(created.cast(point.iid()))

    // The native constructor receives the struct by value, not a borrowed JS buffer.
    input.setF64(2, 999)
    const returned = own(point.methodByName('get_Position').invoke(pointObject, [])).asStruct()
    const actual = [returned.getF64(0), returned.getF64(1), returned.getF64(2)]
    assert.deepEqual(actual, expected)
    return `Latitude: ${actual[0]}\nLongitude: ${actual[1]}\nAltitude: ${actual[2]} (copied by value)`
})
