// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { AsyncLocalStorage } from 'node:async_hooks'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

import {
  DynWinRtArray,
  DynWinRtDelegate,
  DynWinRtDelegateMethod,
  DynWinRtImplementation,
  DynWinRtInterfacePlan,
  DynWinRtMethodSig,
  DynWinRtStruct,
  DynWinRtType,
  DynWinRtValue,
  WinGuid,
  roInitialize,
  type DynWinRtImplementationDescriptor,
  type DynWinRtImplementationMethod,
} from '../dist/winrt.js'
import * as com from '../dist/com.js'

roInitialize(1)

const stringableIid = WinGuid.parse('96369f54-8eb6-48f0-abce-c1b211e627c3')
const closableIid = WinGuid.parse('30d5a829-7fa4-4026-83bb-d75bae4ea99e')
const stringSig = new DynWinRtMethodSig().addOut(DynWinRtType.hstring())
const voidSig = new DynWinRtMethodSig()
const stringMethods = [{ name: 'ToString', vtableIndex: 6, signature: stringSig }]
const stringType = DynWinRtType.registerInterface('Windows.Foundation.IStringable', stringableIid).addMethod(
  'ToString',
  stringSig,
)
const closeType = DynWinRtType.registerInterface('Windows.Foundation.IClosable', closableIid).addMethod(
  'Close',
  voidSig,
)
const stringPlan = DynWinRtInterfacePlan.create('Windows.Foundation.IStringable', stringType, stringMethods)
const closePlan = DynWinRtInterfacePlan.create('Windows.Foundation.IClosable', closeType, [
  { name: 'Close', vtableIndex: 6, signature: voidSig },
])
const toString = stringType.method(6)
const close = closeType.method(6)
const textResult = () => [DynWinRtValue.hstring('standalone')]

function viewOf(owner: DynWinRtImplementation, iid = stringableIid) {
  const identity = owner.toValue()
  try {
    return identity.cast(iid)
  } finally {
    identity.release()
  }
}

function fixture(iid: string, methods: DynWinRtImplementationMethod[], name = 'Tests.IWinRtFixture') {
  let type = DynWinRtType.registerInterface(name, WinGuid.parse(iid))
  for (const method of methods) type = type.addMethod(method.name, method.signature)
  return { type, plan: DynWinRtInterfacePlan.create(name, type, methods) }
}

test('standalone interface plans and implementations are WinRT-root-only exports', (t) => {
  t.is(typeof DynWinRtImplementation.create, 'function')
  t.is(typeof DynWinRtInterfacePlan.create, 'function')
  t.false('DynWinRtImplementation' in com)
  t.false('DynWinRtInterfacePlan' in com)
  t.false('DynWinRtDelegateMethod' in com)
  const descriptor: DynWinRtImplementationDescriptor = {
    plan: stringPlan,
    dispatch: (_slot, _args) => textResult(),
  }
  const owner = DynWinRtImplementation.create([descriptor.plan], (_index, slot, args) =>
    descriptor.dispatch(slot, args),
  )
  const value = viewOf(owner)
  t.is(toString.getString(value), 'standalone')
  owner.dispose()
  value.release()
})

test('plans reject incomplete slots, wrong descriptor types, unsupported roots and nested arrays', (t) => {
  for (const vtableIndex of [-1, 0, 3, 7, 6.5, NaN, Infinity, 2 ** 32]) {
    t.throws(() =>
      DynWinRtInterfacePlan.create('Bad', stringType, [{ name: 'ToString', vtableIndex, signature: stringSig }]),
    )
  }
  t.throws(() => DynWinRtInterfacePlan.create('', stringType, stringMethods), { message: /name/i })
  t.throws(() => DynWinRtInterfacePlan.create('Bad', DynWinRtType.i32(), []), { message: /non-generic/i })
  t.throws(() => DynWinRtInterfacePlan.create('Bad', DynWinRtType.delegate(stringableIid), []))
  t.throws(() =>
    DynWinRtInterfacePlan.create('Bad', DynWinRtType.parameterized(stringableIid, [DynWinRtType.i32()]), []),
  )
  t.throws(
    () =>
      DynWinRtInterfacePlan.create('Bad', stringType, [
        { name: 'ToString', vtableIndex: 6, signature: DynWinRtType.i32() as unknown as DynWinRtMethodSig },
      ]),
    { message: /DynWinRtMethodSig/i },
  )
  const nested = DynWinRtType.arrayType(DynWinRtType.arrayType(DynWinRtType.i32()))
  t.throws(
    () =>
      DynWinRtInterfacePlan.create('Bad', stringType, [
        { name: 'ToString', vtableIndex: 6, signature: new DynWinRtMethodSig().addOut(nested) },
      ]),
    { message: /nested|array/i },
  )
  t.throws(() =>
    DynWinRtInterfacePlan.create('Bad', stringType, [
      { name: 'ToString', vtableIndex: 6, signature: new DynWinRtMethodSig().addOutFill(DynWinRtType.i32()) },
    ]),
  )
  t.throws(() => DynWinRtImplementation.create([], textResult), { message: /at least one/i })
  t.throws(() => DynWinRtImplementation.create([stringPlan, stringPlan], textResult), { message: /duplicate/i })
  t.throws(() => DynWinRtImplementation.create([stringType as unknown as DynWinRtInterfacePlan], textResult))
  t.throws(() => DynWinRtImplementation.create([stringPlan], textResult, 'invalid\0name'))
})

