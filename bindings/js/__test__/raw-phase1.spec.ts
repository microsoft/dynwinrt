import test from 'ava'
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'

import {
  DynCom,
  DynComMethodSig,
  DynComRaw,
  DynComRawCleanup,
  DynComRawMemory,
  DynComRawOwnedComPointer,
  DynComRawPointer,
  DynComRawStructLayout,
  DynComUnsafe,
  WinGuid,
  initializeCom,
} from '../dist/com-unsafe-raw.js'

test('public raw package subpath loads in CJS and ESM', (t) => {
  for (const [arguments_, marker] of [
    [
      [
        '--eval',
        "const raw = require('@microsoft/dynwinrt/com/unsafe/raw');" +
          "if (typeof raw.DynComRawMemory !== 'function') process.exit(2);" +
          "console.log('raw-cjs-ok')",
      ],
      /raw-cjs-ok/,
    ],
    [
      [
        '--input-type=module',
        '--eval',
        "import * as raw from '@microsoft/dynwinrt/com/unsafe/raw';" +
          "if (typeof raw.DynComRawMemory !== 'function') process.exit(2);" +
          "console.log('raw-esm-ok')",
      ],
      /raw-esm-ok/,
    ],
  ] as const) {
    const result = spawnSync(process.execPath, arguments_, {
      cwd: resolve(process.cwd()),
      encoding: 'utf8',
      windowsHide: true,
    })
    t.is(result.status, 0, result.stderr)
    t.regex(result.stdout, marker)
  }
})

test('public raw package supports nested pointer slots and WAVEFORMATEX storage', (t) => {
  const pointerSize = DynComRaw.pointerSize()
  const target = DynComRawMemory.allocate(pointerSize, pointerSize)
  const inner = DynComRawMemory.allocate(pointerSize, pointerSize)
  const outer = DynComRawMemory.allocate(pointerSize, pointerSize)
  try {
    target.writeUsize(0, 1n)
    inner.writePointer(0, target.pointer())
    outer.writePointer(0, inner.pointer())
    t.is(outer.readPointer(0).address, inner.pointer().address)
    t.is(inner.readPointer(0).address, target.pointer().address)

    const waveFormat = DynComRawMemory.allocate(22, 1)
    try {
      waveFormat.writeU16(0, 1)
      waveFormat.writeU16(2, 2)
      waveFormat.writeU32(4, 48000)
      waveFormat.writeU32(8, 192000)
      waveFormat.writeU16(12, 4)
      waveFormat.writeU16(14, 16)
      waveFormat.writeU16(16, 4)
      waveFormat.writeBytes(18, Buffer.from([9, 8, 7, 6]))
      t.is(waveFormat.readU16(16), 4)
      t.deepEqual(waveFormat.readBytes(18, 4), Buffer.from([9, 8, 7, 6]))
    } finally {
      waveFormat.release()
    }
  } finally {
    outer.release()
    inner.release()
    target.release()
  }
})

test('public raw package exposes architecture-selected aggregate layout', (t) => {
  const pointerSize = DynComRaw.pointerSize()
  const layout = {
    size: pointerSize === 8 ? 32 : 20,
    alignment: pointerSize,
    fields: [
      { name: 'format', offset: 0, count: 1, type: { kind: 'u16' } },
      { name: 'targetDevice', offset: pointerSize === 8 ? 8 : 4, count: 1, type: { kind: 'pointer' } },
      { name: 'aspect', offset: pointerSize === 8 ? 16 : 8, count: 1, type: { kind: 'u32' } },
      { name: 'index', offset: pointerSize === 8 ? 20 : 12, count: 1, type: { kind: 'i32' } },
      { name: 'medium', offset: pointerSize === 8 ? 24 : 16, count: 1, type: { kind: 'u32' } },
    ],
  }
  const descriptor = JSON.stringify({
    name: 'Windows.Win32.System.Com.FORMATETC.RawExample',
    x86: {
      size: 20,
      alignment: 4,
      fields: [
        { name: 'format', offset: 0, count: 1, type: { kind: 'u16' } },
        { name: 'targetDevice', offset: 4, count: 1, type: { kind: 'pointer' } },
        { name: 'aspect', offset: 8, count: 1, type: { kind: 'u32' } },
        { name: 'index', offset: 12, count: 1, type: { kind: 'i32' } },
        { name: 'medium', offset: 16, count: 1, type: { kind: 'u32' } },
      ],
    },
    x64: layout,
    arm64: layout,
  })
  const aggregate = DynComRawStructLayout.fromDescriptor(descriptor)
  t.is(aggregate.size, BigInt(layout.size))
  t.is(aggregate.alignment, BigInt(layout.alignment))
  t.truthy(aggregate.pointerType())
  t.truthy(aggregate.byValueType())
})

