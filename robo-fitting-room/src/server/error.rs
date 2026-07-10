//! `ApiError` now lives in `crate::transport_common::error` so it (and
//! `AppState`) compile independently of axum for the `stdio`-only build.
//! Re-exported here unchanged so existing `server::error::ApiError` /
//! `server::ApiError` call sites, and its `IntoResponse` impl
//! (`server`-feature-gated, defined alongside the type), keep working
//! without modification.

pub use crate::transport_common::error::ApiError;
