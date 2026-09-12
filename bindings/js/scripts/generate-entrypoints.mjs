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
  (name) => !name.startsWith('DynCom') && !name.startsWith('DynWin32') &&
    !name.startsWith('win32') && name !== 'initializeCom',
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

const win32NativeExports = [
  'DynWin32', 'DynWin32Unsafe', 'win32CarrierKind',
  'win32NativeStructBytes', 'win32NativeStructLength',
  'win32ResourceValue', 'win32ResourceClosed', 'win32ResourceBusy',
  'win32ResourceActive', 'win32ResourceClose', 'win32Bind',
  'win32FunctionDll', 'win32FunctionEntryPoint', 'win32Invoke', 'win32InvokeWithSubsystem',
  'win32ResultReturnValue', 'win32ResultOutputs', 'win32ResultLastError', 'win32ResultSucceeded',
  'win32SubsystemName', 'win32SubsystemClosed', 'win32SubsystemClose',
  'win32OverlappedCancel', 'win32OverlappedStart',
]
const missingWin32 = win32NativeExports.filter((name) => !nativeExports.includes(name))
if (missingWin32.length) throw new Error(`Missing required Win32 exports: ${missingWin32.join(', ')}`)
const unsafeWin32Methods = [
  'initializeMapiUtilities', 'createNativeStruct', 'setNativeStructU32', 'setNativeStructBool32',
  'setNativeStructPointer', 'getNativeStructU32', 'takeNativeStructResource',
  'markNativeStructCallResult', 'prepareNativeStructCall', 'nativeStruct', 'nativeStructValue',
  'toNativeStruct',
]
const win32CarrierNames = [
  'DynWin32Value', 'DynWin32NativeStruct', 'DynWin32Resource', 'DynWin32Function',
  'DynWin32CallResult', 'DynWin32SubsystemContext', 'DynWin32OverlappedOperation',
]
const safeWin32Names = [
  'DynWin32', 'DynWin32NativeStruct', 'DynWin32OverlappedOperation',
  'DynWin32Resource', 'DynWin32SubsystemContext', 'DynWin32Value', 'DynWinRtValue',
]
const unsafeWin32Names = [
  ...safeWin32Names, 'DynWin32CallResult', 'DynWin32Function', 'DynWin32Unsafe',
]
writeFileSync(join(distDir, 'win32-internal.js'), [
  '// Generated by scripts/generate-entrypoints.mjs - do not edit',
  "'use strict'",
  "const native = require('./index.js')",
  'class DynWin32Value {',
  "  constructor() { throw new TypeError('Win32 values originate from native runtime helpers') }",
  '}',
  'class DynWin32NativeStruct {',
  "  constructor() { throw new TypeError('Use a generated native struct builder') }",
  '  get bytes() { return native.win32NativeStructBytes(this) }',
  '  get length() { return native.win32NativeStructLength(this) }',
  '}',
  'class DynWin32Resource {',
  "  constructor() { throw new TypeError('Resources originate only from native owned outputs') }",
  '  get value() { return native.win32ResourceValue(this) }',
  '  get closed() { return native.win32ResourceClosed(this) }',
  '  get busy() { return native.win32ResourceBusy(this) }',
  '  get active() { return native.win32ResourceActive(this) }',
  '  close() { return native.win32ResourceClose(this) }',
  '}',
  'class DynWin32Function {',
  "  constructor() { throw new TypeError('Use DynWin32Function.bind with an exact unsafe native descriptor') }",
  '  static bind(spec) { return project(native.win32Bind(spec)) }',
  '  get dll() { return native.win32FunctionDll(this) }',
  '  get entryPoint() { return native.win32FunctionEntryPoint(this) }',
  '  invoke(args) { return project(native.win32Invoke(this, args)) }',
  '  invokeWithSubsystem(context, subsystem, args) { return project(native.win32InvokeWithSubsystem(this, context, subsystem, args)) }',
  '}',
  'class DynWin32CallResult {',
  "  constructor() { throw new TypeError('Win32 call results originate only from native dispatch') }",
  '  get returnValue() { return project(native.win32ResultReturnValue(this)) }',
  '  get outputs() { return project(native.win32ResultOutputs(this)) }',
  '  get lastError() { return native.win32ResultLastError(this) }',
  '  get succeeded() { return native.win32ResultSucceeded(this) }',
  '}',
  'class DynWin32SubsystemContext {',
  "  constructor() { throw new TypeError('Use an explicit Win32 subsystem initializer') }",
  '  get subsystem() { return native.win32SubsystemName(this) }',
  '  get closed() { return native.win32SubsystemClosed(this) }',
  '  close() { return native.win32SubsystemClose(this) }',
  '}',
  'class DynWin32OverlappedOperation {',
  "  constructor() { throw new TypeError('Use a generated ReadFile/WriteFile wrapper') }",
  '  cancel() { return native.win32OverlappedCancel(this) }',
  '  start(callback) { return native.win32OverlappedStart(this, callback) }',
  '}',
  `const classes = [null, ${win32CarrierNames.join(', ')}]`,
  'function project(value) {',
  '  if (value === null || typeof value !== "object") return value',
  '  if (Array.isArray(value)) return value.map(project)',
  '  const kind = native.win32CarrierKind(value)',
  '  if (kind) Object.setPrototypeOf(value, classes[kind].prototype)',
  '  return value',
  '}',
  'function statics(source, omitted = []) {',
  '  class Runtime { constructor() { throw new TypeError("Win32 runtime helpers are static") } }',
  '  for (const name of Object.getOwnPropertyNames(source)) {',
  '    if (["name", "length", "prototype"].includes(name) || omitted.includes(name)) continue',
  '    const method = Object.getOwnPropertyDescriptor(source, name).value',
  '    if (typeof method === "function") Object.defineProperty(Runtime, name, {',
  '      value: (...args) => project(Reflect.apply(method, source, args)),',
  '    })',
  '  }',
  '  return Runtime',
  '}',
  `module.exports.safeDynWin32 = statics(native.DynWin32, ${JSON.stringify(unsafeWin32Methods)})`,
  'module.exports.DynWin32 = statics(native.DynWin32)',
  'module.exports.DynWin32Unsafe = statics(native.DynWin32Unsafe)',
  'module.exports.DynWinRtValue = native.DynWinRtValue',
  ...win32CarrierNames.map((name) => `module.exports.${name} = ${name}`),
  '',
].join('\n'))
const carrierMembers = [
  [],
  ['readonly bytes: Buffer', 'readonly length: number'],
  ['readonly value: bigint', 'readonly closed: boolean', 'readonly busy: boolean', 'readonly active: boolean', 'close(): void'],
  [
    'static bind(spec: DynWin32FunctionSpec): DynWin32Function',
    'readonly dll: string', 'readonly entryPoint: string',
    'invoke(args: DynWin32Value[]): DynWin32CallResult',
    'invokeWithSubsystem(context: DynWin32SubsystemContext, subsystem: string, args: DynWin32Value[]): DynWin32CallResult',
  ],
  ['readonly returnValue: DynWin32Value | null', 'readonly outputs: DynWin32Value[]', 'readonly lastError: number | null', 'readonly succeeded: boolean'],
  ['readonly subsystem: string', 'readonly closed: boolean', 'close(): void'],
  ['cancel(): void', 'start(callback: (error: Error | null, bytesTransferred?: number) => void): void'],
]
const carrierMarker = '// Win32 native carrier declarations (generated)'
const nativeDeclaration = readFileSync(join(distDir, 'index.d.ts'), 'utf8').split(carrierMarker)[0]
writeFileSync(join(distDir, 'index.d.ts'), [
  nativeDeclaration.trimEnd(), '', carrierMarker,
  ...win32CarrierNames.flatMap((name, index) => [
    `export declare class ${name} {`,
    '  private constructor()',
    `  private readonly __${name}Brand: never`,
    ...carrierMembers[index].map((member) => `  ${member}`),
    '}',
  ]),
  '',
].join('\n'))
for (const [file, names, safe] of [
  ['win32', safeWin32Names, true],
  ['win32-unsafe', unsafeWin32Names, false],
]) {
  writeFileSync(join(distDir, `${file}.js`), [
    '// Generated by scripts/generate-entrypoints.mjs - do not edit',
    "'use strict'",
    "const runtime = require('./win32-internal.js')",
    ...names.map((name) => `module.exports.${name} = runtime.${safe && name === 'DynWin32' ? 'safeDynWin32' : name}`),
    '',
  ].join('\n'))
  writeFileSync(join(distDir, `${file}.d.ts`), [
    '// Generated by scripts/generate-entrypoints.mjs - do not edit',
    `export { ${names.filter((name) => !safe || name !== 'DynWin32').join(', ')} } from './index.js'`,
    ...(safe ? [
      `export declare const DynWin32: Omit<typeof import('./index.js').DynWin32, ${unsafeWin32Methods.map((name) => JSON.stringify(name)).join(' | ')}>`,
    ] : [
      "export type { DynWin32FunctionSpec, DynWin32ParameterSpec } from './index.js'",
    ]),
    '',
  ].join('\n'))
}

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
