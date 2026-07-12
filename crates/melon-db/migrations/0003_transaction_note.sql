-- Optional free-text note on a transaction, used to record the reason for an
-- admin balance adjustment (audit trail).
ALTER TABLE transactions ADD COLUMN note TEXT;
