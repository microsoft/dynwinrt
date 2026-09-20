// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  assertNoDispatcherQueueImports,
  assertNoEagerWin32Imports,
  assertNoUiHelperImports,
  readPeImports,
  verifyAddonImports,
  verifyDispatcherQueueImports,
  verifyUiHelperImports,
} from '../scripts/pe-imports.mjs'

function fixture(wide: boolean) {
  const bytes = Buffer.alloc(0x800)
  const optional = 0x98
  const optionalSize = wide ? 240 : 224
  const directory = optional + (wide ? 112 : 96)
  const sections = optional + optionalSize
  bytes.write('MZ')
  bytes.writeUInt32LE(0x80, 0x3c)
  bytes.writeUInt32LE(0x4550, 0x80)
  bytes.writeUInt16LE(wide ? 0x8664 : 0x14c, 0x84)
  bytes.writeUInt16LE(2, 0x86)
  bytes.writeUInt16LE(optionalSize, 0x94)
  bytes.writeUInt16LE(wide ? 0x20b : 0x10b, optional)
  bytes.writeUInt32LE(0x200, optional + 60)
  bytes.writeUInt32LE(16, directory - 4)
  bytes.writeUInt32LE(0x2020, directory + 8)
  bytes.writeUInt32LE(60, directory + 12)
  for (const [index, rva, size, offset] of [
    [0, 0x2000, 0x500, 0x200],
    [1, 0x4000, 0x100, 0x700],
  ]) {
    const section = sections + index * 40
    bytes.write(index ? '.names' : '.idata', section)
    bytes.writeUInt32LE(size, section + 8)
    bytes.writeUInt32LE(rva, section + 12)
    bytes.writeUInt32LE(size, section + 16)
    bytes.writeUInt32LE(offset, section + 20)
  }
  for (const [descriptor, lookup, name, address] of [
    [0x220, 0x2080, 0x4020, 0x20c0],
    [0x234, 0x20a0, 0x4060, 0x20e0],
  ]) {
    bytes.writeUInt32LE(lookup, descriptor)
    bytes.writeUInt32LE(name, descriptor + 12)
    bytes.writeUInt32LE(address, descriptor + 16)
  }
  const writeThunk = (offset: number, value: bigint) => {
    if (wide) bytes.writeBigUInt64LE(value, offset)
    else bytes.writeUInt32LE(Number(value), offset)
  }
  const width = wide ? 8 : 4
  for (const offset of [0x280, 0x2c0]) {
    writeThunk(offset, 0x4040n)
    writeThunk(offset + width, (1n << BigInt(width * 8 - 1)) | 17n)
  }
  for (const offset of [0x2a0, 0x2e0]) writeThunk(offset, 0x4080n)
  bytes.write('KERNEL32.dll\0', 0x720)
  bytes.writeUInt16LE(7, 0x740)
  bytes.write('GetTickCount\0', 0x742)
  bytes.write('MFPLAT.DLL\0', 0x760)
  bytes.write('MFStartup\0', 0x782)
  bytes.write('mapi32.dll\0GdiplusStartup\0', 0x600)
  return { bytes, optional, directory, sections, width, writeThunk }
}

const expected = [
  { dll: 'KERNEL32.dll', symbols: ['GetTickCount', 17] },
  { dll: 'MFPLAT.DLL', symbols: ['MFStartup'] },
]

