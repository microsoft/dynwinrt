// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import test from 'ava'
import { spawnSync } from 'node:child_process'
import { mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const packageRoot = resolve(process.cwd())
const repositoryRoot = resolve(packageRoot, '..', '..')
const output = join(packageRoot, 'target-generated-native-completion')
const runtimePath = join(packageRoot, 'dist', 'index.js')
const runner = join(packageRoot, '__test__', 'native-completion-child.mjs')

test.before((t) => {
  const winmd = process.env.DYNWINRT_WIN32_WINMD
  t.truthy(winmd, 'DYNWINRT_WIN32_WINMD is required for exact native-completion generation')
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
    'Windows.Win32.Media.Audio.ActivateAudioInterfaceAsync,Windows.Win32.Media.Audio.IMMDevice',
    '--output',
    output,
  ]
  const dryRun = spawnSync('cargo', [...args, '--dry-run'], {
    cwd: repositoryRoot,
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(dryRun.status, 0, dryRun.stderr)
  t.regex(dryRun.stdout, /Would generate ActivateAudioInterfaceAsync/)
  const generation = spawnSync('cargo', args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    windowsHide: true,
  })
  t.is(generation.status, 0, generation.stderr)
  if (process.env.DYNWINRT_TEST_AUDIO_RENDER === '1') {
    const renderDevice = spawnSync(
      'cargo',
      [
        'run',
        '--quiet',
        '-p',
        'dynwinrt-codegen',
        '--',
        'generate',
        '--namespace',
        'Windows.Media.Devices',
        '--class-name',
        'MediaDevice',
        '--output',
        output,
      ],
      { cwd: repositoryRoot, encoding: 'utf8', windowsHide: true },
    )
    t.is(renderDevice.status, 0, renderDevice.stderr)
  }
  const scope = join(output, 'node_modules', '@microsoft')
  mkdirSync(scope, { recursive: true })
  symlinkSync(packageRoot, join(scope, 'dynwinrt'), 'junction')
  writeFileSync(
    join(output, 'completion-types.mts'),
    [
      "import { activateAudioInterfaceAsync, IAudioClient, IAudioEndpointVolume } from './com/index.js'",
      "const client: Promise<IAudioClient> = activateAudioInterfaceAsync('render', IAudioClient)",
      "const endpoint: Promise<IAudioEndpointVolume> = activateAudioInterfaceAsync('render', IAudioEndpointVolume)",
      'void client; void endpoint',
      '// @ts-expect-error No PROPVARIANT input in the safe subset.',
      "activateAudioInterfaceAsync('render', IAudioClient, {})",
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
      join(output, 'completion-types.mts'),
    ],
    { cwd: packageRoot, encoding: 'utf8', windowsHide: true },
  )
  t.is(types.status, 0, `${types.stdout}\n${types.stderr}`)
})

const scenarios = [
  'uninitialized',
  'sta',
  'mta',
  'bounds',
  'liveness',
  'stale-token',
  'teardown-held',
  'teardown-inflight',
  'teardown-queued',
  'teardown-unload',
]
if (process.env.DYNWINRT_TEST_AUDIO_RENDER === '1') scenarios.push('render-smoke')

for (const scenario of scenarios) {
  test.serial(`native completion: ${scenario}`, (t) => {
    const child =
      scenario === 'teardown-unload' ? join(packageRoot, '__test__', 'native-completion-unload-child.mjs') : runner
    const result = spawnSync(process.execPath, [child, scenario], {
      cwd: packageRoot,
      encoding: 'utf8',
      timeout: 40_000,
      windowsHide: true,
      env: {
        ...process.env,
        DYNWINRT_COMPLETION_RUNTIME: runtimePath,
        DYNWINRT_COMPLETION_GENERATED: output,
      },
    })
    t.is(result.status, 0, `${result.error ?? ''}\n${result.stdout}\n${result.stderr}`)
    t.regex(result.stdout, new RegExp(`native-completion:${scenario}:ok`))
  })
}

test.after.always(() => {
  rmSync(output, { recursive: true, force: true })
  rmSync(join(packageRoot, '.target-generated-native-completion.dynwinrt-lock'), { force: true })
})
