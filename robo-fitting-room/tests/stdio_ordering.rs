//! Integration test for the `stdio` transport's response ordering guarantee.
//!
//! Gated behind the `stdio` feature. Run with:
//! `cargo test --features stdio --test stdio_ordering`.
//!
//! ## Property 5: Response ordering
//! **Validates: Requirements 3.8**
//!
//! Req 3.8: "The request loop SHALL process requests one at a time,
//! completing and emitting the response for each request before reading the
//! next line." This test drives the *actual built binary* over its real
//! stdin/stdout boundary — the most robust way to observe the ordering
//! contract, since it exercises the same `read_line → handle_line → write →
//! flush` loop the extension host relies on.
//!
//! We feed several newline-delimited requests carrying distinct, deliberately
//! *unsorted* ids and interleaved methods (success + error responses), close
//! stdin, then assert the sequence of `id`s emitted on stdout equals the
//! exact order the requests were written on stdin — not numeric order, not
//! grouped by success/error, but consumption order.

#![cfg(feature = "stdio")]

use std::io::Write;
use std::process::{Command, Stdio};

/// Spawn `robo-fitting-room stdio-serve --asset-root <dir>`, write every
/// line in `requests` to its stdin (in order), close stdin, and return the
/// ordered list of response `id`s parsed from stdout.
fn run_ordered(requests: &[String]) -> Vec<serde_json::Value> {
    let asset_dir = tempfile::tempdir().expect("create asset tempdir");

    let mut child = Command::new(env!("CARGO_BIN_EXE_robo-fitting-room"))
        .arg("stdio-serve")
        .arg("--asset-root")
        .arg(asset_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn stdio-serve child process");

    {
        let mut stdin = child.stdin.take().expect("child stdin piped");
        for line in requests {
            stdin.write_all(line.as_bytes()).expect("write request line");
            stdin.write_all(b"\n").expect("write newline");
        }
        // Dropping `stdin` here closes the pipe → the child sees EOF and
        // exits cleanly after emitting one response per request.
    }

    let output = child
        .wait_with_output()
        .expect("collect stdio-serve output");
    assert!(
        output.status.success(),
        "stdio-serve should exit 0 on stdin EOF; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("response line is not valid JSON: {l:?} ({e})"));
            v["id"].clone()
        })
        .collect()
}

#[test]
fn responses_are_emitted_in_request_consumption_order() {
    // Distinct, intentionally unsorted ids with interleaved methods so that
    // "output order == input order" cannot be satisfied by accidental
    // numeric sorting or by grouping successes ahead of errors.
    //   id 10 health.check   -> result
    //   id  3 does.notExist  -> error (method not found)
    //   id  7 health.ready   -> result
    //   id  1 health.check   -> result
    //   id 42 does.notExist  -> error (method not found)
    //   id  5 health.ready   -> result
    let requests: Vec<String> = vec![
        r#"{"id":10,"method":"health.check","params":{}}"#.to_string(),
        r#"{"id":3,"method":"does.notExist","params":{}}"#.to_string(),
        r#"{"id":7,"method":"health.ready","params":{}}"#.to_string(),
        r#"{"id":1,"method":"health.check","params":{}}"#.to_string(),
        r#"{"id":42,"method":"does.notExist","params":{}}"#.to_string(),
        r#"{"id":5,"method":"health.ready","params":{}}"#.to_string(),
    ];

    let expected_ids: Vec<serde_json::Value> =
        [10u64, 3, 7, 1, 42, 5].iter().map(|n| (*n).into()).collect();

    let got_ids = run_ordered(&requests);

    assert_eq!(
        got_ids.len(),
        requests.len(),
        "expected exactly one response per request"
    );
    assert_eq!(
        got_ids, expected_ids,
        "response ids must appear in the same order their requests were consumed from stdin (Req 3.8)"
    );
}
