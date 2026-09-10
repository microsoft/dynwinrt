// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Compile, do not execute. This imports the real generated declarations and
// built runtime declarations, not a permissive hand-written runtime stub.
import {
    IBackgroundTask, IBackgroundTaskInstance, IClosable, IStringable,
    IMemoryBufferReference, IPropertyValue, IBuffer, IDataReader, IDataWriter,
    IBindableIterable, IBindableIterator, IBindableVectorView, INumberParser,
    releaseProjected,
} from '../e2e_generated/implementations/js/index.js';
import {
    DynWinRtArray, DynWinRtImplementationHandle, DynWinRtType, DynWinRtValue,
} from '../../../bindings/js/dist/winrt.js';

function unused(..._args: unknown[]): never {
    throw new Error('Typecheck fixture only');
}

let closes = 0;
IClosable.implement({ close: () => closes++ });
let length = 0;
IBuffer.implement({ getCapacity: () => 12, getLength: () => length, setLength: value => length = value });
const tokens = new Map<bigint, boolean>();
IMemoryBufferReference.implement({
    getCapacity: () => 12,
    addClosed: () => ({ value: 1n }),
    removeClosed: token => tokens.delete(token.value),
}, IClosable.implementation({ close: () => undefined }));

const exactHresult: DynWinRtValue = DynWinRtValue.hresult(-2147467263);
const hresults: DynWinRtArray = DynWinRtArray.fromHresultValues([0, -2147467263]);
const genericHresults: DynWinRtArray = DynWinRtArray.fromObjectValues(
    [exactHresult], DynWinRtType.hresult(),
);
const hresultCodes: number[] = hresults.toI32Vec();
void [genericHresults, hresultCodes];
// @ts-expect-error Exact HRESULT factories accept signed-i32 numbers, not strings.
DynWinRtValue.hresult('0x80004001');
// @ts-expect-error A batch HRESULT constructor does not accept heterogeneous arrays.
DynWinRtArray.fromHresultValues([0, '0x80004001']);

const instance = IBackgroundTaskInstance.implement({
    getInstanceId: () => '243aab7d-9411-49af-b66d-0123456789ab',
    getTask: unused,
    getProgress: () => 0,
    setProgress(value) { const progress: number = value; void progress; },
    getTriggerDetails: unused,
    addCanceled: unused,
    removeCanceled: unused,
    getSuspendedCount: unused,
    getDeferral: unused,
});
const owner: DynWinRtImplementationHandle<IBackgroundTask> = IBackgroundTask.implement({
    run(value) {
        const native: IBackgroundTaskInstance | null = value;
        if (native !== null) {
            const id: string = native.instanceId;
            native.progress = id.length;
        }
    },
}, IStringable.implementation({ toString: () => 'typed owner' }),
IClosable.implementation({ close() {} }));
const taskView: IBackgroundTask = IBackgroundTask.fromImplementation(owner);
owner.value.run(instance.value);
const common = IBackgroundTask.implement({ run: () => {} }, {
    interfaces: [[IStringable, { toString: () => 'text' }], [IClosable, { close() {} }]],
});
const primary: IBackgroundTask = common.value;
void primary;
// @ts-expect-error The management handle is not the interface.
common.run(instance.value);
// @ts-expect-error Each configured interface checks its own handler type.
IBackgroundTask.implement({ run() {} }, { interfaces: [[IStringable, { toString: () => 17 }]] });
// @ts-expect-error The stable primary view is read-only.
common.value = taskView;
taskView.run(IBackgroundTaskInstance.fromImplementation(instance));
const stringView: IStringable = IStringable.fromImplementation(owner);
const closeView: IClosable = IClosable.fromImplementation(owner);
const text: string = stringView.toString();
closeView.close();
releaseProjected(stringView);
releaseProjected(closeView);
const raw: DynWinRtValue = owner.toValue();
const closed: boolean = owner.isClosed;
const error: string | null = owner.takeError();
owner.release();
owner.dispose();
void [text, raw, closed, error];

// @ts-expect-error Handler names are ordinary JS camelCase, not Python names.
IStringable.implementation({ to_string: () => 'wrong name' });
// @ts-expect-error A string-returning native method cannot return a number.
IStringable.implementation({ toString: () => 17 });
// @ts-expect-error Async results are not WinRT logical outputs.
IStringable.implementation({ toString: async () => 'late' });
// @ts-expect-error All nine methods are required, not just Run's used getters.
IBackgroundTaskInstance.implementation({ getProgress: () => 0 });
// @ts-expect-error A typed view takes a root implementation owner.
IStringable.fromImplementation(raw);
// @ts-expect-error Consumers use the typed public view helper, not native constructors.
new IStringable(raw);

