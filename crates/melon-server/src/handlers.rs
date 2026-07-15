//! HTTP handlers for the melon API.
//!
//! Money operations bind to a **card-verified** IDi: the terminal completes
//! mutual authentication (`/v1/mutual-authentication`), then references the live
//! session. Top-up and payment claim the session's one-shot spend capability
//! (`consume_spend`), so each money movement corresponds to a fresh physical tap
//! and a server-verified IDi that no merchant can forge.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use melon_core::account::AccountKey;
use melon_core::idi::Idi;
use melon_core::idm::Idm;
use melon_core::money::PositiveYen;
use melon_db::ops;

use crate::AppState;
use crate::auth::{AdminUser, AuthUser, AuthedMerchant, MerchantUser};
use crate::error::ApiError;

// ----- helpers -----

fn now() -> Timestamp {
    Timestamp::now()
}

/// The card identity — logged at DEBUG, and only when `MELON_LOG_CARD_IDS` asks
/// for it. The audit stream identifies a movement by `transaction_id`, from which
/// the account is one query away, so the default is to keep personal data out of
/// the log entirely. See [`crate::logging`].
fn log_card(state: &AppState, account: AccountKey) {
    if state.log_card_ids {
        tracing::debug!(
            system_code = format_args!("0x{:04x}", account.system_code),
            idm = %account.idm.to_hex(),
            idi = %account.idi.to_hex(),
            "card identity"
        );
    }
}

fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request("Idempotency-Key header is required"))
}

fn positive(amount: i64) -> Result<PositiveYen, ApiError> {
    PositiveYen::from_i64(amount)
        .map_err(|_| ApiError::bad_request("amount must be a positive integer number of yen"))
}

fn parse_ts(s: &str) -> Result<Timestamp, ApiError> {
    s.parse()
        .map_err(|_| ApiError::bad_request("invalid timestamp"))
}

fn parse_idi(s: &str) -> Result<Idi, ApiError> {
    s.trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid idi (expected 16 hex characters)"))
}

fn parse_idm(s: &str) -> Result<Idm, ApiError> {
    s.trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid idm (expected 16 hex characters)"))
}

/// Parse an optional account filter from query strings. The account key is the
/// full `(system_code, idm, idi)` triple, so either all three are supplied or
/// none are.
fn parse_account_filter(
    system_code: Option<&str>,
    idm: Option<&str>,
    idi: Option<&str>,
) -> Result<Option<AccountKey>, ApiError> {
    let sc = system_code.filter(|s| !s.is_empty());
    let idm = idm.filter(|s| !s.is_empty());
    let idi = idi.filter(|s| !s.is_empty());
    match (sc, idm, idi) {
        (Some(sc), Some(idm), Some(idi)) => Ok(Some(AccountKey::new(
            parse_sc(sc)?,
            parse_idm(idm)?,
            parse_idi(idi)?,
        ))),
        (None, None, None) => Ok(None),
        _ => Err(ApiError::bad_request(
            "filter by account requires system_code, idm and idi",
        )),
    }
}

/// Parse a system code accepting a `0x` hex prefix or decimal.
fn parse_sc(s: &str) -> Result<u16, ApiError> {
    let t = s.trim();
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16),
        None => t.parse::<u16>(),
    };
    parsed.map_err(|_| ApiError::bad_request("invalid system_code (hex 0x0003 or decimal)"))
}

// ----- human sign-on (users + HttpOnly cookie sessions) -----

#[derive(Serialize)]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    /// `admin` or `merchant`.
    pub role: String,
    pub merchant_id: Option<Uuid>,
    /// A merchant user's store scope: `null` = merchant-wide admin (all stores),
    /// otherwise the single store they are restricted to.
    pub store_id: Option<Uuid>,
    pub status: String,
    pub created_at: Timestamp,
}

fn user_view(u: melon_db::users::User) -> UserView {
    UserView {
        id: u.id,
        email: u.email,
        name: u.name,
        role: u.role,
        merchant_id: u.merchant_id,
        store_id: u.store_id,
        status: u.status,
        created_at: u.created_at,
    }
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
    /// Cloudflare Turnstile token (`cf-turnstile-response`), required when
    /// Turnstile is configured.
    pub turnstile_token: Option<String>,
}

/// `GET /v1/auth/config`: unauthenticated hints the sign-in page needs before a
/// user logs in (currently just the Turnstile site key, if enabled).
#[derive(Serialize)]
pub struct AuthConfigResp {
    /// Public Turnstile site key, or `null` when the challenge is disabled.
    pub turnstile_site_key: Option<String>,
}

pub async fn auth_config(State(state): State<AppState>) -> Json<AuthConfigResp> {
    Json(AuthConfigResp {
        turnstile_site_key: state.turnstile.as_ref().map(|t| t.site_key.clone()),
    })
}

/// Verify a Turnstile token against Cloudflare's siteverify. Returns an error the
/// sign-in handler surfaces (and the frontend resets the widget on).
async fn verify_turnstile(
    ts: &crate::Turnstile,
    token: &str,
    remote_ip: Option<&str>,
) -> Result<(), ApiError> {
    if token.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "TURNSTILE_REQUIRED",
            "bot verification is required",
        ));
    }
    #[derive(Deserialize)]
    struct SiteVerify {
        success: bool,
    }
    let mut form = vec![("secret", ts.secret.as_str()), ("response", token)];
    if let Some(ip) = remote_ip {
        form.push(("remoteip", ip));
    }
    let outcome = ts
        .http
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&form)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    let outcome = match outcome {
        Ok(resp) => resp.json::<SiteVerify>().await,
        Err(e) => Err(e),
    };
    match outcome {
        Ok(v) if v.success => Ok(()),
        Ok(_) => {
            crate::security!("turnstile rejected the sign-in attempt");
            Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "TURNSTILE_FAILED",
                "bot verification failed — please try again",
            ))
        }
        Err(e) => {
            // Cloudflare being unreachable makes sign-in impossible; that needs to
            // be loud and distinguishable from a user failing the challenge.
            tracing::error!(error = %e, "turnstile siteverify is unavailable");
            Err(ApiError::internal("bot verification is unavailable"))
        }
    }
}

