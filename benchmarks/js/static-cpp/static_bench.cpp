// Pure C++/WinRT node addon for benchmarking static projections.
// Path: JS -> node-addon-api -> C++/WinRT -> COM vtable
// No Rust in the loop.

#include <napi.h>
#include <unknwn.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Devices.Geolocation.h>

namespace wf  = winrt::Windows::Foundation;
namespace wdg = winrt::Windows::Devices::Geolocation;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Release COM pointer (attached to External so GC handles cleanup)
static void release_unk(Napi::Env /*env*/, void* p) {
    if (p) reinterpret_cast<::IUnknown*>(p)->Release();
}

// Wrap a C++/WinRT object as Napi::External<void>.
// detach_abi transfers ownership (+1 ref) to the External.
template <typename T>
static Napi::Value wrap_obj(Napi::Env env, T obj) {
    void* raw = winrt::detach_abi(obj);
    return Napi::External<void>::New(env, raw, release_unk);
}

// Recover a typed C++/WinRT object from an External.
// copy_from_abi AddRefs; the returned value Releases on destruction.
// No QI — the raw pointer is already the correct interface type.
template <typename T>
static T unwrap_obj(const Napi::Value& val) {
    void* raw = val.As<Napi::External<void>>().Data();
    T obj{nullptr};
    winrt::copy_from_abi(obj, raw);
    return obj;
}

// JS string -> winrt::hstring (UTF-16)
static winrt::hstring to_hs(const Napi::Value& val) {
    auto u16 = val.As<Napi::String>().Utf16Value();
    return winrt::hstring{std::wstring_view{
        reinterpret_cast<const wchar_t*>(u16.data()), u16.size()}};
}

// winrt::hstring -> JS string
static Napi::Value from_hs(Napi::Env env, const winrt::hstring& hs) {
    std::wstring_view wv{hs};
    return Napi::String::New(env,
        reinterpret_cast<const char16_t*>(wv.data()), wv.size());
}

// ---------------------------------------------------------------------------
// Uri
// ---------------------------------------------------------------------------

// uriCreate(uriStr: string) -> External
Napi::Value UriCreate(const Napi::CallbackInfo& info) {
    auto uri = wf::Uri(to_hs(info[0]));
    return wrap_obj(info.Env(), uri);
}

// uriGetHost(uriStr: string) -> string   (create + read in one call)
Napi::Value UriGetHost(const Napi::CallbackInfo& info) {
    auto uri = wf::Uri(to_hs(info[0]));
    return from_hs(info.Env(), uri.Host());
}

// uriHostFromObj(uri: External) -> string
Napi::Value UriHostFromObj(const Napi::CallbackInfo& info) {
    return from_hs(info.Env(), unwrap_obj<wf::Uri>(info[0]).Host());
}

// uriPortFromObj(uri: External) -> number
Napi::Value UriPortFromObj(const Napi::CallbackInfo& info) {
    return Napi::Number::New(info.Env(),
        unwrap_obj<wf::Uri>(info[0]).Port());
}

// uriSuspiciousFromObj(uri: External) -> boolean
Napi::Value UriSuspiciousFromObj(const Napi::CallbackInfo& info) {
    return Napi::Boolean::New(info.Env(),
        unwrap_obj<wf::Uri>(info[0]).Suspicious());
}

// uriQueryParsedFromObj(uri: External) -> External (object)
Napi::Value UriQueryParsedFromObj(const Napi::CallbackInfo& info) {
    return wrap_obj(info.Env(), unwrap_obj<wf::Uri>(info[0]).QueryParsed());
}

// uriCombine(uri: External, relative: string) -> External
Napi::Value UriCombine(const Napi::CallbackInfo& info) {
    auto result = unwrap_obj<wf::Uri>(info[0]).CombineUri(to_hs(info[1]));
    return wrap_obj(info.Env(), result);
}

// uriCreateWithRelative(base: string, relative: string) -> External
Napi::Value UriCreateWithRelative(const Napi::CallbackInfo& info) {
    auto uri = wf::Uri(to_hs(info[0]), to_hs(info[1]));
    return wrap_obj(info.Env(), uri);
}

// ---------------------------------------------------------------------------
// PropertyValue
// ---------------------------------------------------------------------------

Napi::Value PvCreateI32(const Napi::CallbackInfo& info) {
    return wrap_obj(info.Env(),
        wf::PropertyValue::CreateInt32(info[0].As<Napi::Number>().Int32Value()));
}

Napi::Value PvCreateF64(const Napi::CallbackInfo& info) {
    return wrap_obj(info.Env(),
        wf::PropertyValue::CreateDouble(info[0].As<Napi::Number>().DoubleValue()));
}

Napi::Value PvCreateBool(const Napi::CallbackInfo& info) {
    return wrap_obj(info.Env(),
        wf::PropertyValue::CreateBoolean(info[0].As<Napi::Boolean>().Value()));
}

Napi::Value PvCreateString(const Napi::CallbackInfo& info) {
    return wrap_obj(info.Env(),
        wf::PropertyValue::CreateString(to_hs(info[0])));
}

// ---------------------------------------------------------------------------
// Geopoint
// ---------------------------------------------------------------------------

Napi::Value GeopointCreate(const Napi::CallbackInfo& info) {
    wdg::BasicGeoposition pos{
        info[0].As<Napi::Number>().DoubleValue(),
        info[1].As<Napi::Number>().DoubleValue(),
        info[2].As<Napi::Number>().DoubleValue(),
    };
    return wrap_obj(info.Env(), wdg::Geopoint(pos));
}

// ---------------------------------------------------------------------------
// Module init
// ---------------------------------------------------------------------------

Napi::Object Init(Napi::Env env, Napi::Object exports) {
    exports.Set("uriCreate",            Napi::Function::New(env, UriCreate));
    exports.Set("uriGetHost",           Napi::Function::New(env, UriGetHost));
    exports.Set("uriHostFromObj",       Napi::Function::New(env, UriHostFromObj));
    exports.Set("uriPortFromObj",       Napi::Function::New(env, UriPortFromObj));
    exports.Set("uriSuspiciousFromObj", Napi::Function::New(env, UriSuspiciousFromObj));
    exports.Set("uriQueryParsedFromObj", Napi::Function::New(env, UriQueryParsedFromObj));
    exports.Set("uriCombine",          Napi::Function::New(env, UriCombine));
    exports.Set("uriCreateWithRelative", Napi::Function::New(env, UriCreateWithRelative));
    exports.Set("pvCreateI32",          Napi::Function::New(env, PvCreateI32));
    exports.Set("pvCreateF64",          Napi::Function::New(env, PvCreateF64));
    exports.Set("pvCreateBool",         Napi::Function::New(env, PvCreateBool));
    exports.Set("pvCreateString",       Napi::Function::New(env, PvCreateString));
    exports.Set("geopointCreate",       Napi::Function::New(env, GeopointCreate));
    return exports;
}

NODE_API_MODULE(static_bench, Init)
