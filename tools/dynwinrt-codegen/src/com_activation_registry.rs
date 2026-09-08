// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationContext {
    InProcess,
}

impl ActivationContext {
    pub(crate) const fn native_value(self) -> u32 {
        match self {
            Self::InProcess => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationParameters {
    NativeNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationOutput {
    OwnedRequestedInterface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivationConditions {
    pub context: ActivationContext,
    pub parameters: ActivationParameters,
    pub output: ActivationOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivationTargetEvidence {
    pub namespace: &'static str,
    pub interface: &'static str,
    pub iid: &'static str,
    pub conditions: ActivationConditions,
    pub reason: &'static str,
    pub citation: &'static str,
}

const ORDINARY_ENDPOINT_ACTIVATION: ActivationConditions = ActivationConditions {
    context: ActivationContext::InProcess,
    parameters: ActivationParameters::NativeNull,
    output: ActivationOutput::OwnedRequestedInterface,
};

const IMM_DEVICE_ACTIVATE_TARGETS: &[ActivationTargetEvidence] = &[
    ActivationTargetEvidence {
        namespace: "Windows.Win32.Media.Audio",
        interface: "IAudioClient",
        iid: "1cb9ad4c-dbfa-4c32-b178-c2f568a703b2",
        conditions: ORDINARY_ENDPOINT_ACTIVATION,
        reason: "Ordinary endpoint audio-client activation uses null parameters; parameterized loopback activation is outside this contract",
        citation: "https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate",
    },
    ActivationTargetEvidence {
        namespace: "Windows.Win32.Media.Audio.Endpoints",
        interface: "IAudioEndpointVolume",
        iid: "5cdf2c82-841e-4546-9722-0cf74078229a",
        conditions: ORDINARY_ENDPOINT_ACTIVATION,
        reason: "Endpoint-volume activation requires null activation parameters",
        citation: "https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate",
    },
    ActivationTargetEvidence {
        namespace: "Windows.Win32.Media.Audio.Endpoints",
        interface: "IAudioMeterInformation",
        iid: "c02216f6-8c67-4b5b-9d00-d008e73e0064",
        conditions: ORDINARY_ENDPOINT_ACTIVATION,
        reason: "Endpoint-meter activation requires null activation parameters",
        citation: "https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate",
    },
    ActivationTargetEvidence {
        namespace: "Windows.Win32.Media.Audio",
        interface: "IAudioSessionManager",
        iid: "bfa971f1-4d5e-40bb-935e-967039bfbee4",
        conditions: ORDINARY_ENDPOINT_ACTIVATION,
        reason: "Audio-session-manager activation requires null activation parameters",
        citation: "https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate",
    },
    ActivationTargetEvidence {
        namespace: "Windows.Win32.Media.Audio",
        interface: "IAudioSessionManager2",
        iid: "77aa99a0-1bd6-484f-8bc7-2c654c9a9b6f",
        conditions: ORDINARY_ENDPOINT_ACTIVATION,
        reason: "IMMDevice documents the extended audio-session manager as an activation target using the ordinary session-manager parameter policy",
        citation: "https://learn.microsoft.com/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-activate",
    },
];

/// Targets for the independently fingerprinted IMMDevice::Activate method contract.
/// Registration does not establish wrapper completeness or device availability.
pub(crate) const fn imm_device_activate_targets() -> &'static [ActivationTargetEvidence] {
    IMM_DEVICE_ACTIVATE_TARGETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_activation_target_set_is_unchanged() {
        let targets = imm_device_activate_targets();
        assert_eq!(
            targets.iter().map(|entry| entry.iid).collect::<Vec<_>>(),
            [
                "1cb9ad4c-dbfa-4c32-b178-c2f568a703b2",
                "5cdf2c82-841e-4546-9722-0cf74078229a",
                "c02216f6-8c67-4b5b-9d00-d008e73e0064",
                "bfa971f1-4d5e-40bb-935e-967039bfbee4",
                "77aa99a0-1bd6-484f-8bc7-2c654c9a9b6f",
            ]
        );
        assert!(
            targets
                .iter()
                .all(|entry| entry.conditions == ORDINARY_ENDPOINT_ACTIVATION)
        );
        assert!(
            !targets
                .iter()
                .any(|entry| entry.interface == "IDeviceTopology")
        );
    }

    #[test]
    fn endpoint_activation_target_names_match_pinned_metadata() {
        let Ok(winmd) = std::env::var("DYNWINRT_WIN32_WINMD") else {
            return;
        };
        for entry in imm_device_activate_targets() {
            let interface =
                crate::com_metadata::parse_com_interface(&winmd, entry.namespace, entry.interface)
                    .expect("registered activation target must exist in the configured metadata");
            assert_eq!(interface.interface.iid, entry.iid);
        }
    }
}
