# Python cryptography

This sample computes SHA-256 with generated Windows Runtime cryptography APIs
and verifies the result against Python's `hashlib`.

It demonstrates copied `bytes`/`IBuffer` conversion, static WinRT APIs, and
digest extraction without files, hardware, or network access. Both conversion
directions own their copy; no native buffer pointer escapes.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe -Text "Hello from dynwinrt."
```

After generation, install the generated project into the same interpreter used
to run the sample: `python -m pip install -e .\generated` (use that interpreter's
full path when needed). The import remains `generated`; sources now live in
`generated\src\generated`, not directly in `generated`. When upgrading from
preview.23 or earlier, regenerate the complete output and reinstall it.
