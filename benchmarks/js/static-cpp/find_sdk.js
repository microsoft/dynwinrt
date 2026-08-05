// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

// Finds the C++/WinRT include directory from the latest Windows SDK.
const fs = require('fs');
const path = require('path');
const base = 'C:\\Program Files (x86)\\Windows Kits\\10\\Include';
const versions = fs.readdirSync(base).sort();
const latest = versions[versions.length - 1];
const cppwinrt = path.join(base, latest, 'cppwinrt');
if (!fs.existsSync(cppwinrt)) {
  throw new Error('C++/WinRT headers not found at ' + cppwinrt);
}
console.log(cppwinrt);
