// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { after, before, test } from 'node:test'
import { mkdtempSync, mkdirSync, rmSync, statSync, symlinkSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { runCodegen } from '../scripts/run-codegen.mjs'
import { checkNullableCollectionInputs } from './fixtures/nullable-collection-inputs.mjs'
import { checkCollectionContracts } from './fixtures/collection-contracts.mjs'

const require = createRequire(import.meta.url)
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const runtimeRoot = resolve(process.env.DYNWINRT_JS_PACKAGE ?? packageRoot)
const winmd =
  process.env.DYNWINRT_WINDOWS_WINMD ??
  String.raw`C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`
const nullableFixture = resolve(
  packageRoot,
  '..',
  '..',
  'tools',
  'dynwinrt-codegen',
  'tests',
  'fixtures',
  'nullable_collection_inputs.winmd',
)
let output
let generated

before(() => {
  assert.ok(statSync(winmd).isFile(), 'Windows SDK metadata is required')
  assert.ok(statSync(nullableFixture).isFile(), 'Nullable collection metadata fixture is required')
  require(runtimeRoot).roInitialize(1)
  output = mkdtempSync(join(tmpdir(), 'dynwinrt-collection-factories-'))
  const generation = runCodegen(
    [
      'generate',
      '--winmd',
      `${winmd};${nullableFixture}`,
      '--class-name',
      'Windows.Storage.StorageFile,Windows.UI.Notifications.NotificationData,Windows.ApplicationModel.Contacts.ContactPicker,Windows.UI.Xaml.Data.ICollectionView,Tests.IProbe',
      '--output',
      output,
    ],
    { cwd: resolve(packageRoot, '..', '..'), encoding: 'utf8', windowsHide: true, timeout: 120_000 },
  )
  assert.equal(generation.status, 0, `${generation.error ?? ''}\n${generation.stderr}`)
  const scope = join(output, 'node_modules', '@microsoft')
  mkdirSync(scope, { recursive: true })
  symlinkSync(runtimeRoot, join(scope, 'dynwinrt'), 'junction')
  generated = require(output)
})

after(() => {
  if (output) {
    rmSync(output, { recursive: true, force: true })
    for (const suffix of ['dynwinrt-generation.lock', 'dynwinrt-lock']) {
      rmSync(join(dirname(output), `.${basename(output)}.${suffix}`), { force: true })
    }
  }
})

function own(t) {
  const values = new Set()
  const release = (value) => {
    if ('_obj' in value) generated.releaseProjected(value)
    else value.release()
    values.delete(value)
  }
  t.after(() => {
    for (const value of [...values].reverse()) release(value)
  })
  const keep = (value) => {
    if (value !== null) values.add(value)
    return value
  }
  keep.release = release
  return keep
}

function collection(prefix) {
  const names = Object.keys(generated).filter((name) => name.startsWith(prefix))
  assert.equal(names.length, 1, `Expected one metadata projection for ${prefix}: ${names}`)
  assert.equal(typeof generated[names[0]].create, 'function')
  return generated[names[0]]
}

test('generated string vector factory accepts nonempty Unicode strings', (t) => {
  const keep = own(t)
  const items = ['alpha', '\u03bb', '\ud83d\ude00', '', 'embedded\0nul']
  const vector = keep(generated.IVector_String.create(items))
  assert.deepEqual(vector.toArray(), items)
  assert.equal(vector.indexOf('\u03bb'), 1)
  const view = keep(vector.getView())
  keep.release(vector)
  assert.deepEqual(view.toArray(), items)
})

test('generated nullable collection inputs reach the native slots without optional arguments', (t) => {
  t.diagnostic(JSON.stringify(checkNullableCollectionInputs(generated, require(runtimeRoot))))
})

test('generated collection input and output contracts agree with native roundtrips', (t) => {
  t.diagnostic(JSON.stringify(checkCollectionContracts(generated, require(runtimeRoot))))
})

test('generated SDK flags preserve unsigned values through native calls and callbacks', () => {
  const unused = () => {
    throw new Error('Unused complete SDK interface slot')
  }
  let attributes = -1
  const received = []
  const owner = generated.IStorageItem.implement({
    renameAsyncOverloadDefaultOptions: unused,
    renameAsync: unused,
    deleteAsyncOverloadDefaultOptions: unused,
    deleteAsync: unused,
    getBasicPropertiesAsync: unused,
    getName: unused,
    getPath: unused,
    getAttributes: () => attributes,
    getDateCreated: unused,
    isOfType: (value) => {
      received.push(value)
      return true
    },
  })
  try {
    assert.equal(owner.value.attributes, 0xffffffff)
    for (const [input, expected] of [
      [-0x80000000, 0x80000000],
      [-1, 0xffffffff],
      [0x80000000, 0x80000000],
      [0xffffffff, 0xffffffff],
    ]) {
      assert.equal(owner.value.isOfType(input), true)
      assert.equal(received.at(-1), expected)
    }
    const before = received.length
    for (const invalid of [-0x80000001, 0x100000000, 1.5, NaN, Infinity]) {
      assert.throws(() => owner.value.isOfType(invalid))
    }
    assert.equal(received.length, before)
    attributes = -0x80000001
    assert.throws(() => owner.value.attributes)
  } finally {
    owner.dispose()
  }
})

test('generated string map factory converts both keys and values', (t) => {
  const keep = own(t)
  const keys = ['first', '\u03bb', '\ud83d\ude00']
  const values = ['alpha', '\ud83d\ude00', '\u03bb']
  const map = keep(generated.IMap_String_String.create(keys, values))
  assert.equal(map.size, keys.length)
  keys.forEach((key, index) => assert.equal(map.lookup(key), values[index]))
  assert.throws(() => generated.IMap_String_String.create(['mismatch'], []), /same length/)
})

test('generated map factory keeps the last duplicate value and first key order', (t) => {
  const keep = own(t)
  const duplicate = keep(generated.IMap_String_String.create(['same', 'same'], ['first', 'second']))
  assert.equal(duplicate.size, 1)
  assert.equal(duplicate.lookup('same'), 'second')
  duplicate.remove('same')
  assert.equal(duplicate.size, 0)
  assert.equal(duplicate.hasKey('same'), false)

  const map = keep(generated.IMap_String_String.create(['A', 'B', 'A'], ['first', 'middle', 'last']))
  const control = keep(generated.IMap_String_String.create([], []))
  assert.equal(control.insert('A', 'first'), false)
  assert.equal(control.insert('B', 'middle'), false)
  assert.equal(control.insert('A', 'last'), true)
  const view = keep(map.getView())
  const iterableNames = Object.keys(generated).filter((name) =>
    name.startsWith('IIterable_WindowsFoundationCollectionsIKeyValuePair_String_String_'),
  )
  assert.equal(iterableNames.length, 1)
  assert.equal(map.size, 2)
  for (const source of [map, control, view]) {
    assert.equal(source.lookup('A'), 'last')
    assert.equal(source.lookup('B'), 'middle')
    const iterable = keep(new generated[iterableNames[0]](source._obj))
    const iterator = keep(iterable.first())
    const entries = []
    while (iterator.hasCurrent) {
      const pair = keep(iterator.current)
      entries.push([pair.key, pair.value])
      iterator.moveNext()
    }
    assert.deepEqual(entries, [
      ['A', 'last'],
      ['B', 'middle'],
    ])
  }
  map.clear()
  assert.equal(view.size, 2)
  assert.equal(view.lookup('A'), 'last')
})

test('typed empty factories still support append and insert', (t) => {
  const keep = own(t)
  const vector = keep(generated.IVector_String.create([]))
  const map = keep(generated.IMap_String_String.create([], []))
  assert.equal(vector.size, 0)
  assert.equal(map.size, 0)
  vector.append('alpha')
  vector.append('\u03bb')
  map.insert('first', 'alpha')
  map.insert('second', '\u03bb')
  assert.deepEqual(vector.toArray(), ['alpha', '\u03bb'])
  assert.equal(map.lookup('first'), 'alpha')
  assert.equal(map.lookup('second'), '\u03bb')
})

test('generated enum vector factory uses the declared scalar projection', (t) => {
  const keep = own(t)
  const Vector = collection('IVector_WindowsApplicationModelContactsContactFieldType_')
  const items = [generated.ContactFieldType.Email, generated.ContactFieldType.PhoneNumber]
  const vector = keep(Vector.create(items))
  assert.deepEqual(vector.toArray(), items)
})

test('mixed map and observable vector factories preserve wrappers and null references', (t) => {
  const keep = own(t)
  const uri = keep(generated.Uri.createUri('https://example.invalid/collection'))
  const map = keep(generated.IMap_String_Object.create(['uri', 'null'], [uri, null]))
  const vector = keep(generated.IObservableVector_Object.create([uri, null]))
  const fromMap = keep(map.lookup('uri'))
  const fromVector = keep(vector.getAt(0))
  assert.equal(keep(generated.projectAs(fromMap, generated.Uri)).host, 'example.invalid')
  assert.equal(keep(generated.projectAs(fromVector, generated.Uri)).host, 'example.invalid')
  assert.equal(map.lookup('null'), null)
  assert.equal(vector.getAt(1), null)
  assert.equal(vector.size, 2)
})

test('runtime-class vector factories retain projected and managed inputs', (t) => {
  const keep = own(t)
  const Vector = collection('IVector_WindowsApplicationModelContactsContactDate_')
  const date = keep(generated.ContactDate.create())
  date.month = 3
  const vector = keep(Vector.create([date, date._obj]))
  keep.release(date)
  assert.equal(keep(vector.getAt(0)).month, 3)
  assert.equal(keep(vector.getAt(1)).month, 3)
})

test('runtime-class vector factory preserves a managed null carrier', (t) => {
  const keep = own(t)
  const Vector = collection('IVector_WindowsApplicationModelContactsContactDate_')
  const value = keep(require(runtimeRoot).DynWinRtValue.nullValue())
  const vector = keep(Vector.create([value]))
  assert.equal(vector.size, 1)
  assert.equal(vector.getAt(0), null)
  assert.equal(value.isNull(), true)
})

test('runtime-class conversion preserves nulls and still rejects invalid non-null inputs', (t) => {
  const keep = own(t)
  const Vector = collection('IVector_WindowsApplicationModelContactsContactDate_')
  const { DynWinRtValue } = require(runtimeRoot)
  const nil = keep(DynWinRtValue.nullValue())
  const uri = keep(generated.Uri.createUri('https://example.invalid/wrong-interface'))
  const scalar = keep(DynWinRtValue.i32(7))
  for (const invalid of [uri, uri._obj, scalar, {}, { isNull: () => true }]) {
    assert.throws(() => Vector.create([invalid]), /QueryInterface|cast/)
  }
  let reads = 0
  const wrappedNull = {
    get _obj() {
      reads++
      return nil
    },
  }
  const date = keep(generated.ContactDate.create())
  date.month = 4
  const vector = keep(Vector.create([nil, wrappedNull, null, undefined, date, date._obj]))
  assert.equal(reads, 1)
  keep.release(date)
  assert.equal(vector.size, 6)
  for (let index = 0; index < 4; index++) assert.equal(vector.getAt(index), null)
  assert.equal(keep(vector.getAt(4)).month, 4)
  assert.equal(keep(vector.getAt(5)).month, 4)
  vector.append(nil)
  assert.equal(vector.size, 7)
  assert.equal(vector.getAt(6), null)
  assert.throws(() => vector.append(uri), /QueryInterface/)
  vector.append(nil)
  assert.equal(vector.size, 8)
  assert.equal(vector.getAt(7), null)
  const retained = keep(vector.getAt(4))
  vector.replaceAll([nil, retained])
  assert.equal(vector.size, 2)
  assert.equal(vector.getAt(0), null)
  assert.equal(keep(vector.getAt(1)).month, 4)
})

test('collection-valued map inputs preserve plain and managed nulls without weakening QI', (t) => {
  const keep = own(t)
  const Map = collection('IMap_String_WindowsFoundationCollectionsIVectorView_WindowsDataTextTextSegment_')
  const nil = keep(require(runtimeRoot).DynWinRtValue.nullValue())
  const uri = keep(generated.Uri.createUri('https://example.invalid/wrong-collection'))
  for (const invalid of [uri, uri._obj, {}, { _obj: null }, { isNull: () => true }, undefined]) {
    assert.throws(() => Map.create(['invalid'], [invalid]), /QueryInterface|cast/)
    assert.throws(() => Map.create(['duplicate', 'duplicate'], [nil, invalid]), /QueryInterface|cast/)
  }
  let reads = 0
  const wrappedNull = {
    get _obj() {
      reads++
      return nil
    },
  }
  const map = keep(Map.create(['managed', 'wrapped', 'plain'], [nil, wrappedNull, null]))
  assert.equal(reads, 1)
  assert.equal(map.size, 3)
  assert.equal(map.lookup('managed'), null)
  assert.equal(map.lookup('wrapped'), null)
  assert.equal(map.lookup('plain'), null)
  assert.throws(() => map.insert('wrong', uri), /QueryInterface/)
  assert.throws(() => map.insert('undefined', undefined), /cast/)
  assert.equal(map.hasKey('wrong'), false)
  assert.equal(map.hasKey('undefined'), false)
  map.insert('later', nil)
  assert.equal(map.size, 4)
  assert.equal(map.lookup('later'), null)
  map.insert('native-null', null)
  map.set('alias-null', null)
  assert.equal(map.size, 6)
  assert.equal(map.lookup('native-null'), null)
  assert.equal(map.lookup('alias-null'), null)
  if (process.arch === 'ia32') {
    assert.throws(() => map.insert('empty', []), /struct|ABI/i)
  } else {
    map.insert('empty', [])
    const empty = keep(map.lookup('empty'))
    assert.notEqual(empty, null)
    assert.equal(empty.size, 0)
  }
})

test('nested collection arguments use supported small-struct packing', (t) => {
  const keep = own(t)
  const Map = collection('IMap_String_WindowsFoundationCollectionsIVectorView_WindowsDataTextTextSegment_')
  const segments = [
    { startPosition: 1, length: 2 },
    { startPosition: 5, length: 3 },
  ]
  if (process.arch === 'ia32') {
    assert.throws(() => Map.create(['ranges'], [segments]), /struct|ABI/i)
  } else {
    const map = keep(Map.create(['ranges'], [segments]))
    const view = keep(map.lookup('ranges'))
    assert.deepEqual(view.toArray(), segments)
    const copied = keep(Map.create(['ranges'], [view]))
    assert.deepEqual(keep(copied.lookup('ranges')).toArray(), segments)
  }
})

test('generated factories keep rejecting unsupported owned structs', () => {
  const Vector = collection('IVector_WindowsStorageSearchSortEntry_')
  assert.throws(() => Vector.create([]), /struct|ownership|HSTRING/i)
  assert.throws(
    () => Vector.create([{ propertyName: 'System.ItemNameDisplay', ascendingOrder: true }]),
    /struct|ownership|HSTRING/i,
  )
})
