//! JSON-RPC-flavored request/response envelope types for the `stdio`
//! transport, plus the three protocol-level error constructors.
//!
//! Framing is newline-delimited JSON (NDJSON): exactly one JSON value per
//! line. `serde_json` escapes embedded newlines inside string values as
//! `\n`, so a serialized value never contains a literal `0x0A` byte
//! outside the frame boundary — "one JSON value per line" is a correct
//! invariant without a length-prefix header.
//!
//! Error codes come from two disjoint ranges:
//! - Protocol errors use the reserved JSON-RPC negative codes
//!   (-32700 parse, -32601 method not found, -32602 invalid params).
//! - Application errors (mapped from `ApiError` in a later step) reuse the
//!   HTTP-status-like numbers (400/404/422/500/503).

use crate::transport_common::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single request read from one stdin line.
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A single response written to one stdout line. `id` is `None` only for
/// parse errors that occur before an id can be extracted (matching
/// JSON-RPC's convention). The `outcome` is flattened so the wire shape is
/// `{"id": N, "result": ...}` or `{"id": N, "error": ...}`.
#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub id: Option<u64>,
    #[serde(flatten)]
    pub outcome: RpcOutcome,
}

/// The result-or-error half of a response, rendered as either a `result`
/// or an `error` field.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RpcOutcome {
    #[serde(rename = "result")]
    Ok(Value),
    #[serde(rename = "error")]
    Err(RpcError),
}

/// Error object carried in an `error` response.
#[derive(Debug, Serialize)]
pub struct RpcError {
    /// Protocol errors (-32700 parse, -32601 method not found, -32602
    /// invalid params) use the reserved JSON-RPC negative range.
    /// Application errors reuse the ApiError's HTTP-status-like numbers
    /// (400/404/422/500/503) so the mapping from `ApiError` is 1:1 and
    /// the extension can branch on familiar codes.
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<RpcErrorData>,
}

/// Machine-readable error detail, mirroring `ErrorEnvelope`'s `code`/`hint`.
#[derive(Debug, Serialize)]
pub struct RpcErrorData {
    /// Stable machine-readable token — the same string ApiError's
    /// constructors already produce (e.g. "invalid_path", "manifest_invalid").
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl RpcError {
    /// Reserved JSON-RPC code for a request line that is not valid JSON.
    pub const CODE_PARSE_ERROR: i32 = -32700;
    /// Reserved JSON-RPC code for an unknown method name.
    pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
    /// Reserved JSON-RPC code for params that don't deserialize into the
    /// shape the named method expects.
    pub const CODE_INVALID_PARAMS: i32 = -32602;

    /// The request line could not be parsed as JSON. The originating id is
    /// unknown, so the resulting response carries `id: None`.
    pub fn parse_error() -> Self {
        Self {
            code: Self::CODE_PARSE_ERROR,
            message: "Parse error: request line is not valid JSON".to_string(),
            data: Some(RpcErrorData {
                code: "parse_error".to_string(),
                hint: None,
            }),
        }
    }

    /// The request named a method with no matching dispatch arm.
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: Self::CODE_METHOD_NOT_FOUND,
            message: format!("Method not found: {method}"),
            data: Some(RpcErrorData {
                code: "method_not_found".to_string(),
                hint: None,
            }),
        }
    }

    /// The request's `params` could not be deserialized into the shape the
    /// named method expects. `detail` is the underlying deserialization
    /// error message.
    pub fn invalid_params(method: &str, detail: impl std::fmt::Display) -> Self {
        Self {
            code: Self::CODE_INVALID_PARAMS,
            message: format!("Invalid params for method '{method}': {detail}"),
            data: Some(RpcErrorData {
                code: "invalid_params".to_string(),
                hint: None,
            }),
        }
    }
}

