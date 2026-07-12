//! Error type shared across the crate.
//!
//! [`ProtocolError`] carries an HTTP status, a human message and an optional
//! numeric `code` (used to surface a FeliCa status flag). It deliberately avoids
//! depending on `axum` so it can travel across the session worker-thread boundary;
//! the `IntoResponse` conversion lives in [`crate::http`].

use felica_rs::felica_standard::FelicaStandardError;

/// An error that maps directly onto an HTTP JSON error response.
#[derive(Debug, Clone)]
pub struct ProtocolError {
    /// HTTP status code to return.
    pub status: u16,
    /// Human readable message.
    pub message: String,
    /// Optional protocol-level code (e.g. a FeliCa status flag `SF1<<8 | SF2`).
    pub code: Option<i64>,
}

impl ProtocolError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            code: None,
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }

    pub fn with_code(mut self, code: i64) -> Self {
        self.code = Some(code);
        self
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// Map an `felica-rs` protocol error onto a [`ProtocolError`].
///
/// Note: `felica-rs` uses its own error taxonomy, so the numeric `code` does not
/// match nfcpy's internal errno values used by the previous Python server. A
/// FeliCa status-flag error is surfaced as `code = SF1 << 8 | SF2`.
pub fn map_felica_error(err: &FelicaStandardError) -> ProtocolError {
    match err {
        FelicaStandardError::Status {
            status_flag1,
            status_flag2,
            ..
        } => ProtocolError::bad_request(err.to_string())
            .with_code(((*status_flag1 as i64) << 8) | (*status_flag2 as i64)),
        FelicaStandardError::AuthenticationRequired => ProtocolError::bad_request(err.to_string()),
        FelicaStandardError::AuthenticationFailed(_) => ProtocolError::bad_request(err.to_string()),
        FelicaStandardError::SecureSession(_) => ProtocolError::bad_request(err.to_string()),
        FelicaStandardError::Protocol(_) => ProtocolError::bad_request(err.to_string()),
        FelicaStandardError::InvalidParameter(_) => ProtocolError::bad_request(err.to_string()),
        FelicaStandardError::UnsupportedTarget(_) => ProtocolError::bad_request(err.to_string()),
        // A driver error in the relay model means the rendezvous channel closed
        // (e.g. the client abandoned the session): treat as a server-side fault.
        FelicaStandardError::Driver(_) => ProtocolError::internal(err.to_string()),
    }
}
