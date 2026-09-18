// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { assertNoEagerWin32Imports, readPeImports, verifyAddonImports } from '../scripts/pe-imports.mjs'

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

test('Win32 PE parser rejects truncated buffers and oversized PE32+ name RVAs', (t) => {
  const { bytes, writeThunk } = fixture(true)
  for (const length of [0, 2, 63, 0x90, 0x100, 0x7ff]) {
    t.throws(() => readPeImports(bytes.subarray(0, length)), { message: /^Invalid PE import table:/ })
  }
  writeThunk(0x280, 0x100004040n)
  t.throws(() => readPeImports(bytes), { message: /import name RVA overflow/ })
  t.throws(() => readPeImports('not a Buffer' as unknown as Buffer), { instanceOf: TypeError })
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
