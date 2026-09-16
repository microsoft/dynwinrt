// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { DynWinRtMethodSig as Sig, DynWinRtType as Type, WinGuid } from '@microsoft/dynwinrt'

// Complete metadata-ordered tables, including unused methods; no placeholder slots.
// check:contracts verifies the OS tables; check:picker-contracts also verifies the WinAppSDK tables.

export function registerUri() {
    const uriIid = WinGuid.parse('9e365e57-48b2-4160-956f-c7385120bbfc')
    const uriValue = Type.runtimeClass('Windows.Foundation.Uri', Type.interface(uriIid))
    const factory = Type.registerInterface('IUriRuntimeClassFactory', WinGuid.parse('44a9796f-723e-4fdf-a218-033e75b0c084'))
        .addMethod('CreateUri', new Sig().addIn(Type.hstring()).addOut(uriValue))
        .addMethod('CreateWithRelativeUri', new Sig().addIn(Type.hstring()).addIn(Type.hstring()).addOut(uriValue))
    const uri = Type.registerInterface('IUriRuntimeClass', uriIid)
    for (const property of ['AbsoluteUri', 'DisplayUri', 'Domain', 'Extension', 'Fragment', 'Host', 'Password', 'Path', 'Query']) {
        uri.addMethod(`get_${property}`, new Sig().addOut(Type.hstring()))
    }
    uri.addMethod('get_QueryParsed', new Sig().addOut(Type.runtimeClass(
        'Windows.Foundation.WwwFormUrlDecoder',
        Type.interface(WinGuid.parse('d45a0451-f225-4542-9296-0e1df5d254df')),
    )))
    for (const property of ['RawUri', 'SchemeName', 'UserName']) {
        uri.addMethod(`get_${property}`, new Sig().addOut(Type.hstring()))
    }
    uri.addMethod('get_Port', new Sig().addOut(Type.i32()))
        .addMethod('get_Suspicious', new Sig().addOut(Type.boolType()))
        .addMethod('Equals', new Sig().addIn(uriValue).addOut(Type.boolType()))
        .addMethod('CombineUri', new Sig().addIn(Type.hstring()).addOut(uriValue))
    return { factory, uri }
}

