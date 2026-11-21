//! Exercises reviewer history safety at cancellation and budget boundaries.

use super::*;
use codex_guardian_context::ContextBudgeting;
use codex_guardian_context::ContextPresentation;
use codex_guardian_context::ContextProfile;
use codex_guardian_context::PlannedAction;
use codex_guardian_context::PlannedActionKind;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TurnAbortReason;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use test_case::test_case;

fn required_context(text: String) -> ComposedContext {
    let action = PlannedAction {
        tool_descriptions: None,
        kind: PlannedActionKind::Command { json: text },
        reason: None,
    };
    let context = super::super::prompt::collect_guardian_context(
        &Vec::<ResponseItem>::new(),
        &ContextProfile::synchronous(),
        /*root_conversation_candidates*/ None,
        &[],
        Some(&action),
        /*permissions*/ None,
        /*node_repl*/ None,
    )
    .expect("collect required context");
    codex_guardian_context::prepare_context(
        std::borrow::Cow::Owned(context),
        &ContextProfile::synchronous(),
        /*entry_number_offset*/ 0,
        ContextPresentation::SyncFull {
            session_id: "test-parent",
        },
        codex_guardian_context::ContextBudgeting::Fixed,
        &[],
    )
    .expect("prepare required context")
}

#[tokio::test]
async fn cancelled_startup_does_not_record_unselected_review_evidence() {
    let (session, turn, events) = crate::session::tests::make_session_and_context_with_rx().await;
    let context = required_context("unselected review evidence ".repeat(/*n*/ 10_000));
    let content = context.clone().into_user_inputs().unwrap();
    session
        .services
        .thread_extension_data
        .insert(PendingReviewContext {
            context,
            elastic: None,
        });
    session
        .set_session_startup_prewarm(
            crate::session::startup_prewarm::SessionStartupPrewarmHandle::new(
                tokio::spawn(std::future::pending()),
                std::time::Instant::now(),
                crate::client::WEBSOCKET_CONNECT_TIMEOUT,
            ),
        )
        .await;
    session
        .spawn_task(
            turn,
            vec![TurnInput::UserInput {
                metadata: Default::default(),
                content,
                client_id: None,
            }],
            crate::tasks::RegularTask::new(),
        )
        .await;
    let started = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(started.msg, EventMsg::TurnStarted(_)));
    session.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let history = session.clone_history().await;
    let recorded = history.raw_items().collect::<Vec<_>>();
    assert!(
        !serde_json::to_string(&recorded)
            .unwrap()
            .contains("unselected review evidence")
    );
    assert!(
        session
            .services
            .thread_extension_data
            .get::<PendingReviewContext>()
            .is_some()
    );
}

#[tokio::test]
async fn feasibility_rejects_required_evidence_inside_the_reserved_margin() {
    let (session, mut turn) = crate::session::tests::make_session_and_context().await;
    let context = required_context("required action".to_owned());
    let base = session.get_prompt_base_instructions().await;
    let prefix = codex_protocol::protocol::TruncationPolicy::Bytes(base.text.len()).token_budget();
    let model = Arc::make_mut(&mut Arc::make_mut(&mut turn.initial_settings).model_info);
    model.effective_context_window_percent = 100;
    Arc::make_mut(&mut turn.config).model_context_window =
        Some(i64::try_from(context.estimated_tokens() + prefix + 128).unwrap());
    session
        .services
        .thread_extension_data
        .insert(PendingReviewContext {
            context,
            elastic: None,
        });

    assert!(check_pending(&session, &turn).await.is_err());
}

