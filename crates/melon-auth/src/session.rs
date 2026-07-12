//! Session management: the per-session worker thread and the [`SessionManager`].
//!
//! Each session owns an OS worker thread that drives `felica-rs`'s high-level
//! `FelicaStandard` API against a [`RelayDriver`]. Because a single
//! `mutual_authentication` call spans two card round-trips — and therefore two
//! HTTP requests — the worker blocks inside the driver's `transceive` between
//! requests. Coordination uses three unbounded channels:
//!
//! - `control` (HTTP → worker): start a mutual-auth or an encrypted exchange.
//! - `card` (HTTP → relay driver): a card response to feed the pending transceive.
//! - `out` (worker/driver → HTTP): the next frame to relay, or a final result.
//!
//! Every client request delivers exactly one input (a control command or a card
//! response) and consumes exactly one `Out`, so the streams stay in lock-step.
//! Per-session serialization is enforced by an async mutex; the handler side
//! tracks whether the worker is next expecting a control command or a card
//! response to route each request and reject out-of-order ones cleanly.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use felica_rs::felica_standard::{FelicaStandard, ServiceCode};
use serde_json::{Value, json};
use tokio::sync::Mutex as TokioMutex;

use crate::error::{ProtocolError, map_felica_error};
use crate::keystore::KeyStore;
use crate::relay_driver::{Out, RelayDriver};

/// A high-level command sent from an HTTP handler to a session worker.
enum Control {
    StartAuth {
        system_code: u16,
        areas: Vec<u16>,
        services: Vec<u16>,
    },
    StartExchange {
        cmd_code: u8,
        payload: Vec<u8>,
        /// Client-provided card timeout; `None` lets the worker pick a default.
        timeout_ms: Option<u16>,
    },
}

/// What the worker is next expecting from the client.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A new control command (start auth / start exchange).
    Control,
    /// A card response feeding the pending transceive.
    Card,
}

struct SessionInner {
    expect: Expect,
    /// Number of command frames emitted since the current auth started
    /// (used to label `auth1` vs `auth2`).
    auth_frames: u8,
}

struct Session {
    idm: [u8; 8],
    pmm: [u8; 8],
    control_tx: flume::Sender<Control>,
    card_tx: flume::Sender<Vec<u8>>,
    out_rx: flume::Receiver<Out>,
    inner: TokioMutex<SessionInner>,
    last_seen: StdMutex<Instant>,
    /// The card-verified IDi (issue_id), set once mutual authentication
    /// completes. `None` until then. This is what makes the session a trusted,
    /// card-present assertion of identity.
    idi: StdMutex<Option<[u8; 8]>>,
    /// The system code the session authenticated under, set when auth starts.
    /// An account is identified by the pair `(system_code, idi)`.
    system_code: StdMutex<Option<u16>>,
    /// One-shot spend capability: a single money operation may be committed per
    /// authentication. Set to `true` once claimed, binding each charge to a
    /// fresh physical tap and defeating `session_id` replay.
    spend_consumed: StdMutex<bool>,
}

/// Parsed input for `POST /mutual-authentication`.
#[derive(Debug, Default)]
pub struct MutualAuthInput {
    pub session_id: Option<String>,
    pub idm: Option<[u8; 8]>,
    pub pmm: Option<[u8; 8]>,
    pub system_code: Option<u16>,
    pub areas: Option<Vec<u16>>,
    pub services: Option<Vec<u16>>,
    pub card_response: Option<Vec<u8>>,
}

/// Parsed input for `POST /encryption-exchange`.
#[derive(Debug, Default)]
pub struct EncryptionExchangeInput {
    pub session_id: Option<String>,
    pub cmd_code: Option<u8>,
    pub payload: Option<Vec<u8>>,
    /// Client-provided card timeout in seconds.
    pub timeout: Option<f64>,
    pub card_response: Option<Vec<u8>>,
}

/// In-memory manager of live FeliCa sessions.
pub struct SessionManager {
    sessions: StdMutex<HashMap<String, Arc<Session>>>,
    keystore: Arc<KeyStore>,
    allowed_cmd_codes: Option<HashSet<u8>>,
    ttl: Duration,
    max_sessions: usize,
}