test('required interfaces are independent views and are validated together', (t) => {
  const required = DynWinRtInterfacePlan.create('RequiredStringable', stringType, stringMethods, [closableIid])
  t.throws(() => DynWinRtImplementation.create([required], textResult), { message: /requires.*separate/i })
  const duplicate = DynWinRtInterfacePlan.create('RequiredStringable', stringType, stringMethods, [
    closableIid,
    closableIid,
  ])
  t.throws(() => DynWinRtImplementation.create([duplicate, closePlan], textResult), { message: /duplicate/i })
  let closes = 0
  const owner = DynWinRtImplementation.create([required, closePlan], (index, slot, args) => {
    t.is(slot, 6)
    t.deepEqual(args, [])
    if (index === 0) return textResult()
    t.is(index, 1)
    closes++
    return []
  })
  const first = viewOf(owner)
  const second = viewOf(owner)
  const closeView = viewOf(owner, closableIid)
  t.is(toString.getString(first), 'standalone')
  t.deepEqual(close.invokeAll(closeView, []), [])
  closeView.release()
  t.is(closes, 1)
  first.release()
  t.is(toString.getString(second), 'standalone')
  owner.dispose()
  t.throws(() => toString.getString(second))
  second.release()
})

test('plans snapshot descriptors rather than observing later JS mutation', (t) => {
  const methods = [{ name: 'ToString', vtableIndex: 6, signature: stringSig }]
  const plan = DynWinRtInterfacePlan.create('ImmutableStringable', stringType, methods)
  methods[0].signature = voidSig
  methods[0].vtableIndex = 99
  methods.length = 0
  const owner = DynWinRtImplementation.create([plan], textResult)
  const value = viewOf(owner)
  t.is(toString.invoke(value, []).toString(), 'standalone')
  value.release()
  owner.dispose()
})

test('exceptions and invalid outputs fail synchronously and leave no pending JS exception', (t) => {
  let fail = true
  const owner = DynWinRtImplementation.create([stringPlan], () => {
    if (fail) throw new Error('reverse callback sentinel')
    return textResult()
  })
  const value = viewOf(owner)
  t.throws(() => toString.getString(value))
  t.regex(owner.takeError()!, /reverse callback sentinel/)
  t.is(owner.takeError(), null)
  fail = false
  t.is(toString.getString(value), 'standalone')
  owner.dispose()
  value.release()

  const invalid: Array<() => unknown> = [
    () => undefined,
    () => null,
    () => [42],
    () => [DynWinRtType.i32()],
    () => [],
    () => [DynWinRtValue.i32(42)],
    () => Promise.resolve(textResult()),
    // oxlint-disable-next-line unicorn/no-thenable -- Deliberately rejected callback result.
    () => ({ then() {} }),
    () =>
      Object.defineProperty([], '0', {
        get() {
          throw new Error('output getter sentinel')
        },
      }),
    () =>
      // oxlint-disable-next-line unicorn/no-thenable -- A throwing then getter must not leak an exception.
      Object.defineProperty([], 'then', {
        get() {
          throw new Error('then getter sentinel')
        },
      }),
  ]
  for (const callback of invalid) {
    const bad = DynWinRtImplementation.create([stringPlan], callback as () => DynWinRtValue[])
    const view = viewOf(bad)
    t.throws(() => toString.getString(view))
    t.truthy(bad.takeError())
    t.is(bad.takeError(), null)
    bad.dispose()
    view.release()
  }
})

test('async and generator dispatchers are rejected before publication', (t) => {
  t.throws(
    () => {
      // @ts-expect-error Async dispatchers are deliberately not part of the contract.
      return DynWinRtImplementation.create([stringPlan], async () => textResult())
    },
    { message: /synchronous/ },
  )
  function* generator() {
    yield textResult()
  }
  t.throws(() => DynWinRtImplementation.create([stringPlan], generator as unknown as () => DynWinRtValue[]), {
    message: /synchronous/,
  })
})

