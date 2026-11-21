#![cfg(test)]

//! Unit tests for the crate and its related test suites.

use super::*;
use pretty_assertions::assert_eq;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[test]
fn ring_buffer_drops_front_when_full() {
    let fb = CodexFeedback::with_capacity(/*max_bytes*/ 8);
    {
        let mut w = fb.make_writer().make_writer();
        w.write_all(b"abcdefgh").unwrap();
        w.write_all(b"ij").unwrap();
    }
    let snap = fb.snapshot(/*session_id*/ None);
    // Capacity 8: after writing 10 bytes, we should keep the last 8.
    pretty_assertions::assert_eq!(std::str::from_utf8(&snap.bytes).unwrap(), "cdefghij");
}

#[test]
fn logger_layer_filters_noisy_trace_payloads() {
    let fb = CodexFeedback::new();
    let _guard = tracing_subscriber::registry()
        // Keep another TRACE subscriber interested so bridged records are
        // emitted; feedback must still reject them with its own filter.
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink))
        .with(fb.logger_layer())
        .set_default();

    tracing::trace!(target: "codex_api::responses_websocket_timing", payload = "secret");
    tracing::event!(
        target: "codex_otel.log_only", tracing::Level::INFO,
        event.name = "codex.agent_response", response = "private-agent-response"
    );
    tracing::info!(target: "codex_otel.log_only", rationale = "private-guardian-rationale");
    tracing::trace!(target: "codex_http_client::transport", "transport-trace");
    tracing::trace!(target: "codex_api::sse", "sse-trace");
    tracing::trace!(target: "codex_api::sse::responses", "nested-sse-trace");
    tracing::debug!(target: "codex_http_client::transport", "transport-debug");
    tracing::debug!(target: "codex_api::sse::responses", "sse-debug");
    tracing::trace!(target: "codex_feedback_test", "unrelated-trace");
    log::trace!(target: "codex_feedback_test", "unrelated-log-trace");
    log::trace!(
        target: "tungstenite::handshake::client",
        "websocket-handshake-payload"
    );
    log::trace!(target: "tungstenite::protocol", "websocket-frame-payload");
    log::debug!(target: "tungstenite::protocol", "websocket-debug");

    let logs = String::from_utf8(fb.snapshot(/*session_id*/ None).bytes).unwrap();
    for excluded in [
        "secret",
        "private-agent-response",
        "private-guardian-rationale",
        "transport-trace",
        "sse-trace",
        "nested-sse-trace",
        "websocket-handshake-payload",
        "websocket-frame-payload",
    ] {
        assert!(!logs.contains(excluded));
    }
    for retained in [
        "transport-debug",
        "sse-debug",
        "unrelated-trace",
        "unrelated-log-trace",
        "websocket-debug",
    ] {
        assert!(logs.contains(retained));
    }
}

#[test]
fn metadata_layer_records_tags_from_feedback_target() {
    let fb = CodexFeedback::new();
    let _guard = tracing_subscriber::registry()
        .with(fb.metadata_layer())
        .set_default();

    tracing::info!(target: FEEDBACK_TAGS_TARGET, model = "gpt-5", cached = true, "tags");

    let snap = fb.snapshot(/*session_id*/ None);
    pretty_assertions::assert_eq!(snap.tags.get("model").map(String::as_str), Some("gpt-5"));
    pretty_assertions::assert_eq!(snap.tags.get("cached").map(String::as_str), Some("true"));
}

#[test]
fn upload_tags_include_client_tags_and_preserve_reserved_fields() {
    let mut tags = BTreeMap::new();
    tags.insert("thread_id".to_string(), "wrong-thread".to_string());
    tags.insert("turn_id".to_string(), "wrong-turn".to_string());
    tags.insert(
        "classification".to_string(),
        "wrong-classification".to_string(),
    );
    tags.insert("cli_version".to_string(), "wrong-version".to_string());
    tags.insert("session_source".to_string(), "wrong-source".to_string());
    tags.insert("reason".to_string(), "wrong-reason".to_string());
    tags.insert("account_id".to_string(), "actual-account".to_string());
    tags.insert("model".to_string(), "gpt-5".to_string());
    tags.insert("effort".to_string(), "Some(High)".to_string());
    let snapshot = FeedbackSnapshot {
        bytes: Vec::new(),
        tags,
        feedback_diagnostics: FeedbackDiagnostics::default(),
        thread_id: "thread-123".to_string(),
    };
    let mut client_tags = BTreeMap::new();
    client_tags.insert("thread_id".to_string(), "wrong-client-thread".to_string());
    client_tags.insert("turn_id".to_string(), "turn-456".to_string());
    client_tags.insert(
        "classification".to_string(),
        "wrong-client-classification".to_string(),
    );
    client_tags.insert(
        "cli_version".to_string(),
        "wrong-client-version".to_string(),
    );
    client_tags.insert(
        "session_source".to_string(),
        "wrong-client-source".to_string(),
    );
    client_tags.insert("reason".to_string(), "wrong-client-reason".to_string());
    client_tags.insert("client_tag".to_string(), "from-client".to_string());
    client_tags.insert("model".to_string(), "mewthree".to_string());
    client_tags.insert("effort".to_string(), "Some(Ultra)".to_string());

    let upload_tags = snapshot.upload_tags(
        "bug",
        Some("actual reason"),
        Some(&client_tags),
        Some(&SessionSource::Cli),
    );

    assert_eq!(
        upload_tags.get("thread_id").map(String::as_str),
        Some("thread-123")
    );
    assert_eq!(
        upload_tags.get("turn_id").map(String::as_str),
        Some("turn-456")
    );
    assert_eq!(
        upload_tags.get("classification").map(String::as_str),
        Some("bug")
    );
    assert_eq!(
        upload_tags.get("session_source").map(String::as_str),
        Some("cli")
    );
    assert_eq!(
        upload_tags.get("reason").map(String::as_str),
        Some("actual reason")
    );
    assert_eq!(
        upload_tags.get("account_id").map(String::as_str),
        Some("actual-account")
    );
    assert_eq!(
        upload_tags.get("model").map(String::as_str),
        Some("mewthree")
    );
    assert_eq!(
        upload_tags.get("effort").map(String::as_str),
        Some("Some(Ultra)")
    );
    assert_eq!(
        upload_tags.get("client_tag").map(String::as_str),
        Some("from-client")
    );
}

#[path = "save_rollout_tests.rs"]
mod save_rollout_tests;
