// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { readFileSync, readdirSync } from 'node:fs'
import { join, win32 } from 'node:path'

/** @typedef {{ dll: string, symbols: (string | number)[] }} PeImport */

/**
 * Read PE imports from the file, not the process-wide loaded-module list.
 * Ordinary imports remain the default for eager-loading checks.
 * @param {Buffer} bytes
 * @param {{ includeDelayImports?: boolean }} options
 */
export function readPeImports(bytes, { includeDelayImports = false } = {}) {
  if (!Buffer.isBuffer(bytes)) throw new TypeError('PE input must be a Buffer')
  const invalid = (/** @type {string} */ reason) => new Error(`Invalid PE import table: ${reason}`)
  const range = (/** @type {number} */ offset, /** @type {number} */ size, /** @type {string} */ label) => {
    if (!Number.isSafeInteger(offset) || offset < 0 || size < 0 || offset + size > bytes.length) {
      throw invalid(`truncated ${label}`)
    }
  }
  range(0, 64, 'DOS header')
  if (bytes.readUInt16LE(0) !== 0x5a4d) throw invalid('missing MZ signature')
  const pe = bytes.readUInt32LE(0x3c)
  if (pe < 64) throw invalid('invalid PE header offset')
  range(pe, 24, 'PE header')
  if (bytes.readUInt32LE(pe) !== 0x4550) throw invalid('missing PE signature')

  const optional = pe + 24
  const optionalSize = bytes.readUInt16LE(pe + 20)
  range(optional, optionalSize, 'optional header')
  if (optionalSize < 2) throw invalid('truncated optional header magic')
  const magic = bytes.readUInt16LE(optional)
  if (magic !== 0x10b && magic !== 0x20b) throw invalid('unsupported optional header magic')
  const width = magic === 0x20b ? 8 : 4
  const directories = magic === 0x20b ? 112 : 96
  if (optionalSize < directories) throw invalid('truncated optional header')
  const directoryCount = bytes.readUInt32LE(optional + directories - 4)
  if (directoryCount > Math.floor((optionalSize - directories) / 8)) {
    throw invalid('data directories exceed optional header')
  }

  const sectionTable = optional + optionalSize
  const sectionCount = bytes.readUInt16LE(pe + 6)
  range(sectionTable, sectionCount * 40, 'section table')
  const headerSize = bytes.readUInt32LE(optional + 60)
  if (headerSize < sectionTable + sectionCount * 40 || headerSize > bytes.length) {
    throw invalid('invalid SizeOfHeaders')
  }
  /** @type {{ rva: number, span: number, size: number, offset: number }[]} */
  const sections = []
  for (let index = 0; index < sectionCount; index++) {
    const section = sectionTable + index * 40
    const virtualSize = bytes.readUInt32LE(section + 8)
    const rva = bytes.readUInt32LE(section + 12)
    const size = bytes.readUInt32LE(section + 16)
    const offset = bytes.readUInt32LE(section + 20)
    if (size && offset < headerSize) throw invalid('section overlaps headers')
    range(offset, size, 'section data')
    const span = Math.max(virtualSize, size)
    if (rva + span > 0x100000000) throw invalid('section RVA overflow')
    sections.push({ rva, span, size, offset })
  }

  const map = (/** @type {number} */ rva, /** @type {number} */ size, /** @type {string} */ label) => {
    if (!Number.isSafeInteger(rva) || rva <= 0 || rva + size > 0x100000000) {
      throw invalid(`invalid ${label} RVA`)
    }
    if (rva < headerSize) {
      if (rva + size > headerSize) throw invalid(`${label} exceeds headers`)
      return { offset: rva, end: headerSize }
    }
    const matches = sections.filter((section) => rva >= section.rva && rva - section.rva < section.span)
    if (matches.length !== 1) throw invalid(`unmapped or ambiguous ${label} RVA`)
    const section = matches[0]
    const delta = rva - section.rva
    if (delta + size > section.size) throw invalid(`${label} is not file-backed`)
    return { offset: section.offset + delta, end: section.offset + section.size }
  }
  const string = (/** @type {number} */ rva, /** @type {string} */ label) => {
    const { offset, end } = map(rva, 1, label)
    const region = bytes.subarray(offset, end)
    const nul = region.indexOf(0)
    if (nul < 0) throw invalid(`unterminated ${label}`)
    if (nul === 0) throw invalid(`empty ${label}`)
    const value = region.subarray(0, nul)
    if (value.some((byte) => byte < 0x20 || byte > 0x7e)) throw invalid(`non-ASCII ${label}`)
    return value.toString('ascii')
  }
  const symbols = (/** @type {number} */ rva) => {
    const { offset, end } = map(rva, width, 'import thunk')
    const ordinalFlag = 1n << BigInt(width * 8 - 1)
    /** @type {(string | number)[]} */
    const result = []
    for (let thunk = offset; thunk + width <= end; thunk += width) {
      const value = width === 8 ? bytes.readBigUInt64LE(thunk) : BigInt(bytes.readUInt32LE(thunk))
      if (value === 0n) return result
      if (value & ordinalFlag) {
        if (value & ~(ordinalFlag | 0xffffn)) throw invalid('reserved ordinal thunk bits')
        result.push(Number(value & 0xffffn))
      } else {
        if (value > 0xffffffffn) throw invalid('import name RVA overflow')
        const name = Number(value)
        map(name, 3, 'import hint/name')
        result.push(string(name + 2, 'import symbol'))
      }
    }
    throw invalid('unterminated import thunk table')
  }

  /** @type {PeImport[]} */
  const imports = []
  const readDirectory = (
    /** @type {number} */ index,
    /** @type {number} */ stride,
    /** @type {string} */ label,
    /** @type {(fields: number[]) => void} */ inspect,
  ) => {
    if (directoryCount <= index) return
    const rva = bytes.readUInt32LE(optional + directories + index * 8)
    const size = bytes.readUInt32LE(optional + directories + index * 8 + 4)
    if (rva === 0 && size === 0) return
    if (!rva || size < stride) throw invalid(`invalid ${label}`)
    const { offset } = map(rva, size, label)
    for (let descriptor = offset; descriptor + stride <= offset + size; descriptor += stride) {
      const fields = Array.from({ length: stride / 4 }, (_, field) => bytes.readUInt32LE(descriptor + field * 4))
      if (fields.every((field) => field === 0)) return
      inspect(fields)
    }
    throw invalid(`unterminated ${label}`)
  }
  readDirectory(1, 20, 'import directory', ([lookup, , , name, addressTable]) => {
    if (!addressTable) throw invalid('missing import address table')
    imports.push({ dll: string(name, 'DLL name'), symbols: symbols(lookup || addressTable) })
  })
  if (includeDelayImports) {
    readDirectory(13, 32, 'delay import directory', ([attributes, name, , addressTable, lookup]) => {
      // ImgDelayDescr.dlattrRva: legacy VA descriptors and unknown flags fail closed.
      if (attributes !== 1) throw invalid(`unsupported delay import attributes: 0x${attributes.toString(16)}`)
      if (!addressTable) throw invalid('missing delay import address table')
      // The delay IAT contains helper addresses, not a fallback name table.
      if (!lookup) throw invalid('missing delay import name table')
      map(addressTable, width, 'delay import address table')
      imports.push({ dll: string(name, 'DLL name'), symbols: symbols(lookup) })
    })
  }
  return imports
}