test('scalar and enum ABI aliases survive synchronous callbacks without JS numeric coercion', (t) => {
  const enumType = DynWinRtType.enumType('Tests.NodeReverseEnum', ['Answer'], [42])
  const cases: Array<{
    type: DynWinRtType
    value: DynWinRtValue
    read(value: DynWinRtValue): unknown
    expected: unknown
  }> = [
    { type: DynWinRtType.boolType(), value: DynWinRtValue.boolValue(true), read: (v) => v.toBool(), expected: true },
    { type: DynWinRtType.i8Type(), value: DynWinRtValue.i8Value(-12), read: (v) => v.toNumber(), expected: -12 },
    { type: DynWinRtType.u8(), value: DynWinRtValue.u8Value(250), read: (v) => v.toNumber(), expected: 250 },
    { type: DynWinRtType.i16(), value: DynWinRtValue.i16(-1234), read: (v) => v.toNumber(), expected: -1234 },
    { type: DynWinRtType.u16(), value: DynWinRtValue.u16(65535), read: (v) => v.toNumber(), expected: 65535 },
    { type: DynWinRtType.char16(), value: DynWinRtValue.u16(0xd800), read: (v) => v.toNumber(), expected: 0xd800 },
    {
      type: DynWinRtType.i32(),
      value: DynWinRtValue.i32(-2147483648),
      read: (v) => v.toNumber(),
      expected: -2147483648,
    },
    { type: DynWinRtType.u32(), value: DynWinRtValue.u32(4294967295), read: (v) => v.toNumber(), expected: 4294967295 },
    {
      type: DynWinRtType.hresult(),
      value: DynWinRtValue.i32(0x80004005 | 0),
      read: (v) => v.toNumber(),
      expected: 0x80004005 | 0,
    },
    {
      type: DynWinRtType.i64(),
      value: DynWinRtValue.i64(-9223372036854775808n),
      read: (v) => v.toI64Bigint(),
      expected: -9223372036854775808n,
    },
    {
      type: DynWinRtType.u64(),
      value: DynWinRtValue.u64(18446744073709551615n),
      read: (v) => v.toU64Bigint(),
      expected: 18446744073709551615n,
    },
    { type: DynWinRtType.f32(), value: DynWinRtValue.f32(1.25), read: (v) => v.toF64(), expected: 1.25 },
    { type: DynWinRtType.f64(), value: DynWinRtValue.f64(-9.5), read: (v) => v.toF64(), expected: -9.5 },
    { type: enumType, value: DynWinRtValue.enumValue(enumType, 42), read: (v) => v.getEnumInt(), expected: 42 },
  ]
  const methods = cases.map(({ type }, index) => ({
    name: `Echo${index}`,
    vtableIndex: index + 6,
    signature: new DynWinRtMethodSig().addIn(type).addOut(type),
  }))
  const { type, plan } = fixture('2799ac23-bcd8-4086-b6f0-1f33b3af84be', methods, 'Tests.IWinRtScalars')
  const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
    t.is(cases[slot - 6].read(args[0]), cases[slot - 6].expected)
    return [args[0]]
  })
  const view = viewOf(owner, type.iid())
  cases.forEach(({ value, read, expected }, index) => {
    t.is(read(type.method(index + 6).invoke(view, [value])), expected)
  })
  owner.dispose()
  view.release()
})

test('recursive structs and arrays retain their HSTRING and interface fields', (t) => {
  const innerType = DynWinRtType.structType('Tests.NodeReverseInner', [
    DynWinRtType.hstring(),
    DynWinRtType.interface(stringableIid),
  ])
  const outerType = DynWinRtType.structType('Tests.NodeReverseOuter', [innerType, DynWinRtType.i32()])
  const arrayType = DynWinRtType.arrayType(outerType)
  const { type, plan } = fixture(
    '6968bfa2-e195-4d07-9726-3442ee1d1133',
    [{ name: 'Copy', vtableIndex: 6, signature: new DynWinRtMethodSig().addIn(arrayType).addOut(arrayType) }],
    'Tests.IWinRtStructArrays',
  )
  const referenced = DynWinRtImplementation.create([stringPlan], textResult)
  const object = viewOf(referenced)
  const inner = DynWinRtStruct.create(innerType)
  inner.setHstring(0, 'nested string')
  inner.setObject(1, object)
  const outer = DynWinRtStruct.create(outerType)
  outer.setStruct(0, inner)
  outer.setI32(1, 19)
  const owner = DynWinRtImplementation.create([plan], (_index, _slot, args) => [args[0]])
  const view = viewOf(owner, type.iid())
  const input = DynWinRtArray.fromObjectValues([outer.toValue()], outerType).toValue()
  const result = type.method(6).invoke(view, [input])
  input.release()
  const copied = result.asArray().get(0).asStruct()
  const copiedInner = copied.getStruct(0)
  t.is(copied.getI32(1), 19)
  t.is(copiedInner.getHstring(0), 'nested string')
  const copiedObject = copiedInner.getObject(1)
  t.is(toString.getString(copiedObject), 'standalone')
  copiedObject.release()
  result.release()
  owner.dispose()
  view.release()
  referenced.dispose()
  object.release()
})

