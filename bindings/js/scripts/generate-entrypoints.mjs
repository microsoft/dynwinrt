// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageDir = fileURLToPath(new URL('..', import.meta.url))
const distDir = join(packageDir, 'dist')
const loader = readFileSync(join(distDir, 'index.js'), 'utf8')
const nativeExports = [
  ...loader.matchAll(/^module\.exports\.([A-Za-z_$][\w$]*) = nativeBinding\.\1$/gm),
].map((match) => match[1])

if (nativeExports.length === 0) {
  throw new Error('No N-API exports found in dist/index.js')
}

const comExports = new Set([
  'DynComDispatchParams',
  'DynComAllocation',
  'DynComExcepInfo',
  'DynComNativeStruct',
  'DynComNativeStructArray',
  'DynComNativeUnion',
  'DynComPropVariant',
  'DynComSafeArray',
  'DynComStatStg',
  'DynComVariant',
  'DynWinRtValue',
  'WinGuid',
  'initializeCom',
])
const comTypeAliases = ['DynWinRTValue', 'WinGUID']
const opaqueComTypes = new Set(['DynComAllocation'])
const comTypeExports = [
  ...[...comExports].filter((name) => !opaqueComTypes.has(name)),
  ...comTypeAliases,
  'DynComSafeArrayBound',
]
const opaqueComDeclarations = [
  'declare const dynComAllocationBrand: unique symbol',
  'declare const dynComImplementationBrand: unique symbol',
  'export interface DynComAllocation {',
  '  readonly [dynComAllocationBrand]: never',
  '  readonly released: boolean',
  '  release(): void',
  '}',
  'export interface DynComImplementation {',
  '  readonly [dynComImplementationBrand]: never',
  '}',
]
const comUnsafeExports = new Set([
  ...comExports,
  'DynCom',
  'DynComDispatchInvokeResult',
  'DynComInterface',
  'DynComMethodHandle',
  'DynComMethodSig',
  'DynComType',
  'DynComUnsafe',
  'DynComUnsafeInterface',
])
const comUnsafeTypeExports = [
  ...[...comUnsafeExports].filter((name) => !opaqueComTypes.has(name)),
  ...comTypeAliases,
  'DynComSafeArrayBound',
]
const comUnsafeRawExports = new Set([
  ...comUnsafeExports,
  'DynComRaw',
  'DynComRawCleanup',
  'DynComRawMemory',
  'DynComRawOwnedComPointer',
  'DynComRawPointer',
  'DynComRawStructLayout',
  'DynComRawUnionLayout',
])
const rawComTypeNames = new Set([
  'DynComRaw',
  'DynComRawCleanup',
  'DynComRawMemory',
  'DynComRawOwnedComPointer',
  'DynComRawPointer',
  'DynComRawStructLayout',
  'DynComRawUnionLayout',
])
const comUnsafeRawTypeExports = [
  ...[...comUnsafeRawExports].filter(
    (name) => !opaqueComTypes.has(name) && !rawComTypeNames.has(name),
  ),
  ...comTypeAliases,
  'DynComSafeArrayBound',
]
const rawComDeclarations = [
  'export declare class DynComRaw {',
  '  private constructor()',
  '  static pointerSize(): number',
  '}',
  'export declare class DynComRawCleanup {',
  '  private constructor()',
  '  static coTaskMemFree(pointer: DynComRawPointer): void',
  '  static localFree(pointer: DynComRawPointer): void',
  '  static globalFree(pointer: DynComRawPointer): void',
  '  static sysFreeString(pointer: DynComRawPointer): void',
  '  static safeArrayDestroy(pointer: DynComRawPointer): void',
  '  static variantClear(memory: DynComRawMemory, offset?: bigint | number | null): void',
  '  static propVariantClear(memory: DynComRawMemory, offset?: bigint | number | null): void',
  '  static releaseStgMedium(memory: DynComRawMemory, offset?: bigint | number | null): void',
  '  static closeHandle(pointer: DynComRawPointer): void',
  '  static destroyIcon(pointer: DynComRawPointer): void',
  '  static deleteObject(pointer: DynComRawPointer): void',
  '}',
  'export declare class DynComRawMemory {',
  '  private constructor()',
  '  static allocate(size: bigint | number, alignment?: bigint | number | null): DynComRawMemory',
  '  static fromUnsafeAddress(address: bigint | number, size: bigint | number, alignment: bigint | number): DynComRawMemory',
  '  static fromUnsafePointer(pointer: DynComRawPointer, size: bigint | number, alignment: bigint | number): DynComRawMemory',
  '  readonly size: bigint',
  '  readonly alignment: bigint',
  '  readonly released: boolean',
  '  release(): void',
  '  pointer(offset?: bigint | number | null): DynComRawPointer',
  '  readBytes(offset: bigint | number, length: bigint | number): Buffer',
  '  writeBytes(offset: bigint | number, value: Buffer): void',
  '  readI8(offset: bigint | number): number',
  '  writeI8(offset: bigint | number, value: number): void',
  '  readU8(offset: bigint | number): number',
  '  writeU8(offset: bigint | number, value: number): void',
  '  readI16(offset: bigint | number): number',
  '  writeI16(offset: bigint | number, value: number): void',
  '  readU16(offset: bigint | number): number',
  '  writeU16(offset: bigint | number, value: number): void',
  '  readI32(offset: bigint | number): number',
  '  writeI32(offset: bigint | number, value: number): void',
  '  readU32(offset: bigint | number): number',
  '  writeU32(offset: bigint | number, value: number): void',
  '  readI64(offset: bigint | number): bigint',
  '  writeI64(offset: bigint | number, value: bigint): void',
  '  readU64(offset: bigint | number): bigint',
  '  writeU64(offset: bigint | number, value: bigint): void',
  '  readF32(offset: bigint | number): number',
  '  writeF32(offset: bigint | number, value: number): void',
  '  readF64(offset: bigint | number): number',
  '  writeF64(offset: bigint | number, value: number): void',
  '  readIsize(offset: bigint | number): bigint',
  '  writeIsize(offset: bigint | number, value: bigint): void',
  '  readUsize(offset: bigint | number): bigint',
  '  writeUsize(offset: bigint | number, value: bigint): void',
  '  readPointer(offset: bigint | number): DynComRawPointer',
  '  writePointer(offset: bigint | number, value: DynComRawPointer): void',
  '}',
  'export declare class DynComRawPointer {',
  '  private constructor()',
  '  static fromAddress(bits: bigint | number): DynComRawPointer',
  "  static fromManagedBorrowed(value: import('./index.js').DynWinRTValue): DynComRawPointer",
  '  static null(): DynComRawPointer',
  '  readonly address: bigint',
  '  readonly isNull: boolean',
  '  offset(byteOffset: bigint | number): DynComRawPointer',
  "  toValue(): import('./index.js').DynWinRTValue",
  '}',
  'export declare class DynComRawOwnedComPointer {',
  '  private constructor()',
  "  static addRef(value: import('./index.js').DynWinRTValue): DynComRawOwnedComPointer",
  "  static queryInterface(value: import('./index.js').DynWinRTValue, iid: import('./index.js').WinGuid): DynComRawOwnedComPointer",
  "  static adoptTransferred(pointer: DynComRawPointer, iid?: import('./index.js').WinGuid | null): DynComRawOwnedComPointer",
  "  static assumeTransferred(pointer: DynComRawPointer, iid?: import('./index.js').WinGuid | null): DynComRawOwnedComPointer",
  '  readonly address: bigint',
  '  readonly released: boolean',
  '  pointer(): DynComRawPointer',
  '  retain(): DynComRawOwnedComPointer',
  "  query(iid: import('./index.js').WinGuid): DynComRawOwnedComPointer",
  '  release(): void',
  '  detach(): DynComRawPointer',
  '  transferTo(memory: DynComRawMemory, offset?: bigint | number | null): void',
  "  intoManaged(iid?: import('./index.js').WinGuid | null): import('./index.js').DynWinRTValue",
  '}',
  'export declare class DynComRawStructLayout {',
  '  private constructor()',
  '  static fromDescriptor(descriptor: string): DynComRawStructLayout',
  '  readonly qualifiedName: string',
  '  readonly descriptor: string',
  '  readonly size: bigint',
  '  readonly alignment: bigint',
  "  byValueType(): import('./index.js').DynComType",
  "  pointerType(nullable?: boolean | null): import('./index.js').DynComType",
  "  createValue(bytes?: Buffer | null): import('./index.js').DynWinRTValue",
  "  readValueBytes(value: import('./index.js').DynWinRTValue): Buffer",
  '}',
  'export declare class DynComRawUnionLayout {',
  '  private constructor()',
  '  static fromDescriptor(descriptor: string): DynComRawUnionLayout',
  '  readonly qualifiedName: string',
  '  readonly descriptor: string',
  '  readonly size: bigint',
  '  readonly alignment: bigint',
  "  pointerType(): import('./index.js').DynComType",
  "  byValueType(): import('./index.js').DynComType",
  "  createValue(activeField: string, bytes?: Buffer | null): import('./index.js').DynWinRTValue",
  "  readValueBytes(value: import('./index.js').DynWinRTValue): Buffer",
  "  assertActiveField(value: import('./index.js').DynWinRTValue, activeField: string): Buffer",
  '}',
]

