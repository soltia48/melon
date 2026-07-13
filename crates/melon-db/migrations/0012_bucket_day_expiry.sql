-- Move top-up bucket expiry to DAY granularity.
--
-- A bucket now forfeits at the START of the JST day (00:00) that is
-- `topup_date + 6 months`, rather than at the exact time-of-day of the top-up.
-- This normalizes every existing bucket's expires_at to that day boundary.
--
-- Truncating an already-materialized expires_at to its JST day needs no calendar
-- month arithmetic (the +6-months clamp was applied when the value was written),
-- so Postgres reproduces jiff's result exactly here. The change only ever moves an
-- expiry EARLIER (it drops the intraday time), so every bucket stays within the
-- 6-month window — the legally conservative direction.
UPDATE topup_buckets
SET expires_at = date_trunc('day', expires_at AT TIME ZONE 'Asia/Tokyo') AT TIME ZONE 'Asia/Tokyo'
WHERE expires_at
    <> date_trunc('day', expires_at AT TIME ZONE 'Asia/Tokyo') AT TIME ZONE 'Asia/Tokyo';