/// Sign in. On success the session token is delivered **only** as an HttpOnly
/// cookie — it is never in the response body, so JavaScript can't read (or leak) it.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginReq>,
) -> Result<Response, ApiError> {
    let email = req.email.trim();
    let ip = crate::logging::client_ip(&headers, state.trust_proxy);

    // Gate the whole attempt behind Turnstile (when configured) before touching
    // the database, so login can't be used as a bot-driven password oracle.
    if let Some(ts) = &state.turnstile {
        let token = req.turnstile_token.as_deref().unwrap_or("");
        verify_turnstile(ts, token, ip.as_deref()).await?;
    }

    let found = melon_db::users::user_for_login(&state.pool, email).await?;

    // Verify a hash even when the user is unknown, so a wrong email and a wrong
    // password take the same time (no user enumeration by timing).
    let (user, hash) = match found {
        Some((user, hash)) => (Some(user), hash),
        None => (None, DUMMY_HASH.to_string()),
    };
    let password_ok = crate::auth::verify_password(&req.password, &hash);
    let user = match (user, password_ok) {
        (Some(u), true) if u.status == "active" => u,
        // The response deliberately cannot tell these apart (no user enumeration),
        // but the log must: a locked-out account and a brute-force sweep look the
        // same from outside and need different responses from us.
        (user, ok) => {
            let reason = match (&user, ok) {
                (None, _) => "unknown email",
                (Some(_), false) => "wrong password",
                (Some(_), true) => "account is not active",
            };
            crate::security!(email, reason, "sign-in failed");
            return Err(ApiError::unauthorized("invalid email or password"));
        }
    };
    crate::security_info!(
        user_id = %user.id,
        email = %user.email,
        role = %user.role,
        "signed in"
    );

    let token = crate::auth::new_secret();
    let expires_at =
        Timestamp::now() + jiff::Span::new().seconds(state.user_session_ttl.as_secs() as i64);
    melon_db::users::create_session(
        &state.pool,
        &crate::auth::sha256_hex(&token),
        user.id,
        expires_at,
    )
    .await?;

    let cookie = crate::auth::session_cookie(&token, state.user_session_ttl, state.cookie_secure);
    let mut response = Json(serde_json::json!({ "user": user_view(user) })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        cookie
            .parse()
            .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok(response)
}

/// An Argon2id hash of a random string, used to equalize timing for unknown emails.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$\
                          Zm9yY2VzIGEgdmVyaWZ5IHRvIHJ1biBhbmQgZmFpbA";

/// Sign out: revoke the server-side session and clear the cookie.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(token) = cookie_value(&headers, crate::auth::SESSION_COOKIE) {
        melon_db::users::delete_session(&state.pool, &crate::auth::sha256_hex(&token)).await?;
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        crate::auth::clear_session_cookie(state.cookie_secure)
            .parse()
            .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok(response)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// The signed-in user (who am I / am I still signed in).
pub async fn auth_me(AuthUser(user): AuthUser) -> Json<UserView> {
    Json(user_view(user))
}

#[derive(Deserialize)]
pub struct ChangePasswordReq {
    pub current_password: String,
    pub new_password: String,
}

/// Change one's own password. Verifies the current password and revokes every
/// existing session (including this one) so all devices must sign in again.
pub async fn change_password(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<ChangePasswordReq>,
) -> Result<Response, ApiError> {
    let current = melon_db::users::password_hash(&state.pool, user.id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("user not found"))?;
    if !crate::auth::verify_password(&req.current_password, &current) {
        crate::security!(user_id = %user.id, "password change refused: wrong current password");
        return Err(ApiError::unauthorized("current password is incorrect"));
    }
    crate::auth::validate_password(&req.new_password)?;
    let hash = crate::auth::hash_password(&req.new_password)?;
    melon_db::users::set_password_hash(&state.pool, user.id, &hash).await?;
    crate::security_info!(user_id = %user.id, "password changed");

    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        crate::auth::clear_session_cookie(state.cookie_secure)
            .parse()
            .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok(response)
}

// ----- user management -----

#[derive(Deserialize)]
pub struct CreateUserReq {
    pub email: String,
    pub name: String,
    pub password: String,
    /// Admin only: `admin` or `merchant`. Ignored for merchant-created staff.
    pub role: Option<String>,
    /// Admin only: required when `role == "merchant"`.
    pub merchant_id: Option<Uuid>,
    /// Optional store scope for a merchant user. `null` = merchant-wide admin
    /// (all stores); otherwise the store this user is restricted to (must belong
    /// to the user's merchant).
    pub store_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct UserStatusReq {
    pub status: String,
}

fn validate_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "active" | "disabled") {
        Ok(())
    } else {
        Err(ApiError::bad_request("status must be active or disabled"))
    }
}

/// Issuer: list every user.
pub async fn admin_list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserView>>, ApiError> {
    let users = melon_db::users::list_users(&state.pool, None).await?;
    Ok(Json(users.into_iter().map(user_view).collect()))
}

