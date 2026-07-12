//! Server configuration from the environment.

use std::time::Duration;

/// Runtime configuration assembled from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: String,
    pub keys_path: String,
    pub sweep_interval: Duration,
    /// TTL of a FeliCa mutual-authentication session (card tap → money op).
    pub session_ttl: Duration,
    pub max_sessions: usize,
    /// TTL of a signed-in **user** session (the HttpOnly cookie).
    pub user_session_ttl: Duration,
    /// Set the `Secure` flag on the session cookie. Defaults to `true` unless the
    /// server binds to loopback (plain-HTTP local development, where a `Secure`
    /// cookie would simply be dropped by the browser).
    pub cookie_secure: bool,
    /// First-run bootstrap: create this admin user if no admin exists yet.
    pub bootstrap_admin: Option<(String, String)>,
    /// Default payment fee rate (basis points) applied to new merchants.
    pub default_fee_bps: i32,
    /// Default credit limit (yen) applied to new merchants.
    pub default_credit_limit: i64,
}

impl Config {
    /// Read configuration from the environment. Only `DATABASE_URL` is required.
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;
        let bind_addr = env_or("MELON_BIND", "127.0.0.1:8080");

        // Secure cookies are the default; loopback dev over plain HTTP opts out
        // automatically (a Secure cookie would never be stored there).
        let cookie_secure = match std::env::var("MELON_COOKIE_SECURE") {
            Ok(v) => matches!(v.trim(), "1" | "true" | "TRUE" | "yes"),
            Err(_) => !is_loopback(&bind_addr),
        };

        let bootstrap_admin = match (
            std::env::var("MELON_BOOTSTRAP_ADMIN_EMAIL"),
            std::env::var("MELON_BOOTSTRAP_ADMIN_PASSWORD"),
        ) {
            (Ok(email), Ok(password)) if !email.is_empty() && !password.is_empty() => {
                Some((email, password))
            }
            _ => None,
        };

        Ok(Self {
            database_url,
            bind_addr,
            keys_path: env_or("MELON_KEYS", "keys.jsonl"),
            sweep_interval: Duration::from_secs(parse_u64("MELON_SWEEP_INTERVAL_SECS", 3600)),
            session_ttl: Duration::from_secs(parse_u64("FELICA_SESSION_TTL", 300)),
            max_sessions: parse_u64("FELICA_MAX_SESSIONS", 1024) as usize,
            user_session_ttl: Duration::from_secs(parse_u64("MELON_USER_SESSION_TTL", 12 * 3600)),
            cookie_secure,
            bootstrap_admin,
            default_fee_bps: parse_u64("MELON_DEFAULT_FEE_BPS", 0).min(10000) as i32,
            default_credit_limit: parse_u64("MELON_DEFAULT_CREDIT_LIMIT", 0) as i64,
        })
    }
}

fn is_loopback(bind_addr: &str) -> bool {
    let host = bind_addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind_addr);
    matches!(host.trim_matches(['[', ']']), "127.0.0.1" | "::1" | "localhost")
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
