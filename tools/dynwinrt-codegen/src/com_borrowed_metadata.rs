// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Exact storage/context evidence. These selectors are not general pointer rules.

use super::{ComInterfaceMeta, RawComMethod, raw_method_fingerprint};
use crate::contract_registry::{
    ContractKind, ExactEntrySelector, ExactFamilyId, ExactRegistryEntry,
};
use sha2::{Digest, Sha256};

pub const METADATA_SHA256: &str =
    "B64EE4818A7ED9F9D135038D58C51BD08369184D4D5ED428F20E9DE55DF8121D";
pub const AUDIO: &str = "Windows.Win32.Media.Audio";
pub const WIC: &str = "Windows.Win32.Graphics.Imaging";
pub const MF: &str = "Windows.Win32.Media.MediaFoundation";
pub const CLIENT_IID: &str = "1cb9ad4c-dbfa-4c32-b178-c2f568a703b2";
pub const CLIENT2_IID: &str = "726778cd-f60a-4eda-82de-e47610cd78aa";
pub const CLIENT3_IID: &str = "7ed4ee07-8e67-4cd4-8c1a-2b7a5987ad42";
pub const RENDER_IID: &str = "f294acfc-3146-4483-a7bf-addca7c260e2";
pub const CAPTURE_IID: &str = "c8adbd64-e71e-48a0-a4de-185c395cd317";
pub const BITMAP_IID: &str = "00000121-a8f2-4877-ba0a-fd2b6645fb94";
pub const LOCK_IID: &str = "00000123-a8f2-4877-ba0a-fd2b6645fb94";
pub const MF_IID: &str = "045fa593-8799-42b8-bc8d-8968c6453507";

#[derive(Debug)]
pub struct Evidence {
    pub namespace: &'static str,
    pub interface: &'static str,
    pub iid: &'static str,
    pub method: &'static str,
    pub slot: usize,
    pub fingerprint: &'static str,
    pub effect: Option<&'static str>,
    pub citation: &'static str,
}

macro_rules! storage {
    ($ns:expr, $interface:literal, $iid:expr, $method:literal, $slot:literal, $hash:literal, $citation:literal) => {
        Evidence {
            namespace: $ns,
            interface: $interface,
            iid: $iid,
            method: $method,
            slot: $slot,
            fingerprint: $hash,
            effect: None,
            citation: $citation,
        }
    };
}

