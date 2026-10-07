//! API error type — maps domain errors to HTTP responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "internal error");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self(StatusCode::UNAUTHORIZED, msg.into())
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, msg.into())
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, format!("{} not found", what.into()))
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<runway_core::Error> for ApiError {
    fn from(e: runway_core::Error) -> Self {
        use runway_core::Error::*;
        match e {
            NotFound(m) => ApiError::not_found(m),
            Validation(m) | BadRequest(m) => ApiError::bad_request(m),
            Unauthorized(m) => ApiError::unauthorized(m),
            Forbidden(m) => ApiError::forbidden(m),
            Conflict(m) => ApiError::conflict(m),
            other => ApiError::internal(other),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::internal(e)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db_err) = &e {
            if db_err.is_unique_violation() {
                return ApiError::conflict("resource already exists");
            }
        }
        ApiError::internal(e)
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;