writeFacade(
  'winrt',
  nativeExports.filter((name) => !name.startsWith('DynCom') && name !== 'initializeCom'),
)
writeFacade(
  'com',
  nativeExports.filter((name) => comExports.has(name)),
  comTypeExports,
  comExports,
  opaqueComDeclarations,
)
writeFacade(
  'com-unsafe',
  nativeExports.filter((name) => comUnsafeExports.has(name)),
  comUnsafeTypeExports,
  comUnsafeExports,
  opaqueComDeclarations,
)
writeFacade(
  'com-unsafe-raw',
  nativeExports.filter((name) => comUnsafeRawExports.has(name)),
  comUnsafeRawTypeExports,
  comUnsafeRawExports,
  [...opaqueComDeclarations, ...rawComDeclarations],
)

function writeFacade(
  name,
  exports,
  typeExports = exports,
  requiredExports = [],
  extraTypeDeclarations = [],
) {
  const missing = [...requiredExports].filter((value) => !exports.includes(value))
  if (missing.length > 0) {
    throw new Error(`Missing required ${name} exports: ${missing.join(', ')}`)
  }

  const js = [
    '// Generated by scripts/generate-entrypoints.mjs - do not edit',
    "'use strict'",
    "const native = require('./index.js')",
    ...exports.map((value) => `module.exports.${value} = native.${value}`),
    '',
  ].join('\n')
  const dts = [
    '// Generated by scripts/generate-entrypoints.mjs - do not edit',
    `export { ${typeExports.join(', ')} } from './index.js'`,
    ...extraTypeDeclarations,
    '',
  ].join('\n')

  writeFileSync(join(distDir, `${name}.js`), js)
  writeFileSync(join(distDir, `${name}.d.ts`), dts)
}
