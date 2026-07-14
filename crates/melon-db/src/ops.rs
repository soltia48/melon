//! Repository operations: accounts, merchants, and the atomic money movements.
//!
//! Every money movement runs in one transaction. Payments lock the account's
//! candidate buckets with `SELECT … FOR UPDATE` (in the deterministic
//! consumption order, which also prevents deadlock), hand them to
//! [`plan_consumption`], apply the deductions, and append immutable ledger rows
//! — so concurrent payments on one account are serialized and can never
//! overspend. Idempotency is enforced by `UNIQUE (kind, idempotency_key)`.

use std::collections::HashMap;

use jiff::{Timestamp, tz::TimeZone};
use sqlx::{PgExecutor, Row};
use uuid::Uuid;

use melon_core::account::AccountKey;
use melon_core::expiry;
use melon_core::idi::Idi;
use melon_core::idm::Idm;
use melon_core::money::{PositiveYen, Yen};
use melon_core::payment::{Deduction, SpendableBucket, plan_consumption};

use crate::conv::{to_jiff, to_odt};
use crate::{DbError, Pool};

/// Reconstruct an [`AccountKey`] from a row exposing `idm`, `system_code`, `idi`.
fn account_from_row(r: &sqlx::postgres::PgRow) -> Result<AccountKey, DbError> {
    Ok(AccountKey::new(
        r.try_get::<i32, _>("system_code")? as u16,
        Idm::from_slice(&r.try_get::<Vec<u8>, _>("idm")?).map_err(|_| DbError::AccountNotFound)?,
        Idi::from_slice(&r.try_get::<Vec<u8>, _>("idi")?).map_err(|_| DbError::AccountNotFound)?,
    ))
}

/// One active bucket in a balance breakdown (soonest-expiry-first).
#[derive(Debug, Clone)]
pub struct BucketView {
    pub bucket_id: Uuid,
    pub remaining: Yen,
    pub expires_at: Timestamp,
}

/// Spendable balance and its per-bucket breakdown at a point in time.
#[derive(Debug, Clone)]
pub struct BalanceBreakdown {
    pub total: Yen,
    pub buckets: Vec<BucketView>,
}

/// Outcome of a top-up.
#[derive(Debug, Clone)]
pub struct TopUp {
    pub transaction_id: Uuid,
    pub bucket_id: Uuid,
    pub amount: Yen,
    pub expires_at: Timestamp,
    pub balance: Yen,
    pub replayed: bool,
}

/// Outcome of a payment.
#[derive(Debug, Clone)]
pub struct Payment {
    pub transaction_id: Uuid,
    pub amount: Yen,
    /// Processing fee charged to the merchant (net to merchant = amount − fee).
    pub fee: Yen,
    pub deductions: Vec<Deduction>,
    pub balance: Yen,
    pub replayed: bool,
}

// ----- per-merchant pseudonymous account IDs -----

/// The merchant-scoped alias for `account`, creating it on first sight. Merchants
/// only ever see this opaque UUID — never the raw `(system_code, idi)`. The same
/// card yields a DIFFERENT alias at each merchant, so merchants cannot correlate a
/// cardholder across merchants.
pub async fn alias_for(
    pool: &Pool,
    merchant_id: Uuid,
    account: AccountKey,
) -> Result<Uuid, DbError> {
    let sc = account.system_code as i32;
    let idm = account.idm.as_bytes().as_slice();
    let idi = account.idi.as_bytes().as_slice();

    let mut tx = pool.begin().await?;
    // The alias references accounts, and an account is otherwise only created on
    // its first money movement — so make sure the row exists.
    ensure_account(&mut tx, account).await?;

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO merchant_account_aliases (alias, merchant_id, system_code, idm, idi)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (merchant_id, system_code, idm, idi) DO NOTHING
         RETURNING alias",
    )
    .bind(Uuid::new_v4()) // v4: opaque (v7 would leak the creation time)
    .bind(merchant_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .fetch_optional(&mut *tx)
    .await?;

    let alias = match inserted {
        Some(alias) => alias,
        // Already issued: reuse it so the merchant sees a stable ID.
        None => {
            sqlx::query_scalar(
                "SELECT alias FROM merchant_account_aliases
              WHERE merchant_id = $1 AND system_code = $2 AND idm = $3 AND idi = $4",
            )
            .bind(merchant_id)
            .bind(sc)
            .bind(idm)
            .bind(idi)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(alias)
}

/// Resolve one of **this merchant's** aliases back to the real account. Scoped to
/// `merchant_id`, so a merchant cannot use another merchant's alias.
pub async fn account_for_alias(
    pool: &Pool,
    merchant_id: Uuid,
    alias: Uuid,
) -> Result<Option<AccountKey>, DbError> {
    let row = sqlx::query(
        "SELECT system_code, idm, idi FROM merchant_account_aliases
          WHERE alias = $1 AND merchant_id = $2",
    )
    .bind(alias)
    .bind(merchant_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(account_from_row(&r)?)),
        None => Ok(None),
    }
}

/// The merchant that owns a payment transaction, if it exists.
pub async fn payment_merchant(pool: &Pool, payment_txn_id: Uuid) -> Result<Option<Uuid>, DbError> {
    let row =
        sqlx::query("SELECT merchant_id FROM transactions WHERE id = $1 AND kind = 'payment'")
            .bind(payment_txn_id)
            .fetch_optional(pool)
            .await?;
    match row {
        Some(r) => Ok(r.try_get("merchant_id")?),
        None => Ok(None),
    }
}

/// A merchant row for admin listings, including its settlement balance
/// (collected payments minus refunds/reversals).
#[derive(Debug, Clone)]
pub struct MerchantRow {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    /// Payment fee rate in basis points (1 bps = 0.01%).
    pub fee_bps: i32,
    /// How far negative the settlement balance may go (for selling top-ups).
    pub credit_limit: Yen,
    pub collected: Yen,
    pub created_at: Timestamp,
}

/// The SQL for a merchant's settlement balance (what the issuer owes the
/// merchant): payments accepted, minus top-ups collected (the merchant holds the
/// issuer's cash), minus refunds/reversals, plus admin adjustments. `$1` is the
/// merchant id.
const MERCHANT_BALANCE_SQL: &str = "(
    COALESCE((SELECT SUM(CASE WHEN t.kind = 'payment' THEN t.amount - t.fee
                              WHEN t.kind IN ('top_up', 'refund', 'reversal') THEN -t.amount
                              ELSE 0 END)
                FROM transactions t WHERE t.merchant_id = $1), 0)
  + COALESCE((SELECT SUM(a.amount) FROM merchant_adjustments a WHERE a.merchant_id = $1), 0)
)::bigint";

/// List all merchants (with settlement balance), newest first.
pub async fn list_merchants(pool: &Pool) -> Result<Vec<MerchantRow>, DbError> {
    let rows = sqlx::query(
        "SELECT m.id, m.code, m.name, m.status, m.fee_bps, m.credit_limit, m.created_at,
                COALESCE(pay.collected, 0) + COALESCE(adj.total, 0) AS collected
           FROM merchants m
           LEFT JOIN (
               SELECT merchant_id,
                      SUM(CASE WHEN kind = 'payment' THEN amount - fee
                               WHEN kind IN ('top_up', 'refund', 'reversal') THEN -amount
                               ELSE 0 END)::bigint AS collected
                 FROM transactions GROUP BY merchant_id
           ) pay ON pay.merchant_id = m.id
           LEFT JOIN (
               SELECT merchant_id, SUM(amount)::bigint AS total
                 FROM merchant_adjustments GROUP BY merchant_id
           ) adj ON adj.merchant_id = m.id
          ORDER BY m.created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_merchant_row).collect()
}

fn map_merchant_row(r: sqlx::postgres::PgRow) -> Result<MerchantRow, DbError> {
    Ok(MerchantRow {
        id: r.try_get("id")?,
        code: r.try_get("code")?,
        name: r.try_get("name")?,
        status: r.try_get("status")?,
        fee_bps: r.try_get("fee_bps")?,
        credit_limit: Yen::new(r.try_get::<i64, _>("credit_limit")?),
        collected: Yen::new(r.try_get::<i64, _>("collected")?),
        created_at: to_jiff(r.try_get("created_at")?),
    })
}