const parserOwner = INumberParser.implement({
    parseInt: () => -(1n << 63n),
    parseUInt: () => (1n << 64n) - 1n,
    parseDouble: () => null,
});
const parser = INumberParser.fromImplementation(parserOwner);
const parsedInt: bigint | null = parser.parseInt('signed');
const parsedUInt: bigint | null = parser.parseUInt('unsigned');
const parsedDouble: number | null = parser.parseDouble('invalid');
releaseProjected(parser);
void [parsedInt, parsedUInt, parsedDouble];

let sender: IMemoryBufferReference;
const memory = IMemoryBufferReference.implement({
    getCapacity: () => 4096,
    addClosed(handler) {
        // Received delegates have a typed native invocation surface.
        if (handler !== null) handler(sender, null);
        return { value: 1n };
    },
    removeClosed(token) {
        const nativeToken: bigint = token.value;
        void nativeToken;
    },
}, IClosable.implementation({ close() {} }));
sender = IMemoryBufferReference.fromImplementation(memory);
const unsubscribe: () => void = sender.onClosed((source, args) => {
    const sourceView: IMemoryBufferReference | null = source;
    const eventArgs: unknown = args;
    void [sourceView, eventArgs];
});
unsubscribe();
const capacity: number = sender.capacity;
void capacity;

declare const readerMethods: Parameters<typeof IDataReader.implementation>[0];
const reader = IDataReader.implement({
    ...readerMethods,
    readBytes(capacity) {
        const count: number = capacity;
        return new Uint8Array(count);
    },
}, IClosable.implementation({ close() {} }));
const filled: Uint8Array = IDataReader.fromImplementation(reader).readBytes(new Uint8Array(4));
void filled;
IDataReader.implementation({
    ...readerMethods,
    // @ts-expect-error FillArray receives logical U32 capacity, not caller storage.
    readBytes: (value: Uint8Array) => value,
});

declare const writerMethods: Parameters<typeof IDataWriter.implementation>[0];
const writerOwner = IDataWriter.implement({
    ...writerMethods,
    writeBytes(value) {
        const bytes: number[] = value;
        void bytes;
    },
    writeBuffer(value) {
        const buffer: IBuffer | null = value;
        void buffer;
    },
}, IClosable.implementation({ close() {} }));
const writerView: IDataWriter = IDataWriter.fromImplementation(writerOwner);
writerView.writeBytes(new Uint8Array([0, 255]));
writerView.writeBuffer(IBuffer.fromBuffer(new Uint8Array([1])));
declare const propertyMethods: Parameters<typeof IPropertyValue.implementation>[0];
const properties = IPropertyValue.implement({
    ...propertyMethods,
    getUInt8Array: () => new Uint8Array([0, 255]),
    getStringArray: () => ['owned string'],
    getInspectableArray: () => [raw, null],
    getPointArray: () => [{ x: 1.25, y: -3.5 }],
});
const propertyView: IPropertyValue = IPropertyValue.fromImplementation(properties);
const copied: Uint8Array = propertyView.getUInt8Array();
const strings: string[] = propertyView.getStringArray();
const points: { x: number; y: number }[] = propertyView.getPointArray();
const objects: (DynWinRtValue | null)[] = propertyView.getInspectableArray();
void [copied, strings, points, objects];
// @ts-expect-error Object array elements project native null references as null.
const nonNullableObjects: DynWinRtValue[] = propertyView.getInspectableArray();
IPropertyValue.implementation({
    ...propertyMethods,
    // @ts-expect-error ReceiveArray elements must match the metadata element type.
    getStringArray: () => ['valid', 42],
});

const iterator = IBindableIterator.implement({
    getCurrent: () => null, getHasCurrent: () => true, moveNext: () => false,
});
const iterable = IBindableIterable.implementation({
    first: () => IBindableIterator.fromImplementation(iterator),
});
const vector = IBindableVectorView.implement({
    getAt: () => null,
    getSize: () => 1,
    indexOf(value) {
        return { result: value === null, index: 0 };
    },
}, iterable);
const result: [number, boolean] = IBindableVectorView.fromImplementation(vector).indexOf(null);
void result;
IBindableVectorView.implementation({
    getAt: () => null, getSize: () => 1,
    // @ts-expect-error Inbound multiple outputs are named, not positional tuples.
    indexOf: () => [0, true],
});
IBindableVectorView.implementation({
    getAt: () => null, getSize: () => 1,
    // @ts-expect-error Explicit index out is required in addition to result.
    indexOf: () => ({ result: true }),
});