/// Issuer: create an admin user, or a merchant staff user for any merchant.
pub async fn admin_create_user(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let role = req.role.as_deref().unwrap_or("merchant");
    let merchant_id =
        match role {
            "admin" => None,
            "merchant" => Some(req.merchant_id.ok_or_else(|| {
                ApiError::bad_request("merchant_id is required for a merchant user")
            })?),
            _ => return Err(ApiError::bad_request("role must be admin or merchant")),
        };
    // A store scope is only meaningful for a merchant user.
    let store_id = if role == "admin" { None } else { req.store_id };
    let user = create_user_checked(&state, &req, role, merchant_id, store_id, admin.id).await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// Issuer: enable/disable any user (disabling revokes their sessions).
pub async fn admin_set_user_status(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(user_id): Path<Uuid>,
    Json(req): Json<UserStatusReq>,
) -> Result<Json<Value>, ApiError> {
    validate_status(&req.status)?;
    if admin.id == user_id && req.status != "active" {
        return Err(ApiError::bad_request("you cannot disable your own account"));
    }
    melon_db::users::set_user_status(&state.pool, user_id, &req.status).await?;
    crate::security_info!(
        user_id = %user_id,
        status = %req.status,
        actor_id = %admin.id,
        "user status changed"
    );
    Ok(Json(
        serde_json::json!({ "user_id": user_id, "status": req.status }),
    ))
}

#[derive(Deserialize)]
pub struct ResetPasswordReq {
    pub new_password: String,
}

/// Issuer: reset any user's password (revokes their sessions).
pub async fn admin_reset_password(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(user_id): Path<Uuid>,
    Json(req): Json<ResetPasswordReq>,
) -> Result<Json<Value>, ApiError> {
    crate::auth::validate_password(&req.new_password)?;
    let hash = crate::auth::hash_password(&req.new_password)?;
    melon_db::users::set_password_hash(&state.pool, user_id, &hash).await?;
    crate::security_info!(
        user_id = %user_id,
        actor_id = %admin.id,
        "password reset by an issuer admin (sessions revoked)"
    );
    Ok(Json(serde_json::json!({ "user_id": user_id, "ok": true })))
}

/// Merchant staff: list the users of one's OWN merchant.
pub async fn list_merchant_users(
    State(state): State<AppState>,
    caller: MerchantUser,
) -> Result<Json<Vec<UserView>>, ApiError> {
    require_merchant_admin(&caller)?;
    let users = melon_db::users::list_users(&state.pool, Some(caller.merchant_id)).await?;
    Ok(Json(users.into_iter().map(user_view).collect()))
}

/// Merchant staff: add a user to one's OWN merchant. The role and merchant are
/// forced from the caller — a merchant can never create an admin or reach another
/// merchant.
pub async fn create_merchant_user(
    State(state): State<AppState>,
    caller: MerchantUser,
    Json(req): Json<CreateUserReq>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    require_merchant_admin(&caller)?;
    // The new user's store scope (if any) must belong to the caller's merchant —
    // validated inside create_user_checked.
    let user = create_user_checked(
        &state,
        &req,
        "merchant",
        Some(caller.merchant_id),
        req.store_id,
        caller.user.id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// Merchant staff: enable/disable a user of one's OWN merchant.
pub async fn set_merchant_user_status(
    State(state): State<AppState>,
    caller: MerchantUser,
    Path(user_id): Path<Uuid>,
    Json(req): Json<UserStatusReq>,
) -> Result<Json<Value>, ApiError> {
    require_merchant_admin(&caller)?;
    validate_status(&req.status)?;
    if caller.user.id == user_id && req.status != "active" {
        return Err(ApiError::bad_request("you cannot disable your own account"));
    }
    // Scope check: the target must belong to the caller's merchant.
    let target = melon_db::users::get_user(&state.pool, user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    if target.merchant_id != Some(caller.merchant_id) {
        return Err(ApiError::not_found("user not found"));
    }
    melon_db::users::set_user_status(&state.pool, user_id, &req.status).await?;
    crate::security_info!(
        user_id = %user_id,
        status = %req.status,
        actor_id = %caller.user.id,
        "user status changed"
    );
    Ok(Json(
        serde_json::json!({ "user_id": user_id, "status": req.status }),
    ))
}

/// Guard: a merchant *administrator* (store-wide, `store_id == None`) is required.
/// Store-scoped users may only view their own store, not manage users or stores.
fn require_merchant_admin(caller: &MerchantUser) -> Result<(), ApiError> {
    if caller.store_id.is_some() {
        return Err(ApiError::forbidden(
            "this action requires a merchant administrator (all-stores) account",
        ));
    }
    Ok(())
}

/// Shared creation path: validate the password, hash it, map a duplicate email to 409.
async fn create_user_checked(
    state: &AppState,
    req: &CreateUserReq,
    role: &str,
    merchant_id: Option<Uuid>,
    store_id: Option<Uuid>,
    actor_id: Uuid,
) -> Result<UserView, ApiError> {
    let email = req.email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(ApiError::bad_request("a valid email is required"));
    }
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    // A store scope is only valid for a merchant user, and the store must belong
    // to that merchant.
    if let Some(sid) = store_id {
        let store = ops::get_store(&state.pool, sid)
            .await?
            .ok_or_else(|| ApiError::bad_request("store not found"))?;
        if Some(store.merchant_id) != merchant_id {
            return Err(ApiError::bad_request(
                "store does not belong to this merchant",
            ));
        }
    }
    crate::auth::validate_password(&req.password)?;
    let hash = crate::auth::hash_password(&req.password)?;

    let user = melon_db::users::create_user(
        &state.pool,
        email,
        req.name.trim(),
        &hash,
        role,
        merchant_id,
        store_id,
    )
    .await?;
    crate::security_info!(
        user_id = %user.id,
        email = %user.email,
        role,
        merchant_id = merchant_id.map(|m| m.to_string()),
        store_id = store_id.map(|s| s.to_string()),
        actor_id = %actor_id,
        "user created"
    );
    Ok(user_view(user))
}

/// The authenticated merchant's own profile and settlement balance.
/// `/v1/me`: the caller's merchant, plus the store it is scoped to (the API key's
/// store for a terminal, or a store user's store; `null` for a merchant-wide admin).
#[derive(Serialize)]
pub struct MeResp {
    #[serde(flatten)]
    merchant: MerchantView,
    store: Option<StoreView>,
}

pub async fn me(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
) -> Result<Json<MeResp>, ApiError> {
    let m = ops::get_merchant(&state.pool, merchant.merchant_id)
        .await?
        .ok_or_else(|| ApiError::not_found("merchant not found"))?;
    let store = match merchant.store_id {
        Some(sid) => ops::get_store(&state.pool, sid).await?.map(store_view),
        None => None,
    };
    Ok(Json(MeResp {
        merchant: merchant_view(m),
        store,
    }))
}

async fn verify_payment_owner(
    state: &AppState,
    payment_id: Uuid,
    merchant_id: Uuid,
) -> Result<(), ApiError> {
    match ops::payment_merchant(&state.pool, payment_id).await? {
        Some(owner) if owner == merchant_id => Ok(()),
        // Don't distinguish "not yours" from "not found".
        _ => Err(ApiError::not_found("payment not found")),
    }
}

// ----- health -----

pub async fn healthz(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::json!({ "status": "ok", "live_sessions": state.manager.live_sessions() }))
}

// ----- usable system codes (which systems the server holds keys for) -----

#[derive(Serialize)]
pub struct SystemCodesResp {
    /// FeliCa system codes the server can authenticate, ascending.
    pub system_codes: Vec<u16>,
}

/// The systems a card may be authenticated under. A terminal polls the card with
/// the wildcard system code, asks the card which systems it exposes, and picks the
/// first one that appears in this list.
pub async fn system_codes(
    State(state): State<AppState>,
    _merchant: AuthedMerchant,
) -> Json<SystemCodesResp> {
    Json(SystemCodesResp {
        system_codes: state.manager.system_codes(),
    })
}

// ----- mutual authentication (relay to the oracle) -----

/// Refuse a card whose IDm is not a stable identifier.
///
/// The account key is `(system_code, idm, idi)`, so a card that hands out a fresh
/// IDm on every tap would look like a brand-new account each time — its holder's
/// balance would simply disappear. The manufacturer code says which cards those
/// are (see [`Idm::has_stable_id`]); reject them at the door, before any
/// authentication work, rather than let one open an account it can never reach
/// again.
fn reject_unstable_idm(idm: Idm) -> Result<(), ApiError> {
    if idm.has_stable_id() {
        return Ok(());
    }
    crate::security!(
        manufacturer_code = format_args!("{:04X}h", idm.manufacturer_code()),
        "card refused: its IDm is not a stable identifier"
    );
    Err(ApiError::unprocessable(
        "UNSUPPORTED_CARD",
        "this card cannot be used: its IDm is randomized or otherwise not a stable identifier",
    ))
}

pub async fn mutual_authentication(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let input = melon_auth::http::parse_mutual_input(&body)?;
    // The IDm arrives with the first step (the terminal has just polled the card);
    // later steps only relay frames for a session that already passed this.
    if let Some(idm) = input.idm {
        reject_unstable_idm(Idm::from_bytes(idm))?;
    }
    let request_session = input.session_id.clone();
    let mut value = state.manager.handle_mutual_authentication(input).await?;

    // On completion the oracle hands back the RAW card identity (`issue_id` = IDi).
    // Merchants must never see it: swap the whole `result` for this merchant's
    // pseudonymous account id. Only the issuer can map it back.
    if value["step"] == "complete" {
        let session_id = request_session
            .or_else(|| value["session_id"].as_str().map(str::to_string))
            .ok_or_else(|| ApiError::internal("authenticated session id missing"))?;
        let (sc, idm, idi) = state
            .manager
            .authenticated_account(&session_id)
            .ok_or_else(|| ApiError::internal("authenticated session not found"))?;
        let account = AccountKey::new(sc, Idm::from_bytes(idm), Idi::from_bytes(idi));
        let account_id = ops::alias_for(&state.pool, merchant.merchant_id, account).await?;
        value["result"] = serde_json::json!({ "account_id": account_id });
    }
    Ok(Json(value))
}

/// Resolve the merchant-scoped aliases for a set of accounts (creating any that
/// are missing), so merchant-facing responses can carry pseudonyms only.
async fn alias_map(
    state: &AppState,
    merchant_id: Uuid,
    accounts: Vec<AccountKey>,
) -> Result<std::collections::HashMap<AccountKey, Uuid>, ApiError> {
    let mut map: std::collections::HashMap<AccountKey, Uuid> = std::collections::HashMap::new();
    for account in accounts {
        if let std::collections::hash_map::Entry::Vacant(slot) = map.entry(account) {
            slot.insert(ops::alias_for(&state.pool, merchant_id, account).await?);
        }
    }
    Ok(map)
}

// ----- balance (read the authenticated card's balance) -----

#[derive(Deserialize)]
pub struct BalanceReq {
    pub session_id: String,
}

#[derive(Serialize)]
pub struct BucketView {
    pub bucket_id: Uuid,
    pub remaining: i64,
    pub expires_at: Timestamp,
}

fn bucket_views(bal: ops::BalanceBreakdown) -> Vec<BucketView> {
    bal.buckets
        .into_iter()
        .map(|b| BucketView {
            bucket_id: b.bucket_id,
            remaining: b.remaining.as_i64(),
            expires_at: b.expires_at,
        })
        .collect()
}

/// Merchant-facing balance: the account is identified ONLY by this merchant's
/// pseudonymous `account_id`.
#[derive(Serialize)]
pub struct BalanceResp {
    pub account_id: Uuid,
    pub total: i64,
    pub buckets: Vec<BucketView>,
}

/// Admin-facing balance: the issuer sees the real card identity.
#[derive(Serialize)]
pub struct AdminBalanceResp {
    pub system_code: u16,
    pub idm: String,
    pub idi: String,
    pub total: i64,
    pub buckets: Vec<BucketView>,
}

fn admin_balance_resp(account: AccountKey, bal: ops::BalanceBreakdown) -> AdminBalanceResp {
    AdminBalanceResp {
        system_code: account.system_code,
        idm: account.idm.to_hex(),
        idi: account.idi.to_hex(),
        total: bal.total.as_i64(),
        buckets: bucket_views(bal),
    }
}

pub async fn balance(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    Json(req): Json<BalanceReq>,
) -> Result<Json<BalanceResp>, ApiError> {
    let (sc, idm, idi) = state
        .manager
        .authenticated_account(&req.session_id)
        .ok_or_else(|| ApiError::forbidden("session is not authenticated"))?;
    let account = AccountKey::new(sc, Idm::from_bytes(idm), Idi::from_bytes(idi));
    let account_id = ops::alias_for(&state.pool, merchant.merchant_id, account).await?;
    let bal = ops::balance(&state.pool, account, now()).await?;
    Ok(Json(BalanceResp {
        account_id,
        total: bal.total.as_i64(),
        buckets: bucket_views(bal),
    }))
}

// ----- self-service balance (unauthenticated, by the cardholder's own IDi) -----

#[derive(Deserialize)]
pub struct SelfBalanceReq {
    /// The 2-byte System Code (`0x0003` for the Suica/transit-IC family).
    pub system_code: u16,
    /// The 16-hex-character IDi. Cardholders read the string form of this — the
    /// "card ID" shown in transit-IC wallet apps (Suica, PASMO, …) — and the
    /// client converts it back to hex. Supply exactly one of `idi` / `idm`.
    pub idi: Option<String>,
    /// The 16-hex-character IDm, as read off a card over NFC. Supply exactly one
    /// of `idi` / `idm`.
    pub idm: Option<String>,
}

/// The self-service balance view. Deliberately carries ONLY the spendable total
/// and its expiry breakdown — never the raw IDi/IDm or any merchant linkage. Keep
/// the payload to what the cardholder needs and nothing that identifies them
/// further.
#[derive(Serialize)]
pub struct SelfBalanceResp {
    pub system_code: u16,
    pub total: i64,
    pub buckets: Vec<BucketView>,
}

/// Unauthenticated self-service balance: a cardholder reads the spendable balance
/// for their own card, identified by `(system_code, idi)` OR `(system_code, idm)`
/// alone. There is no mutual authentication here — the caller simply asserts an
/// IDi (its string form is the "card ID" shown in a wallet app) or an IDm (read
/// off the card over NFC) — so this is a read-only, lower-trust path than the
/// merchant/admin balance endpoints.
///
/// The identifier travels in the request body, never the URL, so it stays out of
/// the access log (card identities are not logged by default).
pub async fn self_balance(
    State(state): State<AppState>,
    Json(req): Json<SelfBalanceReq>,
) -> Result<Json<SelfBalanceResp>, ApiError> {
    let sc = req.system_code;
    let (exists, bal) = match (req.idi.as_deref(), req.idm.as_deref()) {
        (Some(idi), None) => {
            let idi = parse_idi(idi)?;
            (
                ops::account_exists_by_idi(&state.pool, sc, idi).await?,
                ops::balance_by_idi(&state.pool, sc, idi, now()).await?,
            )
        }
        (None, Some(idm)) => {
            let idm = parse_idm(idm)?;
            // A randomized/undefined IDm (the `XXFEh` block) is never a real account.
            reject_unstable_idm(idm)?;
            (
                ops::account_exists_by_idm(&state.pool, sc, idm).await?,
                ops::balance_by_idm(&state.pool, sc, idm, now()).await?,
            )
        }
        _ => {
            return Err(ApiError::bad_request(
                "provide exactly one of idi, idm",
            ));
        }
    };
    if !exists {
        return Err(ApiError::not_found("no account for this card"));
    }
    Ok(Json(SelfBalanceResp {
        system_code: sc,
        total: bal.total.as_i64(),
        buckets: bucket_views(bal),
    }))
}

// ----- top-up -----

#[derive(Deserialize)]
pub struct TopupReq {
    pub session_id: String,
    pub amount: i64,
}

#[derive(Serialize)]
pub struct TopupResp {
    pub transaction_id: Uuid,
    pub bucket_id: Uuid,
    pub amount: i64,
    pub expires_at: Timestamp,
    pub balance: i64,
    pub replayed: bool,
}

pub async fn topup(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    headers: HeaderMap,
    Json(req): Json<TopupReq>,
) -> Result<(StatusCode, Json<TopupResp>), ApiError> {
    let key = idempotency_key(&headers)?;
    let amount = positive(req.amount)?;
    let (sc, idm, idi) = state.manager.consume_spend(&req.session_id)?;
    let account = AccountKey::new(sc, Idm::from_bytes(idm), Idi::from_bytes(idi));
    let out = ops::top_up(
        &state.pool,
        account,
        Some(merchant.merchant_id),
        merchant.store_id,
        amount,
        &key,
        now(),
        &state.tz,
    )
    .await?;
    log_card(&state, account);
    crate::audit!(
        event = "top_up",
        transaction_id = %out.transaction_id,
        merchant_id = %merchant.merchant_id,
        store_id = merchant.store_id.map(|s| s.to_string()),
        actor_kind = merchant.actor.kind(),
        actor_id = %merchant.actor.id(),
        amount = out.amount.as_i64(),
        balance = out.balance.as_i64(),
        expires_at = %out.expires_at,
        replayed = out.replayed,
        "top-up issued"
    );
    Ok((
        StatusCode::CREATED,
        Json(TopupResp {
            transaction_id: out.transaction_id,
            bucket_id: out.bucket_id,
            amount: out.amount.as_i64(),
            expires_at: out.expires_at,
            balance: out.balance.as_i64(),
            replayed: out.replayed,
        }),
    ))
}

// ----- payment -----

#[derive(Deserialize)]
pub struct PayReq {
    pub session_id: String,
    pub amount: i64,
    /// Optional free-text note the merchant attaches to this payment.
    pub note: Option<String>,
}

/// Max length (characters) of a merchant-supplied transaction note.
const MAX_NOTE_LEN: usize = 200;

/// Trim a merchant note, treat blank as absent, and reject over-long input.
fn clean_note(note: Option<&str>) -> Result<Option<String>, ApiError> {
    match note.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if s.chars().count() > MAX_NOTE_LEN => Err(ApiError::bad_request(
            "note is too long (max 200 characters)",
        )),
        Some(s) => Ok(Some(s.to_string())),
    }
}

#[derive(Serialize)]
pub struct DeductionView {
    pub bucket_id: Uuid,
    pub amount: i64,
}

#[derive(Serialize)]
pub struct PayResp {
    pub transaction_id: Uuid,
    pub amount: i64,
    /// Processing fee charged to the merchant.
    pub fee: i64,
    /// Amount settled to the merchant (`amount − fee`).
    pub net: i64,
    pub balance: i64,
    pub deductions: Vec<DeductionView>,
    pub replayed: bool,
}

pub async fn pay(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    headers: HeaderMap,
    Json(req): Json<PayReq>,
) -> Result<(StatusCode, Json<PayResp>), ApiError> {
    let key = idempotency_key(&headers)?;
    let amount = positive(req.amount)?;
    let note = clean_note(req.note.as_deref())?;
    let (sc, idm, idi) = state.manager.consume_spend(&req.session_id)?;
    let account = AccountKey::new(sc, Idm::from_bytes(idm), Idi::from_bytes(idi));
    let out = ops::pay(
        &state.pool,
        account,
        merchant.merchant_id,
        merchant.store_id,
        amount,
        &key,
        note.as_deref(),
        now(),
    )
    .await?;
    log_card(&state, account);
    crate::audit!(
        event = "payment",
        transaction_id = %out.transaction_id,
        merchant_id = %merchant.merchant_id,
        store_id = merchant.store_id.map(|s| s.to_string()),
        actor_kind = merchant.actor.kind(),
        actor_id = %merchant.actor.id(),
        amount = out.amount.as_i64(),
        fee = out.fee.as_i64(),
        balance = out.balance.as_i64(),
        // A retried request that was already booked. Without this field you cannot
        // tell "charged twice" from "asked twice, charged once".
        replayed = out.replayed,
        "payment settled"
    );
    Ok((StatusCode::CREATED, Json(payment_response(out))))
}

fn payment_response(out: ops::Payment) -> PayResp {
    PayResp {
        transaction_id: out.transaction_id,
        amount: out.amount.as_i64(),
        fee: out.fee.as_i64(),
        net: out.amount.as_i64() - out.fee.as_i64(),
        balance: out.balance.as_i64(),
        deductions: out
            .deductions
            .into_iter()
            .map(|d| DeductionView {
                bucket_id: d.bucket_id,
                amount: d.amount.as_i64(),
            })
            .collect(),
        replayed: out.replayed,
    }
}

// ----- refund / void -----

#[derive(Deserialize)]
pub struct RefundReq {
    pub payment_id: Uuid,
    pub amount: Option<i64>,
}

#[derive(Serialize)]
pub struct RefundResp {
    pub transaction_id: Uuid,
    pub payment_id: Uuid,
    pub amount: i64,
    pub balance: i64,
    pub restorations: Vec<DeductionView>,
    pub replayed: bool,
}

/// Audit a refund or a void. Both restore money to the buckets the payment drew
/// from, and both are reachable by a merchant *and* by an admin, so all four paths
/// log the same shape.
fn audit_refund(
    event: &'static str,
    out: &ops::Refund,
    merchant_id: Option<Uuid>,
    actor_kind: &'static str,
    actor_id: Uuid,
) {
    crate::audit!(
        event,
        transaction_id = %out.transaction_id,
        payment_id = %out.payment_txn_id,
        merchant_id = merchant_id.map(|m| m.to_string()),
        actor_kind,
        actor_id = %actor_id,
        amount = out.amount.as_i64(),
        balance = out.balance.as_i64(),
        replayed = out.replayed,
        "{event} booked"
    );
}

fn refund_response(out: ops::Refund) -> RefundResp {
    RefundResp {
        transaction_id: out.transaction_id,
        payment_id: out.payment_txn_id,
        amount: out.amount.as_i64(),
        balance: out.balance.as_i64(),
        restorations: out
            .restorations
            .into_iter()
            .map(|d| DeductionView {
                bucket_id: d.bucket_id,
                amount: d.amount.as_i64(),
            })
            .collect(),
        replayed: out.replayed,
    }
}

pub async fn refund(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    headers: HeaderMap,
    Json(req): Json<RefundReq>,
) -> Result<(StatusCode, Json<RefundResp>), ApiError> {
    let key = idempotency_key(&headers)?;
    verify_payment_owner(&state, req.payment_id, merchant.merchant_id).await?;
    let amount = req.amount.map(positive).transpose()?;
    let out = ops::refund(&state.pool, req.payment_id, amount, &key, now()).await?;
    audit_refund(
        "refund",
        &out,
        Some(merchant.merchant_id),
        merchant.actor.kind(),
        merchant.actor.id(),
    );
    Ok((StatusCode::CREATED, Json(refund_response(out))))
}

pub async fn void(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    headers: HeaderMap,
    Path(payment_id): Path<Uuid>,
) -> Result<Json<RefundResp>, ApiError> {
    let key = idempotency_key(&headers)?;
    verify_payment_owner(&state, payment_id, merchant.merchant_id).await?;
    let out = ops::void(&state.pool, payment_id, &key, now()).await?;
    audit_refund(
        "void",
        &out,
        Some(merchant.merchant_id),
        merchant.actor.kind(),
        merchant.actor.id(),
    );
    Ok(Json(refund_response(out)))
}

// ----- refundable payments (list of payments with a positive remainder) -----

/// Merchant-facing refundable payment: pseudonymous `account_id` only.
#[derive(Serialize)]
pub struct RefundableView {
    pub id: Uuid,
    pub account_id: Uuid,
    pub amount: i64,
    pub fee: i64,
    pub refunded: i64,
    pub refundable: i64,
    pub occurred_at: Timestamp,
}

/// Admin-facing refundable payment: the issuer sees the real card identity.
#[derive(Serialize)]
pub struct AdminRefundableView {
    pub id: Uuid,
    pub system_code: u16,
    pub idm: String,
    pub idi: String,
    pub merchant_id: Option<Uuid>,
    pub amount: i64,
    pub fee: i64,
    pub refunded: i64,
    pub refundable: i64,
    pub occurred_at: Timestamp,
}

fn admin_refundable_view(p: ops::RefundablePayment) -> AdminRefundableView {
    AdminRefundableView {
        id: p.id,
        system_code: p.account.system_code,
        idm: p.account.idm.to_hex(),
        idi: p.account.idi.to_hex(),
        merchant_id: p.merchant_id,
        amount: p.amount.as_i64(),
        fee: p.fee.as_i64(),
        refunded: p.refunded.as_i64(),
        refundable: p.refundable.as_i64(),
        occurred_at: p.occurred_at,
    }
}

#[derive(Deserialize)]
pub struct RefundableQuery {
    /// This merchant's pseudonymous account id (from mutual authentication).
    pub account_id: Option<Uuid>,
    pub limit: Option<i64>,
}

/// The caller merchant's refundable payments for one account, addressed by the
/// merchant's own pseudonymous `account_id`. Used by the terminal kiosk's refund
/// flow — the raw `(system_code, idi)` is never sent or returned.
pub async fn refundable(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    Query(q): Query<RefundableQuery>,
) -> Result<Json<Vec<RefundableView>>, ApiError> {
    let account_id = q
        .account_id
        .ok_or_else(|| ApiError::bad_request("account_id is required"))?;
    // Scoped to the caller: another merchant's alias resolves to nothing.
    let account = ops::account_for_alias(&state.pool, merchant.merchant_id, account_id)
        .await?
        .ok_or_else(|| ApiError::not_found("account not found"))?;
    let rows = ops::list_refundable_payments(
        &state.pool,
        Some(merchant.merchant_id),
        Some(account),
        q.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|p| RefundableView {
                id: p.id,
                account_id,
                amount: p.amount.as_i64(),
                fee: p.fee.as_i64(),
                refunded: p.refunded.as_i64(),
                refundable: p.refundable.as_i64(),
                occurred_at: p.occurred_at,
            })
            .collect(),
    ))
}

