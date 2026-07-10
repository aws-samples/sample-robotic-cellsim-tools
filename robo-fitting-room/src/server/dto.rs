//! Wire types shared between handlers and the SPA.
//!
//! The struct definitions now live in `crate::transport_common::dto` so
//! they compile independently of axum for the `stdio`-only build (the
//! `stdio` dispatch table reuses the exact same shapes). They're
//! re-exported here unchanged so existing `crate::server::dto::*` call
//! sites (handlers, `server::openapi`) keep working without modification.
//! `ErrorEnvelope` likewise lives in `crate::transport_common` and is
//! re-exported here for the same reason.

pub use crate::transport_common::dto::{
    AssetEntry, FloorPlanEntry, FloorPlanSegments, HealthDto, JointInfo, PathQuery, ReadinessDto,
    SaveResponse, ScenePathBody, UploadResponse, UrdfJointsResponse, ValidateResponse,
};
pub use crate::transport_common::ErrorEnvelope;
