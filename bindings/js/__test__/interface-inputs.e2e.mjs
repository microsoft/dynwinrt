// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { before, test } from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { runCodegen } from '../scripts/run-codegen.mjs'

const require = createRequire(import.meta.url)
const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const runtimeRoot = resolve(process.env.DYNWINRT_JS_PACKAGE ?? packageRoot)
const repositoryRoot = resolve(packageRoot, '..', '..')
const winmd =
  process.env.DYNWINRT_WINDOWS_WINMD ??
  String.raw`C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd`
const tsc = process.env.DYNWINRT_TSC ?? join(packageRoot, 'node_modules', 'typescript', 'bin', 'tsc')
const classes = [
  'Windows.Storage.StorageFile',
  'Windows.Storage.StorageFolder',
  'Windows.Storage.FileIO',
  'Windows.Storage.Streams.Buffer',
  'Windows.Foundation.Uri',
].join(',')
const interfaces = [
  'Windows.Storage.IStorageFile',
  'Windows.Storage.IStorageFolder',
  'Windows.Storage.IStorageItem',
  'Windows.Storage.Streams.IBuffer',
  'Windows.Foundation.IStringable',
].join(',')

before(() => {
  assert.ok(statSync(winmd).isFile(), 'Windows SDK metadata is required')
  assert.ok(statSync(tsc).isFile(), 'TypeScript is required')
  require(runtimeRoot).roInitialize(1)
})

async function exercise(generated, directory) {
  mkdirSync(directory)
  const retained = new Set()
  const keep = (value) => {
    assert.notEqual(value, null)
    retained.add(value)
    return value
  }
  try {
    const { FileIO, StorageFile, StorageFolder, IStorageFile, IStorageFolder, NameCollisionOption } = generated
    const path = join(directory, 'source.txt')
    const text = 'natural StorageFile input'
    writeFileSync(path, '')
    const file = keep(await StorageFile.getFileFromPathAsync(path))
    await FileIO.writeTextAsync(file, text)
    assert.equal(await FileIO.readTextAsync(file), text)
    const folder = keep(await StorageFolder.getFolderFromPathAsync(directory))
    const copyDirectory = join(directory, 'copies')
    const defaultCopyDirectory = join(directory, 'default-copies')
    mkdirSync(copyDirectory)
    mkdirSync(defaultCopyDirectory)
    const copies = keep(await StorageFolder.getFolderFromPathAsync(copyDirectory))
    const defaultCopies = keep(await StorageFolder.getFolderFromPathAsync(defaultCopyDirectory))
    const option = NameCollisionOption.FailIfExists
    const signal = new AbortController().signal
    for (const [method, args, destination] of [
      ['copyOverloadDefaultNameAndOptions', [copies, signal], join(copyDirectory, 'source.txt')],
      ['copyOverloadDefaultOptions', [copies, 'alias-default.txt', signal], join(copyDirectory, 'alias-default.txt')],
      ['copyOverload', [copies, 'alias-full.txt', option, signal], join(copyDirectory, 'alias-full.txt')],
      ['copyAsync', [defaultCopies, signal], join(defaultCopyDirectory, 'source.txt')],
      ['copyAsync', [copies, 'canonical-default.txt', signal], join(copyDirectory, 'canonical-default.txt')],
      ['copyAsync', [copies, 'canonical-full.txt', option, signal], join(copyDirectory, 'canonical-full.txt')],
    ]) {
      const copied = keep(await file[method](...args))
      assert.equal(realpathSync.native(copied.path), realpathSync.native(destination))
      assert.equal(await FileIO.readTextAsync(copied), text)
    }
    const moveCases = [
      ['moveOverloadDefaultNameAndOptions', []],
      ['moveOverloadDefaultOptions', ['alias-move-default.txt']],
      ['moveOverload', ['alias-move-full.txt', option]],
      ['moveAsync', []],
      ['moveAsync', ['canonical-move-default.txt']],
      ['moveAsync', ['canonical-move-full.txt', option]],
    ]
    for (const [index, [method, args]] of moveCases.entries()) {
      const source = join(directory, `move-${index}.txt`)
      writeFileSync(source, text)
      const moving = keep(await StorageFile.getFileFromPathAsync(source))
      await moving[method](copies, ...args, signal)
      assert.equal(existsSync(source), false)
      assert.equal(
        realpathSync.native(moving.path),
        realpathSync.native(join(copyDirectory, args[0] ?? `move-${index}.txt`)),
      )
      assert.equal(await FileIO.readTextAsync(moving), text)
    }

    const fileView = keep(file.as(IStorageFile))
    const folderView = keep(copies.as(IStorageFolder))
    assert.equal(await FileIO.readTextAsync(fileView), text)
    const legacy = keep(await fileView.copyOverloadDefaultOptions(folderView, 'legacy.txt'))
    await keep(legacy.as(IStorageFile)).moveOverloadDefaultOptions(folderView, 'legacy-moved.txt')
    assert.equal(await FileIO.readTextAsync(legacy), text)
    await legacy.renameAsyncOverloadDefaultOptions('renamed-by-required-interface.txt')
    const legacyPath = legacy.path
    await legacy.deleteAsyncOverloadDefaultOptions()
    assert.equal(existsSync(legacyPath), false)

    const created = keep(await folder.createFileAsyncOverloadDefaultOptions('created-by-alias.txt'))
    await FileIO.writeTextAsync(created, text)
    keep(await folder.createFolderAsyncOverloadDefaultOptions('child-by-alias'))
    for (const [method, expected] of [
      ['getFilesAsyncOverloadDefaultOptionsStartAndCount', 'created-by-alias.txt'],
      ['getFoldersAsyncOverloadDefaultOptionsStartAndCount', 'child-by-alias'],
      ['getItemsAsyncOverloadDefaultStartAndCount', 'created-by-alias.txt'],
    ]) {
      const view = keep(await folder[method]())
      const items = view.toArray().map(keep)
      assert.ok(
        items.some((item) => item.name === expected),
        `${method} must call its declared slot`,
      )
    }

    const bytes = Uint8Array.of(1, 2, 3, 4)
    const buffer = keep(generated.Buffer.fromBuffer(bytes))
    const bufferFile = keep(await folder.createFileAsync('buffer.txt'))
    await FileIO.writeBufferAsync(bufferFile, buffer)
    assert.deepEqual([...keep(await FileIO.readBufferAsync(bufferFile)).toBuffer()], [...bytes])
    const uri = keep(generated.Uri.createUri('https://example.invalid/interface-input'))
    assert.equal(keep(uri.as(generated.IStringable)).toString(), uri.toString())
    assert.throws(() => uri.as(IStorageFile))
    assert.throws(() => uri.as(IStorageFolder))
  } finally {
    for (const value of [...retained].reverse()) generated.releaseProjected(value)
  }
}

