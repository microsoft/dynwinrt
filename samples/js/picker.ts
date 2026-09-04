// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import {
    initWinappsdk,
    roInitialize,
    DynWinRtValue,
    DynWinRtType,
    DynWinRtMethodSig,
    WinGuid,
} from '../../bindings/js/dist/winrt.js'

// ======================================================================
// Register interfaces (once)
// ======================================================================

// IFileOpenPickerFactory: CreateWithMode(INT64 hwnd) -> IFileOpenPicker
const iPickerFactory = DynWinRtType.registerInterface(
    "IFileOpenPickerFactory",
    WinGuid.parse("315E86D7-D7A2-5D81-B379-7AF78207B1AF"),
).addMethod("CreateWithMode", new DynWinRtMethodSig().addIn(DynWinRtType.i64()).addOut(DynWinRtType.object()))

// IFileOpenPicker: vtable 6..13
const iPickerIid = WinGuid.parse("2C3D04E9-3B09-5260-88BC-01549E8C03A8")
const iPicker = DynWinRtType.registerInterface("IFileOpenPicker", iPickerIid)
    .addMethod("put_ViewMode",              new DynWinRtMethodSig().addIn(DynWinRtType.i32()))                              // 6
    .addMethod("get_ViewMode",              new DynWinRtMethodSig().addOut(DynWinRtType.i32()))                              // 7
    .addMethod("put_SuggestedStartLocation",new DynWinRtMethodSig().addIn(DynWinRtType.object()))                           // 8
    .addMethod("get_SuggestedStartLocation",new DynWinRtMethodSig().addOut(DynWinRtType.object()))                          // 9
    .addMethod("put_CommitButtonText",      new DynWinRtMethodSig().addIn(DynWinRtType.hstring()))                          // 10
    .addMethod("get_CommitButtonText",      new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))                         // 11
    .addMethod("get_FileTypeFilter",        new DynWinRtMethodSig().addOut(DynWinRtType.object()))                          // 12
    .addMethod("PickSingleFileAsync",       new DynWinRtMethodSig().addOut(
        DynWinRtType.iAsyncOperation(
            DynWinRtType.runtimeClass(
                "Microsoft.Windows.Storage.Pickers.PickFileResult",
                DynWinRtType.interface(
                    WinGuid.parse("E6F2E3D6-7BB0-5D81-9E7D-6FD35A1F25AB"),
                ),
            )
        ))) // 13

// IPickFileResult
const iPickResultIid = WinGuid.parse("E6F2E3D6-7BB0-5D81-9E7D-6FD35A1F25AB")
const iPickResult = DynWinRtType.registerInterface("IPickFileResult", iPickResultIid)
    .addMethod("get_File", new DynWinRtMethodSig().addOut(DynWinRtType.hstring()))  // 6

// ======================================================================
// Use
// ======================================================================

async function main() {
    initWinappsdk(1, 8)
    roInitialize(1)

    const factory = DynWinRtValue.activationFactory('Microsoft.Windows.Storage.Pickers.FileOpenPicker')
    const pickerFactory = factory.cast(WinGuid.parse("315E86D7-D7A2-5D81-B379-7AF78207B1AF"))

    // by name
    const picker = iPickerFactory.methodByName("CreateWithMode").invoke(pickerFactory, [DynWinRtValue.i64(0)])

    // by vtable index
    const asyncOp = iPicker.method(13).invoke(picker, [])
    const pickedFile = await asyncOp.toPromise()

    // by name
    const path = iPickResult.methodByName("get_File").invoke(pickedFile, [])
    console.log("Selected Path", path.toString())
}
main();