impl SessionManager {
    pub fn new(
        keystore: Arc<KeyStore>,
        allowed_cmd_codes: Option<HashSet<u8>>,
        ttl: Duration,
        max_sessions: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: StdMutex::new(HashMap::new()),
            keystore,
            allowed_cmd_codes,
            ttl,
            max_sessions,
        })
    }

    /// Spawn the background task that reaps idle sessions. Must be called from
    /// within a Tokio runtime.
    pub fn spawn_reaper(self: Arc<Self>) {
        let ttl = self.ttl;
        let interval = (ttl / 2).max(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let now = Instant::now();
                let mut sessions = self.sessions.lock().unwrap();
                let before = sessions.len();
                sessions.retain(|_, session| {
                    let last = *session.last_seen.lock().unwrap();
                    now.duration_since(last) < ttl
                });
                let reaped = before - sessions.len();
                if reaped > 0 {
                    tracing::debug!(reaped, live = sessions.len(), "reaped idle sessions");
                }
            }
        });
    }

    /// Number of live sessions (for diagnostics / health).
    pub fn live_sessions(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }

    /// The FeliCa system codes this server holds DES keys for — i.e. the systems
    /// a card can be authenticated under. Terminals fetch this to know which of a
    /// card's systems they may select.
    pub fn system_codes(&self) -> Vec<u16> {
        self.keystore.system_codes()
    }

    /// The card-verified account `(system_code, idm, idi)` for a session that
    /// completed mutual authentication, or `None` if unknown or not yet
    /// authenticated. The IDm is the one the terminal presented for the chosen
    /// system at polling time.
    pub fn authenticated_account(&self, session_id: &str) -> Option<(u16, [u8; 8], [u8; 8])> {
        let session = self.sessions.lock().unwrap().get(session_id).cloned()?;
        let idi = (*session.idi.lock().unwrap())?;
        let system_code = (*session.system_code.lock().unwrap())?;
        Some((system_code, session.idm, idi))
    }

    /// Atomically claim the one-shot spend capability of an authenticated
    /// session, returning the verified account `(system_code, idm, idi)` exactly
    /// once. A second claim (or a claim on an unauthenticated/unknown session)
    /// is refused. This binds each money operation to a real, fresh
    /// authentication and prevents replaying a captured `session_id`.
    pub fn consume_spend(
        &self,
        session_id: &str,
    ) -> Result<(u16, [u8; 8], [u8; 8]), ProtocolError> {
        let session = self.get_session(session_id)?;
        let idi = (*session.idi.lock().unwrap())
            .ok_or_else(|| ProtocolError::forbidden("session is not authenticated"))?;
        let system_code = (*session.system_code.lock().unwrap())
            .ok_or_else(|| ProtocolError::forbidden("session is not authenticated"))?;
        let mut consumed = session.spend_consumed.lock().unwrap();
        if *consumed {
            return Err(ProtocolError::forbidden(
                "session spend capability already used",
            ));
        }
        *consumed = true;
        Ok((system_code, session.idm, idi))
    }

    fn get_session(&self, session_id: &str) -> Result<Arc<Session>, ProtocolError> {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .ok_or_else(|| ProtocolError::not_found("unknown session_id"))
    }

    fn get_or_create_session(
        &self,
        session_id: &Option<String>,
        idm: &Option<[u8; 8]>,
        pmm: &Option<[u8; 8]>,
    ) -> Result<(String, Arc<Session>, bool), ProtocolError> {
        if let Some(sid) = session_id {
            let session = self.get_session(sid)?;
            if let Some(idm) = idm
                && *idm != session.idm
            {
                return Err(ProtocolError::bad_request(
                    "idm does not match existing session",
                ));
            }
            if let Some(pmm) = pmm
                && *pmm != session.pmm
            {
                return Err(ProtocolError::bad_request(
                    "pmm does not match existing session",
                ));
            }
            return Ok((sid.clone(), session, false));
        }

        let idm = idm.ok_or_else(|| {
            ProtocolError::bad_request("idm and pmm are required to start a session")
        })?;
        let pmm = pmm.ok_or_else(|| {
            ProtocolError::bad_request("idm and pmm are required to start a session")
        })?;
        let (id, session) = self.create_session(idm, pmm)?;
        Ok((id, session, true))
    }

    fn create_session(
        &self,
        idm: [u8; 8],
        pmm: [u8; 8],
    ) -> Result<(String, Arc<Session>), ProtocolError> {
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.len() >= self.max_sessions {
            return Err(ProtocolError::new(503, "too many active sessions"));
        }

        let (control_tx, control_rx) = flume::unbounded::<Control>();
        let (card_tx, card_rx) = flume::unbounded::<Vec<u8>>();
        let (out_tx, out_rx) = flume::unbounded::<Out>();
        let keystore = Arc::clone(&self.keystore);
        std::thread::Builder::new()
            .name("felica-session".into())
            .spawn(move || run_session(idm, pmm, keystore, control_rx, card_rx, out_tx))
            .map_err(|e| ProtocolError::internal(format!("failed to spawn session worker: {e}")))?;

        let session = Arc::new(Session {
            idm,
            pmm,
            control_tx,
            card_tx,
            out_rx,
            inner: TokioMutex::new(SessionInner {
                expect: Expect::Control,
                auth_frames: 0,
            }),
            last_seen: StdMutex::new(Instant::now()),
            idi: StdMutex::new(None),
            system_code: StdMutex::new(None),
            spend_consumed: StdMutex::new(false),
        });
        let id = new_session_id();
        sessions.insert(id.clone(), Arc::clone(&session));
        Ok((id, session))
    }

    /// Drive `POST /mutual-authentication`.
    pub async fn handle_mutual_authentication(
        &self,
        input: MutualAuthInput,
    ) -> Result<Value, ProtocolError> {
        let (session_id, session, created) =
            self.get_or_create_session(&input.session_id, &input.idm, &input.pmm)?;
        let mut inner = session.inner.lock().await;
        let starting = inner.expect == Expect::Control;

        let out = if starting {
            if input.card_response.is_some() {
                return Err(ProtocolError::bad_request(
                    "card_response not expected at start of authentication",
                ));
            }
            let system_code = input
                .system_code
                .ok_or_else(|| ProtocolError::bad_request("system_code is required"))?;
            let areas = require_nonempty(input.areas, "areas")?;
            let services = require_nonempty(input.services, "services")?;
            inner.auth_frames = 0;
            // Bind the authenticating system code to the session; the resulting
            // IDi is only meaningful together with it.
            *session.system_code.lock().unwrap() = Some(system_code);
            session
                .control_tx
                .send(Control::StartAuth {
                    system_code,
                    areas,
                    services,
                })
                .map_err(|_| ProtocolError::internal("session worker unavailable"))?;
            recv_out(&session).await?
        } else {
            let card = input.card_response.ok_or_else(|| {
                ProtocolError::bad_request("card_response is required to continue authentication")
            })?;
            session
                .card_tx
                .send(card)
                .map_err(|_| ProtocolError::internal("session worker unavailable"))?;
            recv_out(&session).await?
        };

        session.touch();

        let mut value = match out {
            Out::Frame {
                code,
                frame,
                timeout_ms,
            } => {
                inner.expect = Expect::Card;
                inner.auth_frames = inner.auth_frames.saturating_add(1);
                let step = if inner.auth_frames <= 1 {
                    "auth1"
                } else {
                    "auth2"
                };
                json!({
                    "phase": "mutual_authentication",
                    "step": step,
                    "command": command_json(code, &frame, timeout_ms),
                })
            }
            Out::AuthComplete {
                issue_id,
                issue_parameter,
            } => {
                inner.expect = Expect::Control;
                // Retain the card-verified IDi so payment operations can be
                // bound to this authenticated session.
                *session.idi.lock().unwrap() = Some(issue_id);
                json!({
                    "phase": "mutual_authentication",
                    "step": "complete",
                    "result": {
                        "issue_id": hex::encode(issue_id),
                        "issue_parameter": hex::encode(issue_parameter),
                    },
                })
            }
            Out::ExchangeResult { .. } => {
                inner.expect = Expect::Control;
                return Err(ProtocolError::internal(
                    "unexpected exchange result during authentication",
                ));
            }
            Out::Error(err) => {
                inner.expect = Expect::Control;
                return Err(err);
            }
        };

        value["session_id"] = json!(session_id);
        if starting {
            value["session_created"] = json!(created);
        }
        Ok(value)
    }

    /// Drive `POST /encryption-exchange`.
    pub async fn handle_encryption_exchange(
        &self,
        input: EncryptionExchangeInput,
    ) -> Result<Value, ProtocolError> {
        let session_id = input
            .session_id
            .ok_or_else(|| ProtocolError::bad_request("session_id is required"))?;
        let session = self.get_session(&session_id)?;
        let mut inner = session.inner.lock().await;
        let starting = inner.expect == Expect::Control;

        let out = if starting {
            let cmd_code = input
                .cmd_code
                .ok_or_else(|| ProtocolError::bad_request("cmd_code is required"))?;
            if let Some(allowed) = &self.allowed_cmd_codes
                && !allowed.contains(&cmd_code)
            {
                return Err(ProtocolError::forbidden(format!(
                    "cmd_code 0x{cmd_code:02X} is not permitted"
                )));
            }
            let payload = input
                .payload
                .ok_or_else(|| ProtocolError::bad_request("payload is required"))?;
            let timeout_ms = input.timeout.map(secs_to_ms);
            session
                .control_tx
                .send(Control::StartExchange {
                    cmd_code,
                    payload,
                    timeout_ms,
                })
                .map_err(|_| ProtocolError::internal("session worker unavailable"))?;
            recv_out(&session).await?
        } else {
            let card = input.card_response.ok_or_else(|| {
                ProtocolError::bad_request(
                    "card_response is required to complete the pending exchange",
                )
            })?;
            session
                .card_tx
                .send(card)
                .map_err(|_| ProtocolError::internal("session worker unavailable"))?;
            recv_out(&session).await?
        };

        session.touch();

        let mut value = match out {
            Out::Frame {
                code,
                frame,
                timeout_ms,
            } => {
                inner.expect = Expect::Card;
                json!({
                    "phase": "encryption_exchange",
                    "command": command_json(code, &frame, timeout_ms),
                })
            }
            Out::ExchangeResult { response } => {
                inner.expect = Expect::Control;
                json!({
                    "phase": "encryption_exchange",
                    "response": hex::encode(response),
                })
            }
            Out::AuthComplete { .. } => {
                inner.expect = Expect::Control;
                return Err(ProtocolError::internal(
                    "unexpected auth completion during exchange",
                ));
            }
            Out::Error(err) => {
                inner.expect = Expect::Control;
                return Err(err);
            }
        };

        value["session_id"] = json!(session_id);
        Ok(value)
    }
}

