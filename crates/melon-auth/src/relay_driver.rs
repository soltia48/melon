//! A [`FelicaDriver`] that relays every `transceive` to the HTTP client.
//!
//! Instead of talking to a USB reader, [`RelayDriver::transceive`] pushes the
//! command frame onto the outbound channel (so the HTTP handler can return it to
//! the client) and blocks on the inbound channel until the client posts back the
//! card's response. `detect_type_f` never hits the wire: it simply reports the
//! IDm/PMm the client supplied when the session was created.

use felica_rs::felica_standard::{FelicaDriver, Type3TagPollingResult};
use felica_rs::{DriverError, RemoteTarget};

use crate::error::ProtocolError;

/// A message produced by the session worker for the HTTP handler to return.
///
/// Exactly one `Out` is emitted per client request (see [`crate::session`]).
#[derive(Debug)]
pub enum Out {
    /// A command frame the client must relay to the card.
    Frame {
        /// FeliCa command code (frame byte 1).
        code: u8,
        /// The full length-prefixed command frame.
        frame: Vec<u8>,
        /// Suggested time to wait for the card response, in milliseconds.
        timeout_ms: u16,
    },
    /// Mutual authentication finished successfully.
    AuthComplete {
        issue_id: [u8; 8],
        issue_parameter: [u8; 8],
    },
    /// An encrypted exchange finished; `response` is the decrypted payload.
    ExchangeResult { response: Vec<u8> },
    /// The operation failed.
    Error(ProtocolError),
}

/// Relay driver bridging `felica-rs` and the HTTP request/response cycle.
pub struct RelayDriver {
    idm: Vec<u8>,
    pmm: Vec<u8>,
    out_tx: flume::Sender<Out>,
    card_rx: flume::Receiver<Vec<u8>>,
}

impl RelayDriver {
    pub fn new(
        idm: Vec<u8>,
        pmm: Vec<u8>,
        out_tx: flume::Sender<Out>,
        card_rx: flume::Receiver<Vec<u8>>,
    ) -> Self {
        Self {
            idm,
            pmm,
            out_tx,
            card_rx,
        }
    }
}

impl FelicaDriver for RelayDriver {
    fn detect_type_f(
        &mut self,
        _target: &RemoteTarget,
        _system_code: u16,
        _request_code: u8,
        _time_slots: u8,
    ) -> Result<Type3TagPollingResult, DriverError> {
        Ok(Type3TagPollingResult {
            idm: self.idm.clone(),
            pmm: self.pmm.clone(),
            optional: Vec::new(),
        })
    }

    fn transceive(
        &mut self,
        _target: &RemoteTarget,
        data: &[u8],
        timeout_ms: Option<u16>,
    ) -> Result<Vec<u8>, DriverError> {
        let code = data.get(1).copied().unwrap_or(0);
        self.out_tx
            .send(Out::Frame {
                code,
                frame: data.to_vec(),
                timeout_ms: timeout_ms.unwrap_or(0),
            })
            .map_err(|_| DriverError::Other("relay output channel closed".into()))?;
        self.card_rx
            .recv()
            .map_err(|_| DriverError::Other("relay card channel closed".into()))
    }
}
