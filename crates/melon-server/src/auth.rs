//! Authentication: machine credentials (terminal) and human sign-on (people).
//!
//! Two distinct things:
//!
//! * **Merchant API key** — a *machine* credential. The terminal presents
//!   `Authorization: Bearer <secret>`; only its SHA-256 is stored.
//! * **User sign-on** — *people* (issuer staff and merchant staff) log in with an
//!   email + password (Argon2id) and get an opaque session token delivered as an
//!   **HttpOnly, SameSite=Strict cookie**. JavaScript can never read it, so an XSS
//!   cannot exfiltrate the credential, and the server can revoke a session by
//!   deleting one row. Only the SHA-256 of the token is stored.

use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use melon_db::users::User;

use crate::AppState;
use crate::error::ApiError;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "melon_session";

// ----- passwords -----

/// Hash a password with Argon2id (memory-hard, per-password salt). Returns a PHC
/// string that embeds the algorithm, parameters and salt.
pub fn hash_password(plain: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ApiError::internal("failed to hash password"))
}

/// Verify a password against its stored PHC hash. Any parse/verify failure is a
/// mismatch — never an error the caller could distinguish.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|parsed| {
            Argon2::default()
                .verify_password(plain.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Minimum password length we accept (length beats composition rules).
pub const MIN_PASSWORD_LEN: usize = 10;

pub fn validate_password(plain: &str) -> Result<(), ApiError> {
    if plain.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::bad_request(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

// ----- session cookies -----

/// The `Set-Cookie` value that starts a session.
pub fn session_cookie(token: &str, ttl: Duration, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        ttl.as_secs(),
        if secure { "; Secure" } else { "" }
    )
}

/// The `Set-Cookie` value that clears the session (logout).
pub fn clear_session_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

/// Read the session token out of the `Cookie` header.
pub fn session_token(parts: &Parts) -> Option<String> {
    let cookies = parts.headers.get(COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })
}

// ----- extractors -----

/// Any signed-in user (admin or merchant staff).
#[derive(Debug, Clone)]
pub struct AuthUser(pub User);

/// A signed-in **issuer/admin** user.
#[derive(Debug, Clone)]
pub struct AdminUser(pub User);

/// A signed-in **merchant staff** user, with the merchant they belong to.
#[derive(Debug, Clone)]
pub struct MerchantUser {
    pub user: User,
    pub merchant_id: Uuid,
    /// The user's store scope: `None` = merchant-wide admin (all stores);
    /// `Some` = restricted to that store.
    pub store_id: Option<Uuid>,
}

/// A merchant caller — either the **terminal** (API key) or a signed-in
/// **merchant staff user** (session cookie). Both act for one merchant.
#[derive(Debug, Clone, Copy)]
pub struct AuthedMerchant {
    pub merchant_id: Uuid,
    /// The caller's store scope: the terminal's store (API key), or the staff
    /// user's store (`None` for a merchant-wide admin). Payments/top-ups are
    /// attributed to it; listings are filtered by it when set.
    pub store_id: Option<Uuid>,
    /// Which credential acted. Audit lines record it: "the merchant charged ¥500"
    /// is not an answer when the question is which terminal, or which member of
    /// staff, did it.
    pub actor: Actor,
}

/// The credential behind a request, as recorded in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// A terminal, holding a store-scoped API key.
    ApiKey(Uuid),
    /// A person, signed in with email + password.
    User(Uuid),
}

impl Actor {
    /// `api_key` or `user` — the audit log's `actor_kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            Actor::ApiKey(_) => "api_key",
            Actor::User(_) => "user",
        }
    }

    /// The key id or the user id — the audit log's `actor_id`.
    pub fn id(&self) -> Uuid {
        match self {
            Actor::ApiKey(id) | Actor::User(id) => *id,
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = session_token(parts).ok_or_else(|| ApiError::unauthorized("not signed in"))?;
        let user =
            melon_db::users::session_user(&state.pool, &sha256_hex(&token), jiff::Timestamp::now())
                .await?
                .ok_or_else(|| ApiError::unauthorized("session expired or revoked"))?;
        Ok(AuthUser(user))
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if user.role != "admin" {
            return Err(ApiError::forbidden("an issuer (admin) account is required"));
        }
        Ok(AdminUser(user))
    }
}

