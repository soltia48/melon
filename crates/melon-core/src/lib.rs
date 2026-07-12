//! # melon-core
//!
//! Pure domain logic for melon, an online prepaid payment instrument
//! (前払式支払手段) keyed on FeliCa IDi. This crate has **no I/O** — no database,
//! HTTP, or async — so its rules are unit- and property-testable in isolation.
//! The database and service layers mirror these types and the
//! [`payment::plan_consumption`] algorithm under transactional locks.
//!
//! - [`money`] — [`money::Yen`] / [`money::PositiveYen`]: integer yen, no floats.
//! - [`idi`] — [`idi::Idi`]: the FeliCa issue-ID account key.
//! - [`expiry`] — the 6-month JST expiry boundary (資金決済法).
//! - [`ledger`] — immutable ledger, transaction and top-up-bucket types.
//! - [`payment`] — soonest-expiry-first consumption planning.

pub mod account;
pub mod expiry;
pub mod idi;
pub mod ledger;
pub mod money;
pub mod payment;

pub use account::AccountKey;
pub use idi::Idi;
pub use money::{MoneyError, PositiveYen, Yen};
