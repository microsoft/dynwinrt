// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumeratorElementKind {
    Interface,
    Struct,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumeratorDirection {
    Out,
    InOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EnumeratorContract {
    pub interface_namespace: &'static str,
    pub interface_name: &'static str,
    pub interface_iid: &'static str,
    pub next_vtable_index: usize,
    pub element_namespace: &'static str,
    pub element_name: &'static str,
    pub element_kind: EnumeratorElementKind,
    pub element_iid: Option<&'static str>,
    pub citation: &'static str,
}

impl EnumeratorContract {
    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_method_entry_id(
            self.family_id(),
            self.interface_namespace,
            self.interface_name,
            self.interface_iid,
            "Next",
            self.next_vtable_index,
        )
    }

    pub(crate) const fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        crate::contract_registry::ExactFamilyId::EnumeratorException
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::EnumeratorNext
    }

    pub(crate) fn uses_generic_standard(&self) -> bool {
        self.citation == STANDARD_NEXT
    }
}

use EnumeratorElementKind::{Interface, Struct, Unknown};

const STANDARD_NEXT: &str =
    "https://learn.microsoft.com/windows/win32/api/unknwn/nf-unknwn-ienumunknown-next";

const fn entry(
    interface_namespace: &'static str,
    interface_name: &'static str,
    interface_iid: &'static str,
    next_vtable_index: usize,
    element_namespace: &'static str,
    element_name: &'static str,
    element_kind: EnumeratorElementKind,
    element_iid: Option<&'static str>,
    citation: &'static str,
) -> EnumeratorContract {
    EnumeratorContract {
        interface_namespace,
        interface_name,
        interface_iid,
        next_vtable_index,
        element_namespace,
        element_name,
        element_kind,
        element_iid,
        citation,
    }
}