/// Fetch a single merchant (with settlement balance) by id.
pub async fn get_merchant(pool: &Pool, merchant_id: Uuid) -> Result<Option<MerchantRow>, DbError> {
    let row = sqlx::query(
        "SELECT m.id, m.code, m.name, m.status, m.fee_bps, m.credit_limit, m.created_at,
                COALESCE(pay.collected, 0) + COALESCE(adj.total, 0) AS collected
           FROM merchants m
           LEFT JOIN (
               SELECT merchant_id,
                      SUM(CASE WHEN kind = 'payment' THEN amount - fee
                               WHEN kind IN ('top_up', 'refund', 'reversal') THEN -amount
                               ELSE 0 END)::bigint AS collected
                 FROM transactions GROUP BY merchant_id
           ) pay ON pay.merchant_id = m.id
           LEFT JOIN (
               SELECT merchant_id, SUM(amount)::bigint AS total
                 FROM merchant_adjustments GROUP BY merchant_id
           ) adj ON adj.merchant_id = m.id
          WHERE m.id = $1",
    )
    .bind(merchant_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(map_merchant_row(r)?)),
        None => Ok(None),
    }
}

/// Outcome of a merchant settlement adjustment.
#[derive(Debug, Clone)]
pub struct MerchantAdjustment {
    pub id: Uuid,
    pub delta: Yen,
    pub balance: Yen,
}

/// Adjust a merchant's settlement balance by a signed `delta` (non-zero),
/// recorded immutably with an optional reason. Returns the new balance. The
/// settlement balance may legitimately go negative (a clawback beyond what was
/// collected), so no floor is enforced.
pub async fn adjust_merchant(
    pool: &Pool,
    merchant_id: Uuid,
    delta: Yen,
    note: Option<&str>,
) -> Result<MerchantAdjustment, DbError> {
    if delta.is_zero() {
        return Err(DbError::Money(melon_core::money::MoneyError::NonPositive));
    }
    let mut tx = pool.begin().await?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Err(DbError::MerchantNotFound);
    }
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO merchant_adjustments (id, merchant_id, amount, note) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(merchant_id)
    .bind(delta.as_i64())
    .bind(note)
    .execute(&mut *tx)
    .await?;
    let balance: i64 = sqlx::query_scalar(&format!("SELECT {MERCHANT_BALANCE_SQL}"))
        .bind(merchant_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(MerchantAdjustment {
        id,
        delta,
        balance: Yen::new(balance),
    })
}

/// Update a merchant's status. `status` must be one of active/suspended/closed.
pub async fn set_merchant_status(
    pool: &Pool,
    merchant_id: Uuid,
    status: &str,
) -> Result<(), DbError> {
    let affected =
        sqlx::query("UPDATE merchants SET status = $1, updated_at = now() WHERE id = $2")
            .bind(status)
            .bind(merchant_id)
            .execute(pool)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(DbError::MerchantNotFound);
    }
    Ok(())
}

/// One account with its current spendable balance.
#[derive(Debug, Clone)]
pub struct AccountSummary {
    pub account: AccountKey,
    pub status: String,
    pub balance: Yen,
    pub created_at: Timestamp,
}

/// List accounts with their spendable balance at `now`, highest balance first.
pub async fn list_accounts(
    pool: &Pool,
    now: Timestamp,
    limit: i64,
) -> Result<Vec<AccountSummary>, DbError> {
    let limit = if limit <= 0 { 200 } else { limit.min(1000) };
    let rows = sqlx::query(
        "SELECT a.system_code, a.idm, a.idi, a.status, a.created_at,
                COALESCE(SUM(b.remaining_amount)
                    FILTER (WHERE b.status = 'active' AND b.expires_at > $1 AND b.remaining_amount > 0),
                    0)::bigint AS balance
           FROM accounts a
           LEFT JOIN topup_buckets b
             ON b.system_code = a.system_code AND b.idm = a.idm AND b.idi = a.idi
          GROUP BY a.system_code, a.idm, a.idi, a.status, a.created_at
          ORDER BY balance DESC, a.created_at DESC
          LIMIT $2",
    )
    .bind(to_odt(now))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(AccountSummary {
                account: account_from_row(&r)?,
                status: r.try_get("status")?,
                balance: Yen::new(r.try_get::<i64, _>("balance")?),
                created_at: to_jiff(r.try_get("created_at")?),
            })
        })
        .collect()
}

/// Outcome of an admin balance adjustment.
#[derive(Debug, Clone)]
pub struct Adjustment {
    pub transaction_id: Uuid,
    /// Signed delta actually applied.
    pub delta: Yen,
    pub balance: Yen,
    /// The new bucket created for a credit; `None` for a debit.
    pub bucket_id: Option<Uuid>,
}

