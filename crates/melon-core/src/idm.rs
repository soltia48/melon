//! The FeliCa **IDm** (manufacture ID): part of the account key.
//!
//! IDm is the 8-byte identifier a card returns at polling time. In general it can
//! be randomized (mobile FeliCa / privacy modes) — melon includes it in the
//! account key only because this deployment uses cards with a **fixed** IDm; a
//! randomized IDm would make every tap look like a new account. Presented as 16
//! lowercase hex characters.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Length of an IDm in bytes.
pub const IDM_LEN: usize = 8;

/// A FeliCa manufacture ID (IDm).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Idm([u8; IDM_LEN]);

/// Error parsing an [`Idm`] from a hex string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdmParseError {
    #[error("IDm must be {expected} hex characters, got {got}")]
    BadLength { expected: usize, got: usize },
    #[error("IDm contains invalid hex")]
    InvalidHex,
}

impl Idm {
    /// Construct from raw bytes — e.g. the IDm from a polling result.
    pub const fn from_bytes(bytes: [u8; IDM_LEN]) -> Idm {
        Idm(bytes)
    }

    /// The raw 8 bytes.
    pub const fn as_bytes(&self) -> &[u8; IDM_LEN] {
        &self.0
    }

    /// The 8 bytes as an owned array.
    pub const fn to_bytes(self) -> [u8; IDM_LEN] {
        self.0
    }

    /// Lowercase hex encoding (16 characters).
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Parse from a byte slice, requiring exactly 8 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Idm, IdmParseError> {
        let arr: [u8; IDM_LEN] = bytes.try_into().map_err(|_| IdmParseError::BadLength {
            expected: IDM_LEN,
            got: bytes.len(),
        })?;
        Ok(Idm(arr))
    }
}

impl FromStr for Idm {
    type Err = IdmParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() != IDM_LEN * 2 {
            return Err(IdmParseError::BadLength {
                expected: IDM_LEN * 2,
                got: s.len(),
            });
        }
        let bytes = hex::decode(s).map_err(|_| IdmParseError::InvalidHex)?;
        Idm::from_slice(&bytes)
    }
}

impl fmt::Display for Idm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Idm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Idm({})", self.to_hex())
    }
}

impl Serialize for Idm {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Idm {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_parse_errors() {
        let idm = Idm::from_bytes([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11]);
        assert_eq!(idm.to_hex(), "aabbccddeeff0011");
        assert_eq!("aabbccddeeff0011".parse::<Idm>().unwrap(), idm);
        assert!(matches!(
            "0102".parse::<Idm>(),
            Err(IdmParseError::BadLength { .. })
        ));
        assert_eq!(
            "zz02030405060708".parse::<Idm>(),
            Err(IdmParseError::InvalidHex)
        );
    }
}
