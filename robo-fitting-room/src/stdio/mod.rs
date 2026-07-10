//! The `stdio` transport: a long-running newline-delimited-JSON
//! request/response loop over stdin/stdout, for embedding (e.g. the VS
//! Code extension) without opening a network port.
//!
//! This module owns the wire protocol types (`protocol`), the method
//! dispatch table (`dispatch`), and the stdio-specific result shapes
//! (`models`); a later step adds the stdin/stdout run loop. It shares the
//! sandboxed `AppState` and uniform `ApiError` shape with the `server`
//! transport via `crate::transport_common`.

pub mod dispatch;
pub mod models;
pub mod protocol;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::error::FittingRoomError;
use crate::transport_common::AppState;
use protocol::{RpcOutcome, RpcRequest, RpcResponse};

/// Parse one stdin line, dispatch it, and build the response.
///
/// A line that is not valid JSON yields a parse-error response with
/// `id: None` (the originating id is unknown before parsing). Otherwise the
/// request is dispatched and its `Ok(value)`/`Err(rpc_err)` is wrapped into
/// an `RpcResponse` echoing the request's `id`.
///
/// Never panics: every step in the parse → dispatch → wrap chain is matched
/// explicitly, so a response is produced for every input line.
fn handle_line(state: &AppState, line: &str) -> RpcResponse {
    match serde_json::from_str::<RpcRequest>(line) {
        Err(_) => RpcResponse {
            id: None,
            outcome: RpcOutcome::Err(protocol::RpcError::parse_error()),
        },
        Ok(req) => match dispatch::dispatch(state, &req.method, req.params) {
            Ok(value) => RpcResponse {
                id: Some(req.id),
                outcome: RpcOutcome::Ok(value),
            },
            Err(rpc_err) => RpcResponse {
                id: Some(req.id),
                outcome: RpcOutcome::Err(rpc_err),
            },
        },
    }
}

/// Entry point wired from `main.rs`'s `Command::StdioServe` arm. Blocks
/// until stdin closes (EOF) or an unrecoverable I/O error occurs.
///
/// `asset_root` is canonicalized with the same failure behavior as
/// `server::run` (an error is returned before the loop starts if it does
/// not exist / cannot be canonicalized). `web_root` is accepted for
/// signature parity with `server::run` but is unused: the stdio transport
/// never serves static SPA assets. It is still threaded into `AppState`
/// (which ignores it beyond storage) so both transports build state
/// identically.
///
/// The loop reads one line at a time, discards blank/whitespace-only lines,
/// and writes exactly one JSON response line per handled request, flushing
/// after each so the caller (which is blocked awaiting that exact line) is
/// never left waiting on a buffered write. A malformed or unknown-method
/// line never ends the loop; only stdin EOF (clean `Ok(())`) or an
/// unrecoverable stdout write / stdin read failure (`Err`) terminates it.
pub fn run(asset_root: PathBuf, web_root: Option<PathBuf>) -> Result<(), FittingRoomError> {
    let asset_root_canon = std::fs::canonicalize(&asset_root).map_err(|e| {
        FittingRoomError::Validation(format!(
            "asset_root {} not found: {}",
            asset_root.display(),
            e
        ))
    })?;
    let state = AppState::new(asset_root_canon, web_root);

    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            // stdin closed (EOF) — the parent process is gone; exit cleanly.
            return Ok(());
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Tolerate stray blank / whitespace-only lines without erroring.
            continue;
        }

        let response = handle_line(&state, trimmed);
        // `serde_json` escapes embedded newlines inside string values, so
        // the serialized value never contains a literal `\n` outside the
        // frame boundary we append here — one JSON value per line holds.
        let json_line = serde_json::to_string(&response)
            .map_err(|e| FittingRoomError::Validation(format!("response serialize: {e}")))?;
        writer.write_all(json_line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
}