/// Admin adjustment of an account balance by a signed `delta` (non-zero),
/// recorded immutably as an `adjustment` transaction with an optional reason.
///
/// A **credit** (`delta > 0`) mints a new 6-month bucket, like a top-up. A
/// **debit** (`delta < 0`) consumes soonest-expiry-first and fails if the
/// balance cannot cover it (never goes negative). Not the audit-safe way to
/// "set" a balance — it always leaves a signed ledger trail.
pub async fn adjust(
    pool: &Pool,
    account: AccountKey,
    delta: Yen,
    reason: Option<&str>,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<Adjustment, DbError> {
    if delta.is_zero() {
        return Err(DbError::Money(melon_core::money::MoneyError::NonPositive));
    }
    let magnitude = delta.as_i64().abs();
    let sc = account.system_code as i32;
    let idm = account.idm.as_bytes().as_slice();
    let idi = account.idi.as_bytes().as_slice();

    let mut tx = pool.begin().await?;
    ensure_account(&mut tx, account).await?;

    let txn_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO transactions (id, system_code, idm, idi, kind, amount, idempotency_key, note, occurred_at)
         VALUES ($1, $2, $3, $4, 'adjustment', $5, $6, $7, $8)",
    )
    .bind(txn_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(magnitude)
    .bind(txn_id.to_string())
    .bind(reason)
    .bind(to_odt(now))
    .execute(&mut *tx)
    .await?;

    let bucket_id = if delta.is_positive() {
        // Credit: mint a fresh bucket with the standard 6-month expiry.
        let expires_at = expiry::expires_at_in(now, tz)?;
        let bucket_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO topup_buckets
                 (id, system_code, idm, idi, topup_txn_id, original_amount, remaining_amount, topped_up_at, expires_at, status)
             VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, 'active')",
        )
        .bind(bucket_id)
        .bind(sc)
        .bind(idm)
        .bind(idi)
        .bind(txn_id)
        .bind(magnitude)
        .bind(to_odt(now))
        .bind(to_odt(expires_at))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO ledger_entries (id, system_code, idm, idi, transaction_id, bucket_id, kind, amount, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'adjustment', $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(sc)
        .bind(idm)
        .bind(idi)
        .bind(txn_id)
        .bind(bucket_id)
        .bind(magnitude)
        .bind(to_odt(now))
        .execute(&mut *tx)
        .await?;
        Some(bucket_id)
    } else {
        // Debit: consume soonest-expiry-first, never going negative.
        let rows = sqlx::query(
            "SELECT id, remaining_amount, topped_up_at, expires_at
               FROM topup_buckets
              WHERE system_code = $1 AND idm = $2 AND idi = $3 AND status = 'active'
                AND expires_at > $4 AND remaining_amount > 0
              ORDER BY expires_at, topped_up_at, id
              FOR UPDATE",
        )
        .bind(sc)
        .bind(idm)
        .bind(idi)
        .bind(to_odt(now))
        .fetch_all(&mut *tx)
        .await?;
        let locked: Vec<SpendableBucket> = rows
            .into_iter()
            .map(|r| {
                Ok(SpendableBucket {
                    id: r.try_get("id")?,
                    remaining: Yen::new(r.try_get::<i64, _>("remaining_amount")?),
                    topped_up_at: to_jiff(r.try_get("topped_up_at")?),
                    expires_at: to_jiff(r.try_get("expires_at")?),
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        let want = PositiveYen::from_i64(magnitude)?;
        let deductions =
            plan_consumption(&locked, want, now).map_err(|e| DbError::InsufficientFunds {
                available: e.available,
                requested: e.requested,
            })?;
        for d in &deductions {
            sqlx::query(
                "UPDATE topup_buckets
                    SET remaining_amount = remaining_amount - $1,
                        status = CASE WHEN remaining_amount - $1 = 0 THEN 'exhausted' ELSE status END
                  WHERE id = $2",
            )
            .bind(d.amount.as_i64())
            .bind(d.bucket_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO ledger_entries (id, system_code, idm, idi, transaction_id, bucket_id, kind, amount, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'adjustment', $7, $8)",
            )
            .bind(Uuid::now_v7())
            .bind(sc)
            .bind(idm)
            .bind(idi)
            .bind(txn_id)
            .bind(d.bucket_id)
            .bind(-d.amount.as_i64())
            .bind(to_odt(now))
            .execute(&mut *tx)
            .await?;
        }
        None
    };

    let bal = balance(&mut *tx, account, now).await?;
    tx.commit().await?;
    Ok(Adjustment {
        transaction_id: txn_id,
        delta,
        balance: bal.total,
        bucket_id,
    })
}

/// Create a merchant with a payment fee rate (bps) and credit limit (yen),
/// returning its id.
pub async fn create_merchant(
    pool: &Pool,
    code: &str,
    name: &str,
    fee_bps: i32,
    credit_limit: i64,
) -> Result<Uuid, DbError> {
    let mut tx = pool.begin().await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO merchants (id, code, name, fee_bps, credit_limit) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(code)
    .bind(name)
    .bind(fee_bps)
    .bind(credit_limit)
    .execute(&mut *tx)
    .await?;
    // Every merchant starts with one default store.
    sqlx::query(
        "INSERT INTO stores (id, merchant_id, code, name, is_default)
         VALUES ($1, $2, 'default', '本店', true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

// ----- stores (店舗) -----

#[derive(Debug, Clone)]
pub struct StoreRow {
    pub id: Uuid,
    pub merchant_id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub is_default: bool,
    pub created_at: Timestamp,
}

fn store_row(r: sqlx::postgres::PgRow) -> Result<StoreRow, DbError> {
    Ok(StoreRow {
        id: r.try_get("id")?,
        merchant_id: r.try_get("merchant_id")?,
        code: r.try_get("code")?,
        name: r.try_get("name")?,
        status: r.try_get("status")?,
        is_default: r.try_get("is_default")?,
        created_at: to_jiff(r.try_get("created_at")?),
    })
}

/// Create a store under a merchant. Fails with `StoreCodeTaken` if the merchant
/// already has a store with that code.
pub async fn create_store(
    pool: &Pool,
    merchant_id: Uuid,
    code: &str,
    name: &str,
) -> Result<Uuid, DbError> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(DbError::MerchantNotFound);
    }
    let id = Uuid::now_v7();
    match sqlx::query("INSERT INTO stores (id, merchant_id, code, name) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(merchant_id)
        .bind(code)
        .bind(name)
        .execute(pool)
        .await
    {
        Ok(_) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(DbError::StoreCodeTaken),
        Err(e) => Err(e.into()),
    }
}

/// List a merchant's stores (default first, then by creation order).
pub async fn list_stores(pool: &Pool, merchant_id: Uuid) -> Result<Vec<StoreRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, merchant_id, code, name, status, is_default, created_at FROM stores
          WHERE merchant_id = $1 ORDER BY is_default DESC, created_at",
    )
    .bind(merchant_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(store_row).collect()
}

pub async fn get_store(pool: &Pool, store_id: Uuid) -> Result<Option<StoreRow>, DbError> {
    sqlx::query(
        "SELECT id, merchant_id, code, name, status, is_default, created_at
           FROM stores WHERE id = $1",
    )
    .bind(store_id)
    .fetch_optional(pool)
    .await?
    .map(store_row)
    .transpose()
}

/// The id of a merchant's default store.
pub async fn default_store_id(pool: &Pool, merchant_id: Uuid) -> Result<Option<Uuid>, DbError> {
    Ok(
        sqlx::query_scalar("SELECT id FROM stores WHERE merchant_id = $1 AND is_default LIMIT 1")
            .bind(merchant_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn set_store_status(pool: &Pool, store_id: Uuid, status: &str) -> Result<(), DbError> {
    let affected = sqlx::query("UPDATE stores SET status = $1, updated_at = now() WHERE id = $2")
        .bind(status)
        .bind(store_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(DbError::StoreNotFound);
    }
    Ok(())
}

pub async fn update_store_name(pool: &Pool, store_id: Uuid, name: &str) -> Result<(), DbError> {
    let affected = sqlx::query("UPDATE stores SET name = $1, updated_at = now() WHERE id = $2")
        .bind(name)
        .bind(store_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(DbError::StoreNotFound);
    }
    Ok(())
}

/// Update a merchant's credit limit (yen, >= 0).
pub async fn set_merchant_credit_limit(
    pool: &Pool,
    merchant_id: Uuid,
    credit_limit: i64,
) -> Result<(), DbError> {
    let affected =
        sqlx::query("UPDATE merchants SET credit_limit = $1, updated_at = now() WHERE id = $2")
            .bind(credit_limit)
            .bind(merchant_id)
            .execute(pool)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(DbError::MerchantNotFound);
    }
    Ok(())
}

/// Update a merchant's payment fee rate (basis points, 0..=10000).
pub async fn set_merchant_fee(pool: &Pool, merchant_id: Uuid, fee_bps: i32) -> Result<(), DbError> {
    let affected =
        sqlx::query("UPDATE merchants SET fee_bps = $1, updated_at = now() WHERE id = $2")
            .bind(fee_bps)
            .bind(merchant_id)
            .execute(pool)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(DbError::MerchantNotFound);
    }
    Ok(())
}

/// A merchant resolved from an API key, with the store the key belongs to.
#[derive(Debug, Clone)]
pub struct MerchantAuth {
    pub merchant_id: Uuid,
    pub status: String,
    /// The store this API key is scoped to (NULL only for legacy keys predating
    /// the store backfill).
    pub store_id: Option<Uuid>,
    /// Which key was presented. Audit lines name it, so a key that turns out to
    /// have leaked can be traced to everything it did — and then revoked.
    pub key_id: Uuid,
}

/// Store a store-scoped merchant API key (its SHA-256 hash), returning the key id.
pub async fn store_api_key(
    pool: &Pool,
    merchant_id: Uuid,
    store_id: Uuid,
    key_hash: &str,
    label: Option<&str>,
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO merchant_api_keys (id, merchant_id, store_id, key_hash, label)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(merchant_id)
    .bind(store_id)
    .bind(key_hash)
    .bind(label)
    .execute(pool)
    .await?;
    Ok(id)
}

/// A stored API key (metadata only — never the secret or its hash).
#[derive(Debug, Clone)]
pub struct ApiKeyRow {
    pub id: Uuid,
    pub store_id: Option<Uuid>,
    pub label: Option<String>,
    pub created_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}

/// List a merchant's API keys (newest first), optionally scoped to one store.
pub async fn list_api_keys(
    pool: &Pool,
    merchant_id: Uuid,
    store_id: Option<Uuid>,
) -> Result<Vec<ApiKeyRow>, DbError> {
    let rows = sqlx::query(
        "SELECT id, store_id, label, created_at, revoked_at FROM merchant_api_keys
          WHERE merchant_id = $1 AND ($2::uuid IS NULL OR store_id = $2)
          ORDER BY created_at DESC",
    )
    .bind(merchant_id)
    .bind(store_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(ApiKeyRow {
                id: r.try_get("id")?,
                store_id: r.try_get("store_id")?,
                label: r.try_get("label")?,
                created_at: to_jiff(r.try_get("created_at")?),
                revoked_at: r.try_get::<Option<_>, _>("revoked_at")?.map(to_jiff),
            })
        })
        .collect()
}

/// Revoke one API key by id, scoped to `merchant_id` (so a merchant can only
/// revoke its own). Returns true if a live key was revoked.
pub async fn revoke_api_key(pool: &Pool, merchant_id: Uuid, key_id: Uuid) -> Result<bool, DbError> {
    let affected = sqlx::query(
        "UPDATE merchant_api_keys SET revoked_at = now()
          WHERE id = $1 AND merchant_id = $2 AND revoked_at IS NULL",
    )
    .bind(key_id)
    .bind(merchant_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Rotate a merchant's API key: revoke every current (non-revoked) key and
/// store `key_hash` as the new one. Returns the new key id.
pub async fn rotate_api_key(
    pool: &Pool,
    merchant_id: Uuid,
    store_id: Uuid,
    key_hash: &str,
    label: Option<&str>,
) -> Result<Uuid, DbError> {
    let mut tx = pool.begin().await?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Err(DbError::MerchantNotFound);
    }
    // Rotate only the target store's live keys.
    sqlx::query(
        "UPDATE merchant_api_keys SET revoked_at = now()
          WHERE merchant_id = $1 AND store_id = $2 AND revoked_at IS NULL",
    )
    .bind(merchant_id)
    .bind(store_id)
    .execute(&mut *tx)
    .await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO merchant_api_keys (id, merchant_id, store_id, key_hash, label)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(merchant_id)
    .bind(store_id)
    .bind(key_hash)
    .bind(label)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Resolve the (active-or-not) merchant owning a non-revoked API key by its hash.
pub async fn merchant_by_key_hash(
    pool: &Pool,
    key_hash: &str,
) -> Result<Option<MerchantAuth>, DbError> {
    let row = sqlx::query(
        "SELECT m.id, m.status, k.store_id, k.id AS key_id FROM merchant_api_keys k
           JOIN merchants m ON m.id = k.merchant_id
          WHERE k.key_hash = $1 AND k.revoked_at IS NULL",
    )
    .bind(key_hash)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(MerchantAuth {
            merchant_id: r.try_get("id")?,
            status: r.try_get("status")?,
            store_id: r.try_get("store_id")?,
            key_id: r.try_get("key_id")?,
        })),
        None => Ok(None),
    }
}

/// Spendable balance for `account` at `now` (excludes expired/empty buckets).
pub async fn balance<'e, E>(
    exec: E,
    account: AccountKey,
    now: Timestamp,
) -> Result<BalanceBreakdown, DbError>
where
    E: PgExecutor<'e>,
{
    let rows = sqlx::query(
        "SELECT id, remaining_amount, expires_at
           FROM topup_buckets
          WHERE system_code = $1 AND idm = $2 AND idi = $3 AND status = 'active'
            AND expires_at > $4 AND remaining_amount > 0
          ORDER BY expires_at, topped_up_at, id",
    )
    .bind(account.system_code as i32)
    .bind(account.idm.as_bytes().as_slice())
    .bind(account.idi.as_bytes().as_slice())
    .bind(to_odt(now))
    .fetch_all(exec)
    .await?;

    let mut buckets = Vec::with_capacity(rows.len());
    let mut total = 0i64;
    for row in rows {
        let remaining: i64 = row.try_get("remaining_amount")?;
        total = total.saturating_add(remaining);
        buckets.push(BucketView {
            bucket_id: row.try_get("id")?,
            remaining: Yen::new(remaining),
            expires_at: to_jiff(row.try_get("expires_at")?),
        });
    }
    Ok(BalanceBreakdown {
        total: Yen::new(total),
        buckets,
    })
}

/// Whether any account exists for this `(system_code, idi)`. Lets the
/// self-service lookup tell "no melon account for this Suica ID" (404) apart from
/// "account exists, balance is zero" (200). An IDi is unique within its system, so
/// this matches at most one account.
pub async fn account_exists_by_idi<'e, E>(
    exec: E,
    system_code: u16,
    idi: Idi,
) -> Result<bool, DbError>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query("SELECT 1 FROM accounts WHERE system_code = $1 AND idi = $2 LIMIT 1")
        .bind(system_code as i32)
        .bind(idi.as_bytes().as_slice())
        .fetch_optional(exec)
        .await?;
    Ok(row.is_some())
}

/// Spendable balance for the account identified by `(system_code, idi)`. Used by
/// the unauthenticated self-service lookup, where the cardholder supplies their
/// own IDi (the string form of which is the "Suica ID" shown in wallet apps). An
/// IDi is unique within its system, so this addresses one account's buckets.
pub async fn balance_by_idi<'e, E>(
    exec: E,
    system_code: u16,
    idi: Idi,
    now: Timestamp,
) -> Result<BalanceBreakdown, DbError>
where
    E: PgExecutor<'e>,
{
    let rows = sqlx::query(
        "SELECT id, remaining_amount, expires_at
           FROM topup_buckets
          WHERE system_code = $1 AND idi = $2 AND status = 'active'
            AND expires_at > $3 AND remaining_amount > 0
          ORDER BY expires_at, topped_up_at, id",
    )
    .bind(system_code as i32)
    .bind(idi.as_bytes().as_slice())
    .bind(to_odt(now))
    .fetch_all(exec)
    .await?;

    let mut buckets = Vec::with_capacity(rows.len());
    let mut total = 0i64;
    for row in rows {
        let remaining: i64 = row.try_get("remaining_amount")?;
        total = total.saturating_add(remaining);
        buckets.push(BucketView {
            bucket_id: row.try_get("id")?,
            remaining: Yen::new(remaining),
            expires_at: to_jiff(row.try_get("expires_at")?),
        });
    }
    Ok(BalanceBreakdown {
        total: Yen::new(total),
        buckets,
    })
}

async fn ensure_account(tx: &mut sqlx::PgConnection, account: AccountKey) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO accounts (system_code, idm, idi) VALUES ($1, $2, $3)
         ON CONFLICT (system_code, idm, idi) DO NOTHING",
    )
    .bind(account.system_code as i32)
    .bind(account.idm.as_bytes().as_slice())
    .bind(account.idi.as_bytes().as_slice())
    .execute(tx)
    .await?;
    Ok(())
}

