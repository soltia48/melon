//! melon HTTP server: online FeliCa mutual authentication + prepaid payment API.
//!
//! The server embeds the [`melon_auth`] crypto oracle (holding the FeliCa keys)
//! and the [`melon_db`] money engine over PostgreSQL. A money operation is only
//! accepted against a freshly card-authenticated session, so every charge maps
//! to a real physical tap and a server-verified IDi.

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod logging;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post};
use jiff::tz::TimeZone;

use melon_auth::SessionManager;
use melon_db::Pool;

pub use config::Config;

/// This is a pure JSON API (the front-end is a separate app — see `web/`), so it
/// never returns a document that could load a script, style or image. Lock the
/// policy all the way down to `'none'`.
const CSP: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'";

/// Security headers on every response.
///
/// melon is exposed through **cloudflared**, not a reverse proxy, so there is no
/// proxy layer left to add these — the application must set them itself.
/// `Strict-Transport-Security` is only sent when cookies are `Secure` (i.e. we
/// are really behind TLS); pinning HSTS onto a plain-HTTP dev host would lock a
/// developer's browser out of `http://localhost`.
async fn security_headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    if state.cookie_secure {
        headers.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    response
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub manager: Arc<SessionManager>,
    /// JST, used for all 6-month expiry math.
    pub tz: TimeZone,
    /// How long a signed-in user's session cookie lives.
    pub user_session_ttl: Duration,
    /// Whether to mark the session cookie `Secure` (off for loopback HTTP dev).
    pub cookie_secure: bool,
    /// Default payment fee (bps) for newly created merchants.
    pub default_fee_bps: i32,
    /// Default credit limit (yen) for newly created merchants.
    pub default_credit_limit: i64,
    /// Cloudflare Turnstile for login bot-protection; `None` disables the
    /// challenge (login proceeds without it).
    pub turnstile: Option<Turnstile>,
    /// Trust the proxy's client-IP headers (see [`logging::client_ip`]).
    pub trust_proxy: bool,
    /// Log the card identity at DEBUG (off by default; see [`logging`]).
    pub log_card_ids: bool,
}

/// Cloudflare Turnstile configuration for the sign-in form. Present only when a
/// site key **and** secret are configured.
#[derive(Clone)]
pub struct Turnstile {
    /// Public site key handed to the browser to render the widget.
    pub site_key: String,
    /// Secret key used server-side to verify tokens — never sent to the browser.
    pub secret: String,
    /// Shared client for the Cloudflare `siteverify` call.
    pub http: reqwest::Client,
}

impl Turnstile {
    /// Build a Turnstile config with its own HTTP client.
    pub fn new(site_key: String, secret: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build reqwest client for Turnstile");
        Turnstile {
            site_key,
            secret,
            http,
        }
    }
}

