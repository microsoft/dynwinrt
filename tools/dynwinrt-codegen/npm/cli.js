#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Thin wrapper that invokes the platform-specific dynwinrt-codegen.exe.
// The Rust binary now emits .js + .d.ts directly (no SWC, no temp dir).

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const args = process.argv.slice(2);

// We accept legacy flags `--source-map`, `--declaration`, `--no-declaration` for
// backwards compatibility but they are no-ops (the exe always emits .js + .d.ts).
const exeArgs = [];
let outputDir = "./generated";
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === "--source-map" || a === "--declaration" || a === "--no-declaration") {
    // no-op (Rust emit is final)
    continue;
  }
  if (a === "--lang") {
    const v = args[++i];
    // ts and cjs are no longer separate targets — map to js silently
    exeArgs.push("--lang", (v === "ts" || v === "cjs") ? "js" : v);
  } else if (a === "--output" || a === "-o") {
    outputDir = args[++i];
    exeArgs.push(a, outputDir);
  } else {
    exeArgs.push(a);
  }
}

const arch = process.arch === "arm64" ? "arm64" : "x64";
const exe = path.join(__dirname, "bin", arch, "dynwinrt-codegen.exe");

try {
  execFileSync(exe, exeArgs, { stdio: "inherit" });
} catch (e) {
  process.exit(e.status ?? 1);
}

// Write a sub-package.json "type marker" so consumer projects that don't have
// `"type": "module"` at their root still parse the emitted `.js` files as ESM,
// avoiding Node's MODULE_TYPELESS_PACKAGE_JSON reparse warning.
if (fs.existsSync(outputDir)) {
  fs.writeFileSync(path.join(outputDir, "package.json"), '{ "type": "module" }\n');
}
