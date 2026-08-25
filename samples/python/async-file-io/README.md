# Python async file I/O

This deterministic sample uses generated `Windows.Storage` bindings to create,
write, append, and read a temporary file. It demonstrates:

- WinRT asynchronous methods with native Python `await`;
- normal Python temporary-directory management;
- interface projection from `StorageFile` to `IStorageFile`; and
- deterministic projected-object cleanup.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The sample deletes its temporary directory when complete and prints
`python-file-io-ok` on success.
