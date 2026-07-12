-- Human sign-on: user accounts + server-side sessions.
--
-- Distinct from the MACHINE credential: the terminal keeps authenticating with a
-- merchant API key. These are people, with passwords (Argon2id) and revocable
-- server-side sessions delivered as HttpOnly cookies.
CREATE TABLE users (
    id            UUID PRIMARY KEY,
    email         TEXT NOT NULL,
    name          TEXT NOT NULL,
    -- Argon2id PHC string (algorithm + params + per-password salt + hash).
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'merchant')),
    -- Merchant users are scoped to exactly one merchant; admins to none.
    merchant_id   UUID REFERENCES merchants (id),
    status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((role = 'merchant') = (merchant_id IS NOT NULL))
);

-- Email is the login id, case-insensitively unique.
CREATE UNIQUE INDEX ux_users_email ON users (lower(email));
CREATE INDEX ix_users_merchant ON users (merchant_id) WHERE merchant_id IS NOT NULL;

-- Server-side sessions: only the SHA-256 of the cookie value is stored, so a DB
-- leak cannot be replayed as a login. Deleting a row revokes the session at once.
CREATE TABLE user_sessions (
    token_hash   TEXT PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);
CREATE INDEX ix_user_sessions_user ON user_sessions (user_id);
CREATE INDEX ix_user_sessions_expiry ON user_sessions (expires_at);