#[tokio::test]
async fn finalization_overflow_marks_the_reviewer_exhausted() {
    let (session, mut turn) = crate::session::tests::make_session_and_context().await;
    Arc::make_mut(&mut turn.config).model_context_window = Some(1);
    let session = Arc::new(session);
    let step = session
        .capture_step_context(Arc::new(turn), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    let context = required_context("required action".to_owned());
    let mut input = vec![TurnInput::UserInput {
        metadata: Default::default(),
        content: context.clone().into_user_inputs().unwrap(),
        client_id: None,
    }];
    session
        .services
        .thread_extension_data
        .insert(PendingReviewContext {
            context,
            elastic: None,
        });

    assert!(
        finalize(&session, &step, &mut input, HistoryTruncation::Preserve)
            .await
            .is_err()
    );
    assert!(
        session
            .services
            .thread_extension_data
            .get::<super::super::request_budget::ExhaustedReviewBudget>()
            .is_some()
    );
}

#[tokio::test]
#[test_case(ContextBudgeting::Fixed; "fixed")]
#[test_case(ContextBudgeting::Elastic; "elastic")]
async fn compacted_review_restores_originals_once_and_persists_the_request_prefix(
    budgeting: ContextBudgeting,
) {
    const LARGE_INSTRUCTION_REPEATS: usize = 224;
    let instruction = "Do not publish. ".repeat(match budgeting {
        ContextBudgeting::Fixed => 1,
        ContextBudgeting::Elastic => LARGE_INSTRUCTION_REPEATS,
    });
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    let session = Arc::new(session);
    let step = session
        .capture_step_context(Arc::new(turn), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    session
        .record_user_prompt_and_emit_turn_item(
            &step.turn,
            &step.settings.model_info,
            &[codex_protocol::user_input::UserInput::Text {
                text: instruction.clone(),
                text_elements: vec![],
            }],
            /*client_id*/ None,
            crate::session::UserInputMetadata {
                acceptance_order: Some(0),
                ..Default::default()
            },
            codex_thread_store::PersistContext::Standard,
        )
        .await;
    let collected = super::super::prompt::collect_guardian_prompt_items_with_parent_turn(
        &session,
        super::super::prompt::GuardianTranscriptHistory::Retained(
            session.conversation_history_snapshot().await.as_ref(),
        ),
        /*parent_context*/ None,
        super::super::ApprovalRequestReasons {
            approval: None,
            retry: None,
        },
        super::super::GuardianApprovalRequest::RequestPermissions {
            id: "review".to_owned(),
            environment_id: "local".to_owned(),
            turn_id: "parent".to_owned(),
            reason: None,
            permissions: Default::default(),
        },
        super::super::prompt::GuardianPromptMode::Full,
        /*reviewed_node_repl_evidence_sequence*/ 0,
    )
    .await
    .unwrap();
    let context = codex_guardian_context::prepare_context(
        std::borrow::Cow::Owned(collected.sections),
        &collected.profile,
        /*entry_number_offset*/ 0,
        ContextPresentation::SyncFull {
            session_id: &collected.session_id,
        },
        budgeting,
        &[],
    )
    .unwrap();
    let elastic = (budgeting == ContextBudgeting::Elastic).then(|| {
        let section = codex_guardian_context::ElasticSection::TrustedSkills;
        let mut policy = codex_guardian_context::ElasticBudgetConfig::default();
        policy.sections.insert(
            section,
            codex_guardian_context::ElasticSectionLimits::new(
                /*min_total_section_tokens*/ 1, /*max_section_entry_tokens*/ 1,
            )
            .unwrap(),
        );
        let minimum = codex_guardian_context::DEFAULT_MAX_INPUT_TOKENS
            - usize::try_from(policy.total_minimum_tokens()).unwrap()
            + 1;
        policy.sections.insert(
            section,
            codex_guardian_context::ElasticSectionLimits::new(
                minimum, /*max_section_entry_tokens*/ 1,
            )
            .unwrap(),
        );
        policy
    });
    step.turn.extension_data.insert(RetainedReviewContext {
        context: context.retained_instructions(),
        elastic,
        history_version: session.clone_history().await.history_version(),
    });
    // Simulate a reviewer compaction that discarded the earlier user-message block.
    session
        .replace_history(vec![], /*reference_context_item*/ None)
        .await;
    let mut prompt = build_prompt(
        vec![],
        &step,
        session.get_prompt_base_instructions().await,
        /*incremental_tools*/ false,
    );
    let metadata = session
        .responses_metadata(&step, CodexResponsesRequestKind::Turn)
        .await;
    super::super::request_budget::prepare_prompt(&session, &mut prompt, &step, &metadata)
        .await
        .unwrap();
    let restored = serde_json::to_string(&prompt.input).unwrap();
    assert!(restored.contains("Do not publish."));
    assert_eq!(
        restored.contains("<truncated"),
        budgeting == ContextBudgeting::Elastic
    );
    let persisted = session
        .clone_history()
        .await
        .for_prompt(
            &step.settings.model_info.input_modalities,
            session.services.image_store.as_ref(),
        )
        .await;
    assert_eq!(prompt.input, persisted);
    assert!(
        persisted
            .iter()
            .all(crate::context::is_guardian_context_message)
    );
    assert!(
        !persisted
            .iter()
            .any(crate::context::is_user_authorization_message)
    );
    super::super::request_budget::prepare_prompt(&session, &mut prompt, &step, &metadata)
        .await
        .unwrap();
    assert_eq!(prompt.input, persisted);
}
