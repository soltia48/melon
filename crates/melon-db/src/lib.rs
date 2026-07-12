//! PostgreSQL persistence for melon (sqlx).
//!
//! This layer owns the SQL and the transaction/lock boundaries for every money
//! movement. The immutable ledger is the source of truth; `topup_buckets`
//! carries a transactionally-maintained `remaining_amount` cache for the payment
//! hot path. The soonest-expiry-first consumption decision is delegated to the
//! pure, tested [`melon_core::payment::plan_consumption`], applied here under a
//! `SELECT … FOR UPDATE` lock so concurrent payments can never overspend.

pub mod ops;
pub mod users;

use melon_core::expiry::ExpiryError;
use melon_core::money::{MoneyError, Yen};

/// A connection pool to the melon database.
pub type Pool = sqlx::PgPool;

/// Errors surfaced by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("insufficient funds: available {available}, requested {requested}")]
    InsufficientFunds { available: Yen, requested: Yen },
    #[error(
        "merchant credit limit exceeded: top-up up to {available} allowed, requested {requested}"
    )]
    CreditLimitExceeded { available: Yen, requested: Yen },
    #[error("idempotency key reused with different parameters")]
    IdempotencyConflict,
    #[error("account not found")]
    AccountNotFound,
    #[error("merchant not found")]
    MerchantNotFound,
    #[error("user not found")]
    UserNotFound,
    #[error("that email is already registered")]
    EmailTaken,
    #[error("merchant is not active")]
    MerchantNotActive,
    #[error("payment not found")]
    PaymentNotFound,
    #[error("refund of {requested} exceeds the {refundable} still refundable on this payment")]
    RefundExceedsPayment { requested: Yen, refundable: Yen },
    #[error(transparent)]
    Expiry(#[from] ExpiryError),
    #[error(transparent)]
    Money(#[from] MoneyError),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Open a pooled connection to `database_url`.
pub async fn connect(database_url: &str) -> Result<Pool, DbError> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(16)
        .connect(database_url)
        .await?;
    Ok(pool)
}

/// Apply all pending migrations embedded from `migrations/`.
pub async fn migrate(pool: &Pool) -> Result<(), DbError> {
    sqlx::migrate!().run(pool).await?;
    Ok(())
}

/// Conversions between melon-core domain types and the sqlx/Postgres wire types.
// Some helpers (kind/status strings) are used by later features — the expiry
// sweep and reporting — not yet wired up here.
#[allow(dead_code)]
pub(crate) mod conv {
    use jiff::Timestamp;
    use melon_core::ledger::{BucketStatus, LedgerKind, TxnKind};
    use time::OffsetDateTime;

    /// jiff instant -> the `time` type sqlx binds to `timestamptz`.
    pub fn to_odt(ts: Timestamp) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(ts.as_nanosecond())
            .expect("jiff timestamp within OffsetDateTime range")
    }

    /// `timestamptz` (as `time`) -> jiff instant.
    pub fn to_jiff(odt: OffsetDateTime) -> Timestamp {
        Timestamp::from_nanosecond(odt.unix_timestamp_nanos())
            .expect("OffsetDateTime within jiff range")
    }

    pub fn ledger_kind_str(kind: LedgerKind) -> &'static str {
        match kind {
            LedgerKind::TopUp => "top_up",
            LedgerKind::Payment => "payment",
            LedgerKind::Refund => "refund",
            LedgerKind::Expiry => "expiry",
            LedgerKind::Reversal => "reversal",
            LedgerKind::Adjustment => "adjustment",
        }
    }

    pub fn txn_kind_str(kind: TxnKind) -> &'static str {
        match kind {
            TxnKind::TopUp => "top_up",
            TxnKind::Payment => "payment",
            TxnKind::Refund => "refund",
            TxnKind::Reversal => "reversal",
            TxnKind::Adjustment => "adjustment",
        }
    }

    pub fn parse_bucket_status(s: &str) -> Option<BucketStatus> {
        match s {
            "active" => Some(BucketStatus::Active),
            "exhausted" => Some(BucketStatus::Exhausted),
            "expired" => Some(BucketStatus::Expired),
            _ => None,
        }
    }
}
