//! Account identity: the pair of FeliCa **System Code** and **IDi**.
//!
//! An IDi (issue ID) is only unique *within* a system code — the same physical
//! card exposes different IDis under different systems, and IDi bytes may
//! collide across systems/issuers. So an account is keyed by the pair, never by
//! IDi alone.

use std::fmt;

use crate::idi::Idi;

/// The composite identifier of an account: `(system_code, idi)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AccountKey {
    pub system_code: u16,
    pub idi: Idi,
}

impl AccountKey {
    pub const fn new(system_code: u16, idi: Idi) -> Self {
        Self { system_code, idi }
    }
}

impl fmt::Display for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // e.g. "0003:0102030405060708"
        write!(f, "{:04x}:{}", self.system_code, self.idi)
    }
}
