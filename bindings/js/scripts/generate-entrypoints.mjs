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
  'DynComAudioFormat',
  'DynComExcepInfo',
  'DynComFormatEtc',
  'DynComNativeStruct',
  'DynComNativeStructArray',
  'DynComNativeUnion',
  'DynComOwnedHandle',
  'DynComPropVariant',
  'DynComSafeArray',
  'DynComStatStg',
  'DynComStgMedium',
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
  'DynComBorrowedCopyPlan',
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
  "  pointerType(nullable?: boolean | null): import('./index.js').DynComType",
  "  byValueType(): import('./index.js').DynComType",
  "  createValue(activeField: string, bytes?: Buffer | null): import('./index.js').DynWinRTValue",
  "  readValueBytes(value: import('./index.js').DynWinRTValue): Buffer",
  "  assertActiveField(value: import('./index.js').DynWinRTValue, activeField: string): Buffer",
  '}',
]

const winrtExports = nativeExports.filter(
  (name) => !name.startsWith('DynCom') && !name.startsWith('win32') && name !== 'initializeCom',
)
writeFacade('winrt', winrtExports, [...winrtExports, 'DynWinRtImplementationMethod'], [], [
  '/** A generated interface plan and its synchronous, metadata-ordered dispatcher. */',
  'export interface DynWinRtImplementationDescriptor {',
  "  readonly plan: import('./index.js').DynWinRtInterfacePlan",
  "  readonly dispatch: (vtableIndex: number, args: import('./index.js').DynWinRtValue[]) => import('./index.js').DynWinRtValue[]",
  '}',
  'export interface DynWinRtImplementationType { implementation(handlers: never): DynWinRtImplementationDescriptor }',
  'type ImplementationHandlers<T> = T extends { implementation(handlers: infer H): DynWinRtImplementationDescriptor } ? H : never',
  'export type DynWinRtImplementationOptions<T extends readonly DynWinRtImplementationType[]> = {',
  '  readonly interfaces: { readonly [K in keyof T]: readonly [T[K], NoInfer<ImplementationHandlers<T[K]>>] }',
  '}',
  'export declare class DynWinRtImplementationHandle<T> {',
  '  private constructor()',
  '  readonly value: T',
  "  toValue(): import('./index.js').DynWinRtValue",
  '  release(): void',
  '  disconnect(): void',
  '  dispose(): void',
  '  readonly isClosed: boolean',
  '  takeError(): string | null',
  '}',
])
writeFacade(
  'com',
  nativeExports.filter((name) => comExports.has(name)),
  comTypeExports,
  comExports,
  opaqueComDeclarations,
  true,
)
writeFacade(
  'com-unsafe',
  nativeExports.filter((name) => comUnsafeExports.has(name)),
  comUnsafeTypeExports,
  comUnsafeExports,
  opaqueComDeclarations,
  true,
)
writeFacade(
  'com-unsafe-raw',
  nativeExports.filter((name) => comUnsafeRawExports.has(name)),
  comUnsafeRawTypeExports,
  comUnsafeRawExports,
  [...opaqueComDeclarations, ...rawComDeclarations],
  true,
)

