//! Shared melon-terminal logic.
//!
//! The terminal owns the physical PaSoRi reader but holds **no keys**. It polls
//! the card for its IDm/PMm, relays each command frame the server emits during
//! mutual authentication, and posts the card's response back — the server drives
//! the crypto and learns the verified IDi. Once authenticated it charges, tops
//! up, or checks the balance of that session.
//!
//! This library is used by both entry points: the one-shot CLI ([`main`]) and
//! the [`serve`] mode, which runs a local Web UI kiosk that owns the reader.

use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use felica_rs::felica_standard::{
    FelicaStandardCommand, FelicaStandardResponse, Type3TagPollingResult,
};
use felica_rs::prelude::*;
use serde_json::{Value, json};
use tracing::{debug, info, trace, warn};

pub mod serve;

/// Area and service are fixed at 0x0000 for melon (the card is used only for
/// identity; no on-card value is read or written).
pub const AREA: u16 = 0x0000;
pub const SERVICE: u16 = 0x0000;

/// Wildcard system code: polling with it matches whichever system answers first.
pub const WILDCARD_SYSTEM_CODE: u16 = 0xFFFF;

/// The money/read operation a tap performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Pay,
    Topup,
    Balance,
}

impl Op {
    /// Parse the `--op` / API value.
    pub fn parse(s: &str) -> Result<Op> {
        match s {
            "pay" => Ok(Op::Pay),
            "topup" => Ok(Op::Topup),
            "balance" => Ok(Op::Balance),
            other => bail!("unknown op '{other}' (expected 'pay', 'topup', or 'balance')"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Pay => "pay",
            Op::Topup => "topup",
            Op::Balance => "balance",
        }
    }

    /// Whether this operation consumes the card's spend capability (needs an
    /// amount and an idempotency key).
    pub fn is_spend(self) -> bool {
        matches!(self, Op::Pay | Op::Topup)
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A structured error returned by the melon server
/// (`{"error": {"code", "message", "details"}}`). Carried through `anyhow` so
/// callers can localize by the stable `code` (e.g. the kiosk UI renders Japanese
/// per code, showing `details` amounts when present).
#[derive(Debug, Clone)]
pub struct ServerError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "server returned {}: {}", self.status, self.message)
    }
}

impl std::error::Error for ServerError {}

/// Connection/config shared by every operation.
#[derive(Clone)]
pub struct Config {
    pub server: String,
    pub api_key: String,
    /// FeliCa system codes to try, in order; the first that authenticates wins.
    pub system_codes: Vec<u16>,
    /// Delay between polls while waiting for a card.
    pub poll_interval: Duration,
}

/// Why a card-wait loop gave up before a card appeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitAbort {
    /// A deadline passed (one-shot mode).
    Timeout,
    /// The operator cancelled (serve mode).
    Cancelled,
}

/// Open the PaSoRi reader (auto-detect).
pub fn open_reader_auto() -> Result<Reader> {
    let reader =
        open_reader(ReaderPreference::Auto).map_err(|e| anyhow!("failed to open reader: {e}"))?;
    info!(
        vendor = reader.vendor_name().unwrap_or("?"),
        product = reader.product_name().unwrap_or("?"),
        "reader opened"
    );
    Ok(reader)
}

/// Like [`open_reader_auto`] but WITHOUT the "reader opened" log line. Used by the
/// kiosk to probe for a reader repeatedly (only connect/disconnect transitions are
/// logged, by the caller). Errors when no reader is connected.
pub fn open_reader_quiet() -> Result<Reader> {
    open_reader(ReaderPreference::Auto).map_err(|e| anyhow!("failed to open reader: {e}"))
}

/// The FeliCa remote target melon polls (Type 3, 212 kbps).
pub fn make_target() -> Result<RemoteTarget> {
    RemoteTarget::new("212F").map_err(|e| anyhow!("target: {e}"))
}

/// A fresh blocking HTTP client.
pub fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::new()
}