// DocumentationAttribute URLs are preserved where Windows.Win32 metadata
// supplies them. Entries using STANDARD_NEXT have no attached URL in
// Microsoft.Windows.SDK.Win32Metadata 71.0.14-preview; the citation supplies
// the standard Microsoft Next contract while every identity and element fact
// remains an exact Windows.Win32.winmd registry key.
const ENUMERATOR_CONTRACTS: &[EnumeratorContract] = &[
    entry(
        "Windows.Win32.System.Com",
        "IEnumGUID",
        "0002e000-0000-0000-c000-000000000046",
        3,
        "System",
        "Guid",
        Unknown,
        None,
        "https://learn.microsoft.com/windows/win32/api/objidl/nn-objidl-ienumguid",
    ),
    entry(
        "Windows.Win32.System.Com",
        "IEnumConnectionPoints",
        "b196b285-bab4-101a-b69c-00aa00341d07",
        3,
        "Windows.Win32.System.Com",
        "IConnectionPoint",
        Interface,
        Some("b196b286-bab4-101a-b69c-00aa00341d07"),
        "https://learn.microsoft.com/windows/win32/api/ocidl/nn-ocidl-ienumconnectionpoints",
    ),
    entry(
        "Windows.Win32.System.Ole",
        "IEnumVARIANT",
        "00020404-0000-0000-c000-000000000046",
        3,
        "Windows.Win32.System.Variant",
        "VARIANT",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/oaidl/nn-oaidl-ienumvariant",
    ),
    entry(
        "Windows.Win32.System.Com",
        "IEnumString",
        "00000101-0000-0000-c000-000000000046",
        3,
        "Windows.Win32.Foundation",
        "PWSTR",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/objidl/nn-objidl-ienumstring",
    ),
    entry(
        "Windows.Win32.System.Com",
        "IEnumUnknown",
        "00000100-0000-0000-c000-000000000046",
        3,
        "Windows.Win32.System.Com",
        "IUnknown",
        Interface,
        Some("00000000-0000-0000-c000-000000000046"),
        "https://learn.microsoft.com/windows/win32/api/unknwn/nn-unknwn-iunknown",
    ),
    entry(
        "Windows.Win32.Storage.VirtualDiskService",
        "IEnumVdsObject",
        "118610b7-8d94-4030-b5b8-500889788e4e",
        3,
        "Windows.Win32.System.Com",
        "IUnknown",
        Interface,
        Some("00000000-0000-0000-c000-000000000046"),
        "https://learn.microsoft.com/windows/win32/api/vds/nn-vds-ienumvdsobject",
    ),
    entry(
        "Windows.Win32.System.Com.Events",
        "IEnumEventObject",
        "f4a07d63-2e25-11d1-9964-00c04fbbb345",
        4,
        "Windows.Win32.System.Com",
        "IUnknown",
        Interface,
        Some("00000000-0000-0000-c000-000000000046"),
        "https://learn.microsoft.com/windows/win32/api/eventsys/nn-eventsys-ienumeventobject",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumITfCompositionView",
        "5efd22ba-7838-46cb-88e2-cadb14124f8f",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfCompositionView",
        Interface,
        Some("d7540241-f9a1-4364-befc-dbcd2c4395b7"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nn-msctf-ienumitfcompositionview",
    ),
    entry(
        "Windows.Win32.System.Search",
        "IEnumSearchRoots",
        "ab310581-ac80-11d1-8df3-00c04fb6ef52",
        3,
        "Windows.Win32.System.Search",
        "ISearchRoot",
        Interface,
        Some("04c18ccf-1f57-4cbd-88cc-3900f5195ce3"),
        "https://learn.microsoft.com/windows/win32/api/searchapi/nf-searchapi-ienumsearchroots-next",
    ),
    entry(
        "Windows.Win32.System.Search",
        "IEnumSearchScopeRules",
        "ab310581-ac80-11d1-8df3-00c04fb6ef54",
        3,
        "Windows.Win32.System.Search",
        "ISearchScopeRule",
        Interface,
        Some("ab310581-ac80-11d1-8df3-00c04fb6ef53"),
        "https://learn.microsoft.com/windows/win32/api/searchapi/nf-searchapi-ienumsearchscoperules-next",
    ),
    entry(
        "Windows.Win32.System.Search",
        "IEnumSubscription",
        "f72c8d97-6dbd-11d1-a1e8-00c04fc2fbe1",
        3,
        "System",
        "Guid",
        Unknown,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Storage.FileSystem",
        "IEnumDiskQuotaUsers",
        "7988b577-ec89-11cf-9c00-00aa00a14f56",
        3,
        "Windows.Win32.Storage.FileSystem",
        "IDiskQuotaUser",
        Interface,
        Some("7988b574-ec89-11cf-9c00-00aa00a14f56"),
        "https://learn.microsoft.com/windows/win32/api/dskquota/nf-dskquota-ienumdiskquotausers-next",
    ),
    entry(
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmString",
        "0adda826-2c26-11d2-ad65-00a0c9af11a6",
        3,
        "Windows.Win32.Foundation",
        "BSTR",
        Struct,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmError",
        "0adda829-2c26-11d2-ad65-00a0c9af11a6",
        3,
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IMsmError",
        Interface,
        Some("0adda828-2c26-11d2-ad65-00a0c9af11a6"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmDependency",
        "0adda82c-2c26-11d2-ad65-00a0c9af11a6",
        3,
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IMsmDependency",
        Interface,
        Some("0adda82b-2c26-11d2-ad65-00a0c9af11a6"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Storage.Imapi",
        "IEnumDiscMasterFormats",
        "ddf445e1-54ba-11d3-9144-00104ba11c5e",
        3,
        "System",
        "Guid",
        Unknown,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Storage.Imapi",
        "IEnumDiscRecorders",
        "9b1921e1-54ac-11d3-9144-00104ba11c5e",
        3,
        "Windows.Win32.Storage.Imapi",
        "IDiscRecorder",
        Interface,
        Some("85ac9776-ca88-4cf2-894e-09598c078a41"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Storage.Imapi",
        "IEnumProgressItems",
        "2c941fd6-975b-59be-a960-9a2a262853a5",
        3,
        "Windows.Win32.Storage.Imapi",
        "IProgressItem",
        Interface,
        Some("2c941fd5-975b-59be-a960-9a2a262853a5"),
        "https://learn.microsoft.com/windows/win32/api/imapi2fs/nf-imapi2fs-ienumprogressitems-next",
    ),
    entry(
        "Windows.Win32.Storage.Imapi",
        "IEnumFsiItems",
        "2c941fda-975b-59be-a960-9a2a262853a5",
        3,
        "Windows.Win32.Storage.Imapi",
        "IFsiItem",
        Interface,
        Some("2c941fd9-975b-59be-a960-9a2a262853a5"),
        "https://learn.microsoft.com/windows/win32/api/imapi2fs/nf-imapi2fs-ienumfsiitems-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyJobs",
        "1af4f612-3b71-466f-8f58-7b6f73ac57ad",
        3,
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IBackgroundCopyJob",
        Interface,
        Some("37668d37-507e-4160-9316-26306d150b12"),
        "https://learn.microsoft.com/windows/win32/api/bits/nf-bits-ienumbackgroundcopyjobs-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyGroups",
        "d993e603-4aa4-47c5-8665-c20d39c2ba4f",
        3,
        "System",
        "Guid",
        Unknown,
        None,
        "https://learn.microsoft.com/windows/win32/api/qmgr/nf-qmgr-ienumbackgroundcopygroups-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBitsPeers",
        "659cdea5-489e-11d9-a9cd-000d56965251",
        3,
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IBitsPeer",
        Interface,
        Some("659cdea2-489e-11d9-a9cd-000d56965251"),
        "https://learn.microsoft.com/windows/win32/api/bits3_0/nf-bits3_0-ienumbitspeers-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyFiles",
        "ca51e165-c365-424c-8d41-24aaa4ff3c40",
        3,
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IBackgroundCopyFile",
        Interface,
        Some("01b7bd23-fb88-4a77-8490-5891d3e4653a"),
        "https://learn.microsoft.com/windows/win32/api/bits/nf-bits-ienumbackgroundcopyfiles-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBitsPeerCacheRecords",
        "659cdea4-489e-11d9-a9cd-000d56965251",
        3,
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IBitsPeerCacheRecord",
        Interface,
        Some("659cdeaf-489e-11d9-a9cd-000d56965251"),
        "https://learn.microsoft.com/windows/win32/api/bits3_0/nf-bits3_0-ienumbitspeercacherecords-next",
    ),
    entry(
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyJobs1",
        "8baeba9d-8f1c-42c4-b82c-09ae79980d25",
        3,
        "System",
        "Guid",
        Unknown,
        None,
        "https://learn.microsoft.com/windows/win32/api/qmgr/nf-qmgr-ienumbackgroundcopyjobs1-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumGuideDataProperties",
        "ae44423b-4571-475c-ad2c-f40a771d80ef",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "IGuideDataProperty",
        Interface,
        Some("88ec5e58-bb73-41d6-99ce-66c524b8b591"),
        "https://learn.microsoft.com/windows/win32/api/bdatif/nf-bdatif-ienumguidedataproperties-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumComponents",
        "2a6e2939-2595-11d3-b64c-00c04f79498e",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "IComponent",
        Interface,
        Some("1a5576fc-0e19-11d3-9d8e-00c04f72d980"),
        "https://learn.microsoft.com/windows/win32/api/tuner/nf-tuner-ienumcomponents-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumMSVidGraphSegment",
        "3dd2903e-e0aa-11d2-b63a-00c04f79498e",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "IMSVidGraphSegment",
        Interface,
        Some("238dec54-adeb-4005-a349-f772b9afebc4"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumTuneRequests",
        "1993299c-ced6-4788-87a3-420067dce0c7",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "ITuneRequest",
        Interface,
        Some("07ddc146-fc3d-11d2-9d8c-00c04f72d980"),
        "https://learn.microsoft.com/windows/win32/api/bdatif/nf-bdatif-ienumtunerequests-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumTuningSpaces",
        "8b8eb248-fc2b-11d2-9d8c-00c04f72d980",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "ITuningSpace",
        Interface,
        Some("061c6e30-e622-11d2-9493-00c04f72d980"),
        "https://learn.microsoft.com/windows/win32/api/tuner/nf-tuner-ienumtuningspaces-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow.Tv",
        "IEnumComponentTypes",
        "8a674b4a-1f63-11d3-b64c-00c04f79498e",
        3,
        "Windows.Win32.Media.DirectShow.Tv",
        "IComponentType",
        Interface,
        Some("6a340dc0-0311-11d3-9d8e-00c04f72d980"),
        "https://learn.microsoft.com/windows/win32/api/tuner/nf-tuner-ienumcomponenttypes-next",
    ),
    entry(
        "Windows.Win32.Globalization",
        "IEnumCodePage",
        "275c23e3-3747-11d0-9fea-00aa003f8646",
        4,
        "Windows.Win32.Globalization",
        "MIMECPINFO",
        Struct,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Globalization",
        "IEnumScript",
        "ae5f1430-388b-11d2-8380-00c04f8f5da1",
        4,
        "Windows.Win32.Globalization",
        "SCRIPTINFO",
        Struct,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Globalization",
        "IEnumRfc1766",
        "3dc39d1d-c030-11d0-b81b-00c04fc9b31f",
        4,
        "Windows.Win32.Globalization",
        "RFC1766INFO",
        Struct,
        None,
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgBindingInterface",
        "c0e8ae90-306e-11d1-aacf-00805fc1270e",
        3,
        "Windows.Win32.NetworkManagement.NetManagement",
        "INetCfgBindingInterface",
        Interface,
        Some("c0e8ae94-306e-11d1-aacf-00805fc1270e"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgBindingPath",
        "c0e8ae91-306e-11d1-aacf-00805fc1270e",
        3,
        "Windows.Win32.NetworkManagement.NetManagement",
        "INetCfgBindingPath",
        Interface,
        Some("c0e8ae96-306e-11d1-aacf-00805fc1270e"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgComponent",
        "c0e8ae92-306e-11d1-aacf-00805fc1270e",
        3,
        "Windows.Win32.NetworkManagement.NetManagement",
        "INetCfgComponent",
        Interface,
        Some("c0e8ae99-306e-11d1-aacf-00805fc1270e"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWIA_FORMAT_INFO",
        "81befc5b-656d-44f1-b24c-d41d51b4dc81",
        3,
        "Windows.Win32.Devices.ImageAcquisition",
        "WIA_FORMAT_INFO",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/wia_xp/nf-wia_xp-ienumwia_format_info-next",
    ),
    entry(
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWiaItem2",
        "59970af4-cd0d-44d9-ab24-52295630e582",
        3,
        "Windows.Win32.Devices.ImageAcquisition",
        "IWiaItem2",
        Interface,
        Some("6cba0075-1287-407d-9b77-cf0e030435cc"),
        "https://learn.microsoft.com/windows/win32/wia/-wia-ienumwiaitem2-next",
    ),
    entry(
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWIA_DEV_INFO",
        "5e38b83c-8cf1-11d1-bf92-0060081ed811",
        3,
        "Windows.Win32.Devices.ImageAcquisition",
        "IWiaPropertyStorage",
        Interface,
        Some("98b5e8a0-29cc-491a-aac0-e6db4fdcceb6"),
        "https://learn.microsoft.com/windows/win32/api/wia_xp/nf-wia_xp-ienumwia_dev_info-next",
    ),
    entry(
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWiaItem",
        "5e8383fc-3391-11d2-9a33-00c04fa36145",
        3,
        "Windows.Win32.Devices.ImageAcquisition",
        "IWiaItem",
        Interface,
        Some("4db1ad10-3391-11d2-9a33-00c04fa36145"),
        "https://learn.microsoft.com/windows/win32/api/wia_xp/nf-wia_xp-ienumwiaitem-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WiFi",
        "IEnumDot11AdHocNetworks",
        "8f10cc28-cf0d-42a0-acbe-e2de7007384d",
        3,
        "Windows.Win32.NetworkManagement.WiFi",
        "IDot11AdHocNetwork",
        Interface,
        Some("8f10cc29-cf0d-42a0-acbe-e2de7007384d"),
        "https://learn.microsoft.com/windows/win32/api/adhoc/nf-adhoc-ienumdot11adhocnetworks-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WiFi",
        "IEnumDot11AdHocInterfaces",
        "8f10cc2c-cf0d-42a0-acbe-e2de7007384d",
        3,
        "Windows.Win32.NetworkManagement.WiFi",
        "IDot11AdHocInterface",
        Interface,
        Some("8f10cc2b-cf0d-42a0-acbe-e2de7007384d"),
        "https://learn.microsoft.com/windows/win32/api/adhoc/nf-adhoc-ienumdot11adhocinterfaces-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WiFi",
        "IEnumDot11AdHocSecuritySettings",
        "8f10cc2d-cf0d-42a0-acbe-e2de7007384d",
        3,
        "Windows.Win32.NetworkManagement.WiFi",
        "IDot11AdHocSecuritySettings",
        Interface,
        Some("8f10cc2e-cf0d-42a0-acbe-e2de7007384d"),
        "https://learn.microsoft.com/windows/win32/api/adhoc/nf-adhoc-ienumdot11adhocsecuritysettings-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfRanges",
        "f99d3f40-8e32-11d2-bf46-00105a2799b5",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfRange",
        Interface,
        Some("aa80e7ff-2021-11d2-93e0-0060b067b86e"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfranges-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfDisplayAttributeInfo",
        "7cef04d7-cb75-4e80-a7ab-5f5bc7d332de",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfDisplayAttributeInfo",
        Interface,
        Some("70528852-2f26-4aea-8c96-215150578932"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfdisplayattributeinfo-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfContextViews",
        "f0c0f8dd-cf38-44e1-bb0f-68cf0d551c78",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfContextView",
        Interface,
        Some("2433bf8e-0f9b-435c-ba2c-180611978c30"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfUIElements",
        "887aa91e-acba-4931-84da-3c5208cf543f",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfUIElement",
        Interface,
        Some("ea1ea137-19df-11d7-a6d2-00065b84435c"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfuielements-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfCandidates",
        "defb1926-6c80-4ce8-87d4-d6b72b812bde",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfCandidateString",
        Interface,
        Some("581f317e-fd9d-443f-b972-ed00467c5d40"),
        "https://learn.microsoft.com/windows/win32/api/ctffunc/nf-ctffunc-ienumtfcandidates-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfLangBarItems",
        "583f34d0-de25-11d2-afdd-00105a2799b5",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfLangBarItem",
        Interface,
        Some("73540d69-edeb-4ee9-96c9-23aa30b25916"),
        "https://learn.microsoft.com/windows/win32/api/ctfutb/nf-ctfutb-ienumtflangbaritems-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfFunctionProviders",
        "e4b24db0-0990-11d3-8df0-00105a2799b5",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfFunctionProvider",
        Interface,
        Some("101d6610-0990-11d3-8df0-00105a2799b5"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtffunctionproviders-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfProperties",
        "19188cb0-aca9-11d2-afc5-00105a2799b5",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfProperty",
        Interface,
        Some("e2449660-9542-11d2-bf46-00105a2799b5"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfproperties-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfDocumentMgrs",
        "aa80e808-2021-11d2-93e0-0060b067b86e",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfDocumentMgr",
        Interface,
        Some("aa80e7f4-2021-11d2-93e0-0060b067b86e"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfdocumentmgrs-next",
    ),
    entry(
        "Windows.Win32.UI.TextServices",
        "IEnumTfContexts",
        "8f1a7ea6-1654-4502-a86e-b2902344d507",
        4,
        "Windows.Win32.UI.TextServices",
        "ITfContext",
        Interface,
        Some("aa80e7fd-2021-11d2-93e0-0060b067b86e"),
        "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-ienumtfcontexts-next",
    ),
    entry(
        "Windows.Win32.System.Com",
        "IEnumMoniker",
        "00000102-0000-0000-c000-000000000046",
        3,
        "Windows.Win32.System.Com",
        "IMoniker",
        Interface,
        Some("0000000f-0000-0000-c000-000000000046"),
        "https://learn.microsoft.com/windows/win32/api/objidl/nf-objidl-ienummoniker-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumSyncProviderConfigUIInfos",
        "f6be2602-17c6-4658-a2d7-68ed3330f641",
        3,
        "Windows.Win32.System.WindowsSync",
        "ISyncProviderConfigUIInfo",
        Interface,
        Some("214141ae-33d7-4d8d-8e37-f227e880ce50"),
        "https://learn.microsoft.com/windows/win32/api/syncregistration/nf-syncregistration-ienumsyncproviderconfiguiinfos-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumSingleItemExceptions",
        "e563381c-1b4d-4c66-9796-c86faccdcd40",
        3,
        "Windows.Win32.System.WindowsSync",
        "ISingleItemException",
        Interface,
        Some("892fb9b0-7c55-4a18-9316-fdf449569b64"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumsingleitemexceptions-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumSyncChangeUnits",
        "346b35f1-8703-4c6d-ab1a-4dbca2cff97f",
        3,
        "Windows.Win32.System.WindowsSync",
        "ISyncChangeUnit",
        Interface,
        Some("60edd8ca-7341-4bb7-95ce-fab6394b51cb"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumsyncchangeunits-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumSyncProviderInfos",
        "a04ba850-5eb1-460d-a973-393fcb608a11",
        3,
        "Windows.Win32.System.WindowsSync",
        "ISyncProviderInfo",
        Interface,
        Some("1ee135de-88a4-4504-b0d0-f7920d7e5ba6"),
        "https://learn.microsoft.com/windows/win32/api/syncregistration/nf-syncregistration-ienumsyncproviderinfos-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumClockVector",
        "525844db-2837-4799-9e80-81a66e02220c",
        3,
        "Windows.Win32.System.WindowsSync",
        "IClockVectorElement",
        Interface,
        Some("e71c4250-adf8-4a07-8fae-5669596909c1"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumclockvector-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumChangeUnitExceptions",
        "3074e802-9319-4420-be21-1022e2e21da8",
        3,
        "Windows.Win32.System.WindowsSync",
        "IChangeUnitException",
        Interface,
        Some("0cd7ee7c-fec0-4021-99ee-f0e5348f2a5f"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumchangeunitexceptions-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumSyncChanges",
        "5f86be4a-5e78-4e32-ac1c-c24fd223ef85",
        3,
        "Windows.Win32.System.WindowsSync",
        "ISyncChange",
        Interface,
        Some("a1952beb-0f6b-4711-b136-01da85b968a6"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumsyncchanges-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumRangeExceptions",
        "0944439f-ddb1-4176-b703-046ff22a2386",
        3,
        "Windows.Win32.System.WindowsSync",
        "IRangeException",
        Interface,
        Some("75ae8777-6848-49f7-956c-a3a92f5096e8"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumrangeexceptions-next",
    ),
    entry(
        "Windows.Win32.System.WindowsSync",
        "IEnumFeedClockVector",
        "550f763d-146a-48f6-abeb-6c88c7f70514",
        3,
        "Windows.Win32.System.WindowsSync",
        "IFeedClockVectorElement",
        Interface,
        Some("a40b46d2-e97b-4156-b6da-991f501b0f05"),
        "https://learn.microsoft.com/windows/win32/api/winsync/nf-winsync-ienumfeedclockvector-next",
    ),
    entry(
        "Windows.Win32.Media.Speech",
        "IEnumSpObjectTokens",
        "06b64f9e-7fda-11d2-b4f2-00c04f797396",
        3,
        "Windows.Win32.Media.Speech",
        "ISpObjectToken",
        Interface,
        Some("14056589-e16c-11d2-bb90-00c04f8ee6c0"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Web.InternetExplorer",
        "IEnumOpenServiceActivityCategory",
        "33627a56-8c9a-4430-8fd1-b5f5c771afb6",
        3,
        "Windows.Win32.Web.InternetExplorer",
        "IOpenServiceActivityCategory",
        Interface,
        Some("850af9d6-7309-40b5-bdb8-786c106b2153"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Web.InternetExplorer",
        "IEnumOpenServiceActivity",
        "a436d7d2-17c3-4ef4-a1e8-5c86faff26c0",
        3,
        "Windows.Win32.Web.InternetExplorer",
        "IOpenServiceActivity",
        Interface,
        Some("13645c88-221a-4905-8ed1-4f5112cfc108"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Media.DirectShow",
        "IEnumPins",
        "56a86892-0ad4-11ce-b03a-0020af0ba770",
        3,
        "Windows.Win32.Media.DirectShow",
        "IPin",
        Interface,
        Some("56a86891-0ad4-11ce-b03a-0020af0ba770"),
        "https://learn.microsoft.com/windows/win32/api/strmif/nf-strmif-ienumpins-next",
    ),
    entry(
        "Windows.Win32.Media.DirectShow",
        "IEnumFilters",
        "56a86893-0ad4-11ce-b03a-0020af0ba770",
        3,
        "Windows.Win32.Media.DirectShow",
        "IBaseFilter",
        Interface,
        Some("56a86895-0ad4-11ce-b03a-0020af0ba770"),
        "https://learn.microsoft.com/windows/win32/api/strmif/nf-strmif-ienumfilters-next",
    ),
    entry(
        "Windows.Win32.Devices.Tapi",
        "IEnumQueue",
        "5afc3158-4bcc-11d1-bf80-00805fc147d3",
        3,
        "Windows.Win32.Devices.Tapi",
        "ITQueue",
        Interface,
        Some("5afc3149-4bcc-11d1-bf80-00805fc147d3"),
        "https://learn.microsoft.com/windows/win32/api/tapi3cc/nf-tapi3cc-ienumqueue-next",
    ),
    entry(
        "Windows.Win32.Devices.Tapi",
        "IEnumAgentSession",
        "5afc314e-4bcc-11d1-bf80-00805fc147d3",
        3,
        "Windows.Win32.Devices.Tapi",
        "ITAgentSession",
        Interface,
        Some("5afc3147-4bcc-11d1-bf80-00805fc147d3"),
        "https://learn.microsoft.com/windows/win32/api/tapi3cc/nf-tapi3cc-ienumagentsession-next",
    ),
    entry(
        "Windows.Win32.Devices.Tapi",
        "IEnumACDGroup",
        "5afc3157-4bcc-11d1-bf80-00805fc147d3",
        3,
        "Windows.Win32.Devices.Tapi",
        "ITACDGroup",
        Interface,
        Some("5afc3148-4bcc-11d1-bf80-00805fc147d3"),
        "https://learn.microsoft.com/windows/win32/api/tapi3cc/nf-tapi3cc-ienumacdgroup-next",
    ),
    entry(
        "Windows.Win32.Devices.Tapi",
        "IEnumAgent",
        "5afc314d-4bcc-11d1-bf80-00805fc147d3",
        3,
        "Windows.Win32.Devices.Tapi",
        "ITAgent",
        Interface,
        Some("5770ece5-4b27-11d1-bf80-00805fc147d3"),
        "https://learn.microsoft.com/windows/win32/api/tapi3cc/nf-tapi3cc-ienumagent-next",
    ),
    entry(
        "Windows.Win32.Devices.Tapi",
        "IEnumAgentHandler",
        "587e8c28-9802-11d1-a0a4-00805fc147d3",
        3,
        "Windows.Win32.Devices.Tapi",
        "ITAgentHandler",
        Interface,
        Some("587e8c22-9802-11d1-a0a4-00805fc147d3"),
        "https://learn.microsoft.com/windows/win32/api/tapi3cc/nf-tapi3cc-ienumagenthandler-next",
    ),
    entry(
        "Windows.Win32.System.Ole",
        "IEnumOleUndoUnits",
        "b3e7c340-ef97-11ce-9bc9-00aa00608e01",
        3,
        "Windows.Win32.System.Ole",
        "IOleUndoUnit",
        Interface,
        Some("894ad3b0-ef97-11ce-9bc9-00aa00608e01"),
        "https://learn.microsoft.com/windows/win32/api/ocidl/nf-ocidl-ienumoleundounits-next",
    ),
    entry(
        "Windows.Win32.System.Ole",
        "IEnumOleDocumentViews",
        "b722bcc8-4e68-101b-a2bc-00aa00404770",
        3,
        "Windows.Win32.System.Ole",
        "IOleDocumentView",
        Interface,
        Some("b722bcc6-4e68-101b-a2bc-00aa00404770"),
        "https://learn.microsoft.com/windows/win32/api/docobj/nf-docobj-ienumoledocumentviews-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "IEnumNetSharingPrivateConnection",
        "c08956b5-1cd3-11d1-b1c5-00805fc1270e",
        3,
        "Windows.Win32.System.Variant",
        "VARIANT",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/netcon/nf-netcon-ienumnetsharingprivateconnection-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "IEnumNetSharingPublicConnection",
        "c08956b4-1cd3-11d1-b1c5-00805fc1270e",
        3,
        "Windows.Win32.System.Variant",
        "VARIANT",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/netcon/nf-netcon-ienumnetsharingpublicconnection-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "IEnumNetConnection",
        "c08956a0-1cd3-11d1-b1c5-00805fc1270e",
        3,
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "INetConnection",
        Interface,
        Some("c08956a1-1cd3-11d1-b1c5-00805fc1270e"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "IEnumNetSharingEveryConnection",
        "c08956b8-1cd3-11d1-b1c5-00805fc1270e",
        3,
        "Windows.Win32.System.Variant",
        "VARIANT",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/netcon/nf-netcon-ienumnetsharingeveryconnection-next",
    ),
    entry(
        "Windows.Win32.NetworkManagement.WindowsFirewall",
        "IEnumNetSharingPortMapping",
        "c08956b0-1cd3-11d1-b1c5-00805fc1270e",
        3,
        "Windows.Win32.System.Variant",
        "VARIANT",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/netcon/nf-netcon-ienumnetsharingportmapping-next",
    ),
    entry(
        "Windows.Win32.Devices.PortableDevices",
        "IEnumPortableDeviceConnectors",
        "bfdef549-9247-454f-bd82-06fe80853faa",
        3,
        "Windows.Win32.Devices.PortableDevices",
        "IPortableDeviceConnector",
        Interface,
        Some("625e2df8-6392-4cf0-9ad1-3cfa5f17775c"),
        "https://learn.microsoft.com/windows/win32/wpd_sdk/ienumportabledeviceconnectors-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumShellItems",
        "70629033-e363-4a28-a567-0db78006e6d7",
        3,
        "Windows.Win32.UI.Shell",
        "IShellItem",
        Interface,
        Some("43826d1e-e718-42ee-bc55-a1e261c37bfe"),
        "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ienumshellitems-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumSyncMgrEvents",
        "c81a1d4e-8cf7-4683-80e0-bcae88d677b6",
        3,
        "Windows.Win32.UI.Shell",
        "ISyncMgrEvent",
        Interface,
        Some("fee0ef8b-46bd-4db4-b7e6-ff2c687313bc"),
        "https://learn.microsoft.com/windows/win32/api/syncmgr/nf-syncmgr-ienumsyncmgrevents-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumTravelLogEntry",
        "7ebfdd85-ad18-11d3-a4c5-00c04f72d6b8",
        3,
        "Windows.Win32.UI.Shell",
        "ITravelLogEntry",
        Interface,
        Some("7ebfdd87-ad18-11d3-a4c5-00c04f72d6b8"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumSyncMgrConflict",
        "82705914-dda3-4893-ba99-49de6c8c8036",
        3,
        "Windows.Win32.UI.Shell",
        "ISyncMgrConflict",
        Interface,
        Some("9c204249-c443-4ba4-85ed-c972681db137"),
        "https://learn.microsoft.com/windows/win32/api/syncmgr/nf-syncmgr-ienumsyncmgrconflict-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumSyncMgrSyncItems",
        "54b3abf3-f085-4181-b546-e29c403c726b",
        3,
        "Windows.Win32.UI.Shell",
        "ISyncMgrSyncItem",
        Interface,
        Some("b20b24ce-2593-4f04-bd8b-7ad6c45051cd"),
        "https://learn.microsoft.com/windows/win32/api/syncmgr/nf-syncmgr-ienumsyncmgrsyncitems-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumAssocHandlers",
        "973810ae-9599-4b88-9e4d-6ee98c9552da",
        3,
        "Windows.Win32.UI.Shell",
        "IAssocHandler",
        Interface,
        Some("f04061ac-1659-4a3f-a954-775aa57fc083"),
        "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ienumassochandlers-next",
    ),
    entry(
        "Windows.Win32.UI.Shell",
        "IEnumExplorerCommand",
        "a88826f8-186f-4987-aade-ea0cef8fbfe8",
        3,
        "Windows.Win32.UI.Shell",
        "IExplorerCommand",
        Interface,
        Some("a08ce4d0-fa25-44ab-b57c-c7b1c323e0b9"),
        "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ienumexplorercommand-next",
    ),
    entry(
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IEnumRemoteDebugApplications",
        "51973c3b-cb0c-11d0-b5c9-00a0244a0e7a",
        3,
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IRemoteDebugApplication",
        Interface,
        Some("51973c30-cb0c-11d0-b5c9-00a0244a0e7a"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IEnumDebugExpressionContexts",
        "51973c40-cb0c-11d0-b5c9-00a0244a0e7a",
        3,
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IDebugExpressionContext",
        Interface,
        Some("51973c15-cb0c-11d0-b5c9-00a0244a0e7a"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IEnumRemoteDebugApplicationThreads",
        "51973c3c-cb0c-11d0-b5c9-00a0244a0e7a",
        3,
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IRemoteDebugApplicationThread",
        Interface,
        Some("51973c37-cb0c-11d0-b5c9-00a0244a0e7a"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IEnumDebugApplicationNodes",
        "51973c3a-cb0c-11d0-b5c9-00a0244a0e7a",
        3,
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IDebugApplicationNode",
        Interface,
        Some("51973c34-cb0c-11d0-b5c9-00a0244a0e7a"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IEnumDebugCodeContexts",
        "51973c1d-cb0c-11d0-b5c9-00a0244a0e7a",
        3,
        "Windows.Win32.System.Diagnostics.Debug.ActiveScript",
        "IDebugCodeContext",
        Interface,
        Some("51973c13-cb0c-11d0-b5c9-00a0244a0e7a"),
        STANDARD_NEXT,
    ),
    entry(
        "Windows.Win32.Storage.OfflineFiles",
        "IEnumOfflineFilesSettings",
        "729680c4-1a38-47bc-9e5c-02c51562ac30",
        3,
        "Windows.Win32.Storage.OfflineFiles",
        "IOfflineFilesSetting",
        Interface,
        Some("d871d3f7-f613-48a1-827e-7a34e560fff6"),
        "https://learn.microsoft.com/windows/win32/api/cscobj/nf-cscobj-ienumofflinefilessettings-next",
    ),
    entry(
        "Windows.Win32.Storage.OfflineFiles",
        "IEnumOfflineFilesItems",
        "da70e815-c361-4407-bc0b-0d7046e5f2cd",
        3,
        "Windows.Win32.Storage.OfflineFiles",
        "IOfflineFilesItem",
        Interface,
        Some("4a753da6-e044-4f12-a718-5d14d079a906"),
        "https://learn.microsoft.com/windows/win32/api/cscobj/nf-cscobj-ienumofflinefilesitems-next",
    ),
    entry(
        "Windows.Win32.System.ComponentServices",
        "IEnumNames",
        "51372af2-cae7-11cf-be81-00aa00a2fa25",
        3,
        "Windows.Win32.Foundation",
        "BSTR",
        Struct,
        None,
        "https://learn.microsoft.com/windows/win32/api/comsvcs/nf-comsvcs-ienumnames-next",
    ),
];

const INOUT_FETCHED_DECLARATIONS: &[(&str, &str)] = &[
    ("Windows.Win32.System.Search", "IEnumSearchRoots"),
    ("Windows.Win32.System.Search", "IEnumSearchScopeRules"),
    ("Windows.Win32.Storage.FileSystem", "IEnumDiskQuotaUsers"),
    (
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmString",
    ),
    (
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmError",
    ),
    (
        "Windows.Win32.System.ApplicationInstallationAndServicing",
        "IEnumMsmDependency",
    ),
    (
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyJobs",
    ),
    (
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBitsPeers",
    ),
    (
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBackgroundCopyFiles",
    ),
    (
        "Windows.Win32.Networking.BackgroundIntelligentTransferService",
        "IEnumBitsPeerCacheRecords",
    ),
    (
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWIA_FORMAT_INFO",
    ),
    ("Windows.Win32.Devices.ImageAcquisition", "IEnumWiaItem2"),
    (
        "Windows.Win32.Devices.ImageAcquisition",
        "IEnumWIA_DEV_INFO",
    ),
    ("Windows.Win32.Devices.ImageAcquisition", "IEnumWiaItem"),
    ("Windows.Win32.UI.TextServices", "IEnumTfLangBarItems"),
    (
        "Windows.Win32.System.WindowsSync",
        "IEnumSingleItemExceptions",
    ),
    ("Windows.Win32.System.WindowsSync", "IEnumSyncChangeUnits"),
    ("Windows.Win32.System.WindowsSync", "IEnumClockVector"),
    (
        "Windows.Win32.System.WindowsSync",
        "IEnumChangeUnitExceptions",
    ),
    ("Windows.Win32.System.WindowsSync", "IEnumSyncChanges"),
    ("Windows.Win32.System.WindowsSync", "IEnumRangeExceptions"),
    ("Windows.Win32.System.WindowsSync", "IEnumFeedClockVector"),
    (
        "Windows.Win32.Devices.PortableDevices",
        "IEnumPortableDeviceConnectors",
    ),
    ("Windows.Win32.System.ComponentServices", "IEnumNames"),
];

const OPTIONAL_FETCHED_DECLARATIONS: &[(&str, &str)] = &[
    ("Windows.Win32.System.Com", "IEnumGUID"),
    ("Windows.Win32.System.Com", "IEnumString"),
    ("Windows.Win32.System.Com", "IEnumUnknown"),
    ("Windows.Win32.Globalization", "IEnumCodePage"),
    ("Windows.Win32.Globalization", "IEnumScript"),
    ("Windows.Win32.Globalization", "IEnumRfc1766"),
    (
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgBindingInterface",
    ),
    (
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgBindingPath",
    ),
    (
        "Windows.Win32.NetworkManagement.NetManagement",
        "IEnumNetCfgComponent",
    ),
    ("Windows.Win32.System.Com", "IEnumMoniker"),
    ("Windows.Win32.Media.Speech", "IEnumSpObjectTokens"),
    ("Windows.Win32.Media.DirectShow", "IEnumPins"),
    ("Windows.Win32.Media.DirectShow", "IEnumFilters"),
    ("Windows.Win32.UI.Shell", "IEnumShellItems"),
    ("Windows.Win32.UI.Shell", "IEnumAssocHandlers"),
    ("Windows.Win32.UI.Shell", "IEnumExplorerCommand"),
];

fn declaration_is_in(
    declarations: &[(&str, &str)],
    interface_namespace: &str,
    interface_name: &str,
) -> bool {
    declarations
        .iter()
        .any(|(namespace, name)| *namespace == interface_namespace && *name == interface_name)
}

pub(crate) fn fetched_shape(
    interface_namespace: &str,
    interface_name: &str,
) -> (EnumeratorDirection, bool) {
    let direction = if declaration_is_in(
        INOUT_FETCHED_DECLARATIONS,
        interface_namespace,
        interface_name,
    ) {
        EnumeratorDirection::InOut
    } else {
        EnumeratorDirection::Out
    };
    let optional = declaration_is_in(
        OPTIONAL_FETCHED_DECLARATIONS,
        interface_namespace,
        interface_name,
    );
    (direction, optional)
}

pub(crate) fn values_direction(
    interface_namespace: &str,
    interface_name: &str,
) -> EnumeratorDirection {
    if matches!(
        (interface_namespace, interface_name),
        (
            "Windows.Win32.System.ApplicationInstallationAndServicing",
            "IEnumMsmString"
        ) | (
            "Windows.Win32.Devices.ImageAcquisition",
            "IEnumWIA_FORMAT_INFO"
        ) | ("Windows.Win32.System.ComponentServices", "IEnumNames")
    ) {
        EnumeratorDirection::InOut
    } else {
        EnumeratorDirection::Out
    }
}

pub(crate) fn contract_for_declaration(
    interface_namespace: &str,
    interface_name: &str,
) -> Option<&'static EnumeratorContract> {
    ENUMERATOR_CONTRACTS.iter().find(|contract| {
        contract.interface_namespace == interface_namespace
            && contract.interface_name == interface_name
    })
}

pub(crate) fn exact_contract(
    interface_namespace: &str,
    interface_name: &str,
    interface_iid: &str,
    next_vtable_index: usize,
) -> Option<&'static EnumeratorContract> {
    contract_for_declaration(interface_namespace, interface_name).filter(|contract| {
        contract.interface_iid.eq_ignore_ascii_case(interface_iid)
            && contract.next_vtable_index == next_vtable_index
    })
}

pub(crate) fn contracts() -> &'static [EnumeratorContract] {
    ENUMERATOR_CONTRACTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_rejects_identity_and_slot_drift() {
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                3
            )
            .is_some()
        );
        assert!(
            exact_contract(
                "Contoso",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                3
            )
            .is_none()
        );
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
                3
            )
            .is_none()
        );
        assert!(
            exact_contract(
                "Windows.Win32.System.Com",
                "IEnumUnknown",
                "00000100-0000-0000-c000-000000000046",
                4
            )
            .is_none()
        );
    }

    #[test]
    fn declarations_are_unique() {
        for (index, contract) in ENUMERATOR_CONTRACTS.iter().enumerate() {
            assert!(
                !ENUMERATOR_CONTRACTS[..index].iter().any(|previous| {
                    previous.interface_namespace == contract.interface_namespace
                        && previous.interface_name == contract.interface_name
                }),
                "{}.{}",
                contract.interface_namespace,
                contract.interface_name
            );
            assert_eq!(
                contract.element_iid.is_some(),
                contract.element_kind == EnumeratorElementKind::Interface,
                "{}.{} element IID",
                contract.interface_namespace,
                contract.interface_name
            );
        }
    }

    #[test]
    fn generic_standard_next_entries_are_not_exact_exceptions() {
        assert_eq!(
            ENUMERATOR_CONTRACTS
                .iter()
                .filter(|contract| contract.uses_generic_standard())
                .count(),
            24
        );
        assert!(
            ENUMERATOR_CONTRACTS
                .iter()
                .filter(|contract| !contract.uses_generic_standard())
                .all(|contract| contract.citation != STANDARD_NEXT)
        );
    }
}
