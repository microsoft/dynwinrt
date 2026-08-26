# Python OCR image

This sample reads an image with `Windows.Storage`, decodes it with
`BitmapDecoder`, and recognizes its text with `Windows.Media.Ocr`. It explicitly
projects the returned content-type stream to `IRandomAccessStream`, so the
required interface IID is validated before decoding.

The default run creates a local PNG containing `DYNWINRT OCR 42`, so the sample
does not require a camera, network access, or external image asset.

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
.\run.ps1 -Python C:\path\to\python.exe
```

Recognize another image:

```powershell
.\run.ps1 -Python C:\path\to\python.exe -Image C:\path\to\image.png
```

Custom images are accepted without assuming their contents. Use
`-Expect WORD1,WORD2` to require specific recognized words.

Windows must have an OCR language matching one of the current user's profile
languages.
