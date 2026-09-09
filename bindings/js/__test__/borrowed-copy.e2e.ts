// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync } from 'node:child_process'
import { mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const packageRoot = resolve(process.cwd())
const repositoryRoot = resolve(packageRoot, '..', '..')
const output = join(packageRoot, 'target-generated-borrowed-copy')

test.before((t) => {
  const winmd = process.env.DYNWINRT_WIN32_WINMD
  t.truthy(winmd, 'Pinned DYNWINRT_WIN32_WINMD is required')
  rmSync(output, { recursive: true, force: true })
  const args = [
    'run',
    '--quiet',
    '-p',
    'dynwinrt-codegen',
    '--',
    'generate',
    '--winmd',
    winmd!,
    '--class-name',
    'Windows.Win32.Media.Audio.IAudioClient,Windows.Win32.Media.Audio.IAudioClient3,Windows.Win32.Media.Audio.IAudioRenderClient,Windows.Win32.Media.Audio.IAudioCaptureClient,Windows.Win32.Graphics.Imaging.IWICBitmap,Windows.Win32.Media.MediaFoundation.IMFMediaBuffer,Windows.Win32.Media.MediaFoundation.IMFSample',
    '--output',
    output,
  ]
  for (const extra of [['--dry-run'], []]) {
    const generated = spawnSync('cargo', [...args, ...extra], {
      cwd: repositoryRoot,
      encoding: 'utf8',
      windowsHide: true,
    })
    t.is(generated.status, 0, `${generated.stdout}\n${generated.stderr}`)
  }
  const scope = join(output, 'node_modules', '@microsoft')
  mkdirSync(scope, { recursive: true })
  symlinkSync(packageRoot, join(scope, 'dynwinrt'), 'junction')
  writeFileSync(
    join(output, 'borrowed-types.mts'),
    [
      "import { IAudioRenderClient, IAudioCaptureClient, IWICBitmap, IMFMediaBuffer, IMFSample } from './com/index.js'",
      "import type { DynWinRtValue } from '@microsoft/dynwinrt/com'",
      'declare const render: IAudioRenderClient',
      'declare const capture: IAudioCaptureClient',
      'declare const bitmap: IWICBitmap',
      'declare const media: IMFMediaBuffer',
      'declare const sample: IMFSample',
      'declare const mediaValue: DynWinRtValue',
      'sample.addBuffer(mediaValue)',
      'render.writeFramesCopy(new Uint8Array(8)); render.writeSilence(2)',
      'const packet = capture.readPacketCopy(); if (packet?.kind === "data") packet.data?.subarray(0)',
      'const pixels: Buffer = bitmap.readLockedBgra8Copy({ x: 0, y: 0, width: 2, height: 2 }).data',
      'const bytes: Buffer = media.readCopy(); media.replaceCopy(bytes); void pixels',
      '// @ts-expect-error Native acquisition is not a public escape hatch.',
      'render.getBuffer(2)',
      '// @ts-expect-error No caller-asserted block alignment.',
      'render.writeFramesCopy(new Uint8Array(8), 4)',
      '// @ts-expect-error Lock is available only inside owned-copy transactions.',
      'media.lock()',
      '// @ts-expect-error The copy transaction has fixed READ access, no caller flags.',
      'bitmap.readLockedBgra8Copy({ x: 0, y: 0, width: 2, height: 2 }, 2)',
    ].join('\n'),
  )
  const types = spawnSync(
    process.execPath,
    [
      join(packageRoot, 'node_modules', 'typescript', 'bin', 'tsc'),
      '--noEmit',
      '--strict',
      '--target',
      'ESNext',
      '--module',
      'NodeNext',
      '--moduleResolution',
      'NodeNext',
      join(output, 'borrowed-types.mts'),
    ],
    { cwd: packageRoot, encoding: 'utf8', windowsHide: true },
  )
  t.is(types.status, 0, `${types.stdout}\n${types.stderr}`)
})

for (const scenario of ['audio', 'provenance', 'capture', 'media', 'wic', 'mta', 'callback', 'descriptor']) {
  test.serial(`borrowed copy: ${scenario}`, (t) => {
    const result = spawnSync(process.execPath, [join(packageRoot, '__test__', 'borrowed-copy-child.cjs'), scenario], {
      cwd: packageRoot,
      encoding: 'utf8',
      windowsHide: true,
      timeout: 30_000,
      env: { ...process.env, DYNWINRT_BORROWED_GENERATED: output },
    })
    t.is(result.status, 0, `${result.error ?? ''}\n${result.stdout}\n${result.stderr}`)
    t.regex(result.stdout, new RegExp(`borrowed-copy:${scenario}:ok`))
  })
}

test.after.always(() => {
  rmSync(output, { recursive: true, force: true })
  rmSync(join(packageRoot, '.target-generated-borrowed-copy.dynwinrt-lock'), { force: true })
})
