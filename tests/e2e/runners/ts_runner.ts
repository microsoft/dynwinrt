// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

/**
 * E2E test runner for TypeScript generated bindings.
 *
 * Reads e2e_specs.json, imports generated TypeScript modules,
 * and executes checks against real WinRT APIs.
 *
 * Usage:
 *   npx tsx tests/e2e/runners/ts_runner.ts --specs tests/e2e/e2e_specs.json --generated tests/e2e/e2e_generated/ts --runtime bindings/js/dist/winrt.js [--output results.json]
 */

import { strict as assert } from "node:assert";
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import { createRequire } from "node:module";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

interface Instantiate {
  kind: "activate" | "static_factory" | "constructor" | "none";
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
  langs?: string[];
  args?: any[];
  args_factory?: ArgsFactory;
  expected?: any;
  contains?: string;
  min?: number;
  max?: number;
  struct_class?: string;
  struct_args?: Record<string, any>;
  element_type?: string;
  values?: any[];
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

const require = createRequire(import.meta.url);
const generatedRoots = new Map<string, any>();

function generatedRoot(generatedDir: string): any {
  let root = generatedRoots.get(generatedDir);
  if (!root) {
    root = require(path.resolve(generatedDir, "index.js"));
    generatedRoots.set(generatedDir, root);
  }
  return root;
}

function toSnakeCase(name: string): string {
  return name
    .replace(/([A-Z])/g, "_$1")
    .replace(/^_/, "")
    .toLowerCase();
}

function toCamelCase(name: string): string {
  return name.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
}

function toPascalCase(name: string): string {
  const camel = toCamelCase(name);
  return camel.charAt(0).toUpperCase() + camel.slice(1);
}

export function instantiateClass(
  className: string,
  instantiate: Instantiate,
  cls: any,
): any {
  const instKind = instantiate.kind;

  if (instKind === "activate") {
    // Generated code provides create() or createDefault() for default constructors
    if (typeof cls.create === "function") {
      return cls.create();
    }
    if (typeof cls.createDefault === "function") {
      return cls.createDefault();
    }
    throw new Error(
      `${className} has no create() or createDefault() method for activate`,
    );
  }

  if (instKind === "static_factory") {
    const methodName = toCamelCase(instantiate.method!);
    const args = instantiate.args || [];
    if (
      className === "NotificationData" &&
      args[0] != null &&
      !Array.isArray(args[0]) &&
      typeof args[0] === "object"
    ) {
      const obj = cls.createDefault();
      for (const [key, value] of Object.entries(args[0])) {
        obj.values.set(key, value);
      }
      if (args[1] !== undefined) obj.sequenceNumber = args[1];
      return obj;
    }

    const overloadName = methodName.split("With")[0];
    const factory = cls[methodName] ?? cls[overloadName];
    if (typeof factory !== "function") {
      throw new Error(`${className} has no static factory ${methodName}`);
    }
    return factory.call(cls, ...args);
  }

  if (instKind === "constructor") {
    return new cls(...(instantiate.args || []));
  }

  if (instKind === "none") {
    return null;
  }

  throw new Error(`Unknown instantiate kind: ${String(instKind)}`);
}

async function runSpec(
  spec: Spec,
  generatedDir: string,
  runtime: any,
  runtimePath: string,
): Promise<SpecResult> {
  const specId = spec.id || `${spec.namespace}.${spec.class}`;
  const result: SpecResult = {
    id: specId,
    namespace: spec.namespace,
    class: spec.class,
    language: "ts",
    checks: [],
    pass: true,
    error: null,
  };

  try {
    const modulePath = path.resolve(generatedDir, "index.js");
    const mod = generatedRoot(generatedDir);
    const cls = mod[spec.class];
    if (!cls) throw new Error(`Class ${spec.class} not found in ${modulePath}`);

    const obj = instantiateClass(spec.class, spec.instantiate, cls);

    // Run checks
    for (const check of spec.checks) {
      if (!(check.langs || ["py", "ts"]).includes("ts")) continue;
      const cr = await runCheck(
        check,
        cls,
        spec.class,
        obj,
        generatedDir,
        runtime,
        runtimePath,
      );
      result.checks.push(cr);
      if (!cr.pass) result.pass = false;
    }
  } catch (e: any) {
    result.pass = false;
    result.error = e.message || String(e);
  }

  return result;
}

async function importClass(
  generatedDir: string,
  className: string,
): Promise<any> {
  const root = generatedRoot(generatedDir);
  const cls = root[className] ?? root[toPascalCase(className)];
  if (cls) return cls;
  throw new Error(`Class ${className} not found in generated root`);
}

async function runIssueRegression(
  name: string,
  generatedDir: string,
  runtimePath: string,
): Promise<{
  code: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}> {
  const childPath = fileURLToPath(
    new URL("./ts_issue_regression_child.mjs", import.meta.url),
  );
  const child = spawn(
    process.execPath,
    [childPath, name, generatedDir, runtimePath],
    {
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  let stdout = "";
  let stderr = "";
  let timedOut = false;
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });

  const timeout = setTimeout(() => {
    timedOut = true;
    child.kill();
  }, 30_000);

  return new Promise((resolve, reject) => {
    child.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.once("close", (code) => {
      clearTimeout(timeout);
      resolve({ code, stdout, stderr, timedOut });
    });
  });
}

async function runCheck(
  check: Check,
  cls: any,
  clsName: string,
  obj: any,
  generatedDir: string,
  runtime: any,
  runtimePath: string,
): Promise<CheckResult> {
  const kind = check.kind;
  const member = check.member ? toCamelCase(check.member) : "";
  const cr: CheckResult = { kind, member, pass: false, error: null };

  try {
    if (kind === "property_equals") {
      const actual = obj[member];
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "ibuffer_copied_roundtrip") {
      const empty = cls.fromBuffer(Buffer.alloc(0));
      if (
        empty.capacity !== 0 ||
        empty.length !== 0 ||
        !empty.toBuffer().equals(Buffer.alloc(0))
      ) {
        cr.error = "empty IBuffer copy round-trip failed";
        return cr;
      }

      const mutable = Uint8Array.from([0, 1, 2, 0, 255, 128]);
      const buffer = cls.fromBuffer(mutable);
      mutable.fill(9);
      const copied = buffer.toBuffer();
      if (buffer.capacity !== 6 || buffer.length !== 6) {
        cr.error = `expected Length/Capacity 6, got ${buffer.length}/${buffer.capacity}`;
        return cr;
      }
      buffer._obj.release();
      if (!copied.equals(Buffer.from([0, 1, 2, 0, 255, 128]))) {
        cr.error = `copied bytes changed after owner release: ${copied.toString("hex")}`;
        return cr;
      }
      cr.pass = true;
    } else if (kind === "property_exists") {
      const _ = obj[member]; // should not throw
      cr.pass = true;
    } else if (kind === "method_equals") {
      const method = obj[member].bind(obj);
      let args: any[] = [];
      if (check.args) {
        args = check.args;
      } else if (check.args_factory) {
        const af = check.args_factory!;
        const afCls =
          af.class === clsName
            ? cls
            : await importClass(generatedDir, af.class);
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
    } else if (kind === "method_result_contains") {
      const method = obj[member].bind(obj);
      const args = check.args || [];
      const resultObj = method(...args);
      let actual: string;
      if (resultObj && resultObj.absoluteUri !== undefined) {
        actual = resultObj.absoluteUri;
      } else if (resultObj && typeof resultObj.toString === "function") {
        actual = resultObj.toString();
      } else {
        actual = String(resultObj);
      }
      if (!actual.includes(check.contains!)) {
        cr.error = `"${check.contains}" not in "${actual}"`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "static_equals") {
      const method = cls[member].bind(cls);
      const args = check.args || [];
      const actual = method(...args);
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "static_not_null") {
      const method = cls[member].bind(cls);
      const args = check.args || [];
      const actual = method(...args);
      if (actual == null) {
        cr.error = "returned null";
      } else {
        cr.pass = true;
      }
    } else if (kind === "property_in_range") {
      const actual = obj[member];
      const val =
        typeof actual === "object" && actual !== null && "value" in actual
          ? actual.value
          : actual;
      const min = check.min ?? -Infinity;
      const max = check.max ?? Infinity;
      if (val < min || val > max) {
        cr.error = `value ${val} not in [${min}, ${max}]`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "interface_cast") {
      const ifaceClsName = (check as any).interface_class as string;
      const methodName = toCamelCase((check as any).method as string);

      const ifaceCls = await importClass(generatedDir, ifaceClsName);
      const casted = ifaceCls.from(obj._obj);
      const resultVal = casted[methodName];
      const actual = String(
        typeof resultVal === "function" ? resultVal.call(casted) : resultVal,
      );

      if (
        (check as any).contains &&
        !actual.includes((check as any).contains)
      ) {
        cr.error = `"${(check as any).contains}" not in "${actual}"`;
      } else if (
        check.expected !== undefined &&
        actual !== String(check.expected)
      ) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "property_set_equals") {
      const setValue = (check as any).set_value;
      obj[member] = setValue;
      const actual = obj[member];
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "method_then_property_equals") {
      let target = obj;
      const interfaceClass = (check as any).interface_class as
        | string
        | undefined;
      if (interfaceClass) {
        const iface = await importClass(generatedDir, interfaceClass);
        target = iface.from(obj._obj);
      }
      target[member](...((check as any).args || []));
      let actual = obj;
      for (const segment of (check as any).property_path || []) {
        actual = actual[toCamelCase(segment)];
      }
      if (actual !== check.expected) {
        cr.error = `expected ${JSON.stringify(check.expected)}, got ${JSON.stringify(actual)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "vector_view_access") {
      const vec = obj[member];
      const minSize = (check as any).min_size ?? 1;
      const size = vec.size;
      if (size < minSize) {
        cr.error = `vector size ${size} < ${minSize}`;
      } else {
        const first = vec.getAt(0);
        if (first == null) {
          cr.error = "getAt(0) returned null";
        } else {
          cr.pass = true;
        }
      }
    } else if (kind === "vector_index_of") {
      const vec = obj[member];
      let searchValue = (check as any).search_value;
      const expectedIndex = (check as any).expected_index;
      // If search_value is null, use getAt(0) as the search value (tests "found" path)
      if (searchValue === null) {
        searchValue = vec.getAt(0);
      }
      const result = vec.indexOf(searchValue);
      if (result !== expectedIndex) {
        cr.error = `indexOf returned ${result}, expected ${expectedIndex}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "vector_get_many") {
      const vec = obj[member];
      const capacity = Math.min((check as any).capacity ?? 4, vec.size);
      const atEnd = (check as any).at_end ?? false;
      const items = vec.getMany(atEnd ? vec.size : 0, capacity);
      if (atEnd && items.length !== 0) {
        cr.error = `getMany at Size returned ${items.length} items`;
      } else if (!atEnd && items.length === 0) {
        cr.error = "getMany returned no items";
      } else if (!atEnd && items[0] !== vec.getAt(0)) {
        cr.error = `first item ${JSON.stringify(items[0])} does not match getAt(0)`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "ireference_roundtrip") {
      const value = (check as any).value;
      obj[member] = value;
      if (obj[member] !== value) {
        cr.error = `native IReference roundtrip returned ${obj[member]}, expected ${value}`;
        return cr;
      }

      obj[member] = null;
      if (obj[member] !== null) {
        cr.error = "setting nullable value to null did not clear it";
        return cr;
      }

      const propertyValueMod = generatedRoot(generatedDir);
      const referenceModule = (check as any).reference_class;
      const referenceMod = generatedRoot(generatedDir);
      const factory =
        propertyValueMod.PropertyValue[
          (check as any).factory.replace(/_([a-z])/g, (_: string, c: string) =>
            c.toUpperCase(),
          )
        ];
      const boxed = factory((check as any).compatibility_value);
      const referenceClass = referenceMod[(check as any).reference_class];
      obj[member] = referenceClass.from(boxed);
      if (obj[member] !== (check as any).compatibility_value) {
        cr.error = `wrapper IReference roundtrip returned ${obj[member]}, expected ${(check as any).compatibility_value}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "sequence_protocol") {
      const sequence = member === "self" ? obj : obj[member];
      const expectedSize = (check as any).expected_size;
      const values = sequence.toArray();
      if (sequence.length !== expectedSize || values.length !== expectedSize) {
        cr.error = `expected sequence size ${expectedSize}, got length=${sequence.length}, array=${values.length}`;
      } else if (
        expectedSize > 0 &&
        sequence.at(-1) !== values[values.length - 1]
      ) {
        cr.error = "negative sequence indexing returned a different value";
      } else {
        cr.pass = true;
      }
    } else if (kind === "mapping_protocol") {
      const mapping = obj[member];
      const expectedSize = (check as any).expected_size;
      if (mapping.length !== expectedSize) {
        cr.error = `expected mapping size ${expectedSize}, got ${mapping.length}`;
        return cr;
      }
      const setKey = (check as any).set_key;
      if (setKey !== undefined) {
        const setValue = (check as any).set_value;
        mapping.set(setKey, setValue);
        if (!mapping.has(setKey) || mapping.get(setKey) !== setValue) {
          cr.error = "mapping set/get/has did not round-trip";
          return cr;
        }
        const view = mapping.getView();
        if (!view.has(setKey) || view.get(setKey) !== setValue) {
          cr.error = "mapping view did not preserve the new entry";
          return cr;
        }
        mapping.delete(setKey);
        if (mapping.has(setKey) || mapping.get(setKey) !== undefined) {
          cr.error = "mapping delete did not remove the entry";
          return cr;
        }
      }
      cr.pass = true;
    } else if (kind === "value_set_mapping") {
      const PropertyValue = await importClass(generatedDir, "PropertyValue");
      const first = PropertyValue.createString("first");
      const second = PropertyValue.createInt32(2);
      if (obj.length !== 0 || obj.size !== 0) {
        cr.error = "new ValueSet was not empty";
        return cr;
      }
      if (obj.insert("first", first)) {
        cr.error = "first ValueSet insert reported replacement";
        return cr;
      }
      obj.set("second", second);
      if (
        obj.length !== 2 ||
        !obj.has("first") ||
        obj.get("first") == null ||
        obj.get("second") == null
      ) {
        cr.error = "ValueSet insertion or lookup failed";
        return cr;
      }
      const view = obj.getView();
      if (view.length !== 2 || !view.has("first") || !view.has("second")) {
        cr.error = "ValueSet view did not preserve entries";
        return cr;
      }
      obj.delete("first");
      if (obj.has("first") || obj.length !== 1) {
        cr.error = "ValueSet deletion failed";
        return cr;
      }
      obj.clear();
      if (obj.length !== 0) {
        cr.error = "ValueSet clear failed";
      } else {
        cr.pass = true;
      }
    } else if (kind === "mutable_sequence_protocol") {
      const sequence = obj[member];
      const values = (check as any).set_value as any[];
      sequence.replaceAll(values);
      sequence.setAt(0, values[values.length - 1]);
      sequence.insertAt(0, values[0]);
      sequence.removeAt(2);
      sequence.append(".bmp");
      sequence.removeAtEnd();
      sequence.removeAtEnd();
      const actual = sequence.toArray();
      if (actual.length !== 2 || actual[0] !== values[0]) {
        cr.error = `mutable vector operations failed: ${JSON.stringify(actual)}`;
        return cr;
      }
      if (sequence.at(-1) !== actual[actual.length - 1]) {
        cr.error = "negative vector indexing failed";
        return cr;
      }
      sequence.clear();
      if (sequence.length !== 0) {
        cr.error = "mutable vector clear failed";
      } else {
        cr.pass = true;
      }
    } else if (kind === "calendar_comprehensive") {
      obj.year = 2024;
      obj.month = 1;
      obj.day = 2;
      obj.hour = 3;
      obj.minute = 4;
      obj.second = 5;
      obj.nanosecond = 0;
      const numericProperties = [
        "firstEra",
        "lastEra",
        "numberOfEras",
        "era",
        "firstYearInThisEra",
        "lastYearInThisEra",
        "numberOfYearsInThisEra",
        "firstMonthInThisYear",
        "lastMonthInThisYear",
        "numberOfMonthsInThisYear",
        "firstDayInThisMonth",
        "lastDayInThisMonth",
        "numberOfDaysInThisMonth",
        "nanosecond",
      ];
      if (!numericProperties.every((name) => typeof obj[name] === "number")) {
        cr.error = "Calendar numeric metadata returned a non-number";
        return cr;
      }
      const original = obj.clone();
      if (original == null || obj.compare(original) !== 0) {
        cr.error = "Calendar clone or comparison failed";
        return cr;
      }
      for (const [method, amount] of [
        ["addYears", 1],
        ["addMonths", 1],
        ["addWeeks", 1],
        ["addDays", 1],
        ["addHours", 1],
        ["addMinutes", 1],
        ["addSeconds", 1],
        ["addNanoseconds", 10_000],
      ] as const) {
        obj[method](amount);
        if (obj.compare(original) === 0) {
          cr.error = `Calendar ${method} did not change the value`;
          return cr;
        }
        obj[method](-amount);
        if (obj.compare(original) !== 0) {
          cr.error = `Calendar ${method} did not round-trip`;
          return cr;
        }
      }
      const calendarSystem = obj.getCalendarSystem();
      const clock = obj.getClock();
      const timeZone = obj.getTimeZone();
      obj.changeCalendarSystem(calendarSystem);
      obj.changeClock(clock);
      obj.changeTimeZone(timeZone);
      const formatted = [
        obj.eraAsString(),
        obj.yearAsString(),
        obj.monthAsString(),
        obj.dayAsString(),
        obj.dayOfWeekAsString(),
        obj.periodAsString(),
        obj.hourAsString(),
        obj.minuteAsString(),
        obj.secondAsString(),
        obj.timeZoneAsString(),
      ];
      if (!formatted.every((value) => typeof value === "string")) {
        cr.error = "Calendar formatting returned a non-string";
        return cr;
      }
      const minimum = obj.clone();
      const maximum = obj.clone();
      minimum.setToMin();
      maximum.setToMax();
      if (minimum.compare(maximum) >= 0) {
        cr.error = "Calendar min/max ordering was invalid";
      } else {
        obj.setToNow();
        cr.pass = true;
      }
    } else if (kind === "nested_struct_runtime") {
      const mod = generatedRoot(generatedDir);
      const DirectXPixelFormat = await importClass(
        generatedDir,
        "DirectXPixelFormat",
      );
      const descriptor = {
        width: 1920,
        height: 1080,
        format: DirectXPixelFormat.R32G32B32A32Typeless,
        multisampleDescription: { count: 4, quality: 7 },
      };
      const packed = mod.packDirect3DSurfaceDescription(descriptor);
      const roundtrip = mod.unpackDirect3DSurfaceDescription(packed.toValue());
      if (
        roundtrip.width !== descriptor.width ||
        roundtrip.height !== descriptor.height ||
        roundtrip.format !== descriptor.format ||
        roundtrip.multisampleDescription.count !== 4 ||
        roundtrip.multisampleDescription.quality !== 7
      ) {
        cr.error = `nested struct roundtrip failed: ${JSON.stringify(roundtrip)}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "storage_query_temp_folder") {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "dynwinrt-query-"));
      try {
        fs.writeFileSync(path.join(tempDir, "alpha.txt"), "alpha");
        fs.writeFileSync(path.join(tempDir, "beta.txt"), "beta");
        const folder = await cls.getFolderFromPathAsync(tempDir);
        if (folder == null)
          throw new Error("StorageFolder path lookup returned null");
        const directView = await folder.getFilesAsync();
        const directFiles = directView?.toArray() ?? [];
        const query = folder.createFileQuery();
        if (query == null) throw new Error("createFileQuery returned null");
        const count = await query.getItemCountAsync();
        const queryView = await query.getFilesAsync();
        const queryFiles = queryView?.toArray() ?? [];
        const missing = await folder.tryGetItemAsync("missing.file");
        const alpha = await folder.getFileAsync("alpha.txt");
        const directNames = (directFiles || [])
          .map((file: any) => file.name)
          .sort();
        const queryNames = (queryFiles || [])
          .map((file: any) => file.name)
          .sort();
        if (
          JSON.stringify(directNames) !==
            JSON.stringify(["alpha.txt", "beta.txt"]) ||
          JSON.stringify(queryNames) !== JSON.stringify(directNames) ||
          count !== 2 ||
          missing != null ||
          alpha?.name !== "alpha.txt" ||
          !query.folder?.isEqual(folder)
        ) {
          cr.error = `Storage query failed: direct=${JSON.stringify(directNames)}, query=${JSON.stringify(queryNames)}, count=${count}`;
        } else {
          cr.pass = true;
        }
      } finally {
        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    } else if (kind === "struct_roundtrip") {
      const structClass = check.struct_class as string;
      const structMod = generatedRoot(generatedDir);

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
        cr.error = "static method returned null after struct pack";
      } else {
        cr.pass = true;
      }
    } else if (kind === "array_roundtrip") {
      // Use the runtime already imported and passed to runCheck
      const elemType = check.element_type as string;
      const values = check.values as any[];

      let arr: any;
      if (elemType === "i32") arr = runtime.DynWinRtArray.fromI32Values(values);
      else if (elemType === "string")
        arr = runtime.DynWinRtArray.fromStringValues(values);
      else if (elemType === "f64")
        arr = runtime.DynWinRtArray.fromF64Values(values);
      else if (elemType === "u8")
        arr = runtime.DynWinRtArray.fromU8Values(values);
      else if (elemType === "i64")
        arr = runtime.DynWinRtArray.fromI64Values(values);
      else if (elemType === "f32")
        arr = runtime.DynWinRtArray.fromF32Values(values);
      else {
        cr.error = `unsupported element_type: ${elemType}`;
        return cr;
      }

      const methodName = toCamelCase(member);
      // Find method: try exact, then case-insensitive match
      let staticMethod = cls[methodName];
      if (!staticMethod) {
        const lowerTarget = methodName.toLowerCase();
        const found = Object.getOwnPropertyNames(cls).find(
          (k) => k.toLowerCase() === lowerTarget,
        );
        if (found) staticMethod = cls[found];
      }
      if (!staticMethod) {
        cr.error = `method ${methodName} not found on ${clsName}`;
        return cr;
      }
      const result = staticMethod.bind(cls)(arr);
      if (result == null) {
        cr.error = "static method returned null for array";
      } else {
        cr.pass = true;
      }
    } else if (kind === "async_memory_roundtrip") {
      const writeVal = (check as any).write_value ?? 42;
      const stream =
        typeof cls.create === "function" ? cls.create() : cls.createDefault();

      const writerMod = generatedRoot(generatedDir);
      const readerMod = generatedRoot(generatedDir);
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
    } else if (kind === "event_callback") {
      const sourceMethod = obj[member].bind(obj);
      const source = sourceMethod();
      const eventName = (check as any).event_name as string;
      const triggerMethod = toCamelCase((check as any).trigger as string);

      let fired = false;
      const onMethod = `on${eventName}`;
      source[onMethod]((..._args: any[]) => {
        fired = true;
      });

      // Try direct method, fall back to IClosable cast for close()
      if (typeof source[triggerMethod] === "function") {
        source[triggerMethod]();
      } else {
        const IClosable = await importClass(generatedDir, "IClosable");
        const closable = IClosable.from(source._obj);
        closable[triggerMethod]();
      }

      // NonBlocking TSFN: callback is queued on event loop, await a tick
      await new Promise((r) => setTimeout(r, 100));

      if (!fired) {
        cr.error = `event ${eventName} was not fired after ${triggerMethod}()`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "static_string_length") {
      const method =
        cls[member]?.bind(cls) ??
        cls[
          Object.getOwnPropertyNames(cls).find(
            (k) => k.toLowerCase() === member.toLowerCase(),
          )!
        ]?.bind(cls);
      if (!method) {
        cr.error = `method ${member} not found`;
        return cr;
      }
      const args = check.args || [];
      const actual = String(method(...args));
      const minLen = (check as any).min_length ?? 0;
      if (actual.length < minLen) {
        cr.error = `string length ${actual.length} < ${minLen}`;
      } else {
        cr.pass = true;
      }
    } else if (kind === "static_expect_error") {
      const method =
        cls[member]?.bind(cls) ??
        cls[
          Object.getOwnPropertyNames(cls).find(
            (k) => k.toLowerCase() === member.toLowerCase(),
          )!
        ]?.bind(cls);
      if (!method) {
        cr.error = `method ${member} not found`;
        return cr;
      }
      const args = check.args || [];
      try {
        method(...args);
        cr.error = "expected error but call succeeded";
      } catch {
        cr.pass = true;
      }
    } else if (kind === "cross_class_chain") {
      const saved: Record<string, any> = {};
      let chainOk = true;
      for (const step of (check as any).steps) {
        const stepClsName = step.class;
        const stepMod = generatedRoot(generatedDir);
        const stepCls = stepMod[stepClsName];
        const stepMethodName = toCamelCase(step.method);
        const stepMethod =
          stepCls[stepMethodName]?.bind(stepCls) ??
          stepCls[
            Object.getOwnPropertyNames(stepCls).find(
              (k: string) => k.toLowerCase() === stepMethodName.toLowerCase(),
            )!
          ]?.bind(stepCls);
        if (!stepMethod) {
          cr.error = `method ${stepMethodName} not found on ${stepClsName}`;
          chainOk = false;
          break;
        }

        const stepArgs: any[] = [...(step.args || [])];
        for (const ref of step.args_refs || []) {
          stepArgs.push(saved[ref]);
        }
        const result = stepMethod(...stepArgs);
        if (step.save_as) saved[step.save_as] = result;
        if (step.expected !== undefined) {
          const actual = typeof result === "object" ? String(result) : result;
          if (actual !== step.expected) {
            cr.error = `${step.method}: expected ${JSON.stringify(step.expected)}, got ${JSON.stringify(actual)}`;
            chainOk = false;
            break;
          }
        }
      }
      if (chainOk) cr.pass = true;
    } else if (
      kind === "device_information_async_collection" ||
      kind === "bitmap_encoder_async_create"
    ) {
      const child = await runIssueRegression(kind, generatedDir, runtimePath);
      if (child.timedOut) {
        cr.error = `${kind} child timed out`;
      } else if (child.code !== 0) {
        cr.error = `${kind} child exited with ${child.code}: ${child.stderr || child.stdout}`;
      } else {
        cr.pass = true;
      }
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
  let specsPath = "";
  let generatedDir = "";
  let runtimePath = "";
  let outputPath = "";

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--specs") specsPath = args[++i];
    else if (args[i] === "--generated") generatedDir = args[++i];
    else if (args[i] === "--runtime") runtimePath = args[++i];
    else if (args[i] === "--output") outputPath = args[++i];
  }

  if (!specsPath || !generatedDir || !runtimePath) {
    console.error(
      "Usage: ts_runner.ts --specs <path> --generated <dir> --runtime <index.js> [--output <path>]",
    );
    process.exit(2);
  }

  // Fix imports in generated files (both ESM `import ... from` and CJS `require`)
  const absRuntime = path.resolve(runtimePath).replace(/\\/g, "/");
  const generatedModules: string[] = [];
  const collectGeneratedModules = (directory: string): void => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) collectGeneratedModules(entryPath);
      else if (entry.name.endsWith(".js") || entry.name.endsWith(".mjs"))
        generatedModules.push(entryPath);
    }
  };
  collectGeneratedModules(generatedDir);
  for (const filePath of generatedModules) {
    let content = fs.readFileSync(filePath, "utf8");
    content = content.replace(
      /from '@microsoft\/dynwinrt'/g,
      `from 'file://${absRuntime}'`,
    );
    content = content.replace(
      /require\(['"]@microsoft\/dynwinrt['"]\)/g,
      `require('${absRuntime}')`,
    );
    fs.writeFileSync(filePath, content);
  }

  // Import runtime and init
  const runtime = await import(`file://${absRuntime}`);
  runtime.roInitialize(1);

  // Load specs
  const data: SpecFile = JSON.parse(fs.readFileSync(specsPath, "utf8"));
  const specs = data.specs.filter(
    (s) => (s.langs || ["py", "ts"]).includes("ts") && !s.skip_reason,
  );

  const results: SpecResult[] = [];
  let passed = 0;
  let failed = 0;

  for (const spec of specs) {
    const r = await runSpec(spec, generatedDir, runtime, absRuntime);
    results.push(r);
    if (r.pass) {
      passed++;
      console.log(`  PASS ${r.id}`);
    } else {
      failed++;
      const err =
        r.error ||
        r.checks
          .filter((c) => !c.pass)
          .map((c) => c.error)
          .join("; ");
      console.log(`  FAIL ${r.id}: ${err}`);
    }
  }

  console.log(`\n  TypeScript: ${passed} passed, ${failed} failed`);

  const output = {
    language: "ts",
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

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((e) => {
    console.error("Runner error:", e);
    process.exit(2);
  });
}
