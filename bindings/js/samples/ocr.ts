// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * OCR Demo: Pure dynamic WinRT calls using registerInterface + invoke.
 * Requires WinAppSDK with AI features and package identity.
 *
 * Flow: Pick file → load bitmap → create ImageBuffer → recognize text → print lines
 */
import {
    DynWinRtValue,
    DynWinRtType,
    DynWinRtMethodSig,
    WinGuid,
    hasPackageIdentity,
    initWinappsdk,
} from '../dist/winrt.js'

// ======================================================================
// IIDs
// ======================================================================

const IID_ITextRecognizerStatics   = WinGuid.parse('3788c2fd-e496-53ab-85a7-e54a135824e9')
const IID_ITextRecognizer          = WinGuid.parse('be7bf6c0-30f6-570d-bd92-3ffe5665d933')
const IID_IImageBufferStatics      = WinGuid.parse('35b17bd3-f346-529f-8c0f-3bf96c56eb13')
const IID_IRecognizedText          = WinGuid.parse('ae4766d3-2924-57a6-b3d3-b866f59b9972')
const IID_ISoftwareBitmap          = WinGuid.parse('689e0708-7eef-483f-963f-da938818e073')

// IStorageFileStatics: GetFileFromPathAsync
const IID_IStorageFileStatics      = WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f')
// IStorageFile (inherits IStorageItem): OpenAsync is on IStorageFile at vtable 6
const IID_IStorageFile             = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
// IRandomAccessStream
const IID_IRandomAccessStream      = WinGuid.parse('905a0fe1-bc53-11df-8c49-001e4fc686da')
// IBitmapDecoderStatics: CreateAsync
const IID_IBitmapDecoderStatics    = WinGuid.parse('438ccb26-bcef-4e95-bad6-23a822e58d01')
// IBitmapDecoder (base: IBitmapFrame → GetSoftwareBitmapAsync at vtable 6+8=14 on IBitmapFrameWithSoftwareBitmap)
const IID_IBitmapFrameWithSoftwareBitmap = WinGuid.parse('fe287c9a-420c-4963-87ad-691436e08383')

// IFileOpenPickerFactory (WinAppSDK)
const IID_IFileOpenPickerFactory   = WinGuid.parse('315E86D7-D7A2-5D81-B379-7AF78207B1AF')
// IFileOpenPicker
const IID_IFileOpenPicker          = WinGuid.parse('2C3D04E9-3B09-5260-88BC-01549E8C03A8')
// IPickFileResult
const IID_IPickFileResult          = WinGuid.parse('E6F2E3D6-7BB0-5D81-9E7D-6FD35A1F25AB')

// ======================================================================
// Register interfaces
// ======================================================================

// FileOpenPicker (WinAppSDK)
const iPickerFactory = DynWinRtType.registerInterface("IFileOpenPickerFactory", IID_IFileOpenPickerFactory)
    .addMethod("CreateWithMode", new DynWinRtMethodSig().addIn(DynWinRtType.i64()).addOut(DynWinRtType.object()))

const iPicker = DynWinRtType.registerInterface("IFileOpenPicker", IID_IFileOpenPicker)
    .addMethod("put_ViewMode",              new DynWinRtMethodSig().addIn(DynWinRtType.i32()))
    .addMethod("get_ViewMode",              new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
    .addMethod("put_SuggestedStartLocation",new DynWinRtMethodSig().addIn(DynWinRtType.object()))
    .addMethod("get_SuggestedStartLocation",new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("put_CommitButtonText",      new DynWinRtMethodSig().addIn(DynWinRtType.hstring()))
    .addMethod("get_CommitButtonText",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_FileTypeFilter",        new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("PickSingleFileAsync",       new DynWinRtMethodSig().addOut(
        DynWinRtType.iAsyncOperation(
            DynWinRtType.runtimeClass("Microsoft.Windows.Storage.Pickers.PickFileResult",
                DynWinRtType.interface(IID_IPickFileResult)))))

const iPickResult = DynWinRtType.registerInterface("IPickFileResult", IID_IPickFileResult)
    .addMethod("get_File", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))

