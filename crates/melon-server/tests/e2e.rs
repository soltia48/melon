//! End-to-end test of the whole melon stack over HTTP:
//! create merchant → online FeliCa mutual authentication (relayed to the
//! in-memory card emulator) → top-up → pay → balance.
//!
//! This exercises the real trust flow: the server drives mutual auth, learns the
//! verified IDi itself, and only then accepts money operations bound to that
//! authenticated session.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use felica_rs::felica_standard::{
    EmulatedService, EmulatedSystem, FelicaStandardEmulator, ServiceCode,
};
use http_body_util::BodyExt;
use melon_auth::{KeyStore, SessionManager};
use melon_server::{AppState, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const SYSTEM_CODE: u16 = 0x0003;
const SERVICE_CODE: u16 = 0x0048;
const IDM: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
const PMM: [u8; 8] = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const K_SYS: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
const K_AREA: [u8; 8] = [0x21, 0x43, 0x65, 0x87, 0xA9, 0xCB, 0xED, 0x0F];
const K_SVC: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
const ISSUE_ID: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
const ISSUE_PARAM: [u8; 8] = [0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB];
const ADMIN_EMAIL: &str = "admin@example.test";
const ADMIN_PASSWORD: &str = "correct horse battery staple";

fn keystore() -> KeyStore {
    let jsonl = format!(
        concat!(
            r#"{{"system_code":"0003","node":"FFFF","algo":"DES","idm":null,"key":"{sys}"}}"#,
            "\n",
            r#"{{"system_code":"0003","node":"0000","algo":"DES","idm":null,"key":"{area}"}}"#,
            "\n",
            r#"{{"system_code":"0003","node":"0048","algo":"DES","idm":null,"key":"{svc}"}}"#,
            "\n",
        ),
        sys = hex::encode(K_SYS),
        area = hex::encode(K_AREA),
        svc = hex::encode(K_SVC),
    );
    KeyStore::from_reader(jsonl.as_bytes()).expect("keys parse")
}

fn emulated_card() -> FelicaStandardEmulator {
    let mut system = EmulatedSystem::new(SYSTEM_CODE, IDM, PMM).expect("system");
    system.set_system_key(K_SYS);
    system.set_issue_information(ISSUE_ID, ISSUE_PARAM);
    system.root_area_mut().set_key(K_AREA);
    let mut service =
        EmulatedService::with_blocks(ServiceCode::new(SERVICE_CODE), 0x0000, vec![[0u8; 16]]);
    service.set_key(K_SVC);
    system.add_service(service).expect("service fits");
    let mut emulator = FelicaStandardEmulator::new();
    emulator.add_system(system);
    emulator
}

fn relay_to_card(emulator: &mut FelicaStandardEmulator, frame_hex: &str) -> String {
    let frame = hex::decode(frame_hex).expect("hex frame");
    hex::encode(emulator.handle_frame(&frame).expect("card responds"))
}

fn build_app(pool: PgPool) -> Router {
    let manager = SessionManager::new(
        Arc::new(keystore()),
        Some(HashSet::from([0x14])),
        Duration::from_secs(60),
        16,
    );
    let tz = melon_core::expiry::expiry_timezone().unwrap();
    let state = AppState {
        pool,
        manager,
        tz,
        user_session_ttl: Duration::from_secs(3600),
        cookie_secure: false,
        default_fee_bps: 0,
        default_credit_limit: 10_000_000,
        turnstile: None,
        trust_proxy: false,
        log_card_ids: false,
    };
    router(state)
}

/// Seed an issuer (admin) user, then sign in for real over HTTP. Returns the
/// credential marker `send` turns into a session cookie.
async fn sign_in_admin(app: &Router, pool: &PgPool) -> String {
    let hash = melon_server::auth::hash_password(ADMIN_PASSWORD).unwrap();
    melon_db::users::create_user(pool, ADMIN_EMAIL, "Admin", &hash, "admin", None, None)
        .await
        .unwrap();
    sign_in(app, ADMIN_EMAIL, ADMIN_PASSWORD).await
}

/// Sign in and extract the session token from the `Set-Cookie` header. The token
/// is never in the body — it only ever travels as an HttpOnly cookie.
async fn sign_in(app: &Router, email: &str, password: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "email": email, "password": password })).unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "sign-in should succeed");
    let cookie = resp
        .headers()
        .get("set-cookie")
        .expect("login must set a session cookie")
        .to_str()
        .unwrap();
    assert!(
        cookie.contains("HttpOnly"),
        "session cookie must be HttpOnly"
    );
    assert!(
        cookie.contains("SameSite=Strict"),
        "session cookie must be SameSite=Strict"
    );
    let token = cookie
        .split(';')
        .next()
        .unwrap()
        .trim()
        .strip_prefix("melon_session=")
        .expect("session cookie")
        .to_string();
    format!("session:{token}")
}

