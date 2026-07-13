//! User accounts and server-side sessions (human sign-on).
//!
//! Separate from the machine credential: the terminal keeps using a merchant API
//! key. Here, people log in with an email + password (Argon2id hash, produced by
//! the server) and receive an opaque session token whose SHA-256 is all this
//! layer stores — a DB leak cannot be replayed as a login, and deleting the row
//! revokes the session immediately.

use jiff::Timestamp;
use sqlx::Row;
use uuid::Uuid;

use crate::conv::{to_jiff, to_odt};
use crate::{DbError, Pool};

/// A user account (never carries the password hash).
#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    /// `admin` (issuer staff) or `merchant`.
    pub role: String,
    /// `Some` exactly when `role == "merchant"`.
    pub merchant_id: Option<Uuid>,
    /// A merchant user's store scope: `None` = merchant-wide admin (all stores);
    /// `Some` = store user, restricted to that store. Always `None` for `admin`.
    pub store_id: Option<Uuid>,
    /// `active` or `disabled`.
    pub status: String,
    pub created_at: Timestamp,
}

fn map_user(r: sqlx::postgres::PgRow) -> Result<User, DbError> {
    Ok(User {
        id: r.try_get("id")?,
        email: r.try_get("email")?,
        name: r.try_get("name")?,
        role: r.try_get("role")?,
        merchant_id: r.try_get("merchant_id")?,
        store_id: r.try_get("store_id")?,
        status: r.try_get("status")?,
        created_at: to_jiff(r.try_get("created_at")?),
    })
}

const USER_COLS: &str = "id, email, name, role, merchant_id, store_id, status, created_at";

/// Create a user. `password_hash` must already be an Argon2id PHC string — this
/// layer never sees a plaintext password. `merchant_id` is required for the
/// `merchant` role and rejected for `admin` (enforced by a DB CHECK too).
pub async fn create_user(
    pool: &Pool,
    email: &str,
    name: &str,
    password_hash: &str,
    role: &str,
    merchant_id: Option<Uuid>,
    store_id: Option<Uuid>,
) -> Result<User, DbError> {
    let id = Uuid::now_v7();
    let row = sqlx::query(&format!(
        "INSERT INTO users (id, email, name, password_hash, role, merchant_id, store_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {USER_COLS}"
    ))
    .bind(id)
    .bind(email)
    .bind(name)
    .bind(password_hash)
    .bind(role)
    .bind(merchant_id)
    .bind(store_id)
    .fetch_one(pool)
    .await;
    match row {
        Ok(row) => map_user(row),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(DbError::EmailTaken),
        Err(e) => Err(e.into()),
    }
}

/// Look up a user by email (case-insensitive) together with the stored password
/// hash, for login. Returns `None` if no such user.
pub async fn user_for_login(pool: &Pool, email: &str) -> Result<Option<(User, String)>, DbError> {
    let row = sqlx::query(&format!(
        "SELECT {USER_COLS}, password_hash FROM users WHERE lower(email) = lower($1)"
    ))
    .bind(email)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let hash: String = r.try_get("password_hash")?;
            Ok(Some((map_user(r)?, hash)))
        }
        None => Ok(None),
    }
}

/// The stored password hash for a user (used to verify the current password).
pub async fn password_hash(pool: &Pool, user_id: Uuid) -> Result<Option<String>, DbError> {
    Ok(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// List users: all of them for an admin, or one merchant's staff when
/// `merchant_id` is set. Newest first.
pub async fn list_users(pool: &Pool, merchant_id: Option<Uuid>) -> Result<Vec<User>, DbError> {
    let rows = sqlx::query(&format!(
        "SELECT {USER_COLS} FROM users
          WHERE ($1::uuid IS NULL OR merchant_id = $1)
          ORDER BY created_at DESC"
    ))
    .bind(merchant_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_user).collect()
}

/// Fetch one user by id.
pub async fn get_user(pool: &Pool, id: Uuid) -> Result<Option<User>, DbError> {
    let row = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(map_user).transpose()
}

/// Enable/disable a user. Disabling also revokes every session they hold.
pub async fn set_user_status(pool: &Pool, id: Uuid, status: &str) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    let affected = sqlx::query("UPDATE users SET status = $1, updated_at = now() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(DbError::UserNotFound);
    }
    if status != "active" {
        sqlx::query("DELETE FROM user_sessions WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Replace a user's password hash and revoke all their existing sessions (a
/// password change must log out every other device).
pub async fn set_password_hash(pool: &Pool, id: Uuid, hash: &str) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    let affected =
        sqlx::query("UPDATE users SET password_hash = $1, updated_at = now() WHERE id = $2")
            .bind(hash)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(DbError::UserNotFound);
    }
    sqlx::query("DELETE FROM user_sessions WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Whether any admin user exists (drives the first-run bootstrap).
pub async fn admin_exists(pool: &Pool) -> Result<bool, DbError> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin')")
            .fetch_one(pool)
            .await?,
    )
}

// ----- sessions -----

/// Start a session. `token_hash` is the SHA-256 of the opaque cookie value; the
/// value itself is never stored.
pub async fn create_session(
    pool: &Pool,
    token_hash: &str,
    user_id: Uuid,
    expires_at: Timestamp,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO user_sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(token_hash)
        .bind(user_id)
        .bind(to_odt(expires_at))
        .execute(pool)
        .await?;
    Ok(())
}

/// Resolve a live session to its user, refreshing `last_seen_at`. Returns `None`
/// if the session is unknown, expired, or the user has been disabled.
pub async fn session_user(
    pool: &Pool,
    token_hash: &str,
    now: Timestamp,
) -> Result<Option<User>, DbError> {
    let row = sqlx::query(&format!(
        "UPDATE user_sessions s SET last_seen_at = $2
           FROM users u
          WHERE s.token_hash = $1 AND s.expires_at > $2
            AND u.id = s.user_id AND u.status = 'active'
        RETURNING {}",
        USER_COLS
            .split(", ")
            .map(|c| format!("u.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(token_hash)
    .bind(to_odt(now))
    .fetch_optional(pool)
    .await?;
    row.map(map_user).transpose()
}

/// Revoke one session (logout).
pub async fn delete_session(pool: &Pool, token_hash: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM user_sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop sessions that have expired. Returns how many were removed.
pub async fn purge_expired_sessions(pool: &Pool, now: Timestamp) -> Result<u64, DbError> {
    Ok(
        sqlx::query("DELETE FROM user_sessions WHERE expires_at <= $1")
            .bind(to_odt(now))
            .execute(pool)
            .await?
            .rows_affected(),
    )
}
