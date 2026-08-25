# Python cryptography

This sample computes SHA-256 with generated Windows Runtime cryptography APIs
and verifies the result against Python's `hashlib`.

It demonstrates primitive enums, `IBuffer` values, static WinRT APIs, and
hexadecimal conversion without files, hardware, or network access.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe -Text "Hello from dynwinrt."
```
