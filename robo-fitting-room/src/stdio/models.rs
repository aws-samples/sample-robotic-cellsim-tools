//! Result shapes unique to the `stdio` transport.
//!
//! Every other request/response type is reused verbatim from
//! `crate::transport_common::dto`. These two exist only because the stdio
//! transport re-shapes the two large-artifact endpoints (`compose.run` /
//! `compose.urdfBundle`) to return a temp-file path instead of inlining
//! zip bytes, and folds `scene.load`'s raw-YAML body into a JSON object.
//!
//! Wire shape is camelCase to match the rest of the RPC surface consumed
//! by the TypeScript `StdioClient`.

use serde::Serialize;

/// Success result for `compose.run` and `compose.urdfBundle`. Instead of
/// embedding the (potentially 100+ MB) zip bytes in the JSON response, the
/// dispatch handler writes the archive to a uniquely-named file under the
/// OS temp directory and returns its path here; the extension host owns
/// moving/deleting it after this response is sent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeArtifact {
    /// Absolute path to a uniquely-named temp file.
    pub zip_path: String,
    /// Suggested filename for the save dialog (e.g. "scene.zip",
    /// "scene-urdf.zip").
    pub download_name: String,
    pub bytes: u64,
}

/// Success result for `scene.load`. The HTTP transport returns the raw
/// YAML as an `application/x-yaml` body; over stdio it's folded into this
/// one-field JSON object so every response shares the same envelope.
#[derive(Debug, Serialize)]
pub struct SceneLoadResult {
    pub yaml: String,
}