for (const [selection, generations] of [
  ['together', [`${classes},${interfaces}`]],
  ['classes-only', [classes]],
  ['interface-roots', [`${interfaces},Windows.Storage.FileIO,Windows.Storage.Streams.Buffer`]],
  ['class-first', [classes, interfaces]],
  ['interface-first', [interfaces, classes]],
]) {
  test(`interface inputs: ${selection}, strict TS and native CJS/ESM`, async (t) => {
    const directory = realpathSync.native(mkdtempSync(join(tmpdir(), 'dynwinrt-interface-inputs-')))
    try {
      const output = join(directory, 'generated')
      for (const roots of generations) {
        const result = runCodegen(['generate', '--winmd', winmd, '--class-name', roots, '--output', output], {
          cwd: repositoryRoot,
          encoding: 'utf8',
          windowsHide: true,
          timeout: 120_000,
        })
        assert.equal(result.status, 0, `${result.error ?? ''}\n${result.stdout}\n${result.stderr}`)
      }
      const scope = join(directory, 'node_modules', '@microsoft')
      mkdirSync(scope, { recursive: true })
      symlinkSync(runtimeRoot, join(scope, 'dynwinrt'), 'junction')
      symlinkSync(output, join(directory, 'node_modules', 'projected-storage'), 'junction')
      for (const extension of ['cts', 'mts']) {
        copyFileSync(
          join(packageRoot, '__test__', 'fixtures', 'interface-inputs.ts'),
          join(directory, `consumer.${extension}`),
        )
      }
      const compilation = spawnSync(
        process.execPath,
        [
          tsc,
          '--strict',
          '--skipLibCheck',
          'false',
          '--target',
          'ES2022',
          '--module',
          'NodeNext',
          '--moduleResolution',
          'NodeNext',
          '--types',
          'node',
          '--typeRoots',
          join(packageRoot, 'node_modules', '@types'),
          join(directory, 'consumer.cts'),
          join(directory, 'consumer.mts'),
        ],
        { encoding: 'utf8', windowsHide: true, timeout: 120_000 },
      )
      assert.equal(compilation.status, 0, `${compilation.error ?? ''}\n${compilation.stdout}\n${compilation.stderr}`)
      const modules = [
        ['cjs', require(output), require(join(directory, 'consumer.cjs'))],
        [
          'esm',
          await import(pathToFileURL(join(output, 'index.mjs')).href),
          await import(pathToFileURL(join(directory, 'consumer.mjs')).href),
        ],
      ]
      for (const [mode, generated, consumer] of modules) {
        const data = join(directory, mode)
        await exercise(generated, data)
        const path = join(data, 'compiled-consumer.txt')
        writeFileSync(path, '')
        assert.equal(await consumer.roundtrip(path, `${selection} ${mode} direct`), `${selection} ${mode} direct`)
        assert.equal(
          await consumer.packageRoundtrip(path, `${selection} ${mode} package`),
          `${selection} ${mode} package`,
        )
      }
      t.diagnostic(
        JSON.stringify({ architecture: process.arch, selection, strictTypeScript: true, native: ['cjs', 'esm'] }),
      )
    } finally {
      rmSync(directory, { recursive: true, force: true })
    }
  })
}
