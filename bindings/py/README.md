# dynwinrt-py


Develop

Run `uv run maturin develop` to build the package

Use `uv run pytest` to run the tests

After modification `uv sync --reinstall` may be needed to reinstall the package

The wheel includes `__init__.pyi` and `py.typed` for static type checking.

Generated `IReference<T>` values are projected as `T | None`; native values,
`None`, and generated `IReference_*` wrappers are accepted as inputs.