function delayFixture(wide: boolean) {
  const image = fixture(wide)
  const { bytes, directory, width, writeThunk } = image
  const delayDirectory = directory + 13 * 8
  bytes.writeUInt32LE(0x2120, delayDirectory)
  bytes.writeUInt32LE(96, delayDirectory + 4)
  for (const [descriptor, name, module, address, lookup] of [
    [0x320, 0x2250, 0x2220, 0x21c0, 0x2180],
    [0x340, 0x2290, 0x2228, 0x21e0, 0x21a0],
  ]) {
    bytes.writeUInt32LE(1, descriptor)
    bytes.writeUInt32LE(name, descriptor + 4)
    bytes.writeUInt32LE(module, descriptor + 8)
    bytes.writeUInt32LE(address, descriptor + 12)
    bytes.writeUInt32LE(lookup, descriptor + 16)
  }
  writeThunk(0x380, 0x22d0n)
  writeThunk(0x380 + width, (1n << BigInt(width * 8 - 1)) | 23n)
  writeThunk(0x3a0, 0x2320n)
  // Delay IAT entries are helper addresses, not import names.
  writeThunk(0x3c0, 0x12345678n)
  writeThunk(0x3e0, 0x12345678n)
  bytes.write('USER32.dll\0', 0x450)
  bytes.write('OLE32.dll\0', 0x490)
  bytes.write('MessageBeep\0', 0x4d2)
  bytes.write('CoInitializeEx\0', 0x522)
  return { ...image, delayDirectory }
}

const delayExpected = [
  { dll: 'USER32.dll', symbols: ['MessageBeep', 23] },
  { dll: 'OLE32.dll', symbols: ['CoInitializeEx'] },
]

