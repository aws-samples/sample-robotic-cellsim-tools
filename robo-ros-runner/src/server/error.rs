//! HTTP error newtype wrapping the typed `ErrorEnvelope` so every handler can
//! return `Result<T, ApiError>` and have the response match the OpenAPI schema.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

use crate::server::dto::ErrorEnvelope;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub envelope: ErrorEnvelope,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            envelope: ErrorEnvelope {
                code: code.into(),
                message: message.into(),
                hint: None,
            },
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.envelope.hint = Some(hint.into());
        self
    }

    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn unprocessable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    pub fn internal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, code, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.envelope)).into_response()
    }
}