impl Session {
    fn touch(&self) {
        *self.last_seen.lock().unwrap() = Instant::now();
    }
}

async fn recv_out(session: &Session) -> Result<Out, ProtocolError> {
    session
        .out_rx
        .recv_async()
        .await
        .map_err(|_| ProtocolError::internal("session worker terminated"))
}

fn require_nonempty(value: Option<Vec<u16>>, name: &str) -> Result<Vec<u16>, ProtocolError> {
    match value {
        Some(list) if !list.is_empty() => Ok(list),
        _ => Err(ProtocolError::bad_request(format!(
            "{name} must be a non-empty list"
        ))),
    }
}

fn command_json(code: u8, frame: &[u8], timeout_ms: u16) -> Value {
    json!({
        "code": code,
        "frame": hex::encode(frame),
        "timeout": ms_to_secs(timeout_ms),
    })
}

fn ms_to_secs(ms: u16) -> f64 {
    ms as f64 / 1000.0
}

fn secs_to_ms(secs: f64) -> u16 {
    (secs * 1000.0).ceil().clamp(0.0, u16::MAX as f64) as u16
}

fn new_session_id() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// The per-session worker loop: authenticate, then serve encrypted exchanges,
/// reusing `felica-rs`'s `FelicaStandard` verbatim through the relay driver.
fn run_session(
    idm: [u8; 8],
    pmm: [u8; 8],
    keystore: Arc<KeyStore>,
    control_rx: flume::Receiver<Control>,
    card_rx: flume::Receiver<Vec<u8>>,
    out_tx: flume::Sender<Out>,
) {
    let mut driver = RelayDriver::new(idm.to_vec(), pmm.to_vec(), out_tx.clone(), card_rx);
    let (mut felica, poll) = match FelicaStandard::polling(&mut driver, "212F", 0xFFFF, 0x00, 0x00)
    {
        Ok(value) => value,
        Err(err) => {
            let _ = out_tx.send(Out::Error(map_felica_error(&err)));
            return;
        }
    };

    while let Ok(control) = control_rx.recv() {
        match control {
            Control::StartAuth {
                system_code,
                areas,
                services,
            } => {
                let derived =
                    match keystore.derive_service_keys(system_code, &idm, &areas, &services) {
                        Ok(keys) => keys,
                        Err(err) => {
                            let _ = out_tx.send(Out::Error(err));
                            continue;
                        }
                    };
                let service_codes: Vec<ServiceCode> = services
                    .iter()
                    .map(|code| ServiceCode::new(*code))
                    .collect();
                let out = match felica.mutual_authentication(
                    &areas,
                    &service_codes,
                    &derived.group,
                    &derived.user,
                ) {
                    Ok(result) => Out::AuthComplete {
                        issue_id: result.issue_id,
                        issue_parameter: result.issue_parameter,
                    },
                    Err(err) => Out::Error(map_felica_error(&err)),
                };
                let _ = out_tx.send(out);
            }
            Control::StartExchange {
                cmd_code,
                payload,
                timeout_ms,
            } => {
                if felica.authenticated_context().is_none() {
                    let _ = out_tx.send(Out::Error(ProtocolError::bad_request(
                        "session is not authenticated",
                    )));
                    continue;
                }
                let timeout = timeout_ms.unwrap_or_else(|| poll.read_timeout_ms(1));
                let out = match felica.secure_transceive(cmd_code, &payload, timeout) {
                    Ok(response) => Out::ExchangeResult { response },
                    Err(err) => Out::Error(map_felica_error(&err)),
                };
                let _ = out_tx.send(out);
            }
        }
    }
}
