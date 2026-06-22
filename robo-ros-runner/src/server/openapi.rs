//! OpenAPI spec + Swagger UI. The checked-in `openapi.json` at the crate root
//! is the wire contract, enforced by `tests/openapi_snapshot.rs`.

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::server::dto::{
    CsvInspectRequest, CsvInspectResponse, ErrorEnvelope, HealthDto, PlaybackRequest,
    PlaybackStartResponse, PlaybackStatusResponse, ReadinessDto, TopicsRequest, TopicsResponse,
};
use crate::server::state::AppState;
use crate::topics::ResolvedTopic;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "robo-ros-runner API",
        description = "Versioned REST API exposed by `robo-ros-runner serve`. Resolve topics, \
                       inspect captures, and start/stop playback jobs over the same pipeline the \
                       CLI drives. Liveness / readiness probes sit at the root (`/healthz`, \
                       `/readyz`).",
        version = "0.1.0",
        license(name = "MIT OR Apache-2.0")
    ),
    paths(
        crate::server::handlers::topics::resolve,
        crate::server::handlers::csv::inspect,
        crate::server::handlers::playback::start,
        crate::server::handlers::playback::stop,
        crate::server::handlers::playback::status,
        crate::server::handlers::health::healthz,
        crate::server::handlers::health::readyz,
    ),
    components(schemas(
        TopicsRequest,
        TopicsResponse,
        CsvInspectRequest,
        CsvInspectResponse,
        PlaybackRequest,
        PlaybackStartResponse,
        PlaybackStatusResponse,
        ResolvedTopic,
        ErrorEnvelope,
        HealthDto,
        ReadinessDto,
    )),
    tags(
        (name = "topics", description = "Resolve per-robot joint_command topics from a cell manifest."),
        (name = "csv", description = "Inspect capture CSV schemas."),
        (name = "playback", description = "Start / stop / query playback jobs."),
        (name = "system", description = "Liveness + readiness probes."),
    ),
)]
pub struct ApiDoc;

/// Serve the spec at `/openapi.json` and Swagger UI at `/docs`.
pub fn docs_router() -> Router<AppState> {
    SwaggerUi::new("/docs")
        .url("/openapi.json", ApiDoc::openapi())
        .into()
}
