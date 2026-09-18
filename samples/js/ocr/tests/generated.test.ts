// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const generated = (...parts: string[]) => readFileSync(join(root, '.winapp', 'bindings', ...parts), 'utf8')

test('the production sample contains no handwritten IIDs, method tables, or async erasure', () => {
    for (const name of ['main.ts', 'cli.ts', 'runtime.ts', 'model.ts']) {
        const source = readFileSync(join(root, name), 'utf8')
        assert.doesNotMatch(source, /DynWinRt(?:MethodSig|Type|Value)|WinGuid|registerInterface|activationFactory|methodByName/)
        assert.doesNotMatch(source, /\.method\(\d+\)|\.toPromise\(|IStringable|@microsoft\/dynwinrt\/(?:com|win32)/)
    }
    const runtime = readFileSync(join(root, 'runtime.ts'), 'utf8')
    assert.match(runtime, /roInitialize\(1\)/)
    assert.match(runtime, /if \(!hasPackageIdentity\(\)\) \{[\s\S]*?initWinappsdk\(1, 8\)/)
    assert.ok(runtime.indexOf('roInitialize(1)') < runtime.indexOf('initWinappsdk(1, 8)'))
    assert.match(runtime, /new FileOpenPicker\(\{ value: 0n \}\)/)
    assert.match(runtime, /result\.path/)
    assert.match(runtime, /BitmapPixelFormat\.Bgra8, BitmapAlphaMode\.Premultiplied/)
    assert.doesNotMatch(runtime, /roUninitialize|shutdownWinappsdk/)
})

test('StorageFile.OpenAsync has the closed IAsyncOperation<IRandomAccessStream> contract', () => {
    const source = generated('windows', 'storage', 'StorageFile.js')
    const signature = source.split('\n').find((line) => line.includes('.addMethod("OpenAsync",'))
    assert.ok(signature)
    assert.match(signature, /addOut\(DynWinRtType\.iAsyncOperation\(DynWinRtType\.interface\(WinGuid\.parse\('905a0fe1-bc53-11df-8c49-001e4fc686da'/)
    assert.match(source, /_IStorageFile\.method\(8\)\.invoke/)
    assert.match(generated('windows', 'storage', 'StorageFile.d.ts'), /openAsync\(accessMode: FileAccessMode, signal\?: AbortSignal\): Promise<IRandomAccessStream>/)
})

test('decoder and ImageBuffer dispatch match the pinned complete metadata tables', () => {
    const decoder = generated('windows', 'graphics', 'imaging', 'BitmapDecoder.js')
    assert.match(decoder, /\.addMethod\("GetDecoderInformationEnumerator"/)
    assert.match(decoder, /_IBitmapDecoderStatics\.method\(14\)\.invoke/)
    for (const name of ['CreateAsync', 'GetSoftwareBitmapAsync', 'GetSoftwareBitmapConvertedAsync']) {
        const signature = decoder.split('\n').find((line) => line.includes(`.addMethod("${name}",`))
        assert.ok(signature)
        assert.match(signature, /addOut\(DynWinRtType\.iAsyncOperation\(DynWinRtType\.runtimeClass/)
    }
    const image = generated('microsoft', 'graphics', 'imaging', 'ImageBuffer.js')
    assert.match(image, /\.addMethod\("CreateForBuffer"/)
    assert.match(image, /_IImageBufferStatics\.method\(7\)\.invoke/)
})

test('the picker uses a metadata-defined WindowId and typed nullable-result conversion', () => {
    const picker = generated('microsoft', 'windows', 'storage', 'pickers', 'FileOpenPicker.js')
    assert.match(picker, /structType\('Microsoft\.UI\.WindowId', \[DynWinRtType\.u64\(\)\]\)/)
    assert.match(picker, /pickSingleFileAsync\(signal\)/)
    assert.match(picker, /v\.isNull\(\) \? null : .*PickFileResult/)
    assert.match(generated('microsoft', 'windows', 'storage', 'pickers', 'PickFileResult.d.ts'), /get path\(\): string/)
})

test('AI async results and recognized lines remain typed rather than Object/IStringable', () => {
    const text = generated('microsoft', 'windows', 'ai', 'imaging', 'TextRecognizer.d.ts')
    assert.match(text, /ensureReadyAsync\(signal\?: AbortSignal\): WinRTAsyncWithProgress<AIFeatureReadyResult, number>/)
    assert.match(text, /createAsync\(signal\?: AbortSignal\): Promise<TextRecognizer>/)
    assert.match(text, /recognizeTextFromImageAsync\(imageBuffer: ImageBuffer, signal\?: AbortSignal\): Promise<RecognizedText>/)
    const recognized = generated('microsoft', 'windows', 'ai', 'imaging', 'RecognizedText.js')
    assert.match(recognized, /addOut\(DynWinRtType\.arrayType\(DynWinRtType\.runtimeClass\('Microsoft\.Windows\.AI\.Imaging\.RecognizedLine'/)
    assert.doesNotMatch(recognized, /IStringable/)
})

test('the pure model helpers only load generated enum modules, never the addon', () => {
    for (const name of ['AIFeatureReadyState', 'AIFeatureReadyResultState']) {
        assert.doesNotMatch(generated('microsoft', 'windows', 'ai', `${name}.js`), /require\(|from ['"]@microsoft\/dynwinrt/)
    }
})