test('managed WinRT reference categories are nullable and closed generic values stay owned', (t) => {
  const referenceType = DynWinRtType.parameterized(WinGuid.parse('61c17706-2d65-11e0-9ae8-d48564015472'), [
    DynWinRtType.i32(),
  ])
  const types = [
    DynWinRtType.object(),
    DynWinRtType.interface(stringableIid),
    DynWinRtType.runtimeClass('Tests.NodeReverseRuntimeClass', stringType),
    DynWinRtType.delegate(WinGuid.parse('de77ae11-536d-4f38-9354-22f9414a82ec')),
    DynWinRtType.iAsyncAction(),
    DynWinRtType.iAsyncOperation(DynWinRtType.i32()),
    DynWinRtType.iAsyncActionWithProgress(DynWinRtType.u32()),
    DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.i32(), DynWinRtType.u32()),
    referenceType,
  ]
  const { type, plan } = fixture(
    'f8e75843-5f9b-4af9-b3ea-d123e9c841a0',
    types.map((item, index) => ({
      name: `Echo${index}`,
      vtableIndex: index + 6,
      signature: new DynWinRtMethodSig().addIn(item).addOut(item),
    })),
    'Tests.IWinRtReferences',
  )
  const owner = DynWinRtImplementation.create([plan], (_index, _slot, args) => [args[0]])
  const view = viewOf(owner, type.iid())
  types.forEach((_item, index) => {
    t.true(
      type
        .method(index + 6)
        .invoke(view, [DynWinRtValue.nullValue()])
        .isNull(),
    )
  })
  const boxed = DynWinRtValue.boxReference(DynWinRtValue.i32(123), DynWinRtType.i32())
  const result = type.method(types.length + 5).invoke(view, [boxed])
  boxed.release()
  const getter = DynWinRtType.registerInterface('Tests.IReferenceI32', referenceType.iid())
    .addMethod('get_Value', new DynWinRtMethodSig().addOut(DynWinRtType.i32()))
    .method(6)
  const reference = result.cast(referenceType.iid())
  result.release()
  t.is(getter.getI32(reference), 123)
  reference.release()
  owner.dispose()
  view.release()
})

test('synchronous dispatch preserves its Node async context', (t) => {
  const storage = new AsyncLocalStorage<string>()
  const owner = storage.run('creation context', () =>
    DynWinRtImplementation.create([stringPlan], () => [DynWinRtValue.hstring(storage.getStore() ?? 'missing')]),
  )
  const value = viewOf(owner)
  t.is(
    storage.run('caller context', () => toString.getString(value)),
    'creation context',
  )
  value.release()
  owner.dispose()
})

test('metadata-described delegate callers use slot 3 and reject invalid targets and arguments', (t) => {
  const iid = WinGuid.parse('74ca6c7f-502c-4b4b-bb77-7c2025a73fc8')
  const signature = new DynWinRtMethodSig().addIn(DynWinRtType.i32())
  const method = DynWinRtDelegateMethod.create(iid, signature)
  const received: number[] = []
  const delegate = DynWinRtDelegate.create(iid, [DynWinRtType.i32()], (value) => {
    received.push(value.toNumber())
  })
  const value = delegate.toValue()
  t.deepEqual(method.invoke(value, [DynWinRtValue.i32(41)]), [])
  t.deepEqual(received, [41])
  t.throws(() => method.invoke(value, []), { message: /count/i })
  t.throws(() => method.invoke(value, [DynWinRtValue.hstring('wrong')]))
  t.throws(() => method.invoke(value, [DynWinRtType.i32() as unknown as DynWinRtValue]))
  t.throws(() => method.invoke(DynWinRtValue.nullValue(), [DynWinRtValue.i32(1)]))
  t.throws(() => DynWinRtDelegateMethod.create(WinGuid.parse('00000000-0000-0000-c000-000000000046'), signature), {
    message: /infrastructure/,
  })
  t.throws(() => DynWinRtDelegateMethod.create(iid, DynWinRtType.i32() as unknown as DynWinRtMethodSig))
  const unsupported = DynWinRtDelegateMethod.create(WinGuid.parse('22bf990d-2be0-4239-8c44-c3959b1b6011'), signature)
  t.throws(() => unsupported.invoke(value, [DynWinRtValue.i32(1)]), { message: /QueryInterface/ })
  t.deepEqual(received, [41])
  t.deepEqual(method.invoke(value, [DynWinRtValue.i32(42)]), [])
  value.release()
  t.throws(() => method.invoke(value, [DynWinRtValue.i32(43)]), { message: /live managed Object/ })
})

test('delegate owner release is idempotent and preserves independently retained native values', (t) => {
  const iid = WinGuid.parse('312a69d6-444a-4d3a-ab56-f7e85b23e654')
  const signature = new DynWinRtMethodSig().addIn(DynWinRtType.i32())
  let received = 0
  const owner = DynWinRtDelegate.create(iid, [DynWinRtType.i32()], (value) => {
    received = value.toNumber()
  })
  const retained = owner.toValue()
  owner.release()
  owner.release()
  t.throws(() => owner.toValue(), { message: /released/ })
  t.deepEqual(retained.invokeDelegate(iid, signature, [DynWinRtValue.i32(42)]), [])
  t.is(received, 42)
  retained.release()
})

