//! felica-auth-server: a remote FeliCa crypto oracle.
//!
//! The server holds the secret keys ([`keystore`]) and drives the FeliCa Standard
//! mutual-authentication and secure-messaging protocol, while a separate *client*
//! owns the physical reader. For every protocol step the server emits the exact
//! command frame the client must relay to the card, and consumes the card's
//! response on the following request.
//!
//! The heavy lifting (challenge math, MACs, secure framing) is reused verbatim
//! from the `felica-rs` library: a per-session worker thread drives
//! [`felica_rs::felica_standard::FelicaStandard`] through a custom relay
//! [`felica_rs::felica_standard::FelicaDriver`] whose `transceive` bounces each frame
//! to the HTTP client and blocks for the reply. See [`relay_driver`] and
//! [`session`].

pub mod config;
pub mod error;
pub mod http;
pub mod keystore;
pub mod relay_driver;
pub mod session;

#[cfg(test)]
mod integration_tests;

pub use config::ServerConfig;
pub use error::ProtocolError;
pub use keystore::KeyStore;
pub use session::SessionManager;
