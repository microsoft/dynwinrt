// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { DynWinRtValue, roInitialize } from '@microsoft/dynwinrt'
import { registerUri } from './contracts.ts'
import { runExample } from './example.ts'

process.exitCode = await runExample('uri', ({ own }) => {
    roInitialize(1)
    const { factory, uri } = registerUri()
    const activation = own(DynWinRtValue.activationFactory('Windows.Foundation.Uri'))
    const factoryObject = own(activation.cast(factory.iid()))
    const input = own(DynWinRtValue.hstring('https://example.com/low-level?q=42'))
    const created = own(factory.methodByName('CreateUri').invoke(factoryObject, [input]))
    const uriObject = own(created.cast(uri.iid()))

    const host = own(uri.methodByName('get_Host').invoke(uriObject, [])).toString()
    const path = own(uri.methodByName('get_Path').invoke(uriObject, [])).toString()
    const port = own(uri.methodByName('get_Port').invoke(uriObject, [])).toNumber()
    assert.equal(host, 'example.com')
    assert.equal(path, '/low-level')
    assert.equal(port, 443)
    return `Host: ${host}\nPath: ${path}\nPort: ${port}`
})
