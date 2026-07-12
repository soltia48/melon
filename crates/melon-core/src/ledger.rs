//! Domain types for the immutable ledger, transactions and top-up buckets.
//!
//! The ledger is append-only and is the source of truth. A business event
//! ([`Transaction`]) produces one or more immutable postings ([`LedgerEntry`]),
//! each of which touches at most one top-up bucket ([`TopupBucket`]). Balance is
//! derived by summing signed posting amounts; `TopupBucket::remaining_amount` is
//! a transactionally-maintained cache of that sum for the payment hot path.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::idi::Idi;
use crate::money::Yen;

/// Lifecycle of a top-up bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketStatus {
    /// Has spendable value and has not yet expired.
    Active,
    /// Fully spent (`remaining_amount == 0`) but not (yet) expired.
    Exhausted,
    /// Past its expiry instant; any residual value has been forfeited.
    Expired,
}

/// The kind of an individual ledger posting. Carries accounting semantics; the
/// signed `amount` carries direction. A DB `CHECK` ties the two together (see
/// [`LedgerKind::sign_is_valid`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    /// Value added by a top-up (`+`).
    TopUp,
    /// Value spent at a merchant (`-`).
    Payment,
    /// Value returned to a bucket by a refund (`+`).
    Refund,
    /// Value forfeited when a bucket expires (`-`).
    Expiry,
    /// Technical correction that negates a prior posting (`±`).
    Reversal,
    /// Manual adjustment under maker/checker control (`±`).
    Adjustment,
}

impl LedgerKind {
    /// Whether a signed posting `amount` is consistent with this kind. Mirrors
    /// the DB `CHECK` constraint on `ledger_entries`.
    pub fn sign_is_valid(self, amount: Yen) -> bool {
        match self {
            LedgerKind::TopUp | LedgerKind::Refund => amount.is_positive(),
            LedgerKind::Payment | LedgerKind::Expiry => amount.is_negative(),
            LedgerKind::Reversal | LedgerKind::Adjustment => !amount.is_zero(),
        }
    }
}

/// The kind of a business-level transaction (取引).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxnKind {
    TopUp,
    Payment,
    Refund,
    Reversal,
    Adjustment,
}

/// A top-up lot: value added by one top-up, with its own expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopupBucket {
    pub id: Uuid,
    pub idi: Idi,
    pub topup_txn_id: Uuid,
    pub original_amount: Yen,
    /// Maintained cache of `SUM(ledger.amount WHERE bucket_id = id)`; in
    /// `0..=original_amount`.
    pub remaining_amount: Yen,
    pub topped_up_at: Timestamp,
    pub expires_at: Timestamp,
    pub status: BucketStatus,
}

/// One immutable posting in the ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub idi: Idi,
    pub transaction_id: Uuid,
    /// `Some` for value-bearing postings (top-up/payment/refund/expiry).
    pub bucket_id: Option<Uuid>,
    pub kind: LedgerKind,
    /// Signed delta.
    pub amount: Yen,
    pub created_at: Timestamp,
}

/// A business event grouping one or more [`LedgerEntry`] postings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Uuid,
    pub idi: Idi,
    pub kind: TxnKind,
    /// `None` for system top-ups.
    pub merchant_id: Option<Uuid>,
    /// Positive magnitude of the business event.
    pub amount: Yen,
    pub idempotency_key: String,
    /// Refund -> its payment; reversal -> its target.
    pub related_txn_id: Option<Uuid>,
    pub occurred_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_matches_kind() {
        assert!(LedgerKind::TopUp.sign_is_valid(Yen::new(100)));
        assert!(!LedgerKind::TopUp.sign_is_valid(Yen::new(-100)));
        assert!(LedgerKind::Payment.sign_is_valid(Yen::new(-100)));
        assert!(!LedgerKind::Payment.sign_is_valid(Yen::new(100)));
        assert!(LedgerKind::Expiry.sign_is_valid(Yen::new(-100)));
        assert!(LedgerKind::Refund.sign_is_valid(Yen::new(100)));
        assert!(LedgerKind::Reversal.sign_is_valid(Yen::new(-100)));
        assert!(LedgerKind::Reversal.sign_is_valid(Yen::new(100)));
        assert!(!LedgerKind::Adjustment.sign_is_valid(Yen::ZERO));
    }

    #[test]
    fn enums_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&LedgerKind::TopUp).unwrap(),
            "\"top_up\""
        );
        assert_eq!(
            serde_json::to_string(&BucketStatus::Exhausted).unwrap(),
            "\"exhausted\""
        );
        assert_eq!(
            serde_json::to_string(&TxnKind::Payment).unwrap(),
            "\"payment\""
        );
    }
}
