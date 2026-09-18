// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { DynWinRtArray, DynWinRtValue, roInitialize } from '@microsoft/dynwinrt'
import { registerPropertyValue } from './contracts.ts'
import { runExample } from './example.ts'

process.exitCode = await runExample('array-property-value', ({ own }) => {
    roInitialize(1)
    const { factory, property } = registerPropertyValue()
    const activation = own(DynWinRtValue.activationFactory('Windows.Foundation.PropertyValue'))
    const factoryObject = own(activation.cast(factory.iid()))
    const output: string[] = []
    for (const expected of [[], [-2147483648, -1, 0, 2147483647]]) {
        const input = own(DynWinRtArray.fromI32Values(expected).toValue())
        // Object is correct here: PropertyValue returns a boxed IInspectable.
        const boxed = own(factory.methodByName('CreateInt32Array').invoke(factoryObject, [input]))
        const propertyObject = own(boxed.cast(property.iid()))
        const returned = own(property.methodByName('GetInt32Array').invoke(propertyObject, []))
        const array = returned.asArray()
        assert.equal(array.len(), expected.length)
        assert.deepEqual(array.toI32Vec(), expected)
        output.push(`Int32 array (${array.len()} elements): ${JSON.stringify(array.toI32Vec())}`)
    }
    return output.join('\n')
})
