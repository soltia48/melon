//! JSONL-backed store of shared FeliCa keys and DES service-key derivation.
//!
//! Each line is a JSON object in the same shape as `felica-rs`'s `keys.jsonl`:
//!
//! ```json
//! {"system_code":"0003","node":"FFFF","algo":"DES","version":"0003","idm":null,"key":"00112233..."}
//! ```
//!
//! - `system_code` / `node` — hex integers. Node `FFFF` is the system key.
//! - `algo` — `"DES"` (8-byte key) or `"AES"` (16-byte key).
//! - `version` — key version (informational; ignored for lookup).
//! - `idm` — `null` for a system-wide key, or an 8-byte hex IDm for a
//!   card-specific key. When authenticating a card, a key whose `idm` matches the
//!   card's IDm is preferred, otherwise the system-wide key is used.
//! - `key` — the key, hex-encoded.
//!
//! This crypto oracle authenticates over DES, so only DES keys are indexed; AES
//! records are counted and skipped.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use felica_rs::felica_standard::generate_service_keys_des;
use serde::Deserialize;

use crate::error::ProtocolError;

const DES_KEY_LEN: usize = 8;
const IDM_LEN: usize = 8;

/// Group and individual (user) service keys derived for a mutual-authentication.
#[derive(Clone, Copy, Debug)]
pub struct DerivedKeys {
    pub group: [u8; DES_KEY_LEN],
    pub user: [u8; DES_KEY_LEN],
}

/// Raw record as it appears in the JSONL file.
#[derive(Debug, Deserialize)]
struct KeyRecord {
    system_code: String,
    node: String,
    #[serde(default)]
    algo: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,
    #[serde(default)]
    idm: Option<String>,
    key: String,
}

/// In-memory DES key store, split into system-wide and per-IDm keys.
#[derive(Clone, Debug, Default)]
pub struct KeyStore {
    /// `(system_code, node) -> key` for records with `idm == null`.
    system_wide: HashMap<(u16, u16), [u8; DES_KEY_LEN]>,
    /// `(system_code, node, idm) -> key` for card-specific records.
    per_idm: HashMap<(u16, u16, [u8; IDM_LEN]), [u8; DES_KEY_LEN]>,
    /// Distinct system codes in the order they first appear in the key file. The
    /// maps above are unordered, so this preserves the operator's intent: a
    /// terminal picks the FIRST of these that a tapped card also exposes, so the
    /// order of `keys.jsonl` decides which system a multi-system card transacts
    /// under.
    system_code_order: Vec<u16>,
}

fn parse_hex_u16(value: &str, field: &str, line: usize) -> Result<u16, ProtocolError> {
    let cleaned = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    u16::from_str_radix(cleaned, 16).map_err(|_| {
        ProtocolError::internal(format!("line {line}: invalid {field} hex value '{value}'"))
    })
}

fn parse_hex_array<const N: usize>(
    value: &str,
    field: &str,
    line: usize,
) -> Result<[u8; N], ProtocolError> {
    let bytes = hex::decode(value.trim())
        .map_err(|_| ProtocolError::internal(format!("line {line}: invalid {field} hex")))?;
    bytes
        .try_into()
        .map_err(|_| ProtocolError::internal(format!("line {line}: {field} must be {N} bytes")))
}

