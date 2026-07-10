//! Property-based test for the `stdio` transport's RPC framing safety.
//!
//! Gated behind the `stdio` feature so default-feature builds still run it
//! but a `--no-default-features` build without `stdio` skips it. Run with:
//! `cargo test --features stdio --test stdio_framing`.
//!
//! ## Property 1: Framing safety
//! **Validates: Requirements 2.1, 2.2**
//!
//! For every `RpcResponse` value `r`, `serde_json::to_string(&r)` contains
//! no `\n` byte — JSON string escaping renders any embedded newline (or
//! carriage return, tab, quote, backslash, control char, or unicode) inside
//! a string value as an escape sequence (`\n`, `\r`, ...), never a literal
//! `0x0A` byte, and the compact serializer emits no whitespace between
//! tokens. Therefore appending exactly one `\n` always produces a frame
//! containing exactly one newline (the terminator), which is precisely what
//! makes the line-oriented reader on the other end (Req 2.2 — "treat
//! everything up to the next `\n` as one complete request") correct.

#![cfg(feature = "stdio")]

use proptest::prelude::*;
use robo_fitting_room::stdio::protocol::{RpcError, RpcErrorData, RpcOutcome, RpcResponse};
use serde_json::Value;

/// Strategy for JSON strings that deliberately stresses the escaping path:
/// fully-arbitrary `char` sequences (which include `\n`, `\r`, `\t`, NUL and
/// other control chars, surrogates-free unicode, quotes, and backslashes)
/// plus a set of hand-picked adversarial literals that are guaranteed to
/// carry the specific bytes framing depends on.
fn arb_json_string() -> impl Strategy<Value = String> {
    prop_oneof![
        // Arbitrary UTF-8: `char::any()` ranges over the whole scalar space,
        // including embedded newlines and control characters.
        prop::collection::vec(any::<char>(), 0..40).prop_map(|cs| cs.into_iter().collect()),
        Just("line1\nline2\nline3".to_string()),
        Just("carriage\r\nwindows".to_string()),
        Just("tab\tseparated".to_string()),
        Just("quote\"inside\"quotes".to_string()),
        Just("back\\slash\\path".to_string()),
        Just("nul\0and\u{0007}bell".to_string()),
        Just("unicode \u{1F600} \u{00E9} \u{2603} snowman".to_string()),
        // A realistic scene-YAML-ish blob (scene.load / scene.save payloads
        // carry multi-line YAML text through the `result`/`error` fields).
        Just("robots:\n  - id: r1\n    urdf: \"a\\b\"\nfloor: null\n".to_string()),
    ]
}

/// Recursive strategy for arbitrary `serde_json::Value` result payloads,
/// nesting arrays and objects whose keys and string values may themselves
/// contain newlines and control characters.
fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("json has no NaN/Infinity", |f| f.is_finite())
            .prop_map(|f| serde_json::json!(f)),
        arb_json_string().prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 48, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::hash_map(arb_json_string(), inner, 0..8)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// Strategy for arbitrary `RpcError` values, including error messages and
/// `data.code`/`data.hint` strings that may contain newlines.
fn arb_rpc_error() -> impl Strategy<Value = RpcError> {
    (
        any::<i32>(),
        arb_json_string(),
        proptest::option::of((arb_json_string(), proptest::option::of(arb_json_string()))),
    )
        .prop_map(|(code, message, data)| RpcError {
            code,
            message,
            data: data.map(|(code, hint)| RpcErrorData { code, hint }),
        })
}

/// Strategy for an arbitrary response outcome: either an `Ok(result)` with an
/// arbitrary JSON payload or an `Err(error)`.
fn arb_outcome() -> impl Strategy<Value = RpcOutcome> {
    prop_oneof![
        arb_json().prop_map(RpcOutcome::Ok),
        arb_rpc_error().prop_map(RpcOutcome::Err),
    ]
}

/// Strategy for an arbitrary `RpcResponse` — arbitrary `id` (including the
/// `None`/parse-error case) and an arbitrary outcome.
fn arb_response() -> impl Strategy<Value = RpcResponse> {
    (proptest::option::of(any::<u64>()), arb_outcome())
        .prop_map(|(id, outcome)| RpcResponse { id, outcome })
}

proptest! {
    /// The framing invariant: serializing any `RpcResponse` yields a string
    /// with no literal newline, so the frame (serialized value + `\n`) has
    /// exactly one newline — its terminator — and the bytes before that
    /// terminator round-trip back to a valid JSON object.
    #[test]
    fn framing_safety_serialization_has_no_embedded_newline(resp in arb_response()) {
        let serialized = serde_json::to_string(&resp)
            .expect("RpcResponse must always serialize");

        // Req 2.1: no `\n` byte occurs within the serialized JSON value.
        prop_assert!(
            !serialized.as_bytes().contains(&b'\n'),
            "serialized response contained an embedded newline: {serialized:?}"
        );

        // Appending the single terminator yields a frame with exactly one
        // newline, positioned at the very end.
        let frame = format!("{serialized}\n");
        prop_assert_eq!(
            frame.as_bytes().iter().filter(|&&b| b == b'\n').count(),
            1
        );
        prop_assert!(frame.ends_with('\n'));

        // Req 2.2: everything up to the terminator is one complete, parseable
        // JSON request/response value. Parse it back as a generic `Value` to
        // confirm the line is self-contained and well-formed.
        let line = &frame[..frame.len() - 1];
        let reparsed: Value = serde_json::from_str(line)
            .expect("the framed line must parse back as valid JSON");
        prop_assert!(reparsed.is_object());
    }
}
