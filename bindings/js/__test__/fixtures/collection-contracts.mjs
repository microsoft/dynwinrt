// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

function ownership(g) {
  const values = new Set()
  const keep = (value) => {
    if (value !== null) values.add(value)
    return value
  }
  const release = (value) => {
    if ('_obj' in value) g.releaseProjected(value)
    else value.release()
    values.delete(value)
  }
  return { keep, release, close: () => [...values].reverse().forEach(release) }
}

function projected(g, prefix) {
  const names = Object.keys(g).filter((name) => name.startsWith(prefix))
  assert.equal(names.length, 1, `Expected one projection: ${prefix}: ${names}`)
  return g[names[0]]
}

function unused() {
  throw new Error('Unused complete metadata fixture slot')
}

const numberExpected = {
  code: 'NumberExpected',
  message: 'Failed to convert napi value String into rust type `f64`',
}

export function checkCollectionContracts(g, runtime) {
  const scope = ownership(g)
  const { keep, release } = scope
  const received = []
  function sequence(kind, value) {
    try {
      const items = []
      if (typeof value.toArray === 'function') items.push(...value.toArray())
      else {
        const iterator = keep(value.first())
        while (iterator.hasCurrent) {
          items.push(iterator.current)
          iterator.moveNext()
        }
        release(iterator)
      }
      received.push({ kind, items })
    } finally {
      g.releaseProjected(value)
    }
  }
  const owner = g.IProbe.implement({
    takeVector: (value) => sequence('vector', value),
    takeView: (value) => sequence('view', value),
    takeIterable: (value) => sequence('iterable', value),
    takeObservable: unused,
    takeMap: (value) => received.push({ kind: 'map', value: keep(value) }),
    takeKeys: (value) => received.push({ kind: 'keys', value: keep(value) }),
    takeMapView: (value) => received.push({ kind: 'mapView', value: keep(value) }),
    takeNested: unused,
    takeNestedValues: unused,
    takeNestedKeys: unused,
    takeVectorArray: (value) => received.push({ kind: 'array', items: value }),
    takeArray: (value) => received.push({ kind: 'array', items: value }),
    takeStrings: unused,
    takeNumbers: unused,
    takeBytes: unused,
    getCollection: () => null,
    setCollection: unused,
    setWriteOnlyCollection: unused,
    takePositions: unused,
  })
  try {
    const probe = owner.value
    for (const name of ['takeVector', 'takeView', 'takeIterable', 'takeArray']) {
      probe[name]([17, null])
      assert.deepEqual(received.at(-1).items, [17, null])
    }
    const nil = keep(runtime.DynWinRtValue.nullValue())
    const carrier = keep(runtime.DynWinRtValue.boxReference(runtime.DynWinRtValue.u32(23), runtime.DynWinRtType.u32()))
    const boxed = keep(g.IReference_UInt32.from(carrier))
    let reads = 0
    const wrapped = {
      get _obj() {
        reads++
        return boxed._obj
      },
    }
    for (const name of ['takeVector', 'takeView', 'takeIterable', 'takeArray']) {
      const before = reads
      probe[name]([17, null, nil, boxed, wrapped])
      assert.equal(reads, before + 1)
      assert.deepEqual(received.at(-1).items, [17, null, null, 23, 23])
    }
    probe.takeMap(
      new Map([
        ['value', 17],
        ['null', null],
        ['managed', nil],
        ['boxed', boxed],
      ]),
    )
    const map = received.at(-1).value
    assert.equal(map.lookup('value'), 17)
    assert.equal(map.get('null'), null)
    assert.equal(map.get('managed'), null)
    assert.equal(map.lookup('boxed'), 23)
    assert.equal(map.get('missing'), undefined)
    probe.takeKeys(
      new Map([
        [boxed, 23],
        [null, 17],
      ]),
    )
    const keys = received.at(-1).value
    assert.equal(keys.lookup(boxed), 23)
    assert.equal(keys.get(null), 17)
    const beforeLookup = reads
    assert.equal(keys.get(wrapped), 23)
    assert.equal(reads, beforeLookup + 1)
    assert.throws(() => keys.get('invalid'), numberExpected)
    probe.takeKeys(new Map([[17, 23]]))
    assert.equal(received.at(-1).value.size, 1)
    // Keep IReference's existing undefined convention separate from collection inputs.
    probe.takeArray([undefined])
    assert.deepEqual(received.at(-1).items, [null])

    let keyReads = 0
    let valueReads = 0
    class CountedMap extends Map {
      keys() {
        keyReads++
        return super.keys()
      }
      values() {
        valueReads++
        return super.values()
      }
    }
    for (const items of [
      [],
      [
        ['value', 17],
        ['null', null],
        ['boxed', boxed],
      ],
    ]) {
      const source = new CountedMap(items)
      probe.takeMapView(source)
      const view = received.at(-1).value
      assert.equal(view.size, items.length)
      if (items.length) {
        assert.equal(view.lookup('value'), 17)
        assert.equal(view.get('null'), null)
        assert.equal(view.lookup('boxed'), 23)
      }
      source.clear()
      source.set('later', 99)
      assert.equal(view.size, items.length)
      assert.equal(view.hasKey('later'), false)
      assert.equal(view.get('later'), undefined)
    }
    assert.equal(keyReads, 2)
    assert.equal(valueReads, 2)

    const Vector = projected(g, 'IVector_WindowsFoundationIReference_UInt32_')
    const vector = keep(Vector.create([17, null]))
    assert.deepEqual(vector.getMany(0, [null, null]), [17, null])
    assert.deepEqual(vector.toArray(), [17, null])
    assert.equal(vector.at(1), null)
    assert.equal(vector.at(2), undefined)
    const Nested = projected(g, 'IVector_WindowsFoundationCollectionsIVectorView_UInt32_')
    const Values = projected(g, 'IMap_String_WindowsFoundationCollectionsIVectorView_UInt32_')
    const nested = keep(Nested.create([null]))
    const nullableMap = keep(Values.create(['present'], [null]))
    assert.equal(nested.getAt(0), null)
    assert.equal(nested.at(-1), null)
    assert.equal(nested.at(1), undefined)
    assert.deepEqual(nested.toArray(), [null])
    assert.deepEqual([...nested], [null])
    assert.equal(nullableMap.lookup('present'), null)
    assert.equal(nullableMap.get('present'), null)
    assert.equal(nullableMap.get('missing'), undefined)
    probe.takeVectorArray([null])
    assert.deepEqual(received.at(-1).items, [null])
    assert.equal(probe.collection, null)
    assert.equal(owner.takeError(), null)

    const count = received.length
    for (const name of ['takeVector', 'takeView', 'takeIterable', 'takeMap', 'takeMapView']) {
      assert.throws(() => probe[name](undefined), /cast/)
    }
    const wrong = keep(g.NotificationData.createDefault())
    assert.throws(() => probe.takeMapView(wrong), /QueryInterface/)
    assert.throws(() => keys.get(wrong), { code: 'GenericFailure', message: /^0x80004002:/ })
    assert.throws(() => probe.takeVector(['bad']), numberExpected)
    assert.throws(() => probe.takeMap(new Map([['bad', 'bad']])), numberExpected)
    assert.throws(() => probe.takeMapView(new Map([['bad', 'bad']])), numberExpected)
    assert.equal(received.length, count, 'conversion failures must not dispatch')
    assert.equal(owner.takeError(), null)
    return { automaticBoxing: true, ownedMapViews: true, nullableReadbacks: true, reads, keyReads, valueReads }
  } finally {
    scope.close()
    owner.dispose()
    owner.release()
  }
}