/// One wildcard poll: the first system on the card that answers. Its IDm belongs
/// to THAT system only — a multi-system card must be re-polled with the chosen
/// system code before authenticating (see [`resolve_card`]).
pub fn poll_any(reader: &mut Reader, target: &RemoteTarget) -> Option<Type3TagPollingResult> {
    match reader
        .driver_mut()
        .detect_type_f(target, WILDCARD_SYSTEM_CODE, 0x00, 0x00)
    {
        Ok(poll) => {
            debug!(
                idm = %hex::encode(&poll.idm),
                pmm = %hex::encode(&poll.pmm),
                "wildcard poll (0xFFFF): card responded"
            );
            Some(poll)
        }
        Err(e) => {
            // The common case while idle: nothing on the reader.
            trace!(error = %e, "wildcard poll (0xFFFF): no card");
            None
        }
    }
}

/// Poll (wildcard) until a card is present, sleeping `poll_interval` between
/// passes. Before each sleep `should_abort` is consulted; returning `Some` stops
/// the wait. This waiting is the ONLY retry the terminal does — once a card is
/// present the caller makes a single attempt.
pub fn wait_for_card(
    reader: &mut Reader,
    target: &RemoteTarget,
    poll_interval: Duration,
    mut should_abort: impl FnMut() -> Option<WaitAbort>,
) -> Result<Type3TagPollingResult, WaitAbort> {
    info!(
        poll_interval_ms = poll_interval.as_millis() as u64,
        "waiting for a card (wildcard 0xFFFF polling)"
    );
    loop {
        if let Some(poll) = poll_any(reader, target) {
            info!(idm = %hex::encode(&poll.idm), "card detected");
            return Ok(poll);
        }
        if let Some(reason) = should_abort() {
            info!(?reason, "card wait aborted");
            return Err(reason);
        }
        std::thread::sleep(poll_interval);
    }
}

/// Ask the card which systems it contains (FeliCa **Request System Code**), using
/// the IDm from the wildcard poll. Returns the card's system codes in card order.
pub fn card_system_codes(
    reader: &mut Reader,
    target: &RemoteTarget,
    poll: &Type3TagPollingResult,
) -> Result<Vec<u16>> {
    let idm: [u8; 8] = poll
        .idm
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("card returned an IDm that is not 8 bytes"))?;
    let frame = FelicaStandardCommand::RequestSystemCode { idm }.to_frame();
    let timeout_ms = poll.request_system_code_timeout_ms();
    debug!(frame = %hex::encode(&frame), timeout_ms, "→ card: Request System Code");
    let response = reader
        .driver_mut()
        .transceive(target, &frame, Some(timeout_ms))
        .map_err(|e| anyhow!("Request System Code failed: {e}"))?;
    debug!(response = %hex::encode(&response), "← card: Request System Code response");
    match FelicaStandardResponse::from_bytes(&response)
        .map_err(|e| anyhow!("invalid Request System Code response: {e}"))?
    {
        FelicaStandardResponse::RequestSystemCode { system_codes, .. } => {
            info!(card_systems = %fmt_codes(&system_codes), "card system codes");
            Ok(system_codes)
        }
        _ => bail!("unexpected response to Request System Code"),
    }
}

/// Poll one specific system. This switches the card to that system and yields the
/// system's own IDm/PMm — which authentication requires.
pub fn poll_system(
    reader: &mut Reader,
    target: &RemoteTarget,
    system_code: u16,
) -> Result<Type3TagPollingResult> {
    debug!(
        system_code = %format!("0x{system_code:04X}"),
        "re-polling the selected system (each system has its own IDm)"
    );
    let poll = reader
        .driver_mut()
        .detect_type_f(target, system_code, 0x00, 0x00)
        .map_err(|e| anyhow!("polling system 0x{system_code:04X} failed: {e}"))?;
    debug!(
        system_code = %format!("0x{system_code:04X}"),
        idm = %hex::encode(&poll.idm),
        pmm = %hex::encode(&poll.pmm),
        "selected system polling result"
    );
    Ok(poll)
}

/// The system selected for a tapped card.
#[derive(Debug, Clone)]
pub struct CardSystem {
    /// The chosen system code: present on the card AND known to the server.
    pub system_code: u16,
    /// Polling result for the CHOSEN system (its own IDm/PMm). Authenticate with this.
    pub poll: Type3TagPollingResult,
    /// Every system the card exposes, in card order (diagnostics).
    pub card_codes: Vec<u16>,
}