// ----- transaction history (scoped to the caller merchant) -----

#[derive(Deserialize)]
pub struct TxnQuery {
    pub kind: Option<String>,
    /// Filter to one store (merchant-admins only; store users are forced to their
    /// own store regardless of this).
    pub store_id: Option<Uuid>,
    pub before: Option<String>,
    pub limit: Option<i64>,
}

/// Merchant-facing transaction: pseudonymous `account_id` only.
#[derive(Serialize)]
pub struct TxnResp {
    pub id: Uuid,
    pub account_id: Uuid,
    pub kind: String,
    pub merchant_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub store_name: Option<String>,
    pub amount: i64,
    /// Processing fee (payments only; 0 otherwise).
    pub fee: i64,
    /// Optional free-text note the merchant attached to a payment.
    pub note: Option<String>,
    pub related_txn_id: Option<Uuid>,
    pub occurred_at: Timestamp,
}

/// Admin-facing transaction: the issuer sees the real card identity.
#[derive(Serialize)]
pub struct AdminTxnResp {
    pub id: Uuid,
    pub system_code: u16,
    pub idm: String,
    pub idi: String,
    pub kind: String,
    pub merchant_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub store_name: Option<String>,
    pub amount: i64,
    pub fee: i64,
    pub note: Option<String>,
    pub related_txn_id: Option<Uuid>,
    pub occurred_at: Timestamp,
}

