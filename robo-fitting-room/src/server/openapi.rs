//! OpenAPI spec generation. Gated behind the `server` feature alongside the
//! rest of the HTTP surface. Exposes:
//!   GET /openapi.json   — the generated spec
//!   GET /docs           — Swagger UI (loads the spec above)
//!
//! The spec is the project's contract — every PR should leave the spec
//! either unchanged or with a deliberate diff. The
//! `tests/openapi_snapshot.rs` test enforces this against the checked-in
//! `openapi.json` at the crate root.

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::server::dto::{
    AssetEntry, ErrorEnvelope, FloorPlanEntry, FloorPlanSegments, HealthDto, JointInfo,
    PathQuery, ReadinessDto, SaveResponse, ScenePathBody, UploadResponse, UrdfJointsResponse,
    ValidateResponse,
};
use crate::server::handlers::compose::ComposeQuery;
use crate::server::state::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "robo-fitting-room API",
        description = "Versioned REST API exposed by `robo-fitting-room serve`. Handlers under \
                       `/api/v1/*` are stable; the unversioned `/api/*` aliases are deprecated \
                       and will be removed in the next release. Liveness / readiness probes \
                       sit at the root (`/healthz`, `/readyz`).",
        version = "0.2.0",
        license(name = "MIT OR Apache-2.0")
    ),
    paths(
        crate::server::handlers::assets::list_robots,
        crate::server::handlers::assets::list_fixtures,
        crate::server::handlers::assets::list_floor_plans,
        crate::server::handlers::assets::upload_obj,
        crate::server::handlers::urdf::joints,
        crate::server::handlers::floor_plan::segments,
        crate::server::handlers::compose::validate,
        crate::server::handlers::compose::compose_zip,
        crate::server::handlers::scene::load,
        crate::server::handlers::scene::save,
        crate::server::handlers::health::healthz,
        crate::server::handlers::health::readyz,
    ),
    components(schemas(
        AssetEntry,
        FloorPlanEntry,
        JointInfo,
        UrdfJointsResponse,
        FloorPlanSegments,
        ValidateResponse,
        PathQuery,
        ScenePathBody,
        UploadResponse,
        SaveResponse,
        ComposeQuery,
        ErrorEnvelope,
        HealthDto,
        ReadinessDto,
    )),
    tags(
        (name = "assets", description = "Catalogue listings + uploads under `<asset_root>`."),
        (name = "urdf", description = "URDF parsing helpers (joint listing)."),
        (name = "floor-plan", description = "Floor-plan OBJ → segment arrays."),
        (name = "scene", description = "Manifest YAML round-trip (load + save)."),
        (name = "compose", description = "Validate manifests; compose `.zip` packages."),
        (name = "system", description = "Liveness + readiness probes."),
    ),
)]
pub struct ApiDoc;

/// Build a router that serves the spec at `/openapi.json` and Swagger UI at `/docs`.
/// `SwaggerUi::url` registers the JSON spec route itself, so we don't add it again.
pub fn docs_router() -> Router<AppState> {
    SwaggerUi::new("/docs")
        .url("/openapi.json", ApiDoc::openapi())
        .into()
}
