//! Integration tests for the money engine against a real PostgreSQL.
//!
//! `#[sqlx::test]` provisions a fresh database per test and applies the
//! `migrations/`. Requires `DATABASE_URL` to point at a Postgres the test user
//! can create databases on (the dedicated `melon-postgres` container).

use jiff::{Timestamp, tz::TimeZone};
use melon_core::account::AccountKey;
use melon_core::idi::Idi;
use melon_core::idm::Idm;
use melon_core::money::{PositiveYen, Yen};
use melon_db::ops;
use sqlx::PgPool;

/// A fixed test IDm (cards in this deployment have a stable IDm).
const IDM: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

/// A test account under system code 0x0003 with an IDi of repeated byte `n`.
fn acct(n: u8) -> AccountKey {
    AccountKey::new(0x0003, Idm::from_bytes(IDM), Idi::from_bytes([n; 8]))
}

fn ts(s: &str) -> Timestamp {
    s.parse().expect("valid timestamp")
}

fn yen(v: i64) -> PositiveYen {
    PositiveYen::from_i64(v).expect("positive")
}

fn jst() -> TimeZone {
    melon_core::expiry::expiry_timezone().unwrap()
}

#[sqlx::test]
async fn top_up_then_balance(pool: PgPool) {
    let a = acct(1);
    let t0 = ts("2026-01-15T09:00:00+09:00");
    let out = ops::top_up(&pool, a, None, None, yen(1000), "topup-1", t0, &jst())
        .await
        .unwrap();
    assert_eq!(out.amount, Yen::new(1000));
    assert!(!out.replayed);
    // expires 6 months later on the JST wall clock.
    assert_eq!(out.expires_at, ts("2026-07-15T09:00:00+09:00"));

    let bal = ops::balance(&pool, a, t0).await.unwrap();
    assert_eq!(bal.total, Yen::new(1000));
    assert_eq!(bal.buckets.len(), 1);
}

#[sqlx::test]
async fn same_idi_different_system_codes_are_separate_accounts(pool: PgPool) {
    // Identical IDi bytes under two different FeliCa system codes.
    let idi = Idi::from_bytes([0x42; 8]);
    let idm = Idm::from_bytes(IDM);
    let a3 = AccountKey::new(0x0003, idm, idi);
    let afe = AccountKey::new(0xFE00, idm, idi);
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a3, None, None, yen(1000), "k3", t0, &jst())
        .await
        .unwrap();
    ops::top_up(&pool, afe, None, None, yen(500), "kfe", t0, &jst())
        .await
        .unwrap();

    // They are independent accounts with independent balances.
    assert_eq!(
        ops::balance(&pool, a3, t0).await.unwrap().total,
        Yen::new(1000)
    );
    assert_eq!(
        ops::balance(&pool, afe, t0).await.unwrap().total,
        Yen::new(500)
    );
    assert_eq!(ops::list_accounts(&pool, t0, 100).await.unwrap().len(), 2);
}

