# Dynamic Projection for JavaScript and Python

## Objective

The primary objective of this project is to expose WinRT Windows APIs to dynamic languages, specifically JavaScript and Python, while ensuring a seamless developer experience. This approach eliminates the need for native compilation (such as C++ compilers) and removes strict version coupling between the projection and the Windows App SDK (WASDK) components.

## Motivation and Challenges

### Versioning Complexity

Current static projections, such as existing PyWinRT, necessitate the generation and compilation of native code for specific versions of WinRT components. This requirement results in a complex compatibility matrix and mandates the release of new projection packages for every WASDK release, and more importantly, does not expose full WinRT APIs to developers due to AOT compilation constraints.

### Developer Experience (DX)

Developers in the Node.js, Electron, and Python ecosystems expect a streamlined setup process, typically via `npm install` or `pip install`. They should not be required to configure MSVC tools or integrate C++ compilers into their build chains.

### Static vs. Dynamic Nature

A key distinction exists between static and dynamic language projections:

*   **Static Languages (C++, Rust, C#)**: These languages possess full type information at compile time, allowing static projections to tailor bindings precisely to what is utilized.
*   **Dynamic Languages (JavaScript, Python)**: These rely on runtime evaluation, where objects are often type-erased. Static projections can create gaps when a function encounters an unknown WinRT object at runtime that was not statically projected.

## Architecture

The proposed architecture transitions from static native bindings to a dynamic approach, comprising two primary, separable components:

### The Runtime Library

This component is a minimal runtime library native to the target ecosystem (e.g., a `.pyd` module or Node.js addon) that facilitates dynamic calls to arbitrary WinRT APIs.

#### FFI and ABI Handling

A minimal dynamic Foreign Function Interface (FFI) layer is responsible for:
*   Invoking arbitrary WinRT methods via function pointers with correct parameter type information, typically leveraging `dyncall` or `libffi`.
*   Managing `out` parameters through stack allocation.
*   Support of WinRT type system, especially runtime generic type system.
*   Supporting direct pass-by-value for value types, computing struct sizes and alignments at runtime in the absence of `sizeof`.

This infrastructure can be mostly shared across dynamic languages.

#### Platform Primitives 

*   The library wraps fundamental OS APIs, including string handling, `RoInitialize`, `RoGetActivationFactory`, and `QueryInterface`.
*   **WinAppSDK Bootstrap**: Including the necessary bootstrap DLL and native interop to enable WinAppSDK usage for unpackaged applications.

This infrastructure can be mostly shared across dynamic languages.

#### Language Adaptation

*   Mapping WinRT `HSTRING`s to native language strings.
*   Converting WinRT `HRESULT`s into language-specific exceptions.
*   Transforming `IAsyncAction` into language-specific Promises or Awaitables.

### Metadata Parser and Projection Generator

This component bridges the gap between raw WinMD metadata and the runtime projection, operating in two distinct modes:

#### Mode A: Fully Lazy Assessment (Runtime)

In this mode, the runtime parses `.winmd` files on the fly as APIs are accessed.
*   **Advantages**: Proven stability in previous JavaScript projections and simpler distribution (no generation step required).
*   **Disadvantages**: Incurs runtime parsing overhead (potentially negligible compared to marshalling) and lacks IDE IntelliSense support.

#### Mode B: Design-Time Generation (Pre-processed)

A CLI tool parses `.winmd` files to generate **non-native** code (pure `.js` or `.py` files) that defines interface shapes and method signatures for the runtime. This mode can also generate IDE helpers, such as TypeScript `.d.ts` files and Python `.pyi` stubs.
*   **Advantages**: Enhanced Developer Experience (IntelliSense/Autocomplete) and faster startup times (eliminating WinMD parsing).
*   **Disadvantages**: Requires a generation step, although strictly without native compilation.

## Developer Workflow

The usage workflow involves two stages:

### Step 1: Runtime Distribution
The runtime is distributed as a generic library for the target language (e.g., `npm install @microsoft/dynwinrt` or `pip install dynwinrt`).

### Step 2: Projection Usage
Developers have two options for using the projection:
1.  **Direct Usage**: The runtime directly supports runtime interface specifications. Developers can use libraries directly by lazily loading namespaces with distributed WinMDs. The runtime parses the WinMDs and generates necessary interface shapes on the fly.
2.  **Generated Bindings**: Alternatively, developers can execute a tool (e.g., `npx dynwinrt-codegen generate ...`) to generate bindings and types specifically for the WinMDs they intend to use.

## Performance Considerations

### Overhead Analysis

The primary performance costs are attributed to WinMD parsing (in lazy mode) and the overhead of dynamic WinRT method invocation (dynamic dispatch). This invocation overhead is expected to be comparable to existing marshalling costs associated with crossing the JavaScript/Python boundary.

### Optimization Strategies

Adopting a hybrid approach—specifically, the design-time generation of interface shapes—can eliminate runtime WinMD parsing costs, reducing overhead strictly to FFI operations.

## Implementation Strategy

The runtime provides a minimal representation of WinRT types and values, along with conversions to language-native equivalents. It also offers a minimal interface specification language for the target language, enabling users to define WinRT interfaces and classes at runtime.

### Runtime Interface Specification Example

TypeScript interface as demonstrated below,
thus with the minimum runtime provided by `@microsoft/dynwinrt`,
all WinRT APIs can be specified and invoked dynamically using the target language.

> Illustrative sketch only — the runtime API surface actually shipped is `DynWinRtType` / `DynWinRtValue` / `DynWinRtMethodHandle`, described in `bindings/js/README.md`. The `WinRT.Interface({...})` shape below is design-language pseudocode kept for historical continuity with the original Lazy-WinRT prototype.

```ts
import { WinRT } from "@microsoft/dynwinrt";

const UriInterface = WinRT.Interface({
  namespace: "Windows.Foundation",
  name: "IUriRuntimeClass",
  guid: "<...guid of the interface...>",
  methods: [
    // get AbsoluteUri(): string
    WinRT.Method([WinRT.Out(WinRT.HSTRING)]), // Implicitly assumes first argument is ComPtr, result is HRESULT
    // get Domain(): string
    WinRT.Method([WinRT.Out(WinRT.HSTRING)]),
    // ... other methods
  ],
});

const UriFactoryInterface = WinRT.Interface({
  namespace: "Windows.Foundation",
  name: "IUriRuntimeClassFactory",
  guid: "<...guid of the interface...>",
  methods: [
    // CreateUri(string uri): IUriRuntimeClass
    WinRT.Method([WinRT.HSTRING, WinRT.Out(UriInterface)]),
  ],
});

class Uri {
  // Statically cached factory object in target dynamic language
  static Factory = WinRT.as(UriFactoryInterface, WinRT.getActivationFactory("Windows.Foundation.Uri"));

  constructor(uriString: string) {
    this._instance = WinRT.callMethod(UriFactoryInterface, 7, [
      uriString,
    ]);
  }

  get Host(): string {
    return WinRT.callMethod(UriInterface, 11, [this._instance]); // 11 is the vtable index of get_Host
  }
}
```

The WinMD parser allows for the generation of these interface specifications and developer-friendly projection classes at design time or runtime. Async handling and generic type support (IVector&lt;T&gt;, IAsyncOperation&lt;T&gt;, etc.) are fully implemented in the core runtime.

### Stub Method Optimizations

To optimize performance, common method signatures can be implemented within the runtime library. This allows frequent operations to execute with speeds comparable to static projections. Since only the actual ABI signature is critical for these stub methods, a wide variety of WinRT methods can map to a single stub. For instance, getter-like or factory-like methods can often map to a unified signature:

```cpp
// Common stub for object.get_X -> Com/HSTRING reference types
HRESULT Method_Out_Pointer(void* funPtr, ComPtr self, void* outValue) {
    var f = // cast funPtr to proper function pointer type
    return f(self, outValue);
}
```

### Known Challenges

*   **Signature Casting**: Proper handling of GUID casting is required, particularly for type-safety and generics.
*   **Representation Mapping**: Special handling is needed for JavaScript/Python representations; for example, `IVector` may need to map to a function rather than a simple interface instance.
*   **Async Operations**: While mentioned, the handling of asynchronous operations requires robust implementation details.

## References

*   **Legacy JS Projection**: Historical JavaScript applications utilized a dynamic projection where the runtime read WinMDs, achieving acceptable performance.
*   [PyWinRT](https://github.com/pywinrt/pywinrt) Static C++/WinRT-based projections demonstrated significant versioning and distribution challenges.
*  [lazy-winrt](https://github.com/JesseCol/lazy-winrt) **"Lazy-WinRT" Prototype**: This prototype validated the feasibility and potential performance of parsing WinMDs and invoking methods dynamically.
*  [dynwinrt](https://github.com/microsoft/dynwinrt) — Rust-based implementation inspired by Lazy-WinRT. Ships the core runtime, JS bindings via `napi-rs`, Python bindings via `PyO3`, and the `dynwinrt-codegen` code-generation tool.
