#!/usr/bin/env node
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.
//
// Thin wrapper that invokes the platform-specific dynwinrt-codegen.exe.
// The Rust binary now emits .js + .d.ts directly (no SWC, no temp dir).

const { execFileSync } = require("child_process");
const path = require("path");

const args = process.argv.slice(2);

// We accept legacy flags `--source-map`, `--declaration`, `--no-declaration` for
// backwards compatibility but they are no-ops (the exe always emits .js + .d.ts).
const exeArgs = [];
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
