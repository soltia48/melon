//! `serve` mode: a local Web UI kiosk that owns the reader.
//!
//! A browser cannot touch the USB PaSoRi, so this process owns the reader and
//! serves both the UI and a small local JSON API on the same `http://localhost`
//! origin (which sidesteps CORS/mixed-content). The merchant API key never
//! leaves the process — the browser only triggers pay/topup/balance.
//!
//! The reader is a single resource, so a dedicated worker thread owns it and
//! runs one job at a time (mirroring the server's session worker). The HTTP loop
//! only enqueues jobs and reports status, so it never blocks on a card tap.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{Config, Op, WaitAbort};

/// The kiosk single-page app, embedded in the binary.
const INDEX_HTML: &str = include_str!("../static/terminal.html");

/// One queued unit of work for the reader worker.
enum Job {
    /// Wait for a card, authenticate, then pay / top up / read balance.
    Operate { op: Op, amount: Option<i64> },
    /// Wait for a card, authenticate, then list that account's refundable
    /// payments (phase 1 of a refund). Ends in the `select` phase.
    RefundLookup,
    /// Refund a chosen payment (phase 2). Needs no card.
    RefundExec {
        payment_id: String,
        amount: Option<i64>,
    },
}

/// The kiosk's current state, polled by the UI via `GET /status`.
#[derive(Clone)]
struct Status {
    /// idle | waiting | authenticating | processing | select | done | error
    phase: &'static str,
    op: Option<&'static str>,
    amount: Option<i64>,
    message: String,
    /// The server's response on success (balance/payment/topup/refund).
    result: Option<Value>,
    /// The refundable-payment list, in the `select` phase.
    refundable: Option<Value>,
    /// Stable error code the UI localizes on (e.g. `INSUFFICIENT_FUNDS`).
    error_code: Option<String>,
    /// Structured error fields (e.g. `{available, requested}` amounts).
    error_details: Option<Value>,
    /// The raw/technical error message (secondary; the UI shows localized text).
    error: Option<String>,
    /// `{ account_id }` once a card has authenticated — this merchant's pseudonym
    /// for the card. The raw (system_code, idi) never reaches the merchant.
    card: Option<Value>,
}

impl Status {
    fn idle(message: &str) -> Self {
        Status {
            phase: "idle",
            op: None,
            amount: None,
            message: message.to_string(),
            result: None,
            refundable: None,
            error_code: None,
            error_details: None,
            error: None,
            card: None,
        }
    }

    /// The initial "place the card on the reader" state for a job.
    fn waiting(op: &'static str, amount: Option<i64>) -> Self {
        Status {
            phase: "waiting",
            op: Some(op),
            amount,
            message: "カードをかざしてください".into(),
            result: None,
            refundable: None,
            error_code: None,
            error_details: None,
            error: None,
            card: None,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "phase": self.phase,
            "op": self.op,
            "amount": self.amount,
            "message": self.message,
            "result": self.result,
            "refundable": self.refundable,
            "error_code": self.error_code,
            "error_details": self.error_details,
            "error": self.error,
            "card": self.card,
        })
    }

    /// Whether a job is currently in flight (so a new one must be rejected).
    fn is_busy(&self) -> bool {
        matches!(self.phase, "waiting" | "authenticating" | "processing")
    }
}

/// Shared handles the HTTP loop and the reader worker both hold.
#[derive(Clone)]
struct Shared {
    status: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
    jobs: Sender<Job>,
    /// Config + HTTP client for reader-free server calls (e.g. merchant info).
    cfg: Config,
    http: reqwest::blocking::Client,
}

