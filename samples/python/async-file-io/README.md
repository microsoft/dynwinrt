# Python async file I/O

This deterministic sample uses generated `Windows.Storage` bindings to create,
write, append, and read a temporary file. It demonstrates:

- WinRT asynchronous methods with native Python `await`;
- normalized Python overloads (`create_file_async(name)` or
  `create_file_async(name, options)`);
- normal Python temporary-directory management;
- structural interface typing, so `StorageFile` is accepted directly where
  `IStorageFile` is required; and
- deterministic projected-object cleanup.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

The sample deletes its temporary directory when complete and prints
`python-file-io-ok` on success.

After generation, install the generated project into the same interpreter used
to run the sample: `python -m pip install -e .\generated` (use that interpreter's
full path when needed). The import remains `generated`; sources now live in
`generated\src\generated`, not directly in `generated`. When upgrading from
preview.23 or earlier, regenerate the complete output and reinstall it.
