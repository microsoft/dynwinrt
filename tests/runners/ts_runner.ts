// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * E2E test runner for TypeScript generated bindings.
 *
 * Reads e2e_specs.json, imports generated TypeScript modules,
 * and executes checks against real WinRT APIs.
 *
 * Usage:
 *   npx tsx tests/runners/ts_runner.ts --specs tests/e2e_specs.json --generated tests/e2e_generated/ts --runtime bindings/js/dist/index.js [--output results.json]
 */

import { strict as assert } from 'node:assert';
import * as fs from 'node:fs';
import * as path from 'node:path';

interface Instantiate {
  kind: 'activate' | 'static_factory' | 'none';
  method?: string;
  args?: any[];
}

interface ArgsFactory {
  class: string;
  method: string;
  args?: any[];
}

interface Check {
  kind: string;
  member: string;
  args?: any[];
  args_factory?: ArgsFactory;
  expected?: any;
  contains?: string;
}

interface Spec {
  namespace: string;
  class: string;
  id?: string;
  langs?: string[];
  skip_reason?: string;
  instantiate: Instantiate;
  checks: Check[];
}

interface SpecFile {
  specs: Spec[];
}

interface CheckResult {
  kind: string;
  member: string;
  pass: boolean;
  error: string | null;
}

interface SpecResult {
  id: string;
  namespace: string;
  class: string;
  language: string;
  checks: CheckResult[];
  pass: boolean;
  error: string | null;
}

function toSnakeCase(name: string): string {
  return name.replace(/([A-Z])/g, '_$1').replace(/^_/, '').toLowerCase();
}

function toCamelCase(name: string): string {
  return name.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
}

function toPascalCase(name: string): string {
  const camel = toCamelCase(name);
  return camel.charAt(0).toUpperCase() + camel.slice(1);
}

async function runSpec(
  spec: Spec,
  generatedDir: string,
  runtime: any,
): Promise<SpecResult> {
  const specId = spec.id || `${spec.namespace}.${spec.class}`;
  const result: SpecResult = {
    id: specId,
    namespace: spec.namespace,
    class: spec.class,
    language: 'ts',
    checks: [],
    pass: true,
    error: null,
  };

  try {
    // Import the generated module
    const modulePath = path.resolve(generatedDir, `${spec.class}.ts`);
    const mod = await import(`file://${modulePath.replace(/\\/g, '/')}`);
    const cls = mod[spec.class];
    if (!cls) throw new Error(`Class ${spec.class} not found in ${modulePath}`);

    // Instantiate
    let obj: any = null;
    const instKind = spec.instantiate.kind;

    if (instKind === 'activate') {
      // Generated code provides create() or createDefault() for default constructors
      if (typeof cls.create === 'function') {
        obj = cls.create();
      } else if (typeof cls.createDefault === 'function') {
        obj = cls.createDefault();
      } else {
        throw new Error(`${spec.class} has no create() or createDefault() method for activate`);
      }
    } else if (instKind === 'static_factory') {
      const methodName = toCamelCase(spec.instantiate.method!);
      const args = spec.instantiate.args || [];
      obj = cls[methodName](...args);
    }
    // kind === 'none': no instantiation

    // Run checks
    for (const check of spec.checks) {
      const cr = await runCheck(check, cls, spec.class, obj, generatedDir, runtime);
      result.checks.push(cr);
      if (!cr.pass) result.pass = false;
    }
  } catch (e: any) {
    result.pass = false;
    result.error = e.message || String(e);
  }

  return result;
}

async function importClass(generatedDir: string, className: string): Promise<any> {
  const candidates = [
    path.resolve(generatedDir, `${className}.ts`),
    path.resolve(generatedDir, `${toPascalCase(className)}.ts`),
  ];
  for (const p of candidates) {
    if (fs.existsSync(p)) {
      const mod = await import(`file://${p.replace(/\\/g, '/')}`);
      if (mod[className]) return mod[className];
    }
  }
  throw new Error(`Class ${className} not found in: ${candidates.join(', ')}`);
}