for (const mode of ['remove', 'once', 'add-failure']) {
  test.serial(`generated event ownership exits naturally after ${mode}`, (t) => {
    const child = spawnSync(process.execPath, [
      fileURLToPath(new URL('./winrt-event-lifetime-child.mjs', import.meta.url)), mode,
    ], { encoding: 'utf8', timeout: 10_000, windowsHide: true })
    t.is(child.status, 0, `${child.error ?? ''}\n${child.stdout}\n${child.stderr}`)
    t.regex(child.stdout, new RegExp(`generated-event-lifetime-ok:${mode}`))
  })
}

for (const api of ['prepared', 'value'] as const) {
  test(`delegate ${api} callers pin managed values through nested invocation and reentrant release`, (t) => {
    const iid = WinGuid.parse('00c1b0c2-2f1c-4e9f-a167-373bc6149513')
    const signature = new DynWinRtMethodSig().addIn(DynWinRtType.i32())
    const method = DynWinRtDelegateMethod.create(iid, signature)
    const invoke = (value: DynWinRtValue, args: DynWinRtValue[]) =>
      api === 'prepared' ? method.invoke(value, args) : value.invokeDelegate(iid, signature, args)
    let value: DynWinRtValue
    const received: number[] = []
    const delegate = DynWinRtDelegate.create(iid, [DynWinRtType.i32()], (arg) => {
      const number = arg.toNumber()
      received.push(number)
      if (number === 1) {
        t.deepEqual(invoke(value, [DynWinRtValue.i32(2)]), [])
      } else {
        value.release()
      }
    })
    value = delegate.toValue()
    t.deepEqual(invoke(value, [DynWinRtValue.i32(1)]), [])
    t.deepEqual(received, [1, 2])
    t.true(value.isNull())
    t.throws(() => invoke(value, [DynWinRtValue.i32(3)]), { message: /live managed Object/ })
  })
}

test('release, disconnect and dispose have distinct idempotent lifetime semantics', (t) => {
  const owner = DynWinRtImplementation.create([stringPlan], textResult)
  const value = viewOf(owner)
  owner.release()
  owner.release()
  t.false(owner.isClosed)
  t.throws(() => owner.toValue(), { message: /released/i })
  t.is(toString.getString(value), 'standalone')
  owner.disconnect()
  t.true(owner.isClosed)
  t.throws(() => toString.getString(value))
  owner.dispose()
  owner.dispose()
  value.release()

  const unique = DynWinRtImplementation.create([stringPlan], textResult)
  unique.release()
  t.true(unique.isClosed)
})

test('reentrant calls, owner release and disposal do not deadlock or destroy active callback roots', (t) => {
  let depth = 0
  let view: DynWinRtValue
  const owner = DynWinRtImplementation.create([stringPlan], () => {
    if (depth++ === 0) {
      const inner = toString.getString(view)
      owner.dispose()
      return [DynWinRtValue.hstring(`outer:${inner}`)]
    }
    owner.release()
    return [DynWinRtValue.hstring('inner')]
  })
  view = viewOf(owner)
  t.is(toString.getString(view), 'outer:inner')
  t.true(owner.isClosed)
  t.throws(() => toString.getString(view))
  view.release()
})