pub async fn transactions(
    State(state): State<AppState>,
    merchant: AuthedMerchant,
    Query(q): Query<TxnQuery>,
) -> Result<Json<Vec<TxnResp>>, ApiError> {
    let before = q.before.as_deref().map(parse_ts).transpose()?;
    let filter = ops::TxnFilter {
        account: None,
        merchant_id: Some(merchant.merchant_id),
        // A store-scoped caller is forced to its own store; a merchant-wide admin
        // may optionally filter by one store.
        store_id: merchant.store_id.or(q.store_id),
        kind: q.kind,
        before,
        limit: q.limit.unwrap_or(50),
    };
    let rows = ops::list_transactions(&state.pool, &filter).await?;
    let accounts: Vec<AccountKey> = rows.iter().map(|t| t.account).collect();
    let aliases = alias_map(&state, merchant.merchant_id, accounts).await?;
    Ok(Json(
        rows.into_iter()
            .map(|t| TxnResp {
                account_id: aliases[&t.account],
                id: t.id,
                kind: t.kind,
                merchant_id: t.merchant_id,
                store_id: t.store_id,
                store_name: t.store_name,
                amount: t.amount.as_i64(),
                fee: t.fee.as_i64(),
                note: t.note,
                related_txn_id: t.related_txn_id,
                occurred_at: t.occurred_at,
            })
            .collect(),
    ))
}

fn admin_txn_view(t: ops::TransactionRow) -> AdminTxnResp {
    AdminTxnResp {
        id: t.id,
        system_code: t.account.system_code,
        idm: t.account.idm.to_hex(),
        idi: t.account.idi.to_hex(),
        kind: t.kind,
        merchant_id: t.merchant_id,
        store_id: t.store_id,
        store_name: t.store_name,
        amount: t.amount.as_i64(),
        fee: t.fee.as_i64(),
        note: t.note,
        related_txn_id: t.related_txn_id,
        occurred_at: t.occurred_at,
    }
}

/// Admin transaction listing — any merchant/idi/kind, not scoped to the caller.
#[derive(Deserialize)]
pub struct AdminTxnQuery {
    pub merchant_id: Option<Uuid>,
    pub store_id: Option<Uuid>,
    pub system_code: Option<String>,
    pub idm: Option<String>,
    pub idi: Option<String>,
    pub kind: Option<String>,
    pub before: Option<String>,
    pub limit: Option<i64>,
}

