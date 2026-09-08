// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'

const native = createRequire(import.meta.url)('../dist/index.js')
const registry = JSON.parse(
  readFileSync(new URL('../../../crates/dynwinrt-com-contracts/src/registry.json', import.meta.url), 'utf8'),
)
const descriptor = (id: string) => structuredClone(registry.copies.find((copy: { id: string }) => copy.id === id))
const prepare = (id: string) => native.DynComBorrowedCopyPlan.prepare(JSON.stringify(descriptor(id)))
const events = (fixture: { events(): string[] }, prefix: string) =>
  fixture.events().filter((event) => event.startsWith(prefix)).length

test('borrowed plan rejects old, forged and ill-typed descriptors', (t) => {
  for (const edit of [
    (d: any) => (d.version = 1),
    (d: any) => (d.id = 'unreviewed'),
    (d: any) => (d.calls[0].evidence = 'unknown.Lock'),
    (d: any) => (d.calls[0].bindings[1].unit = 'frames'),
    (d: any) => (d.calls[0].bindings[1].name = 'current'),
    (d: any) => (d.operations[0].after[0].length = 'forward-output'),
    (d: any) => (d.operations[1].commit = []),
    (d: any) => (d.operations[0].cleanup = null),
  ]) {
    const changed = descriptor('media-buffer')
    edit(changed)
    t.throws(() => native.DynComBorrowedCopyPlan.prepare(JSON.stringify(changed)), {
      message: /fully regenerate/i,
    })
  }
})

if (typeof native.DynComBorrowedCopyTestFixture !== 'function') {
  test.skip('borrowed plan native fixtures require test-hooks', () => {})
} else {
  test.before(() => native.initializeCom(0))

  test.serial('borrowed plan copies owned bytes and executes length commit/cleanup', (t) => {
    const fixture = new native.DynComBorrowedCopyTestFixture()
    const object = fixture.object('media')
    const plan = prepare('media-buffer')
    try {
      const before = plan.readCopy(object)
      plan.replaceCopy(object, new Uint8Array([9, 8, 7]))
      t.deepEqual(before, Buffer.from([0, 1, 2, 3]))
      t.deepEqual(plan.readCopy(object), Buffer.from([9, 8, 7]))
      t.throws(() => plan.replaceCopy(object, Buffer.alloc(17)), { message: /capacity/i })
      fixture.configure('mediaSetHr', 0x80004005)
      t.throws(() => plan.replaceCopy(object, Buffer.alloc(4)))
      t.is(events(fixture, 'media.acquire'), events(fixture, 'media.unlock'))
      fixture.configure('mediaUnlockHr', 0x80004005)
      t.throws(() => plan.readCopy(object))
      const unlocks = events(fixture, 'media.unlock')
      t.throws(() => plan.readCopy(object), { message: /poisoned/i })
      t.is(events(fixture, 'media.unlock'), unlocks)
    } finally {
      object.release()
      fixture.releaseOwners()
    }
  })

  test.serial('borrowed plan packs bounded rows and releases its acquired owner', (t) => {
    const fixture = new native.DynComBorrowedCopyTestFixture()
    const object = fixture.object('bitmap')
    const plan = prepare('bitmap-bgra8')
    try {
      const copy = plan.readLockedBgra8Copy(object, [0, 0, 2, 2])
      t.deepEqual(copy.data, Buffer.from([0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18, 19]))
      t.deepEqual([copy.width, copy.height], [2, 2])
      copy.data[0] = 99
      t.is(fixture.bytes()[0], 0)
      fixture.configure('wicCount', 19)
      t.throws(() => plan.readLockedBgra8Copy(object, [0, 0, 2, 2]), { message: /span/i })
      t.is(events(fixture, 'wic.release'), 2)
      t.throws(() => plan.readLockedBgra8Copy(object, [0, 0, 2.5, 2]), { message: /integer/i })
    } finally {
      object.release()
      fixture.releaseOwners()
    }
  })

  test.serial('borrowed plan cannot create provenance for untracked audio', (t) => {
    const fixture = new native.DynComBorrowedCopyTestFixture()
    const render = fixture.object('render')
    const capture = fixture.object('capture')
    try {
      t.throws(() => prepare('audio-render').writeFramesCopy(render, Buffer.alloc(4)), { message: /observed/i })
      t.throws(() => prepare('audio-capture').readPacketCopy(capture), { message: /observed/i })
      t.is(events(fixture, 'render.acquire'), 0)
      t.is(events(fixture, 'capture.acquire'), 0)
    } finally {
      render.release()
      capture.release()
      fixture.releaseOwners()
    }
  })
}
