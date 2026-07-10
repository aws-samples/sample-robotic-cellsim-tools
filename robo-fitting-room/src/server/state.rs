//! `AppState` now lives in `crate::transport_common::state` so it compiles
//! independently of axum for the `stdio`-only build. Re-exported here
//! unchanged so existing `server::state::AppState` / `server::AppState`
//! call sites keep working without modification.

pub use crate::transport_common::state::AppState;