pub async fn admin_transactions(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AdminTxnQuery>,
) -> Result<Json<Vec<AdminTxnResp>>, ApiError> {
    // Filtering by account requires the full (system_code, IDm, IDi) key — an IDi
    // alone is ambiguous across systems, and the account key includes the IDm.
    let account =
        parse_account_filter(q.system_code.as_deref(), q.idm.as_deref(), q.idi.as_deref())?;
    let before = q
        .before
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(parse_ts)
        .transpose()?;
    let filter = ops::TxnFilter {
        account,
        merchant_id: q.merchant_id,
        store_id: q.store_id,
        kind: q.kind.filter(|s| !s.is_empty()),
        before,
        limit: q.limit.unwrap_or(50),
    };
    let rows = ops::list_transactions(&state.pool, &filter).await?;
    Ok(Json(rows.into_iter().map(admin_txn_view).collect()))
}

/// Admin lookup of any account's balance by (system_code, IDm, IDi).
pub async fn admin_account_balance(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((system_code, idm, idi)): Path<(String, String, String)>,
) -> Result<Json<AdminBalanceResp>, ApiError> {
    let account = AccountKey::new(parse_sc(&system_code)?, parse_idm(&idm)?, parse_idi(&idi)?);
    let bal = ops::balance(&state.pool, account, now()).await?;
    Ok(Json(admin_balance_resp(account, bal)))
}

// ----- accounts (admin) -----

