# Native Layer Performance Analysis

> **Recorded benchmark snapshot (August 2026).** These measurements document a
> specific implementation and machine, not a permanent performance guarantee.
> Re-run the commands at the end of this document before making current
> performance decisions.

Benchmark for direct napi calls (no Electron IPC). Measures the actual overhead of dynwinrt's dynamic dispatch compared to static C++/WinRT projections.

Environment: AMD Ryzen 9 7940HX, Windows 11, Node.js v22. All Rust builds: release + LTO + codegen-units=1.

---

## Recorded Performance (napi-rs standard path)

JS → **napi-rs** → **dynwinrt** (Rust + libffi) → COM vtable.

### By parameter count

| Params | API | Static | Dynamic | Ratio | Overhead |
|--------|-----|--------|---------|-------|----------|
| 0 in → 1 out | `Uri.get_Host()` | 80 ns | 1.09 µs | 13.6x | +1.01 µs |
| 1 in → 1 out | `Uri.CombineUri(hstring)` | 1.31 µs | 3.38 µs | 2.6x | +2.07 µs |
| 2 in → 1 out | `Uri.CreateWithRelativeUri(hstring, hstring)` | 1.38 µs | 5.13 µs | 3.7x | +3.75 µs |

### By input type

| Input type | API | Static | Dynamic | Ratio | Overhead |
|------------|-----|--------|---------|-------|----------|
| i32 | `PropertyValue.CreateInt32(42)` | 370 ns | 1.95 µs | 5.3x | +1.58 µs |
| f64 | `PropertyValue.CreateDouble(3.14)` | 370 ns | 3.09 µs | 8.4x | +2.72 µs |
| bool | `PropertyValue.CreateBoolean(true)` | 334 ns | 3.00 µs | 9.0x | +2.67 µs |
| hstring | `PropertyValue.CreateString("hello")` | 393 ns | 4.17 µs | 10.6x | +3.78 µs |
| struct (3×f64) | `Geopoint.Create(BasicGeoposition)` | 410 ns | 6.70 µs | 16.3x | +6.29 µs |

### By return type

| Return type | API | Static | Dynamic | Ratio | Overhead |
|-------------|-----|--------|---------|-------|----------|
| i32 | `Uri.get_Port()` | 48 ns | 1.55 µs | 32x | +1.50 µs |
| bool | `Uri.get_Suspicious()` | 47 ns | 1.78 µs | 38x | +1.73 µs |
| hstring | `Uri.get_Host()` | 233 ns | 1.24 µs | 5.3x | +1.01 µs |

High ratios on i32/bool are because C++/WinRT getters are extremely fast (~48 ns). **Absolute overhead is consistent (~1-1.7 µs)** across all return types — this is acceptable for most application scenarios.

### Batch workload

| Workload | Static | Dynamic | Ratio | Overhead |
|----------|--------|---------|-------|----------|
| 200× create Uri + read Host | 395 µs | 2.10 ms | 5.3x | +1.71 ms |

---

## Where the Overhead Comes From

The standard `invoke()` path crosses the napi boundary multiple times per call. This is the dominant cost:

| Layer | Per-call | Source |
|-------|----------|--------|
| COM vtable dispatch | <1 ns | Function pointer indirection |
| dynwinrt Rust core | ~10 ns | RwLock read + vtable ptr extraction |
| Raw N-API boundary | ~30-70 ns | `napi_get_value_external` + `napi_create_string_utf16` |
| **napi-rs macro layer** | **+75-170 ns** | Type checking, class unwrap, UTF-8 string roundtrip |
| DynWinRtValue wrapping | +100-200 ns | External creation, Vec allocation |
| Extra napi call (`.toString()`) | +500-1000 ns | Second napi boundary crossing for result extraction |

The **napi-rs macro layer** and **extra napi calls** are the main bottleneck — not dynwinrt's dynamic dispatch itself.

### Complete call path breakdown (get_Port → i32)

```
38 ns   C++ static (node-addon-api + C++/WinRT)
48 ns   Rust raw N-API + dynwinrt          = 38 + 10 ns dynwinrt dispatch
124 ns  napi-rs fast path (getI32)         = 48 + 76 ns napi-rs macro overhead
1180 ns Standard (invoke + toNumber)       = 124 + 1056 ns (Vec + WinRTValue + 2nd napi call)
```

---

## Performance Optimization Validation

We built optimized fast-path APIs and raw napi-sys bindings to validate that the overhead is in the napi-rs layer, not in dynwinrt itself.

### 4-Way Comparison

Four JS → native → WinRT paths benchmarked side by side (50,000 iterations):

| Path | Stack |
|------|-------|
| C++ static | JS → node-addon-api → C++/WinRT → COM vtable |
| Rust raw N-API | JS → napi-sys (raw) → windows crate → COM vtable |
| Rust napi-rs | JS → napi-rs (macro) → windows crate → COM vtable |
| Dynamic (standard) | JS → napi-rs → dynwinrt (Rust + libffi) → COM vtable |

