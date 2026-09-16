// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DynWinRtValue, roInitialize } from '@microsoft/dynwinrt'
import { registerAsyncFile } from './contracts.ts'
import { runExample } from './example.ts'

process.exitCode = await runExample('async-file', async ({ own, defer }) => {
    roInitialize(1)
    const directory = mkdtempSync(join(tmpdir(), 'dynwinrt-low-level-'))
    defer('owned temporary directory', () => rmSync(directory, { recursive: true }))
    const filename = join(directory, 'input.txt')
    const contents = 'Hello from a typed WinRT async operation.\n'
    writeFileSync(filename, contents, { flag: 'wx' })

    const { factory, file, stream, closable, accessMode } = registerAsyncFile()
    const activation = own(DynWinRtValue.activationFactory('Windows.Storage.StorageFile'))
    const factoryObject = own(activation.cast(factory.iid()))
    const path = own(DynWinRtValue.hstring(filename))
    const fileOperation = own(factory.methodByName('GetFileFromPathAsync').invoke(factoryObject, [path]))
    // The contract is IAsyncOperation<StorageFile>, not Object.
    const fileResult = own(await fileOperation.toPromise())
    const fileObject = own(fileResult.cast(file.iid()))
    const read = own(DynWinRtValue.enumValue(accessMode, 0))
    const openOperation = own(file.methodByName('OpenAsync').invoke(fileObject, [read]))
    // This different closed async type returns IRandomAccessStream.
    const streamResult = own(await openOperation.toPromise())
    const closeObject = own(streamResult.cast(closable.iid()))
    defer('IClosable.Close on the file stream', () => {
        closable.methodByName('Close').invoke(closeObject, []).release()
    })
    const streamObject = own(streamResult.cast(stream.iid()))
    const size = own(stream.methodByName('get_Size').invoke(streamObject, [])).toU64Bigint()
    assert.equal(size, BigInt(Buffer.byteLength(contents)))
    return `Opened owned input.txt asynchronously: ${size} bytes.\nStream closed; temporary directory removed.`
})
