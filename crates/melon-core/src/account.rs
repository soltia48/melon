//! Account identity: the triple of FeliCa **IDm**, **System Code** and **IDi**.
//!
//! An IDi (issue ID) is only unique *within* a system code, and IDi bytes may
//! collide across systems/issuers — so an account is keyed by more than IDi
//! alone. This deployment also keys on the card's **IDm** (manufacture ID),
//! which is valid only because its cards have a *fixed* IDm; a randomized IDm
//! would make each tap look like a new account.

use std::fmt;

use crate::idi::Idi;
use crate::idm::Idm;

/// The composite identifier of an account: `(system_code, idm, idi)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AccountKey {
    pub system_code: u16,
    pub idm: Idm,
    pub idi: Idi,
}

impl AccountKey {
    pub const fn new(system_code: u16, idm: Idm, idi: Idi) -> Self {
        Self {
            system_code,
            idm,
            idi,
        }
    }
}

impl fmt::Display for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // e.g. "0003/0102030405060708:1122334455667788"
        write!(f, "{:04x}/{}:{}", self.system_code, self.idm, self.idi)
    }
}
