-- Per-merchant credit limit for selling top-ups on credit.
--
-- A merchant's settlement balance (what the issuer owes the merchant) goes
-- negative as they sell top-ups (they hold the issuer's cash). `credit_limit` is
-- how far negative the settlement may go: a top-up is rejected if it would push
-- the settlement below `-credit_limit`. 0 means no credit (top-ups only up to a
-- positive settlement).
ALTER TABLE merchants
    ADD COLUMN credit_limit BIGINT NOT NULL DEFAULT 0 CHECK (credit_limit >= 0);
