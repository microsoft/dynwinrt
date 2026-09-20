// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const require = createRequire(import.meta.url)
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const { DynWinRtType, DynWinRtStruct, DynWinRtValue, DynWinRtMethodSig, WinGuid, roInitialize } = require(
  resolve(process.env.DYNWINRT_JS_PACKAGE ?? packageRoot),
)
roInitialize(1)

const pointType = DynWinRtType.structType('Windows.Foundation.Point', [DynWinRtType.f32(), DynWinRtType.f32()])
const mapIid = DynWinRtType.parameterized(WinGuid.parse('3c2925fe-8519-45c1-aa79-197b6718c1c1'), [
  pointType,
  DynWinRtType.i32(),
]).iid()
const reader = DynWinRtType.registerInterface('StructEquality.PointMap', mapIid)
  .addMethod('Lookup', new DynWinRtMethodSig().addIn(pointType).addOut(DynWinRtType.i32()))
  .addMethod('get_Size', new DynWinRtMethodSig().addOut(DynWinRtType.u32()))

function own(t) {
  const values = []
  t.after(() => {
    for (const value of values.reverse()) value.release()
  })
  return (value) => {
    values.push(value)
    return value
  }
}

function point(x, y) {
  const value = DynWinRtStruct.create(pointType)
  value.setF32(0, x)
  value.setF32(1, y)
  return value.toValue()
}

test('checked Point map keys compare fields numerically', { skip: process.arch !== 'x64' }, (t) => {
  const keep = own(t)
  for (const [stored, query, equal] of [
    [[+0, 1], [-0, 1], true],
    [[-0, 1], [+0, 1], true],
    [[2, 1], [2, 1], true],
    [[2, 1], [3, 1], false],
    [[2, 1], [2, 3], false],
    [[NaN, 1], [NaN, 1], false],
    [[1, NaN], [1, NaN], false],
  ]) {
    const source = keep(point(...stored))
    const needle = keep(point(...query))
    const value = keep(DynWinRtValue.i32(7))
    const map = keep(DynWinRtValue.createMap([source], [value], pointType, DynWinRtType.i32()))
    const typed = keep(map.cast(mapIid))
    assert.equal(keep(reader.method(7).invoke(typed, [])).toNumber(), 1)
    if (equal) {
      assert.equal(keep(reader.method(6).invoke(typed, [needle])).toNumber(), 7)
    } else {
      assert.throws(() => reader.method(6).invoke(typed, [needle]))
    }
    assert.ok(Object.is(source.asStruct().getF32(0), stored[0]))
    assert.ok(Object.is(source.asStruct().getF32(1), stored[1]))
  }
})

test('Point collection admission is unchanged on non-x64 targets', { skip: process.arch === 'x64' }, (t) => {
  const keep = own(t)
  const source = keep(point(0, 1))
  const value = keep(DynWinRtValue.i32(7))
  for (const items of [[], [source]]) {
    assert.throws(() => DynWinRtValue.createVector(items, pointType))
    assert.throws(() =>
      DynWinRtValue.createMap(items, items.map(() => value), pointType, DynWinRtType.i32()),
    )
  }
})