async function runCheck(
  check: Check,
  cls: any,
  clsName: string,
  obj: any,
  generatedDir: string,
  runtime: any,
): Promise<CheckResult> {
  const kind = check.kind;
  const member = check.member ? toCamelCase(check.member) : '';
  const cr: CheckResult = { kind, member, pass: false, error: null };

  try {
    if (kind === 'property_equals') {
      const actual = obj[member];
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'property_exists') {
      const _ = obj[member]; // should not throw
      cr.pass = true;
    } else if (kind === 'method_equals') {
      const method = obj[member].bind(obj);
      let args: any[] = [];
      if (check.args) {
        args = check.args;
      } else if (check.args_factory) {
        const af = check.args_factory!;
        const afCls = af.class === clsName ? cls : await importClass(generatedDir, af.class);
        const afMethod = toCamelCase(af.method);
        const afArgs = af.args || [];
        args = [afCls[afMethod](...afArgs)];
      }
      const actual = method(...args);
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'method_result_contains') {
      const method = obj[member].bind(obj);
      const args = check.args || [];
      const resultObj = method(...args);
      let actual: string;
      if (resultObj && resultObj.absoluteUri !== undefined) {
        actual = resultObj.absoluteUri;
      } else if (resultObj && typeof resultObj.toString === 'function') {
        actual = resultObj.toString();
      } else {
        actual = String(resultObj);
      }
      if (!actual.includes(check.contains!)) {
        cr.error = `"${check.contains}" not in "${actual}"`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'static_equals') {
      const method = cls[member].bind(cls);
      const args = check.args || [];
      const actual = method(...args);
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'static_not_null') {
      const method = cls[member].bind(cls);
      const args = check.args || [];
      const actual = method(...args);
      if (actual == null) {
        cr.error = 'returned null';
      } else {
        cr.pass = true;
      }
    } else if (kind === 'property_in_range') {
      const actual = obj[member];
      const val = typeof actual === 'object' && actual !== null && 'value' in actual ? actual.value : actual;
      const min = check.min ?? -Infinity;
      const max = check.max ?? Infinity;
      if (val < min || val > max) {
        cr.error = `value ${val} not in [${min}, ${max}]`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'interface_cast') {
      const ifaceClsName = (check as any).interface_class as string;
      const methodName = toCamelCase((check as any).method as string);

      const ifaceCls = await importClass(generatedDir, ifaceClsName);
      const casted = ifaceCls.from(obj._obj);
      const resultVal = casted[methodName];
      const actual = String(typeof resultVal === 'function' ? resultVal.call(casted) : resultVal);

      if ((check as any).contains && !actual.includes((check as any).contains)) {
        cr.error = `"${(check as any).contains}" not in "${actual}"`;
      } else if (check.expected !== undefined && actual !== String(check.expected)) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'property_set_equals') {
      const setValue = (check as any).set_value;
      obj[member] = setValue;
      const actual = obj[member];
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'vector_view_access') {
      const vec = obj[member];
      const minSize = (check as any).min_size ?? 1;
      const size = vec.size;
      if (size < minSize) {
        cr.error = `vector size ${size} < ${minSize}`;
      } else {
        const first = vec.getAt(0);
        if (first == null) {
          cr.error = 'getAt(0) returned null';
        } else {
          cr.pass = true;
        }
      }
    } else if (kind === 'struct_roundtrip') {
      const structClass = check.struct_class as string;
      const structModule = check.struct_module as string;
      // In TS, structs are interfaces + packFn. Find the module file.
      const candidates = [
        path.resolve(generatedDir, `${toPascalCase(structModule)}.ts`),
        path.resolve(generatedDir, `${structModule}.ts`),
      ];
      let modPath = candidates.find(p => fs.existsSync(p));
      if (!modPath) throw new Error(`Struct module not found: tried ${candidates.join(', ')}`);
      const structMod = await import(`file://${modPath.replace(/\\/g, '/')}`);

      // TS structs are plain objects (interfaces), create one directly
      const structArgs = check.struct_args as Record<string, any>;
      const structObj: Record<string, any> = {};
      for (const [k, v] of Object.entries(structArgs)) {
        structObj[toCamelCase(k)] = v;
      }

      const methodName = toCamelCase(member);
      const staticMethod = cls[methodName].bind(cls);
      const result = staticMethod(structObj);
      if (result == null) {
        cr.error = 'static method returned null after struct pack';
      } else {
        cr.pass = true;
      }
    } else if (kind === 'array_roundtrip') {
      // Use the runtime already imported and passed to runCheck
      const elemType = check.element_type as string;
      const values = check.values as any[];

      let arr: any;
      if (elemType === 'i32') arr = runtime.DynWinRtArray.fromI32Values(values);
      else if (elemType === 'string') arr = runtime.DynWinRtArray.fromStringValues(values);
      else if (elemType === 'f64') arr = runtime.DynWinRtArray.fromF64Values(values);
      else if (elemType === 'u8') arr = runtime.DynWinRtArray.fromU8Values(values);
      else if (elemType === 'i64') arr = runtime.DynWinRtArray.fromI64Values(values);
      else if (elemType === 'f32') arr = runtime.DynWinRtArray.fromF32Values(values);
      else { cr.error = `unsupported element_type: ${elemType}`; return cr; }

      const methodName = toCamelCase(member);
      // Find method: try exact, then case-insensitive match
      let staticMethod = cls[methodName];
      if (!staticMethod) {
        const lowerTarget = methodName.toLowerCase();
        const found = Object.getOwnPropertyNames(cls).find(k => k.toLowerCase() === lowerTarget);
        if (found) staticMethod = cls[found];
      }
      if (!staticMethod) { cr.error = `method ${methodName} not found on ${clsName}`; return cr; }
      const result = staticMethod.bind(cls)(arr);
      if (result == null) {
        cr.error = 'static method returned null for array';
      } else {
        cr.pass = true;
      }
    } else if (kind === 'async_memory_roundtrip') {
      const writeVal = (check as any).write_value ?? 42;
      const stream = typeof cls.create === 'function' ? cls.create() : cls.createDefault();

      const writerMod = await import(`file://${path.resolve(generatedDir, 'DataWriter.ts').replace(/\\/g, '/')}`);
      const readerMod = await import(`file://${path.resolve(generatedDir, 'DataReader.ts').replace(/\\/g, '/')}`);
      const DataWriter = writerMod.DataWriter;
      const DataReader = readerMod.DataReader;

      const writer = DataWriter.createDataWriter(stream.getOutputStreamAt(0));
      writer.writeInt32(writeVal);
      const stored = await writer.storeAsync();

      stream.seek(0);
      const reader = DataReader.createDataReader(stream.getInputStreamAt(0));
      const loaded = await reader.loadAsync(4);
      const readVal = reader.readInt32();

      if (stored < 4 || loaded < 4 || readVal !== writeVal) {
        cr.error = `async roundtrip failed: stored=${stored}, loaded=${loaded}, wrote ${writeVal}, read ${readVal}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'event_callback') {
      const sourceMethod = obj[member].bind(obj);
      const source = sourceMethod();
      const eventName = (check as any).event_name as string;
      const triggerMethod = toCamelCase((check as any).trigger as string);

      let fired = false;
      const onMethod = `on${eventName}`;
      source[onMethod]((..._args: any[]) => { fired = true; });

      // Try direct method, fall back to IClosable cast for close()
      if (typeof source[triggerMethod] === 'function') {
        source[triggerMethod]();
      } else {
        const IClosable = await importClass(generatedDir, 'IClosable');
        const closable = IClosable.from(source._obj);
        closable[triggerMethod]();
      }

      // NonBlocking TSFN: callback is queued on event loop, await a tick
      await new Promise(r => setTimeout(r, 100));

      if (!fired) {
        cr.error = `event ${eventName} was not fired after ${triggerMethod}()`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'static_string_length') {
      const method = cls[member]?.bind(cls) ?? cls[Object.getOwnPropertyNames(cls).find(k => k.toLowerCase() === member.toLowerCase())!]?.bind(cls);
      if (!method) { cr.error = `method ${member} not found`; return cr; }
      const args = check.args || [];
      const actual = String(method(...args));
      const minLen = (check as any).min_length ?? 0;
      if (actual.length < minLen) {
        cr.error = `string length ${actual.length} < ${minLen}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === 'static_expect_error') {
      const method = cls[member]?.bind(cls) ?? cls[Object.getOwnPropertyNames(cls).find(k => k.toLowerCase() === member.toLowerCase())!]?.bind(cls);
      if (!method) { cr.error = `method ${member} not found`; return cr; }
      const args = check.args || [];
      try {
        method(...args);
        cr.error = 'expected error but call succeeded';
      } catch {
        cr.pass = true;
      }
    } else if (kind === 'cross_class_chain') {
      const saved: Record<string, any> = {};
      let chainOk = true;
      for (const step of (check as any).steps) {
        const stepClsName = step.class;
        const stepModPath = path.resolve(generatedDir, `${stepClsName}.ts`);
        const stepMod = await import(`file://${stepModPath.replace(/\\/g, '/')}`);
        const stepCls = stepMod[stepClsName];
        const stepMethodName = toCamelCase(step.method);
        const stepMethod = stepCls[stepMethodName]?.bind(stepCls)
          ?? stepCls[Object.getOwnPropertyNames(stepCls).find((k: string) => k.toLowerCase() === stepMethodName.toLowerCase())!]?.bind(stepCls);
        if (!stepMethod) { cr.error = `method ${stepMethodName} not found on ${stepClsName}`; chainOk = false; break; }

        const stepArgs: any[] = [...(step.args || [])];
        for (const ref of (step.args_refs || [])) {
          stepArgs.push(saved[ref]);
        }
        const result = stepMethod(...stepArgs);
        if (step.save_as) saved[step.save_as] = result;
        if (step.expected !== undefined) {
          const actual = typeof result === 'object' ? String(result) : result;
          if (actual !== step.expected) {
            cr.error = `${step.method}: expected ${JSON.stringify(step.expected)}, got ${JSON.stringify(actual)}`;
            chainOk = false;
            break;
          }
        }
      }
      if (chainOk) cr.pass = true;
    } else {
      cr.error = `unknown check kind: ${kind}`;
    }
  } catch (e: any) {
    cr.error = e.message || String(e);
  }

  return cr;
}

async function main() {
  const args = process.argv.slice(2);
  let specsPath = '';
  let generatedDir = '';
  let runtimePath = '';
  let outputPath = '';

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--specs') specsPath = args[++i];
    else if (args[i] === '--generated') generatedDir = args[++i];
    else if (args[i] === '--runtime') runtimePath = args[++i];
    else if (args[i] === '--output') outputPath = args[++i];
  }

  if (!specsPath || !generatedDir || !runtimePath) {
    console.error('Usage: ts_runner.ts --specs <path> --generated <dir> --runtime <index.js> [--output <path>]');
    process.exit(2);
  }

  // Fix imports in generated files
  const absRuntime = path.resolve(runtimePath).replace(/\\/g, '/');
  const tsFiles = fs.readdirSync(generatedDir).filter(f => f.endsWith('.ts'));
  for (const f of tsFiles) {
    const filePath = path.join(generatedDir, f);
    let content = fs.readFileSync(filePath, 'utf8');
    content = content.replace(/from 'dynwinrt-js'/g, `from 'file://${absRuntime}'`);
    fs.writeFileSync(filePath, content);
  }

  // Import runtime and init
  const runtime = await import(`file://${absRuntime}`);
  runtime.roInitialize(1);

  // Load specs
  const data: SpecFile = JSON.parse(fs.readFileSync(specsPath, 'utf8'));
  const specs = data.specs.filter(s =>
    (s.langs || ['py', 'ts']).includes('ts') && !s.skip_reason
  );

  const results: SpecResult[] = [];
  let passed = 0;
  let failed = 0;

  for (const spec of specs) {
    const r = await runSpec(spec, generatedDir, runtime);
    results.push(r);
    if (r.pass) {
      passed++;
      console.log(`  PASS ${r.id}`);
    } else {
      failed++;
      const err = r.error || r.checks.filter(c => !c.pass).map(c => c.error).join('; ');
      console.log(`  FAIL ${r.id}: ${err}`);
    }
  }

  console.log(`\n  TypeScript: ${passed} passed, ${failed} failed`);

  const output = {
    language: 'ts',
    total: results.length,
    passed,
    failed,
    results,
  };

  if (outputPath) {
    fs.writeFileSync(outputPath, JSON.stringify(output, null, 2));
  }

  process.exit(failed > 0 ? 1 : 0);
}

main().catch(e => {
  console.error('Runner error:', e);
  process.exit(2);
});
