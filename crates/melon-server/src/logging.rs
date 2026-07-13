//! Logging: the subscriber, the per-request span, and the three log streams.
//!
//! Everything goes to **stdout** (the container runtime collects it). What
//! separates the streams is the `target`, so an operator can filter or route them
//! with `RUST_LOG` without us having to ship logs anywhere ourselves:
//!
//! * [`AUDIT`] — one line per money movement or privileged change. This is the
//!   stream that matters for a payment system: the ledger is the source of truth,
//!   but the audit log is how you answer "who charged what, when" without a
//!   database. Idempotent replays are logged with `replayed = true`, which is what
//!   lets you prove a retried request was only booked once.
//! * [`SECURITY`] — sign-in attempts, rejected credentials, denied authorization.
//! * [`ACCESS`] — one line per request: method, path, status, latency.
//!
//! # What must never be logged
//!
//! FeliCa keys, merchant API key secrets (log the key **id**, or the first bytes
//! of the key's *hash* when the key is unknown), session tokens, passwords or
//! their hashes, the Turnstile secret and its tokens, and the raw mutual
//! authentication frames.
//!
//! The card identity `(system_code, idm, idi)` is **also** kept out by default:
//! every audit line carries a `transaction_id`, and the account is reachable from
//! there in the database, so nothing is lost by leaving personal data out of a
//! log stream that gets shipped around and retained. `MELON_LOG_CARD_IDS=true`
//! puts it back at DEBUG for field debugging.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::AppState;

/// Money movements and privileged changes.
pub const AUDIT: &str = "melon::audit";
/// Authentication and authorization outcomes.
pub const SECURITY: &str = "melon::security";
/// One line per HTTP request.
pub const ACCESS: &str = "melon::access";
/// The per-request **span**, which emits no lines of its own — it only attaches
/// `request_id`, `method` and `path` to every other line. It has its own target so
/// that a filter which quiets the access log (`melon::access=warn`) does not also
/// switch off the correlation id on the audit lines it kept. Keep it enabled:
/// `melon::request=info` costs no output.
pub const REQUEST: &str = "melon::request";

/// Log one money movement or privileged change to the [`AUDIT`] stream.
#[macro_export]
macro_rules! audit {
    ($($arg:tt)*) => { ::tracing::info!(target: $crate::logging::AUDIT, $($arg)*) };
}

/// Log a rejected credential or a denied authorization to the [`SECURITY`] stream.
#[macro_export]
macro_rules! security {
    ($($arg:tt)*) => { ::tracing::warn!(target: $crate::logging::SECURITY, $($arg)*) };
}

/// Log an accepted credential (sign-in, key rotation) to the [`SECURITY`] stream.
#[macro_export]
macro_rules! security_info {
    ($($arg:tt)*) => { ::tracing::info!(target: $crate::logging::SECURITY, $($arg)*) };
}

/// Header carrying the request id, in and out.
const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The request id, readable by handlers through the request extensions.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

/// Output format for the log stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable, for a terminal.
    Text,
    /// One JSON object per line, for a log pipeline.
    Json,
}

impl LogFormat {
    /// From `MELON_LOG_FORMAT` (`text` | `json`), defaulting to text.
    pub fn from_env() -> Self {
        match std::env::var("MELON_LOG_FORMAT").as_deref() {
            Ok("json") | Ok("JSON") => LogFormat::Json,
            _ => LogFormat::Text,
        }
    }
}

/// Install the global subscriber.
///
/// Called before [`crate::Config::from_env`] — a configuration error has to be
/// loggable — so this reads its own two variables straight from the environment
/// rather than taking a `Config`.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match LogFormat::from_env() {
        // `with_target(false)` would hide exactly the thing that distinguishes the
        // audit and security streams, so the target stays on in both formats.
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder.json().flatten_event(true).init(),
    }
}

/// The caller's IP.
///
/// melon is reached through **cloudflared**, so the socket peer is always the
/// tunnel: the real client only exists in a header. A header is trivially forged
/// by anyone who can reach the server directly, so it is trusted **only** when
/// `MELON_TRUST_PROXY=true` says a proxy we control is in front of us — otherwise
/// an attacker could pin their failed sign-ins on someone else's IP.
pub fn client_ip(headers: &HeaderMap, trust_proxy: bool) -> Option<String> {
    if !trust_proxy {
        return None;
    }
    headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        // X-Forwarded-For is a chain; the client is the first entry.
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Per-request span + access log.
///
/// Every log line emitted while handling a request — audit, security, error —
/// inherits `request_id`, `method` and `path` from the span this opens, so one id
/// pulls up everything that happened for a single call. An inbound `X-Request-Id`
/// is adopted (a proxy or the caller may already have one) and always echoed back.
///
/// This must be the **outermost** layer, so the status it logs is the one the
/// client actually receives.
pub async fn request_context(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let request_id = req
        .headers()
        .get(REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    let method = req.method().clone();
    // The path only — a query string is not a place we control, and we would
    // rather never find a credential in the log because someone put one there.
    let path = req.uri().path().to_string();
    let ip = client_ip(req.headers(), state.trust_proxy);

    let span = tracing::info_span!(
        target: REQUEST,
        "request",
        request_id = %request_id,
        method = %method,
        path = %path,
        ip = ip.as_deref().unwrap_or("-"),
    );
    req.extensions_mut().insert(RequestId(request_id.clone()));

    let started = Instant::now();
    let mut response = next.run(req).instrument(span.clone()).await;
    let status = response.status();
    let latency_ms = started.elapsed().as_millis() as u64;

    span.in_scope(|| {
        // The health probe fires every few seconds forever; at INFO it would be
        // most of the log.
        if path == "/healthz" {
            tracing::debug!(target: ACCESS, status = status.as_u16(), latency_ms, "request");
        } else if status.is_server_error() {
            tracing::warn!(target: ACCESS, status = status.as_u16(), latency_ms, "request");
        } else {
            tracing::info!(target: ACCESS, status = status.as_u16(), latency_ms, "request");
        }
    });

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ip_is_ignored_unless_a_proxy_is_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(client_ip(&headers, false), None);
        assert_eq!(client_ip(&headers, true).as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn forwarded_for_takes_the_first_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 70.41.3.18"),
        );
        assert_eq!(client_ip(&headers, true).as_deref(), Some("203.0.113.7"));
    }
}
