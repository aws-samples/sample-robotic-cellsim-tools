use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DresserError {
    #[error("URDF parse error in {path}: {detail}")]
    UrdfParse { path: String, detail: String },

    #[error("Validation failed with {0} error(s)")]
    Validation(usize),

    #[error("Mesh not found: {path} (referenced from {urdf})")]
    MeshNotFound { path: String, urdf: String },

    #[error("Invalid orientation: {0}")]
    OrientationInvalid(String),

    #[error("Tool orientation check failed: {0}")]
    IntersectionDetected(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub location: Option<DiagLocation>,
}

#[derive(Debug, Clone)]
pub struct DiagLocation {
    pub file: PathBuf,
    pub element: String,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            location: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            location: None,
        }
    }

    pub fn with_location(mut self, file: PathBuf, element: impl Into<String>) -> Self {
        self.location = Some(DiagLocation {
            file,
            element: element.into(),
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
