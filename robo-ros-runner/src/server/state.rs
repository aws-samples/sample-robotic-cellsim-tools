use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::R3Error;
use crate::playback::{CancelFlag, PlaybackReport};

/// One tracked playback job. `report` is filled in when the job finishes; while
/// running it holds the cancel handle so `DELETE` can stop it.
#[derive(Debug, Clone)]
pub struct JobEntry {
    pub status: JobStatus,
    pub cancel: CancelFlag,
    pub report: Option<PlaybackReport>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Cancelled,
    Failed,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed => "failed",
        }
    }
}

/// Process-wide state shared across handlers. `Arc` for cheap per-request
/// cloning; the job registry is behind a `Mutex` (mutated rarely).
#[derive(Debug, Clone)]
pub struct AppState {
    pub asset_root: Arc<PathBuf>,
    pub jobs: Arc<Mutex<BTreeMap<String, JobEntry>>>,
}

impl AppState {
    pub fn new(asset_root: PathBuf) -> Self {
        Self {
            asset_root: Arc::new(asset_root),
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Resolve a user-supplied relative path against `asset_root`, refusing
    /// anything that escapes the sandbox via `..` or absolute paths. Mirrors
    /// the sibling crates' `resolve_asset`.
    pub fn resolve_asset(&self, rel: &str) -> Result<PathBuf, R3Error> {
        let candidate = Path::new(rel);
        if candidate.is_absolute() {
            return Err(R3Error::Config(format!(
                "asset path must be relative: {}",
                rel
            )));
        }
        for comp in candidate.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(R3Error::Config(format!(
                    "asset path may not contain '..': {}",
                    rel
                )));
            }
        }
        let joined = self.asset_root.join(candidate);
        let canon_root = std::fs::canonicalize(self.asset_root.as_path())
            .unwrap_or_else(|_| self.asset_root.as_path().to_path_buf());
        let canon_joined = canonicalize_existing_prefix(&joined);
        if !canon_joined.starts_with(&canon_root) {
            return Err(R3Error::Config(format!(
                "asset path escapes asset_root: {}",
                rel
            )));
        }
        Ok(canon_joined)
    }
}

/// Canonicalise the deepest existing prefix of `p` and re-attach the trailing
/// components (the target file may not exist yet).
fn canonicalize_existing_prefix(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let name = match existing.file_name() {
            Some(n) => n.to_os_string(),
            None => break,
        };
        tail.push(name);
        if !existing.pop() {
            break;
        }
    }
    let canon = std::fs::canonicalize(&existing).unwrap_or(existing);
    let mut out = canon;
    for n in tail.into_iter().rev() {
        out.push(n);
    }
    out
}
