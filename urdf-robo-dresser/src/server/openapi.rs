//! OpenAPI spec generation. Mirrors the fitting-room shape so both
//! services publish their wire contracts the same way: handler-level
//! `#[utoipa::path(...)]` decorators + DTO-level `#[derive(ToSchema)]`,
//! aggregated by `ApiDoc` and snapshot-tested via
//! `tests/openapi_snapshot.rs` against the checked-in `openapi.json`.

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::server::dto::{
    AssetStatusDto, BatchAccepted, BatchRequest, CatalogueListing, DiagnosticDto,
    DressRequest, DressResponse, DressedEntry, ErrorEnvelope, HealthDto, JobStatus, ReadinessDto,
    RobotEntry, ToolEntry, ValidateRequest, ValidateResponse,
};
use crate::server::state::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "urdf-robo-dresser API",
        description = "Versioned REST API exposed by `urdf-robo-dresser serve`. Mirrors the \
                       CLI subcommands one-for-one (`dress`, `validate`, `batch`, `list`) so \
                       both transports call the same library functions. Routes are pinned \
                       under `/api/v1/...`; liveness/readiness probes sit at the root.",
        version = "0.1.0",
        license(name = "MIT OR Apache-2.0")
    ),
    paths(
        crate::server::handlers::catalogue::list_all,
        crate::server::handlers::catalogue::list_robots,
        crate::server::handlers::catalogue::list_tools,
        crate::server::handlers::catalogue::list_dressed,
        crate::server::handlers::dress::dress,
        crate::server::handlers::validate::validate,
        crate::server::handlers::batch::start,
        crate::server::handlers::jobs::status,
        crate::server::handlers::jobs::artifact,
        crate::server::handlers::health::healthz,
        crate::server::handlers::health::readyz,
    ),
    components(schemas(
        CatalogueListing,
        RobotEntry,
        ToolEntry,
        DressedEntry,
        AssetStatusDto,
        DressRequest,
        DressResponse,
        DiagnosticDto,
        ValidateRequest,
        ValidateResponse,
        BatchRequest,
        BatchAccepted,
        JobStatus,
        HealthDto,
        ReadinessDto,
        ErrorEnvelope,
    )),
    tags(
        (name = "catalogue", description = "Robot + tool catalogue listings."),
        (name = "dress", description = "Merge a robot URDF with a tool URDF."),
        (name = "validate", description = "URDF static validation."),
        (name = "batch", description = "Batch dress every robot × tool permutation."),
        (name = "system", description = "Liveness + readiness probes."),
    ),
)]
pub struct ApiDoc;

/// Build a router that serves the spec at `/openapi.json` and Swagger UI
/// at `/docs`. `SwaggerUi::url` registers the JSON spec route itself so
/// we don't add it again.
pub fn docs_router() -> Router<AppState> {
    SwaggerUi::new("/docs")
        .url("/openapi.json", ApiDoc::openapi())
        .into()
}
