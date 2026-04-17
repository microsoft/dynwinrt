#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { execFileSync } = require("child_process");
const path = require("path");
const fs = require("fs");
const os = require("os");

const args = process.argv.slice(2);

// Parse --lang, --output, --source-map, and --declaration from args
let lang = null;
let outputDir = null;
let sourceMaps = false;
let declarations = false;
const exeArgs = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--lang") {
    lang = args[++i];
    exeArgs.push("--lang", "ts"); // exe always generates TS; shim handles js/cjs
  } else if (args[i] === "--output") {
    outputDir = args[++i];
    exeArgs.push("--output"); // placeholder, patched below
    exeArgs.push(null);
  } else if (args[i] === "--source-map") {
    sourceMaps = true; // consumed by shim, not passed to exe
  } else if (args[i] === "--declaration") {
    declarations = true; // consumed by shim, not passed to exe
  } else {
    exeArgs.push(args[i]);
  }
}

const needsCompile = lang && lang !== "ts";

// If compiling, redirect exe output to a temp directory
let tsDir = outputDir;
if (needsCompile && outputDir) {
  tsDir = fs.mkdtempSync(path.join(os.tmpdir(), "dynwinrt-codegen-"));

  // Copy existing index.js as index.ts so exe can append to it (--class-name mode)
  const existingIndex = path.join(outputDir, "index.js");
  if (fs.existsSync(existingIndex)) {
    fs.copyFileSync(existingIndex, path.join(tsDir, "index.ts"));
  }
}

// Patch --output value in exeArgs
const outputIdx = exeArgs.indexOf(null);
if (outputIdx !== -1) {
  exeArgs[outputIdx] = tsDir || outputDir;
}

const arch = process.arch === "arm64" ? "arm64" : "x64";
const exe = path.join(__dirname, "bin", arch, "dynwinrt-codegen.exe");

// When compiling, suppress exe stdout (temp paths are noisy) but keep stderr
const stdio = needsCompile ? ["inherit", "pipe", "inherit"] : "inherit";

try {
  execFileSync(exe, exeArgs, { stdio });
} catch (e) {
  process.exit(e.status ?? 1);
}

if (needsCompile && outputDir) {
  const { compileDir } = require("./lib/compile");
  const moduleType = lang === "cjs" ? "commonjs" : "es6";

  if (!fs.existsSync(outputDir)) {
    fs.mkdirSync(outputDir, { recursive: true });
  }

  try {
    compileDir(tsDir, outputDir, { moduleType, sourceMaps });

    // Emit .d.ts files from the original .ts sources
    if (declarations) {
      emitDeclarations(tsDir, outputDir);
    }

    console.log(`Done. Output in ${outputDir}`);
  } finally {
    fs.rmSync(tsDir, { recursive: true, force: true });
  }
}

/**
 * Copy .ts files as .d.ts into destDir, rewriting relative .ts imports to .js.
 */
function emitDeclarations(srcDir, destDir) {
  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    if (entry.isDirectory()) {
      const sub = path.join(destDir, entry.name);
      if (!fs.existsSync(sub)) fs.mkdirSync(sub, { recursive: true });
      emitDeclarations(srcPath, sub);
    } else if (entry.name.endsWith(".ts")) {
      const dtsName = entry.name.replace(/\.ts$/, ".d.ts");
      let code = fs.readFileSync(srcPath, "utf-8");
      // Rewrite './Foo.ts' → './Foo.js' for declaration module resolution
      code = code.replace(
        /(from\s+['"])(\.\/[^'"]+?)\.ts(['"])/g,
        "$1$2.js$3"
      );
      fs.writeFileSync(path.join(destDir, dtsName), code);
    }
  }
}
