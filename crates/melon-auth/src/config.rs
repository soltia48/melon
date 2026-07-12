//! Server configuration structures.

use std::collections::HashSet;
use std::time::Duration;

/// Top-level server configuration assembled from CLI arguments / environment.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub keys_path: String,
    /// If set, restrict the encrypted-exchange command codes to this set.
    pub allowed_cmd_codes: Option<HashSet<u8>>,
    /// Idle lifetime after which an inactive session is reaped.
    pub session_ttl: Duration,
    /// Maximum number of concurrent live sessions.
    pub max_sessions: usize,
}

impl ServerConfig {
    pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(300);
    pub const DEFAULT_MAX_SESSIONS: usize = 1024;
}