/// Run the kiosk: open the reader on a worker thread, then serve the UI + local
/// API. Blocks until the process is stopped.
pub fn run(cfg: Config, bind: &str) -> Result<()> {
    let status = Arc::new(Mutex::new(Status::idle("待機中")));
    let cancel = Arc::new(AtomicBool::new(false));
    let http = crate::http_client();
    let (jobs_tx, jobs_rx) = channel::<Job>();
    let (ready_tx, ready_rx) = channel::<Result<(), String>>();

    {
        let worker_cfg = cfg.clone();
        let worker_http = http.clone();
        let worker_status = status.clone();
        let worker_cancel = cancel.clone();
        thread::Builder::new()
            .name("reader".into())
            .spawn(move || {
                worker(
                    worker_cfg,
                    worker_http,
                    jobs_rx,
                    worker_status,
                    worker_cancel,
                    ready_tx,
                )
            })
            .map_err(|e| anyhow!("failed to spawn reader worker: {e}"))?;
    }

    // Fail fast (like the CLI) if the reader can't be opened.
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(anyhow!(e)),
        Err(_) => return Err(anyhow!("reader worker exited before it was ready")),
    }

    let shared = Shared {
        status,
        cancel,
        jobs: jobs_tx,
        cfg,
        http,
    };

    let server = Server::http(bind).map_err(|e| anyhow!("failed to bind {bind}: {e}"))?;
    println!("melon-terminal kiosk: open http://{bind}/ in a browser");
    for request in server.incoming_requests() {
        handle(request, &shared);
    }
    Ok(())
}

