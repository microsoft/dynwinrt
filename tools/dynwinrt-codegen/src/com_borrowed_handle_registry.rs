// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BorrowedHwndOutputEvidence {
    pub declaring_namespace: &'static str,
    pub declaring_interface: &'static str,
    pub declaring_iid: &'static str,
    pub method_name: &'static str,
    pub vtable_index: usize,
    pub parameter_count: usize,
    pub parameter_index: usize,
    pub parameter_name: &'static str,
    pub optional: bool,
    pub reason: &'static str,
    pub citation: &'static str,
}

impl BorrowedHwndOutputEvidence {
    pub(crate) fn entry_id(&self) -> String {
        crate::contract_registry::exact_parameter_entry_id(
            self.family_id(),
            self.declaring_namespace,
            self.declaring_interface,
            self.declaring_iid,
            self.method_name,
            self.vtable_index,
            self.parameter_index,
            self.parameter_name,
        )
    }

    pub(crate) const fn family_id(&self) -> crate::contract_registry::ExactFamilyId {
        crate::contract_registry::ExactFamilyId::BorrowedHwndOutput
    }

    pub(crate) fn entries() -> &'static [BorrowedHwndOutputEvidence] {
        BORROWED_HWND_OUTPUTS
    }

    pub(crate) const fn contract_kind(&self) -> crate::contract_registry::ContractKind {
        crate::contract_registry::ContractKind::BorrowedHandle
    }
}

