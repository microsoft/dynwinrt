# Contributing to dynwinrt

This project welcomes contributions and suggestions. Most contributions require you to agree to a
Contributor License Agreement (CLA) declaring that you have the right to, and actually do, grant us
the rights to use your contribution. For details, visit https://cla.opensource.microsoft.com.

When you submit a pull request, a CLA bot will automatically determine whether you need to provide
a CLA and decorate the PR appropriately (e.g., status check, comment). Simply follow the instructions
provided by the bot. You will only need to do this once across all repos using our CLA.

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/).
For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or
contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.

## Development Setup

### Prerequisites

- **Rust** (stable, 2024 edition)
- **Windows 10/11** with Windows SDK installed
- **libffi** (bundled via the `libffi` crate)

### Building

```bash
# Build the core library
cargo build -p dynwinrt

# Build the code generator
cargo build -p dynwinrt-codegen
```

### Running Tests

```bash
# Core library tests
cargo test -p dynwinrt

# Code generator tests
cargo test -p dynwinrt-codegen
```

### Coverage

Install the Rust coverage tool once:

```powershell
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.7 --locked
```

Run the mixed-language coverage pipeline:

```powershell
.\eng\coverage\coverage.ps1
```

The script builds instrumented Python and Node native modules, runs the
Rust/Python/JavaScript tests and E2E suite, and writes HTML, LCOV, XML, and JSON
reports under `artifacts\coverage`.

### Code Style

- Run `cargo clippy` before submitting PRs
- Run `cargo fmt` to format code
- Follow standard Rust conventions

## Filing Issues

Use the [GitHub issue tracker](https://github.com/microsoft/dynwinrt/issues) to report bugs or request features.
Please use the provided issue templates when available.

## Pull Requests

1. Fork the repo and create your branch from `main`
2. If you've added code, add tests that cover the new functionality
3. Ensure all tests pass (`cargo test`)
4. Run `cargo clippy` and address any warnings
5. Submit your pull request

Thank you for contributing!
