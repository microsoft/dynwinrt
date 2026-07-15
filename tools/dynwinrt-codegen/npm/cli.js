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
const pkg = require("./package.json");
const arch = process.arch === "arm64" ? "arm64" : "x64";
const exe = path.join(__dirname, "bin", arch, "dynwinrt-codegen.exe");

if (args[0] === "runtime-dependency") {
  console.log(`@microsoft/dynwinrt@${pkg.version}`);
  process.exit(0);
}

if (args[0] === "capabilities") {
  const exeOutput = execFileSync(exe, ["capabilities"], { encoding: "utf8" });
  const capabilities = new Set(
    exeOutput
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
  );
  capabilities.add("runtime-dependency");
  console.log(Array.from(capabilities).join("\n"));
  process.exit(0);
}

// We accept legacy flags `--source-map`, `--declaration`, `--no-declaration` for
// backwards compatibility but they are no-ops (the exe always emits .js + .d.ts).
const exeArgs = [];
let outputDir = "./generated";
const isGenerate = args[0] === "generate";
const isHelp = args.includes("--help") || args.includes("-h");
const isDryRun = args.includes("--dry-run");
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === "--source-map" || a === "--declaration" || a === "--no-declaration") {
    // no-op (Rust emit is final)
    continue;
  } else if (a === "--output" || a === "-o") {
    outputDir = args[++i];
    exeArgs.push(a, outputDir);
  } else {
    exeArgs.push(a);
  }
}

try {
  execFileSync(exe, exeArgs, { stdio: "inherit" });
} catch (e) {
  process.exit(e.status ?? 1);
}