const optionalSubsystemDlls = new Set(['mapi32.dll', 'gdiplus.dll', 'ws2_32.dll', 'mfplat.dll'])

/**
 * Production forbids these DLL descriptors entirely. The test-hooks-only
 * DynComBorrowedCopyTestFixture uses MFCreateSample in com_borrowed_test_support.rs;
 * no lifecycle import, ordinal, or other MFCreate* gets that exception.
 * @param {PeImport[]} imports
 * @param {{ testHooks?: boolean }} options
 */
export function assertNoEagerWin32Imports(imports, { testHooks = false } = {}) {
  const violations = []
  for (const { dll, symbols } of imports) {
    const name = win32.basename(dll).toLowerCase()
    if (!optionalSubsystemDlls.has(name)) continue
    if (
      testHooks &&
      name === 'mfplat.dll' &&
      symbols.length &&
      symbols.every((symbol) => symbol === 'MFCreateSample')
    ) {
      continue
    }
    violations.push(
      `${dll}: ${symbols.map((symbol) => (typeof symbol === 'number' ? `#${symbol}` : symbol)).join(', ')}`,
    )
  }
  if (violations.length) {
    throw new Error(`Optional Win32 subsystem DLLs must load lazily; eager PE imports:\n${violations.join('\n')}`)
  }
}