export function registerAsyncFile() {
    const fileIid = WinGuid.parse('fa3f6186-4214-428c-a64c-14c9ac7315ea')
    const fileInterface = Type.interface(fileIid)
    const fileValue = Type.runtimeClass('Windows.Storage.StorageFile', fileInterface)
    const uriValue = Type.runtimeClass('Windows.Foundation.Uri', Type.interface(WinGuid.parse('9e365e57-48b2-4160-956f-c7385120bbfc')))
    const folder = Type.interface(WinGuid.parse('72d1cb78-b3ef-4f75-a80b-6fd9dae2944b'))
    const dataRequested = Type.interface(WinGuid.parse('fef6a824-2fe1-4d07-a35b-b77c50b5f4cc'))
    const thumbnail = Type.interface(WinGuid.parse('33ee3134-1dd6-4e3a-8067-d1c162e8642b'))
    const accessMode = Type.enumType('Windows.Storage.FileAccessMode', ['Read', 'ReadWrite'], [0, 1])
    const collision = Type.enumType('Windows.Storage.NameCollisionOption', ['GenerateUniqueName', 'ReplaceExisting', 'FailIfExists'], [0, 1, 2])
    const streamIid = WinGuid.parse('905a0fe1-bc53-11df-8c49-001e4fc686da')
    const streamValue = Type.interface(streamIid)
    const fileAsync = Type.iAsyncOperation(fileValue)

    const factory = Type.registerInterface('IStorageFileStatics', WinGuid.parse('5984c710-daf2-43c8-8bb4-a4d3eacfd03f'))
        .addMethod('GetFileFromPathAsync', new Sig().addIn(Type.hstring()).addOut(fileAsync))
        .addMethod('GetFileFromApplicationUriAsync', new Sig().addIn(uriValue).addOut(fileAsync))
        .addMethod('CreateStreamedFileAsync', new Sig().addIn(Type.hstring()).addIn(dataRequested).addIn(thumbnail).addOut(fileAsync))
        .addMethod('ReplaceWithStreamedFileAsync', new Sig().addIn(fileInterface).addIn(dataRequested).addIn(thumbnail).addOut(fileAsync))
        .addMethod('CreateStreamedFileFromUriAsync', new Sig().addIn(Type.hstring()).addIn(uriValue).addIn(thumbnail).addOut(fileAsync))
        .addMethod('ReplaceWithStreamedFileFromUriAsync', new Sig().addIn(fileInterface).addIn(uriValue).addIn(thumbnail).addOut(fileAsync))

    const file = Type.registerInterface('IStorageFile', fileIid)
        .addMethod('get_FileType', new Sig().addOut(Type.hstring()))
        .addMethod('get_ContentType', new Sig().addOut(Type.hstring()))
        .addMethod('OpenAsync', new Sig().addIn(accessMode).addOut(Type.iAsyncOperation(streamValue)))
        .addMethod('OpenTransactedWriteAsync', new Sig().addOut(Type.iAsyncOperation(Type.runtimeClass(
            'Windows.Storage.StorageStreamTransaction',
            Type.interface(WinGuid.parse('f67cf363-a53d-4d94-ae2c-67232d93acdd')),
        ))))
        .addMethod('CopyOverloadDefaultNameAndOptions', new Sig().addIn(folder).addOut(fileAsync))
        .addMethod('CopyOverloadDefaultOptions', new Sig().addIn(folder).addIn(Type.hstring()).addOut(fileAsync))
        .addMethod('CopyOverload', new Sig().addIn(folder).addIn(Type.hstring()).addIn(collision).addOut(fileAsync))
        .addMethod('CopyAndReplaceAsync', new Sig().addIn(fileInterface).addOut(Type.iAsyncAction()))
        .addMethod('MoveOverloadDefaultNameAndOptions', new Sig().addIn(folder).addOut(Type.iAsyncAction()))
        .addMethod('MoveOverloadDefaultOptions', new Sig().addIn(folder).addIn(Type.hstring()).addOut(Type.iAsyncAction()))
        .addMethod('MoveOverload', new Sig().addIn(folder).addIn(Type.hstring()).addIn(collision).addOut(Type.iAsyncAction()))
        .addMethod('MoveAndReplaceAsync', new Sig().addIn(fileInterface).addOut(Type.iAsyncAction()))

    const stream = Type.registerInterface('IRandomAccessStream', streamIid)
        .addMethod('get_Size', new Sig().addOut(Type.u64()))
        .addMethod('put_Size', new Sig().addIn(Type.u64()))
        .addMethod('GetInputStreamAt', new Sig().addIn(Type.u64()).addOut(Type.interface(WinGuid.parse('905a0fe2-bc53-11df-8c49-001e4fc686da'))))
        .addMethod('GetOutputStreamAt', new Sig().addIn(Type.u64()).addOut(Type.interface(WinGuid.parse('905a0fe6-bc53-11df-8c49-001e4fc686da'))))
        .addMethod('get_Position', new Sig().addOut(Type.u64()))
        .addMethod('Seek', new Sig().addIn(Type.u64()))
        .addMethod('CloneStream', new Sig().addOut(streamValue))
        .addMethod('get_CanRead', new Sig().addOut(Type.boolType()))
        .addMethod('get_CanWrite', new Sig().addOut(Type.boolType()))
    const closable = Type.registerInterface('IClosable', WinGuid.parse('30d5a829-7fa4-4026-83bb-d75bae4ea99e'))
        .addMethod('Close', new Sig())
    return { factory, file, stream, closable, accessMode }
}