test('public raw package transfers and reconciles one COM reference', (t) => {
  initializeCom(1)
  const iidIUnknown = WinGuid.parse('00000000-0000-0000-c000-000000000046')
  const managed = DynComUnsafe.coCreateInstance('00021401-0000-0000-c000-000000000046', iidIUnknown)
  const owner = DynComRawOwnedComPointer.addRef(managed)
  const slot = DynComRawMemory.allocate(DynComRaw.pointerSize(), DynComRaw.pointerSize())
  try {
    owner.transferTo(slot)
    t.true(owner.released)
    const transferred = slot.readPointer(0)
    const reconciled = DynComRawOwnedComPointer.assumeTransferred(transferred, iidIUnknown)
    reconciled.release()
    t.true(reconciled.released)
  } finally {
    slot.release()
    managed.release()
  }

  t.is(typeof DynComRawCleanup.releaseStgMedium, 'function')
  t.is(typeof DynComRawPointer.fromAddress, 'function')
})

test.serial('generated unsafe wiring converts raw storage and owns its interface view', (t) => {
  initializeCom(1)
  const iid = WinGuid.parse('c5326f42-ff35-4de2-9c17-35ea64bbf8ec')
  const registered = DynCom.registerIUnknownInterface('Tests.IGeneratedUnsafeFixture', iid).addMethodAt(
    3,
    'writeValue',
    new DynComMethodSig().addIn(DynCom.i32Type()).addIn(DynCom.pointerType()),
  )
  const slot = DynComRawMemory.allocate(4, 4)
  let seenAddress = 0n
  const implementation = DynCom.createComObject([registered], (_interfaceIid, vtableIndex, value, pointer) => {
    t.is(vtableIndex, 3)
    t.is(DynCom.toNumber(value), 42)
    seenAddress = DynCom.asPointerBigint(pointer)
    slot.writeU32(0, 0xc0decafe)
    return 0
  })

  const token = Symbol('generated unsafe constructor')
  class GeneratedUnsafeFixture {
    readonly #object: typeof implementation

    private constructor(actualToken: symbol, object: typeof implementation) {
      if (actualToken !== token) throw new TypeError('Use GeneratedUnsafeFixture.from(...)')
      DynCom.bindComObject(object)
      this.#object = object
    }

    static from(value: typeof implementation | { readonly nativeValue: typeof implementation }) {
      const source = 'nativeValue' in value ? value.nativeValue : value
      return new GeneratedUnsafeFixture(token, source.cast(iid))
    }

    get nativeValue() {
      return this.#object
    }

    release() {
      this.#object.release()
    }

    writeValue(value: number, output: DynComRawMemory | DynComRawPointer) {
      const pointer = output instanceof DynComRawMemory ? output.pointer() : output
      registered.method(3).invokeAll(this.#object, [DynCom.i32(value), pointer.toValue()])
    }
  }

  const wrapper = GeneratedUnsafeFixture.from(implementation)
  implementation.release()
  try {
    wrapper.writeValue(42, slot)
    t.is(seenAddress, slot.pointer().address)
    t.is(slot.readU32(0), 0xc0decafe)
    t.truthy(wrapper.nativeValue)
    wrapper.release()
    wrapper.release()
    t.throws(() => wrapper.writeValue(42, slot), { message: /released|apartment-bound/ })
  } finally {
    wrapper.release()
    slot.release()
  }
})
