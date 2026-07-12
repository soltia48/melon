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
use std::time::Duration;

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

/// The runtime-settable merchant credentials. The API key can be supplied on the
/// CLI/env at startup, or entered from the kiosk screen (`POST /config`); either
/// way it is validated by fetching the server's usable system codes, and the key
/// never leaves this process (the browser only ever sees whether one is set).
struct Creds {
    api_key: Option<String>,
    /// The systems the server holds keys for (fetched when the key is adopted).
    system_codes: Vec<u16>,
}

/// Shared handles the HTTP loop and the reader worker both hold.
#[derive(Clone)]
struct Shared {
    status: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
    jobs: Sender<Job>,
    /// Immutable connection settings.
    server: String,
    poll_interval: Duration,
    /// The mutable, runtime-settable merchant credentials.
    creds: Arc<Mutex<Creds>>,
    http: reqwest::blocking::Client,
}

/// Where the kiosk persists its merchant API key between runs, e.g.
/// `~/.config/melon-terminal/credentials.json` (platform-specific).
fn creds_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("melon-terminal").join("credentials.json"))
}

/// Load a previously-saved API key, if any. Any error (missing/corrupt) → `None`.
fn load_saved_key() -> Option<String> {
    let text = std::fs::read_to_string(creds_path()?).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["api_key"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Persist the API key (and the server it was validated against) so the next
/// launch is already configured. The file is owner-only (0600 on Unix) — it holds
/// a secret. Best-effort; the caller logs any error and continues.
fn save_key(server: &str, api_key: &str) -> std::io::Result<()> {
    let path = creds_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = json!({ "server": server, "api_key": api_key }).to_string();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // applies on creation
            .open(&path)?;
        f.write_all(body.as_bytes())?;
        // Tighten an already-existing file too (mode() only affects creation).
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, body)?;
    }
    Ok(())
}

/// A browsable URL for the bind address. A wildcard host (`0.0.0.0` / `::`) isn't
/// browsable, so map it to loopback; a specific host/IP is used as-is.
fn browse_url(bind: &str) -> String {
    let (host, port) = bind.rsplit_once(':').unwrap_or((bind, ""));
    let host = match host {
        "0.0.0.0" | "" | "::" | "[::]" => "127.0.0.1",
        h => h,
    };
    if port.is_empty() {
        format!("http://{host}/")
    } else {
        format!("http://{host}:{port}/")
    }
}

/// Snapshot the current credentials into a [`Config`], or `None` if no API key is
/// set yet. Combines the immutable connection settings with the live key.
fn snapshot(creds: &Arc<Mutex<Creds>>, server: &str, poll_interval: Duration) -> Option<Config> {
    let c = creds.lock().ok()?;
    let api_key = c.api_key.clone()?;
    Some(Config {
        server: server.to_string(),
        api_key,
        system_codes: c.system_codes.clone(),
        poll_interval,
    })
}

impl Shared {
    /// A [`Config`] snapshot for a reader-free server call, or `None` if unset.
    fn config(&self) -> Option<Config> {
        snapshot(&self.creds, &self.server, self.poll_interval)
    }

    /// Whether an API key has been set.
    fn configured(&self) -> bool {
        self.creds
            .lock()
            .map(|c| c.api_key.is_some())
            .unwrap_or(false)
    }
}