/// Resolve which system to transact under, given a card that answered the wildcard
/// poll and the list of systems the server holds keys for:
///
/// 1. **Request System Code** — ask the card which systems it contains.
/// 2. Pick the first system in the **server's** list that the card also exposes —
///    server order decides, so a multi-system card's layout cannot steer us.
/// 3. **Re-poll that system** — each system has its own IDm, so the wildcard poll's
///    IDm cannot be reused (doing so is what makes Authentication1 fail).
pub fn resolve_card(
    reader: &mut Reader,
    target: &RemoteTarget,
    poll: &Type3TagPollingResult,
    allowed: &[u16],
) -> Result<CardSystem> {
    let card_codes = card_system_codes(reader, target, poll)?;
    let system_code = select_system_code(&card_codes, allowed).ok_or_else(|| {
        anyhow!(
            "card exposes no system the server can authenticate (card: {}, server: {})",
            fmt_codes(&card_codes),
            fmt_codes(allowed)
        )
    })?;
    info!(
        card_systems = %fmt_codes(&card_codes),
        server_systems = %fmt_codes(allowed),
        selected = %format!("0x{system_code:04X}"),
        "system selected (server order wins)"
    );
    let poll = poll_system(reader, target, system_code)?;
    Ok(CardSystem {
        system_code,
        poll,
        card_codes,
    })
}

/// The first system the **server** can authenticate that the **card** also exposes.
/// Server order wins: the server's list is scanned in order, so which system a
/// multi-system card transacts under is decided server-side, not by the card's
/// internal layout.
pub fn select_system_code(card_codes: &[u16], allowed: &[u16]) -> Option<u16> {
    allowed.iter().copied().find(|c| card_codes.contains(c))
}