#[derive(Deserialize)]
pub struct AccountsQuery {
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct AccountView {
    pub system_code: u16,
    pub idm: String,
    pub idi: String,
    pub status: String,
    pub balance: i64,
    pub created_at: Timestamp,
}

pub async fn list_accounts(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AccountsQuery>,
) -> Result<Json<Vec<AccountView>>, ApiError> {
    let rows = ops::list_accounts(&state.pool, now(), q.limit.unwrap_or(200)).await?;
    Ok(Json(
        rows.into_iter()
            .map(|a| AccountView {
                system_code: a.account.system_code,
                idm: a.account.idm.to_hex(),
                idi: a.account.idi.to_hex(),
                status: a.status,
                balance: a.balance.as_i64(),
                created_at: a.created_at,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct AdjustReq {
    /// Signed yen delta: positive credits, negative debits. Non-zero.
    pub delta: i64,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct AdjustResp {
    pub transaction_id: Uuid,
    pub delta: i64,
    pub balance: i64,
    pub bucket_id: Option<Uuid>,
}

pub async fn adjust_account(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path((system_code, idm, idi)): Path<(String, String, String)>,
    Json(req): Json<AdjustReq>,
) -> Result<Json<AdjustResp>, ApiError> {
    let account = AccountKey::new(parse_sc(&system_code)?, parse_idm(&idm)?, parse_idi(&idi)?);
    if req.delta == 0 {
        return Err(ApiError::bad_request("delta must be non-zero"));
    }
    let reason = req.reason.as_deref().filter(|s| !s.is_empty());
    let out = ops::adjust(
        &state.pool,
        account,
        melon_core::money::Yen::new(req.delta),
        reason,
        now(),
        &state.tz,
    )
    .await?;
    // A hand-written change to someone's balance. If any line in this log is ever
    // read in anger, it is this one — record who, how much, and why.
    crate::audit!(
        event = "account_adjust",
        transaction_id = %out.transaction_id,
        actor_kind = "user",
        actor_id = %admin.id,
        delta = out.delta.as_i64(),
        balance = out.balance.as_i64(),
        reason = reason.unwrap_or("-"),
        "account balance adjusted"
    );
    Ok(Json(AdjustResp {
        transaction_id: out.transaction_id,
        delta: out.delta.as_i64(),
        balance: out.balance.as_i64(),
        bucket_id: out.bucket_id,
    }))
}

// ----- admin: refundable payments + refund/void of any payment -----

#[derive(Deserialize)]
pub struct AdminRefundableQuery {
    pub merchant_id: Option<Uuid>,
    pub system_code: Option<String>,
    pub idm: Option<String>,
    pub idi: Option<String>,
    pub limit: Option<i64>,
}

pub async fn admin_refundable(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AdminRefundableQuery>,
) -> Result<Json<Vec<AdminRefundableView>>, ApiError> {
    let account =
        parse_account_filter(q.system_code.as_deref(), q.idm.as_deref(), q.idi.as_deref())?;
    let rows =
        ops::list_refundable_payments(&state.pool, q.merchant_id, account, q.limit.unwrap_or(50))
            .await?;
    Ok(Json(rows.into_iter().map(admin_refundable_view).collect()))
}

/// Refund any payment (admin; no merchant-owner check).
pub async fn admin_refund(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    headers: HeaderMap,
    Json(req): Json<RefundReq>,
) -> Result<(StatusCode, Json<RefundResp>), ApiError> {
    let key = idempotency_key(&headers)?;
    let amount = req.amount.map(positive).transpose()?;
    let out = ops::refund(&state.pool, req.payment_id, amount, &key, now()).await?;
    audit_refund("refund", &out, None, "user", admin.id);
    Ok((StatusCode::CREATED, Json(refund_response(out))))
}

/// Void (fully reverse) any payment (admin; no merchant-owner check).
pub async fn admin_void(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    headers: HeaderMap,
    Path(payment_id): Path<Uuid>,
) -> Result<Json<RefundResp>, ApiError> {
    let key = idempotency_key(&headers)?;
    let out = ops::void(&state.pool, payment_id, &key, now()).await?;
    audit_refund("void", &out, None, "user", admin.id);
    Ok(Json(refund_response(out)))
}

// ----- stores (店舗) -----

#[derive(Serialize)]
pub struct StoreView {
    pub id: Uuid,
    pub merchant_id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub is_default: bool,
    pub created_at: Timestamp,
}

fn store_view(s: ops::StoreRow) -> StoreView {
    StoreView {
        id: s.id,
        merchant_id: s.merchant_id,
        code: s.code,
        name: s.name,
        status: s.status,
        is_default: s.is_default,
        created_at: s.created_at,
    }
}

fn validate_store_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "active" | "suspended" | "closed") {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "status must be active, suspended or closed",
        ))
    }
}

// -- admin: store CRUD under a merchant --

#[derive(Deserialize)]
pub struct CreateStoreReq {
    pub code: String,
    pub name: String,
}

pub async fn admin_list_stores(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(merchant_id): Path<Uuid>,
) -> Result<Json<Vec<StoreView>>, ApiError> {
    let rows = ops::list_stores(&state.pool, merchant_id).await?;
    Ok(Json(rows.into_iter().map(store_view).collect()))
}

pub async fn admin_create_store(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
    Json(req): Json<CreateStoreReq>,
) -> Result<(StatusCode, Json<StoreView>), ApiError> {
    let code = req.code.trim();
    let name = req.name.trim();
    if code.is_empty() || name.is_empty() {
        return Err(ApiError::bad_request("code and name are required"));
    }
    let id = ops::create_store(&state.pool, merchant_id, code, name).await?;
    let store = ops::get_store(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("store vanished after creation"))?;
    crate::audit!(
        event = "store_created",
        store_id = %id,
        merchant_id = %merchant_id,
        actor_kind = "user",
        actor_id = %admin.id,
        code,
        "store created"
    );
    Ok((StatusCode::CREATED, Json(store_view(store))))
}

#[derive(Deserialize)]
pub struct StoreStatusReq {
    pub status: String,
}

pub async fn admin_set_store_status(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(store_id): Path<Uuid>,
    Json(req): Json<StoreStatusReq>,
) -> Result<Json<Value>, ApiError> {
    validate_store_status(&req.status)?;
    ops::set_store_status(&state.pool, store_id, &req.status).await?;
    crate::audit!(
        event = "store_status",
        store_id = %store_id,
        actor_kind = "user",
        actor_id = %admin.id,
        status = %req.status,
        "store status changed"
    );
    Ok(Json(
        serde_json::json!({ "store_id": store_id, "status": req.status }),
    ))
}

#[derive(Deserialize)]
pub struct StoreNameReq {
    pub name: String,
}

pub async fn admin_update_store(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(store_id): Path<Uuid>,
    Json(req): Json<StoreNameReq>,
) -> Result<Json<Value>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    ops::update_store_name(&state.pool, store_id, name).await?;
    crate::audit!(
        event = "store_renamed",
        store_id = %store_id,
        actor_kind = "user",
        actor_id = %admin.id,
        "store renamed"
    );
    Ok(Json(
        serde_json::json!({ "store_id": store_id, "name": name }),
    ))
}

// -- merchant: list own stores (store users see only their own) --

pub async fn merchant_list_stores(
    State(state): State<AppState>,
    caller: MerchantUser,
) -> Result<Json<Vec<StoreView>>, ApiError> {
    let mut rows = ops::list_stores(&state.pool, caller.merchant_id).await?;
    if let Some(scope) = caller.store_id {
        rows.retain(|s| s.id == scope);
    }
    Ok(Json(rows.into_iter().map(store_view).collect()))
}

// -- merchant: API key management (store-scoped) --

/// Verify `store_id` belongs to the caller's merchant and is within the caller's
/// store scope (a store user may only touch its own store).
async fn authorize_store(
    state: &AppState,
    caller: &MerchantUser,
    store_id: Uuid,
) -> Result<(), ApiError> {
    let store = ops::get_store(&state.pool, store_id)
        .await?
        .ok_or_else(|| ApiError::not_found("store not found"))?;
    // Don't reveal other merchants' stores.
    if store.merchant_id != caller.merchant_id {
        return Err(ApiError::not_found("store not found"));
    }
    if caller.store_id.is_some_and(|scope| scope != store_id) {
        return Err(ApiError::forbidden("outside your store scope"));
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ApiKeyView {
    pub id: Uuid,
    pub store_id: Option<Uuid>,
    pub label: Option<String>,
    pub created_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
    pub active: bool,
}

fn api_key_view(k: ops::ApiKeyRow) -> ApiKeyView {
    ApiKeyView {
        active: k.revoked_at.is_none(),
        id: k.id,
        store_id: k.store_id,
        label: k.label,
        created_at: k.created_at,
        revoked_at: k.revoked_at,
    }
}

pub async fn merchant_list_api_keys(
    State(state): State<AppState>,
    caller: MerchantUser,
    Path(store_id): Path<Uuid>,
) -> Result<Json<Vec<ApiKeyView>>, ApiError> {
    authorize_store(&state, &caller, store_id).await?;
    let rows = ops::list_api_keys(&state.pool, caller.merchant_id, Some(store_id)).await?;
    Ok(Json(rows.into_iter().map(api_key_view).collect()))
}

#[derive(Deserialize)]
pub struct CreateApiKeyReq {
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct CreateApiKeyResp {
    pub id: Uuid,
    pub store_id: Uuid,
    /// Shown once — the plaintext API secret. Store it now; only its hash is kept.
    pub api_key: String,
}

pub async fn merchant_create_api_key(
    State(state): State<AppState>,
    caller: MerchantUser,
    Path(store_id): Path<Uuid>,
    Json(req): Json<CreateApiKeyReq>,
) -> Result<(StatusCode, Json<CreateApiKeyResp>), ApiError> {
    authorize_store(&state, &caller, store_id).await?;
    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let secret = crate::auth::new_secret();
    let id = ops::store_api_key(
        &state.pool,
        caller.merchant_id,
        store_id,
        &crate::auth::sha256_hex(&secret),
        label,
    )
    .await?;
    // The key id, never the secret — the secret is shown to the caller once and
    // exists nowhere else in plaintext, including here.
    crate::security_info!(
        key_id = %id,
        merchant_id = %caller.merchant_id,
        store_id = %store_id,
        actor_id = %caller.user.id,
        "api key issued"
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResp {
            id,
            store_id,
            api_key: secret,
        }),
    ))
}

pub async fn merchant_revoke_api_key(
    State(state): State<AppState>,
    caller: MerchantUser,
    Path((store_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    authorize_store(&state, &caller, store_id).await?;
    let revoked = ops::revoke_api_key(&state.pool, caller.merchant_id, key_id).await?;
    if !revoked {
        return Err(ApiError::not_found("api key not found"));
    }
    crate::security_info!(
        key_id = %key_id,
        merchant_id = %caller.merchant_id,
        store_id = %store_id,
        actor_id = %caller.user.id,
        "api key revoked"
    );
    Ok(Json(
        serde_json::json!({ "key_id": key_id, "revoked": true }),
    ))
}

// ----- merchant management (admin) -----

#[derive(Deserialize)]
pub struct CreateMerchantReq {
    pub code: String,
    pub name: String,
    /// Payment fee in basis points; defaults to the server default if omitted.
    pub fee_bps: Option<i32>,
    /// Credit limit (yen); defaults to the server default if omitted.
    pub credit_limit: Option<i64>,
}

fn validate_credit_limit(credit_limit: i64) -> Result<i64, ApiError> {
    if credit_limit >= 0 {
        Ok(credit_limit)
    } else {
        Err(ApiError::bad_request("credit_limit must be >= 0"))
    }
}

fn validate_fee_bps(fee_bps: i32) -> Result<i32, ApiError> {
    if (0..=10000).contains(&fee_bps) {
        Ok(fee_bps)
    } else {
        Err(ApiError::bad_request("fee_bps must be between 0 and 10000"))
    }
}

#[derive(Serialize)]
pub struct CreateMerchantResp {
    pub merchant_id: Uuid,
    /// Shown once — the plaintext API secret. Store it now; only its hash is kept.
    pub api_key: String,
}

#[derive(Serialize)]
pub struct MerchantView {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    /// Payment fee rate in basis points (1 bps = 0.01%).
    pub fee_bps: i32,
    /// How far negative the settlement may go when selling top-ups (yen).
    pub credit_limit: i64,
    /// Settlement balance: net payments minus top-ups/refunds plus adjustments.
    pub collected: i64,
    pub created_at: Timestamp,
}

fn merchant_view(m: ops::MerchantRow) -> MerchantView {
    MerchantView {
        id: m.id,
        code: m.code,
        name: m.name,
        status: m.status,
        fee_bps: m.fee_bps,
        credit_limit: m.credit_limit.as_i64(),
        collected: m.collected.as_i64(),
        created_at: m.created_at,
    }
}

pub async fn list_merchants(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<MerchantView>>, ApiError> {
    let rows = ops::list_merchants(&state.pool).await?;
    Ok(Json(rows.into_iter().map(merchant_view).collect()))
}

#[derive(Deserialize)]
pub struct MerchantStatusReq {
    pub status: String,
}

pub async fn set_merchant_status(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
    Json(req): Json<MerchantStatusReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !matches!(req.status.as_str(), "active" | "suspended" | "closed") {
        return Err(ApiError::bad_request(
            "status must be active, suspended, or closed",
        ));
    }
    ops::set_merchant_status(&state.pool, merchant_id, &req.status).await?;
    crate::audit!(
        event = "merchant_status",
        merchant_id = %merchant_id,
        actor_kind = "user",
        actor_id = %admin.id,
        status = %req.status,
        "merchant status changed"
    );
    Ok(Json(
        serde_json::json!({ "merchant_id": merchant_id, "status": req.status }),
    ))
}

#[derive(Deserialize)]
pub struct MerchantAdjustReq {
    /// Signed yen delta: positive credits, negative debits. Non-zero.
    pub delta: i64,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct MerchantAdjustResp {
    pub id: Uuid,
    pub delta: i64,
    pub balance: i64,
}

#[derive(Deserialize)]
pub struct MerchantFeeReq {
    pub fee_bps: i32,
}

pub async fn set_merchant_fee(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
    Json(req): Json<MerchantFeeReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let fee_bps = validate_fee_bps(req.fee_bps)?;
    ops::set_merchant_fee(&state.pool, merchant_id, fee_bps).await?;
    crate::audit!(
        event = "merchant_fee",
        merchant_id = %merchant_id,
        actor_kind = "user",
        actor_id = %admin.id,
        fee_bps,
        "merchant fee changed"
    );
    Ok(Json(
        serde_json::json!({ "merchant_id": merchant_id, "fee_bps": fee_bps }),
    ))
}

#[derive(Deserialize)]
pub struct MerchantCreditReq {
    pub credit_limit: i64,
}

pub async fn set_merchant_credit_limit(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
    Json(req): Json<MerchantCreditReq>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let credit_limit = validate_credit_limit(req.credit_limit)?;
    ops::set_merchant_credit_limit(&state.pool, merchant_id, credit_limit).await?;
    crate::audit!(
        event = "merchant_credit_limit",
        merchant_id = %merchant_id,
        actor_kind = "user",
        actor_id = %admin.id,
        credit_limit,
        "merchant credit limit changed"
    );
    Ok(Json(
        serde_json::json!({ "merchant_id": merchant_id, "credit_limit": credit_limit }),
    ))
}

/// Rotate a merchant's API key: revoke the old one, return a fresh secret (once).
pub async fn rotate_api_key(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
) -> Result<Json<CreateMerchantResp>, ApiError> {
    // Admin rotation targets the merchant's default store.
    let store_id = ops::default_store_id(&state.pool, merchant_id)
        .await?
        .ok_or(melon_db::DbError::MerchantNotFound)?;
    let secret = crate::auth::new_secret();
    ops::rotate_api_key(
        &state.pool,
        merchant_id,
        store_id,
        &crate::auth::sha256_hex(&secret),
        Some("rotated"),
    )
    .await?;
    // The secret is returned to the caller once and never logged.
    crate::security_info!(
        merchant_id = %merchant_id,
        store_id = %store_id,
        actor_id = %admin.id,
        "merchant api key rotated (previous keys revoked)"
    );
    Ok(Json(CreateMerchantResp {
        merchant_id,
        api_key: secret,
    }))
}

pub async fn adjust_merchant(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(merchant_id): Path<Uuid>,
    Json(req): Json<MerchantAdjustReq>,
) -> Result<Json<MerchantAdjustResp>, ApiError> {
    if req.delta == 0 {
        return Err(ApiError::bad_request("delta must be non-zero"));
    }
    let reason = req.reason.as_deref().filter(|s| !s.is_empty());
    let out = ops::adjust_merchant(
        &state.pool,
        merchant_id,
        melon_core::money::Yen::new(req.delta),
        reason,
    )
    .await?;
    crate::audit!(
        event = "merchant_adjust",
        adjustment_id = %out.id,
        merchant_id = %merchant_id,
        actor_kind = "user",
        actor_id = %admin.id,
        delta = out.delta.as_i64(),
        balance = out.balance.as_i64(),
        reason = reason.unwrap_or("-"),
        "merchant settlement adjusted"
    );
    Ok(Json(MerchantAdjustResp {
        id: out.id,
        delta: out.delta.as_i64(),
        balance: out.balance.as_i64(),
    }))
}

pub async fn create_merchant(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<CreateMerchantReq>,
) -> Result<(StatusCode, Json<CreateMerchantResp>), ApiError> {
    let fee_bps = validate_fee_bps(req.fee_bps.unwrap_or(state.default_fee_bps))?;
    let credit_limit =
        validate_credit_limit(req.credit_limit.unwrap_or(state.default_credit_limit))?;
    let merchant_id =
        ops::create_merchant(&state.pool, &req.code, &req.name, fee_bps, credit_limit).await?;
    // The initial API key is issued for the merchant's default store.
    let store_id = ops::default_store_id(&state.pool, merchant_id)
        .await?
        .ok_or_else(|| ApiError::internal("default store missing after merchant creation"))?;
    let secret = crate::auth::new_secret();
    let key_id = ops::store_api_key(
        &state.pool,
        merchant_id,
        store_id,
        &crate::auth::sha256_hex(&secret),
        Some("initial"),
    )
    .await?;
    crate::audit!(
        event = "merchant_created",
        merchant_id = %merchant_id,
        store_id = %store_id,
        key_id = %key_id,
        actor_kind = "user",
        actor_id = %admin.id,
        code = %req.code,
        fee_bps,
        credit_limit,
        "merchant created"
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateMerchantResp {
            merchant_id,
            api_key: secret,
        }),
    ))
}

// ----- admin: expiry sweep -----

#[derive(Serialize)]
pub struct SweepResp {
    pub ran: bool,
    pub expired_buckets: i64,
    pub expired_amount: i64,
}

pub async fn sweep(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
) -> Result<Json<SweepResp>, ApiError> {
    let out = ops::expire_due(&state.pool, now(), 500).await?;
    crate::audit!(
        event = "expiry_sweep",
        actor_kind = "user",
        actor_id = %admin.id,
        ran = out.ran,
        expired_buckets = out.expired_buckets,
        expired_amount = out.expired_amount.as_i64(),
        "expiry sweep run by hand"
    );
    Ok(Json(SweepResp {
        ran: out.ran,
        expired_buckets: out.expired_buckets,
        expired_amount: out.expired_amount.as_i64(),
    }))
}

// ----- admin: outstanding balance report -----

#[derive(Deserialize)]
pub struct OutstandingQuery {
    pub as_of: Option<String>,
}

#[derive(Serialize)]
pub struct ExpiryMonthView {
    pub month: String,
    pub amount: i64,
}

#[derive(Serialize)]
pub struct OutstandingResp {
    pub as_of: Timestamp,
    pub total: i64,
    pub account_count: i64,
    pub by_expiry_month: Vec<ExpiryMonthView>,
}

pub async fn outstanding(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<OutstandingQuery>,
) -> Result<Json<OutstandingResp>, ApiError> {
    let as_of = q
        .as_of
        .as_deref()
        .map(parse_ts)
        .transpose()?
        .unwrap_or_else(now);
    let r = ops::outstanding_balance(&state.pool, as_of).await?;
    Ok(Json(OutstandingResp {
        as_of: r.as_of,
        total: r.total.as_i64(),
        account_count: r.account_count,
        by_expiry_month: r
            .by_expiry_month
            .into_iter()
            .map(|m| ExpiryMonthView {
                month: m.month,
                amount: m.amount.as_i64(),
            })
            .collect(),
    }))
}

// ----- admin: issuer (発行者) revenue account -----

#[derive(Serialize)]
pub struct IssuerBalanceResp {
    /// `fee_income + expiry_income + adjustments`.
    pub balance: i64,
    /// Payment fees collected from merchants (cumulative, non-refundable).
    pub fee_income: i64,
    /// Forfeited (expired) prepaid balances — breakage income (cumulative).
    pub expiry_income: i64,
    /// Net of manual issuer withdrawals (−) and corrections/injections (+).
    pub adjustments: i64,
}

pub async fn issuer_balance(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<IssuerBalanceResp>, ApiError> {
    let b = ops::issuer_balance(&state.pool).await?;
    Ok(Json(IssuerBalanceResp {
        balance: b.balance.as_i64(),
        fee_income: b.fee_income.as_i64(),
        expiry_income: b.expiry_income.as_i64(),
        adjustments: b.adjustments.as_i64(),
    }))
}

#[derive(Serialize)]
pub struct IssuerAdjustResp {
    pub id: Uuid,
    pub delta: i64,
    pub balance: i64,
}

pub async fn adjust_issuer(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Json(req): Json<AdjustReq>,
) -> Result<Json<IssuerAdjustResp>, ApiError> {
    if req.delta == 0 {
        return Err(ApiError::bad_request("delta must be non-zero"));
    }
    let reason = req.reason.as_deref().filter(|s| !s.is_empty());
    let out =
        ops::adjust_issuer(&state.pool, melon_core::money::Yen::new(req.delta), reason).await?;
    crate::audit!(
        event = "issuer_adjust",
        adjustment_id = %out.id,
        actor_kind = "user",
        actor_id = %admin.id,
        delta = out.delta.as_i64(),
        balance = out.balance.as_i64(),
        reason = reason.unwrap_or("-"),
        "issuer balance adjusted"
    );
    Ok(Json(IssuerAdjustResp {
        id: out.id,
        delta: out.delta.as_i64(),
        balance: out.balance.as_i64(),
    }))
}

#[derive(Serialize)]
pub struct IssuerAdjustmentView {
    pub id: Uuid,
    pub amount: i64,
    pub note: Option<String>,
    pub created_at: Timestamp,
}

pub async fn issuer_adjustments(
    State(state): State<AppState>,
    _admin: AdminUser,
    Query(q): Query<AccountsQuery>,
) -> Result<Json<Vec<IssuerAdjustmentView>>, ApiError> {
    let rows = ops::list_issuer_adjustments(&state.pool, q.limit.unwrap_or(50)).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| IssuerAdjustmentView {
                id: r.id,
                amount: r.amount.as_i64(),
                note: r.note,
                created_at: r.created_at,
            })
            .collect(),
    ))
}
