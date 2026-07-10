//! Types shared by every request transport (`server`'s HTTP handlers and
//! `stdio`'s dispatch table): the sandboxed asset-root state (`AppState`)
//! and the uniform error shape (`ApiError` / `ErrorEnvelope`).
//!
//! Compiled whenever either the `server` or the `stdio` feature is
//! enabled, and depends only on the plain `http` crate (for
//! `StatusCode`) — never on axum/tokio/tower/utoipa — so a
//! `--no-default-features --features stdio` build never pulls the HTTP
//! stack into its dependency graph. `axum::http::StatusCode` is itself a
//! re-export of `http::StatusCode`, so this is the same type the
//! `server` handlers already use; nothing about `ApiError`'s status
//! values or constructors changes.

pub mod dto;
pub mod error;
pub mod state;

pub use error::{ApiError, ErrorEnvelope};
pub use state::AppState;
