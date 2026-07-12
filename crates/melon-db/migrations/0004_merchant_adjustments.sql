-- Admin adjustments to a merchant's settlement balance, append-only for audit.
-- A merchant's balance = (payments received − refunds/reversals) + SUM(these).
CREATE TABLE merchant_adjustments (
    id          UUID PRIMARY KEY,
    merchant_id UUID NOT NULL REFERENCES merchants (id),
    amount      BIGINT NOT NULL CHECK (amount <> 0), -- signed: + credits, - debits
    note        TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ix_merchant_adj ON merchant_adjustments (merchant_id);

-- Reuse the append-only guard defined in 0001.
CREATE TRIGGER merchant_adjustments_append_only
    BEFORE UPDATE OR DELETE ON merchant_adjustments
    FOR EACH ROW EXECUTE FUNCTION melon_forbid_ledger_mutation();
