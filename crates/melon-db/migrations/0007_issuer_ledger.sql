-- Issuer (発行者) revenue account.
--
-- The issuer's balance is a *revenue* figure, derived from data already in the
-- books plus manual entries here:
--
--   issuer balance = payment fee income (SUM transactions.fee over payments)
--                  + breakage income     (SUM -ledger_entries.amount, kind='expiry')
--                  + SUM(these adjustments)
--
-- Fee income and breakage need no new storage (single source of truth). This
-- table only records manual issuer entries: withdrawals (profit taken out, -) and
-- corrections / capital injections (+). Append-only for audit, like the ledger.
CREATE TABLE issuer_adjustments (
    id         UUID PRIMARY KEY,
    amount     BIGINT NOT NULL CHECK (amount <> 0), -- signed: + credits/injections, - withdrawals
    note       TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Reuse the append-only guard defined in 0001.
CREATE TRIGGER issuer_adjustments_append_only
    BEFORE UPDATE OR DELETE ON issuer_adjustments
    FOR EACH ROW EXECUTE FUNCTION melon_forbid_ledger_mutation();