// Fingerprints include native layouts, directions, optional/Const flags,
// NativeArrayInfo, return convention and all existing exact-contract evidence.
pub const EVIDENCE: &[Evidence] = &[
    Evidence {
        namespace: AUDIO,
        interface: "IAudioClient",
        iid: CLIENT_IID,
        method: "Initialize",
        slot: 3,
        fingerprint: "7F14AD1E6E6E178A91283B68002988D362898FCAC97823B866D61FAFF5766039",
        effect: Some("audio-initialize"),
        citation: "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-initialize",
    },
    Evidence {
        namespace: AUDIO,
        interface: "IAudioClient",
        iid: CLIENT_IID,
        method: "GetService",
        slot: 14,
        fingerprint: "6362D65B61F18446F4E990C84DA998C4A7DE08517C7405981DF823854227CA7E",
        effect: Some("audio-get-service"),
        citation: "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getservice",
    },
    Evidence {
        namespace: AUDIO,
        interface: "IAudioClient3",
        iid: CLIENT3_IID,
        method: "InitializeSharedAudioStream",
        slot: 20,
        fingerprint: "08879857019485B6813E45F4BEB2941E4D83ACF80CA55AA4538F50B08C1B9531",
        effect: Some("audio-initialize-shared"),
        citation: "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient3-initializesharedaudiostream",
    },
    storage!(
        AUDIO,
        "IAudioClient",
        CLIENT_IID,
        "GetBufferSize",
        4,
        "9F0C09DD69B785A9B4A339ECD2A1970E8574BDBA0F173E3ACF630407A9AB14BE",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getbuffersize"
    ),
    storage!(
        AUDIO,
        "IAudioRenderClient",
        RENDER_IID,
        "GetBuffer",
        3,
        "E99E87341B5820E896608A563B67ABCF2A0F9BD833202788950D9FAE8F69E2DB",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-getbuffer"
    ),
    storage!(
        AUDIO,
        "IAudioRenderClient",
        RENDER_IID,
        "ReleaseBuffer",
        4,
        "C79B7868B7940E7D879DC90CA1BAC56E386DA7C8E0F7790DFAB7FB8971E0D767",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiorenderclient-releasebuffer"
    ),
    storage!(
        AUDIO,
        "IAudioCaptureClient",
        CAPTURE_IID,
        "GetBuffer",
        3,
        "DDD954B2807361F2D50D38C8054846A02577E820B6D42E0B82130F785AA3E370",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer"
    ),
    storage!(
        AUDIO,
        "IAudioCaptureClient",
        CAPTURE_IID,
        "ReleaseBuffer",
        4,
        "82D3C575E5031A345E01026F01E600C8887B081DF6F79CA514B49B78F01E5E17",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-releasebuffer"
    ),
    storage!(
        AUDIO,
        "IAudioCaptureClient",
        CAPTURE_IID,
        "GetNextPacketSize",
        5,
        "C540B744DB900C7D5EDAD03422FC17E7034A822832F4A39DC2DD8519B0978209",
        "https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getnextpacketsize"
    ),
    storage!(
        WIC,
        "IWICBitmapSource",
        "00000120-a8f2-4877-ba0a-fd2b6645fb94",
        "GetSize",
        3,
        "F0CBF53A188710273FE737C1A06CDA40FA799C4326F3CA9BF68988E5072B5DA2",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapsource-getsize"
    ),
    storage!(
        WIC,
        "IWICBitmapSource",
        "00000120-a8f2-4877-ba0a-fd2b6645fb94",
        "GetPixelFormat",
        4,
        "D45F675B2B76A3A9BE908AFBAC03BF4CEC2BD5D8162C1BF28F01FA558B2B6F98",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapsource-getpixelformat"
    ),
    storage!(
        WIC,
        "IWICBitmapSource",
        "00000120-a8f2-4877-ba0a-fd2b6645fb94",
        "GetResolution",
        5,
        "3545A1714671B8C7223BF21693848300001272CDD14E1BE6C188680EA5BD75DD",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapsource-getresolution"
    ),
    storage!(
        WIC,
        "IWICBitmapSource",
        "00000120-a8f2-4877-ba0a-fd2b6645fb94",
        "CopyPalette",
        6,
        "CBBEDB597008ED4EA9032EE361F19AC676DA973DA93A1CB8B0B7D67E24CA4A8C",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapsource-copypalette"
    ),
    storage!(
        WIC,
        "IWICBitmapSource",
        "00000120-a8f2-4877-ba0a-fd2b6645fb94",
        "CopyPixels",
        7,
        "78BA461831509B5574CB3F8485FE071B875E1AA1FF4ABC4EBB4ADE74A1AE0A9A",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmapsource-copypixels"
    ),
    storage!(
        WIC,
        "IWICBitmap",
        BITMAP_IID,
        "Lock",
        8,
        "BE432E367AC62157C546DB3F21AE11B89025F930D4AE397ECC5B6A6F2016D154",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmap-lock"
    ),
    storage!(
        WIC,
        "IWICBitmap",
        BITMAP_IID,
        "SetPalette",
        9,
        "D714FBE65EE1E239F04D9046C2270572CC8DE81094FCE549D81FF50242BC6F8F",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmap-setpalette"
    ),
    storage!(
        WIC,
        "IWICBitmap",
        BITMAP_IID,
        "SetResolution",
        10,
        "394965547E9C14CB338B9AD84677C27CA1D54BF22E4CDBA79A46D26F31C25EB6",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmap-setresolution"
    ),
    storage!(
        WIC,
        "IWICBitmapLock",
        LOCK_IID,
        "GetSize",
        3,
        "5FBF3161D277C77796793A167F68999C7B678A182FCBB823CD721DC4BAC83F64",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmaplock-getsize"
    ),
    storage!(
        WIC,
        "IWICBitmapLock",
        LOCK_IID,
        "GetStride",
        4,
        "0DC2598054C7BCB1D19940355D5BE568D508B332E269D00956BAF22DC854F799",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmaplock-getstride"
    ),
    storage!(
        WIC,
        "IWICBitmapLock",
        LOCK_IID,
        "GetDataPointer",
        5,
        "FB4F6B8BD302FCA2BB91B231BF957E10D37AC979C39A88992E2526E53349F66C",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmaplock-getdatapointer"
    ),
    storage!(
        WIC,
        "IWICBitmapLock",
        LOCK_IID,
        "GetPixelFormat",
        6,
        "A065B19968592FDA319A1A9DAC7EC1EB51CE63F5D3FFA62C6DD38E92742065E6",
        "https://learn.microsoft.com/en-us/windows/win32/api/wincodec/nf-wincodec-iwicbitmaplock-getpixelformat"
    ),
    storage!(
        MF,
        "IMFMediaBuffer",
        MF_IID,
        "Lock",
        3,
        "1B9E2C63825F73EE5599F05E9EA180D52FD22F02EDD33BB23028711DDF335172",
        "https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-lock"
    ),
    storage!(
        MF,
        "IMFMediaBuffer",
        MF_IID,
        "Unlock",
        4,
        "83FB2903C23EBD4BB98C182C6575135225ABD0C3EA78183CDADFB3B027B8107B",
        "https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-unlock"
    ),
    storage!(
        MF,
        "IMFMediaBuffer",
        MF_IID,
        "GetCurrentLength",
        5,
        "AABA562CD26F16571A75D54894E001700500994E98949B4E7A7DA1B6CDE61862",
        "https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-getcurrentlength"
    ),
    storage!(
        MF,
        "IMFMediaBuffer",
        MF_IID,
        "SetCurrentLength",
        6,
        "75AB892B1685032D840B1E5FECB9CD9A87F8E1865492D24796996674372864CA",
        "https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-setcurrentlength"
    ),
    storage!(
        MF,
        "IMFMediaBuffer",
        MF_IID,
        "GetMaxLength",
        7,
        "738D4850D037E6B9F76F28C7D0403B4D9BDF97F3B04C9B8E5A0EFBA18A3A962C",
        "https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-getmaxlength"
    ),
];