for (const wide of [false, true]) {
  const format = wide ? 'PE32+' : 'PE32'

  test(`Win32 ${format} parser follows DLL and named/ordinal thunk RVAs across sections`, (t) => {
    t.deepEqual(readPeImports(fixture(wide).bytes), expected)
  })

  test(`Win32 ${format} parser falls back to FirstThunk when OriginalFirstThunk is absent`, (t) => {
    const { bytes } = fixture(wide)
    bytes.writeUInt32LE(0, 0x220)
    bytes.writeUInt32LE(0, 0x234)
    t.deepEqual(readPeImports(bytes), expected)
  })

  test(`Win32 ${format} parser supports header RVAs and an absent import directory`, (t) => {
    const { bytes, directory } = fixture(wide)
    bytes.writeUInt32LE(0x1e0, 0x22c)
    bytes.write('ole32.dll\0', 0x1e0)
    t.deepEqual(readPeImports(bytes), [{ ...expected[0], dll: 'ole32.dll' }, expected[1]])
    bytes.writeUInt32LE(1, directory - 4)
    t.deepEqual(readPeImports(bytes), [])
    bytes.writeUInt32LE(16, directory - 4)
    bytes.writeUInt32LE(0, directory + 8)
    bytes.writeUInt32LE(0, directory + 12)
    t.deepEqual(readPeImports(bytes), [])
  })

  test(`Win32 ${format} parser ignores delay imports and incidental DLL/symbol strings`, (t) => {
    const { bytes, directory } = fixture(wide)
    bytes.writeUInt32LE(0, directory + 8)
    bytes.writeUInt32LE(0, directory + 12)
    bytes.writeUInt32LE(0x2020, directory + 13 * 8)
    bytes.writeUInt32LE(60, directory + 13 * 8 + 4)
    t.deepEqual(readPeImports(bytes), [])
  })

  const malformed: [string, (image: ReturnType<typeof fixture>) => void, RegExp][] = [
    ['MZ signature', ({ bytes }) => bytes.writeUInt16LE(0, 0), /missing MZ/],
    ['PE header offset', ({ bytes }) => bytes.writeUInt32LE(0xfffffff0, 0x3c), /truncated PE header/],
    ['overlapping PE header', ({ bytes }) => bytes.writeUInt32LE(2, 0x3c), /invalid PE header offset/],
    ['PE signature', ({ bytes }) => bytes.writeUInt32LE(0, 0x80), /missing PE signature/],
    ['optional header size', ({ bytes }) => bytes.writeUInt16LE(2, 0x94), /truncated optional header/],
    ['optional header magic', ({ bytes, optional }) => bytes.writeUInt16LE(0x107, optional), /unsupported.*magic/],
    ['directory count', ({ bytes, directory }) => bytes.writeUInt32LE(17, directory - 4), /directories exceed/],
    ['section table', ({ bytes }) => bytes.writeUInt16LE(0xffff, 0x86), /truncated section table/],
    ['header size', ({ bytes, optional }) => bytes.writeUInt32LE(0x100, optional + 60), /SizeOfHeaders/],
    ['section data', ({ bytes, sections }) => bytes.writeUInt32LE(0x800, sections + 16), /truncated section data/],
    ['section in headers', ({ bytes, sections }) => bytes.writeUInt32LE(0x100, sections + 20), /overlaps headers/],
    ['section RVA overflow', ({ bytes, sections }) => bytes.writeUInt32LE(0xffffff00, sections + 12), /RVA overflow/],
    ['zero directory RVA', ({ bytes, directory }) => bytes.writeUInt32LE(0, directory + 8), /invalid import directory/],
    [
      'zero directory size',
      ({ bytes, directory }) => bytes.writeUInt32LE(0, directory + 12),
      /invalid import directory/,
    ],
    ['short directory', ({ bytes, directory }) => bytes.writeUInt32LE(19, directory + 12), /invalid import directory/],
    [
      'directory terminator',
      ({ bytes, directory }) => bytes.writeUInt32LE(40, directory + 12),
      /unterminated import directory/,
    ],
    ['unmapped directory', ({ bytes, directory }) => bytes.writeUInt32LE(0x3000, directory + 8), /unmapped.*directory/],
    ['unmapped name', ({ bytes }) => bytes.writeUInt32LE(0x3000, 0x22c), /unmapped.*DLL name/],
    ['missing address table', ({ bytes }) => bytes.writeUInt32LE(0, 0x230), /missing import address table/],
    ['unmapped thunk', ({ bytes }) => bytes.writeUInt32LE(0x3000, 0x220), /unmapped.*thunk/],
    ['empty DLL name', ({ bytes }) => bytes.writeUInt8(0, 0x720), /empty DLL name/],
    ['non-ASCII DLL name', ({ bytes }) => bytes.writeUInt8(0xff, 0x720), /non-ASCII DLL name/],
    ['empty symbol', ({ bytes }) => bytes.writeUInt8(0, 0x742), /empty import symbol/],
    ['ambiguous RVA', ({ bytes, sections }) => bytes.writeUInt32LE(0x2000, sections + 40 + 12), /ambiguous.*directory/],
    [
      'virtual-only directory',
      ({ bytes, sections, directory }) => {
        bytes.writeUInt32LE(0x1000, sections + 8)
        bytes.writeUInt32LE(0x2600, directory + 8)
      },
      /directory is not file-backed/,
    ],
    [
      'thunk terminator',
      ({ bytes, width, writeThunk }) => {
        bytes.writeUInt32LE(0x2500 - width, 0x220)
        writeThunk(0x700 - width, 0x4040n)
      },
      /unterminated import thunk/,
    ],
    [
      'DLL terminator',
      ({ bytes }) => {
        bytes.writeUInt32LE(0x40fc, 0x22c)
        bytes.fill(65, 0x7fc)
      },
      /unterminated DLL name/,
    ],
    [
      'symbol terminator',
      ({ bytes, writeThunk }) => {
        writeThunk(0x280, 0x40fan)
        bytes.fill(65, 0x7fc)
      },
      /unterminated import symbol/,
    ],
    ['hint/name bounds', ({ writeThunk }) => writeThunk(0x280, 0x40fen), /hint\/name is not file-backed/],
    [
      'ordinal reserved bits',
      ({ width, writeThunk }) => writeThunk(0x280, (1n << BigInt(width * 8 - 1)) | 0x10011n),
      /reserved ordinal thunk bits/,
    ],
  ]
  for (const [name, corrupt, message] of malformed) {
    test(`Win32 ${format} parser rejects malformed ${name}`, (t) => {
      const image = fixture(wide)
      corrupt(image)
      t.throws(() => readPeImports(image.bytes), { message })
    })
  }
}

