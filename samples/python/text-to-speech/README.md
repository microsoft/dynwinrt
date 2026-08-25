# Python text-to-speech

This sample adapts PyWinRT's
[text-to-speech example](https://github.com/pywinrt/pywinrt/blob/main/samples/text_to_speech.py)
to generated dynwinrt bindings. It demonstrates:

- native Python `asyncio` over a WinRT asynchronous operation;
- structural interface typing that passes `SpeechSynthesisStream` directly
  where `IRandomAccessStream` is required;
- a typed `MediaPlayer.MediaEnded` callback; and
- deterministic COM/WinRT cleanup.

## Prerequisites

- Windows 10 or 11 with a Windows SDK installed;
- `dynwinrt` installed in the selected Python interpreter; and
- `dynwinrt-codegen` on `PATH`, or passed to `generate.ps1` with `-Codegen`.

Generate the bindings:

```powershell
.\generate.ps1 -Codegen ..\..\..\target\release\dynwinrt-codegen.exe
```

Play synthesized speech:

```powershell
.\run.ps1 -Python C:\path\to\python.exe -Text "Hello from dynwinrt."
```

For non-interactive validation, synthesize the stream without playing it:

```powershell
.\run.ps1 -Python C:\path\to\python.exe -Smoke
```
