//! The pure payment-consumption algorithm: **soonest-expiry-first**.
//!
//! Spending draws from the buckets whose value is closest to expiring, so the
//! holder forfeits as little as possible — the consumer-protective, regulator-
//! defensible order. This module is deliberately free of any I/O: the database
//! layer mirrors exactly this plan under a `SELECT … FOR UPDATE` lock, and these
//! functions are what the unit/property tests exercise.

use jiff::Timestamp;
use uuid::Uuid;

use crate::expiry::is_active;
use crate::money::{PositiveYen, Yen};

/// A bucket as seen by the consumption planner.
#[derive(Debug, Clone, Copy)]
pub struct SpendableBucket {
    pub id: Uuid,
    pub remaining: Yen,
    pub topped_up_at: Timestamp,
    pub expires_at: Timestamp,
}

/// A planned debit from a single bucket. `amount` is a positive magnitude; the
/// corresponding ledger posting stores it negated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deduction {
    pub bucket_id: Uuid,
    pub amount: Yen,
}

/// Not enough spendable balance to cover a payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("insufficient funds: available {available}, requested {requested}")]
pub struct InsufficientFunds {
    pub available: Yen,
    pub requested: Yen,
}

/// Order buckets by the consumption rule: soonest expiry first, then oldest
/// top-up, then id — a total, deterministic order (reproducible replays/tests).
fn consumption_order(a: &SpendableBucket, b: &SpendableBucket) -> std::cmp::Ordering {
    a.expires_at
        .cmp(&b.expires_at)
        .then(a.topped_up_at.cmp(&b.topped_up_at))
        .then(a.id.cmp(&b.id))
}

/// Spendable balance across active (non-expired, positive) buckets at `now`.
///
/// Uses saturating addition: a real account's total is far within `i64` yen, so
/// this read-only aggregate never needs to surface an overflow.
pub fn spendable_balance(buckets: &[SpendableBucket], now: Timestamp) -> Yen {
    let total = buckets
        .iter()
        .filter(|b| is_active(b.expires_at, now) && b.remaining.is_positive())
        .map(|b| b.remaining.as_i64())
        .fold(0i64, i64::saturating_add);
    Yen::new(total)
}