impl FromRequestParts<AppState> for MerchantUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        let merchant_id = match (user.role.as_str(), user.merchant_id) {
            ("merchant", Some(id)) => id,
            _ => return Err(ApiError::forbidden("a merchant account is required")),
        };
        let store_id = user.store_id;
        ensure_merchant_active(state, merchant_id).await?;
        Ok(MerchantUser {
            user,
            merchant_id,
            store_id,
        })
    }
}

impl FromRequestParts<AppState> for AuthedMerchant {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // Machine credential first: the terminal always sends a bearer API key.
        if let Some(token) = bearer_token(parts) {
            let key_hash = sha256_hex(&token);
            let merchant = match melon_db::ops::merchant_by_key_hash(&state.pool, &key_hash).await?
            {
                Some(m) => m,
                None => {
                    // The key itself must never reach the log. The first bytes of
                    // its *hash* are enough to tell repeated attempts with one bad
                    // key apart from a scan through many, and reveal nothing.
                    crate::security!(
                        key_hash_prefix = &key_hash[..8],
                        "api key rejected: unknown key"
                    );
                    return Err(ApiError::unauthorized("invalid api key"));
                }
            };
            if merchant.status != "active" {
                crate::security!(
                    merchant_id = %merchant.merchant_id,
                    key_id = %merchant.key_id,
                    status = %merchant.status,
                    "api key rejected: merchant is not active"
                );
                return Err(ApiError::forbidden("merchant is not active"));
            }
            return Ok(AuthedMerchant {
                merchant_id: merchant.merchant_id,
                store_id: merchant.store_id,
                actor: Actor::ApiKey(merchant.key_id),
            });
        }
        // Otherwise a signed-in merchant staff user (the portal).
        let MerchantUser {
            user,
            merchant_id,
            store_id,
        } = MerchantUser::from_request_parts(parts, state).await?;
        Ok(AuthedMerchant {
            merchant_id,
            store_id,
            actor: Actor::User(user.id),
        })
    }
}

async fn ensure_merchant_active(state: &AppState, merchant_id: Uuid) -> Result<(), ApiError> {
    let merchant = melon_db::ops::get_merchant(&state.pool, merchant_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("merchant not found"))?;
    if merchant.status != "active" {
        return Err(ApiError::forbidden("merchant is not active"));
    }
    Ok(())
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

/// Hex-encoded SHA-256 of a secret (the stored form of an API key / session token).
pub fn sha256_hex(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Generate a fresh high-entropy secret (64 hex chars) — API keys and session tokens.
pub fn new_secret() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hashing_roundtrips_and_rejects_wrong_passwords() {
        let phc = hash_password("correct horse battery").unwrap();
        assert!(phc.starts_with("$argon2id$"), "must be Argon2id: {phc}");
        assert!(verify_password("correct horse battery", &phc));
        assert!(!verify_password("wrong password", &phc));
        // A per-password salt means two hashes of the same password differ.
        assert_ne!(phc, hash_password("correct horse battery").unwrap());
        // Garbage never verifies.
        assert!(!verify_password("x", "not-a-phc-string"));
    }

    #[test]
    fn short_passwords_are_rejected() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("a-long-enough-password").is_ok());
    }

    #[test]
    fn session_cookie_is_httponly_and_samesite_strict() {
        let c = session_cookie("abc", Duration::from_secs(3600), true);
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Secure"));
        assert!(c.contains("Max-Age=3600"));
        // Without TLS the Secure flag must be omitted, or the cookie is dropped.
        assert!(!session_cookie("abc", Duration::from_secs(1), false).contains("Secure"));
        assert!(clear_session_cookie(false).contains("Max-Age=0"));
    }
}
