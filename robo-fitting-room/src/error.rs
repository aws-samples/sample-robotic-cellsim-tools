#[derive(Debug, thiserror::Error)]
pub enum FittingRoomError {
    #[error("Scene parse error: {0}")]
    SceneParse(String),
    #[error("URDF not found: {0}")]
    UrdfNotFound(String),
    #[error("URDF invalid: {path}: {detail}")]
    UrdfInvalid { path: String, detail: String },
    #[error("URDF parse error: {0}")]
    UrdfParse(String),
    #[error("Floor plan not found: {0}")]
    FloorPlanNotFound(String),
    #[error("Validation failed: {0}")]
    Validation(String),
    #[error("USD write error: {0}")]
    UsdWrite(String),
    #[error("Unsupported output format: {0} (only .usda is supported)")]
    UnsupportedFormat(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