/// Add `amount` to `account` as a new 6-month bucket. Idempotent on
/// `idempotency_key`. `merchant_id` records which merchant performed the top-up
/// (they collected the cash on the issuer's behalf); `None` for an issuer/system
/// top-up.
#[allow(clippy::too_many_arguments)]
pub async fn top_up(
    pool: &Pool,
    account: AccountKey,
    merchant_id: Option<Uuid>,
    store_id: Option<Uuid>,
    amount: PositiveYen,
    idempotency_key: &str,
    now: Timestamp,
    tz: &TimeZone,
) -> Result<TopUp, DbError> {
    let sc = account.system_code as i32;
    let idm = account.idm.as_bytes().as_slice();
    let idi = account.idi.as_bytes().as_slice();

    let mut tx = pool.begin().await?;

    // Lock the merchant row (if attributed) so concurrent top-ups serialize on
    // the credit-limit check below.
    let credit_limit: Option<i64> = match merchant_id {
        Some(mid) => {
            let row = sqlx::query("SELECT credit_limit FROM merchants WHERE id = $1 FOR UPDATE")
                .bind(mid)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(DbError::MerchantNotFound)?;
            Some(row.try_get("credit_limit")?)
        }
        None => None,
    };

    ensure_account(&mut tx, account).await?;

    let txn_id = Uuid::now_v7();
    let inserted: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO transactions (id, system_code, idm, idi, kind, merchant_id, store_id, amount, idempotency_key, occurred_at)
         VALUES ($1, $2, $3, $4, 'top_up', $5, $6, $7, $8, $9)
         ON CONFLICT (kind, idempotency_key) DO NOTHING
         RETURNING id",
    )
    .bind(txn_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(merchant_id)
    .bind(store_id)
    .bind(amount.as_i64())
    .bind(idempotency_key)
    .bind(to_odt(now))
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        // Replay: return the original top-up verbatim.
        let row = sqlx::query(
            "SELECT id, system_code, idm, idi, merchant_id, amount FROM transactions WHERE kind = 'top_up' AND idempotency_key = $1",
        )
        .bind(idempotency_key)
        .fetch_one(&mut *tx)
        .await?;
        let existing_id: Uuid = row.try_get("id")?;
        let existing_idm: Vec<u8> = row.try_get("idm")?;
        let existing_sc: i32 = row.try_get("system_code")?;
        let existing_idi: Vec<u8> = row.try_get("idi")?;
        let existing_merchant: Option<Uuid> = row.try_get("merchant_id")?;
        let existing_amount: i64 = row.try_get("amount")?;
        if existing_idm.as_slice() != idm
            || existing_sc != sc
            || existing_idi.as_slice() != idi
            || existing_merchant != merchant_id
            || existing_amount != amount.as_i64()
        {
            return Err(DbError::IdempotencyConflict);
        }
        let brow = sqlx::query("SELECT id, expires_at FROM topup_buckets WHERE topup_txn_id = $1")
            .bind(existing_id)
            .fetch_one(&mut *tx)
            .await?;
        let bucket_id: Uuid = brow.try_get("id")?;
        let expires_at = to_jiff(brow.try_get("expires_at")?);
        let bal = balance(&mut *tx, account, now).await?;
        tx.commit().await?;
        return Ok(TopUp {
            transaction_id: existing_id,
            bucket_id,
            amount: amount.get(),
            expires_at,
            balance: bal.total,
            replayed: true,
        });
    }

    // Enforce the merchant credit limit (revolving): the top-up just inserted must
    // not push the merchant's settlement below -credit_limit. The limit bounds the
    // *current* settlement, so payments received restore top-up headroom. Refunds
    // are a consumer obligation and are never credit-checked, so a refund may
    // legitimately push settlement below -credit_limit.
    if let (Some(mid), Some(limit)) = (merchant_id, credit_limit) {
        let settlement: i64 = sqlx::query_scalar(&format!("SELECT {MERCHANT_BALANCE_SQL}"))
            .bind(mid)
            .fetch_one(&mut *tx)
            .await?;
        if settlement < -limit {
            return Err(DbError::CreditLimitExceeded {
                available: Yen::new(settlement + amount.as_i64() + limit),
                requested: amount.get(),
            });
        }
    }

    let expires_at = expiry::expires_at_in(now, tz)?;
    let bucket_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO topup_buckets
             (id, system_code, idm, idi, topup_txn_id, original_amount, remaining_amount, topped_up_at, expires_at, status)
         VALUES ($1, $2, $3, $4, $5, $6, $6, $7, $8, 'active')",
    )
    .bind(bucket_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(txn_id)
    .bind(amount.as_i64())
    .bind(to_odt(now))
    .bind(to_odt(expires_at))
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO ledger_entries (id, system_code, idm, idi, transaction_id, bucket_id, kind, amount, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'top_up', $7, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(txn_id)
    .bind(bucket_id)
    .bind(amount.as_i64())
    .bind(to_odt(now))
    .execute(&mut *tx)
    .await?;

    let bal = balance(&mut *tx, account, now).await?;
    tx.commit().await?;
    Ok(TopUp {
        transaction_id: txn_id,
        bucket_id,
        amount: amount.get(),
        expires_at,
        balance: bal.total,
        replayed: false,
    })
}

/// Charge `amount` from `account` on behalf of `merchant_id`, drawing
/// soonest-expiry first. Atomic and never overspends. Idempotent.
#[allow(clippy::too_many_arguments)]
pub async fn pay(
    pool: &Pool,
    account: AccountKey,
    merchant_id: Uuid,
    store_id: Option<Uuid>,
    amount: PositiveYen,
    idempotency_key: &str,
    note: Option<&str>,
    now: Timestamp,
) -> Result<Payment, DbError> {
    let sc = account.system_code as i32;
    let idm = account.idm.as_bytes().as_slice();
    let idi = account.idi.as_bytes().as_slice();

    let mut tx = pool.begin().await?;

    let merchant_row = sqlx::query("SELECT status, fee_bps FROM merchants WHERE id = $1")
        .bind(merchant_id)
        .fetch_optional(&mut *tx)
        .await?;
    let fee_bps: i32 = match merchant_row {
        None => return Err(DbError::MerchantNotFound),
        Some(r) => {
            if r.try_get::<String, _>("status")? != "active" {
                return Err(DbError::MerchantNotActive);
            }
            r.try_get("fee_bps")?
        }
    };
    let fee = amount.get().fee_bps(fee_bps);

    ensure_account(&mut tx, account).await?;

    let txn_id = Uuid::now_v7();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO transactions (id, system_code, idm, idi, kind, merchant_id, store_id, amount, fee, idempotency_key, note, occurred_at)
         VALUES ($1, $2, $3, $4, 'payment', $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (kind, idempotency_key) DO NOTHING
         RETURNING id",
    )
    .bind(txn_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(merchant_id)
    .bind(store_id)
    .bind(amount.as_i64())
    .bind(fee.as_i64())
    .bind(idempotency_key)
    .bind(note)
    .bind(to_odt(now))
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        // Replay: reconstruct the original deductions from the ledger.
        let row = sqlx::query(
            "SELECT id, system_code, idm, idi, merchant_id, amount, fee FROM transactions
              WHERE kind = 'payment' AND idempotency_key = $1",
        )
        .bind(idempotency_key)
        .fetch_one(&mut *tx)
        .await?;
        let existing_id: Uuid = row.try_get("id")?;
        let existing_sc: i32 = row.try_get("system_code")?;
        let existing_idm: Vec<u8> = row.try_get("idm")?;
        let existing_idi: Vec<u8> = row.try_get("idi")?;
        let existing_merchant: Option<Uuid> = row.try_get("merchant_id")?;
        let existing_amount: i64 = row.try_get("amount")?;
        let existing_fee: i64 = row.try_get("fee")?;
        if existing_sc != sc
            || existing_idm.as_slice() != idm
            || existing_idi.as_slice() != idi
            || existing_merchant != Some(merchant_id)
            || existing_amount != amount.as_i64()
        {
            return Err(DbError::IdempotencyConflict);
        }
        let drows = sqlx::query(
            "SELECT bucket_id, amount FROM ledger_entries
              WHERE transaction_id = $1 AND kind = 'payment' ORDER BY seq",
        )
        .bind(existing_id)
        .fetch_all(&mut *tx)
        .await?;
        let deductions = drows
            .into_iter()
            .map(|r| {
                let bucket_id: Uuid = r.try_get("bucket_id")?;
                let signed: i64 = r.try_get("amount")?;
                Ok(Deduction {
                    bucket_id,
                    amount: Yen::new(-signed),
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        let bal = balance(&mut *tx, account, now).await?;
        tx.commit().await?;
        return Ok(Payment {
            transaction_id: existing_id,
            amount: amount.get(),
            fee: Yen::new(existing_fee),
            deductions,
            balance: bal.total,
            replayed: true,
        });
    }

    // Lock the candidate buckets in the exact consumption order.
    let rows = sqlx::query(
        "SELECT id, remaining_amount, topped_up_at, expires_at
           FROM topup_buckets
          WHERE system_code = $1 AND idm = $2 AND idi = $3 AND status = 'active'
            AND expires_at > $4 AND remaining_amount > 0
          ORDER BY expires_at, topped_up_at, id
          FOR UPDATE",
    )
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(to_odt(now))
    .fetch_all(&mut *tx)
    .await?;

    let locked: Vec<SpendableBucket> = rows
        .into_iter()
        .map(|r| {
            Ok(SpendableBucket {
                id: r.try_get("id")?,
                remaining: Yen::new(r.try_get::<i64, _>("remaining_amount")?),
                topped_up_at: to_jiff(r.try_get("topped_up_at")?),
                expires_at: to_jiff(r.try_get("expires_at")?),
            })
        })
        .collect::<Result<Vec<_>, DbError>>()?;

    let deductions =
        plan_consumption(&locked, amount, now).map_err(|e| DbError::InsufficientFunds {
            available: e.available,
            requested: e.requested,
        })?;

    for d in &deductions {
        sqlx::query(
            "UPDATE topup_buckets
                SET remaining_amount = remaining_amount - $1,
                    status = CASE WHEN remaining_amount - $1 = 0 THEN 'exhausted' ELSE status END
              WHERE id = $2",
        )
        .bind(d.amount.as_i64())
        .bind(d.bucket_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO ledger_entries (id, system_code, idm, idi, transaction_id, bucket_id, kind, amount, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, 'payment', $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(sc)
        .bind(idm)
        .bind(idi)
        .bind(txn_id)
        .bind(d.bucket_id)
        .bind(-d.amount.as_i64())
        .bind(to_odt(now))
        .execute(&mut *tx)
        .await?;
    }

    let bal = balance(&mut *tx, account, now).await?;
    tx.commit().await?;
    Ok(Payment {
        transaction_id: txn_id,
        amount: amount.get(),
        fee,
        deductions,
        balance: bal.total,
        replayed: false,
    })
}

/// Outcome of a refund or void.
#[derive(Debug, Clone)]
pub struct Refund {
    pub transaction_id: Uuid,
    pub payment_txn_id: Uuid,
    pub amount: Yen,
    /// Value restored, per original bucket.
    pub restorations: Vec<Deduction>,
    pub balance: Yen,
    pub replayed: bool,
}

/// Refund up to `amount` (or the full refundable remainder when `None`) of a
/// payment, restoring value to the **original buckets with their original
/// expiry** — never extending validity. Idempotent on `idempotency_key`.
pub async fn refund(
    pool: &Pool,
    payment_txn_id: Uuid,
    amount: Option<PositiveYen>,
    idempotency_key: &str,
    now: Timestamp,
) -> Result<Refund, DbError> {
    restore(pool, payment_txn_id, amount, idempotency_key, now, "refund").await
}

/// Fully reverse a payment (same-day void). Like a full refund but recorded as a
/// technical `reversal`. Idempotent on `idempotency_key`.
pub async fn void(
    pool: &Pool,
    payment_txn_id: Uuid,
    idempotency_key: &str,
    now: Timestamp,
) -> Result<Refund, DbError> {
    restore(pool, payment_txn_id, None, idempotency_key, now, "reversal").await
}

/// Shared refund/void machinery. `kind` is the transaction *and* ledger kind
/// (`refund` or `reversal`), both of which take positive restoring postings.
async fn restore(
    pool: &Pool,
    payment_txn_id: Uuid,
    amount: Option<PositiveYen>,
    idempotency_key: &str,
    now: Timestamp,
    kind: &str,
) -> Result<Refund, DbError> {
    let mut tx = pool.begin().await?;

    // Original payment.
    let prow = sqlx::query(
        "SELECT system_code, idm, idi, merchant_id, store_id, amount, kind FROM transactions WHERE id = $1",
    )
    .bind(payment_txn_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::PaymentNotFound)?;
    if prow.try_get::<String, _>("kind")? != "payment" {
        return Err(DbError::PaymentNotFound);
    }
    let account = account_from_row(&prow).map_err(|_| DbError::PaymentNotFound)?;
    let sc = account.system_code as i32;
    let idm = account.idm.as_bytes().as_slice();
    let idi = account.idi.as_bytes().as_slice();
    let merchant_id: Option<Uuid> = prow.try_get("merchant_id")?;
    // A refund/void inherits the store of the payment it reverses.
    let store_id: Option<Uuid> = prow.try_get("store_id")?;
    let payment_amount: i64 = prow.try_get("amount")?;

    // Idempotency pre-check — must run before the refundable computation, so a
    // replay still succeeds once earlier refunds have reduced what remains
    // refundable.
    let expected = amount.map(|a| a.as_i64());
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM transactions WHERE kind = $1 AND idempotency_key = $2)",
    )
    .bind(kind)
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await?;
    if exists {
        let replay = refund_replay(
            &mut tx,
            kind,
            idempotency_key,
            payment_txn_id,
            account,
            expected,
            now,
        )
        .await?;
        tx.commit().await?;
        return Ok(replay);
    }

    // New refund: validate against what is still refundable.
    let already: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0)::bigint FROM transactions
          WHERE related_txn_id = $1 AND kind IN ('refund', 'reversal')",
    )
    .bind(payment_txn_id)
    .fetch_one(&mut *tx)
    .await?;
    let refundable = payment_amount - already;
    let refund_amount = amount.map(|a| a.as_i64()).unwrap_or(refundable);
    if refund_amount <= 0 || refund_amount > refundable {
        return Err(DbError::RefundExceedsPayment {
            requested: Yen::new(refund_amount),
            refundable: Yen::new(refundable),
        });
    }

    let txn_id = Uuid::now_v7();
    let inserted: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO transactions (id, system_code, idm, idi, kind, merchant_id, store_id, amount, idempotency_key, related_txn_id, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (kind, idempotency_key) DO NOTHING
         RETURNING id",
    )
    .bind(txn_id)
    .bind(sc)
    .bind(idm)
    .bind(idi)
    .bind(kind)
    .bind(merchant_id)
    .bind(store_id)
    .bind(refund_amount)
    .bind(idempotency_key)
    .bind(payment_txn_id)
    .bind(to_odt(now))
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        // Lost a race with a concurrent identical refund — replay it.
        let replay = refund_replay(
            &mut tx,
            kind,
            idempotency_key,
            payment_txn_id,
            account,
            expected,
            now,
        )
        .await?;
        tx.commit().await?;
        return Ok(replay);
    }

    // Original per-bucket debits (in consumption order).
    let debits = sqlx::query(
        "SELECT bucket_id, amount FROM ledger_entries
          WHERE transaction_id = $1 AND kind = 'payment' ORDER BY seq",
    )
    .bind(payment_txn_id)
    .fetch_all(&mut *tx)
    .await?;

    // Amount already restored to each bucket by prior refunds/reversals.
    let mut already_by_bucket: HashMap<Uuid, i64> = HashMap::new();
    let ref_rows = sqlx::query(
        "SELECT le.bucket_id, COALESCE(SUM(le.amount), 0)::bigint AS refunded
           FROM ledger_entries le
           JOIN transactions t ON t.id = le.transaction_id
          WHERE t.related_txn_id = $1 AND le.kind IN ('refund', 'reversal')
          GROUP BY le.bucket_id",
    )
    .bind(payment_txn_id)
    .fetch_all(&mut *tx)
    .await?;
    for r in ref_rows {
        if let Some(b) = r.try_get::<Option<Uuid>, _>("bucket_id")? {
            already_by_bucket.insert(b, r.try_get("refunded")?);
        }
    }

    // Restore in reverse consumption order, capped per bucket at what it still
    // owes. The original expiry is preserved (we never touch expires_at).
    let mut remaining_to_refund = refund_amount;
    let mut restorations = Vec::new();
    for r in debits.into_iter().rev() {
        if remaining_to_refund == 0 {
            break;
        }
        let bucket_id: Uuid = r.try_get("bucket_id")?;
        let debited = -r.try_get::<i64, _>("amount")?;
        let bucket_refundable = debited - already_by_bucket.get(&bucket_id).copied().unwrap_or(0);
        let d = remaining_to_refund.min(bucket_refundable);
        if d > 0 {
            sqlx::query("UPDATE topup_buckets SET remaining_amount = remaining_amount + $1, status = 'active' WHERE id = $2")
                .bind(d)
                .bind(bucket_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO ledger_entries (id, system_code, idm, idi, transaction_id, bucket_id, kind, amount, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(Uuid::now_v7())
            .bind(sc)
            .bind(idm)
            .bind(idi)
            .bind(txn_id)
            .bind(bucket_id)
            .bind(kind)
            .bind(d)
            .bind(to_odt(now))
            .execute(&mut *tx)
            .await?;
            restorations.push(Deduction {
                bucket_id,
                amount: Yen::new(d),
            });
            remaining_to_refund -= d;
        }
    }

    let bal = balance(&mut *tx, account, now).await?;
    tx.commit().await?;
    Ok(Refund {
        transaction_id: txn_id,
        payment_txn_id,
        amount: Yen::new(refund_amount),
        restorations,
        balance: bal.total,
        replayed: false,
    })
}

/// Reconstruct the replay result for an already-recorded refund/void.
async fn refund_replay(
    tx: &mut sqlx::PgConnection,
    kind: &str,
    idempotency_key: &str,
    payment_txn_id: Uuid,
    account: AccountKey,
    expected_amount: Option<i64>,
    now: Timestamp,
) -> Result<Refund, DbError> {
    let row = sqlx::query(
        "SELECT id, amount, related_txn_id FROM transactions WHERE kind = $1 AND idempotency_key = $2",
    )
    .bind(kind)
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await?;
    let existing_id: Uuid = row.try_get("id")?;
    let existing_amount: i64 = row.try_get("amount")?;
    if row.try_get::<Option<Uuid>, _>("related_txn_id")? != Some(payment_txn_id) {
        return Err(DbError::IdempotencyConflict);
    }
    if let Some(e) = expected_amount
        && e != existing_amount
    {
        return Err(DbError::IdempotencyConflict);
    }
    let restorations = load_postings(&mut *tx, existing_id, kind).await?;
    let bal = balance(&mut *tx, account, now).await?;
    Ok(Refund {
        transaction_id: existing_id,
        payment_txn_id,
        amount: Yen::new(existing_amount),
        restorations,
        balance: bal.total,
        replayed: true,
    })
}

async fn load_postings(
    tx: &mut sqlx::PgConnection,
    transaction_id: Uuid,
    kind: &str,
) -> Result<Vec<Deduction>, DbError> {
    let rows = sqlx::query(
        "SELECT bucket_id, amount FROM ledger_entries
          WHERE transaction_id = $1 AND kind = $2 ORDER BY seq",
    )
    .bind(transaction_id)
    .bind(kind)
    .fetch_all(tx)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Deduction {
                bucket_id: r.try_get("bucket_id")?,
                amount: Yen::new(r.try_get::<i64, _>("amount")?),
            })
        })
        .collect()
}

/// A transaction as returned by history queries.
#[derive(Debug, Clone)]
pub struct TransactionRow {
    pub id: Uuid,
    pub account: AccountKey,
    pub kind: String,
    pub merchant_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub store_name: Option<String>,
    pub amount: Yen,
    /// Processing fee (payments only; 0 otherwise).
    pub fee: Yen,
    /// Optional free-text note the merchant attached to a payment.
    pub note: Option<String>,
    pub related_txn_id: Option<Uuid>,
    pub occurred_at: Timestamp,
}

/// Filters for [`list_transactions`]. All fields are optional; `limit` is
/// clamped to `1..=500` (default 50). `before` is a keyset cursor on
/// `occurred_at` for pagination (newest first). When `account` is set its
/// system code, IDm and IDi must all match.
#[derive(Debug, Clone, Default)]
pub struct TxnFilter {
    pub account: Option<AccountKey>,
    pub merchant_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub kind: Option<String>,
    pub before: Option<Timestamp>,
    pub limit: i64,
}

/// List transactions matching `filter`, newest first.
pub async fn list_transactions(
    pool: &Pool,
    filter: &TxnFilter,
) -> Result<Vec<TransactionRow>, DbError> {
    let limit = if filter.limit <= 0 {
        50
    } else {
        filter.limit.min(500)
    };
    let rows = sqlx::query(
        "SELECT t.id, t.system_code, t.idm, t.idi, t.kind, t.merchant_id, t.store_id,
                s.name AS store_name, t.amount, t.fee, t.note, t.related_txn_id, t.occurred_at
           FROM transactions t
           LEFT JOIN stores s ON s.id = t.store_id
          WHERE ($1::integer IS NULL OR t.system_code = $1)
            AND ($2::bytea IS NULL OR t.idm = $2)
            AND ($3::bytea IS NULL OR t.idi = $3)
            AND ($4::uuid IS NULL OR t.merchant_id = $4)
            AND ($5::text IS NULL OR t.kind = $5)
            AND ($6::timestamptz IS NULL OR t.occurred_at < $6)
            AND ($8::uuid IS NULL OR t.store_id = $8)
          ORDER BY t.occurred_at DESC, t.id DESC
          LIMIT $7",
    )
    .bind(filter.account.map(|a| a.system_code as i32))
    .bind(filter.account.map(|a| a.idm.to_bytes().to_vec()))
    .bind(filter.account.map(|a| a.idi.to_bytes().to_vec()))
    .bind(filter.merchant_id)
    .bind(filter.kind.as_deref())
    .bind(filter.before.map(to_odt))
    .bind(limit)
    .bind(filter.store_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(TransactionRow {
                id: r.try_get("id")?,
                account: account_from_row(&r)?,
                kind: r.try_get("kind")?,
                merchant_id: r.try_get("merchant_id")?,
                store_id: r.try_get("store_id")?,
                store_name: r.try_get("store_name")?,
                amount: Yen::new(r.try_get::<i64, _>("amount")?),
                fee: Yen::new(r.try_get::<i64, _>("fee")?),
                note: r.try_get("note")?,
                related_txn_id: r.try_get("related_txn_id")?,
                occurred_at: to_jiff(r.try_get("occurred_at")?),
            })
        })
        .collect()
}

