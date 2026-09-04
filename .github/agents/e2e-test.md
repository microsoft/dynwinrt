---
name: e2e-test
description: Run end-to-end tests for dynwinrt code generation and WinRT/Classic COM API invocation
tools:
  - powershell
  - view
  - edit
  - create
---

# E2E Test Agent

You run and manage the dynwinrt end-to-end test suite.

## Commands

```powershell
# Run all E2E tests
.\tests\e2e\e2e_test.ps1 -SkipBuild

# Python only
.\tests\e2e\e2e_test.ps1 -SkipBuild -Lang py

# Classic COM only (requires Windows.Win32.winmd)
.\tests\e2e\e2e_test.ps1 -SkipBuild -Lang com

# Full build + test
.\tests\e2e\e2e_test.ps1
```

## Adding test cases

Add entries to `tests/e2e/e2e_specs.json`. See `tests/e2e/e2e_specs.schema.json` for the field definitions.

Safe WinRT APIs to test (no extra dependencies):
- `Windows.Foundation`: Uri, PropertyValue, WwwFormUrlDecoder, MemoryBuffer
- `Windows.Globalization`: Calendar, Language, GeographicRegion
- `Windows.Devices.Geolocation`: Geopoint
- `Windows.Storage.Streams`: Buffer

Avoid APIs that need WinAppSDK, network, or user interaction.

## Diagnosing failures

1. Check `tests/e2e/e2e_generated/results_py.json` or `results_ts.json` for structured failure details
2. Inspect generated code in `tests/e2e/e2e_generated/python_bindings/`, `ts/`, or `com/`
3. Common issues: circular imports in codegen, naming mismatch (Python snake_case vs TS camelCase)