export function registerGeopoint() {
    const position = Type.structType('Windows.Devices.Geolocation.BasicGeoposition', [Type.f64(), Type.f64(), Type.f64()])
    const pointIid = WinGuid.parse('6bfa00eb-e56e-49bb-9caf-cbaa78a8bcef')
    const pointValue = Type.runtimeClass('Windows.Devices.Geolocation.Geopoint', Type.interface(pointIid))
    const altitudeReference = Type.enumType(
        'Windows.Devices.Geolocation.AltitudeReferenceSystem',
        ['Unspecified', 'Terrain', 'Ellipsoid', 'Geoid', 'Surface'], [0, 1, 2, 3, 4],
    )
    const factory = Type.registerInterface('IGeopointFactory', WinGuid.parse('db6b8d33-76bd-4e30-8af7-a844dc37b7a0'))
        .addMethod('Create', new Sig().addIn(position).addOut(pointValue))
        .addMethod('CreateWithAltitudeReferenceSystem', new Sig().addIn(position).addIn(altitudeReference).addOut(pointValue))
        .addMethod('CreateWithAltitudeReferenceSystemAndSpatialReferenceId', new Sig().addIn(position).addIn(altitudeReference).addIn(Type.u32()).addOut(pointValue))
    const point = Type.registerInterface('IGeopoint', pointIid)
        .addMethod('get_Position', new Sig().addOut(position))
    return { factory, point, position }
}

export function registerPropertyValue() {
    const scalars: [string, Type][] = [
        ['UInt8', Type.u8()], ['Int16', Type.i16()], ['UInt16', Type.u16()],
        ['Int32', Type.i32()], ['UInt32', Type.u32()], ['Int64', Type.i64()], ['UInt64', Type.u64()],
        ['Single', Type.f32()], ['Double', Type.f64()], ['Char16', Type.u16()],
        ['Boolean', Type.boolType()], ['String', Type.hstring()], ['Inspectable', Type.object()],
        ['Guid', Type.guidType()],
        ['DateTime', Type.structType('Windows.Foundation.DateTime', [Type.i64()])],
        ['TimeSpan', Type.structType('Windows.Foundation.TimeSpan', [Type.i64()])],
        ['Point', Type.structType('Windows.Foundation.Point', [Type.f32(), Type.f32()])],
        ['Size', Type.structType('Windows.Foundation.Size', [Type.f32(), Type.f32()])],
        ['Rect', Type.structType('Windows.Foundation.Rect', [Type.f32(), Type.f32(), Type.f32(), Type.f32()])],
    ]
    const propertyType = Type.enumType('Windows.Foundation.PropertyType', [
        'Empty', 'UInt8', 'Int16', 'UInt16', 'Int32', 'UInt32', 'Int64', 'UInt64', 'Single', 'Double',
        'Char16', 'Boolean', 'String', 'Inspectable', 'DateTime', 'TimeSpan', 'Guid', 'Point', 'Size', 'Rect', 'OtherType',
        'UInt8Array', 'Int16Array', 'UInt16Array', 'Int32Array', 'UInt32Array', 'Int64Array', 'UInt64Array',
        'SingleArray', 'DoubleArray', 'Char16Array', 'BooleanArray', 'StringArray', 'InspectableArray',
        'DateTimeArray', 'TimeSpanArray', 'GuidArray', 'PointArray', 'SizeArray', 'RectArray', 'OtherTypeArray',
    ], [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        1025, 1026, 1027, 1028, 1029, 1030, 1031, 1032, 1033, 1034, 1035, 1036, 1037,
        1038, 1039, 1040, 1041, 1042, 1043, 1044,
    ])
    const factory = Type.registerInterface('IPropertyValueStatics', WinGuid.parse('629bdbc8-d932-4ff4-96b9-8d96c5c1e858'))
        .addMethod('CreateEmpty', new Sig().addOut(Type.object()))
    for (const [name, type] of scalars) {
        factory.addMethod(`Create${name}`, new Sig().addIn(type).addOut(Type.object()))
    }
    for (const [name, type] of scalars) {
        factory.addMethod(`Create${name}Array`, new Sig().addIn(Type.arrayType(type)).addOut(Type.object()))
    }

    const property = Type.registerInterface('IPropertyValue', WinGuid.parse('4bd682dd-7554-40e9-9a9b-82654ede7e62'))
        .addMethod('get_Type', new Sig().addOut(propertyType))
        .addMethod('get_IsNumericScalar', new Sig().addOut(Type.boolType()))
    // IPropertyValue has no scalar GetInspectable, but does have GetInspectableArray.
    for (const [name, type] of scalars) {
        if (name !== 'Inspectable') property.addMethod(`Get${name}`, new Sig().addOut(type))
    }
    for (const [name, type] of scalars) {
        property.addMethod(`Get${name}Array`, new Sig().addOut(Type.arrayType(type)))
    }
    return { factory, property }
}

