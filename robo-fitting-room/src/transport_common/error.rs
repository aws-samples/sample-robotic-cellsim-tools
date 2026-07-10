//! Uniform error shape shared by every transport: the wire body
//! (`ErrorEnvelope`) and the Rust-side newtype handlers/dispatch arms
//! return (`ApiError`). Depends only on `http::StatusCode` (a plain,
//! axum-independent crate — `axum::http` is a re-export of it), so this
//! module compiles under `stdio`-only builds with no HTTP framework in
//! the dependency graph. Constructors and the `status`/`envelope`
//! fields are unchanged from their pre-move `server::error`/`server::dto`
//! shapes.

use http::StatusCode;
use serde::Serialize;

/// Uniform error body for every 4xx/5xx response. Wire schema documented
/// in `docs/api.md`. `code` is a stable machine-readable token (no spaces);
/// `message` is human-readable; `hint` is an optional remediation pointer.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// HTTP-status-shaped error + uniform body. Implements `IntoResponse`
/// (under the `server` feature) so axum handlers can just
/// `return Err(ApiError::not_found("urdf_missing", ...))`; the `stdio`
/// dispatch table maps the same value via `From<ApiError> for RpcError`
/// instead.
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

    pub fn service_unavailable(
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
    }
}

#[cfg(feature = "server")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, axum::response::Json(self.envelope)).into_response()
    }
}