/// A payment with a positive refundable remainder.
#[derive(Debug, Clone)]
pub struct RefundablePayment {
    pub id: Uuid,
    pub account: AccountKey,
    pub merchant_id: Option<Uuid>,
    /// Original payment amount (customer paid this in full).
    pub amount: Yen,
    /// Processing fee (non-refundable).
    pub fee: Yen,
    /// Already refunded/reversed against this payment.
    pub refunded: Yen,
    /// Still refundable: `amount − refunded` (always > 0 here).
    pub refundable: Yen,
    pub occurred_at: Timestamp,
}

/// List payments that still have a positive refundable remainder, newest first.
/// Optionally scoped to a merchant and/or an account. `limit` is clamped to
/// `1..=200` (default 50).
pub async fn list_refundable_payments(
    pool: &Pool,
    merchant_id: Option<Uuid>,
    account: Option<AccountKey>,
    limit: i64,
) -> Result<Vec<RefundablePayment>, DbError> {
    let limit = if limit <= 0 { 50 } else { limit.min(200) };
    let rows = sqlx::query(
        "SELECT p.id, p.system_code, p.idm, p.idi, p.merchant_id, p.amount, p.fee, p.occurred_at,
                COALESCE(r.refunded, 0)::bigint AS refunded
           FROM transactions p
           LEFT JOIN (
               SELECT related_txn_id, SUM(amount) AS refunded
                 FROM transactions
                WHERE kind IN ('refund', 'reversal')
                GROUP BY related_txn_id
           ) r ON r.related_txn_id = p.id
          WHERE p.kind = 'payment'
            AND ($1::uuid IS NULL OR p.merchant_id = $1)
            AND ($2::integer IS NULL OR p.system_code = $2)
            AND ($3::bytea IS NULL OR p.idm = $3)
            AND ($4::bytea IS NULL OR p.idi = $4)
            AND p.amount - COALESCE(r.refunded, 0) > 0
          ORDER BY p.occurred_at DESC, p.id DESC
          LIMIT $5",
    )
    .bind(merchant_id)
    .bind(account.map(|a| a.system_code as i32))
    .bind(account.map(|a| a.idm.to_bytes().to_vec()))
    .bind(account.map(|a| a.idi.to_bytes().to_vec()))
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            let amount: i64 = r.try_get("amount")?;
            let refunded: i64 = r.try_get("refunded")?;
            Ok(RefundablePayment {
                id: r.try_get("id")?,
                account: account_from_row(&r)?,
                merchant_id: r.try_get("merchant_id")?,
                amount: Yen::new(amount),
                fee: Yen::new(r.try_get::<i64, _>("fee")?),
                refunded: Yen::new(refunded),
                refundable: Yen::new(amount - refunded),
                occurred_at: to_jiff(r.try_get("occurred_at")?),
            })
        })
        .collect()
}

