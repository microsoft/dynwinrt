// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::diagnostics::ModelError;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(in crate::codegen::com) struct $name(u32);

        impl $name {
            pub(super) fn from_index(index: usize) -> Option<Self> {
                u32::try_from(index).ok().map(Self)
            }

            pub(in crate::codegen::com) const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_id!(TypeId);
define_id!(LayoutId);
define_id!(EnumId);
define_id!(SignatureId);
define_id!(CleanupId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::codegen::com) struct ParamIndex(usize);

impl ParamIndex {
    pub(super) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub(in crate::codegen::com) const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::codegen::com) struct ComGuid([u8; 16]);

impl ComGuid {
    pub(super) const ZERO: Self = Self([0; 16]);

    pub(super) const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub(in crate::codegen::com) const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub(super) fn is_zero(self) -> bool {
        self == Self::ZERO
    }

    pub(super) fn parse(value: &str) -> Result<Self, ModelError> {
        if value.len() != 36
            || value.as_bytes().get(8) != Some(&b'-')
            || value.as_bytes().get(13) != Some(&b'-')
            || value.as_bytes().get(18) != Some(&b'-')
            || value.as_bytes().get(23) != Some(&b'-')
        {
            return Err(ModelError::InvalidContract(format!(
                "invalid GUID `{value}`"
            )));
        }
        let hex = value
            .bytes()
            .filter(|byte| *byte != b'-')
            .collect::<Vec<_>>();
        if hex.len() != 32 {
            return Err(ModelError::InvalidContract(format!(
                "invalid GUID `{value}`"
            )));
        }
        let mut bytes = [0; 16];
        for (index, pair) in hex.chunks_exact(2).enumerate() {
            let high = hex_digit(pair[0])
                .ok_or_else(|| ModelError::InvalidContract(format!("invalid GUID `{value}`")))?;
            let low = hex_digit(pair[1])
                .ok_or_else(|| ModelError::InvalidContract(format!("invalid GUID `{value}`")))?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_their_arena_index() {
        let id = TypeId::from_index(42).unwrap();
        assert_eq!(id.index(), 42);
        #[cfg(target_pointer_width = "64")]
        assert!(TypeId::from_index(u32::MAX as usize + 1).is_none());
        #[cfg(target_pointer_width = "32")]
        assert_eq!(TypeId::from_index(usize::MAX).unwrap().index(), usize::MAX);
    }

    #[test]
    fn guid_keeps_all_abi_bytes() {
        let guid = ComGuid::from_bytes([0x5a; 16]);
        assert_eq!(guid.as_bytes(), &[0x5a; 16]);
        assert!(!guid.is_zero());
        assert!(ComGuid::ZERO.is_zero());
    }

    #[test]
    fn guid_parser_requires_canonical_shape() {
        let guid = ComGuid::parse("00000000-0000-0000-c000-000000000046").unwrap();
        assert!(!guid.is_zero());
        assert!(ComGuid::parse("not-a-guid").is_err());
        assert!(ComGuid::parse("000000000000-0000-c000-000000000046").is_err());
    }
}