const BORROWED_HWND_OUTPUTS: &[BorrowedHwndOutputEvidence] = &[
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Devices.ImageAcquisition",
        declaring_interface: "IWiaAppErrorHandler",
        declaring_iid: "6c16186c-d0a6-400c-80f4-d26986a0e734",
        method_name: "GetWindow",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as the existing WIA error-handler dialog HWND, which may be NULL and must remain valid for the transfer; the caller does not acquire window ownership",
        citation: "https://learn.microsoft.com/previous-versions/windows/desktop/wia/-wia-iwiaapperrorhandler-getwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.PictureAcquisition",
        declaring_interface: "IPhotoProgressDialog",
        declaring_iid: "00f246f9-0750-4f08-9381-2cd8e906a4ae",
        method_name: "GetWindow",
        vtable_index: 4,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwndProgressDialog",
        optional: false,
        reason: "Microsoft documents this getter as retrieving the progress dialog box handle; it does not create or transfer ownership of the dialog",
        citation: "https://learn.microsoft.com/windows/win32/api/photoacquire/nf-photoacquire-iphotoprogressdialog-getwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.DirectShow",
        declaring_interface: "IOverlay",
        declaring_iid: "56a868a1-0ad4-11ce-b03a-0020af0ba770",
        method_name: "GetWindowHandle",
        vtable_index: 8,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "pHwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving the clipping window handle already associated with the overlay; it does not create or transfer ownership of the window",
        citation: "https://learn.microsoft.com/windows/win32/api/strmif/nf-strmif-ioverlay-getwindowhandle",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.DirectShow.Tv",
        declaring_interface: "IMSVidCtl",
        declaring_iid: "b0edf162-910a-11d2-b632-00c04f79498e",
        method_name: "get_Window",
        vtable_index: 15,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this property as retrieving the existing video control window handle; the caller does not acquire ownership of that window",
        citation: "https://learn.microsoft.com/windows/win32/api/msvidctl/nf-msvidctl-imsvidctl-get_window",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.DirectShow.Tv",
        declaring_interface: "IMSVidRect",
        declaring_iid: "7f5000a6-a440-47ca-8acc-c0e75531a2c2",
        method_name: "get_HWnd",
        vtable_index: 15,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "HWndVal",
        optional: false,
        reason: "Microsoft documents this property as retrieving the window handle represented by the existing video rectangle; it does not create or transfer the window",
        citation: "https://learn.microsoft.com/windows/win32/api/segment/nf-segment-imsvidrect-get_hwnd",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.MediaFoundation",
        declaring_interface: "IMFPMediaPlayer",
        declaring_iid: "a714590a-58af-430a-85bf-44f5ec838d85",
        method_name: "GetVideoWindow",
        vtable_index: 31,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwndVideo",
        optional: false,
        reason: "Microsoft documents this as retrieving the media player's current video window; it is an existing caller-configured window and ownership is not transferred",
        citation: "https://learn.microsoft.com/windows/win32/api/mfplay/nf-mfplay-imfpmediaplayer-getvideowindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.Media.MediaFoundation",
        declaring_interface: "IMFVideoDisplayControl",
        declaring_iid: "a490b1e4-ab84-4d31-a1b2-181e03b1077a",
        method_name: "GetVideoWindow",
        vtable_index: 10,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwndVideo",
        optional: false,
        reason: "Microsoft documents this as retrieving the video window already set on the display control; it does not create or transfer ownership of the window",
        citation: "https://learn.microsoft.com/windows/win32/api/evr/nf-evr-imfvideodisplaycontrol-getvideowindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.Mmc",
        declaring_interface: "IConsole",
        declaring_iid: "43136eb1-d36c-11cf-adbc-00aa00a80033",
        method_name: "GetMainWindow",
        vtable_index: 12,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving MMC's existing main frame window handle, not creating a caller-owned window",
        citation: "https://learn.microsoft.com/windows/win32/api/mmc/nf-mmc-iconsole-getmainwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.Ole",
        declaring_interface: "IOleWindow",
        declaring_iid: "00000114-0000-0000-c000-000000000046",
        method_name: "GetWindow",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving an existing frame, document, parent, or in-place activation window; ownership remains with the participant that created that window",
        citation: "https://learn.microsoft.com/windows/win32/api/oleidl/nf-oleidl-iolewindow-getwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.WinRT",
        declaring_interface: "ICoreWindowInterop",
        declaring_iid: "45d64a29-a63e-4cb6-b498-5781d298cb4f",
        method_name: "get_WindowHandle",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "hwnd",
        optional: false,
        reason: "Microsoft documents this read-only property as obtaining the HWND of the existing CoreWindow; it does not transfer window lifetime",
        citation: "https://learn.microsoft.com/windows/win32/api/corewindow/nf-corewindow-icorewindowinterop-get_windowhandle",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.WinRT",
        declaring_interface: "IShareWindowCommandEventArgsInterop",
        declaring_iid: "6571a721-643d-43d4-aca4-6b6f5f30f1ad",
        method_name: "GetWindow",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "value",
        optional: false,
        reason: "Microsoft documents this as receiving the window identifier carried by the event arguments, with no creation or ownership transfer",
        citation: "https://learn.microsoft.com/windows/win32/api/sharewindowcommandsourceinterop/nf-sharewindowcommandsourceinterop-isharewindowcommandeventargsinterop-getwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.WinRT.Xaml",
        declaring_interface: "IDesktopWindowXamlSourceNative",
        declaring_iid: "3cbcf1bf-2f76-4e9c-96ab-e84b37972554",
        method_name: "get_WindowHandle",
        vtable_index: 4,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "hWnd",
        optional: false,
        reason: "Microsoft documents this as the parent UI element HWND associated with the current XAML source instance; the associated UI element retains ownership",
        citation: "https://learn.microsoft.com/windows/win32/api/windows.ui.xaml.hosting.desktopwindowxamlsource/nf-windows-ui-xaml-hosting-desktopwindowxamlsource-idesktopwindowxamlsourcenative-get_windowhandle",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.System.UpdateAgent",
        declaring_interface: "IUpdateInstaller",
        declaring_iid: "7b929c68-ccdc-4226-96b1-8724600b54c2",
        method_name: "get_ParentHwnd",
        vtable_index: 11,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "retval",
        optional: false,
        reason: "Microsoft documents this property as retrieving the existing parent window configured for the update installer; no window ownership is transferred",
        citation: "https://learn.microsoft.com/windows/win32/api/wuapi/nf-wuapi-iupdateinstaller-get_parenthwnd",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Accessibility",
        declaring_interface: "IUIAutomationElement",
        declaring_iid: "d22108aa-8ac5-49a5-837b-37bbb3d7591e",
        method_name: "get_CachedNativeWindowHandle",
        vtable_index: 68,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "retVal",
        optional: false,
        reason: "Microsoft documents this property as retrieving the cached native window handle of the existing UI Automation element; the element's window remains externally owned",
        citation: "https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_cachednativewindowhandle",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Accessibility",
        declaring_interface: "IUIAutomationElement",
        declaring_iid: "d22108aa-8ac5-49a5-837b-37bbb3d7591e",
        method_name: "get_CurrentNativeWindowHandle",
        vtable_index: 36,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "retVal",
        optional: false,
        reason: "Microsoft documents this property as retrieving the current native window handle of the existing UI Automation element; the element's window remains externally owned",
        citation: "https://learn.microsoft.com/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-get_currentnativewindowhandle",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Shell",
        declaring_interface: "ICredentialProviderCredentialEvents",
        declaring_iid: "fa6fa76b-66b7-4b11-95f1-86171118e816",
        method_name: "OnCreatingWindow",
        vtable_index: 12,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwndOwner",
        optional: false,
        reason: "Microsoft documents this as the Credential UI or Logon UI parent HWND that providers must borrow when parenting dialogs",
        citation: "https://learn.microsoft.com/windows/win32/api/credentialprovider/nf-credentialprovider-icredentialprovidercredentialevents-oncreatingwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Shell",
        declaring_interface: "ILaunchSourceViewSizePreference",
        declaring_iid: "e5aa01f7-1fb8-4830-8720-4e6734cbd5f3",
        method_name: "GetSourceViewToPosition",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "hwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving the existing source application window used for positioning, without creating or transferring it",
        citation: "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ilaunchsourceviewsizepreference-getsourceviewtoposition",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Shell",
        declaring_interface: "IFileIsInUse",
        declaring_iid: "64a1cbf0-3a1a-4461-9158-376969693950",
        method_name: "GetSwitchToHWND",
        vtable_index: 6,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving the existing application window to switch to; the caller observes the handle without acquiring the window",
        citation: "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileisinuse-getswitchtohwnd",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.Shell",
        declaring_interface: "IPreviewHandler",
        declaring_iid: "8895b1c6-b41f-4c1c-a562-0d564250836f",
        method_name: "QueryFocus",
        vtable_index: 8,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as the HWND returned by GetFocus on the preview handler's foreground thread, which is an observed borrowed window identity",
        citation: "https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ipreviewhandler-queryfocus",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.TabletPC",
        declaring_interface: "ITextInputPanel",
        declaring_iid: "6b6a65a5-6af3-46c2-b6ea-56cd1f80df71",
        method_name: "get_AttachedEditWindow",
        vtable_index: 3,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "AttachedEditWindow",
        optional: false,
        reason: "Microsoft documents this property as retrieving the edit window already attached to the text input panel; attachment does not transfer window ownership",
        citation: "https://learn.microsoft.com/windows/win32/api/peninputpanel/nf-peninputpanel-itextinputpanel-get_attachededitwindow",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.TextServices",
        declaring_interface: "ITfContextOwner",
        declaring_iid: "aa80e80c-2021-11d2-93e0-0060b067b86e",
        method_name: "GetWnd",
        vtable_index: 7,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving the existing owner window associated with the text context; ownership remains with the context owner",
        citation: "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextowner-getwnd",
    },
    BorrowedHwndOutputEvidence {
        declaring_namespace: "Windows.Win32.UI.TextServices",
        declaring_interface: "ITfContextView",
        declaring_iid: "2433bf8e-0f9b-435c-ba2c-180611978c30",
        method_name: "GetWnd",
        vtable_index: 6,
        parameter_count: 1,
        parameter_index: 0,
        parameter_name: "phwnd",
        optional: false,
        reason: "Microsoft documents this as retrieving the existing window represented by the text context view; the view retains window ownership",
        citation: "https://learn.microsoft.com/windows/win32/api/msctf/nf-msctf-itfcontextview-getwnd",
    },
];

