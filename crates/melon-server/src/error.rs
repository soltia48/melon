//! The API error type and its mapping from domain/oracle errors to HTTP.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use melon_auth::ProtocolError;
use melon_db::DbError;

/// An error rendered as `{"error": {"code", "message", "details"?}}` with an HTTP
/// status. `code` is a stable machine-readable string clients localize on;
/// `details` optionally carries structured fields (e.g. amounts) for a precise UI.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Attach structured fields (surfaced under `error.details`).
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg)
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "FORBIDDEN", msg)
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "BAD_REQUEST", msg)
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", msg)
    }
    pub fn unprocessable(code: &'static str, msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(code = self.code, message = %self.message, "request failed");
        }
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(details) = self.details {
            error["details"] = details;
        }
        (self.status, Json(json!({ "error": error }))).into_response()
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::InsufficientFunds {
                available,
                requested,
            } => ApiError::unprocessable(
                "INSUFFICIENT_FUNDS",
                format!("insufficient funds: available {available}, requested {requested}"),
            )
            .with_details(
                json!({ "available": available.as_i64(), "requested": requested.as_i64() }),
            ),
            DbError::CreditLimitExceeded {
                available,
                requested,
            } => ApiError::unprocessable(
                "CREDIT_LIMIT_EXCEEDED",
                format!(
                    "credit limit exceeded: top-up up to {available} allowed, requested {requested}"
                ),
            )
            .with_details(
                json!({ "available": available.as_i64(), "requested": requested.as_i64() }),
            ),
            DbError::RefundExceedsPayment {
                requested,
                refundable,
            } => ApiError::unprocessable(
                "REFUND_EXCEEDS_PAYMENT",
                format!("refund {requested} exceeds refundable {refundable}"),
            )
            .with_details(
                json!({ "requested": requested.as_i64(), "refundable": refundable.as_i64() }),
            ),
            DbError::IdempotencyConflict => ApiError::new(
                StatusCode::CONFLICT,
                "IDEMPOTENCY_CONFLICT",
                "idempotency key reused with different parameters",
            ),
            DbError::MerchantNotFound => ApiError::not_found("merchant not found"),
            DbError::StoreNotFound => ApiError::not_found("store not found"),
            DbError::StoreCodeTaken => ApiError::new(
                StatusCode::CONFLICT,
                "STORE_CODE_TAKEN",
                "that store code is already in use for this merchant",
            ),
            DbError::UserNotFound => ApiError::not_found("user not found"),
            DbError::EmailTaken => ApiError::new(
                StatusCode::CONFLICT,
                "EMAIL_TAKEN",
                "that email is already registered",
            ),
            DbError::MerchantNotActive => ApiError::forbidden("merchant is not active"),
            DbError::PaymentNotFound => ApiError::not_found("payment not found"),
            DbError::AccountNotFound => ApiError::not_found("account not found"),
            DbError::Expiry(_) | DbError::Money(_) | DbError::Migrate(_) | DbError::Sqlx(_) => {
                ApiError::internal(e.to_string())
            }
        }
    }
}

impl From<ProtocolError> for ApiError {
    fn from(e: ProtocolError) -> Self {
        let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        ApiError::new(status, "AUTH", e.message)
    }
}
