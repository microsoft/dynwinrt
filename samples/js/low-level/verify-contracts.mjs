// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { runInNewContext } from 'node:vm'
import ts from 'typescript'

const root = fileURLToPath(new URL('.', import.meta.url))
const require = createRequire(import.meta.url)
const { values } = parseArgs({
    options: { picker: { type: 'boolean', default: false }, evidence: { type: 'string' } },
    strict: true,
    allowPositionals: false,
})
if (values.evidence !== undefined && !values.evidence.trim()) throw new Error('--evidence requires a nonempty output path.')
const metadata = process.env.DYNWINRT_WINDOWS_WINMD ?? join(
    process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)',
    'Windows Kits', '10', 'UnionMetadata', '10.0.26100.0', 'Windows.winmd',
)
const targets = [
    ['Windows.Foundation.IUriRuntimeClassFactory', 2, 'Windows.Foundation.Uri'],
    ['Windows.Foundation.IUriRuntimeClass', 17, 'Windows.Foundation.Uri'],
    ['Windows.Storage.IStorageFileStatics', 6, 'Windows.Storage.StorageFile'],
    ['Windows.Storage.IStorageFile', 12, 'Windows.Storage.IStorageFile'],
    ['Windows.Storage.Streams.IRandomAccessStream', 9, 'Windows.Storage.Streams.IRandomAccessStream'],
    ['Windows.Foundation.IClosable', 1, 'Windows.Foundation.IClosable'],
    ['Windows.Devices.Geolocation.IGeopointFactory', 3, 'Windows.Devices.Geolocation.Geopoint'],
    ['Windows.Devices.Geolocation.IGeopoint', 1, 'Windows.Devices.Geolocation.Geopoint'],
    ['Windows.Foundation.IPropertyValueStatics', 39, 'Windows.Foundation.PropertyValue'],
    ['Windows.Foundation.IPropertyValue', 39, 'Windows.Foundation.IPropertyValue'],
]
const additionalMetadata = []
if (values.picker) {
    const nuget = process.env.NUGET_PACKAGES ?? join(homedir(), '.nuget', 'packages')
    additionalMetadata.push(
        process.env.DYNWINRT_PICKER_WINMD ?? join(nuget, 'microsoft.windowsappsdk.foundation', '1.8.251104000', 'metadata', 'Microsoft.Windows.Storage.Pickers.winmd'),
        process.env.DYNWINRT_UI_WINMD ?? join(nuget, 'microsoft.windowsappsdk.interactiveexperiences', '1.8.251104001', 'metadata', '10.0.18362.0', 'Microsoft.UI.winmd'),
    )
    targets.push(
        ['Microsoft.Windows.Storage.Pickers.IFileOpenPickerFactory', 1, 'Microsoft.Windows.Storage.Pickers.FileOpenPicker'],
        ['Microsoft.Windows.Storage.Pickers.IFileOpenPicker', 9, 'Microsoft.Windows.Storage.Pickers.FileOpenPicker'],
        ['Microsoft.Windows.Storage.Pickers.IPickFileResult', 1, 'Microsoft.Windows.Storage.Pickers.PickFileResult'],
    )
}
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex')
const plain = (value) => JSON.parse(JSON.stringify(value))

function recordingRuntime() {
    const interfaces = new Map()
    const forbidden = () => { throw new Error('Native invocation is forbidden during contract verification.') }
    const Type = {}
    for (const kind of [
        'i32', 'i64', 'hstring', 'object', 'f64', 'f32', 'u8', 'u32', 'u64', 'i16', 'u16', 'boolType',
        'runtimeClass', 'guidType', 'interface', 'iAsyncAction', 'iAsyncOperation', 'structType', 'enumType', 'arrayType', 'parameterized',
    ]) {
        Type[kind] = (...args) => ({ kind, args })
    }
    Type.registerInterface = (name, iid) => {
        assert.ok(!interfaces.has(name), `Duplicate registration: ${name}`)
        const contract = { iid, methods: [] }
        interfaces.set(name, contract)
        return {
            addMethod(name, signature) {
                contract.methods.push({ name, slot: 6 + contract.methods.length, params: signature.params })
                return this
            },
            method: forbidden,
            methodByName: forbidden,
        }
    }
    class Signature {
        params = []
        addIn(type) { this.params.push({ direction: 'in', type }); return this }
        addOut(type) { this.params.push({ direction: 'out', type }); return this }
    }
    return {
        interfaces,
        api: {
            DynWinRtType: Type,
            DynWinRtMethodSig: Signature,
            WinGuid: { parse: (iid) => iid.toLowerCase() },
        },
    }
}