pub(crate) fn registered_borrowed_hwnd_output(
    namespace: &str,
    interface: &str,
    iid: &str,
    method: &str,
    slot: usize,
    parameter_index: usize,
) -> Option<&'static BorrowedHwndOutputEvidence> {
    BORROWED_HWND_OUTPUTS.iter().find(|evidence| {
        evidence.declaring_namespace == namespace
            && evidence.declaring_interface == interface
            && evidence.declaring_iid.eq_ignore_ascii_case(iid)
            && evidence.method_name == method
            && evidence.vtable_index == slot
            && evidence.parameter_index == parameter_index
    })
}

pub(crate) fn borrowed_hwnd_evidence_for_declaration(
    namespace: &str,
    interface: &str,
    method: &str,
    slot: usize,
) -> Option<&'static BorrowedHwndOutputEvidence> {
    BORROWED_HWND_OUTPUTS.iter().find(|evidence| {
        evidence.declaring_namespace == namespace
            && evidence.declaring_interface == interface
            && (evidence.method_name == method || evidence.vtable_index == slot)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn registry_entries_have_unique_exact_identity_and_microsoft_citations() {
        let mut identities = BTreeSet::new();
        assert_eq!(BORROWED_HWND_OUTPUTS.len(), 22);
        for evidence in BORROWED_HWND_OUTPUTS {
            assert!(identities.insert((
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.declaring_iid.to_ascii_lowercase(),
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )));
            assert_eq!(evidence.declaring_iid.len(), 36);
            assert!(evidence.parameter_index < evidence.parameter_count);
            assert!(!evidence.optional);
            assert!(
                evidence
                    .citation
                    .starts_with("https://learn.microsoft.com/")
            );
            assert!(!evidence.reason.is_empty());
        }
    }

    #[test]
    fn exact_lookup_rejects_identity_drift() {
        let evidence = BORROWED_HWND_OUTPUTS
            .iter()
            .find(|evidence| evidence.declaring_interface == "IOleWindow")
            .unwrap();
        assert!(
            registered_borrowed_hwnd_output(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.declaring_iid,
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )
            .is_some()
        );
        assert!(
            registered_borrowed_hwnd_output(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                "00000000-0000-0000-c000-000000000046",
                evidence.method_name,
                evidence.vtable_index,
                evidence.parameter_index,
            )
            .is_none()
        );
        assert!(
            borrowed_hwnd_evidence_for_declaration(
                evidence.declaring_namespace,
                evidence.declaring_interface,
                evidence.method_name,
                evidence.vtable_index + 1,
            )
            .is_some()
        );
    }
}
