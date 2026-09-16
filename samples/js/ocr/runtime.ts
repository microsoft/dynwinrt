// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { fileURLToPath } from 'node:url'
import { hasPackageIdentity, initWinappsdk, roInitialize } from '@microsoft/dynwinrt'
import { createProjectedLifetimeScope, releaseProjected } from '#winapp/bindings/lifetime'
import { FileOpenPicker } from '#winapp/bindings/microsoft/windows/storage/pickers/FileOpenPicker'
import { StorageFile } from '#winapp/bindings/windows/storage/StorageFile'
import { FileAccessMode } from '#winapp/bindings/windows/storage/FileAccessMode'
import { BitmapDecoder } from '#winapp/bindings/windows/graphics/imaging/BitmapDecoder'
import { BitmapPixelFormat } from '#winapp/bindings/windows/graphics/imaging/BitmapPixelFormat'
import { BitmapAlphaMode } from '#winapp/bindings/windows/graphics/imaging/BitmapAlphaMode'
import { ImageBuffer } from '#winapp/bindings/microsoft/graphics/imaging/ImageBuffer'
import { TextRecognizer } from '#winapp/bindings/microsoft/windows/ai/imaging/TextRecognizer'
import { imagePath } from './cli.ts'
import type { OcrRunner } from './cli.ts'
import { ensureModelReady, recognizedLines } from './model.ts'
import { bootstrapPath, step, withCleanup } from './support.ts'

export function initialize(): void {
    roInitialize(1)
    // The full packaged host gets its framework from PackageDependency, not MddBootstrapInitialize2.
    if (!hasPackageIdentity()) {
        process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH = bootstrapPath(
            fileURLToPath(new URL('.', import.meta.url)),
            process.arch,
            process.env.WINAPPSDK_BOOTSTRAP_DLL_PATH,
        )
        initWinappsdk(1, 8)
    }
}

export async function pickImage(signal?: AbortSignal): Promise<string | null> {
    return withCleanup(async (defer) => {
        const scope = createProjectedLifetimeScope()
        defer('picker projections', () => scope.dispose())
        // A zero WindowId keeps this standalone console sample's dialog unowned.
        const picker = new FileOpenPicker({ value: 0n })
        const filter = picker.fileTypeFilter
        if (filter === null) throw new Error('FileOpenPicker did not supply a file-type filter.')
        defer('picker filter', () => releaseProjected(filter))
        for (const extension of ['.png', '.jpg', '.jpeg', '.bmp', '.tif', '.tiff', '.gif']) {
            filter.append(extension)
        }
        const result = await picker.pickSingleFileAsync(signal)
        return result === null ? null : imagePath(result.path)
    })
}

export async function withImageBuffer<T>(
    filename: string,
    use: (image: ImageBuffer) => Promise<T>,
    signal?: AbortSignal,
): Promise<T> {
    return withCleanup(async (defer) => {
        const scope = createProjectedLifetimeScope()
        defer('image projections', () => scope.dispose())
        const file = await step('StorageFile.GetFileFromPathAsync failed. Check the image path and file access', () =>
            StorageFile.getFileFromPathAsync(filename, signal))
        const stream = await step('StorageFile.OpenAsync failed', () => file.openAsync(FileAccessMode.Read, signal))
        if (stream === null) throw new Error('StorageFile.OpenAsync returned no stream.')
        // Interface projections are not tracked by the generated runtime-class scope.
        defer('file stream', () => releaseProjected(stream))
        const decoder = await step('BitmapDecoder.CreateAsync failed. Use a supported, valid image', () =>
            BitmapDecoder.createAsync(stream, signal))
        const bitmap = await step('BitmapDecoder.GetSoftwareBitmapAsync failed', () =>
            decoder.getSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, signal))
        if (bitmap === null) throw new Error('BitmapDecoder returned no SoftwareBitmap.')
        defer('SoftwareBitmap', () => bitmap.close())
        const image = await step('ImageBuffer.CreateForSoftwareBitmap failed', () => ImageBuffer.createForSoftwareBitmap(bitmap))
        if (image === null) throw new Error('ImageBuffer.CreateForSoftwareBitmap returned no image.')
        defer('ImageBuffer', () => image.close())
        return use(image)
    })
}

export const recognize: OcrRunner = async (options, report, signal) => {
    await step('Windows App SDK 1.8 initialization failed. Check the framework dependency or unpackaged bootstrap setup (see README)', initialize)
    if (!hasPackageIdentity()) {
        throw new Error(
            'Windows AI OCR requires package identity and the systemAIModels capability, not just bootstrap. ' +
            'Launch with npm start from samples\\js\\ocr; do not assign identity to the installed node.exe.',
        )
    }
    const filename = options.image ?? await step('FileOpenPicker failed. Use --image to bypass the dialog', () => pickImage(signal))
    if (filename === null) return { kind: 'cancelled' }
    const lines = await withImageBuffer(filename, (image) =>
        withCleanup(async (defer) => {
            report(`Decoded image: ${image.pixelWidth} x ${image.pixelHeight}; ImageBuffer created.`)
            await ensureModelReady(TextRecognizer, options.ensureReady, report, signal)
            const recognizer = await step('TextRecognizer.CreateAsync failed. Check Windows AI prerequisites (see README)', () =>
                TextRecognizer.createAsync(signal))
            if (recognizer === null) throw new Error('TextRecognizer.CreateAsync returned no recognizer.')
            defer('TextRecognizer', () => recognizer.close())
            const result = await step('TextRecognizer.RecognizeTextFromImageAsync failed', () =>
                recognizer.recognizeTextFromImageAsync(image, signal))
            return recognizedLines(result)
        }), signal)
    return { kind: 'recognized', lines }
}
