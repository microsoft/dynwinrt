// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

fn main() {
  // Link against node.lib so N-API symbols resolve at load time.
  // node-gyp puts node.lib in the node-gyp cache; napi-build finds it.
  napi_build::setup();
}
