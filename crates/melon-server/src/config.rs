//! Server configuration from the environment.
//!
//! `MELON_LOG_FORMAT` and `RUST_LOG` are deliberately **not** here: the subscriber
//! has to be installed before this module runs, or a configuration error would
//! have nowhere to go. See [`crate::logging::init`].

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
    /// Cloudflare Turnstile `(site_key, secret)` for login bot-protection.
    /// Enabled only when both are provided.
    pub turnstile: Option<(String, String)>,
    /// Trust `CF-Connecting-IP` / `X-Forwarded-For` for the client's IP. Only
    /// enable when a proxy we control (cloudflared) really is in front — the
    /// headers are forgeable by anyone who can reach the server directly.
    pub trust_proxy: bool,
    /// Include the card identity `(system_code, idm, idi)` in DEBUG logs. Off by
    /// default: `transaction_id` is enough to reach the account in the database,
    /// so the log stream stays free of personal data.
    pub log_card_ids: bool,
}

impl Config {
    /// Read configuration from the environment. Only `DATABASE_URL` is required.
    ///
    /// Secrets may be supplied out-of-band as `<VAR>_FILE` pointing at a file
    /// (Docker/Kubernetes secrets), so they never appear in the process
    /// environment where `docker inspect` or `/proc/<pid>/environ` would expose
    /// them.
    pub fn from_env() -> Result<Self, String> {
        let database_url = env_secret("DATABASE_URL")
            .ok_or_else(|| "DATABASE_URL (or DATABASE_URL_FILE) is required".to_string())?;
        let bind_addr = env_or("MELON_BIND", "127.0.0.1:8080");

        // Secure cookies are the default; loopback dev over plain HTTP opts out
        // automatically (a Secure cookie would never be stored there).
        let cookie_secure = match std::env::var("MELON_COOKIE_SECURE") {
            Ok(v) => matches!(v.trim(), "1" | "true" | "TRUE" | "yes"),
            Err(_) => !is_loopback(&bind_addr),
        };

        let bootstrap_admin = match (
            env_secret("MELON_BOOTSTRAP_ADMIN_EMAIL"),
            env_secret("MELON_BOOTSTRAP_ADMIN_PASSWORD"),
        ) {
            (Some(email), Some(password)) => Some((email, password)),
            _ => None,
        };

        // Turnstile is enabled only when BOTH the public site key and the secret
        // are present (the frontend needs the site key to render the widget, the
        // server needs the secret to verify tokens).
        let turnstile = match (
            std::env::var("MELON_TURNSTILE_SITE_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            env_secret("MELON_TURNSTILE_SECRET"),
        ) {
            (Some(site_key), Some(secret)) => Some((site_key, secret)),
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
            turnstile,
            trust_proxy: parse_bool("MELON_TRUST_PROXY"),
            log_card_ids: parse_bool("MELON_LOG_CARD_IDS"),
        })
    }
}

fn is_loopback(bind_addr: &str) -> bool {
    let host = bind_addr
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(bind_addr);
    matches!(
        host.trim_matches(['[', ']']),
        "127.0.0.1" | "::1" | "localhost"
    )
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// A secret from `KEY`, or from the file named by `KEY_FILE` (Docker/K8s secrets).
/// The file's trailing newline is trimmed. Empty values count as absent.
fn env_secret(key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key)
        && !value.is_empty()
    {
        return Some(value);
    }
    let path = std::env::var(format!("{key}_FILE")).ok()?;
    let value = std::fs::read_to_string(&path)
        .map_err(|e| tracing::error!(%path, error = %e, "failed to read secret file"))
        .ok()?;
    Some(value.trim().to_string()).filter(|v| !v.is_empty())
}

/// An opt-in flag: absent or anything unrecognized means off.
fn parse_bool(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false)
}

fn parse_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