const win32NativeExports = ['win32Hkey', 'win32Close', 'win32Closed', 'win32Bind', 'win32Invoke']
const missingWin32 = win32NativeExports.filter((name) => !nativeExports.includes(name))
if (missingWin32.length) throw new Error(`Missing required Win32 exports: ${missingWin32.join(', ')}`)
writeFileSync(join(distDir, 'win32.js'), [
  '// Generated by scripts/generate-entrypoints.mjs - do not edit',
  "'use strict'",
  "const native = require('./index.js')",
  'class Win32Handle {',
  "  constructor() { throw new TypeError('Use Win32Handle.hkey(bits) for a borrowed HKEY') }",
  '  static hkey(bits) { return Object.setPrototypeOf(native.win32Hkey(bits), Win32Handle.prototype) }',
  '}',
  'class Win32Resource {',
  "  constructor() { throw new TypeError('Resources originate only from native owned outputs') }",
  '  get closed() { return native.win32Closed(this) }',
  '  close() { return native.win32Close(this) }',
  '}',
  'module.exports.Win32Handle = Win32Handle',
  'module.exports.Win32Resource = Win32Resource',
  '',
].join('\n'))
const win32Declarations = [
  'export declare class Win32Handle {',
  '  private constructor()',
  '  private readonly __win32HandleBrand: never',
  '  /** Explicit borrowed HKEY bits. Predefined HKEY constants are sign-extended. */',
  '  static hkey(bits: bigint): Win32Handle',
  '}',
  'export declare class Win32Resource {',
  '  private constructor()',
  '  private readonly __win32ResourceBrand: never',
  '  readonly closed: boolean',
  '  /** Exact RegCloseKey cleanup, once. Nonzero status retains ownership for retry. */',
  '  close(): number',
  '}',
  '',
].join('\n')
writeFileSync(join(distDir, 'win32.d.ts'), win32Declarations)
writeFileSync(join(distDir, 'win32-unsafe.js'), [
  '// Generated by scripts/generate-entrypoints.mjs - do not edit',
  "'use strict'",
  "const native = require('./index.js')",
  "const { Win32Handle, Win32Resource } = require('./win32.js')",
  'class Win32CallPlan {',
  "  constructor() { throw new TypeError('Use Win32CallPlan.bind with a complete unsafe specification') }",
  '  static bind(specification) { return Object.setPrototypeOf(native.win32Bind(specification), Win32CallPlan.prototype) }',
  '  invoke(args) {',
  '    const result = native.win32Invoke(this, args)',
  '    for (let i = 0; i < result.outputs.length; i++) {',
  "      if (result.kinds[i] === 'resource') Object.setPrototypeOf(result.outputs[i], Win32Resource.prototype)",
  "      else if (result.kinds[i] === 'handle') Object.setPrototypeOf(result.outputs[i], Win32Handle.prototype)",
  '    }',
  '    return { value: result.value, outputs: result.outputs }',
  '  }',
  '}',
  'module.exports.Win32Handle = Win32Handle',
  'module.exports.Win32Resource = Win32Resource',
  'module.exports.Win32CallPlan = Win32CallPlan',
  '',
].join('\n'))
writeFileSync(join(distDir, 'win32-unsafe.d.ts'), [
  "export { Win32Handle, Win32Resource } from './win32.js'",
  'export interface Win32CallResult { readonly value: number | bigint; readonly outputs: unknown[] }',
  '/** Unsafe: the specification asserts native ABI and ownership. It is not a safe contract extension. */',
  'export declare class Win32CallPlan {',
  '  private constructor()',
  '  private readonly __win32PlanBrand: never',
  '  static bind(specification: string): Win32CallPlan',
  '  invoke(args: unknown[]): Win32CallResult',
  '}',
  '',
].join('\n'))

writeFileSync(
  join(distDir, 'winrt-implementation.cjs'),
  readFileSync(join(packageDir, 'runtime', 'winrt-implementation.cjs'), 'utf8'),
)
writeFileSync(
  join(distDir, 'com-projection.js'),
  readFileSync(join(packageDir, 'runtime', 'com-projection.cjs'), 'utf8'),
)

function writeFacade(
  name,
  exports,
  typeExports = exports,
  requiredExports = [],
  extraTypeDeclarations = [],
  comProjection = false,
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
    ...(name === 'winrt' ? [
      "module.exports.DynWinRtImplementationHandle = require('./winrt-implementation.cjs').DynWinRtImplementationHandle",
    ] : []),
    ...(comProjection ? [
      "const projection = require('./com-projection.js')",
      'module.exports.projectAs = projection.projectAs',
      ...(name === 'com' ? [] : [
        'module.exports.__registerComProjection = projection.registerProjection',
        'module.exports.__comProjectionIid = projection.projectionIid',
        'module.exports.__activateAudioInterfaceAsync = projection.activateAudioInterfaceAsync',
      ]),
    ] : []),
    '',
  ].join('\n')
  const dts = [
    '// Generated by scripts/generate-entrypoints.mjs - do not edit',
    `export { ${typeExports.join(', ')} } from './index.js'`,
    ...extraTypeDeclarations,
    ...(comProjection ? [
      "export interface ComInterfaceType<T> { readonly IID: import('./index.js').WinGuid; _fromNative(value: unknown): T }",
      'export declare function projectAs<T>(value: unknown, type: ComInterfaceType<T>): T',
      ...(name === 'com' ? [] : [
        "export declare function __registerComProjection<T>(type: ComInterfaceType<T>, iid: import('./index.js').WinGuid): void",
        "export declare function __comProjectionIid<T>(type: ComInterfaceType<T>): import('./index.js').WinGuid",
        'export declare function __activateAudioInterfaceAsync<T>(descriptor: string, deviceInterfacePath: string, type: ComInterfaceType<T>): Promise<T>',
      ]),
    ] : []),
    '',
  ].join('\n')

  writeFileSync(join(distDir, `${name}.js`), js)
  writeFileSync(join(distDir, `${name}.d.ts`), dts)
}
