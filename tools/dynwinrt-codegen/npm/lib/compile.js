// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const path = require("path");
const fs = require("fs");

/**
 * Compile all .ts files in srcDir to .js files in destDir using SWC.
 * @param {string} srcDir - Directory containing .ts files
 * @param {string} destDir - Output directory for .js files
 * @param {{ moduleType?: string, sourceMaps?: boolean }} options
 */
function compileDir(srcDir, destDir, options = {}) {
  const swc = require("@swc/core");
  const moduleType = options.moduleType || "es6";
  const sourceMaps = options.sourceMaps || false;

  if (!fs.existsSync(destDir)) {
    fs.mkdirSync(destDir, { recursive: true });
  }

  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    if (entry.isDirectory()) {
      compileDir(srcPath, path.join(destDir, entry.name), options);
    } else if (entry.name.endsWith(".ts")) {
      const code = fs.readFileSync(srcPath, "utf-8");
      const result = swc.transformSync(code, {
        filename: srcPath,
        jsc: {
          parser: { syntax: "typescript" },
          target: "es2022",
        },
        module: { type: moduleType },
        sourceMaps,
      });
      // Add .js extension to relative imports (ESM requires it, CJS does not)
      let jsCode = result.code;
      if (moduleType === "es6") {
        jsCode = jsCode.replace(
          /(from\s+['"])(\.\/[^'"]+?)(?<!\.js)(['"])/g,
          '$1$2.js$3'
        );
      }
      const jsName = entry.name.replace(/\.ts$/, ".js");
      const destPath = path.join(destDir, jsName);
      // Append sourceMappingURL if source map was generated
      if (sourceMaps && result.map) {
        const mapName = jsName + ".map";
        jsCode += `\n//# sourceMappingURL=${mapName}\n`;
        fs.writeFileSync(path.join(destDir, mapName), result.map);
      }
      fs.writeFileSync(destPath, jsCode);
      console.log(`Generated ${destPath}`);
    }
  }
}

module.exports = { compileDir };