export function registerPicker() {
    const windowId = Type.structType('Microsoft.UI.WindowId', [Type.u64()])
    const pickerIid = WinGuid.parse('9d00f175-c783-51bd-8c93-fb63695d3abc')
    const resultIid = WinGuid.parse('e6f2e3d6-7bb0-5d81-9e7d-6fd35a1f25ab')
    const pickerValue = Type.runtimeClass('Microsoft.Windows.Storage.Pickers.FileOpenPicker', Type.interface(pickerIid))
    const resultValue = Type.runtimeClass('Microsoft.Windows.Storage.Pickers.PickFileResult', Type.interface(resultIid))
    const viewMode = Type.enumType('Microsoft.Windows.Storage.Pickers.PickerViewMode', ['List', 'Thumbnail'], [0, 1])
    const startLocation = Type.enumType(
        'Microsoft.Windows.Storage.Pickers.PickerLocationId',
        ['DocumentsLibrary', 'ComputerFolder', 'Desktop', 'Downloads', 'MusicLibrary', 'PicturesLibrary', 'VideosLibrary', 'Objects3D', 'Unspecified'],
        [0, 1, 2, 3, 5, 6, 7, 8, 9],
    )
    const factory = Type.registerInterface('IFileOpenPickerFactory', WinGuid.parse('315e86d7-d7a2-5d81-b379-7af78207b1af'))
        .addMethod('CreateInstance', new Sig().addIn(windowId).addOut(pickerValue))
    const picker = Type.registerInterface('IFileOpenPicker', pickerIid)
        .addMethod('get_ViewMode', new Sig().addOut(viewMode))
        .addMethod('put_ViewMode', new Sig().addIn(viewMode))
        .addMethod('get_SuggestedStartLocation', new Sig().addOut(startLocation))
        .addMethod('put_SuggestedStartLocation', new Sig().addIn(startLocation))
        .addMethod('get_CommitButtonText', new Sig().addOut(Type.hstring()))
        .addMethod('put_CommitButtonText', new Sig().addIn(Type.hstring()))
        .addMethod('get_FileTypeFilter', new Sig().addOut(Type.parameterized(
            WinGuid.parse('913337e9-11a1-4345-a3a2-4e7f956e222d'), [Type.hstring()],
        )))
        .addMethod('PickSingleFileAsync', new Sig().addOut(Type.iAsyncOperation(resultValue)))
        .addMethod('PickMultipleFilesAsync', new Sig().addOut(Type.iAsyncOperation(Type.parameterized(
            WinGuid.parse('bbe1fa4c-b0e3-4583-baef-1f1b2e483e56'), [resultValue],
        ))))
    const result = Type.registerInterface('IPickFileResult', resultIid)
        .addMethod('get_Path', new Sig().addOut(Type.hstring()))
    return { factory, picker, result, windowId, viewMode, startLocation }
}