/// Run the kiosk: open the reader on a worker thread, then serve the UI + local
/// API. Blocks until the process is stopped.
///
/// `initial_api_key` is the key supplied on the CLI/env, if any. Unlike one-shot
/// mode the kiosk can start WITHOUT one: it falls back to a previously saved key,
/// and otherwise the operator sets it from the settings screen. A key is
/// validated now (fail-soft — a bad key just leaves the kiosk unconfigured, and
/// the screen can fix it). When `open_browser` is set, the UI is opened in the
/// operator's default browser once the server is up.
pub fn run(
    server_url: String,
    initial_api_key: Option<String>,
    poll_interval: Duration,
    bind: &str,
    open_browser: bool,
) -> Result<()> {
    let status = Arc::new(Mutex::new(Status::idle("待機中")));
    let cancel = Arc::new(AtomicBool::new(false));
    let http = crate::http_client();
    let creds = Arc::new(Mutex::new(Creds {
        api_key: None,
        system_codes: Vec::new(),
    }));

    // Prefer an explicit --api-key; otherwise reuse the last key saved from the UI.
    if let Some(key) = initial_api_key.or_else(load_saved_key) {
        match crate::fetch_system_codes(&http, &server_url, &key) {
            Ok(system_codes) => {
                *creds.lock().unwrap() = Creds {
                    api_key: Some(key),
                    system_codes,
                };
            }
            Err(e) => tracing::warn!(
                error = %e,
                "the configured API key could not be validated; set it from the kiosk settings screen"
            ),
        }
    }

    let (jobs_tx, jobs_rx) = channel::<Job>();
    let (ready_tx, ready_rx) = channel::<Result<(), String>>();

    {
        let worker_server = server_url.clone();
        let worker_creds = creds.clone();
        let worker_http = http.clone();
        let worker_status = status.clone();
        let worker_cancel = cancel.clone();
        thread::Builder::new()
            .name("reader".into())
            .spawn(move || {
                worker(
                    worker_server,
                    poll_interval,
                    worker_creds,
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
        server: server_url,
        poll_interval,
        creds,
        http,
    };

    let server = Server::http(bind).map_err(|e| anyhow!("failed to bind {bind}: {e}"))?;
    let url = browse_url(bind);
    println!("melon-terminal kiosk: open {url} in a browser");
    if !shared.configured() {
        println!("  (API キー未設定 — 画面の「⚙ 設定」から入力してください)");
    }

    // Open the UI in the default browser. Fire-and-forget on its own thread so a
    // slow launcher never delays serving; a failure (e.g. headless) is non-fatal.
    if open_browser {
        thread::spawn(move || {
            if let Err(e) = open::that(&url) {
                tracing::warn!(error = %e, %url, "could not open the default browser; open it manually");
            }
        });
    }

    for request in server.incoming_requests() {
        handle(request, &shared);
    }
    Ok(())
}

/// The reader worker: owns the PaSoRi and runs one job at a time.
#[allow(clippy::too_many_arguments)]
fn worker(
    server: String,
    poll_interval: Duration,
    creds: Arc<Mutex<Creds>>,
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
        // Snapshot the credentials for this job — the operator can set/replace the
        // API key between jobs from the settings screen.
        let Some(cfg) = snapshot(&creds, &server, poll_interval) else {
            set(&status, |s| {
                s.phase = "error";
                s.message = "エラー".into();
                s.error_code = Some("NOT_CONFIGURED".into());
                s.error_details = None;
                s.error = Some("API キーが設定されていません".into());
                s.result = None;
            });
            continue;
        };
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
        (Method::Get, "/config") => {
            // Only ever expose WHETHER a key is set — never the key itself.
            respond_json(
                request,
                200,
                json!({ "configured": shared.configured(), "server": shared.server }),
            );
        }
        (Method::Post, "/config") => {
            let (code, body) = set_config(&mut request, shared);
            respond_json(request, code, body);
        }
        (Method::Get, "/me") => {
            // Proxy the merchant's own profile (settlement, fee, credit, …). The
            // API key stays in this process; the browser never sees it.
            let Some(cfg) = shared.config() else {
                return respond_json(
                    request,
                    409,
                    json!({ "error": "API キーが設定されていません" }),
                );
            };
            match crate::get(&shared.http, &cfg.server, "/v1/me", &cfg.api_key) {
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
    if !shared.configured() {
        return (409, json!({ "error": "API キーが設定されていません" }));
    }
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
    if !shared.configured() {
        return (409, json!({ "error": "API キーが設定されていません" }));
    }
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

/// Set (and validate) the merchant API key from the UI. The key is checked by
/// fetching the server's usable system codes — which requires a valid merchant
/// key — and, on success, it and the fetched system codes replace the current
/// credentials. Refused while a job is in flight. The key is never echoed back.
fn set_config(request: &mut Request, shared: &Shared) -> (u16, Value) {
    if let Ok(s) = shared.status.lock()
        && (s.is_busy() || s.phase == "select")
    {
        return (409, json!({ "error": "操作中は API キーを変更できません" }));
    }
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return (400, json!({ "error": "リクエストを読み取れません" }));
    }
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let key = value["api_key"].as_str().unwrap_or("").trim().to_string();
    if key.is_empty() {
        return (400, json!({ "error": "API キーを入力してください" }));
    }
    match crate::fetch_system_codes(&shared.http, &shared.server, &key) {
        Ok(system_codes) => {
            if let Ok(mut c) = shared.creds.lock() {
                c.api_key = Some(key.clone());
                c.system_codes = system_codes;
            }
            // Persist so the next launch is already configured. Non-fatal on error
            // (the key still works for this session).
            if let Err(e) = save_key(&shared.server, &key) {
                tracing::warn!(error = %e, "could not persist the API key (this session only)");
            } else {
                tracing::info!("kiosk API key set from the settings screen and saved");
            }
            (200, json!({ "configured": true }))
        }
        Err(e) => {
            // Never surface the raw key; give the operator an actionable reason.
            let msg = if let Some(se) = e.downcast_ref::<crate::ServerError>() {
                match se.status {
                    401 | 403 => "API キーが正しくありません".to_string(),
                    _ => format!("サーバエラー: {}", se.message),
                }
            } else {
                "サーバに接続できません".to_string()
            };
            tracing::warn!(error = %e, "kiosk API key validation failed");
            (400, json!({ "configured": false, "error": msg }))
        }
    }
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
    use super::{Creds, browse_url, classify, snapshot};
    use crate::ServerError;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn browse_url_maps_wildcard_to_loopback() {
        assert_eq!(browse_url("127.0.0.1:8899"), "http://127.0.0.1:8899/");
        assert_eq!(browse_url("0.0.0.0:8899"), "http://127.0.0.1:8899/");
        assert_eq!(browse_url("localhost:8899"), "http://localhost:8899/");
        assert_eq!(browse_url("192.168.1.5:9000"), "http://192.168.1.5:9000/");
        // IPv6 loopback is browsable as-is; the unspecified address maps to v4 loopback.
        assert_eq!(browse_url("[::1]:8899"), "http://[::1]:8899/");
        assert_eq!(browse_url("[::]:8899"), "http://127.0.0.1:8899/");
    }

    #[test]
    fn snapshot_reflects_credential_state() {
        let creds = Arc::new(Mutex::new(Creds {
            api_key: None,
            system_codes: Vec::new(),
        }));
        let server = "http://127.0.0.1:8080".to_string();
        let poll = Duration::from_millis(500);

        // No key set → no runnable config.
        assert!(snapshot(&creds, &server, poll).is_none());

        // Once a key is adopted, a snapshot carries it and the fetched systems.
        *creds.lock().unwrap() = Creds {
            api_key: Some("secret-key".into()),
            system_codes: vec![0x0003, 0xFE00],
        };
        let cfg = snapshot(&creds, &server, poll).expect("configured");
        assert_eq!(cfg.api_key, "secret-key");
        assert_eq!(cfg.system_codes, vec![0x0003, 0xFE00]);
        assert_eq!(cfg.server, server);
        assert_eq!(cfg.poll_interval, poll);
    }

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
