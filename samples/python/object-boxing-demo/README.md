# Python Object boxing demo (experimental)

This sample demonstrates the experimental automatic boxing and unboxing of WinRT
`Object` (`IInspectable`) values in the Python projection. Every `Object`
position converts through `dynwinrt.to_winrt_object` and
`dynwinrt.from_winrt_object`, so this natural code works:

```python
settings = PropertySet()
settings["retries"] = 3                 # boxed as Int32
settings["tags"] = ["a", "b"]           # boxed as StringArray
settings["port"] = UInt32(8080)         # boxed as UInt32
assert settings["retries"] + 1 == 4     # read back as a plain int

size = file_properties["System.Size"]   # dynwinrt.UInt64(11)
copy["System.Size"] = size              # written back as UInt64
```

The demo covers a `PropertySet` with plain values, a lossless read-then-write
round trip for types that need a tag (`UInt8`, `Int64`, `Char16`, `Single`,
`Int16Array`), the `StorageFile` property store (`System.Size` as
`dynwinrt.UInt64`, `System.DateModified` as `datetime`), the
`DeviceInformation` property store, and objects that keep their identity and
`project_as()` support.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
.\run.ps1 -Python C:\path\to\python.exe -Benchmark
```

A successful run prints `python-object-boxing-demo-ok`. `-Benchmark` times
single `Object` reads and writes and reading a 1000-entry `PropertySet`.
`typing_check.py` is a small consumer for `mypy --strict` and `pyright`.