/**
 * @param {PeImport[]} imports
 */
export function assertNoDispatcherQueueImports(imports) {
  const violations = imports.filter(
    ({ dll, symbols }) =>
      win32.basename(dll).toLowerCase() === 'coremessaging.dll' ||
      symbols.some((symbol) => typeof symbol === 'string' && /CreateDispatcherQueueController/i.test(symbol)),
  )
  if (violations.length) {
    throw new Error(
      `DispatcherQueue must resolve CoreMessaging.dll!CreateDispatcherQueueController dynamically; PE imports:\n${violations
        .map(
          ({ dll, symbols }) =>
            `${dll}: ${symbols.map((symbol) => (typeof symbol === 'number' ? `#${symbol}` : symbol)).join(', ')}`,
        )
        .join('\n')}`,
    )
  }
}

const uiHelperSymbols = new Set([
  'deleteobject',
  'destroyicon',
  'createwindowexw',
  'aredpiawarenesscontextsequal',
  'getdpiawarenesscontextforprocess',
  'setprocessdpiawarenesscontext',
  'setthreaddpiawarenesscontext',
])

/**
 * Production binaries must resolve the scoped GDI/USER32 helpers dynamically.
 * This is a direct-import guarantee, not an OS/host transitive DLL-load guarantee.
 * @param {PeImport[]} imports
 */
export function assertNoUiHelperImports(imports) {
  const violations = imports.filter(
    ({ dll, symbols }) =>
      ['gdi32.dll', 'user32.dll'].includes(win32.basename(dll).toLowerCase()) ||
      symbols.some(
        (symbol) =>
          typeof symbol === 'string' &&
          uiHelperSymbols.has(
            symbol
              .toLowerCase()
              .replace(/^(__imp_)?_?/, '')
              .replace(/@\d+$/, ''),
          ),
      ),
  )
  if (violations.length) {
    throw new Error(
      `Production GDI/USER32 helpers must resolve dynamically; PE imports:\n${violations
        .map(
          ({ dll, symbols }) =>
            `${dll}: ${symbols.map((symbol) => (typeof symbol === 'number' ? `#${symbol}` : symbol)).join(', ')}`,
        )
        .join('\n')}`,
    )
  }
}

/**
 * @param {string} directory
 * @param {(bytes: Buffer, addon: string) => void} inspect
 * @param {{ testHooksAddon?: string }} options
 */
function verifyAddonFiles(directory, inspect, { testHooksAddon } = {}) {
  const addons = readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.toLowerCase().endsWith('.node'))
    .map((entry) => entry.name)
    .sort()
  if (!addons.length) throw new Error(`No native addons found in ${directory}`)
  if (testHooksAddon !== undefined && !addons.includes(testHooksAddon)) {
    throw new Error(`Missing test-hooks addon: ${testHooksAddon}`)
  }
  for (const addon of addons) {
    try {
      inspect(readFileSync(join(directory, addon)), addon)
    } catch (error) {
      throw new Error(`${addon}: ${error instanceof Error ? error.message : error}`, { cause: error })
    }
  }
  return addons
}

/**
 * Inspect every shipped architecture without loading the addon. A caller allowing
 * a local test-hooks addon must positively identify its native fixture export.
 * @param {string} directory
 * @param {{ testHooksAddon?: string }} options
 */
export function verifyAddonImports(directory, { testHooksAddon } = {}) {
  return verifyAddonFiles(
    directory,
    (bytes, addon) => assertNoEagerWin32Imports(readPeImports(bytes), { testHooks: addon === testHooksAddon }),
    { testHooksAddon },
  )
}

/** @param {string} directory */
export function verifyDispatcherQueueImports(directory) {
  return verifyAddonFiles(directory, (bytes) =>
    assertNoDispatcherQueueImports(readPeImports(bytes, { includeDelayImports: true })),
  )
}

/** @param {string} directory */
export function verifyUiHelperImports(directory) {
  return verifyAddonFiles(directory, (bytes) =>
    assertNoUiHelperImports(readPeImports(bytes, { includeDelayImports: true })),
  )
}
