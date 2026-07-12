-- Add the FeliCa **IDm** to the account key: an account is now identified by the
-- triple (system_code, idm, idi), not just (system_code, idi).
--
-- IDm is folded into the composite PK/FK, keyed in (system_code, idm, idi)
-- order. There is no source to back-fill an IDm for pre-existing rows, so this
-- migration only applies cleanly to EMPTY account tables — i.e. a fresh or reset
-- database. (Adding a NOT NULL column with no default is fine on an empty table.)

-- Drop the FKs that reference accounts (and the alias uniqueness) so the accounts
-- PK can be rebuilt.
ALTER TABLE transactions             DROP CONSTRAINT transactions_system_code_idi_fkey;
ALTER TABLE topup_buckets            DROP CONSTRAINT topup_buckets_system_code_idi_fkey;
ALTER TABLE ledger_entries           DROP CONSTRAINT ledger_entries_system_code_idi_fkey;
ALTER TABLE merchant_account_aliases DROP CONSTRAINT merchant_account_aliases_system_code_idi_fkey;
ALTER TABLE merchant_account_aliases DROP CONSTRAINT merchant_account_aliases_merchant_id_system_code_idi_key;

-- accounts: new column, new primary key (system_code, idm, idi).
ALTER TABLE accounts
    ADD COLUMN idm BYTEA NOT NULL CHECK (octet_length(idm) = 8);
ALTER TABLE accounts DROP CONSTRAINT accounts_pkey;
ALTER TABLE accounts ADD PRIMARY KEY (system_code, idm, idi);

-- Child tables: carry idm and reference the account by the full triple.
ALTER TABLE transactions
    ADD COLUMN idm BYTEA NOT NULL CHECK (octet_length(idm) = 8);
ALTER TABLE transactions
    ADD FOREIGN KEY (system_code, idm, idi) REFERENCES accounts (system_code, idm, idi);

ALTER TABLE topup_buckets
    ADD COLUMN idm BYTEA NOT NULL CHECK (octet_length(idm) = 8);
ALTER TABLE topup_buckets
    ADD FOREIGN KEY (system_code, idm, idi) REFERENCES accounts (system_code, idm, idi);

ALTER TABLE ledger_entries
    ADD COLUMN idm BYTEA NOT NULL CHECK (octet_length(idm) = 8);
ALTER TABLE ledger_entries
    ADD FOREIGN KEY (system_code, idm, idi) REFERENCES accounts (system_code, idm, idi);

ALTER TABLE merchant_account_aliases
    ADD COLUMN idm BYTEA NOT NULL CHECK (octet_length(idm) = 8);
ALTER TABLE merchant_account_aliases
    ADD FOREIGN KEY (system_code, idm, idi) REFERENCES accounts (system_code, idm, idi);
-- One alias per (merchant, full account).
ALTER TABLE merchant_account_aliases
    ADD UNIQUE (merchant_id, system_code, idm, idi);

-- Refresh the hot-path indexes to lead with the full account key.
DROP INDEX ix_txn_acct;
CREATE INDEX ix_txn_acct ON transactions (system_code, idm, idi, occurred_at DESC);
DROP INDEX ix_buckets_spend;
CREATE INDEX ix_buckets_spend ON topup_buckets (system_code, idm, idi, expires_at)
    WHERE status = 'active';
DROP INDEX ix_ledger_acct;
CREATE INDEX ix_ledger_acct ON ledger_entries (system_code, idm, idi, seq);
DROP INDEX ix_alias_account;
CREATE INDEX ix_alias_account ON merchant_account_aliases (system_code, idm, idi);