/// Build the API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(handlers::healthz))
        // --- human sign-on ---
        .route("/v1/auth/config", get(handlers::auth_config))
        .route("/v1/auth/login", post(handlers::login))
        .route("/v1/auth/logout", post(handlers::logout))
        .route("/v1/auth/me", get(handlers::auth_me))
        .route("/v1/auth/password", post(handlers::change_password))
        // --- merchant staff management (a merchant manages its own users) ---
        .route(
            "/v1/users",
            get(handlers::list_merchant_users).post(handlers::create_merchant_user),
        )
        .route(
            "/v1/users/{user_id}/status",
            post(handlers::set_merchant_user_status),
        )
        // --- issuer user management ---
        .route(
            "/v1/admin/users",
            get(handlers::admin_list_users).post(handlers::admin_create_user),
        )
        .route(
            "/v1/admin/users/{user_id}/status",
            post(handlers::admin_set_user_status),
        )
        .route(
            "/v1/admin/users/{user_id}/password",
            post(handlers::admin_reset_password),
        )
        .route("/v1/me", get(handlers::me))
        .route("/v1/system-codes", get(handlers::system_codes))
        .route(
            "/v1/mutual-authentication",
            post(handlers::mutual_authentication),
        )
        .route("/v1/balance", post(handlers::balance))
        // Unauthenticated self-service balance, keyed on the semi-public IDm.
        .route("/v1/self/balance", post(handlers::self_balance))
        .route("/v1/topups", post(handlers::topup))
        .route("/v1/payments", post(handlers::pay))
        .route("/v1/refunds", post(handlers::refund))
        .route("/v1/payments/{payment_id}/void", post(handlers::void))
        .route("/v1/payments/refundable", get(handlers::refundable))
        .route("/v1/transactions", get(handlers::transactions))
        // Merchant-managed stores (read) and store-scoped API keys.
        .route("/v1/stores", get(handlers::merchant_list_stores))
        .route(
            "/v1/stores/{store_id}/api-keys",
            get(handlers::merchant_list_api_keys).post(handlers::merchant_create_api_key),
        )
        .route(
            "/v1/stores/{store_id}/api-keys/{key_id}",
            delete(handlers::merchant_revoke_api_key),
        )
        .route(
            "/v1/merchants",
            get(handlers::list_merchants).post(handlers::create_merchant),
        )
        // Admin-managed stores under a merchant.
        .route(
            "/v1/admin/merchants/{merchant_id}/stores",
            get(handlers::admin_list_stores).post(handlers::admin_create_store),
        )
        .route(
            "/v1/admin/stores/{store_id}",
            patch(handlers::admin_update_store),
        )
        .route(
            "/v1/admin/stores/{store_id}/status",
            post(handlers::admin_set_store_status),
        )
        .route(
            "/v1/admin/merchants/{merchant_id}/status",
            post(handlers::set_merchant_status),
        )
        .route(
            "/v1/admin/merchants/{merchant_id}/fee",
            post(handlers::set_merchant_fee),
        )
        .route(
            "/v1/admin/merchants/{merchant_id}/credit-limit",
            post(handlers::set_merchant_credit_limit),
        )
        .route(
            "/v1/admin/merchants/{merchant_id}/api-keys",
            post(handlers::rotate_api_key),
        )
        .route(
            "/v1/admin/merchants/{merchant_id}/adjust",
            post(handlers::adjust_merchant),
        )
        .route("/v1/admin/accounts", get(handlers::list_accounts))
        .route("/v1/admin/transactions", get(handlers::admin_transactions))
        .route("/v1/admin/refundable", get(handlers::admin_refundable))
        .route("/v1/admin/refunds", post(handlers::admin_refund))
        .route(
            "/v1/admin/payments/{payment_id}/void",
            post(handlers::admin_void),
        )
        .route(
            "/v1/admin/accounts/{system_code}/{idm}/{idi}/balance",
            get(handlers::admin_account_balance),
        )
        .route(
            "/v1/admin/accounts/{system_code}/{idm}/{idi}/adjust",
            post(handlers::adjust_account),
        )
        .route("/v1/admin/expiry/sweep", post(handlers::sweep))
        .route(
            "/v1/admin/reports/outstanding-balance",
            get(handlers::outstanding),
        )
        .route("/v1/admin/issuer/balance", get(handlers::issuer_balance))
        .route("/v1/admin/issuer/adjust", post(handlers::adjust_issuer))
        .route(
            "/v1/admin/issuer/adjustments",
            get(handlers::issuer_adjustments),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        // Outermost, so the status it logs is the one the client receives — and so
        // the request span wraps every other layer's logs, including rejections.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            logging::request_context,
        ))
        .with_state(state)
}

/// Drop expired user sessions periodically. Expiry is already enforced on every
/// request; this only keeps the table from growing without bound.
pub fn spawn_session_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match melon_db::users::purge_expired_sessions(&state.pool, jiff::Timestamp::now()).await
            {
                Ok(n) if n > 0 => tracing::info!(purged = n, "expired user sessions removed"),
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "failed to purge expired user sessions"),
            }
        }
    });
}

/// Spawn the background expiry sweeper. Because the sweep takes a Postgres
/// advisory lock, running it on every replica is safe — only one wins each tick.
pub fn spawn_expiry_sweeper(state: AppState, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match melon_db::ops::expire_due(&state.pool, jiff::Timestamp::now(), 500).await {
                Ok(o) if o.ran && o.expired_buckets > 0 => tracing::info!(
                    buckets = o.expired_buckets,
                    amount = o.expired_amount.as_i64(),
                    "expiry sweep forfeited buckets"
                ),
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "expiry sweep failed"),
            }
        }
    });
}