test('custom property, event, GUID, struct and multiple-output signatures cross actual vtables', (t) => {
  const tokenType = DynWinRtType.structType('Windows.Foundation.EventRegistrationToken', [DynWinRtType.i64()])
  const eventIid = WinGuid.parse('ac0a9714-89f6-4e72-9c44-ac29b89b671c')
  const pointType = DynWinRtType.structType('Tests.NodeReversePoint', [DynWinRtType.f64(), DynWinRtType.f64()])
  const methods = [
    { name: 'get_Value', vtableIndex: 6, signature: new DynWinRtMethodSig().addOut(DynWinRtType.i32()) },
    { name: 'put_Value', vtableIndex: 7, signature: new DynWinRtMethodSig().addIn(DynWinRtType.i32()) },
    {
      name: 'add_Changed',
      vtableIndex: 8,
      signature: new DynWinRtMethodSig().addIn(DynWinRtType.delegate(eventIid)).addOut(tokenType),
    },
    { name: 'remove_Changed', vtableIndex: 9, signature: new DynWinRtMethodSig().addIn(tokenType) },
    {
      name: 'Transform',
      vtableIndex: 10,
      signature: new DynWinRtMethodSig()
        .addIn(pointType)
        .addIn(DynWinRtType.guidType())
        .addOut(pointType)
        .addOut(DynWinRtType.guidType())
        .addOut(DynWinRtType.boolType()),
    },
  ]
  const { type, plan } = fixture('281144a6-d56b-4cbf-abcc-87bbd1fcf5e3', methods)
  const eventSignature = new DynWinRtMethodSig().addIn(DynWinRtType.i32())
  let current = 5
  let handler: DynWinRtValue | undefined
  const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
    if (slot === 6) return [DynWinRtValue.i32(current)]
    if (slot === 7) {
      current = args[0].toNumber()
      if (handler) handler.invokeDelegate(eventIid, eventSignature, [DynWinRtValue.i32(current)])
      return []
    }
    if (slot === 8) {
      handler = args[0].cast(eventIid)
      const token = DynWinRtStruct.create(tokenType)
      token.setI64(0, 123n)
      return [token.toValue()]
    }
    if (slot === 9) {
      t.is(args[0].asStruct().getI64(0), 123n)
      handler!.release()
      handler = undefined
      return []
    }
    t.is(slot, 10)
    const point = args[0].asStruct()
    const result = DynWinRtStruct.create(pointType)
    result.setF64(0, point.getF64(0) + 1)
    result.setF64(1, point.getF64(1) + 2)
    return [result.toValue(), args[1], DynWinRtValue.boolValue(true)]
  })
  const value = viewOf(owner, type.iid())
  type.method(7).setI32(value, 17)
  t.is(type.method(6).getI32(value), 17)
  const changes: number[] = []
  const delegate = DynWinRtDelegate.create(eventIid, [DynWinRtType.i32()], (value) => {
    changes.push(value.toNumber())
  })
  const delegateValue = delegate.toValue()
  const token = type.method(8).invoke(value, [delegateValue])
  t.truthy(handler)
  type.method(7).setI32(value, 18)
  t.deepEqual(changes, [18])
  type.method(9).invoke(value, [token])
  t.is(handler, undefined)
  type.method(7).setI32(value, 19)
  t.deepEqual(changes, [18])
  const point = DynWinRtStruct.create(pointType)
  point.setF64(0, 3)
  point.setF64(1, 9)
  const outputs = type.method(10).invokeAll(value, [point.toValue(), DynWinRtValue.guid(stringableIid)])
  t.is(outputs[0].asStruct().getF64(0), 4)
  t.is(outputs[0].asStruct().getF64(1), 11)
  t.is(outputs[1].toGuid().toString(), stringableIid.toString())
  t.true(outputs[2].toBool())
  delegateValue.release()
  value.release()
  owner.dispose()
})

test('pass, receive and fill arrays preserve argument order, capacities and reference elements', (t) => {
  const i32Array = DynWinRtType.arrayType(DynWinRtType.i32())
  const stringArray = DynWinRtType.arrayType(DynWinRtType.hstring())
  const objectArray = DynWinRtType.arrayType(DynWinRtType.interface(stringableIid))
  const methods = [
    { name: 'Pass', vtableIndex: 6, signature: new DynWinRtMethodSig().addIn(i32Array).addOut(i32Array) },
    { name: 'Receive', vtableIndex: 7, signature: new DynWinRtMethodSig().addOut(stringArray) },
    {
      name: 'Fill',
      vtableIndex: 8,
      signature: new DynWinRtMethodSig()
        .addIn(DynWinRtType.i32())
        .addOutFill(i32Array)
        .addIn(DynWinRtType.i32())
        .addOut(DynWinRtType.i32()),
    },
    { name: 'References', vtableIndex: 9, signature: new DynWinRtMethodSig().addIn(objectArray).addOut(objectArray) },
  ]
  const { type, plan } = fixture('92735c9b-4c5b-425a-9ff3-2b01e9927556', methods, 'Tests.IWinRtArrays')
  let invalidFill = false
  const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
    if (slot === 6)
      return [
        DynWinRtArray.fromI32Values(
          args[0]
            .asArray()
            .toI32Vec()
            .map((v) => v * 2),
        ).toValue(),
      ]
    if (slot === 7) return [DynWinRtArray.fromStringValues(['', 'native', '🌍']).toValue()]
    if (slot === 9) return [args[0]]
    t.deepEqual(
      args.map((arg) => arg.toNumber()),
      [10, 3, 20],
    )
    return [DynWinRtArray.fromI32Values(invalidFill ? [7] : [7, 8, 9]).toValue(), DynWinRtValue.i32(30)]
  })
  const value = viewOf(owner, type.iid())
  t.deepEqual(
    type
      .method(6)
      .invoke(value, [DynWinRtArray.fromI32Values([1, -2]).toValue()])
      .asArray()
      .toI32Vec(),
    [2, -4],
  )
  t.deepEqual(
    type
      .method(6)
      .invoke(value, [DynWinRtArray.fromI32Values([]).toValue()])
      .asArray()
      .toI32Vec(),
    [],
  )
  t.deepEqual(type.method(7).invoke(value, []).asArray().toStringVec(), ['', 'native', '🌍'])
  const fill = DynWinRtArray.fromI32Values([0, 0, 0]).toValue()
  const fillArgs = [DynWinRtValue.i32(10), fill, DynWinRtValue.i32(20)]
  const outputs = type.method(8).invokeAll(value, fillArgs)
  t.deepEqual(outputs[0].asArray().toI32Vec(), [7, 8, 9])
  t.is(outputs[1].toNumber(), 30)
  invalidFill = true
  t.throws(() => type.method(8).invokeAll(value, fillArgs))
  t.regex(owner.takeError()!, /capacity|length|fill/i)
  const referenceOwner = DynWinRtImplementation.create([stringPlan], textResult)
  const reference = viewOf(referenceOwner)
  const inputs = DynWinRtArray.fromObjectValues(
    [reference, DynWinRtValue.nullValue()],
    DynWinRtType.interface(stringableIid),
  )
  const references = type.method(9).invoke(value, [inputs.toValue()]).asArray().toValues()
  t.is(toString.getString(references[0]), 'standalone')
  t.true(references[1].isNull())
  referenceOwner.dispose()
  reference.release()
  references.forEach((item) => item.release())
  owner.dispose()
  value.release()
})

