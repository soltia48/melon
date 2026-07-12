//! HTTP layer: axum router, JWT gate, lenient request parsing and the JSON
//! error representation.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Map, Value, json};

use crate::error::ProtocolError;
use crate::session::{EncryptionExchangeInput, MutualAuthInput, SessionManager};

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<SessionManager>,
}

/// Build the application router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/mutual-authentication", post(mutual_authentication))
        .route("/encryption-exchange", post(encryption_exchange))
        .with_state(state)
}

impl IntoResponse for ProtocolError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut error = json!({ "message": self.message });
        if let Some(code) = self.code {
            error["code"] = json!(code);
        }
        (status, Json(json!({ "error": error }))).into_response()
    }
}

async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

async fn mutual_authentication(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, ProtocolError> {
    let input = parse_mutual_input(&body)?;
    let value = state.manager.handle_mutual_authentication(input).await?;
    Ok(Json(value))
}

async fn encryption_exchange(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, ProtocolError> {
    let input = parse_encryption_input(&body)?;
    let value = state.manager.handle_encryption_exchange(input).await?;
    Ok(Json(value))
}

// --- request parsing (lenient, mirroring the reference server) ---

pub fn parse_mutual_input(body: &Bytes) -> Result<MutualAuthInput, ProtocolError> {
    let obj = parse_body(body)?;
    Ok(MutualAuthInput {
        session_id: get_str(&obj, "session_id"),
        idm: field(&obj, "idm")
            .map(|v| hex_to_array8(v, "idm"))
            .transpose()?,
        pmm: field(&obj, "pmm")
            .map(|v| hex_to_array8(v, "pmm"))
            .transpose()?,
        system_code: field(&obj, "system_code")
            .map(|v| parse_u16_value(v, "system_code"))
            .transpose()?,
        areas: field(&obj, "areas")
            .map(|v| parse_u16_list(v, "areas"))
            .transpose()?,
        services: field(&obj, "services")
            .map(|v| parse_u16_list(v, "services"))
            .transpose()?,
        card_response: field(&obj, "card_response")
            .map(|v| hex_to_bytes(v, "card_response", None))
            .transpose()?,
    })
}

pub fn parse_encryption_input(body: &Bytes) -> Result<EncryptionExchangeInput, ProtocolError> {
    let obj = parse_body(body)?;
    Ok(EncryptionExchangeInput {
        session_id: get_str(&obj, "session_id"),
        cmd_code: field(&obj, "cmd_code")
            .map(|v| parse_u8(v, "cmd_code"))
            .transpose()?,
        payload: field(&obj, "payload")
            .map(|v| hex_to_bytes(v, "payload", None))
            .transpose()?,
        timeout: field(&obj, "timeout")
            .map(|v| parse_f64(v, "timeout"))
            .transpose()?,
        card_response: field(&obj, "card_response")
            .map(|v| hex_to_bytes(v, "card_response", None))
            .transpose()?,
    })
}

fn parse_body(body: &Bytes) -> Result<Value, ProtocolError> {
    if body.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| ProtocolError::bad_request("invalid JSON payload"))?;
    if !value.is_object() {
        return Err(ProtocolError::bad_request("invalid JSON payload"));
    }
    Ok(value)
}

/// A present, non-null field.
fn field<'a>(obj: &'a Value, name: &str) -> Option<&'a Value> {
    match obj.get(name) {
        Some(value) if !value.is_null() => Some(value),
        _ => None,
    }
}

fn get_str(obj: &Value, name: &str) -> Option<String> {
    field(obj, name).and_then(Value::as_str).map(str::to_string)
}

fn parse_u64_from_value(value: &Value, name: &str) -> Result<u64, ProtocolError> {
    if let Some(unsigned) = value.as_u64() {
        return Ok(unsigned);
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        let (radix, digits) = match trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
        {
            Some(hex_digits) => (16, hex_digits),
            None => (10, trimmed),
        };
        return u64::from_str_radix(digits, radix).map_err(|_| {
            ProtocolError::bad_request(format!("{name} contains invalid integer '{text}'"))
        });
    }
    Err(ProtocolError::bad_request(format!(
        "{name} must be an integer or hex string"
    )))
}

fn parse_u16_value(value: &Value, name: &str) -> Result<u16, ProtocolError> {
    let number = parse_u64_from_value(value, name)?;
    if number > 0xFFFF {
        return Err(ProtocolError::bad_request(format!(
            "{name} value {number} outside 0..65535"
        )));
    }
    Ok(number as u16)
}

fn parse_u16_list(value: &Value, name: &str) -> Result<Vec<u16>, ProtocolError> {
    let array = value
        .as_array()
        .ok_or_else(|| ProtocolError::bad_request(format!("{name} must be a non-empty list")))?;
    array
        .iter()
        .map(|item| parse_u16_value(item, name))
        .collect()
}

fn parse_u8(value: &Value, name: &str) -> Result<u8, ProtocolError> {
    let number = parse_u64_from_value(value, name)?;
    if number > 0xFF {
        return Err(ProtocolError::bad_request(format!(
            "{name} must fit into one byte"
        )));
    }
    Ok(number as u8)
}

fn parse_f64(value: &Value, name: &str) -> Result<f64, ProtocolError> {
    value
        .as_f64()
        .ok_or_else(|| ProtocolError::bad_request(format!("{name} must be a number")))
}

fn hex_to_bytes(
    value: &Value,
    name: &str,
    expected_len: Option<usize>,
) -> Result<Vec<u8>, ProtocolError> {
    let text = value.as_str().ok_or_else(|| {
        ProtocolError::bad_request(format!("{name} must be provided as a hex string"))
    })?;
    let cleaned = text.trim();
    if cleaned.len() % 2 != 0 {
        return Err(ProtocolError::bad_request(format!(
            "{name} hex string must have even length"
        )));
    }
    let bytes = hex::decode(cleaned)
        .map_err(|_| ProtocolError::bad_request(format!("{name} is not valid hex data")))?;
    if let Some(len) = expected_len
        && bytes.len() != len
    {
        return Err(ProtocolError::bad_request(format!(
            "{name} must be {len} bytes"
        )));
    }
    Ok(bytes)
}

fn hex_to_array8(value: &Value, name: &str) -> Result<[u8; 8], ProtocolError> {
    let bytes = hex_to_bytes(value, name, Some(8))?;
    Ok(bytes.try_into().expect("length checked to be 8"))
}
