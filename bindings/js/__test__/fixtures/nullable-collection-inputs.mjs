// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'

export function checkNullableCollectionInputs(g, runtime) {
  const received = []
  let expectEmpty = false

  function receive(name, value) {
    received.push(name)
    if (expectEmpty) {
      assert.notEqual(value, null, `${name}: an empty collection is not null`)
      assert.equal(value.size, 0)
      g.releaseProjected(value)
    } else {
      assert.equal(value, null, `${name}: native input must be a null interface pointer`)
    }
  }

  function unused() {
    throw new Error('Unused complete IProbe fixture slot')
  }

  // All slots come from the checked metadata fixture. Calls traverse generated
  // outbound projection, native vtable dispatch, and the reverse-call decoder.
  const owner = g.IProbe.implement({
    takeVector: (value) => receive('vector', value),
    takeView: (value) => receive('view', value),
    takeIterable: (value) => receive('iterable', value),
    takeObservable: unused,
    takeMap: (value) => receive('map', value),
    takeKeys: (value) => receive('keys', value),
    takeMapView: (value) => receive('mapView', value),
    takeNested: unused,
    takeNestedValues: unused,
    takeNestedKeys: unused,
    takeVectorArray(value) {
      received.push('vectorArray')
      assert.deepEqual(value, [null])
    },
    takeArray: unused,
    takeStrings: unused,
    takeNumbers: unused,
    takeBytes: unused,
    getCollection: unused,
    setCollection: (value) => receive('setter', value),
    setWriteOnlyCollection: (value) => receive('writeOnlySetter', value),
    takePositions(before, value, after) {
      assert.equal(before, 23)
      assert.equal(after, 'tail')
      receive('positions', value)
    },
  })
  const nil = runtime.DynWinRtValue.nullValue()
  const wrongIid = g.NotificationData.createDefault()
  const collectionKeyMaps = Object.keys(g).filter((name) =>
    name.startsWith('IMap_WindowsFoundationCollectionsIVectorView_UInt32_'),
  )
  assert.equal(collectionKeyMaps.length, 1)
  const keyMap = g[collectionKeyMaps[0]].create([null], [17])
  const referenceMaps = Object.keys(g).filter((name) =>
    name.startsWith('IMap_String_WindowsFoundationIReference_UInt32_'),
  )
  assert.equal(referenceMaps.length, 1)
  const emptyMap = g[referenceMaps[0]].create([], [])
  const emptyView = emptyMap.getView()
  let unwrapReads = 0
  const wrappedNil = {
    get _obj() {
      unwrapReads++
      return nil
    },
  }
  try {
    const probe = owner.value
    for (const value of [null, nil, wrappedNil]) {
      probe.takeVector(value)
      probe.takeView(value)
      probe.takeIterable(value)
      probe.takeMap(value)
      probe.takeKeys(value)
      probe.takeMapView(value)
      probe.collection = value
      probe.writeOnlyCollection = value
    }
    assert.equal(unwrapReads, 8)
    probe.takeVectorArray([null])
    probe.takePositions(23, null, 'tail')
    assert.deepEqual(received, [
      ...Array.from({ length: 3 }, () => [
        'vector', 'view', 'iterable', 'map', 'keys', 'mapView', 'setter', 'writeOnlySetter',
      ]).flat(),
      'vectorArray',
      'positions',
    ])
    for (const value of [undefined, {}, { _obj: null }, { isNull: () => true }, wrongIid, wrongIid._obj]) {
      const count = received.length
      for (const method of ['takeVector', 'takeView', 'takeIterable', 'takeMap', 'takeMapView']) {
        assert.throws(() => probe[method](value), /QueryInterface|cast/)
      }
      assert.throws(() => {
        probe.collection = value
      }, /QueryInterface|cast/)
      assert.throws(() => {
        probe.writeOnlyCollection = value
      }, /QueryInterface|cast/)
      assert.equal(received.length, count, 'invalid inputs must fail before native dispatch')
    }
    assert.throws(() => probe.takeVector(), /cast/)
    assert.throws(() => probe.takeMap([]), /cast/)
    assert.throws(() => probe.takeVector(new Map()), /cast/)
    assert.equal(keyMap.lookup(null), 17)
    assert.equal(keyMap.get(null), 17)
    keyMap.set(null, 18)
    assert.equal(keyMap.get(null), 18)
    assert.equal(keyMap.has(null), true)
    for (const invalid of [undefined, {}, wrongIid, wrongIid._obj]) {
      assert.throws(() => keyMap.get(invalid), /QueryInterface|cast/)
    }
    keyMap.delete(null)
    assert.equal(keyMap.size, 0)
    assert.equal(keyMap.get(null), undefined)
    expectEmpty = true
    probe.takeVector([])
    probe.takeView([])
    probe.takeMap(new Map())
    probe.takeMapView(emptyView)
    let nonNullUnwrapReads = 0
    probe.takeMapView({
      get _obj() {
        nonNullUnwrapReads++
        return emptyView._obj
      },
    })
    assert.equal(nonNullUnwrapReads, 1)
    assert.equal(owner.takeError(), null)
    return { nativeNullInputs: 25, nullableArrayElements: 1, distinctEmptyInputs: 5, unwrapReads, nonNullUnwrapReads }
  } finally {
    g.releaseProjected(wrongIid)
    g.releaseProjected(keyMap)
    g.releaseProjected(emptyView)
    g.releaseProjected(emptyMap)
    nil.release()
    owner.dispose()
    owner.release()
  }
}