test('semantic HRESULT values and arrays round-trip through pass, receive and fill contracts', (t) => {
  const codes = [0, 1, 0x80004005 | 0, -2147483648, 2147483647]
  const elementType = DynWinRtType.hresult()
  const arrayType = DynWinRtType.arrayType(elementType)
  const { type, plan } = fixture(
    '9dd1d5b0-e3f4-4c79-a9c5-3f182d570d7b',
    [
      { name: 'Pass', vtableIndex: 6, signature: new DynWinRtMethodSig().addIn(arrayType).addOut(arrayType) },
      { name: 'Receive', vtableIndex: 7, signature: new DynWinRtMethodSig().addOut(arrayType) },
      { name: 'Fill', vtableIndex: 8, signature: new DynWinRtMethodSig().addOutFill(arrayType) },
      { name: 'Echo', vtableIndex: 9, signature: new DynWinRtMethodSig().addIn(elementType).addOut(elementType) },
      { name: 'Consume', vtableIndex: 10, signature: new DynWinRtMethodSig().addIn(elementType) },
    ],
    'Tests.IWinRtHResultArrays',
  )
  const consumed: number[] = []
  const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
    if (slot === 10) {
      consumed.push(args[0].toNumber())
      return []
    }
    if (slot === 6 || slot === 9) return [args[0]]
    if (slot === 8) t.is(args[0].toNumber(), codes.length)
    return [DynWinRtArray.fromHresultValues(codes).toValue()]
  })
  const view = viewOf(owner, type.iid())
  const input = DynWinRtArray.fromHresultValues(codes).toValue()
  t.deepEqual(type.method(6).invoke(view, [input]).asArray().toI32Vec(), codes)
  const genericInput = DynWinRtArray.fromObjectValues(
    codes.map((code) => DynWinRtValue.fromHResult(code)),
    elementType,
  ).toValue()
  t.deepEqual(type.method(6).invoke(view, [genericInput]).asArray().toI32Vec(), codes)
  t.deepEqual(type.method(7).invoke(view, []).asArray().toI32Vec(), codes)
  const fill = DynWinRtArray.fromHresultValues(codes.map(() => 0)).toValue()
  t.deepEqual(type.method(8).invoke(view, [fill]).asArray().toI32Vec(), codes)
  for (const code of codes) {
    t.is(
      type
        .method(9)
        .invoke(view, [DynWinRtValue.hresult(code)])
        .toNumber(),
      code,
    )
    t.deepEqual(type.method(10).invokeAll(view, [DynWinRtValue.fromHResult(code)]), [])
  }
  t.deepEqual(consumed, codes)
  t.deepEqual(DynWinRtArray.fromHresultValues([]).toI32Vec(), [])
  for (const invalid of [NaN, Infinity, 0.5, -2147483649, 2147483648]) {
    t.throws(() => DynWinRtValue.hresult(invalid))
    t.throws(() => DynWinRtValue.fromHResult(invalid))
    t.throws(() => DynWinRtArray.fromHresultValues([0, invalid]))
  }
  input.release()
  genericInput.release()
  fill.release()
  view.release()
  owner.dispose()
})