/// Projects an application-level [`ApiError`] onto the RPC wire error
/// shape. This is the stdio analog of the HTTP transport's
/// `IntoResponse`: the same status/body an `ApiError` would render over
/// `serve` is carried here as `code` + `data` instead.
///
/// Mapping (lossless, total):
/// - `status` → `code` via `status.as_u16() as i32` (400/404/422/500/503),
///   so the extension can branch on the same HTTP-status-like numbers the
///   `serve` transport uses.
/// - `envelope.message` → `message` verbatim.
/// - `envelope.code` and `envelope.hint` → `data.{code,hint}` verbatim.
impl From<ApiError> for RpcError {
    fn from(err: ApiError) -> Self {
        let ApiError { status, envelope } = err;
        Self {
            code: i32::from(status.as_u16()),
            message: envelope.message,
            data: Some(RpcErrorData {
                code: envelope.code,
                hint: envelope.hint,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport_common::ApiError;

    /// Validates Property 8: Error mapping is total and lossless — Requirement 3.7.
    ///
    /// For a representative `ApiError` from every constructor (covering
    /// statuses 400/404/422/500/503), converting via `RpcError::from` must:
    /// - map `status.as_u16()` to `code` (an `i32`),
    /// - copy `envelope.message` into `message` verbatim,
    /// - copy `envelope.code` into `data.code` verbatim,
    /// - copy `envelope.hint` into `data.hint` verbatim.
    ///
    /// Each case is checked both without a hint and with a hint, so the
    /// optional `hint` field is exercised in both `None` and `Some` states.
    #[test]
    fn error_mapping_is_lossless_across_all_constructors() {
        // (constructor result, expected HTTP status code)
        let cases: Vec<(ApiError, u16)> = vec![
            (ApiError::bad_request("invalid_manifest_path", "path escapes asset root"), 400),
            (ApiError::not_found("urdf_missing", "no such robot"), 404),
            (ApiError::unprocessable("manifest_invalid", "3 validation errors"), 422),
            (ApiError::internal("compose_failed", "zip writer error"), 500),
            (ApiError::service_unavailable("not_ready", "asset root not mounted"), 503),
        ];

        for (api_error, expected_status) in cases {
            // Capture the envelope contents verbatim before the conversion
            // consumes the ApiError.
            let expected_code = api_error.envelope.code.clone();
            let expected_message = api_error.envelope.message.clone();
            let expected_hint = api_error.envelope.hint.clone();
            assert_eq!(
                api_error.status.as_u16(),
                expected_status,
                "constructor should yield status {expected_status}"
            );

            let rpc_err = RpcError::from(api_error);

            assert_eq!(
                rpc_err.code,
                expected_status as i32,
                "code must equal the HTTP status as i32 for {expected_code}"
            );
            assert_eq!(
                rpc_err.message, expected_message,
                "message must be copied verbatim for {expected_code}"
            );
            let data = rpc_err
                .data
                .as_ref()
                .expect("application errors always carry data");
            assert_eq!(
                data.code, expected_code,
                "data.code must equal envelope.code verbatim"
            );
            assert_eq!(
                data.hint, expected_hint,
                "data.hint must equal envelope.hint verbatim ({expected_code})"
            );
        }
    }

    /// Validates Property 8 for the hint-carrying variant: `with_hint`
    /// output must survive the mapping into `data.hint` verbatim across the
    /// same set of statuses.
    #[test]
    fn error_mapping_preserves_hint_across_all_constructors() {
        let cases: Vec<(ApiError, u16)> = vec![
            (
                ApiError::bad_request("invalid_manifest_path", "path escapes asset root")
                    .with_hint("use a path relative to --asset-root"),
                400,
            ),
            (
                ApiError::not_found("urdf_missing", "no such robot")
                    .with_hint("call assets.listRobots to see available ids"),
                404,
            ),
            (
                ApiError::unprocessable("manifest_invalid", "3 validation errors")
                    .with_hint("call compose.validate to inspect each error"),
                422,
            ),
            (
                ApiError::internal("compose_failed", "zip writer error")
                    .with_hint("retry; if it persists, file a bug"),
                500,
            ),
            (
                ApiError::service_unavailable("not_ready", "asset root not mounted")
                    .with_hint("wait for the asset root to become available"),
                503,
            ),
        ];

        for (api_error, expected_status) in cases {
            let expected_code = api_error.envelope.code.clone();
            let expected_message = api_error.envelope.message.clone();
            let expected_hint = api_error.envelope.hint.clone();
            assert!(
                expected_hint.is_some(),
                "with_hint should populate the hint for {expected_code}"
            );

            let rpc_err = RpcError::from(api_error);

            assert_eq!(rpc_err.code, expected_status as i32);
            assert_eq!(rpc_err.message, expected_message);
            let data = rpc_err.data.expect("application errors always carry data");
            assert_eq!(data.code, expected_code);
            assert_eq!(data.hint, expected_hint);
        }
    }
}