/// Outcome of an expiry sweep.
#[derive(Debug, Clone)]
pub struct SweepOutcome {
    /// `false` if another instance held the advisory lock and this call did nothing.
    pub ran: bool,
    pub expired_buckets: i64,
    pub expired_amount: Yen,
}

/// Advisory-lock key so only one sweeper runs at a time across instances.
const SWEEP_ADVISORY_KEY: i64 = 0x6d656c6f6e5f7377; // "melon_sw"

/// Forfeit every bucket whose expiry has passed as of `now`: post an immutable
/// `expiry` ledger entry for its residual and zero it out. Idempotent, batched,
/// and single-runner (Postgres advisory lock) so it is replica-safe. Because
/// spending already filters expired buckets lazily, this only realizes the
/// forfeiture in the books.
pub async fn expire_due(
    pool: &Pool,
    now: Timestamp,
    batch_size: i64,
) -> Result<SweepOutcome, DbError> {
    let batch = if batch_size <= 0 { 500 } else { batch_size };
    let mut lock_conn = pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(SWEEP_ADVISORY_KEY)
        .fetch_one(&mut *lock_conn)
        .await?;
    if !locked {
        return Ok(SweepOutcome {
            ran: false,
            expired_buckets: 0,
            expired_amount: Yen::ZERO,
        });
    }

    let mut expired_buckets = 0i64;
    let mut expired_amount = 0i64;
    let result: Result<(), DbError> = async {
        loop {
            let mut tx = pool.begin().await?;
            let rows = sqlx::query(
                "SELECT id, system_code, idm, idi, remaining_amount FROM topup_buckets
                  WHERE status = 'active' AND expires_at <= $1 AND remaining_amount > 0
                  ORDER BY expires_at
                  FOR UPDATE SKIP LOCKED
                  LIMIT $2",
            )
            .bind(to_odt(now))
            .bind(batch)
            .fetch_all(&mut *tx)
            .await?;
            if rows.is_empty() {
                break;
            }
            for r in rows {
                let bucket_id: Uuid = r.try_get("id")?;
                let bucket_sc: i32 = r.try_get("system_code")?;
                let idm_bytes: Vec<u8> = r.try_get("idm")?;
                let idi_bytes: Vec<u8> = r.try_get("idi")?;
                let remaining: i64 = r.try_get("remaining_amount")?;
                sqlx::query(
                    "INSERT INTO ledger_entries (id, system_code, idm, idi, bucket_id, kind, amount, created_at)
                     VALUES ($1, $2, $3, $4, $5, 'expiry', $6, $7)",
                )
                .bind(Uuid::now_v7())
                .bind(bucket_sc)
                .bind(&idm_bytes)
                .bind(&idi_bytes)
                .bind(bucket_id)
                .bind(-remaining)
                .bind(to_odt(now))
                .execute(&mut *tx)
                .await?;
                sqlx::query("UPDATE topup_buckets SET remaining_amount = 0, status = 'expired' WHERE id = $1")
                    .bind(bucket_id)
                    .execute(&mut *tx)
                    .await?;
                expired_buckets += 1;
                expired_amount += remaining;
            }
            tx.commit().await?;
        }
        Ok(())
    }
    .await;

    // Always release the advisory lock, even on error.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(SWEEP_ADVISORY_KEY)
        .execute(&mut *lock_conn)
        .await;
    result?;

    Ok(SweepOutcome {
        ran: true,
        expired_buckets,
        expired_amount: Yen::new(expired_amount),
    })
}