/// The reader worker: owns the PaSoRi and runs one job at a time.
fn worker(
    cfg: Config,
    http: reqwest::blocking::Client,
    jobs: Receiver<Job>,
    status: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
    ready: Sender<Result<(), String>>,
) {
    let mut reader = match crate::open_reader_auto() {
        Ok(r) => r,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let target = match crate::make_target() {
        Ok(t) => t,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    for job in jobs {
        cancel.store(false, Ordering::SeqCst);
        match job {
            Job::Operate { op, amount } => {
                tracing::info!(job = "operate", op = %op, amount, "kiosk job started");
                let Some((_sc, session_id, account_id)) = wait_and_auth(
                    &mut reader,
                    &target,
                    &http,
                    &cfg,
                    &cancel,
                    &status,
                    op.as_str(),
                    amount,
                ) else {
                    continue;
                };
                let card = json!({ "account_id": account_id });
                set(&status, |s| {
                    s.phase = "processing";
                    s.message = "処理中…".into();
                    s.card = Some(card.clone());
                });
                match crate::run_operation(&http, &cfg, &session_id, op, amount) {
                    Ok(result) => {
                        tracing::info!(job = "operate", op = %op, "kiosk job done");
                        set(&status, |s| done(s, Some(op.as_str()), result))
                    }
                    Err(e) => set(&status, |s| fail(s, &e)),
                }
            }
            Job::RefundLookup => {
                tracing::info!(job = "refund-lookup", "kiosk job started");
                let Some((_sc, _session, account_id)) = wait_and_auth(
                    &mut reader,
                    &target,
                    &http,
                    &cfg,
                    &cancel,
                    &status,
                    "refund",
                    None,
                ) else {
                    continue;
                };
                let card = json!({ "account_id": account_id });
                match crate::list_refundable(&http, &cfg, &account_id) {
                    Ok(list) => {
                        tracing::info!(
                            job = "refund-lookup",
                            refundable = list.as_array().map(|a| a.len()).unwrap_or(0),
                            "kiosk job done → awaiting selection"
                        );
                        set(&status, |s| {
                            s.phase = "select";
                            s.op = Some("refund");
                            s.message = "返金する支払いを選択してください".into();
                            s.card = Some(card.clone());
                            s.refundable = Some(list);
                            s.error = None;
                            s.error_code = None;
                            s.error_details = None;
                        })
                    }
                    Err(e) => set(&status, |s| fail(s, &e)),
                }
            }
            Job::RefundExec { payment_id, amount } => {
                tracing::info!(
                    job = "refund-exec",
                    %payment_id,
                    amount,
                    "kiosk job started (no card needed)"
                );
                set(&status, |s| {
                    s.phase = "processing";
                    s.op = Some("refund");
                    s.message = "返金処理中…".into();
                });
                match crate::refund(&http, &cfg, &payment_id, amount) {
                    Ok(result) => {
                        tracing::info!(job = "refund-exec", "kiosk job done");
                        set(&status, |s| done(s, Some("refund"), result))
                    }
                    Err(e) => set(&status, |s| fail(s, &e)),
                }
            }
        }
    }
}

/// Set the "waiting → authenticating" states and run the card wait + auth shared
/// by every card-present job. Returns `(system_code, session_id, account_id)` or `None`
/// (status already set to cancelled/error).
#[allow(clippy::too_many_arguments)]
fn wait_and_auth(
    reader: &mut felica_rs::prelude::Reader,
    target: &felica_rs::prelude::RemoteTarget,
    http: &reqwest::blocking::Client,
    cfg: &Config,
    cancel: &Arc<AtomicBool>,
    status: &Arc<Mutex<Status>>,
    op_label: &'static str,
    amount: Option<i64>,
) -> Option<(u16, String, String)> {
    set(status, |s| *s = Status::waiting(op_label, amount));

    // Wildcard-poll until a card is present, honouring cancel. The only retry.
    let poll = match crate::wait_for_card(reader, target, cfg.poll_interval, || {
        cancel
            .load(Ordering::SeqCst)
            .then_some(WaitAbort::Cancelled)
    }) {
        Ok(p) => p,
        Err(_) => {
            set(status, |s| *s = Status::idle("キャンセルしました"));
            return None;
        }
    };

    // A card is present: one attempt. Ask it which systems it exposes, pick the
    // first the server can authenticate, re-poll that system (each system has its
    // own IDm), then authenticate. Any failure aborts (no re-read, no re-send).
    set(status, |s| {
        s.phase = "authenticating";
        s.message = "認証中…".into();
    });
    let card = match crate::resolve_card(reader, target, &poll, &cfg.system_codes) {
        Ok(c) => c,
        Err(e) => {
            set(status, |s| fail(s, &e));
            return None;
        }
    };
    match crate::authenticate(http, cfg, reader, target, card.system_code, &card.poll) {
        Ok((session_id, account_id)) => Some((card.system_code, session_id, account_id)),
        Err(e) => {
            set(status, |s| fail(s, &e));
            None
        }
    }
}

/// Put the status into the `done` phase with a server result.
fn done(s: &mut Status, op: Option<&'static str>, result: Value) {
    s.phase = "done";
    s.op = op;
    s.message = "完了".into();
    s.result = Some(result);
    s.error = None;
    s.error_code = None;
    s.error_details = None;
}

/// Put the status into the error phase, classifying the cause into a stable code
/// (plus structured details) the UI can render in Japanese.
fn fail(s: &mut Status, err: &anyhow::Error) {
    let (code, details, message) = classify(err);
    tracing::warn!(error_code = %code, error = %message, "kiosk job failed");
    s.phase = "error";
    s.message = "エラー".into();
    s.error_code = Some(code);
    s.error_details = details;
    s.error = Some(message);
    s.result = None;
}

/// Map an operation/auth failure to a stable error code the UI localizes on.
/// A server error carries its own `code` and `details`; hardware/network failures
/// are recognized heuristically from the message.
fn classify(err: &anyhow::Error) -> (String, Option<Value>, String) {
    if let Some(se) = err.downcast_ref::<crate::ServerError>() {
        return (se.code.clone(), se.details.clone(), se.message.clone());
    }
    let message = err.to_string();
    let low = message.to_lowercase();
    let code =
        // The card exposes no system this server holds keys for.
        if low.contains("no system the server") {
            "SYSTEM_NOT_SUPPORTED"
        } else if low.contains("card exchange")
            || low.contains("transceive")
            || low.contains("timeout")
            || low.contains("request system code")
            || low.contains("polling system")
        {
            "CARD_LOST"
        } else if low.contains("401") || low.contains("unauthorized") {
            "UNAUTHORIZED"
        } else if low.contains("403") || low.contains("not active") || low.contains("forbidden") {
            "FORBIDDEN"
        } else if low.contains("request failed")
            || low.contains("sending request")
            || low.contains("connect")
            || low.contains("dns")
            || low.contains("tcp")
        {
            "NETWORK"
        } else if low.contains("authenticat") || low.contains("no candidate") {
            "AUTH_FAILED"
        } else {
            "UNKNOWN"
        };
    (code.to_string(), None, message)
}

fn set(status: &Arc<Mutex<Status>>, f: impl FnOnce(&mut Status)) {
    if let Ok(mut s) = status.lock() {
        f(&mut s);
    }
}

// ----- HTTP handling -----

fn handle(mut request: Request, shared: &Shared) {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("").to_string();
    // /status is polled every 400ms by the UI — keep it at trace, the rest at debug.
    if path == "/status" {
        tracing::trace!(method = %method, %path, "kiosk UI request");
    } else {
        tracing::debug!(method = %method, %path, "kiosk UI request");
    }

    match (&method, path.as_str()) {
        (Method::Get, "/") => {
            respond_html(request, INDEX_HTML);
        }
        (Method::Get, "/status") => {
            let body = shared
                .status
                .lock()
                .map(|s| s.to_json())
                .unwrap_or(Value::Null);
            respond_json(request, 200, body);
        }
        (Method::Get, "/me") => {
            // Proxy the merchant's own profile (settlement, fee, credit, …). The
            // API key stays in this process; the browser never sees it.
            match crate::get(
                &shared.http,
                &shared.cfg.server,
                "/v1/me",
                &shared.cfg.api_key,
            ) {
                Ok(info) => respond_json(request, 200, info),
                Err(e) => respond_json(request, 502, json!({ "error": e.to_string() })),
            }
        }
        (Method::Get, "/op/balance") => {
            let (code, body) = start_operate(shared, Op::Balance, None);
            respond_json(request, code, body);
        }
        (Method::Post, "/op/pay") => {
            let amount = read_amount(&mut request);
            let (code, body) = start_operate(shared, Op::Pay, amount);
            respond_json(request, code, body);
        }
        (Method::Post, "/op/topup") => {
            let amount = read_amount(&mut request);
            let (code, body) = start_operate(shared, Op::Topup, amount);
            respond_json(request, code, body);
        }
        (Method::Post, "/op/refund-lookup") => {
            let (code, body) = start_lookup(shared);
            respond_json(request, code, body);
        }
        (Method::Post, "/op/refund") => {
            let (payment_id, amount) = read_refund_req(&mut request);
            let (code, body) = start_refund_exec(shared, payment_id, amount);
            respond_json(request, code, body);
        }
        (Method::Post, "/cancel") => {
            shared.cancel.store(true, Ordering::SeqCst);
            respond_json(request, 200, json!({ "ok": true }));
        }
        (Method::Post, "/reset") => {
            // Clear a finished/selecting job back to idle so a new sale can start.
            set(&shared.status, |s| {
                if !s.is_busy() {
                    *s = Status::idle("待機中");
                }
            });
            respond_json(request, 200, json!({ "ok": true }));
        }
        _ => respond_json(request, 404, json!({ "error": "not found" })),
    }
}

/// Reject a new card-present job if one is running or a refund selection is open.
fn blocked(s: &Status) -> Option<(u16, Value)> {
    if s.is_busy() {
        Some((409, json!({ "error": "別の操作を実行中です" })))
    } else if s.phase == "select" {
        Some((409, json!({ "error": "返金の選択中です" })))
    } else {
        None
    }
}

/// Enqueue a pay/topup/balance job unless one is running or the amount is invalid.
fn start_operate(shared: &Shared, op: Op, amount: Option<i64>) -> (u16, Value) {
    if op.is_spend() && !matches!(amount, Some(a) if a > 0) {
        return (400, json!({ "error": "金額を入力してください" }));
    }
    {
        let mut s = match shared.status.lock() {
            Ok(s) => s,
            Err(_) => return (500, json!({ "error": "internal error" })),
        };
        if let Some(rejection) = blocked(&s) {
            return rejection;
        }
        *s = Status::waiting(op.as_str(), amount); // optimistic; worker confirms
    }
    shared.cancel.store(false, Ordering::SeqCst);
    if shared.jobs.send(Job::Operate { op, amount }).is_err() {
        return (500, json!({ "error": "reader worker unavailable" }));
    }
    (202, json!({ "started": true }))
}

/// Enqueue the refund lookup (phase 1): wait for a card, then list refundable
/// payments.
fn start_lookup(shared: &Shared) -> (u16, Value) {
    {
        let mut s = match shared.status.lock() {
            Ok(s) => s,
            Err(_) => return (500, json!({ "error": "internal error" })),
        };
        if let Some(rejection) = blocked(&s) {
            return rejection;
        }
        *s = Status::waiting("refund", None);
    }
    shared.cancel.store(false, Ordering::SeqCst);
    if shared.jobs.send(Job::RefundLookup).is_err() {
        return (500, json!({ "error": "reader worker unavailable" }));
    }
    (202, json!({ "started": true }))
}

/// Enqueue the refund itself (phase 2): only valid while a selection is open.
fn start_refund_exec(
    shared: &Shared,
    payment_id: Option<String>,
    amount: Option<i64>,
) -> (u16, Value) {
    let payment_id = match payment_id {
        Some(p) if !p.is_empty() => p,
        _ => return (400, json!({ "error": "返金対象が指定されていません" })),
    };
    if matches!(amount, Some(a) if a <= 0) {
        return (400, json!({ "error": "金額が不正です" }));
    }
    {
        let mut s = match shared.status.lock() {
            Ok(s) => s,
            Err(_) => return (500, json!({ "error": "internal error" })),
        };
        if s.phase != "select" {
            return (409, json!({ "error": "返金対象が選択されていません" }));
        }
        s.phase = "processing";
        s.op = Some("refund");
        s.message = "返金処理中…".into();
        s.result = None;
        s.error = None;
        s.error_code = None;
        s.error_details = None;
    }
    if shared
        .jobs
        .send(Job::RefundExec { payment_id, amount })
        .is_err()
    {
        return (500, json!({ "error": "reader worker unavailable" }));
    }
    (202, json!({ "started": true }))
}

/// Read `{ "amount": <int> }` from the request body.
fn read_amount(request: &mut Request) -> Option<i64> {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body).ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    value["amount"].as_i64()
}

/// Read `{ "payment_id": <str>, "amount"?: <int> }` from the request body.
fn read_refund_req(request: &mut Request) -> (Option<String>, Option<i64>) {
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return (None, None);
    }
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    (
        value["payment_id"].as_str().map(str::to_string),
        value["amount"].as_i64(),
    )
}

