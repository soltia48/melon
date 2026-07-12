//! The 6-month expiry boundary for top-up buckets.
//!
//! Under 資金決済法, a prepaid instrument usable only within 6 months of
//! issuance is exempt from deposit/registration obligations. Each top-up is an
//! issuance, so its value expires `topped_up_at + 6 months`, computed on the
//! **Asia/Tokyo (JST)** wall clock and stored as a UTC instant.
//!
//! The month arithmetic uses jiff, whose calendar addition *clamps* an
//! out-of-range day to the last valid day of the target month (e.g.
//! `Aug 31 + 6 months -> Feb 28`, never rolling into March). That is the
//! legally-conservative rule: the validity period never exceeds 6 months.
//!
//! A bucket is valid while `now < expires_at` and forfeited once
//! `now >= expires_at` (see [`is_active`] / [`is_expired`]).

use jiff::{Timestamp, ToSpan, tz::TimeZone};

/// The validity window, in calendar months, applied to every top-up.
pub const VALIDITY_MONTHS: i32 = 6;

/// The IANA timezone whose wall clock defines the expiry boundary.
pub const EXPIRY_TZ_NAME: &str = "Asia/Tokyo";

/// An error computing an expiry instant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpiryError {
    #[error("expiry timezone '{0}' is unavailable")]
    TimezoneUnavailable(&'static str),
    #[error("expiry instant is out of the representable range")]
    OutOfRange,
}

/// Look up the JST timezone used for expiry math. Cache and reuse the result
/// (e.g. once per sweep or per request batch) rather than calling per top-up.
pub fn expiry_timezone() -> Result<TimeZone, ExpiryError> {
    TimeZone::get(EXPIRY_TZ_NAME).map_err(|_| ExpiryError::TimezoneUnavailable(EXPIRY_TZ_NAME))
}

/// Compute `topped_up_at + 6 months` on the JST wall clock, using a caller-held
/// timezone. Materialize this at top-up time and store it — do not recompute in
/// SQL, which cannot reproduce jiff's JST-civil + day-clamp rule.
pub fn expires_at_in(topped_up_at: Timestamp, tz: &TimeZone) -> Result<Timestamp, ExpiryError> {
    let zoned = topped_up_at.to_zoned(tz.clone());
    let expires = zoned
        .checked_add(VALIDITY_MONTHS.months())
        .map_err(|_| ExpiryError::OutOfRange)?;
    Ok(expires.timestamp())
}

/// Convenience wrapper that looks up the JST timezone each call.
pub fn expires_at(topped_up_at: Timestamp) -> Result<Timestamp, ExpiryError> {
    let tz = expiry_timezone()?;
    expires_at_in(topped_up_at, &tz)
}

/// A bucket is spendable while `now` is strictly before its expiry instant.
pub fn is_active(expires_at: Timestamp, now: Timestamp) -> bool {
    now < expires_at
}

/// A bucket is forfeited once `now` reaches its expiry instant.
pub fn is_expired(expires_at: Timestamp, now: Timestamp) -> bool {
    now >= expires_at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().expect("valid timestamp")
    }

    /// Table-driven golden cases: (top-up JST wall time, expected expiry JST).
    #[test]
    fn six_month_boundary_golden_cases() {
        let cases = [
            // Plain case.
            ("2026-01-15T09:00:00+09:00", "2026-07-15T09:00:00+09:00"),
            // Aug 31 -> Feb has no 31st: clamp to Feb 28 (2026 not a leap year),
            // never rolling into March.
            ("2025-08-31T12:00:00+09:00", "2026-02-28T12:00:00+09:00"),
            // Leap-year target: Aug 29 2023 -> Feb 29 2024 exists.
            ("2023-08-29T12:00:00+09:00", "2024-02-29T12:00:00+09:00"),
            // Month-end that stays valid.
            ("2026-04-30T00:00:00+09:00", "2026-10-30T00:00:00+09:00"),
        ];
        let tz = expiry_timezone().unwrap();
        for (topup, expected) in cases {
            assert_eq!(
                expires_at_in(ts(topup), &tz).unwrap(),
                ts(expected),
                "top-up {topup}",
            );
        }
    }

    /// The month arithmetic runs on the JST civil date, not the UTC date:
    /// 2026-03-31T23:30:00Z is 2026-04-01T08:30 JST, so +6 months lands in
    /// October (from April), not September (from March).
    #[test]
    fn boundary_uses_jst_civil_date_not_utc() {
        let tz = expiry_timezone().unwrap();
        let topup = ts("2026-03-31T23:30:00Z");
        let expires = expires_at_in(topup, &tz).unwrap();
        assert_eq!(expires, ts("2026-10-01T08:30:00+09:00"));
    }

    #[test]
    fn active_until_the_instant_then_expired() {
        let expires = ts("2026-07-15T09:00:00+09:00");
        assert!(is_active(expires, ts("2026-07-15T08:59:59+09:00")));
        // Exactly at the boundary the bucket is forfeited.
        assert!(!is_active(expires, expires));
        assert!(is_expired(expires, expires));
        assert!(is_expired(expires, ts("2026-07-15T09:00:01+09:00")));
    }
}