function readManualContracts() {
    const { api, interfaces } = recordingRuntime()
    const filename = join(root, 'contracts.ts')
    const compiled = ts.transpileModule(readFileSync(filename, 'utf8'), {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
        fileName: filename,
    })
    const exports = {}
    runInNewContext(compiled.outputText, {
        exports,
        require(specifier) {
            assert.equal(specifier, '@microsoft/dynwinrt')
            return api
        },
    }, { filename, timeout: 5000 })
    for (const register of ['registerUri', 'registerAsyncFile', 'registerGeopoint', 'registerPropertyValue']) {
        exports[register]()
    }
    if (values.picker) exports.registerPicker()
    return interfaces
}

function readGeneratedContract(filename, name) {
    const source = ts.createSourceFile(filename, readFileSync(filename, 'utf8'), ts.ScriptTarget.Latest, true)
    const names = new Set([`IID_${name}`, `_${name}Cache`, `_${name}`])
    const declarations = source.statements.filter((statement) =>
        ts.isVariableStatement(statement) && statement.declarationList.declarations.some((declaration) =>
            ts.isIdentifier(declaration.name) && names.has(declaration.name.text)))
    assert.equal(declarations.length, 3, `Cannot locate the complete generated registration for ${name}`)
    const { api, interfaces } = recordingRuntime()
    // Evaluate only the metadata-derived declarations, not generated application methods or imports.
    runInNewContext(`${declarations.map((declaration) => declaration.getText(source)).join('\n')}\nvoid _${name}.method;`, api, {
        filename,
        timeout: 5000,
    })
    assert.equal(interfaces.size, 1)
    return interfaces.get(name)
}

const staging = mkdtempSync(join(tmpdir(), 'dynwinrt-low-level-contracts-'))
const output = join(staging, 'bindings')
try {
    const metadataHash = hash(readFileSync(metadata))
    const additionalEvidence = additionalMetadata.map((path) => ({ path, sha256: hash(readFileSync(path)) }))
    const codegen = require.resolve('@microsoft/dynwinrt-codegen/cli.js')
    const generation = spawnSync(process.execPath, [
        codegen, 'generate', '--winmd', values.picker ? `${metadata};${additionalMetadata[0]}` : metadata,
        ...(values.picker ? ['--ref', additionalMetadata[1]] : []),
        '--class-name', [...new Set(targets.map(([, , owner]) => owner))].join(','),
        '--output', output,
    ], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 })
    assert.ifError(generation.error)
    assert.equal(generation.status, 0, `Metadata generation failed:\n${generation.stdout}\n${generation.stderr}`)
    assert.doesNotMatch(generation.stdout + generation.stderr, /\b(?:warning|error):/i, 'Resolve metadata diagnostics before using these contracts.')
    const manual = readManualContracts()
    assert.equal(manual.size, targets.length)
    const verified = []
    for (const [fullName, methodCount, owner] of targets) {
        const name = fullName.split('.').at(-1)
        // Exclusive factory interfaces are emitted inside their owning runtime class.
        const parts = owner.split('.')
        const module = parts.pop()
        const filename = join(output, ...parts.map((part) => part.toLowerCase()), `${module}.js`)
        const generated = readGeneratedContract(filename, name)
        assert.equal(generated.methods.length, methodCount, `Metadata method count changed for ${name}`)
        assert.deepEqual(plain(manual.get(name)), plain(generated), `Handwritten contract differs from metadata: ${fullName}`)
        verified.push({ name: fullName, methodCount, generatedSha256: hash(readFileSync(filename)), contract: plain(generated) })
    }
    if (values.evidence !== undefined) {
        writeFileSync(resolve(values.evidence), JSON.stringify({
            metadata,
            metadataSha256: metadataHash,
            additionalMetadata: additionalEvidence,
            manualContractsSha256: hash(readFileSync(join(root, 'contracts.ts'))),
            interfaces: verified,
        }, null, 2) + '\n')
    }
    console.log(`Matched ${verified.length} complete interfaces / ${verified.reduce((count, entry) => count + entry.methodCount, 0)} methods.`)
    console.log(`Metadata: ${metadata}\nSHA256: ${metadataHash}\nNo native addon or WinRT call was executed.`)
} finally {
    rmSync(staging, { recursive: true })
}