test('non-null async elements stay awaitable after pass, receive and fill array calls', async (t) => {
  const activationIid = WinGuid.parse('00000035-0000-0000-c000-000000000046')
  const outputIid = WinGuid.parse('905a0fe6-bc53-11df-8c49-001e4fc686da')
  const asyncType = DynWinRtType.iAsyncOperation(DynWinRtType.boolType())
  const arrayType = DynWinRtType.arrayType(asyncType)
  const activation = DynWinRtType.registerInterface('IActivationFactory', activationIid)
    .addMethod('ActivateInstance', new DynWinRtMethodSig().addOut(DynWinRtType.object()))
    .method(6)
  const outputType = DynWinRtType.registerInterface('Windows.Storage.Streams.IOutputStream', outputIid)
    .addMethod(
      'WriteAsync',
      new DynWinRtMethodSig()
        .addIn(DynWinRtType.interface(WinGuid.parse('905a0fe0-bc53-11df-8c49-001e4fc686da')))
        .addOut(DynWinRtType.iAsyncOperationWithProgress(DynWinRtType.u32(), DynWinRtType.u32())),
    )
    .addMethod('FlushAsync', new DynWinRtMethodSig().addOut(asyncType))
  const { type, plan } = fixture(
    'ebdb96e6-790a-4b55-927c-eb6e570471b5',
    [
      { name: 'Pass', vtableIndex: 6, signature: new DynWinRtMethodSig().addIn(arrayType).addOut(arrayType) },
      { name: 'Receive', vtableIndex: 7, signature: new DynWinRtMethodSig().addOut(arrayType) },
      { name: 'Fill', vtableIndex: 8, signature: new DynWinRtMethodSig().addOutFill(arrayType) },
    ],
    'Tests.IWinRtAsyncArrays',
  )
  const rawFactory = DynWinRtValue.activationFactory('Windows.Storage.Streams.InMemoryRandomAccessStream')
  let factory: DynWinRtValue
  try {
    factory = rawFactory.cast(activationIid)
  } finally {
    rawFactory.release()
  }
  let stream: DynWinRtValue
  try {
    stream = activation.invoke(factory, [])
  } finally {
    factory.release()
  }
  let output: DynWinRtValue
  try {
    output = stream.cast(outputIid)
  } finally {
    stream.release()
  }
  const makeArray = () => {
    const operation = outputType.method(7).invoke(output, [])
    try {
      return DynWinRtArray.fromObjectValues([operation], asyncType).toValue()
    } finally {
      operation.release()
    }
  }
  const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
    if (slot === 6) return [args[0]]
    if (slot === 8) t.is(args[0].toNumber(), 1)
    return [makeArray()]
  })
  const view = viewOf(owner, type.iid())
  try {
    for (const slot of [6, 7, 8]) {
      const input =
        slot === 6
          ? makeArray()
          : slot === 8
            ? DynWinRtArray.fromObjectValues([DynWinRtValue.nullValue()], asyncType).toValue()
            : undefined
      let result: DynWinRtValue
      try {
        result = type.method(slot).invoke(view, input ? [input] : [])
      } finally {
        input?.release()
      }
      const element = result.asArray().get(0)
      result.release()
      try {
        t.true((await element.toPromise()).toBool())
      } finally {
        element.release()
      }
    }
  } finally {
    view.release()
    owner.dispose()
    const closable = output.cast(closableIid)
    try {
      close.invokeAll(closable, [])
    } finally {
      closable.release()
      output.release()
    }
  }
})

interface NativeHooks {
  winrtImplementationTestBackgroundInstance(): DynWinRtValue
  winrtImplementationTestBackgroundProgress(value: DynWinRtValue): number
}
const hooks = createRequire(import.meta.url)('../dist/index.js') as Partial<NativeHooks>

if (!hooks.winrtImplementationTestBackgroundInstance) {
  test.skip('native retention and SDK fixtures require the test-hooks Cargo feature', () => {})
} else {
  test('IBackgroundTask receives and can retain a complete native IBackgroundTaskInstance fixture', (t) => {
    const taskIid = WinGuid.parse('7d13d534-fd12-43ce-8c22-ea1ff13c06df')
    const instanceIid = WinGuid.parse('865bda7a-21d8-4573-8f32-928a1b0641f6')
    const signature = new DynWinRtMethodSig().addIn(DynWinRtType.interface(instanceIid))
    const taskType = DynWinRtType.registerInterface(
      'Windows.ApplicationModel.Background.IBackgroundTask',
      taskIid,
    ).addMethod('Run', signature)
    const plan = DynWinRtInterfacePlan.create('Windows.ApplicationModel.Background.IBackgroundTask', taskType, [
      { name: 'Run', vtableIndex: 6, signature },
    ])
    let retained: DynWinRtValue | undefined
    const owner = DynWinRtImplementation.create([plan], (_index, slot, args) => {
      t.is(slot, 6)
      retained = args[0]
      t.is(hooks.winrtImplementationTestBackgroundProgress!(retained), 42)
      return []
    })
    const value = viewOf(owner, taskIid)
    const instance = hooks.winrtImplementationTestBackgroundInstance!()
    t.deepEqual(taskType.method(6).invokeAll(value, [instance]), [])
    instance.release()
    t.is(hooks.winrtImplementationTestBackgroundProgress!(retained!), 42)
    retained!.release()
    owner.dispose()
    value.release()
  })

  for (const mode of [
    'retention',
    'cycle',
    'reentrant-release',
    'delegate-release',
    'delegate-value-release',
    'shutdown',
    'natural-exit',
  ]) {
    test.serial(`native callback lifecycle: ${mode}`, (t) => {
      const child = spawnSync(
        process.execPath,
        ['--expose-gc', fileURLToPath(new URL('./winrt-implementation-child.mjs', import.meta.url)), mode],
        {
          env: { ...process.env, DYNWINRT_TEST_RUNTIME: fileURLToPath(new URL('../dist/index.js', import.meta.url)) },
          encoding: 'utf8',
          timeout: 20_000,
          windowsHide: true,
        },
      )
      t.is(child.status, 0, `${child.error ?? ''}\n${child.stdout}\n${child.stderr}`)
      t.regex(child.stdout, new RegExp(`winrt-${mode}-ok`))
    })
  }
}
