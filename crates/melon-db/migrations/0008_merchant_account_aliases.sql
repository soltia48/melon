-- Per-merchant pseudonymous account IDs.
--
-- A merchant must never see the raw card identity `(system_code, idi)`. Instead,
-- every (merchant, account) pair gets its own opaque alias (UUID v4 — v7 would
-- leak a creation timestamp). The SAME card therefore appears under a DIFFERENT
-- alias at each merchant, so merchants cannot correlate a cardholder across
-- merchants even if they collude. Only the issuer (admin) can map an alias back
-- to `(system_code, idi)`.
CREATE TABLE merchant_account_aliases (
    alias       UUID PRIMARY KEY,
    merchant_id UUID NOT NULL REFERENCES merchants (id),
    system_code INTEGER NOT NULL,
    idi         BYTEA NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One alias per (merchant, account) — stable, so a merchant recognizes its
    -- own returning customer.
    UNIQUE (merchant_id, system_code, idi),
    FOREIGN KEY (system_code, idi) REFERENCES accounts (system_code, idi)
);

-- Reverse lookup (alias -> account) is the PK; this covers the forward lookup.
CREATE INDEX ix_alias_account ON merchant_account_aliases (system_code, idi);

-- Aliases are handed out to merchants and must never be reassigned or rewritten.
CREATE TRIGGER merchant_account_aliases_append_only
    BEFORE UPDATE OR DELETE ON merchant_account_aliases
    FOR EACH ROW EXECUTE FUNCTION melon_forbid_ledger_mutation();