for (const wide of [false, true]) {
  const format = wide ? 'PE32+' : 'PE32'
  const options = { includeDelayImports: true }

  test(`Win32 ${format} parser optionally includes named and ordinal delay imports`, (t) => {
    const { bytes } = delayFixture(wide)
    t.deepEqual(readPeImports(bytes), expected)
    t.deepEqual(readPeImports(bytes, { includeDelayImports: false }), expected)
    t.deepEqual(readPeImports(bytes, options), [...expected, ...delayExpected])
  })

  test(`Win32 ${format} delay inspection handles absent ordinary and delay directories independently`, (t) => {
    const { bytes, directory, delayDirectory } = delayFixture(wide)
    bytes.writeUInt32LE(0, directory + 8)
    bytes.writeUInt32LE(0, directory + 12)
    t.deepEqual(readPeImports(bytes, options), delayExpected)
    bytes.writeUInt32LE(0, delayDirectory)
    bytes.writeUInt32LE(0, delayDirectory + 4)
    t.deepEqual(readPeImports(bytes, options), [])
    const counted = delayFixture(wide)
    counted.bytes.writeUInt32LE(13, counted.directory - 4)
    t.deepEqual(readPeImports(counted.bytes, options), expected)
    counted.bytes.writeUInt32LE(0, counted.directory - 4)
    t.deepEqual(readPeImports(counted.bytes, options), [])
  })

  test(`Win32 ${format} delay inspection ignores incidental forbidden DLL and symbol strings`, (t) => {
    const { bytes } = delayFixture(wide)
    bytes.write('CoreMessaging.dll\0CreateDispatcherQueueController\0', 0x600)
    t.deepEqual(readPeImports(bytes, options), [...expected, ...delayExpected])
    t.notThrows(() => assertNoDispatcherQueueImports(readPeImports(bytes, options)))
  })

  const malformed: [string, (image: ReturnType<typeof delayFixture>) => void, RegExp][] = [
    ['legacy VA descriptors', ({ bytes }) => bytes.writeUInt32LE(0, 0x320), /unsupported delay import attributes: 0x0/],
    ['unknown flags', ({ bytes }) => bytes.writeUInt32LE(3, 0x320), /unsupported delay import attributes: 0x3/],
    [
      'zero RVA',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(0, delayDirectory),
      /invalid delay import directory/,
    ],
    [
      'zero size',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(0, delayDirectory + 4),
      /invalid delay import directory/,
    ],
    [
      'short descriptor',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(31, delayDirectory + 4),
      /invalid delay import directory/,
    ],
    [
      'missing terminator',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(64, delayDirectory + 4),
      /unterminated delay import directory/,
    ],
    [
      'partial terminator',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(80, delayDirectory + 4),
      /unterminated delay import directory/,
    ],
    [
      'unmapped directory',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(0x3000, delayDirectory),
      /unmapped.*delay import directory/,
    ],
    [
      'directory bounds',
      ({ bytes, delayDirectory }) => bytes.writeUInt32LE(0x400, delayDirectory + 4),
      /delay import directory is not file-backed/,
    ],
    ['missing address table', ({ bytes }) => bytes.writeUInt32LE(0, 0x32c), /missing delay import address table/],
    [
      'unmapped address table',
      ({ bytes }) => bytes.writeUInt32LE(0x3000, 0x32c),
      /unmapped.*delay import address table/,
    ],
    ['missing name table', ({ bytes }) => bytes.writeUInt32LE(0, 0x330), /missing delay import name table/],
    ['unmapped name table', ({ bytes }) => bytes.writeUInt32LE(0x3000, 0x330), /unmapped.*import thunk/],
    ['unmapped DLL name', ({ bytes }) => bytes.writeUInt32LE(0x3000, 0x324), /unmapped.*DLL name/],
    ['empty DLL name', ({ bytes }) => bytes.writeUInt8(0, 0x450), /empty DLL name/],
    ['empty symbol', ({ bytes }) => bytes.writeUInt8(0, 0x4d2), /empty import symbol/],
    [
      'thunk terminator',
      ({ bytes, width, writeThunk }) => {
        bytes.writeUInt32LE(0x2500 - width, 0x330)
        writeThunk(0x700 - width, 0x22d0n)
      },
      /unterminated import thunk/,
    ],
    [
      'symbol terminator',
      ({ bytes, writeThunk }) => {
        writeThunk(0x380, 0x40fan)
        bytes.fill(65, 0x7fc)
      },
      /unterminated import symbol/,
    ],
    [
      'ordinal reserved bits',
      ({ width, writeThunk }) => writeThunk(0x380, (1n << BigInt(width * 8 - 1)) | 0x10017n),
      /reserved ordinal thunk bits/,
    ],
  ]
  for (const [name, corrupt, message] of malformed) {
    test(`Win32 ${format} delay parser rejects malformed ${name} only when requested`, (t) => {
      const image = delayFixture(wide)
      corrupt(image)
      t.deepEqual(readPeImports(image.bytes), expected)
      t.throws(() => readPeImports(image.bytes, options), { message })
    })
  }

  for (const delayed of [false, true]) {
    const kind = delayed ? 'delay' : 'ordinary'
    test(`Win32 ${format} DispatcherQueue policy rejects prohibited ${kind} DLL and symbol imports`, (t) => {
      const image = delayFixture(wide)
      const { bytes, width, writeThunk } = image
      const name = delayed ? 0x490 : 0x760
      const symbol = delayed ? 0x522 : 0x782
      const lookup = delayed ? 0x3a0 : 0x2a0
      t.notThrows(() => assertNoDispatcherQueueImports(readPeImports(bytes, options)))
      bytes.write('C:\\Windows\\System32\\CoReMeSsAgInG.DlL\0', name)
      writeThunk(lookup, (1n << BigInt(width * 8 - 1)) | 17n)
      t.throws(() => assertNoDispatcherQueueImports(readPeImports(bytes, options)), {
        message: /CoReMeSsAgInG\.DlL: #17/,
      })
      writeThunk(lookup, 0n)
      t.throws(() => assertNoDispatcherQueueImports(readPeImports(bytes, options)), { message: /CoReMeSsAgInG\.DlL:/ })
      bytes.write('other.dll\0', name)
      writeThunk(lookup, BigInt(delayed ? 0x2320 : 0x4080))
      bytes.write('_cReAtEdIsPaTcHeRqUeUeCoNtRoLlEr@12\0', symbol)
      t.throws(() => assertNoDispatcherQueueImports(readPeImports(bytes, options)), { message: /other\.dll: _cReAtE/ })
    })
  }
}