fn respond_json(request: Request, code: u16, body: Value) {
    let header = Header::from_bytes(
        &b"Content-Type"[..],
        &b"application/json; charset=utf-8"[..],
    )
    .expect("valid header");
    let response = Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(header);
    let _ = request.respond(response);
}

fn respond_html(request: Request, html: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
        .expect("valid header");
    let _ = request.respond(Response::from_string(html).with_header(header));
}

#[cfg(test)]
mod tests {
    use super::classify;
    use crate::ServerError;
    use serde_json::json;

    #[test]
    fn server_error_keeps_code_and_details() {
        let err = anyhow::Error::new(ServerError {
            status: 422,
            code: "INSUFFICIENT_FUNDS".into(),
            message: "insufficient funds".into(),
            details: Some(json!({ "available": 700, "requested": 5000 })),
        });
        let (code, details, _) = classify(&err);
        assert_eq!(code, "INSUFFICIENT_FUNDS");
        assert_eq!(details.unwrap()["available"], 700);
    }

    #[test]
    fn hardware_and_network_failures_are_categorized() {
        let cases = [
            (
                "system 0x0003: card exchange failed: no response",
                "CARD_LOST",
            ),
            ("server returned 401: unauthorized", "UNAUTHORIZED"),
            ("server returned 403: merchant is not active", "FORBIDDEN"),
            ("request failed: error sending request", "NETWORK"),
            (
                "card present but no candidate system code authenticated it",
                "AUTH_FAILED",
            ),
            ("something unexpected", "UNKNOWN"),
        ];
        for (msg, want) in cases {
            let (code, details, _) = classify(&anyhow::anyhow!("{msg}"));
            assert_eq!(code, want, "message: {msg}");
            assert!(details.is_none());
        }
    }
}