/// Send one JSON request and return `(status, body)`.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    idempotency_key: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(b) = bearer {
        match b.strip_prefix("session:") {
            Some(token) => builder = builder.header("cookie", format!("melon_session={token}")),
            None => builder = builder.header("authorization", format!("Bearer {b}")),
        }
    }
    if let Some(k) = idempotency_key {
        builder = builder.header("idempotency-key", k);
    }
    let req = builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// Run the 3-step mutual authentication over HTTP against a fresh emulated card,
/// returning `(session_id, account_id)`. `account_id` is the merchant's pseudonym —
/// the raw card identity (IDi) must never reach the merchant.
async fn authenticate(app: &Router, merchant_key: &str) -> (String, String) {
    let mut card = emulated_card();

    let (status, v) = send(
        app,
        "POST",
        "/v1/mutual-authentication",
        Some(merchant_key),
        None,
        json!({
            "idm": hex::encode(IDM),
            "pmm": hex::encode(PMM),
            "system_code": "0x0003",
            "areas": ["0x0000"],
            "services": ["0x0048"],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "auth1: {v}");
    assert_eq!(v["step"], "auth1");
    let session_id = v["session_id"].as_str().unwrap().to_string();

    // Steps 2 and 3: relay the card responses back.
    let mut card_response = relay_to_card(&mut card, v["command"]["frame"].as_str().unwrap());
    for _ in 0..2 {
        let (status, v) = send(
            app,
            "POST",
            "/v1/mutual-authentication",
            Some(merchant_key),
            None,
            json!({ "session_id": session_id, "card_response": card_response }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "auth step: {v}");
        if v["step"] == "complete" {
            // The merchant must never receive the raw card identity.
            assert!(
                v["result"]["issue_id"].is_null(),
                "raw IDi leaked to the merchant: {v}"
            );
            let account_id = v["result"]["account_id"]
                .as_str()
                .expect("account_id")
                .to_string();
            return (session_id, account_id);
        }
        card_response = relay_to_card(&mut card, v["command"]["frame"].as_str().unwrap());
    }
    panic!("authentication did not complete");
}

#[sqlx::test(migrations = "../melon-db/migrations")]
async fn end_to_end_auth_topup_pay_balance(pool: PgPool) {
    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;

    // Admin creates a merchant and receives its one-time API key.
    let (status, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-1", "name": "Test Shop" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create merchant: {v}");
    let merchant_key = v["api_key"].as_str().unwrap().to_string();

    // A request without a valid key is rejected.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/balance",
        Some("wrong"),
        None,
        json!({ "session_id": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Top-up ¥1000 against a freshly authenticated card.
    let (session, account_id) = authenticate(&app, &merchant_key).await;
    let (status, v) = send(
        &app,
        "POST",
        "/v1/topups",
        Some(&merchant_key),
        Some("topup-1"),
        json!({ "session_id": session, "amount": 1000 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "topup: {v}");
    assert_eq!(v["amount"], 1000);
    assert_eq!(v["balance"], 1000);

    // The session's spend capability is one-shot: a second money op is refused.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("pay-reuse"),
        json!({ "session_id": session, "amount": 100 }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "reused session must be refused"
    );

    // Pay ¥300 against a new authentication.
    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, v) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("pay-1"),
        json!({ "session_id": session, "amount": 300 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "pay: {v}");
    assert_eq!(v["amount"], 300);
    assert_eq!(v["balance"], 700);
    assert_eq!(v["deductions"].as_array().unwrap().len(), 1);

    // Balance for the authenticated card is ¥700.
    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, v) = send(
        &app,
        "POST",
        "/v1/balance",
        Some(&merchant_key),
        None,
        json!({ "session_id": session }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "balance: {v}");
    // Pseudonym only — no raw card identity anywhere in the merchant response.
    assert_eq!(v["account_id"], account_id);
    assert!(
        v["idi"].is_null() && v["system_code"].is_null(),
        "raw identity leaked: {v}"
    );
    assert_eq!(v["total"], 700);

    // Overspending is refused with a localizable code and structured details
    // (amounts the terminal renders in Japanese).
    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, v) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("pay-over"),
        json!({ "session_id": session, "amount": 5000 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "overspend: {v}");
    assert_eq!(v["error"]["code"], "INSUFFICIENT_FUNDS");
    assert_eq!(v["error"]["details"]["available"], 700);
    assert_eq!(v["error"]["details"]["requested"], 5000);

    // Merchant sees its payment in history.
    let (status, v) = send(
        &app,
        "GET",
        "/v1/transactions",
        Some(&merchant_key),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kinds: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    // The merchant sees both the payment and the top-up it performed.
    assert!(kinds.contains(&"payment"));
    assert!(kinds.contains(&"top_up"));

    // The ¥300 payment is refundable; the merchant's kiosk endpoint lists it.
    let idm_hex = hex::encode(IDM);
    let idi_hex = hex::encode(ISSUE_ID);
    let (status, v) = send(
        &app,
        "GET",
        &format!("/v1/payments/refundable?account_id={account_id}"),
        Some(&merchant_key),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "refundable: {v}");
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let payment_id = list[0]["id"].as_str().unwrap().to_string();
    assert_eq!(list[0]["amount"], 300);
    assert_eq!(list[0]["refundable"], 300);

    // Admin refunds ¥100 of it (no merchant-owner check needed for admin).
    let (status, v) = send(
        &app,
        "POST",
        "/v1/admin/refunds",
        Some(&admin),
        Some("admin-refund-1"),
        json!({ "payment_id": payment_id, "amount": 100 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "admin refund: {v}");
    assert_eq!(v["amount"], 100);
    assert_eq!(v["balance"], 800); // 700 + 100 restored

    // Now only ¥200 remains refundable on that payment.
    let (_, v) = send(
        &app,
        "GET",
        &format!("/v1/admin/refundable?system_code=0x0003&idm={idm_hex}&idi={idi_hex}"),
        Some(&admin),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(v.as_array().unwrap()[0]["refundable"], 200);
}

#[sqlx::test(migrations = "../melon-db/migrations")]
async fn merchants_see_different_pseudonyms_for_the_same_card(pool: PgPool) {
    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;

    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-a", "name": "A" }),
    )
    .await;
    let key_a = v["api_key"].as_str().unwrap().to_string();
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-b", "name": "B" }),
    )
    .await;
    let key_b = v["api_key"].as_str().unwrap().to_string();

    // The SAME physical card taps at both merchants.
    let (_, alias_a) = authenticate(&app, &key_a).await;
    let (_, alias_a_again) = authenticate(&app, &key_a).await;
    let (_, alias_b) = authenticate(&app, &key_b).await;

    // Stable for one merchant (it can recognize its own returning customer) …
    assert_eq!(
        alias_a, alias_a_again,
        "a merchant must see a stable pseudonym for the same card"
    );
    // … but unlinkable across merchants, even if they collude.
    assert_ne!(
        alias_a, alias_b,
        "merchants must not be able to correlate the same cardholder"
    );

    // One merchant cannot use another merchant's pseudonym.
    let (status, _) = send(
        &app,
        "GET",
        &format!("/v1/payments/refundable?account_id={alias_b}"),
        Some(&key_a),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a pseudonym must be scoped to the merchant it was issued to"
    );
}

#[sqlx::test(migrations = "../melon-db/migrations")]
async fn sign_on_enforces_roles_and_merchant_isolation(pool: PgPool) {
    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;

    // Two merchants, each with a staff user created by the issuer.
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-a", "name": "A" }),
    )
    .await;
    let merchant_a: uuid::Uuid = v["merchant_id"].as_str().unwrap().parse().unwrap();
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-b", "name": "B" }),
    )
    .await;
    let merchant_b: uuid::Uuid = v["merchant_id"].as_str().unwrap().parse().unwrap();

    let (status, _) = send(
        &app,
        "POST",
        "/v1/admin/users",
        Some(&admin),
        None,
        json!({ "email": "a@shop.test", "name": "A staff", "password": "a-long-password",
                "role": "merchant", "merchant_id": merchant_a }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    send(
        &app,
        "POST",
        "/v1/admin/users",
        Some(&admin),
        None,
        json!({ "email": "b@shop.test", "name": "B staff", "password": "b-long-password",
                "role": "merchant", "merchant_id": merchant_b }),
    )
    .await;

    // Wrong password never signs in; a short password is rejected up front.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/auth/login",
        None,
        None,
        json!({ "email": "a@shop.test", "password": "wrong" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &app,
        "POST",
        "/v1/admin/users",
        Some(&admin),
        None,
        json!({ "email": "short@shop.test", "name": "x", "password": "short",
                "role": "merchant", "merchant_id": merchant_a }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "short passwords must be rejected"
    );

    let staff_a = sign_in(&app, "a@shop.test", "a-long-password").await;

    // A merchant user cannot reach issuer endpoints.
    let (status, _) = send(
        &app,
        "GET",
        "/v1/admin/users",
        Some(&staff_a),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "merchant staff must not be admins"
    );

    // A merchant user sees ONLY its own merchant's staff …
    let (status, v) = send(&app, "GET", "/v1/users", Some(&staff_a), None, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let emails: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["email"].as_str().unwrap())
        .collect();
    assert_eq!(
        emails,
        vec!["a@shop.test"],
        "must not see another merchant's users"
    );

    // … and any staff it creates is forced onto its own merchant, never admin.
    let (status, v) = send(
        &app,
        "POST",
        "/v1/users",
        Some(&staff_a),
        None,
        json!({ "email": "a2@shop.test", "name": "A staff 2", "password": "another-long-pw",
                "role": "admin", "merchant_id": merchant_b }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(v["role"], "merchant", "a merchant cannot mint an admin");
    assert_eq!(
        v["merchant_id"],
        merchant_a.to_string(),
        "cannot attach staff to another merchant"
    );

    // A merchant user cannot disable another merchant's user.
    let (_, v) = send(
        &app,
        "GET",
        "/v1/admin/users",
        Some(&admin),
        None,
        Value::Null,
    )
    .await;
    let b_user = v
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "b@shop.test")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/v1/users/{b_user}/status"),
        Some(&staff_a),
        None,
        json!({ "status": "disabled" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "cannot touch another merchant's user"
    );

    // Disabling a user revokes their live session immediately.
    let (_, v) = send(&app, "GET", "/v1/users", Some(&staff_a), None, Value::Null).await;
    let a_id = v
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "a@shop.test")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = send(
        &app,
        "POST",
        &format!("/v1/admin/users/{a_id}/status"),
        Some(&admin),
        None,
        json!({ "status": "disabled" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app,
        "GET",
        "/v1/auth/me",
        Some(&staff_a),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "disabling a user must kill their session"
    );

    // Signing out revokes the admin's session too.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/auth/logout",
        Some(&admin),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app,
        "GET",
        "/v1/admin/users",
        Some(&admin),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "logout must revoke the session"
    );

    // No session at all → 401.
    let (status, _) = send(&app, "GET", "/v1/admin/users", None, None, Value::Null).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../melon-db/migrations")]
async fn security_headers_are_set_on_every_response(pool: PgPool) {
    // There is no reverse proxy in front of melon (it is exposed via cloudflared),
    // so the application itself must send the hardening headers.
    let app = build_app(pool.clone());

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let h = resp.headers();
    assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
    assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    // A pure JSON API serves no document, so the policy is locked all the way down.
    let csp = h.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "csp: {csp}");
    assert!(csp.contains("default-src 'none'"), "csp: {csp}");
    // build_app has cookie_secure = false (plain-HTTP test), so HSTS must NOT be
    // sent — pinning it would lock a developer's browser out of http://localhost.
    assert!(
        h.get("strict-transport-security").is_none(),
        "HSTS must only be sent when we are really behind TLS"
    );

    // Error responses carry the headers too (the layer wraps everything).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
}

// ----- logging -----

// Capturing the log in a test binary is trickier than it looks: tracing caches,
// globally and per call site, whether *anyone* is interested in a span or event.
// A subscriber installed only on this thread (`set_default`) loses that race
// against the tests running in parallel with no subscriber at all, and spans go
// missing — nondeterministically.
//
// So: one subscriber for the whole binary, installed once, whose writer routes
// each line to the buffer belonging to the thread that emitted it. Tests that did
// not ask to capture have no buffer, and their output is dropped.
thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Arc<std::sync::Mutex<Vec<u8>>>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct ThreadWriter;

impl std::io::Write for ThreadWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        CAPTURE.with(|slot| {
            if let Some(sink) = slot.borrow().as_ref() {
                sink.lock().unwrap().extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ThreadWriter {
    type Writer = ThreadWriter;
    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

/// The log this test produced.
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl Logs {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
    /// The captured lines belonging to one target, parsed as JSON.
    fn events(&self, target: &str) -> Vec<Value> {
        self.text()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["target"] == target)
            .collect()
    }
}

/// Start capturing this thread's log output as JSON — the same format the server
/// emits in production (`MELON_LOG_FORMAT=json`).
fn capture_logs() -> Logs {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_writer(ThreadWriter)
            .with_max_level(tracing::Level::INFO)
            .init();
    });
    let sink = Arc::new(std::sync::Mutex::new(Vec::new()));
    CAPTURE.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&sink)));
    Logs(sink)
}

/// The audit stream has to be able to answer "who charged what, when" — and must
/// do it without ever writing down a secret or a card identity.
#[sqlx::test(migrations = "../melon-db/migrations")]
async fn the_audit_log_records_money_without_leaking_secrets(pool: PgPool) {
    let logs = capture_logs();

    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-log", "name": "Log Shop" }),
    )
    .await;
    let merchant_key = v["api_key"].as_str().unwrap().to_string();

    let (session, _) = authenticate(&app, &merchant_key).await;
    send(
        &app,
        "POST",
        "/v1/topups",
        Some(&merchant_key),
        Some("topup-log"),
        json!({ "session_id": session, "amount": 1000 }),
    )
    .await;

    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, v) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("pay-log"),
        json!({ "session_id": session, "amount": 400, "note": "coffee" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "payment: {v}");
    let transaction_id = v["transaction_id"].as_str().unwrap().to_string();

    // Retrying with the same idempotency key books nothing new — and the audit log
    // has to say so, or "charged twice" and "asked twice, charged once" look alike.
    // A retry needs a fresh tap: the session's spend capability is one-shot, so a
    // terminal re-sending after a timeout re-authenticates the card first.
    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, replay) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("pay-log"),
        json!({ "session_id": session, "amount": 400, "note": "coffee" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "replay: {replay}");
    assert_eq!(replay["replayed"], true, "the retry must not book twice");

    let audit = logs.events("melon::audit");
    let payments: Vec<&Value> = audit.iter().filter(|e| e["event"] == "payment").collect();
    assert_eq!(payments.len(), 2, "one audit line per attempt: {audit:#?}");
    assert_eq!(payments[0]["transaction_id"], transaction_id);
    assert_eq!(payments[0]["amount"], 400);
    assert_eq!(payments[0]["replayed"], false);
    assert_eq!(payments[1]["replayed"], true, "the retry must be marked");
    // The actor: this was the terminal's API key, not a person.
    assert_eq!(payments[0]["actor_kind"], "api_key");
    assert!(payments[0]["actor_id"].is_string());
    // Every line inherits the request span, so one id pulls up the whole request.
    assert!(
        payments[0]["span"]["request_id"].is_string(),
        "{:#?}",
        payments[0]
    );

    assert_eq!(
        audit.iter().filter(|e| e["event"] == "top_up").count(),
        1,
        "the top-up is audited too"
    );

    // The whole point: none of this may ever be in a log.
    let text = logs.text();
    assert!(
        !text.contains(&merchant_key),
        "the API key leaked into the log"
    );
    assert!(
        !text.contains(&hex::encode(ISSUE_ID)),
        "the card's IDi leaked into the log"
    );
    assert!(
        !text.contains(&hex::encode(IDM)),
        "the card's IDm leaked into the log"
    );
    assert!(
        !text.contains(&admin.replace("session:", "")),
        "the session token leaked into the log"
    );
    assert!(
        !text.contains(ADMIN_PASSWORD),
        "a password leaked into the log"
    );
}

/// A caller's request id is adopted and echoed, so one id ties the client's record
/// of a call to ours. Without one, an inbound id is minted.
#[sqlx::test(migrations = "../melon-db/migrations")]
async fn every_response_carries_a_request_id(pool: PgPool) {
    let app = build_app(pool);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("x-request-id", "caller-supplied-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("x-request-id").unwrap(),
        "caller-supplied-id"
    );

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let minted = resp
        .headers()
        .get("x-request-id")
        .expect("a request id is minted");
    assert!(!minted.is_empty());
}

/// A card whose IDm is randomized cannot be an account: every tap would look like
/// a new one, and the holder's balance would vanish. The manufacturer code gives
/// it away, so the card is turned away before any authentication work — and long
/// before it can open an account it could never reach again.
#[sqlx::test(migrations = "../melon-db/migrations")]
async fn a_card_with_a_randomized_idm_is_refused(pool: PgPool) {
    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-idm", "name": "IDm Shop" }),
    )
    .await;
    let merchant_key = v["api_key"].as_str().unwrap().to_string();

    // 04FEh: a FeliCa Standard card with a randomized ID (X4FEh, system number 0).
    // The rest of the XXFEh block is covered by melon-core's unit tests.
    let start = |idm: [u8; 8]| {
        json!({
            "idm": hex::encode(idm),
            "pmm": hex::encode(PMM),
            "system_code": "0x0003",
            "areas": ["0x0000"],
            "services": ["0x0048"],
        })
    };
    let (status, v) = send(
        &app,
        "POST",
        "/v1/mutual-authentication",
        Some(&merchant_key),
        None,
        start([0x04, 0xFE, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["code"], "UNSUPPORTED_CARD");
    // Refused at the door: no session was opened for it.
    assert!(v["session_id"].is_null(), "{v}");

    // The emulated card's IDm (0102h) is a normal one, and still authenticates.
    let (status, v) = send(
        &app,
        "POST",
        "/v1/mutual-authentication",
        Some(&merchant_key),
        None,
        start(IDM),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a normal card must still be let in: {v}"
    );
    assert_eq!(v["step"], "auth1");
}

#[sqlx::test(migrations = "../melon-db/migrations")]
async fn a_cardholder_reads_their_own_balance_by_idi(pool: PgPool) {
    let app = build_app(pool.clone());
    let admin = sign_in_admin(&app, &pool).await;

    // A merchant funds the card: top up ¥1000, pay ¥300 → ¥700 spendable.
    let (_, v) = send(
        &app,
        "POST",
        "/v1/merchants",
        Some(&admin),
        None,
        json!({ "code": "shop-self", "name": "Self Shop" }),
    )
    .await;
    let merchant_key = v["api_key"].as_str().unwrap().to_string();

    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, _) = send(
        &app,
        "POST",
        "/v1/topups",
        Some(&merchant_key),
        Some("self-topup"),
        json!({ "session_id": session, "amount": 1000 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (session, _) = authenticate(&app, &merchant_key).await;
    let (status, _) = send(
        &app,
        "POST",
        "/v1/payments",
        Some(&merchant_key),
        Some("self-pay"),
        json!({ "session_id": session, "amount": 300 }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The cardholder reads their Suica ID from their wallet app; the client turns
    // it back into the IDi and asks for the balance — no credentials, no session.
    let (status, v) = send(
        &app,
        "POST",
        "/v1/self/balance",
        None,
        None,
        json!({ "system_code": 3, "idi": hex::encode(ISSUE_ID) }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "self balance: {v}");
    assert_eq!(v["total"], 700);
    assert_eq!(v["system_code"], 3);
    assert_eq!(v["buckets"].as_array().unwrap().len(), 1);
    // The lower-trust path must not echo the raw identity or any merchant alias.
    assert!(
        v["idi"].is_null() && v["idm"].is_null() && v["account_id"].is_null(),
        "self-service path leaked identity: {v}"
    );

    // An IDi with no melon account → 404, told apart from a zero balance.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/self/balance",
        None,
        None,
        json!({ "system_code": 3, "idi": "1111222233334444" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A malformed IDi is a client error.
    let (status, _) = send(
        &app,
        "POST",
        "/v1/self/balance",
        None,
        None,
        json!({ "system_code": 3, "idi": "not-hex" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