#[sqlx::test]
async fn merchant_topup_reduces_settlement_and_shows_in_history(pool: PgPool) {
    let a = acct(30);
    let m = ops::create_merchant(&pool, "m-settle", "Settle Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");

    // The merchant sells a ¥1000 top-up, then the customer pays ¥300 there.
    ops::top_up(&pool, a, Some(m), None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    ops::pay(&pool, a, m, None, yen(300), "p", None, t0)
        .await
        .unwrap();

    // Settlement = payments(300) − top-ups(1000) = −700 (merchant owes the issuer).
    let merchants = ops::list_merchants(&pool).await.unwrap();
    let row = merchants.iter().find(|x| x.id == m).unwrap();
    assert_eq!(row.collected, Yen::new(-700));

    // Both the top-up and the payment appear in the merchant's history.
    let txns = ops::list_transactions(
        &pool,
        &ops::TxnFilter {
            merchant_id: Some(m),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let kinds: Vec<&str> = txns.iter().map(|t| t.kind.as_str()).collect();
    assert!(kinds.contains(&"top_up"));
    assert!(kinds.contains(&"payment"));
}

#[sqlx::test]
async fn payment_fee_reduces_settlement(pool: PgPool) {
    let a = acct(31);
    // Merchant with a 3% (300 bps) fee.
    let m = ops::create_merchant(&pool, "m-fee", "Fee Test", 300, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();

    let pay = ops::pay(&pool, a, m, None, yen(1000), "p", None, t0)
        .await
        .unwrap();
    // Customer paid the full ¥1000; the merchant is charged a ¥30 fee.
    assert_eq!(pay.fee, Yen::new(30));
    assert_eq!(pay.balance, Yen::new(0));

    // Settlement to the merchant is net of the fee: 1000 − 30 = 970.
    let merchants = ops::list_merchants(&pool).await.unwrap();
    let row = merchants.iter().find(|x| x.id == m).unwrap();
    assert_eq!(row.fee_bps, 300);
    assert_eq!(row.collected, Yen::new(970));
}

#[sqlx::test]
async fn topup_respects_merchant_credit_limit(pool: PgPool) {
    let a = acct(32);
    // Merchant with a ¥1000 credit limit, no fee.
    let m = ops::create_merchant(&pool, "m-credit", "Credit Test", 0, 1000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");

    // ¥1000 top-up pushes settlement to exactly −1000 (at the limit) → OK.
    ops::top_up(&pool, a, Some(m), None, yen(1000), "t1", t0, &jst())
        .await
        .unwrap();
    // A further ¥1 would push it to −1001, past the limit → rejected.
    let err = ops::top_up(&pool, a, Some(m), None, yen(1), "t2", t0, &jst())
        .await
        .unwrap_err();
    assert!(matches!(err, melon_db::DbError::CreditLimitExceeded { .. }));

    // Revolving limit: a payment raises settlement to −500, restoring top-up
    // headroom, so a ¥500 top-up fits again.
    ops::pay(&pool, a, m, None, yen(500), "p", None, t0)
        .await
        .unwrap();
    ops::top_up(&pool, a, Some(m), None, yen(500), "t3", t0, &jst())
        .await
        .unwrap();

    // Issuer/system top-ups (no merchant) are never limited.
    ops::top_up(&pool, a, None, None, yen(999_999), "sys", t0, &jst())
        .await
        .unwrap();
}

#[sqlx::test]
async fn payment_draws_soonest_expiry_first(pool: PgPool) {
    let a = acct(2);
    let m = ops::create_merchant(&pool, "m-soon", "Soonest Test", 0, 10_000_000)
        .await
        .unwrap();

    // Bucket A tops up earlier (expires sooner); bucket B later.
    let early = ts("2026-01-15T09:00:00+09:00"); // expires 2026-07-15
    let late = ts("2026-02-15T09:00:00+09:00"); // expires 2026-08-15
    let a_bucket = ops::top_up(&pool, a, None, None, yen(300), "t-a", early, &jst())
        .await
        .unwrap();
    let b_bucket = ops::top_up(&pool, a, None, None, yen(1000), "t-b", late, &jst())
        .await
        .unwrap();

    let now = ts("2026-03-01T00:00:00+09:00"); // both active
    let pay = ops::pay(&pool, a, m, None, yen(500), "pay-1", None, now)
        .await
        .unwrap();

    // 300 drawn from the sooner-expiring bucket A, then 200 from B.
    assert_eq!(pay.deductions.len(), 2);
    assert_eq!(pay.deductions[0].bucket_id, a_bucket.bucket_id);
    assert_eq!(pay.deductions[0].amount, Yen::new(300));
    assert_eq!(pay.deductions[1].bucket_id, b_bucket.bucket_id);
    assert_eq!(pay.deductions[1].amount, Yen::new(200));
    assert_eq!(pay.balance, Yen::new(800));
}

#[sqlx::test]
async fn payment_rejects_insufficient_funds(pool: PgPool) {
    let a = acct(3);
    let m = ops::create_merchant(&pool, "m-insuf", "Insuf Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(400), "t", t0, &jst())
        .await
        .unwrap();

    let err = ops::pay(&pool, a, m, None, yen(500), "pay", None, t0)
        .await
        .unwrap_err();
    match err {
        melon_db::DbError::InsufficientFunds {
            available,
            requested,
        } => {
            assert_eq!(available, Yen::new(400));
            assert_eq!(requested, Yen::new(500));
        }
        other => panic!("expected InsufficientFunds, got {other:?}"),
    }
    // Balance untouched.
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(400)
    );
}

#[sqlx::test]
async fn top_up_is_idempotent(pool: PgPool) {
    let a = acct(4);
    let t0 = ts("2026-01-15T09:00:00+09:00");
    let first = ops::top_up(&pool, a, None, None, yen(1000), "same-key", t0, &jst())
        .await
        .unwrap();
    let second = ops::top_up(&pool, a, None, None, yen(1000), "same-key", t0, &jst())
        .await
        .unwrap();

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.bucket_id, second.bucket_id);
    // Not double-credited.
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(1000)
    );
}

#[sqlx::test]
async fn payment_is_idempotent(pool: PgPool) {
    let a = acct(5);
    let m = ops::create_merchant(&pool, "m-idem", "Idem Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();

    let first = ops::pay(&pool, a, m, None, yen(500), "pay-key", None, t0)
        .await
        .unwrap();
    let second = ops::pay(&pool, a, m, None, yen(500), "pay-key", None, t0)
        .await
        .unwrap();

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.transaction_id, second.transaction_id);
    assert_eq!(first.deductions, second.deductions);
    // Charged once, not twice.
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(500)
    );
}

#[sqlx::test]
async fn expired_value_is_not_spendable(pool: PgPool) {
    let a = acct(6);
    let m = ops::create_merchant(&pool, "m-exp", "Expiry Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00"); // expires 2026-07-15
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();

    // Before expiry: full balance.
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(1000)
    );

    // After expiry: nothing spendable (lazy expiry at read/pay time).
    let after = ts("2026-08-01T00:00:00+09:00");
    assert_eq!(
        ops::balance(&pool, a, after).await.unwrap().total,
        Yen::new(0)
    );
    let err = ops::pay(&pool, a, m, None, yen(1), "p", None, after)
        .await
        .unwrap_err();
    assert!(matches!(err, melon_db::DbError::InsufficientFunds { .. }));
}

#[sqlx::test]
async fn refund_restores_to_original_bucket(pool: PgPool) {
    let a = acct(10);
    let m = ops::create_merchant(&pool, "m-refund", "Refund Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    let topup = ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(&pool, a, m, None, yen(600), "p", None, t0)
        .await
        .unwrap();

    let refund = ops::refund(&pool, pay.transaction_id, Some(yen(400)), "r", t0)
        .await
        .unwrap();
    assert_eq!(refund.amount, Yen::new(400));
    assert_eq!(refund.restorations.len(), 1);
    assert_eq!(refund.restorations[0].bucket_id, topup.bucket_id);
    assert_eq!(refund.balance, Yen::new(800));

    // Restored onto the ORIGINAL bucket, keeping its original expiry.
    let bal = ops::balance(&pool, a, t0).await.unwrap();
    assert_eq!(bal.buckets.len(), 1);
    assert_eq!(bal.buckets[0].bucket_id, topup.bucket_id);
    assert_eq!(bal.buckets[0].expires_at, ts("2026-07-15T09:00:00+09:00"));
    assert_eq!(bal.total, Yen::new(800));
}

#[sqlx::test]
async fn refund_does_not_extend_validity(pool: PgPool) {
    let a = acct(11);
    let m = ops::create_merchant(&pool, "m-noext", "NoExt Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00"); // expires 2026-07-15
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(
        &pool,
        a,
        m,
        None,
        yen(600),
        "p",
        None,
        ts("2026-02-01T00:00:00+09:00"),
    )
    .await
    .unwrap();
    let _ = ops::refund(
        &pool,
        pay.transaction_id,
        Some(yen(400)),
        "r",
        ts("2026-03-01T00:00:00+09:00"),
    )
    .await
    .unwrap();

    // The refunded value rides the original bucket's expiry — after it, gone.
    let after = ts("2026-08-01T00:00:00+09:00");
    assert_eq!(
        ops::balance(&pool, a, after).await.unwrap().total,
        Yen::new(0)
    );
}

#[sqlx::test]
async fn over_refund_is_rejected(pool: PgPool) {
    let a = acct(12);
    let m = ops::create_merchant(&pool, "m-over", "Over Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(&pool, a, m, None, yen(500), "p", None, t0)
        .await
        .unwrap();

    ops::refund(&pool, pay.transaction_id, Some(yen(500)), "r1", t0)
        .await
        .unwrap();
    // A second refund has nothing left to refund.
    let err = ops::refund(&pool, pay.transaction_id, Some(yen(1)), "r2", t0)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        melon_db::DbError::RefundExceedsPayment { .. }
    ));
    // Refunding more than the payment in one shot is rejected up front.
    let pay2 = ops::pay(&pool, a, m, None, yen(300), "p2", None, t0)
        .await
        .unwrap();
    let err2 = ops::refund(&pool, pay2.transaction_id, Some(yen(400)), "r3", t0)
        .await
        .unwrap_err();
    assert!(matches!(
        err2,
        melon_db::DbError::RefundExceedsPayment { .. }
    ));
}

#[sqlx::test]
async fn void_reverses_full_payment(pool: PgPool) {
    let a = acct(13);
    let m = ops::create_merchant(&pool, "m-void", "Void Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(&pool, a, m, None, yen(700), "p", None, t0)
        .await
        .unwrap();

    let void = ops::void(&pool, pay.transaction_id, "v", t0).await.unwrap();
    assert_eq!(void.amount, Yen::new(700));
    assert_eq!(void.balance, Yen::new(1000));
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(1000)
    );
}

#[sqlx::test]
async fn refund_is_idempotent(pool: PgPool) {
    let a = acct(14);
    let m = ops::create_merchant(&pool, "m-refidem", "RefIdem Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(&pool, a, m, None, yen(600), "p", None, t0)
        .await
        .unwrap();

    let first = ops::refund(&pool, pay.transaction_id, Some(yen(400)), "same", t0)
        .await
        .unwrap();
    let second = ops::refund(&pool, pay.transaction_id, Some(yen(400)), "same", t0)
        .await
        .unwrap();
    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.transaction_id, second.transaction_id);
    assert_eq!(
        ops::balance(&pool, a, t0).await.unwrap().total,
        Yen::new(800)
    );
}

#[sqlx::test]
async fn sweep_forfeits_expired_and_is_idempotent(pool: PgPool) {
    let a = acct(15);
    let t0 = ts("2026-01-15T09:00:00+09:00"); // expires 2026-07-15
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();

    let after = ts("2026-08-01T00:00:00+09:00");
    let swept = ops::expire_due(&pool, after, 100).await.unwrap();
    assert!(swept.ran);
    assert_eq!(swept.expired_buckets, 1);
    assert_eq!(swept.expired_amount, Yen::new(1000));
    assert_eq!(
        ops::balance(&pool, a, after).await.unwrap().total,
        Yen::new(0)
    );

    // Ledger for the account nets to zero after forfeiture (+1000 topup, -1000 expiry).
    let sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount),0)::bigint FROM ledger_entries WHERE idi = $1",
    )
    .bind(a.idi.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sum, 0);

    // Running again forfeits nothing.
    let again = ops::expire_due(&pool, after, 100).await.unwrap();
    assert_eq!(again.expired_buckets, 0);
    assert_eq!(again.expired_amount, Yen::new(0));
}