test('Win32 PE parser rejects truncated buffers and oversized PE32+ name RVAs', (t) => {
  const { bytes, writeThunk } = fixture(true)
  for (const length of [0, 2, 63, 0x90, 0x100, 0x7ff]) {
    t.throws(() => readPeImports(bytes.subarray(0, length)), { message: /^Invalid PE import table:/ })
  }
  writeThunk(0x280, 0x100004040n)
  t.throws(() => readPeImports(bytes), { message: /import name RVA overflow/ })
  t.throws(() => readPeImports('not a Buffer' as unknown as Buffer), { instanceOf: TypeError })
  const delayed = delayFixture(true)
  delayed.writeThunk(0x380, 0x1000022d0n)
  t.throws(() => readPeImports(delayed.bytes, { includeDelayImports: true }), { message: /import name RVA overflow/ })
  for (const length of [0, 2, 63, 0x90, 0x100, 0x7ff]) {
    t.throws(() => readPeImports(delayed.bytes.subarray(0, length), { includeDelayImports: true }), {
      message: /^Invalid PE import table:/,
    })
  }
})

test('Win32 DispatcherQueue import policy normalizes DLL paths/case and rejects named exports from any DLL', (t) => {
  for (const dll of [
    'coremessaging.dll',
    'COREMESSAGING.DLL',
    'C:\\Windows\\CoreMessaging.dll',
    'C:/Windows/CoreMessaging.dll',
  ]) {
    for (const symbols of [['SafeExport'], [17], []]) {
      t.throws(() => assertNoDispatcherQueueImports([{ dll, symbols }]), { message: /must resolve.*dynamically/ })
    }
  }
  for (const symbol of [
    'CreateDispatcherQueueController',
    'createdispatcherqueuecontroller',
    '_CreateDispatcherQueueController@12',
  ]) {
    t.throws(() => assertNoDispatcherQueueImports([{ dll: 'other.dll', symbols: [symbol] }]), {
      message: /other\.dll:/,
    })
  }
  t.notThrows(() =>
    assertNoDispatcherQueueImports([{ dll: 'kernel32.dll', symbols: ['LoadLibraryExW', 'GetProcAddress', 17] }]),
  )
})

