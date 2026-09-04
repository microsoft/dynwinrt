// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * Test: IAsyncOperationWithProgress progress handler
 * Uses InMemoryRandomAccessStream.WriteAsync → IAsyncOperationWithProgress<u32, u32>
 * No WinAppSDK needed.
 */
import {
  DynWinRtValue,
  DynWinRtType,
  DynWinRtMethodSig,
  WinGuid,
  roInitialize,
} from '../../bindings/js/dist/winrt.js'

roInitialize(1)

// --- Register interfaces ---
const IID_IActivationFactory = WinGuid.parse('00000035-0000-0000-C000-000000000046')
const iActivationFactory = DynWinRtType.registerInterface("IActivationFactory", IID_IActivationFactory)
  .addMethod("ActivateInstance", new DynWinRtMethodSig().addOut(DynWinRtType.object()))

// IOutputStream: vtable 6 = WriteAsync, 7 = FlushAsync
const IID_IOutputStream = WinGuid.parse('905A0FE6-BC53-11DF-8C49-001E4FC686DA')
const iOutputStream = DynWinRtType.registerInterface("IOutputStream", IID_IOutputStream)
  .addMethod("WriteAsync", new DynWinRtMethodSig()
    .addIn(DynWinRtType.object()) // IBuffer
    .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.u32(), DynWinRtType.u32())))

// IRandomAccessStream inherits IOutputStream via IClosable chain, but we can QI directly
// InMemoryRandomAccessStream default IID
const IID_IRandomAccessStream = WinGuid.parse('905A0FE1-BC53-11DF-8C49-001E4FC686DA')

// IBufferFactory (Windows.Storage.Streams.Buffer)
const IID_IBufferFactory = WinGuid.parse('71AF914D-C10F-484B-BC50-14BC623B3A27')
const iBufferFactory = DynWinRtType.registerInterface("IBufferFactory", IID_IBufferFactory)
  .addMethod("Create", new DynWinRtMethodSig()
    .addIn(DynWinRtType.u32()) // capacity
    .addOut(DynWinRtType.object())) // IBuffer

// IBuffer
const IID_IBuffer = WinGuid.parse('905A0FE0-BC53-11DF-8C49-001E4FC686DA')
const iBuffer = DynWinRtType.registerInterface("IBuffer", IID_IBuffer)
  .addMethod("get_Capacity", new DynWinRtMethodSig().addOut(DynWinRtType.u32()))
  .addMethod("get_Length", new DynWinRtMethodSig().addOut(DynWinRtType.u32()))
  .addMethod("put_Length", new DynWinRtMethodSig().addIn(DynWinRtType.u32()))

// --- Test ---
async function main() {
  // Create InMemoryRandomAccessStream
  const streamFactory = DynWinRtValue.activationFactory('Windows.Storage.Streams.InMemoryRandomAccessStream')
    .cast(IID_IActivationFactory)
  const stream = iActivationFactory.method(6).invoke(streamFactory, [])
  const outputStream = stream.cast(IID_IOutputStream)
  console.log("InMemoryRandomAccessStream created")

  // Create a Buffer with some data
  const bufFactory = DynWinRtValue.activationFactory('Windows.Storage.Streams.Buffer')
    .cast(IID_IBufferFactory)
  const buf = iBufferFactory.method(6).invoke(bufFactory, [DynWinRtValue.u32(1024)])
  const bufObj = buf.cast(IID_IBuffer)
  // Set length = capacity to simulate filled buffer
  iBuffer.method(8).invoke(bufObj, [DynWinRtValue.u32(1024)])
  console.log("Buffer created (1024 bytes)")

  // Test 1: WriteAsync without progress handler
  console.log("\n=== Test 1: WriteAsync without progress handler ===")
  const writeOp1 = iOutputStream.method(6).invoke(outputStream, [buf])
  console.log("  WriteAsync called, awaiting...")
  const result1 = await writeOp1.toPromise()
  console.log("  Result:", result1.toNumber(), "bytes written")

  // Test 2: WriteAsync WITH progress handler
  console.log("\n=== Test 2: WriteAsync WITH progress handler ===")
  // Create another buffer
  const buf2 = iBufferFactory.method(6).invoke(bufFactory, [DynWinRtValue.u32(2048)])
  const buf2Obj = buf2.cast(IID_IBuffer)
  iBuffer.method(8).invoke(buf2Obj, [DynWinRtValue.u32(2048)])

  const writeOp2 = iOutputStream.method(6).invoke(outputStream, [buf2])
  let progressCount = 0
  console.log("  Setting progress handler...")
  writeOp2.onProgress((p: any) => {
    progressCount++
    console.log(`  >> Progress #${progressCount}: ${p.toNumber()} bytes`)
  })
  console.log("  Progress handler set, awaiting...")
  const result2 = await writeOp2.toPromise()
  console.log("  Result:", result2.toNumber(), "bytes written")
  console.log("  Progress callbacks received:", progressCount)

  // Test 3: Simulate EnsureReadyAsync pattern — IAsyncOperationWithProgress<Object, f64>
  // Same as AI API's EnsureReadyAsync but using WriteAsync's raw op with f64 type declared
  console.log("\n=== Test 3: onProgress with f64 progress type (simulated) ===")
  const buf3 = iBufferFactory.method(6).invoke(bufFactory, [DynWinRtValue.u32(512)])
  const buf3Obj = buf3.cast(IID_IBuffer)
  iBuffer.method(8).invoke(buf3Obj, [DynWinRtValue.u32(512)])

  // Register a different interface for WriteAsync that declares f64 progress type
  // (wrong type, but tests the delegate creation / set_progress_handler path with f64)
  const IID_IOutputStream2 = WinGuid.parse('905A0FE6-BC53-11DF-8C49-001E4FC686DA')
  const iOutputStream2 = DynWinRtType.registerInterface("IOutputStream_f64", IID_IOutputStream2)
    .addMethod("WriteAsync", new DynWinRtMethodSig()
      .addIn(DynWinRtType.object())
      .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.u32(), DynWinRtType.f64())))

  const writeOp3 = iOutputStream2.method(6).invoke(outputStream, [buf3])
  console.log("  Setting f64 progress handler...")
  writeOp3.onProgress((p: any) => {
    console.log(`  >> f64 Progress: ${p.toF64()}`)
  })
  console.log("  f64 Progress handler set, awaiting...")
  try {
    const result3 = await writeOp3.toPromise()
    console.log("  Result:", result3.toNumber())
  } catch (e: any) {
    console.log("  Error (expected - wrong type):", e.message)
  }

  console.log("\n=== All tests passed! ===")
}

main().catch(e => {
  console.error("FATAL:", e)
  process.exit(1)
})