#[sqlx::test]
async fn outstanding_balance_report(pool: PgPool) {
    let a = acct(16);
    let b = acct(17);
    // Two accounts, buckets expiring in different months (2026-07 and 2026-08).
    ops::top_up(
        &pool,
        a,
        None,
        None,
        yen(1000),
        "ta",
        ts("2026-01-15T09:00:00+09:00"),
        &jst(),
    )
    .await
    .unwrap();
    ops::top_up(
        &pool,
        b,
        None,
        None,
        yen(500),
        "tb",
        ts("2026-02-10T09:00:00+09:00"),
        &jst(),
    )
    .await
    .unwrap();

    let report = ops::outstanding_balance(&pool, ts("2026-03-01T00:00:00+09:00"))
        .await
        .unwrap();
    assert_eq!(report.total, Yen::new(1500));
    assert_eq!(report.account_count, 2);
    assert_eq!(report.by_expiry_month.len(), 2);
    assert_eq!(report.by_expiry_month[0].month, "2026-07");
    assert_eq!(report.by_expiry_month[0].amount, Yen::new(1000));
    assert_eq!(report.by_expiry_month[1].month, "2026-08");
    assert_eq!(report.by_expiry_month[1].amount, Yen::new(500));
}

#[sqlx::test]
async fn transaction_history_lists_and_filters(pool: PgPool) {
    let a = acct(18);
    let m = ops::create_merchant(&pool, "m-hist", "Hist Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();
    ops::pay(&pool, a, m, None, yen(300), "p", None, t0)
        .await
        .unwrap();

    // All for the account: top-up + payment.
    let by_idi = ops::list_transactions(
        &pool,
        &ops::TxnFilter {
            account: Some(a),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(by_idi.len(), 2);

    // Merchant sees only the payment.
    let by_merchant = ops::list_transactions(
        &pool,
        &ops::TxnFilter {
            merchant_id: Some(m),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(by_merchant.len(), 1);
    assert_eq!(by_merchant[0].kind, "payment");

    // Filter by kind.
    let topups = ops::list_transactions(
        &pool,
        &ops::TxnFilter {
            kind: Some("top_up".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(topups.len(), 1);
    assert_eq!(topups[0].kind, "top_up");
}

#[sqlx::test]
async fn concurrent_payments_never_overspend(pool: PgPool) {
    let a = acct(7);
    let m = ops::create_merchant(&pool, "m-conc", "Concurrency Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(1000), "t", t0, &jst())
        .await
        .unwrap();

    // Five concurrent ¥300 charges (¥1500 demanded) against a ¥1000 balance.
    let mut handles = Vec::new();
    for i in 0..5 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            ops::pay(&pool, a, m, None, yen(300), &format!("c-{i}"), None, t0).await
        }));
    }
    let mut successes = 0;
    for h in handles {
        if h.await.unwrap().is_ok() {
            successes += 1;
        }
    }

    // Exactly floor(1000 / 300) = 3 succeed; the rest hit InsufficientFunds.
    assert_eq!(successes, 3);
    let bal = ops::balance(&pool, a, t0).await.unwrap();
    assert_eq!(bal.total, Yen::new(1000 - 3 * 300));
    assert!(bal.total.as_i64() >= 0);
}

#[sqlx::test]
async fn issuer_balance_composes_fees_breakage_and_adjustments(pool: PgPool) {
    // Empty books: everything is zero.
    let z = ops::issuer_balance(&pool).await.unwrap();
    assert_eq!(z.balance, Yen::new(0));
    assert_eq!(z.fee_income, Yen::new(0));
    assert_eq!(z.expiry_income, Yen::new(0));
    assert_eq!(z.adjustments, Yen::new(0));

    // Fee income: a 3% payment on ¥1000 yields a ¥30 fee to the issuer.
    let payer = acct(60);
    let m = ops::create_merchant(&pool, "m-issuer", "Issuer Test", 300, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00"); // expires 2026-07-15
    ops::top_up(&pool, payer, None, None, yen(1000), "t-pay", t0, &jst())
        .await
        .unwrap();
    ops::pay(&pool, payer, m, None, yen(1000), "p", None, t0)
        .await
        .unwrap();

    // Breakage income: a separate account's ¥500 top-up expires and is swept.
    let breaker = acct(61);
    ops::top_up(&pool, breaker, None, None, yen(500), "t-brk", t0, &jst())
        .await
        .unwrap();
    let after = ts("2026-08-01T00:00:00+09:00");
    let swept = ops::expire_due(&pool, after, 100).await.unwrap();
    assert_eq!(swept.expired_amount, Yen::new(500));

    let b = ops::issuer_balance(&pool).await.unwrap();
    assert_eq!(b.fee_income, Yen::new(30));
    assert_eq!(b.expiry_income, Yen::new(500));
    assert_eq!(b.adjustments, Yen::new(0));
    assert_eq!(b.balance, Yen::new(530)); // 30 + 500 + 0

    // A ¥200 withdrawal reduces the balance; the composition identity holds.
    let w = ops::adjust_issuer(&pool, Yen::new(-200), Some("profit withdrawal"))
        .await
        .unwrap();
    assert_eq!(w.balance, Yen::new(330)); // 530 − 200
    let b2 = ops::issuer_balance(&pool).await.unwrap();
    assert_eq!(b2.adjustments, Yen::new(-200));
    assert_eq!(
        b2.balance,
        Yen::new(b2.fee_income.as_i64() + b2.expiry_income.as_i64() + b2.adjustments.as_i64()),
    );

    // Fees are non-refundable: a full refund of the payment leaves fee income intact.
    let pay_txn = ops::list_transactions(
        &pool,
        &ops::TxnFilter {
            merchant_id: Some(m),
            kind: Some("payment".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()[0]
        .id;
    ops::refund(&pool, pay_txn, None, "r", t0).await.unwrap();
    assert_eq!(
        ops::issuer_balance(&pool).await.unwrap().fee_income,
        Yen::new(30)
    );

    // The adjustment shows up in the history, newest first.
    let hist = ops::list_issuer_adjustments(&pool, 50).await.unwrap();
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].amount, Yen::new(-200));
    assert_eq!(hist[0].note.as_deref(), Some("profit withdrawal"));

    // Zero-delta adjustments are rejected.
    assert!(ops::adjust_issuer(&pool, Yen::new(0), None).await.is_err());
}

#[sqlx::test]
async fn refundable_payments_reflect_prior_refunds(pool: PgPool) {
    let a = acct(70);
    let m = ops::create_merchant(&pool, "m-refundable", "Refundable Test", 0, 10_000_000)
        .await
        .unwrap();
    let t0 = ts("2026-01-15T09:00:00+09:00");
    ops::top_up(&pool, a, None, None, yen(2000), "t", t0, &jst())
        .await
        .unwrap();

    // Two payments; partially refund the first.
    let p1 = ops::pay(&pool, a, m, None, yen(500), "p1", None, t0)
        .await
        .unwrap();
    let p2 = ops::pay(&pool, a, m, None, yen(300), "p2", None, t0)
        .await
        .unwrap();
    ops::refund(&pool, p1.transaction_id, Some(yen(200)), "r1", t0)
        .await
        .unwrap();

    let list = ops::list_refundable_payments(&pool, Some(m), Some(a), 50)
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    let by_id = |id| list.iter().find(|r| r.id == id).unwrap();
    // p1: 500 paid, 200 refunded → 300 refundable.
    assert_eq!(by_id(p1.transaction_id).refunded, Yen::new(200));
    assert_eq!(by_id(p1.transaction_id).refundable, Yen::new(300));
    // p2: untouched → 300 refundable.
    assert_eq!(by_id(p2.transaction_id).refundable, Yen::new(300));

    // Fully refunding p2 drops it from the list.
    ops::refund(&pool, p2.transaction_id, None, "r2", t0)
        .await
        .unwrap();
    let list = ops::list_refundable_payments(&pool, Some(m), Some(a), 50)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, p1.transaction_id);

    // Scoping to a different merchant yields nothing.
    let other = ops::create_merchant(&pool, "m-other", "Other", 0, 0)
        .await
        .unwrap();
    assert!(
        ops::list_refundable_payments(&pool, Some(other), Some(a), 50)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn refund_is_allowed_even_below_credit_limit(pool: PgPool) {
    // Revolving credit model: top-ups are bounded by the *current* settlement, so
    // payments restore head-room. Refunds are a consumer obligation and are never
    // credit-checked, so a refund may legitimately push settlement below the
    // credit limit (an accepted, bounded exposure).
    let m = ops::create_merchant(&pool, "m-rev", "Revolving Test", 0, 1000)
        .await
        .unwrap();
    let a = acct(80);
    let b = acct(81);
    let t0 = ts("2026-01-15T09:00:00+09:00");

    // A tops up ¥500 and spends it at M → settlement 0, restoring top-up headroom.
    ops::top_up(&pool, a, Some(m), None, yen(500), "a-top", t0, &jst())
        .await
        .unwrap();
    let pay = ops::pay(&pool, a, m, None, yen(500), "a-pay", None, t0)
        .await
        .unwrap();
    // The recovered headroom lets M sell another ¥1000 top-up (settlement −1000).
    ops::top_up(&pool, b, Some(m), None, yen(1000), "b-top", t0, &jst())
        .await
        .unwrap();

    // Refunding A's payment is not blocked and pushes settlement below −1000.
    ops::refund(&pool, pay.transaction_id, None, "a-refund", t0)
        .await
        .unwrap();
    let m_row = ops::get_merchant(&pool, m).await.unwrap().unwrap();
    assert_eq!(m_row.collected, Yen::new(-1500));
}
