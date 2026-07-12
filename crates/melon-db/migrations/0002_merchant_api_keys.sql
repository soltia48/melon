-- Merchant API credentials. Only the SHA-256 hash of the secret is stored; the
-- plaintext secret is shown once at issuance and never persisted.
CREATE TABLE merchant_api_keys (
    id          UUID PRIMARY KEY,
    merchant_id UUID NOT NULL REFERENCES merchants (id),
    key_hash    TEXT UNIQUE NOT NULL,
    label       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ
);
CREATE INDEX ix_api_keys_merchant ON merchant_api_keys (merchant_id);