/// A slice of the outstanding balance grouped by the JST month of expiry.
#[derive(Debug, Clone)]
pub struct ExpiryMonthSummary {
    pub month: String,
    pub amount: Yen,
}

/// The total unused, still-valid balance as of `as_of` — the 未使用残高 figure
/// used for 資金決済法 base-date reporting — with a per-expiry-month breakdown.
#[derive(Debug, Clone)]
pub struct OutstandingReport {
    pub as_of: Timestamp,
    pub total: Yen,
    pub account_count: i64,
    pub by_expiry_month: Vec<ExpiryMonthSummary>,
}

// ----- issuer (発行者) revenue account -----

/// The issuer's revenue balance and its composition. This is an *accounting*
/// figure (the cash itself is held by merchants who collected the top-ups); it
/// is what the issuer has earned, not cash on hand.
#[derive(Debug, Clone)]
pub struct IssuerBalance {
    /// Payment fees collected from merchants, cumulative. Fees are non-refundable,
    /// so this counts every payment's fee even if the payment was later refunded.
    pub fee_income: Yen,
    /// Forfeited (expired) prepaid balances — breakage income, cumulative.
    pub expiry_income: Yen,
    /// Net of manual issuer entries: withdrawals (−) and corrections/injections (+).
    pub adjustments: Yen,
    /// `fee_income + expiry_income + adjustments`.
    pub balance: Yen,
}

