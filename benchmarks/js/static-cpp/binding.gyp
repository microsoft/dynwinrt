{
  "targets": [{
    "target_name": "static_bench",
    "sources": ["static_bench.cpp"],
    "include_dirs": [
      "<!@(node -p \"require('node-addon-api').include\")",
      "<!@(node find_sdk.js)"
    ],
    "libraries": ["WindowsApp.lib"],
    "msvs_settings": {
      "VCCLCompilerTool": {
        "AdditionalOptions": ["/std:c++17", "/EHsc", "/bigobj"],
        "ExceptionHandling": 1
      }
    },
    "defines": ["NAPI_VERSION=9"]
  }]
}