export function checkNestedMapViews(g) {
  const scope = ownership(g)
  const { keep, release } = scope
  const received = []
  const owner = g.IMapViewProbe.implement({
    take: (value) => received.push(keep(value)),
    takeNested: (value) => received.push(keep(value)),
    takeVector: (value) => received.push(keep(value)),
  })
  try {
    owner.value.take(new Map([['k', 17]]))
    const view = received.at(-1)
    assert.equal(view.lookup('k'), 17)
    assert.equal(view.hasKey('absent'), false)
    const map = keep(g.IMap_String_UInt32.create(['k'], [23]))
    const snapshot = keep(map.getView())
    map.set('k', 99)
    assert.equal(snapshot.lookup('k'), 23)
    release(map)
    assert.equal(snapshot.lookup('k'), 23)
    owner.value.takeNested(
      new Map([
        ['present', snapshot],
        ['null', null],
      ]),
    )
    const outer = received.at(-1)
    release(snapshot)
    assert.equal(keep(outer.lookup('present')).lookup('k'), 23)
    assert.equal(outer.lookup('null'), null)
    assert.equal(outer.get('null'), null)
    assert.equal(outer.get('missing'), undefined)
    owner.value.takeVector([view, null])
    const vector = received.at(-1)
    release(view)
    assert.equal(keep(vector.getAt(0)).lookup('k'), 17)
    assert.equal(vector.getAt(1), null)
    const count = received.length
    assert.throws(() => owner.value.takeNested(new Map([['invalid', undefined]])), /cast/)
    assert.equal(received.length, count)
    assert.equal(owner.takeError(), null)
    return { nestedViewsRetained: true, snapshotAfterSourceRelease: true }
  } finally {
    scope.close()
    owner.dispose()
    owner.release()
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const require = createRequire(import.meta.url)
  const [generated, packageRoot] = process.argv.slice(2)
  const runtime = require(resolve(packageRoot))
  runtime.roInitialize(1)
  const g = require(resolve(generated))
  if (g.IProbe) console.log(JSON.stringify(checkCollectionContracts(g, runtime)))
  if (g.IMapViewProbe) console.log(JSON.stringify(checkNestedMapViews(g)))
}