/// Format system codes as `0x0003,0xFE00` for messages.
pub fn fmt_codes(codes: &[u16]) -> String {
    codes
        .iter()
        .map(|c| format!("0x{c:04X}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Run the mutual-authentication relay for one system code (area/service fixed at
/// 0x0000), returning `(session_id, account_id)` on success. `account_id` is this
/// merchant's **pseudonym** for the card — the terminal never learns the raw IDi.
pub fn authenticate(
    http: &reqwest::blocking::Client,
    cfg: &Config,
    reader: &mut Reader,
    target: &RemoteTarget,
    system_code: u16,
    poll: &Type3TagPollingResult,
) -> Result<(String, String)> {
    info!(
        system_code = %format!("0x{system_code:04X}"),
        idm = %hex::encode(&poll.idm),
        "starting online mutual authentication"
    );
    let start = json!({
        "idm": hex::encode(&poll.idm),
        "pmm": hex::encode(&poll.pmm),
        "system_code": system_code,
        "areas": [AREA],
        "services": [SERVICE],
    });
    let mut resp = post(
        http,
        &cfg.server,
        "/v1/mutual-authentication",
        &cfg.api_key,
        None,
        &start,
    )?;
    let session_id = resp["session_id"]
        .as_str()
        .ok_or_else(|| anyhow!("server did not return a session_id"))?
        .to_string();
    debug!(
        session_id = %session_id,
        step = %resp["step"],
        "mutual auth: session opened (server drives the crypto; we only relay)"
    );

    let mut step = 0u32;
    while resp["step"] != "complete" {
        step += 1;
        let frame = resp["command"]["frame"]
            .as_str()
            .ok_or_else(|| anyhow!("server did not return a command frame"))?;
        let timeout_ms =
            (resp["command"]["timeout"].as_f64().unwrap_or(0.1) * 1000.0).max(1.0) as u16;
        debug!(step, server_step = %resp["step"], frame, timeout_ms, "→ card: relay frame from server");
        let card_response = reader
            .driver_mut()
            .transceive(target, &hex::decode(frame)?, Some(timeout_ms))
            .map_err(|e| anyhow!("card exchange failed: {e}"))?;
        debug!(step, response = %hex::encode(&card_response), "← card: response (relaying to server)");
        let body = json!({ "session_id": session_id, "card_response": hex::encode(card_response) });
        resp = post(
            http,
            &cfg.server,
            "/v1/mutual-authentication",
            &cfg.api_key,
            None,
            &body,
        )?;
    }
    // The server never returns the raw IDi to a merchant — only this merchant's
    // pseudonymous account id (a different id for the same card at each merchant).
    let account_id = resp["result"]["account_id"]
        .as_str()
        .ok_or_else(|| anyhow!("server did not return an account_id"))?
        .to_string();
    info!(
        system_code = %format!("0x{system_code:04X}"),
        account_id = %account_id,
        session_id = %session_id,
        relay_steps = step,
        "mutual authentication complete (server-verified IDi)"
    );
    Ok((session_id, account_id))
}

/// Fetch the system codes the server holds keys for — the systems a card may be
/// authenticated under. Takes raw connection parameters (not [`Config`]) because it
/// runs *before* the config's system-code list exists.
pub fn fetch_system_codes(
    http: &reqwest::blocking::Client,
    server: &str,
    api_key: &str,
) -> Result<Vec<u16>> {
    let value = get(http, server, "/v1/system-codes", api_key)?;
    let codes = value["system_codes"]
        .as_array()
        .ok_or_else(|| anyhow!("server did not return a system_codes array"))?
        .iter()
        .map(|c| {
            c.as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| anyhow!("server returned an invalid system code"))
        })
        .collect::<Result<Vec<u16>>>()?;
    if codes.is_empty() {
        bail!("server has no usable system codes (no keys loaded)");
    }
    info!(
        system_codes = %fmt_codes(&codes),
        "usable systems fetched from the server (order = priority)"
    );
    Ok(codes)
}

/// Perform `op` against an authenticated `session_id`. Spend operations
/// (pay/topup) require a positive `amount` and generate a fresh idempotency key;
/// balance is read-only.
pub fn run_operation(
    http: &reqwest::blocking::Client,
    cfg: &Config,
    session_id: &str,
    op: Op,
    amount: Option<i64>,
) -> Result<Value> {
    match op {
        Op::Balance => {
            info!(op = %op, "operation: reading balance (does not consume the session)");
            post(
                http,
                &cfg.server,
                "/v1/balance",
                &cfg.api_key,
                None,
                &json!({ "session_id": session_id }),
            )
        }
        Op::Pay | Op::Topup => {
            let amount = amount.ok_or_else(|| anyhow!("amount is required for {op}"))?;
            if amount <= 0 {
                bail!("amount must be a positive integer number of yen");
            }
            let endpoint = if op == Op::Topup {
                "/v1/topups"
            } else {
                "/v1/payments"
            };
            let idem_key = format!("{op}-{}", hex::encode(rand::random::<[u8; 16]>()));
            info!(
                op = %op,
                amount,
                endpoint,
                idempotency_key = %idem_key,
                "operation: money movement (consumes the session's one-shot spend)"
            );
            post(
                http,
                &cfg.server,
                endpoint,
                &cfg.api_key,
                Some(&idem_key),
                &json!({ "session_id": session_id, "amount": amount }),
            )
        }
    }
}

/// List the caller merchant's refundable payments for one account, addressed by
/// the merchant's pseudonymous `account_id` (never the raw IDi). Returns the
/// server's JSON array (`[{id, account_id, amount, refunded, refundable, …}]`).
pub fn list_refundable(
    http: &reqwest::blocking::Client,
    cfg: &Config,
    account_id: &str,
) -> Result<Value> {
    let path = format!("/v1/payments/refundable?account_id={account_id}");
    get(http, &cfg.server, &path, &cfg.api_key)
}

/// Refund a payment (a positive `amount`, or the full refundable remainder when
/// `None`). Idempotent on a freshly generated key.
pub fn refund(
    http: &reqwest::blocking::Client,
    cfg: &Config,
    payment_id: &str,
    amount: Option<i64>,
) -> Result<Value> {
    let idem_key = format!("refund-{}", hex::encode(rand::random::<[u8; 16]>()));
    let mut body = json!({ "payment_id": payment_id });
    if let Some(a) = amount {
        body["amount"] = json!(a);
    }
    post(
        http,
        &cfg.server,
        "/v1/refunds",
        &cfg.api_key,
        Some(&idem_key),
        &body,
    )
}

/// POST JSON with a bearer token (and optional idempotency key), returning the
/// parsed body or an error carrying the server's message.
pub fn post(
    http: &reqwest::blocking::Client,
    base: &str,
    path: &str,
    bearer: &str,
    idempotency_key: Option<&str>,
    body: &Value,
) -> Result<Value> {
    let mut req = http
        .post(format!("{base}{path}"))
        .bearer_auth(bearer)
        .json(body);
    if let Some(key) = idempotency_key {
        req = req.header("Idempotency-Key", key);
    }
    // The bearer (merchant API key) is never logged.
    debug!(
        method = "POST",
        path,
        idempotent = idempotency_key.is_some(),
        "→ server"
    );
    trace!(method = "POST", path, body = %body, "→ server (request body)");
    let resp = req.send().context("request failed")?;
    let status = resp.status();
    let value: Value = resp.json().unwrap_or(Value::Null);
    log_response("POST", path, status, &value);
    into_result(status, value)
}

/// Log a server response: status at debug, body at trace, failures at warn.
fn log_response(method: &str, path: &str, status: reqwest::StatusCode, value: &Value) {
    if status.is_success() {
        debug!(method, path, status = status.as_u16(), "← server");
    } else {
        warn!(method, path, status = status.as_u16(), body = %value, "← server (error)");
    }
    trace!(method, path, body = %value, "← server (response body)");
}

/// GET JSON with a bearer token, returning the parsed body or a [`ServerError`].
pub fn get(
    http: &reqwest::blocking::Client,
    base: &str,
    path: &str,
    bearer: &str,
) -> Result<Value> {
    debug!(method = "GET", path, "→ server");
    let resp = http
        .get(format!("{base}{path}"))
        .bearer_auth(bearer)
        .send()
        .context("request failed")?;
    let status = resp.status();
    let value: Value = resp.json().unwrap_or(Value::Null);
    log_response("GET", path, status, &value);
    into_result(status, value)
}

/// Turn an HTTP status + body into a result. Server errors are
/// `{"error": {"code", "message", "details"?}}`; the structured shape is
/// preserved as a [`ServerError`] so callers can localize by `code`.
fn into_result(status: reqwest::StatusCode, value: Value) -> Result<Value> {
    if status.is_success() {
        return Ok(value);
    }
    let err = value.get("error");
    let code = err
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_string();
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let details = err.and_then(|e| e.get("details")).cloned();
    Err(ServerError {
        status: status.as_u16(),
        code,
        message,
        details,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_order_decides_the_selected_system() {
        // The card lists 0xFE00 first, but the SERVER's order wins.
        let card = [0xFE00u16, 0x0003];
        assert_eq!(select_system_code(&card, &[0x0003, 0xFE00]), Some(0x0003));
        // Flip the server's order → the other system is chosen, same card.
        assert_eq!(select_system_code(&card, &[0xFE00, 0x0003]), Some(0xFE00));
        // A server system the card lacks is skipped.
        assert_eq!(select_system_code(&card, &[0x1234, 0xFE00]), Some(0xFE00));
        // No overlap → no system to transact under.
        assert_eq!(select_system_code(&card, &[0x1234]), None);
        assert_eq!(select_system_code(&[], &[0x0003]), None);
    }

    #[test]
    fn formats_system_codes_as_hex() {
        assert_eq!(fmt_codes(&[0x0003, 0xFE00]), "0x0003,0xFE00");
        assert_eq!(fmt_codes(&[]), "");
    }

    #[test]
    fn op_roundtrips() {
        for s in ["pay", "topup", "balance"] {
            assert_eq!(Op::parse(s).unwrap().as_str(), s);
        }
        assert!(Op::parse("zap").is_err());
        assert!(Op::Pay.is_spend());
        assert!(Op::Topup.is_spend());
        assert!(!Op::Balance.is_spend());
    }
}
