-- melon initial schema.
--
-- Money is BIGINT yen (no minor unit, no float). Enum-like columns are TEXT +
-- CHECK. Timestamps are timestamptz; `topup_buckets.expires_at` is materialized
-- by the app (jiff, JST + 6 months) and never recomputed in SQL.
--
-- An account is identified by the PAIR (system_code, idi): an IDi is only unique
-- within a FeliCa system, so every account-bearing table carries both and
-- references accounts by the composite key.

CREATE TABLE accounts (
    system_code INTEGER NOT NULL CHECK (system_code BETWEEN 0 AND 65535),
    idi         BYTEA NOT NULL CHECK (octet_length(idi) = 8),
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'frozen', 'closed')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (system_code, idi)
);

CREATE TABLE merchants (
    id         UUID PRIMARY KEY,
    code       TEXT UNIQUE NOT NULL,
    name       TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'closed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Business events (取引). One row here plus one or more ledger postings.
CREATE TABLE transactions (
    id              UUID PRIMARY KEY,
    system_code     INTEGER NOT NULL,
    idi             BYTEA NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('top_up', 'payment', 'refund', 'reversal', 'adjustment')),
    merchant_id     UUID REFERENCES merchants (id),
    amount          BIGINT NOT NULL CHECK (amount > 0),
    idempotency_key TEXT NOT NULL,
    related_txn_id  UUID REFERENCES transactions (id),
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (kind, idempotency_key),
    FOREIGN KEY (system_code, idi) REFERENCES accounts (system_code, idi)
);
CREATE INDEX ix_txn_acct ON transactions (system_code, idi, occurred_at DESC);
CREATE INDEX ix_txn_merchant ON transactions (merchant_id, occurred_at DESC) WHERE merchant_id IS NOT NULL;

-- One top-up lot with its own 6-month expiry. `remaining_amount` is a
-- transactionally-maintained cache of SUM(ledger.amount) over this bucket.
CREATE TABLE topup_buckets (
    id               UUID PRIMARY KEY,
    system_code      INTEGER NOT NULL,
    idi              BYTEA NOT NULL,
    topup_txn_id     UUID NOT NULL REFERENCES transactions (id),
    original_amount  BIGINT NOT NULL CHECK (original_amount > 0),
    remaining_amount BIGINT NOT NULL CHECK (remaining_amount >= 0),
    topped_up_at     TIMESTAMPTZ NOT NULL,
    expires_at       TIMESTAMPTZ NOT NULL,
    status           TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'exhausted', 'expired')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (remaining_amount <= original_amount),
    CHECK (expires_at > topped_up_at),
    FOREIGN KEY (system_code, idi) REFERENCES accounts (system_code, idi)
);
CREATE INDEX ix_buckets_spend ON topup_buckets (system_code, idi, expires_at) WHERE status = 'active';
CREATE INDEX ix_buckets_sweep ON topup_buckets (expires_at) WHERE status = 'active';

-- Immutable, append-only ledger: the source of truth. Signed `amount`; a CHECK
-- ties sign to kind.
CREATE TABLE ledger_entries (
    seq            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id             UUID NOT NULL UNIQUE,
    system_code    INTEGER NOT NULL,
    idi            BYTEA NOT NULL,
    -- NULL only for system-generated `expiry` postings.
    transaction_id UUID REFERENCES transactions (id),
    bucket_id      UUID REFERENCES topup_buckets (id),
    kind           TEXT NOT NULL CHECK (kind IN ('top_up', 'payment', 'refund', 'expiry', 'reversal', 'adjustment')),
    amount         BIGINT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (kind = 'expiry' OR transaction_id IS NOT NULL),
    CHECK (
        (kind IN ('top_up', 'refund') AND amount > 0)
        OR (kind IN ('payment', 'expiry') AND amount < 0)
        OR (kind IN ('reversal', 'adjustment') AND amount <> 0)
    ),
    FOREIGN KEY (system_code, idi) REFERENCES accounts (system_code, idi)
);
CREATE INDEX ix_ledger_bucket ON ledger_entries (bucket_id);
CREATE INDEX ix_ledger_acct ON ledger_entries (system_code, idi, seq);

-- Append-only enforcement (fires regardless of table ownership).
CREATE FUNCTION melon_forbid_ledger_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'ledger_entries is append-only; % is not permitted', TG_OP;
END;
$$;
CREATE TRIGGER ledger_entries_append_only
    BEFORE UPDATE OR DELETE ON ledger_entries
    FOR EACH ROW EXECUTE FUNCTION melon_forbid_ledger_mutation();
