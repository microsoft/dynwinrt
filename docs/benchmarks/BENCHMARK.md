# End-to-End Performance: Electron IPC

Benchmark for the full Electron UI call path: **Renderer → IPC → Main Process → native addon → WinRT → IPC → Renderer**.

This demonstrates that **dynwinrt has no meaningful performance impact** in real Electron applications — IPC serialization dominates all native-layer differences.

Environment: AMD Ryzen 9 7940HX, Windows 11, Node.js v22, Electron.

---

## IPC Baseline

IPC noop round-trip (no WinRT call): **~81 µs**.

---

## Getter (0 in → 1 out)

| API | Static (C++/WinRT) | Dynamic (dynwinrt) | Ratio | Overhead |
|-----|--------------------|--------------------|-------|----------|
| `Uri.get_Host()` → hstring | 81.0 µs | 85.0 µs | 1.0x | +4.0 µs |
| `Uri.get_Port()` → i32 | 72.8 µs | 78.5 µs | 1.1x | +5.7 µs |
| `Uri.get_Suspicious()` → bool | 74.5 µs | 81.0 µs | 1.1x | +6.5 µs |

## Factory (1 in → 1 out)

| API | Static (C++/WinRT) | Dynamic (dynwinrt) | Ratio | Overhead |
|-----|--------------------|--------------------|-------|----------|
| `Uri.CreateUri(hstring)` | 80.0 µs | 85.7 µs | 1.1x | +5.7 µs |
| `PV.CreateInt32(i32)` | 86.5 µs | 84.7 µs | 1.0x | -1.8 µs |
| `PV.CreateDouble(f64)` | 81.9 µs | 82.4 µs | 1.0x | +0.5 µs |
| `PV.CreateBoolean(bool)` | 83.9 µs | 83.8 µs | 1.0x | -0.1 µs |
| `PV.CreateString(hstring)` | 77.9 µs | 88.8 µs | 1.1x | +10.9 µs |
| `Geopoint.Create(3×f64 struct)` | 92.1 µs | 94.3 µs | 1.0x | +2.2 µs |

## Method (1 in → 1 out)

| API | Static (C++/WinRT) | Dynamic (dynwinrt) | Ratio | Overhead |
|-----|--------------------|--------------------|-------|----------|
| `Uri.CombineUri(hstring)` | 95.5 µs | 85.9 µs | 0.9x | -9.6 µs |

---

## About Measurement Variance

IPC round-trip measurements have significant run-to-run noise caused by V8 JIT/GC, OS scheduling, and IPC serialization jitter. Across multiple runs, the same API can flip direction entirely:

| API | Run 1 | Run 2 | Run 3 |
|-----|-------|-------|-------|
| `get_Port` → i32 | 0.9x | 1.2x | 1.1x |
| `CombineUri` | 1.0x | 1.0x | 0.9x |
| IPC noop baseline | 82 µs | 114 µs | 81 µs |

Even the IPC baseline varies 81-114 µs between runs depending on system load. This confirms that all differences are noise — the native-layer overhead (1-3 µs for dynwinrt) is far below the IPC noise floor.

## Why It Doesn't Matter

The Electron IPC round-trip costs **~80-114 µs** (varies with system load). The native addon execution is a tiny fraction of that:

```
Electron IPC overhead     ████████████████████████████████████████  ~80-114 µs
dynwinrt invoke overhead  █                                         ~1-3 µs
C++/WinRT invoke          ▏                                         ~0.04-0.9 µs
```

All ratios cluster around **0.9-1.1x** — static and dynamic are indistinguishable through the IPC layer, with variations fully explained by measurement noise.

## Takeaway

For Electron desktop apps, **the choice between static C++/WinRT and dynamic dynwinrt has zero user-visible performance impact**. The IPC boundary is 50-2000x more expensive than any native-layer overhead. This means dynwinrt's benefits — no MSVC compilation, no version-specific generated code, simplified distribution — come at no practical performance cost.

---

## How to Run

```bash
cd benchmarks/electron && npm install && npm run dev
```
