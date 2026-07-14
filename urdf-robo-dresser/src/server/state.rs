use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::DresserError;

/// Process-wide state shared across handlers. Cloned cheaply via `Arc`.
#[derive(Debug, Clone)]
pub struct AppState {
    pub catalogue_root: Arc<PathBuf>,
    pub jobs: Arc<RwLock<JobRegistry>>,
    /// `Some` => serve the SPA from this disk dir (live dev, `--web-root`);
    /// `None` => serve the `include_dir!` bundle baked into the binary.
    pub web_root: Option<Arc<PathBuf>>,
}

impl AppState {
    pub fn new(catalogue_root: PathBuf) -> Self {
        Self {
            catalogue_root: Arc::new(catalogue_root),
            jobs: Arc::new(RwLock::new(JobRegistry::default())),
            web_root: None,
        }
    }

    /// Override the embedded SPA bundle with files read from disk. Mirrors
    /// the fitting-room's `--web-root` live-dev path.
    pub fn with_web_root(mut self, web_root: Option<PathBuf>) -> Self {
        self.web_root = web_root.map(Arc::new);
        self
    }

    /// Resolve a user-supplied catalogue-relative path. Refuses absolute
    /// paths and any segment containing `..`. Mirrors the fitting-room
    /// `AppState::resolve_asset` policy so the two services share one
    /// security model.
    pub fn resolve_asset(&self, rel: &str) -> Result<PathBuf, DresserError> {
        let candidate = Path::new(rel);
        if candidate.is_absolute() {
            return Err(DresserError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("asset path must be relative: {}", rel),
            )));
        }
        for comp in candidate.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(DresserError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("asset path may not contain '..': {}", rel),
                )));
            }
        }
        Ok(self.catalogue_root.join(candidate))
    }
}

/// Lifetime state of an in-flight `POST /api/v1/batch` job.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Queued; tokio task scheduled but not yet started.
    Queued,
    /// Running; `progress` is 0.0..1.0 (best-effort).
    Running { progress: f32 },
    /// Completed; `artifact_path` points at a temp zip on disk that
    /// `GET /api/v1/jobs/:id/artifact` streams back.
    Completed { artifact_path: PathBuf, success: usize, failed: usize },
    /// Errored; `message` carries the reason.
    Failed { message: String },
}

/// In-memory job registry. Jobs survive only as long as the process —
/// persistence to the FSx ONTAP volume is a v1.1 follow-up if batch SLAs
/// turn out to be multi-minute.
#[derive(Debug, Default)]
pub struct JobRegistry {
    jobs: HashMap<Uuid, JobState>,
}

impl JobRegistry {
    pub fn insert(&mut self, id: Uuid, state: JobState) {
        self.jobs.insert(id, state);
    }

    pub fn get(&self, id: &Uuid) -> Option<JobState> {
        self.jobs.get(id).cloned()
    }

    pub fn update(&mut self, id: &Uuid, state: JobState) {
        if let Some(slot) = self.jobs.get_mut(id) {
            *slot = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FR-056: authoritative sandbox guard for the /assets byte route. This is
    // the real boundary — HTTP clients often normalise `..` away before it
    // reaches the wire, so the guard is tested at the function level.
    #[test]
    fn resolve_asset_rejects_traversal_and_absolute() {
        let state = AppState::new(PathBuf::from("/catalogue"));
        assert!(state.resolve_asset("../etc/passwd").is_err());
        assert!(state.resolve_asset("a/../../b").is_err());
        assert!(state.resolve_asset("/etc/passwd").is_err());
        // A clean relative path resolves under the catalogue root.
        let ok = state.resolve_asset("urdf-dressed-robots/x/x.urdf").unwrap();
        assert!(ok.starts_with("/catalogue"));
    }
}