#### Getter (0 in → 1 out)

| API | C++ static | Rust raw | Rust napi-rs | Dynamic |
|-----|-----------|----------|-------------|---------|
| `get_Host()` → hstring | 78 ns | 68 ns | 208 ns | 1.17 µs |
| `get_Port()` → i32 | 37 ns | 46 ns | 66 ns | 1.07 µs |
| `get_Suspicious()` → bool | 35 ns | 44 ns | 66 ns | 1.35 µs |
| `get_QueryParsed()` → object | 201 ns | 433 ns | 1.02 µs | 1.25 µs |

#### Factory (1 in → 1 out object)

| API | C++ static | Rust raw | Rust napi-rs | Dynamic |
|-----|-----------|----------|-------------|---------|
| `Uri.CreateUri(hstring)` | 1.03 µs | 711 ns | 2.26 µs | 3.56 µs |
| `PV.CreateInt32(i32)` | 767 ns | 893 ns | 2.03 µs | 3.30 µs |
| `PV.CreateDouble(f64)` | 607 ns | 947 ns | 1.52 µs | 3.98 µs |
| `PV.CreateBoolean(bool)` | 792 ns | 1.12 µs | 1.24 µs | 3.25 µs |
| `PV.CreateString(hstring)` | 567 ns | 1.25 µs | 2.74 µs | 2.93 µs |
| `Geopoint.Create(struct 3×f64)` | 356 ns | 772 ns | 2.84 µs | 4.32 µs |

### Fast Path Optimization Results

Specialized methods that combine invoke + result extraction into a single napi call:

| Operation | Standard | Fast path (napi-rs) | Raw napi-sys + dynwinrt | C++ static |
|-----------|----------|--------------------|-----------------------|-----------|
| `get_Host()` → string | 1.52 µs | 341 ns | **76 ns** | 68 ns |
| `get_Port()` → i32 | 1.41 µs | 140 ns | **72 ns** | 40 ns |
| `CreateUri(hstring)` → obj | 4.13 µs | 2.19 µs | - | 1.03 µs |
| `PV.CreateInt32(i32)` → obj | 3.06 µs | 1.73 µs | - | 767 ns |

### Speedup summary

| Optimization | get_Host | get_Port |
|-------------|---------|---------|
| Standard → napi-rs fast path | **4.5x** | **10.1x** |
| Standard → raw napi-sys + dynwinrt | **20.0x** | **19.6x** |
| Raw napi-sys + dynwinrt vs C++ static | **1.1x** | **1.8x** |

---

## Theoretical Performance Limits

### Recorded production path: napi-rs macros (~1-3 µs overhead)

Our production path uses napi-rs macros for ergonomic Rust ↔ JS bindings. The napi-rs layer adds 75-170 ns per call for type checking and string roundtrips, plus each `DynWinRtValue` argument or result extraction is an extra napi boundary crossing (~500-1000 ns each).

### Optimized: napi-rs fast path (~100-250 ns overhead)

By combining invoke + result extraction into a single napi call (e.g., `getString()`, `getI32()`), we eliminate extra napi round-trips and Vec allocations. This is **5-10x faster** than the standard path and represents the practical performance ceiling for napi-rs based bindings.

### Optimal: raw napi-sys + dynwinrt (~10-30 ns overhead)

Bypassing napi-rs entirely and calling napi-sys directly, dynwinrt's dynamic dispatch achieves **near C++/WinRT parity**:

```
                    get_Port → i32 (per-call latency)

C++ static            ██ 37 ns
Rust raw static       ██ 46 ns
Raw napi + dynwinrt   ███ 72 ns          ← dynamic dispatch, near C++ parity
napi-rs fast path     █████ 140 ns
napi-rs standard      ██████████████████████████████████████████████████████████ 1410 ns
```

The ~35 ns gap between raw napi + dynwinrt (72 ns) and C++ static (37 ns) is dynwinrt's RwLock read + libffi dispatch for method lookup — the dynamic dispatch itself (libffi vtable call) adds negligible cost.

**Conclusion**: dynwinrt's core engine is not the bottleneck. The overhead comes from the napi-rs binding layer. With raw N-API bindings, dynamic WinRT invocation matches C++ static projection performance.

---

## How to Run

```bash
# 4-way benchmark (C++ / Rust raw / Rust napi-rs / Dynamic)
cd bindings/js && npm run build
cd benchmarks/js/static-cpp && npm install && npm run build
cargo build -p static-bench-rust --release
cp target/release/static_bench_rust.dll benchmarks/js/static-rust/static_bench_rust.node
cd bindings/js && npx tsx ../../benchmarks/js/bench_3way.ts

# Rust core engine (no JS overhead)
cargo bench -p dynwinrt --bench bench
```
