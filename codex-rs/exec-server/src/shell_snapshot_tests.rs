use std::collections::HashMap;

use codex_protocol::config_types::ShellEnvironmentPolicyInherit;
use pretty_assertions::assert_eq;
use test_case::test_case;

use super::MAX_FILE_SNAPSHOT_BYTES;
use super::MAX_SNAPSHOT_BYTES;
use super::SnapshotReplay;
use super::parse_snapshot;
use crate::protocol::ExecEnvPolicy;

#[test]
fn snapshot_size_limit_counts_state_and_environment_before_filtering() {
    let half = "x".repeat(MAX_SNAPSHOT_BYTES / 2);
    let oversized = format!("# Snapshot file\n# {half}\n\0\0\0FILTERED={half}\0");
    let policy = ExecEnvPolicy {
        inherit: ShellEnvironmentPolicyInherit::All,
        ignore_default_excludes: false,
        exclude: vec!["FILTERED".to_string()],
        r#set: HashMap::new(),
        include_only: Vec::new(),
    };
    assert!(
        parse_snapshot(
            ShellType::Bash,
            oversized.as_bytes(),
            Some(&policy),
            SnapshotReplay::Environment
        )
        .is_err()
    );
}

#[test]
fn snapshot_filters_profile_exports_after_capture() {
    let policy = ExecEnvPolicy {
        inherit: ShellEnvironmentPolicyInherit::All,
        ignore_default_excludes: false,
        exclude: vec!["PROFILE_DENIED".to_string()],
        r#set: HashMap::from([
            ("PROFILE_ALLOWED".to_string(), "override".to_string()),
            ("CODEX_THREAD_ID".to_string(), "policy-thread".to_string()),
            ("CODEX_TOOL_CALL_ID".to_string(), "policy-call".to_string()),
        ]),
        // Metadata must survive policy filtering so the cache owns its removal.
        include_only: vec![
            "PROFILE_*".to_string(),
            "CODEX_THREAD_ID".to_string(),
            "CODEX_TOOL_CALL_ID".to_string(),
        ],
    };
    let snapshot = parse_snapshot(
        ShellType::Bash,
        b"profile \xff noise\n# Snapshot file\nfunction profile_helper() { :; }\n\0alias profile_alias='profile_helper'\n\0PROFILE_DENIED\0export PROFILE_DENIED=denied\n\0NON_UTF8\0export NON_UTF8='\xff'\n\0\0PROFILE_ALLOWED=profile\0PROFILE_DENIED=denied\0PROFILE_SECRET=secret\0PWD=/tmp\0NON_UTF8=\xff\0CODEX_THREAD_ID=capture-thread\0CODEX_TOOL_CALL_ID=profile-call\0",
        Some(&policy),
        SnapshotReplay::Environment,
    )
    .expect("snapshot should parse");

    assert_eq!(
        snapshot.environment,
        HashMap::from([("PROFILE_ALLOWED".to_string(), "override".to_string())])
    );
    assert_eq!(
        snapshot.state,
        "# Snapshot file\nfunction profile_helper() { :; }\nalias profile_alias='profile_helper'\n"
    );
}

#[test]
fn snapshot_preserves_profile_exports_with_restrictive_inheritance() {
    for inherit in [
        ShellEnvironmentPolicyInherit::None,
        ShellEnvironmentPolicyInherit::Core,
    ] {
        let policy = ExecEnvPolicy {
            inherit,
            ignore_default_excludes: false,
            exclude: vec!["PROFILE_DENIED".to_string()],
            r#set: HashMap::new(),
            include_only: Vec::new(),
        };
        let snapshot = parse_snapshot(
            ShellType::Bash,
            b"# Snapshot file\n\0\0\0PROFILE_ALLOWED=profile\0SDKROOT=/sdk\0PROFILE_SECRET=secret\0PROFILE_DENIED=denied\0",
            Some(&policy),
            SnapshotReplay::Environment,
        )
        .expect("snapshot should parse");

        assert_eq!(
            snapshot.environment,
            HashMap::from([
                ("PROFILE_ALLOWED".to_string(), "profile".to_string()),
                ("SDKROOT".to_string(), "/sdk".to_string()),
            ])
        );
    }
}

#[test]
fn snapshot_caches_only_unmanaged_proxy_state() {
    for (exports, expected) in [
        (
            "PROFILE_ALLOWED=profile\0HTTP_PROXY=http://127.0.0.1:4321\0CODEX_NETWORK_PROXY_ACTIVE=1\0CODEX_NETWORK_PROXY_CREDENTIAL_BROKER_ACTIVE=1\0",
            HashMap::from([("PROFILE_ALLOWED".to_string(), "profile".to_string())]),
        ),
        (
            "PROFILE_ALLOWED=profile\0HTTP_PROXY=http://user-proxy.example\0",
            HashMap::from([
                ("PROFILE_ALLOWED".to_string(), "profile".to_string()),
                (
                    "HTTP_PROXY".to_string(),
                    "http://user-proxy.example".to_string(),
                ),
            ]),
        ),
    ] {
        let output = format!(
            "# Snapshot file\n\0\0\0{exports}CODEX_THREAD_ID=capture-thread\0CODEX_TOOL_CALL_ID=profile-call\0"
        );
        let snapshot = parse_snapshot(
            ShellType::Bash,
            output.as_bytes(),
            /*env_policy*/ None,
            SnapshotReplay::Environment,
        )
        .expect("snapshot should parse");

        assert_eq!(snapshot.environment, expected);
    }
}

use codex_shell_command::shell_detect::ShellType;

#[test_case(SnapshotReplay::File, 1024 * 1024, 0, None; "file_accepts_large_state")]
#[test_case(SnapshotReplay::Environment, 1024 * 1024, 0, Some("state_too_large"); "fallback_rejects_large_state")]
#[test_case(SnapshotReplay::File, MAX_FILE_SNAPSHOT_BYTES, 0, Some("state_too_large"); "file_rejects_oversized_state")]
#[test_case(SnapshotReplay::File, 0, MAX_SNAPSHOT_BYTES, Some("environment_too_large"); "file_keeps_environment_limit")]
fn snapshot_replay_size_limits(
    replay: SnapshotReplay,
    state_bytes: usize,
    environment_bytes: usize,
    expected_failure: Option<&str>,
) {
    let output = format!(
        "# Snapshot file\n# {}\n\0\0\0PADDING={}\0",
        "x".repeat(state_bytes),
        "x".repeat(environment_bytes),
    );
    let result = parse_snapshot(
        ShellType::Bash,
        output.as_bytes(),
        /*env_policy*/ None,
        replay,
    );
    assert_eq!(result.err().map(|(reason, _)| reason), expected_failure);
}