/// Plan how to draw `amount` from `buckets` at `now`, soonest-expiry-first.
///
/// Returns the per-bucket debits (each `> 0`, summing to `amount`) or
/// [`InsufficientFunds`] if the active balance cannot cover it. Expired and
/// empty buckets are ignored.
pub fn plan_consumption(
    buckets: &[SpendableBucket],
    amount: PositiveYen,
    now: Timestamp,
) -> Result<Vec<Deduction>, InsufficientFunds> {
    let mut candidates: Vec<&SpendableBucket> = buckets
        .iter()
        .filter(|b| is_active(b.expires_at, now) && b.remaining.is_positive())
        .collect();
    candidates.sort_by(|a, b| consumption_order(a, b));

    let requested = amount.as_i64();
    let available = candidates
        .iter()
        .map(|b| b.remaining.as_i64())
        .fold(0i64, i64::saturating_add);
    if available < requested {
        return Err(InsufficientFunds {
            available: Yen::new(available),
            requested: amount.get(),
        });
    }

    let mut need = requested;
    let mut deductions = Vec::new();
    for b in candidates {
        if need == 0 {
            break;
        }
        let debit = need.min(b.remaining.as_i64());
        if debit > 0 {
            deductions.push(Deduction {
                bucket_id: b.id,
                amount: Yen::new(debit),
            });
            need -= debit;
        }
    }
    debug_assert_eq!(need, 0, "covered balance must fully allocate");
    Ok(deductions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ts(secs: i64) -> Timestamp {
        Timestamp::from_second(secs).expect("in range")
    }

    fn bucket(id: u128, remaining: i64, topped_up: i64, expires: i64) -> SpendableBucket {
        SpendableBucket {
            id: Uuid::from_u128(id),
            remaining: Yen::new(remaining),
            topped_up_at: ts(topped_up),
            expires_at: ts(expires),
        }
    }

    const NOW: i64 = 1_000_000_000;

    #[test]
    fn draws_soonest_expiry_first_and_splits_across_buckets() {
        // Two buckets: one expiring sooner (600) and one later (900).
        let buckets = vec![
            bucket(2, 1000, NOW - 10, NOW + 900),
            bucket(1, 300, NOW - 5, NOW + 600),
        ];
        let plan =
            plan_consumption(&buckets, PositiveYen::from_i64(500).unwrap(), ts(NOW)).unwrap();
        // 300 from the sooner-expiring bucket, then 200 from the later one.
        assert_eq!(
            plan,
            vec![
                Deduction {
                    bucket_id: Uuid::from_u128(1),
                    amount: Yen::new(300)
                },
                Deduction {
                    bucket_id: Uuid::from_u128(2),
                    amount: Yen::new(200)
                },
            ]
        );
    }

    #[test]
    fn ignores_expired_buckets() {
        let buckets = vec![
            bucket(1, 1000, NOW - 10, NOW - 1), // already expired
            bucket(2, 400, NOW - 5, NOW + 100),
        ];
        // Only 400 is spendable.
        assert_eq!(spendable_balance(&buckets, ts(NOW)), Yen::new(400));
        let err =
            plan_consumption(&buckets, PositiveYen::from_i64(500).unwrap(), ts(NOW)).unwrap_err();
        assert_eq!(err.available, Yen::new(400));
        assert_eq!(err.requested, Yen::new(500));
        // But 400 exactly is fine, drawn only from the active bucket.
        let plan =
            plan_consumption(&buckets, PositiveYen::from_i64(400).unwrap(), ts(NOW)).unwrap();
        assert_eq!(
            plan,
            vec![Deduction {
                bucket_id: Uuid::from_u128(2),
                amount: Yen::new(400)
            }]
        );
    }

    #[test]
    fn boundary_bucket_is_not_spendable() {
        // expires_at == now -> forfeited.
        let buckets = vec![bucket(1, 100, NOW - 10, NOW)];
        assert_eq!(spendable_balance(&buckets, ts(NOW)), Yen::ZERO);
        assert!(plan_consumption(&buckets, PositiveYen::from_i64(1).unwrap(), ts(NOW)).is_err());
    }

    proptest! {
        #[test]
        fn plan_is_consistent_with_balance(
            // (remaining, expiry_offset_secs, topped_up_offset_secs)
            specs in proptest::collection::vec(
                (0i64..=100_000, -50i64..=50, -100i64..=0),
                0..12usize,
            ),
            amount in 1i64..=600_000,
        ) {
            let buckets: Vec<SpendableBucket> = specs
                .iter()
                .enumerate()
                .map(|(i, (rem, exp_off, top_off))| {
                    bucket(i as u128 + 1, *rem, NOW + top_off, NOW + exp_off)
                })
                .collect();
            let now = ts(NOW);
            let amount_pos = PositiveYen::from_i64(amount).unwrap();
            let balance = spendable_balance(&buckets, now);

            match plan_consumption(&buckets, amount_pos, now) {
                Ok(plan) => {
                    // Enough balance existed.
                    prop_assert!(amount <= balance.as_i64());
                    // Deductions sum exactly to the requested amount.
                    let total: i64 = plan.iter().map(|d| d.amount.as_i64()).sum();
                    prop_assert_eq!(total, amount);
                    // Each deduction is positive and never exceeds that bucket's
                    // remaining; only active buckets are touched.
                    for d in &plan {
                        prop_assert!(d.amount.is_positive());
                        let b = buckets.iter().find(|b| b.id == d.bucket_id).unwrap();
                        prop_assert!(d.amount.as_i64() <= b.remaining.as_i64());
                        prop_assert!(is_active(b.expires_at, now));
                    }
                    // Deductions honor soonest-expiry-first order.
                    for w in plan.windows(2) {
                        let a = buckets.iter().find(|b| b.id == w[0].bucket_id).unwrap();
                        let b = buckets.iter().find(|b| b.id == w[1].bucket_id).unwrap();
                        prop_assert!(consumption_order(a, b) != std::cmp::Ordering::Greater);
                    }
                }
                Err(e) => {
                    // Failure happens exactly when the balance was short.
                    prop_assert!(amount > balance.as_i64());
                    prop_assert_eq!(e.available, balance);
                }
            }
        }
    }
}