test('Win32 production import policy bans optional DLLs, including ordinals, regardless of case or path', (t) => {
  for (const dll of ['mfplat.dll', 'GDIPLUS.DLL', 'ws2_32.dll', 'C:\\Windows\\System32\\MAPI32.dll']) {
    for (const symbols of [['MFCreateSample'], [17], []]) {
      t.throws(() => assertNoEagerWin32Imports([{ dll, symbols }]), { message: /must load lazily/ })
    }
  }
  t.notThrows(() =>
    assertNoEagerWin32Imports([
      { dll: 'KERNEL32.dll', symbols: ['LoadLibraryExW', 'GetProcAddress'] },
      { dll: 'ole32.dll', symbols: ['CoInitializeEx'] },
    ]),
  )
  t.throws(() => assertNoEagerWin32Imports(readPeImports(fixture(true).bytes)), { message: /MFPLAT\.DLL: MFStartup/ })
})

test('Win32 test-hooks import policy permits only the exact existing COM media fixture symbol', (t) => {
  t.notThrows(() =>
    assertNoEagerWin32Imports([{ dll: 'mfplat.dll', symbols: ['MFCreateSample'] }], { testHooks: true }),
  )
  for (const symbol of ['MFStartup', 'MFShutdown', 'MFCreateMemoryBuffer', 'MFCreateSomethingNew', 17]) {
    t.throws(
      () =>
        assertNoEagerWin32Imports([{ dll: 'mfplat.dll', symbols: ['MFCreateSample', symbol] }], { testHooks: true }),
      { message: /must load lazily/ },
    )
  }
  for (const dll of ['ws2_32.dll', 'gdiplus.dll', 'mapi32.dll']) {
    t.throws(() => assertNoEagerWin32Imports([{ dll, symbols: ['MFCreateSample'] }], { testHooks: true }))
  }
  t.throws(() => assertNoEagerWin32Imports([{ dll: 'mfplat.dll', symbols: [] }], { testHooks: true }))
})

