//! The FeliCa **IDi** (issue ID): the account primary key.
//!
//! IDi is the 8-byte identifier obtained from `felica-rs`'s
//! `MutualAuthenticationResult.issue_id`, available only *after* a successful
//! FeliCa Standard mutual authentication. It is distinct from the IDm, which is
//! transmitted in the clear at polling time and may be randomized — so IDi, and
//! never IDm, is what we key accounts on. Canonically presented as 16 lowercase
//! hex characters.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Length of an IDi in bytes.
pub const IDI_LEN: usize = 8;

/// A FeliCa issue ID (IDi).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Idi([u8; IDI_LEN]);

/// Error parsing an [`Idi`] from a hex string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdiParseError {
    #[error("IDi must be {expected} hex characters, got {got}")]
    BadLength { expected: usize, got: usize },
    #[error("IDi contains invalid hex")]
    InvalidHex,
}

impl Idi {
    /// Construct from raw bytes — e.g. `Idi::from_bytes(auth_result.issue_id)`.
    pub const fn from_bytes(bytes: [u8; IDI_LEN]) -> Idi {
        Idi(bytes)
    }

    /// The raw 8 bytes.
    pub const fn as_bytes(&self) -> &[u8; IDI_LEN] {
        &self.0
    }

    /// The 8 bytes as an owned array.
    pub const fn to_bytes(self) -> [u8; IDI_LEN] {
        self.0
    }

    /// Lowercase hex encoding (16 characters).
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Parse from a byte slice, requiring exactly 8 bytes.
    pub fn from_slice(bytes: &[u8]) -> Result<Idi, IdiParseError> {
        let arr: [u8; IDI_LEN] = bytes.try_into().map_err(|_| IdiParseError::BadLength {
            expected: IDI_LEN,
            got: bytes.len(),
        })?;
        Ok(Idi(arr))
    }
}

impl FromStr for Idi {
    type Err = IdiParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() != IDI_LEN * 2 {
            return Err(IdiParseError::BadLength {
                expected: IDI_LEN * 2,
                got: s.len(),
            });
        }
        let bytes = hex::decode(s).map_err(|_| IdiParseError::InvalidHex)?;
        Idi::from_slice(&bytes)
    }
}

impl fmt::Display for Idi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Idi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Idi({})", self.to_hex())
    }
}

impl Serialize for Idi {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Idi {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

    #[test]
    fn hex_round_trip() {
        let idi = Idi::from_bytes(RAW);
        assert_eq!(idi.to_hex(), "0102030405060708");
        assert_eq!("0102030405060708".parse::<Idi>().unwrap(), idi);
    }

    #[test]
    fn parse_rejects_bad_length_and_hex() {
        assert!(matches!(
            "0102".parse::<Idi>(),
            Err(IdiParseError::BadLength { .. })
        ));
        assert_eq!(
            "zz02030405060708".parse::<Idi>(),
            Err(IdiParseError::InvalidHex)
        );
    }

    #[test]
    fn serde_is_a_hex_string() {
        let idi = Idi::from_bytes(RAW);
        assert_eq!(serde_json::to_string(&idi).unwrap(), "\"0102030405060708\"");
        assert_eq!(
            serde_json::from_str::<Idi>("\"0102030405060708\"").unwrap(),
            idi
        );
        assert!(serde_json::from_str::<Idi>("\"nothex\"").is_err());
    }
}
