-- Stores (店舗) live under a merchant.
--
-- A merchant can have many stores; a transaction is attributed to a store, an API
-- key belongs to a store (so a terminal's key identifies its store), and a
-- merchant staff user is either merchant-wide (store_id NULL) or scoped to one
-- store. Every EXISTING merchant gets one default store, and its current
-- transactions and API keys are backfilled onto it.
--
-- Fees, credit limit and settlement stay at the merchant level — a store is an
-- organizational + reporting unit, not a separate settlement entity.

CREATE TABLE stores (
    id          UUID PRIMARY KEY,
    merchant_id UUID NOT NULL REFERENCES merchants (id),
    code        TEXT NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'closed')),
    is_default  BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Store code is unique within its merchant (not globally).
    UNIQUE (merchant_id, code)
);
-- At most one default store per merchant.
CREATE UNIQUE INDEX ux_stores_default ON stores (merchant_id) WHERE is_default;
CREATE INDEX ix_stores_merchant ON stores (merchant_id);

-- Attribution / scoping columns (all nullable: issuer top-ups & adjustments have
-- no store, and pre-existing rows are backfilled below).
ALTER TABLE transactions      ADD COLUMN store_id UUID REFERENCES stores (id);
ALTER TABLE merchant_api_keys ADD COLUMN store_id UUID REFERENCES stores (id);
ALTER TABLE users             ADD COLUMN store_id UUID REFERENCES stores (id);
-- A store scope only makes sense for a merchant user (admins are issuer-wide).
ALTER TABLE users ADD CONSTRAINT users_store_requires_merchant
    CHECK (store_id IS NULL OR role = 'merchant');

CREATE INDEX ix_txn_store ON transactions (store_id, occurred_at DESC) WHERE store_id IS NOT NULL;
CREATE INDEX ix_api_keys_store ON merchant_api_keys (store_id);
CREATE INDEX ix_users_store ON users (store_id) WHERE store_id IS NOT NULL;

-- ----- backfill existing data onto a per-merchant default store -----

-- One default store per existing merchant.
INSERT INTO stores (id, merchant_id, code, name, is_default)
SELECT gen_random_uuid(), m.id, 'default', '本店', true
FROM merchants m;

-- Attribute each merchant's existing transactions to its default store.
UPDATE transactions t
SET store_id = s.id
FROM stores s
WHERE s.merchant_id = t.merchant_id
  AND s.is_default
  AND t.merchant_id IS NOT NULL;

-- Existing API keys belong to the merchant's default store.
UPDATE merchant_api_keys k
SET store_id = s.id
FROM stores s
WHERE s.merchant_id = k.merchant_id
  AND s.is_default;

-- Existing merchant users keep store_id = NULL, i.e. they become merchant-wide
-- administrators (able to see every store). New store-scoped users are created
-- with a store_id going forward.
