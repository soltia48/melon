//! Money as integer Japanese yen. Never a float.
//!
//! [`Yen`] is a signed amount used for ledger deltas; [`PositiveYen`] is a
//! strictly-positive amount used to guard inputs (top-up amounts, prices).
//! Arithmetic is checked-only: there is no `From<f64>` and no multiplication or
//! division by a float, so a rounding error can never enter the system.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An error from a money operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    /// A checked add/sub overflowed the `i64` range.
    #[error("yen arithmetic overflow")]
    Overflow,
    /// A value required to be strictly positive was zero or negative.
    #[error("amount must be strictly positive")]
    NonPositive,
}

/// A signed amount of Japanese yen (no minor unit). Stored as `i64` yen.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Yen(i64);

impl Yen {
    /// Zero yen.
    pub const ZERO: Yen = Yen(0);

    /// Wrap a raw `i64` yen value.
    pub const fn new(value: i64) -> Yen {
        Yen(value)
    }

    /// The raw `i64` yen value.
    pub const fn as_i64(self) -> i64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Checked addition; `Err(Overflow)` instead of wrapping.
    pub fn checked_add(self, other: Yen) -> Result<Yen, MoneyError> {
        self.0
            .checked_add(other.0)
            .map(Yen)
            .ok_or(MoneyError::Overflow)
    }

    /// Checked subtraction; `Err(Overflow)` instead of wrapping. Note this may
    /// return a *negative* `Yen` — callers that must not go negative check the
    /// sign (or rely on the DB `CHECK (remaining_amount >= 0)`).
    pub fn checked_sub(self, other: Yen) -> Result<Yen, MoneyError> {
        self.0
            .checked_sub(other.0)
            .map(Yen)
            .ok_or(MoneyError::Overflow)
    }

    /// The fee for this amount at `bps` basis points (1 bps = 0.01%), floored
    /// toward zero. e.g. `Yen(1000).fee_bps(300)` (3%) = `Yen(30)`.
    pub fn fee_bps(self, bps: i32) -> Yen {
        Yen(((self.0 as i128) * (bps as i128) / 10_000) as i64)
    }

    /// Sum an iterator of amounts with overflow checking.
    pub fn checked_sum<I: IntoIterator<Item = Yen>>(iter: I) -> Result<Yen, MoneyError> {
        let mut acc = Yen::ZERO;
        for y in iter {
            acc = acc.checked_add(y)?;
        }
        Ok(acc)
    }
}

impl fmt::Display for Yen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "¥{}", self.0)
    }
}

impl fmt::Debug for Yen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Yen({})", self.0)
    }
}

/// A strictly-positive amount of yen (`> 0`). Used to type-guard inputs such as
/// a top-up or payment amount.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PositiveYen(Yen);

impl PositiveYen {
    /// Construct from a [`Yen`], rejecting zero or negative values.
    pub fn new(value: Yen) -> Result<Self, MoneyError> {
        if value.is_positive() {
            Ok(PositiveYen(value))
        } else {
            Err(MoneyError::NonPositive)
        }
    }

    /// Construct from a raw `i64` yen value, rejecting non-positive values.
    pub fn from_i64(value: i64) -> Result<Self, MoneyError> {
        PositiveYen::new(Yen::new(value))
    }

    /// The underlying (positive) [`Yen`].
    pub const fn get(self) -> Yen {
        self.0
    }

    /// The raw `i64` value (always `> 0`).
    pub const fn as_i64(self) -> i64 {
        self.0.as_i64()
    }
}

impl fmt::Display for PositiveYen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for PositiveYen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PositiveYen({})", self.0.as_i64())
    }
}

impl<'de> Deserialize<'de> for PositiveYen {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Yen::deserialize(deserializer)?;
        PositiveYen::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_add_and_sub() {
        assert_eq!(
            Yen::new(300).checked_add(Yen::new(200)).unwrap(),
            Yen::new(500)
        );
        assert_eq!(
            Yen::new(300).checked_sub(Yen::new(500)).unwrap(),
            Yen::new(-200)
        );
    }

    #[test]
    fn overflow_is_reported_not_wrapped() {
        assert_eq!(
            Yen::new(i64::MAX).checked_add(Yen::new(1)),
            Err(MoneyError::Overflow)
        );
        assert_eq!(
            Yen::new(i64::MIN).checked_sub(Yen::new(1)),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn positive_yen_rejects_zero_and_negative() {
        assert!(PositiveYen::from_i64(1).is_ok());
        assert_eq!(PositiveYen::from_i64(0), Err(MoneyError::NonPositive));
        assert_eq!(PositiveYen::from_i64(-5), Err(MoneyError::NonPositive));
    }

    #[test]
    fn fee_bps_floors() {
        assert_eq!(Yen::new(1000).fee_bps(300), Yen::new(30)); // 3%
        assert_eq!(Yen::new(1234).fee_bps(300), Yen::new(37)); // 37.02 -> 37
        assert_eq!(Yen::new(1000).fee_bps(0), Yen::ZERO);
        assert_eq!(Yen::new(1000).fee_bps(10000), Yen::new(1000)); // 100%
    }

    #[test]
    fn checked_sum_totals() {
        let total = Yen::checked_sum([Yen::new(100), Yen::new(250), Yen::new(50)]).unwrap();
        assert_eq!(total, Yen::new(400));
    }

    #[test]
    fn serde_yen_is_a_bare_integer() {
        assert_eq!(serde_json::to_string(&Yen::new(500)).unwrap(), "500");
        assert_eq!(serde_json::from_str::<Yen>("500").unwrap(), Yen::new(500));
    }

    #[test]
    fn serde_positive_yen_validates_on_deserialize() {
        assert_eq!(
            serde_json::to_string(&PositiveYen::from_i64(500).unwrap()).unwrap(),
            "500"
        );
        assert!(serde_json::from_str::<PositiveYen>("500").is_ok());
        assert!(serde_json::from_str::<PositiveYen>("0").is_err());
        assert!(serde_json::from_str::<PositiveYen>("-1").is_err());
    }
}
