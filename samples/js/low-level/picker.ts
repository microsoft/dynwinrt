// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { parseArgs } from 'node:util'
import * as runtime from '@microsoft/dynwinrt'
import { registerPicker } from './contracts.ts'
import { runExample } from './example.ts'
import { initializePicker, pickerOutcome } from './picker-support.ts'

let resultExitCode = 0
process.exitCode = await runExample('picker', async ({ own }) => {
    const { values } = parseArgs({
        options: { 'no-ui': { type: 'boolean', default: false } },
        allowPositionals: false,
        strict: true,
    })
    initializePicker(runtime)
    const { DynWinRtValue, DynWinRtStruct } = runtime
    const { factory, picker, result, windowId, viewMode, startLocation } = registerPicker()
    const activation = own(DynWinRtValue.activationFactory('Microsoft.Windows.Storage.Pickers.FileOpenPicker'))
    const factoryObject = own(activation.cast(factory.iid()))
    const owner = DynWinRtStruct.create(windowId)
    // A zero WindowId gives this standalone console example an unowned dialog.
    owner.setU64(0, 0n)
    const created = own(factory.methodByName('CreateInstance').invoke(factoryObject, [own(owner.toValue())]))
    const pickerObject = own(created.cast(picker.iid()))

    own(picker.methodByName('put_ViewMode').invoke(pickerObject, [own(DynWinRtValue.enumValue(viewMode, 1))]))
    assert.equal(own(picker.methodByName('get_ViewMode').invoke(pickerObject, [])).toNumber(), 1)
    own(picker.methodByName('put_SuggestedStartLocation').invoke(pickerObject, [own(DynWinRtValue.enumValue(startLocation, 9))]))
    assert.equal(own(picker.methodByName('get_SuggestedStartLocation').invoke(pickerObject, [])).toNumber(), 9)
    own(picker.methodByName('put_CommitButtonText').invoke(pickerObject, [own(DynWinRtValue.hstring('Select file'))]))
    assert.equal(own(picker.methodByName('get_CommitButtonText').invoke(pickerObject, [])).toString(), 'Select file')

    if (values['no-ui']) return 'Picker initialized; properties round-tripped. No dialog was shown.'

    const operation = own(picker.methodByName('PickSingleFileAsync').invoke(pickerObject, []))
    const selected = own(await operation.toPromise())
    const outcome = selected.isNull()
        ? pickerOutcome(null)
        : pickerOutcome(own(result.methodByName('get_Path').invoke(own(selected.cast(result.iid())), [])).toString())
    resultExitCode = outcome.exitCode
    return outcome.message
})
if (process.exitCode === 0) process.exitCode = resultExitCode