test('Win32 artifact import verification checks every shipped architecture without loading binaries', (t) => {
  const directory = mkdtempSync(fileURLToPath(new URL('../target-pe-imports-', import.meta.url)))
  const x64 = 'dynwinrt.win32-x64-msvc.node'
  const arm64 = 'dynwinrt.win32-arm64-msvc.node'
  const safe = fixture(true)
  safe.bytes.write('ole32.dll\0', 0x760)
  const hooks = fixture(true)
  hooks.bytes.fill(0, 0x782)
  hooks.bytes.write('MFCreateSample\0', 0x782)
  try {
    t.throws(() => verifyAddonImports(directory), { message: /No native addons/ })
    writeFileSync(join(directory, x64), safe.bytes)
    writeFileSync(join(directory, 'not-an-addon.txt'), fixture(true).bytes)
    t.deepEqual(verifyAddonImports(directory), [x64])
    writeFileSync(join(directory, arm64), fixture(true).bytes)
    t.throws(() => verifyAddonImports(directory), { message: /arm64-msvc\.node:.*eager PE imports/ })
    writeFileSync(join(directory, arm64), safe.bytes)
    t.deepEqual(verifyAddonImports(directory), [arm64, x64])
    writeFileSync(join(directory, x64), hooks.bytes)
    t.throws(() => verifyAddonImports(directory), { message: /x64-msvc\.node:.*eager PE imports/ })
    t.deepEqual(verifyAddonImports(directory, { testHooksAddon: x64 }), [arm64, x64])
    writeFileSync(join(directory, arm64), hooks.bytes)
    t.throws(() => verifyAddonImports(directory, { testHooksAddon: x64 }), {
      message: /arm64-msvc\.node:.*eager PE imports/,
    })
    t.throws(() => verifyAddonImports(directory, { testHooksAddon: 'absent.node' }), {
      message: /Missing test-hooks addon/,
    })
    writeFileSync(join(directory, x64), Buffer.from('MZ'))
    t.throws(() => verifyAddonImports(directory), { message: /arm64-msvc\.node:.*eager PE imports/ })
    writeFileSync(join(directory, arm64), safe.bytes)
    t.throws(() => verifyAddonImports(directory), { message: /x64-msvc\.node: Invalid PE import table/ })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('Win32 DispatcherQueue verification scans every addon including ARM64 delay imports without loading binaries', (t) => {
  const directory = mkdtempSync(fileURLToPath(new URL('../target-pe-imports-', import.meta.url)))
  const x64 = 'dynwinrt.win32-x64-msvc.node'
  const arm64 = 'dynwinrt.win32-arm64-msvc.node'
  const extra = 'EXTRA.NODE'
  const safe = delayFixture(true)
  const forbidden = delayFixture(true)
  const eager = fixture(true)
  forbidden.bytes.writeUInt16LE(0xaa64, 0x84)
  forbidden.bytes.write('CreateDispatcherQueueController\0', 0x522)
  eager.bytes.write('CoreMessaging.dll\0', 0x760)
  try {
    t.throws(() => verifyDispatcherQueueImports(directory), { message: /No native addons/ })
    writeFileSync(join(directory, x64), safe.bytes)
    writeFileSync(join(directory, 'ignored.txt'), forbidden.bytes)
    t.deepEqual(verifyDispatcherQueueImports(directory), [x64])
    writeFileSync(join(directory, arm64), forbidden.bytes)
    t.throws(() => verifyDispatcherQueueImports(directory), { message: /arm64-msvc\.node:.*must resolve/ })
    forbidden.bytes.write('CoInitializeEx\0', 0x522)
    writeFileSync(join(directory, arm64), forbidden.bytes)
    t.deepEqual(verifyDispatcherQueueImports(directory), [arm64, x64])
    writeFileSync(join(directory, x64), eager.bytes)
    t.throws(() => verifyDispatcherQueueImports(directory), { message: /x64-msvc\.node:.*must resolve/ })
    writeFileSync(join(directory, x64), safe.bytes)
    writeFileSync(join(directory, extra), eager.bytes)
    t.throws(() => verifyDispatcherQueueImports(directory), { message: /EXTRA\.NODE:.*must resolve/ })
    forbidden.bytes.writeUInt32LE(0, 0x320)
    writeFileSync(join(directory, extra), forbidden.bytes)
    t.throws(() => verifyDispatcherQueueImports(directory), {
      message: /EXTRA\.NODE:.*unsupported delay import attributes/,
    })
    writeFileSync(join(directory, extra), Buffer.from('MZ'))
    t.throws(() => verifyDispatcherQueueImports(directory), { message: /EXTRA\.NODE: Invalid PE import table/ })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('UI helper policy rejects DLL descriptors and scoped exports without banning OLEAUT32', (t) => {
  for (const dll of ['GDI32.dll', 'user32.dll', 'C:\\Windows\\System32\\UsEr32.DlL']) {
    for (const symbols of [['UnrelatedExport'], [17], []]) {
      t.throws(() => assertNoUiHelperImports([{ dll, symbols }]), { message: /GDI\/USER32.*dynamically/ })
    }
  }
  for (const symbol of [
    'DeleteObject',
    'DestroyIcon',
    'CreateWindowExW',
    'AreDpiAwarenessContextsEqual',
    'GetDpiAwarenessContextForProcess',
    'SetProcessDpiAwarenessContext',
    'SetThreadDpiAwarenessContext',
  ]) {
    for (const spelling of [symbol, symbol.toLowerCase(), `_${symbol}@8`, `__imp_${symbol}`]) {
      t.throws(() => assertNoUiHelperImports([{ dll: 'api-alias.dll', symbols: [spelling] }]), {
        message: /api-alias\.dll:/,
      })
    }
  }
  t.notThrows(() =>
    assertNoUiHelperImports([
      { dll: 'kernel32.dll', symbols: ['LoadLibraryExW', 'GetProcAddress'] },
      { dll: 'oleaut32.dll', symbols: ['SysAllocStringLen', 'VariantClear', 'SafeArrayDestroy'] },
      { dll: 'ole32.dll', symbols: ['CoInitializeEx'] },
      { dll: 'combase.dll', symbols: ['RoInitialize'] },
    ]),
  )
})

for (const wide of [false, true]) {
  for (const delayed of [false, true]) {
    test(`UI helper policy inspects ${wide ? 'PE32+' : 'PE32'} ${delayed ? 'delay' : 'ordinary'} tables`, (t) => {
      const { bytes, writeThunk, width } = delayFixture(wide)
      // Start with unrelated DLLs in both directories.
      bytes.write('other.dll\0', 0x450)
      const name = delayed ? 0x490 : 0x760
      const symbol = delayed ? 0x522 : 0x782
      const lookup = delayed ? 0x3a0 : 0x2a0
      const options = { includeDelayImports: true }
      t.notThrows(() => assertNoUiHelperImports(readPeImports(bytes, options)))
      bytes.write('gdi32.dll\0', name)
      writeThunk(lookup, (1n << BigInt(width * 8 - 1)) | 17n)
      t.throws(() => assertNoUiHelperImports(readPeImports(bytes, options)), { message: /gdi32\.dll: #17/ })
      bytes.write('alias.dll\0', name)
      writeThunk(lookup, BigInt(delayed ? 0x2320 : 0x4080))
      bytes.write('_SetThreadDpiAwarenessContext@4\0', symbol)
      t.throws(() => assertNoUiHelperImports(readPeImports(bytes, options)), { message: /alias\.dll: _SetThread/ })
    })
  }
}

test('UI helper production checks scan every architecture and Python extensions without loading them', (t) => {
  const directory = mkdtempSync(fileURLToPath(new URL('../target-pe-imports-', import.meta.url)))
  const safe = fixture(true)
  safe.bytes.write('ole32.dll\0', 0x760)
  const forbidden = delayFixture(true)
  forbidden.bytes.writeUInt16LE(0xaa64, 0x84)
  const x64 = 'dynwinrt.win32-x64-msvc.node'
  const arm64 = 'dynwinrt.win32-arm64-msvc.node'
  const script = fileURLToPath(new URL('../scripts/check-ui-helper-imports.mjs', import.meta.url))
  const pyd = join(directory, 'dynwinrt.cp313-win_arm64.pyd')
  try {
    t.throws(() => verifyUiHelperImports(directory), { message: /No native addons/ })
    writeFileSync(join(directory, x64), safe.bytes)
    writeFileSync(join(directory, arm64), forbidden.bytes)
    t.throws(() => verifyUiHelperImports(directory), { message: /arm64-msvc\.node:.*GDI\/USER32/ })
    writeFileSync(join(directory, arm64), safe.bytes)
    t.deepEqual(verifyUiHelperImports(directory), [arm64, x64])
    writeFileSync(pyd, safe.bytes)
    t.is(spawnSync(process.execPath, [script, pyd], { encoding: 'utf8' }).status, 0)
    writeFileSync(pyd, forbidden.bytes)
    const child = spawnSync(process.execPath, [script, pyd], { encoding: 'utf8' })
    t.not(child.status, 0)
    t.regex(child.stderr, /GDI\/USER32/)
    writeFileSync(join(directory, arm64), Buffer.from('MZ'))
    t.throws(() => verifyUiHelperImports(directory), { message: /arm64-msvc\.node: Invalid PE import table/ })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