impl Evidence {
    pub(crate) fn entry(&self) -> ExactRegistryEntry {
        let family = if self.effect.is_some() {
            ExactFamilyId::AudioContext
        } else {
            ExactFamilyId::BorrowedCopy
        };
        let selector = ExactEntrySelector {
            namespace: self.namespace.into(),
            interface: self.interface.into(),
            iid: self.iid.into(),
            method: self.method.into(),
            slot: self.slot,
            parameter: None,
        };
        ExactRegistryEntry {
            entry_id: selector.entry_id(family), selector, family_id: family,
            contract_kind: if self.effect.is_some() { ContractKind::ContextualEffect } else { ContractKind::BorrowedStorage },
            source_fingerprint: self.fingerprint.into(),
            reason: if self.effect.is_some() {
                "Only the actual successful metadata-described native call may create immutable initialization or service-origin provenance"
            } else {
                "Bounded owned-copy storage/extent/finalization plan; native borrowed bytes and unsupported methods are not public"
            }.into(),
            citation: self.citation.into(),
        }
    }
}

pub fn evidence_for(raw: &RawComMethod) -> Result<Option<&'static Evidence>, String> {
    let selected = EVIDENCE.iter().find(|entry| {
        entry.namespace == raw.declaring_namespace
            && entry.interface == raw.declaring_interface
            && entry.method == raw.metadata_name
    });
    if let Some(entry) = selected
        && (entry.iid != raw.declaring_iid
            || entry.slot != raw.vtable_index
            || entry.fingerprint != raw_method_fingerprint(raw))
    {
        return Err(format!(
            "Borrowed-copy/context evidence drift: {}.{}::{}",
            entry.namespace, entry.interface, entry.method
        ));
    }
    Ok(selected)
}

pub fn is_audio_context(meta: &ComInterfaceMeta) -> bool {
    meta.interface.namespace == AUDIO
        && matches!(
            meta.interface.name.as_str(),
            "IAudioClient" | "IAudioClient2" | "IAudioClient3"
        )
}

pub fn is_bounded_copy(meta: &ComInterfaceMeta) -> bool {
    matches!(
        (
            meta.interface.namespace.as_str(),
            meta.interface.name.as_str()
        ),
        (AUDIO, "IAudioRenderClient" | "IAudioCaptureClient")
            | (WIC, "IWICBitmap")
            | (MF, "IMFMediaBuffer")
    )
}

pub fn is_copy_only(meta: &ComInterfaceMeta) -> bool {
    is_bounded_copy(meta) && meta.interface.name != "IWICBitmap"
}

pub(crate) fn catalog_entries(meta: &ComInterfaceMeta) -> Vec<ExactRegistryEntry> {
    if !is_audio_context(meta)
        && !is_bounded_copy(meta)
        && !(meta.interface.namespace == WIC && meta.interface.name == "IWICBitmapLock")
    {
        return vec![];
    }
    meta.raw_methods
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|raw| evidence_for(raw).ok().flatten())
        .map(Evidence::entry)
        .collect()
}

pub fn require_metadata_hash(paths: &str) -> Result<(), String> {
    // Hash only relevant projections, not every interface in a census. Unknown
    // metadata versions need reviewed selectors, never a name-based fallback.
    let matching = paths
        .split(';')
        .filter(|path| !path.trim().is_empty())
        .any(|path| {
            std::fs::read(path)
                .ok()
                .is_some_and(|bytes| format!("{:X}", Sha256::digest(bytes)) == METADATA_SHA256)
        });
    if matching {
        Ok(())
    } else {
        Err("Borrowed-copy plans require the pinned Win32Metadata 71.0.14-preview SHA256".into())
    }
}