/// Compute the issuer's revenue balance from the books: payment fee income +
/// breakage (expired balances) + manual issuer adjustments. Fee income and
/// breakage are derived from existing data (no separate ledger).
pub async fn issuer_balance<'e, E>(exec: E) -> Result<IssuerBalance, DbError>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query(
        "SELECT
            (SELECT COALESCE(SUM(fee), 0) FROM transactions WHERE kind = 'payment')::bigint AS fee_income,
            (SELECT COALESCE(SUM(-amount), 0) FROM ledger_entries WHERE kind = 'expiry')::bigint AS expiry_income,
            (SELECT COALESCE(SUM(amount), 0) FROM issuer_adjustments)::bigint AS adjustments",
    )
    .fetch_one(exec)
    .await?;
    let fee_income = Yen::new(row.try_get::<i64, _>("fee_income")?);
    let expiry_income = Yen::new(row.try_get::<i64, _>("expiry_income")?);
    let adjustments = Yen::new(row.try_get::<i64, _>("adjustments")?);
    let balance = fee_income
        .checked_add(expiry_income)
        .and_then(|s| s.checked_add(adjustments))?;
    Ok(IssuerBalance {
        fee_income,
        expiry_income,
        adjustments,
        balance,
    })
}

/// Outcome of an issuer adjustment.
#[derive(Debug, Clone)]
pub struct IssuerAdjustment {
    pub id: Uuid,
    pub delta: Yen,
    pub balance: Yen,
}

/// Record a manual issuer entry (non-zero signed `delta`): a withdrawal (−, profit
/// taken out) or a correction / capital injection (+), with an optional note.
/// Append-only; returns the new issuer balance. There is no floor — a withdrawal
/// may legitimately exceed accrued revenue (an advance against future income).
pub async fn adjust_issuer(
    pool: &Pool,
    delta: Yen,
    note: Option<&str>,
) -> Result<IssuerAdjustment, DbError> {
    if delta.is_zero() {
        return Err(DbError::Money(melon_core::money::MoneyError::NonPositive));
    }
    let mut tx = pool.begin().await?;
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO issuer_adjustments (id, amount, note) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(delta.as_i64())
        .bind(note)
        .execute(&mut *tx)
        .await?;
    let bal = issuer_balance(&mut *tx).await?;
    tx.commit().await?;
    Ok(IssuerAdjustment {
        id,
        delta,
        balance: bal.balance,
    })
}

/// One manual issuer entry (withdrawal or correction) in the history.
#[derive(Debug, Clone)]
pub struct IssuerAdjustmentRow {
    pub id: Uuid,
    /// Signed: `+` credit/injection, `−` withdrawal.
    pub amount: Yen,
    pub note: Option<String>,
    pub created_at: Timestamp,
}

/// List issuer adjustments, newest first (`limit` clamped to `1..=500`, default 50).
pub async fn list_issuer_adjustments(
    pool: &Pool,
    limit: i64,
) -> Result<Vec<IssuerAdjustmentRow>, DbError> {
    let limit = if limit <= 0 { 50 } else { limit.min(500) };
    let rows = sqlx::query(
        "SELECT id, amount, note, created_at FROM issuer_adjustments
          ORDER BY created_at DESC, id DESC
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(IssuerAdjustmentRow {
                id: r.try_get("id")?,
                amount: Yen::new(r.try_get::<i64, _>("amount")?),
                note: r.try_get("note")?,
                created_at: to_jiff(r.try_get("created_at")?),
            })
        })
        .collect()
}

/// Compute the outstanding-balance report as of `as_of`.
pub async fn outstanding_balance(
    pool: &Pool,
    as_of: Timestamp,
) -> Result<OutstandingReport, DbError> {
    let head = sqlx::query(
        "SELECT COALESCE(SUM(remaining_amount), 0)::bigint AS total,
                COUNT(DISTINCT (system_code, idm, idi)) AS accounts
           FROM topup_buckets
          WHERE expires_at > $1 AND remaining_amount > 0",
    )
    .bind(to_odt(as_of))
    .fetch_one(pool)
    .await?;

    let by_month = sqlx::query(
        "SELECT to_char(expires_at AT TIME ZONE 'Asia/Tokyo', 'YYYY-MM') AS month,
                SUM(remaining_amount)::bigint AS amount
           FROM topup_buckets
          WHERE expires_at > $1 AND remaining_amount > 0
          GROUP BY month
          ORDER BY month",
    )
    .bind(to_odt(as_of))
    .fetch_all(pool)
    .await?;

    Ok(OutstandingReport {
        as_of,
        total: Yen::new(head.try_get::<i64, _>("total")?),
        account_count: head.try_get("accounts")?,
        by_expiry_month: by_month
            .into_iter()
            .map(|r| {
                Ok(ExpiryMonthSummary {
                    month: r.try_get("month")?,
                    amount: Yen::new(r.try_get::<i64, _>("amount")?),
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?,
    })
}
