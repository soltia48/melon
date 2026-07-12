-- Payment processing fees.
--
-- `merchants.fee_bps` is the per-merchant fee rate in basis points (1 bps =
-- 0.01%; 10000 = 100%). `transactions.fee` is the fee actually charged on a
-- payment, recorded at payment time (the rate may change later). The fee is
-- borne by the merchant: the customer pays the full amount, and the merchant's
-- settlement is the payment amount minus the fee (the issuer keeps the fee).
ALTER TABLE merchants
    ADD COLUMN fee_bps INTEGER NOT NULL DEFAULT 0 CHECK (fee_bps BETWEEN 0 AND 10000);

ALTER TABLE transactions
    ADD COLUMN fee BIGINT NOT NULL DEFAULT 0 CHECK (fee >= 0);