pub fn validate_interface(meta: &ComInterfaceMeta) -> Result<(), String> {
    let (iid, bases, base_iids, own_start, last) = match (
        meta.interface.namespace.as_str(),
        meta.interface.name.as_str(),
    ) {
        (AUDIO, "IAudioClient") => (CLIENT_IID, vec!["IUnknown"], vec![], 3, 14),
        (AUDIO, "IAudioClient2") => (
            CLIENT2_IID,
            vec!["IAudioClient", "IUnknown"],
            vec![CLIENT_IID],
            15,
            17,
        ),
        (AUDIO, "IAudioClient3") => (
            CLIENT3_IID,
            vec!["IAudioClient2", "IAudioClient", "IUnknown"],
            vec![CLIENT2_IID, CLIENT_IID],
            18,
            20,
        ),
        (AUDIO, "IAudioRenderClient") => (RENDER_IID, vec!["IUnknown"], vec![], 3, 4),
        (AUDIO, "IAudioCaptureClient") => (CAPTURE_IID, vec!["IUnknown"], vec![], 3, 5),
        (WIC, "IWICBitmap") => (
            BITMAP_IID,
            vec!["IWICBitmapSource", "IUnknown"],
            vec!["00000120-a8f2-4877-ba0a-fd2b6645fb94"],
            8,
            10,
        ),
        (WIC, "IWICBitmapLock") => (LOCK_IID, vec!["IUnknown"], vec![], 3, 6),
        (MF, "IMFMediaBuffer") => (MF_IID, vec!["IUnknown"], vec![], 3, 7),
        _ => return Err("Not an exact borrowed-copy/context interface".into()),
    };
    let methods = meta
        .raw_methods
        .as_deref()
        .ok_or("Borrowed-copy/context raw evidence is absent")?;
    if meta.interface.iid != iid
        || !meta.is_iunknown_rooted
        || meta.base_offset != 3
        || meta.own_methods_start != own_start
        || meta.base_chain != bases
        || meta.base_iids != base_iids
        || meta.interface.generic_piid.is_some()
        || !meta.interface.generic_args.is_empty()
        || methods.len() != last - 2
        || meta.interface.methods.len() != methods.len()
        || methods
            .iter()
            .enumerate()
            .any(|(index, raw)| raw.vtable_index != index + 3)
    {
        return Err(format!(
            "Borrowed-copy/context identity or complete vtable drift: {}",
            meta.interface.name
        ));
    }
    for raw in methods {
        let entry = evidence_for(raw)?;
        if !is_audio_context(meta) && entry.is_none() {
            return Err(format!(
                "Absent borrowed-storage evidence at slot {}",
                raw.vtable_index
            ));
        }
    }
    if is_audio_context(meta) {
        for slot in [3, 4, 14].into_iter().chain((last == 20).then_some(20)) {
            let raw = &methods[slot - 3];
            if evidence_for(raw)?.is_none() {
                return Err(format!("Absent audio context evidence at slot {slot}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borrowed_copy_metadata_evidence_and_fail_closed_drift() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        require_metadata_hash(&winmd).unwrap();
        for (namespace, name) in [
            (AUDIO, "IAudioClient3"),
            (AUDIO, "IAudioRenderClient"),
            (AUDIO, "IAudioCaptureClient"),
            (WIC, "IWICBitmap"),
            (WIC, "IWICBitmapLock"),
            (MF, "IMFMediaBuffer"),
        ] {
            let meta = super::super::parse_com_interface(&winmd, namespace, name).unwrap();
            validate_interface(&meta).unwrap();
            let mut drift = meta.clone();
            drift.raw_methods = None;
            assert!(validate_interface(&drift).is_err());
            let mut drift = meta.clone();
            drift.interface.iid = CLIENT_IID.into();
            assert!(validate_interface(&drift).is_err());
            for index in 0..meta.raw_methods.as_ref().unwrap().len() {
                if evidence_for(&meta.raw_methods.as_ref().unwrap()[index])
                    .unwrap()
                    .is_none()
                {
                    continue;
                }
                let mut drift = meta.clone();
                drift.raw_methods.as_mut().unwrap()[index]
                    .params
                    .iter_mut()
                    .for_each(|param| param.optional = !param.optional);
                if !drift.raw_methods.as_ref().unwrap()[index].params.is_empty() {
                    assert!(validate_interface(&drift).is_err(), "{name} slot {index}");
                }
                let mut drift = meta.clone();
                drift.raw_methods.as_mut().unwrap()[index].vtable_index += 1;
                assert!(validate_interface(&drift).is_err());
            }
        }
        assert!(require_metadata_hash("").is_err());
    }
}
