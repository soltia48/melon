//! The FeliCa **IDm** (manufacture ID): part of the account key.
//!
//! IDm is the 8-byte identifier a card returns at polling time. It is part of the
//! account key, so melon can only work with cards whose IDm is **fixed**: an IDm
//! that changes between taps would make every tap look like a new account, and the
//! balance would vanish from under the holder.
//!
//! Whether it is fixed is readable from the IDm itself — see
//! [`Idm::has_stable_id`]. Presented as 16 lowercase hex characters.

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

    /// The **manufacturer code**: the first two bytes, big-endian.
    pub const fn manufacturer_code(self) -> u16 {
        u16::from_be_bytes([self.0[0], self.0[1]])
    }

    /// Does this IDm name one card, the same way, every time?
    ///
    /// The manufacturer code says so. FeliCa reserves the whole `XXFEh` block —
    /// every code whose low byte is `FEh` — for numbering schemes melon cannot use
    /// as an account key:
    ///
    /// | code | numbering | product |
    /// |---|---|---|
    /// | `01FEh` | **random** (ISO/IEC 18092, NFCIP-1) | NFCIP-1 devices |
    /// | `02FEh` | none defined | NFC Forum Type 3 Tag |
    /// | `03FEh` | contains a data-format code | FeliCa Plug, FeliCa Lite-S |
    /// | `X4FEh`, `X6FEh` (`X` = in-card system number) | **contains a random part** | FeliCa Standard with a randomized ID |
    /// | `05FEh` | **random** (marks an unissued card) | — |
    /// | any other `XXFEh` | reserved | — |
    ///
    /// The randomizing ones are the reason for this check: a card that hands out a
    /// fresh IDm on every tap cannot be identified at all. The rest of the block is
    /// rejected with them — the numbering is undefined, reserved, or belongs to a
    /// product that cannot do FeliCa Standard mutual authentication anyway, so
    /// nothing usable is lost, and a reserved code that turns out to be random one
    /// day cannot surprise us.
    ///
    /// An issued FeliCa Standard card never carries a code in this block, so no
    /// card melon can actually serve is refused by this.
    pub const fn has_stable_id(self) -> bool {
        self.0[1] != 0xFE
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

    /// An IDm with the given manufacturer code (the rest of the bytes are filler).
    fn idm(manufacturer: u16) -> Idm {
        let [hi, lo] = manufacturer.to_be_bytes();
        Idm::from_bytes([hi, lo, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
    }

    #[test]
    fn manufacturer_code_is_the_first_two_bytes_big_endian() {
        let id = Idm::from_bytes([0x01, 0xFE, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        assert_eq!(id.manufacturer_code(), 0x01FE);
    }

    /// The whole `XXFEh` block is refused — see [`Idm::has_stable_id`].
    #[test]
    fn every_xxfe_manufacturer_code_is_refused() {
        // Random: NFCIP-1, and the code that marks an unissued card.
        assert!(!idm(0x01FE).has_stable_id());
        assert!(!idm(0x05FE).has_stable_id());
        // FeliCa Standard with a randomized ID: X4FEh / X6FEh, any system number.
        for x in 0x0..=0xF {
            assert!(!idm((x << 12) | 0x04FE).has_stable_id(), "{x:X}4FEh");
            assert!(!idm((x << 12) | 0x06FE).has_stable_id(), "{x:X}6FEh");
        }
        // Undefined numbering (NFC Forum Type 3 Tag), a non-Standard product, and
        // the reserved codes: not random, but not an account key either.
        assert!(!idm(0x02FE).has_stable_id());
        assert!(!idm(0x03FE).has_stable_id());
        assert!(!idm(0x00FE).has_stable_id());
        assert!(!idm(0xFFFE).has_stable_id());
    }

    /// An issued FeliCa Standard card is not in that block, and must still work.
    #[test]
    fn an_issued_standard_card_is_accepted() {
        assert!(idm(0x0101).has_stable_id());
        assert!(idm(0x012E).has_stable_id());
        // Only the LOW byte of the code decides: FEh elsewhere is unremarkable.
        assert!(idm(0xFE01).has_stable_id());
        assert!(Idm::from_bytes([0x01, 0x02, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE]).has_stable_id());
    }

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