impl KeyStore {
    /// Load keys from a JSONL file.
    pub fn from_jsonl(path: impl AsRef<Path>) -> Result<Self, ProtocolError> {
        let path = path.as_ref();
        let file = std::fs::File::open(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                ProtocolError::internal(format!("keys file not found: {}", path.display()))
            }
            _ => ProtocolError::internal(format!("failed to open keys file: {e}")),
        })?;
        Self::from_reader(BufReader::new(file))
    }

    /// Load keys from any line reader (file or, in tests, an in-memory buffer).
    pub fn from_reader(reader: impl BufRead) -> Result<Self, ProtocolError> {
        let mut store = KeyStore::default();
        let mut skipped_non_des = 0usize;

        for (index, line) in reader.lines().enumerate() {
            let line_no = index + 1;
            let line = line
                .map_err(|e| ProtocolError::internal(format!("line {line_no}: read error: {e}")))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let record: KeyRecord = serde_json::from_str(trimmed).map_err(|e| {
                ProtocolError::internal(format!("line {line_no}: invalid JSON record: {e}"))
            })?;

            let algo = record.algo.as_deref().unwrap_or("DES");
            if !algo.eq_ignore_ascii_case("DES") {
                skipped_non_des += 1;
                continue;
            }

            let system_code = parse_hex_u16(&record.system_code, "system_code", line_no)?;
            let node = parse_hex_u16(&record.node, "node", line_no)?;
            let key = parse_hex_array::<DES_KEY_LEN>(&record.key, "key", line_no)?;

            // Remember first-appearance order — this is the system priority.
            if !store.system_code_order.contains(&system_code) {
                store.system_code_order.push(system_code);
            }

            let overwritten = match record.idm.as_deref().map(str::trim) {
                Some(idm_hex) if !idm_hex.is_empty() => {
                    let idm = parse_hex_array::<IDM_LEN>(idm_hex, "idm", line_no)?;
                    store
                        .per_idm
                        .insert((system_code, node, idm), key)
                        .is_some()
                }
                _ => store.system_wide.insert((system_code, node), key).is_some(),
            };
            if overwritten {
                tracing::debug!(
                    system_code = format!("0x{system_code:04X}"),
                    node = format!("0x{node:04X}"),
                    "duplicate DES key entry; later entry overrides earlier",
                );
            }
        }

        if skipped_non_des > 0 {
            tracing::debug!(skipped_non_des, "skipped non-DES key records");
        }

        Ok(store)
    }

    /// Fetch the DES key for `(system_code, node)`, preferring a key bound to
    /// `idm` when one exists.
    pub fn get_key(
        &self,
        system_code: u16,
        node: u16,
        idm: Option<&[u8; IDM_LEN]>,
    ) -> Result<[u8; DES_KEY_LEN], ProtocolError> {
        if let Some(idm) = idm
            && let Some(key) = self.per_idm.get(&(system_code, node, *idm))
        {
            return Ok(*key);
        }
        self.system_wide
            .get(&(system_code, node))
            .copied()
            .ok_or_else(|| {
                ProtocolError::bad_request(format!(
                    "missing key for system 0x{system_code:04X} node 0x{node:04X}"
                ))
            })
    }

    /// The system codes for which DES keys were loaded, **in `keys.jsonl` order**.
    /// This order is a priority: a terminal transacts under the first of these that
    /// the tapped card also exposes, so putting a system earlier in the key file
    /// makes it win on multi-system cards.
    pub fn system_codes(&self) -> Vec<u16> {
        self.system_code_order.clone()
    }

    /// Derive the DES group/user service keys used to mutually authenticate the
    /// given `areas` and `services` under `system_code` for the card `idm`.
    ///
    /// Mirrors the reference server: system key at node `0xFFFF`, then each area
    /// key and service key folded in via [`generate_service_keys_des`].
    pub fn derive_service_keys(
        &self,
        system_code: u16,
        idm: &[u8; IDM_LEN],
        areas: &[u16],
        services: &[u16],
    ) -> Result<DerivedKeys, ProtocolError> {
        let system_key = self.get_key(system_code, 0xFFFF, Some(idm))?;
        let area_keys = areas
            .iter()
            .map(|area| self.get_key(system_code, *area, Some(idm)))
            .collect::<Result<Vec<_>, _>>()?;
        let service_keys = services
            .iter()
            .map(|service| self.get_key(system_code, *service, Some(idm)))
            .collect::<Result<Vec<_>, _>>()?;
        let (group, user) = generate_service_keys_des(&system_key, &area_keys, &service_keys);
        Ok(DerivedKeys { group, user })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        r#"{"system_code":"0003","node":"FFFF","algo":"DES","version":"0000","idm":null,"key":"0011223344556677"}"#,
        "\n",
        r#"{"system_code":"0003","node":"0048","algo":"DES","version":"0000","idm":null,"key":"0102030405060708"}"#,
        "\n",
        r#"{"system_code":"0003","node":"0048","algo":"DES","version":"0001","idm":"1122334455667788","key":"AABBCCDDEEFF0011"}"#,
        "\n",
        r#"{"system_code":"FE00","node":"0000","algo":"AES","version":"0000","idm":null,"key":"000102030405060708090A0B0C0D0E0F"}"#,
        "\n\n",
    );

    fn store() -> KeyStore {
        KeyStore::from_reader(SAMPLE.as_bytes()).expect("sample keys should parse")
    }

    #[test]
    fn system_codes_follow_key_file_order() {
        // File order — NOT ascending — is the priority order a terminal applies.
        let jsonl = concat!(
            r#"{"system_code":"FE00","node":"FFFF","algo":"DES","version":"0000","idm":null,"key":"0011223344556677"}"#,
            "\n",
            r#"{"system_code":"0003","node":"FFFF","algo":"DES","version":"0000","idm":null,"key":"0102030405060708"}"#,
            "\n",
            // A second record for an already-seen system must not repeat it.
            r#"{"system_code":"FE00","node":"0048","algo":"DES","version":"0000","idm":null,"key":"0807060504030201"}"#,
            "\n",
        );
        let store = KeyStore::from_reader(jsonl.as_bytes()).expect("keys should parse");
        assert_eq!(store.system_codes(), vec![0xFE00, 0x0003]);
    }

    #[test]
    fn system_codes_exclude_systems_without_des_keys() {
        // SAMPLE's FE00 record is AES, so it is skipped and not usable.
        assert_eq!(store().system_codes(), vec![0x0003]);
    }

    #[test]
    fn parses_des_and_skips_aes() {
        let store = store();
        assert_eq!(
            store.get_key(0x0003, 0xFFFF, None).unwrap(),
            [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]
        );
        // AES record is skipped, so the FE00 system is not indexed.
        assert!(store.get_key(0xFE00, 0x0000, None).is_err());
    }

    #[test]
    fn prefers_idm_specific_key_with_fallback() {
        let store = store();
        let bound_idm = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let system_wide = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

        assert_eq!(
            store.get_key(0x0003, 0x0048, Some(&bound_idm)).unwrap(),
            [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11]
        );
        assert_eq!(
            store.get_key(0x0003, 0x0048, Some(&[0u8; 8])).unwrap(),
            system_wide
        );
        assert_eq!(store.get_key(0x0003, 0x0048, None).unwrap(), system_wide);
    }

    #[test]
    fn missing_key_is_a_bad_request() {
        let err = store().get_key(0x9999, 0x0000, None).unwrap_err();
        assert_eq!(err.status, 400);
        assert!(err.message.contains("missing key"));
    }

    #[test]
    fn derive_service_keys_matches_manual_derivation() {
        let store = store();
        let idm = [0u8; 8];
        let derived = store
            .derive_service_keys(0x0003, &idm, &[], &[0x0048])
            .unwrap();
        let (group, user) = generate_service_keys_des(
            &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
            &[],
            &[[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]],
        );
        assert_eq!(derived.group, group);
        assert_eq!(derived.user, user);
    }
}