// StorageFile
const iStorageFileStatics = DynWinRtType.registerInterface("IStorageFileStatics", IID_IStorageFileStatics)
    .addMethod("GetFileFromPathAsync", new DynWinRtMethodSig()
        .addIn(DynWinRtType.hstring())
        .addOut(DynWinRtType.iAsyncOperation(
            DynWinRtType.runtimeClass(
                "Windows.Storage.StorageFile",
                DynWinRtType.interface(IID_IStorageFile)))))

// IStorageFile: OpenAsync at vtable 6
const iStorageFile = DynWinRtType.registerInterface("IStorageFile", IID_IStorageFile)
    .addMethod("get_FileType", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("get_ContentType", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))
    .addMethod("OpenAsync", new DynWinRtMethodSig()
        .addIn(DynWinRtType.i32())  // FileAccessMode
        .addOut(DynWinRtType.object()))  // IAsyncOperation<IRandomAccessStream>

// BitmapDecoder
const iBitmapDecoderStatics = DynWinRtType.registerInterface("IBitmapDecoderStatics", IID_IBitmapDecoderStatics)
    .addMethod("get_BmpDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_JpegDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_PngDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_TiffDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_GifDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_JpegXRDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("get_IcoDecoderId", new DynWinRtMethodSig().addOut(DynWinRtType.guidType()))
    .addMethod("CreateAsync", new DynWinRtMethodSig()
        .addIn(DynWinRtType.object())  // IRandomAccessStream
        .addOut(DynWinRtType.object()))  // IAsyncOperation<BitmapDecoder>

// IBitmapFrameWithSoftwareBitmap: GetSoftwareBitmapAsync at vtable 6
const iBitmapFrameWithSoftwareBitmap = DynWinRtType.registerInterface(
    "IBitmapFrameWithSoftwareBitmap", IID_IBitmapFrameWithSoftwareBitmap)
    .addMethod("GetSoftwareBitmapAsync", new DynWinRtMethodSig().addOut(DynWinRtType.object()))

// TextRecognizer
const iTextRecognizerStatics = DynWinRtType.registerInterface("ITextRecognizerStatics", IID_ITextRecognizerStatics)
    .addMethod("GetReadyState", new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
    .addMethod("EnsureReadyAsync", new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .addMethod("CreateAsync", new DynWinRtMethodSig().addOut(
        DynWinRtType.iAsyncOperation(
            DynWinRtType.runtimeClass(
                'Microsoft.Windows.AI.Imaging.TextRecognizer',
                DynWinRtType.interface(IID_ITextRecognizer)))))

const iImageBufferStatics = DynWinRtType.registerInterface("IImageBufferStatics", IID_IImageBufferStatics)
    .addMethod("CreateForSoftwareBitmap", new DynWinRtMethodSig()
        .addIn(DynWinRtType.object()).addOut(DynWinRtType.object()))

const iTextRecognizer = DynWinRtType.registerInterface("ITextRecognizer", IID_ITextRecognizer)
    .addMethod("RecognizeTextFromImageAsync", new DynWinRtMethodSig()
        .addIn(DynWinRtType.object())
        .addOut(DynWinRtType.iAsyncOperation(
            DynWinRtType.runtimeClass(
                'Microsoft.Windows.AI.Imaging.RecognizedText',
                DynWinRtType.interface(IID_IRecognizedText)))))

// IRecognizedText: Lines is a ReceiveArray of IRecognizedTextLine objects
// For simplicity, we use the ToString() on IStringable to get the full text
const IID_IStringable = WinGuid.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
const iStringable = DynWinRtType.registerInterface("IStringable", IID_IStringable)
    .addMethod("ToString", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))

// ======================================================================
// Main
// ======================================================================

async function main() {
    console.log(hasPackageIdentity() ? 'Has package identity' : 'No package identity')

    // --- Pick a file via WinAppSDK FileOpenPicker ---
    const pickerAf = DynWinRtValue.activationFactory('Microsoft.Windows.Storage.Pickers.FileOpenPicker')
    const pickerFactory = pickerAf.cast(IID_IFileOpenPickerFactory)
    const picker = iPickerFactory.methodByName("CreateWithMode").invoke(pickerFactory, [DynWinRtValue.i64(0)])
    const asyncPickOp = iPicker.methodByName("PickSingleFileAsync").invoke(picker, [])
    const pickResult = await asyncPickOp.toPromise()
    const filePath = iPickResult.methodByName("get_File").invoke(pickResult, [])
    console.log('Selected file:', filePath.toString())

    // --- Load bitmap from file path ---
    const storageFileAf = DynWinRtValue.activationFactory('Windows.Storage.StorageFile')
        .cast(IID_IStorageFileStatics)
    const storageFileOp = iStorageFileStatics.methodByName("GetFileFromPathAsync")
        .invoke(storageFileAf, [DynWinRtValue.hstring(filePath.toString())])
    const storageFile = await storageFileOp.toPromise()
    console.log('StorageFile loaded')

    // OpenAsync(FileAccessMode.Read = 0)
    const streamOp = iStorageFile.methodByName("OpenAsync")
        .invoke(storageFile.cast(IID_IStorageFile), [DynWinRtValue.i32(0)])
    const stream = await streamOp.toPromise()
    console.log('Stream opened')

    // BitmapDecoder.CreateAsync(stream)
    const decoderAf = DynWinRtValue.activationFactory('Windows.Graphics.Imaging.BitmapDecoder')
        .cast(IID_IBitmapDecoderStatics)
    const decoderOp = iBitmapDecoderStatics.methodByName("CreateAsync")
        .invoke(decoderAf, [stream])
    const decoder = await decoderOp.toPromise()
    console.log('BitmapDecoder created')

    // GetSoftwareBitmapAsync
    const bitmapOp = iBitmapFrameWithSoftwareBitmap.methodByName("GetSoftwareBitmapAsync")
        .invoke(decoder.cast(IID_IBitmapFrameWithSoftwareBitmap), [])
    const bitmap = await bitmapOp.toPromise()
    console.log('SoftwareBitmap obtained')

    // --- Create TextRecognizer ---
    const trFactory = DynWinRtValue.activationFactory('Microsoft.Windows.AI.Imaging.TextRecognizer')
        .cast(IID_ITextRecognizerStatics)

    const readyState = iTextRecognizerStatics.methodByName("GetReadyState").invoke(trFactory, [])
    console.log('TextRecognizer ready state:', readyState.toNumber())

    const recognizer = await iTextRecognizerStatics.methodByName("CreateAsync")
        .invoke(trFactory, []).toPromise()
    console.log('TextRecognizer created')

    // --- Create ImageBuffer from SoftwareBitmap ---
    const ibFactory = DynWinRtValue.activationFactory('Microsoft.Graphics.Imaging.ImageBuffer')
        .cast(IID_IImageBufferStatics)
    const imageBuffer = iImageBufferStatics.methodByName("CreateForSoftwareBitmap")
        .invoke(ibFactory, [bitmap.cast(IID_ISoftwareBitmap)])
    console.log('ImageBuffer created')

    // --- Recognize text ---
    const resultOp = iTextRecognizer.methodByName("RecognizeTextFromImageAsync")
        .invoke(recognizer.cast(IID_ITextRecognizer), [imageBuffer])
    const result = await resultOp.toPromise()

    // Print result via IStringable.ToString()
    const text = iStringable.methodByName("ToString").invoke(result.cast(IID_IStringable), [])
    console.log('\n=== Recognized Text ===')
    console.log(text.toString())
}

main().catch(console.error)
