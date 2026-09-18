// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#include <windows.h>
#include <roapi.h>
#include <activation.h>
#include <wrl.h>
#include <wrl/module.h>
#include <wrl/wrappers/corewrappers.h>

using namespace Microsoft::WRL;
using Microsoft::WRL::Wrappers::HStringReference;

class Factory final : public ActivationFactory<>
{
public:
    HRESULT STDMETHODCALLTYPE ActivateInstance(IInspectable** result) override
    {
        if (!result)
            return E_POINTER;
        *result = nullptr;
        return E_NOTIMPL;
    }
};

extern "C" __declspec(dllexport) HRESULT WINAPI DllGetActivationFactory(
    HSTRING name, IActivationFactory** result)
{
    if (!result)
        return E_POINTER;
    *result = nullptr;
    const auto className = WindowsGetStringRawBuffer(name, nullptr);
    if (wcscmp(className, L"Microsoft.UI.Xaml.DynWinRTFixture") &&
        wcscmp(className, L"Microsoft.UI.Xaml.Controls.DynWinRTFixture"))
        return CLASS_E_CLASSNOTAVAILABLE;

    wchar_t mode[32]{};
    GetEnvironmentVariableW(L"DYNWINRT_WINUI_FIXTURE_MODE", mode, ARRAYSIZE(mode));
    if (!wcscmp(mode, L"fail"))
        return E_ACCESSDENIED;
    if (!wcscmp(mode, L"null"))
        return S_OK;
    if (!wcscmp(mode, L"forward"))
        return RoGetActivationFactory(
            HStringReference(L"Windows.Foundation.Uri").Get(), IID_PPV_ARGS(result));
    auto factory = Make<Factory>();
    return factory ? factory.CopyTo(result) : E_OUTOFMEMORY;
}

extern "C" __declspec(dllexport) unsigned long WINAPI FixtureObjectCount()
{
    return Module<InProc>::GetModule().GetObjectCount();
}
