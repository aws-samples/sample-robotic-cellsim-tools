use axum::extract::State;
use axum::response::Json;

use crate::catalogue;
#[allow(unused_imports)]
use crate::server::dto::ErrorEnvelope;
use crate::server::dto::{AssetStatusDto, CatalogueListing, DressedEntry, RobotEntry, ToolEntry};
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// GET /api/v1/catalogue — full catalogue listing under
/// `<catalogue_root>/{urdf-robot-collection,urdf-tool-collection}`.
#[utoipa::path(
    get,
    path = "/api/v1/catalogue",
    tag = "catalogue",
    responses(
        (status = 200, description = "Catalogue listing", body = CatalogueListing),
        (status = 500, description = "Catalogue scan failed", body = ErrorEnvelope),
    ),
)]
pub async fn list_all(State(state): State<AppState>) -> Result<Json<CatalogueListing>, ApiError> {
    let cat = catalogue::scan(state.catalogue_root.as_path())
        .map_err(|e| ApiError::internal("catalogue_scan_failed", e.to_string()))?;
    Ok(Json(CatalogueListing {
        robots: cat.robots.iter().map(robot_entry_to_dto).collect(),
        tools: cat.tools.iter().map(tool_entry_to_dto).collect(),
    }))
}

/// GET /api/v1/catalogue/robots — robot listing only.
#[utoipa::path(
    get,
    path = "/api/v1/catalogue/robots",
    tag = "catalogue",
    responses(
        (status = 200, description = "Robot listing", body = [RobotEntry]),
        (status = 500, description = "Catalogue scan failed", body = ErrorEnvelope),
    ),
)]
pub async fn list_robots(State(state): State<AppState>) -> Result<Json<Vec<RobotEntry>>, ApiError> {
    let cat = catalogue::scan(state.catalogue_root.as_path())
        .map_err(|e| ApiError::internal("catalogue_scan_failed", e.to_string()))?;
    Ok(Json(cat.robots.iter().map(robot_entry_to_dto).collect()))
}

/// GET /api/v1/catalogue/tools — tool listing only.
#[utoipa::path(
    get,
    path = "/api/v1/catalogue/tools",
    tag = "catalogue",
    responses(
        (status = 200, description = "Tool listing", body = [ToolEntry]),
        (status = 500, description = "Catalogue scan failed", body = ErrorEnvelope),
    ),
)]
pub async fn list_tools(State(state): State<AppState>) -> Result<Json<Vec<ToolEntry>>, ApiError> {
    let cat = catalogue::scan(state.catalogue_root.as_path())
        .map_err(|e| ApiError::internal("catalogue_scan_failed", e.to_string()))?;
    Ok(Json(cat.tools.iter().map(tool_entry_to_dto).collect()))
}

/// GET /api/v1/catalogue/dressed — already-dressed robots under
/// `<catalogue>/urdf-dressed-robots/<combo>/<combo>.urdf`.
// FR-057
#[utoipa::path(
    get,
    path = "/api/v1/catalogue/dressed",
    tag = "catalogue",
    responses(
        (status = 200, description = "Dressed-robot listing", body = [DressedEntry]),
        (status = 500, description = "Dressed-robots scan failed", body = ErrorEnvelope),
    ),
)]
pub async fn list_dressed(
    State(state): State<AppState>,
) -> Result<Json<Vec<DressedEntry>>, ApiError> {
    let dir = state.catalogue_root.join("urdf-dressed-robots");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // Convention: <combo>/<combo>.urdf (mirrors dress output layout).
            if path.join(format!("{}.urdf", name)).exists() {
                out.push(DressedEntry {
                    urdf_rel: format!("urdf-dressed-robots/{}/{}.urdf", name, name),
                    name,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(out))
}

fn robot_entry_to_dto(e: &catalogue::RobotEntry) -> RobotEntry {
    RobotEntry {
        name: e.name.clone(),
        flange_link: e.flange_link.clone(),
        joint_count: e.joint_count,
        link_count: e.link_count,
        status: status_to_dto(&e.status),
    }
}

fn tool_entry_to_dto(e: &catalogue::ToolEntry) -> ToolEntry {
    ToolEntry {
        name: e.name.clone(),
        visual_count: e.visual_count,
        status: status_to_dto(&e.status),
    }
}

fn status_to_dto(s: &catalogue::AssetStatus) -> AssetStatusDto {
    match s {
        catalogue::AssetStatus::Valid => AssetStatusDto::Valid,
        catalogue::AssetStatus::Incomplete { issues } => AssetStatusDto::Incomplete {
            issues: issues.clone(),
        },
        catalogue::AssetStatus::Error(message) => AssetStatusDto::Error {
            message: message.clone(),
        },
    }
}
