use super::SessionTask;
use super::SessionTaskResult;
use crate::session::TurnInput;
use crate::session::session::Session;
use crate::session::tests::make_session_and_context_with_rx;
use crate::session::turn_context::TurnContext;
use crate::state::TaskKind;
use codex_protocol::models::AudioReference;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::EventMsg;
use codex_protocol::user_input::UserInput;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct PendingTask;

impl SessionTask for PendingTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.pending"
    }

    async fn run(
        self: Arc<Self>,
        _session: Arc<Session>,
        _turn_context: Arc<TurnContext>,
        _input: Vec<TurnInput>,
        _cancellation_token: CancellationToken,
    ) -> SessionTaskResult {
        std::future::pending().await
    }
}

#[tokio::test]
async fn task_finish_rejects_uploaded_audio_without_losing_other_pending_input() {
    let (session, turn_context, receiver) = make_session_and_context_with_rx().await;
    session
        .spawn_task(Arc::clone(&turn_context), Vec::new(), PendingTask)
        .await;
    while receiver.try_recv().is_ok() {}

    let turn_state = {
        let active_turn = session.active_turn.lock().await;
        Arc::clone(
            &active_turn
                .as_ref()
                .expect("active pending task")
                .turn_state,
        )
    };
    session
        .input_queue
        .extend_pending_input_for_turn_state(
            turn_state.as_ref(),
            vec![TurnInput::UserInput {
                content: vec![
                    UserInput::Text {
                        text: "preserve pending text".to_string(),
                        text_elements: Vec::new(),
                    },
                    UserInput::Audio {
                        audio: AudioReference::File {
                            file_id: "file_disabled_at_finish".to_string(),
                        },
                    },
                ],
                client_id: None,
                metadata: Default::default(),
            }],
        )
        .await;

    session
        .on_task_finished(Arc::clone(&turn_context), /*task_result*/ Ok(None))
        .await;

    let history = session.clone_history().await;
    assert!(history.raw_items().any(|item| {
        matches!(item, ResponseItem::Message { content, .. } if content.iter().any(|part| {
            matches!(part, ContentItem::InputText { text } if text == "preserve pending text")
        }))
    }));
    assert!(history.raw_items().all(|item| {
        !matches!(item, ResponseItem::Message { content, .. } if content.iter().any(|part| {
            matches!(part, ContentItem::InputAudio {
                audio: AudioReference::File { file_id, .. },
            } if file_id == "file_disabled_at_finish")
        }))
    }));

    let (saw_error, completed) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut saw_error = false;
        loop {
            match receiver.recv().await.expect("event channel open").msg {
                EventMsg::Error(error) => {
                    saw_error = true;
                    assert_eq!(error.codex_error_info, Some(CodexErrorInfo::BadRequest));
                }
                EventMsg::TurnComplete(completed) => break (saw_error, completed),
                _ => {}
            }
        }
    })
    .await
    .expect("task finish events");
    assert!(saw_error, "the rejected drain should emit an error event");
    assert_eq!(
        completed
            .error
            .as_ref()
            .and_then(|error| error.codex_error_info.clone()),
        Some(CodexErrorInfo::BadRequest)
    );
}

struct UncooperativeTimedTask {
    entered: Arc<tokio::sync::Notify>,
}

impl SessionTask for UncooperativeTimedTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }
    fn span_name(&self) -> &'static str {
        "session_task.uncooperative_timing"
    }
    async fn run(
        self: Arc<Self>,
        _session: Arc<Session>,
        context: Arc<TurnContext>,
        _input: Vec<TurnInput>,
        _cancellation_token: CancellationToken,
    ) -> SessionTaskResult {
        let timing = &context.turn_timing_state;
        timing.enable_critical_path();
        let batch = codex_direct_tool_timing::ToolBatchTiming::default();
        let now = std::time::Instant::now();
        let _blocking;
        let _work = timing.direct_tool_timings.start_turn_direct_call(
            "uncooperative",
            &context.sub_id,
            batch.clone(),
            now,
            timing.snapshot_path(now),
        );
        _blocking = timing.begin_tool_blocking(Some(batch));
        let _review = _work.start_review();
        self.entered.notify_one();
        std::future::pending().await
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hard_abort_drops_timing_guards_before_profile_completion() {
    let (session, context, _receiver) = make_session_and_context_with_rx().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    session
        .spawn_task(
            Arc::clone(&context),
            Vec::new(),
            UncooperativeTimedTask {
                entered: Arc::clone(&entered),
            },
        )
        .await;
    entered.notified().await;
    session
        .abort_all_tasks(codex_protocol::protocol::TurnAbortReason::Interrupted)
        .await;
    // The task's review and tool guards are gone before abort completion.
    assert!(
        context
            .turn_timing_state
            .direct_tool_timings
            .owner_for_turn("uncooperative", &context.sub_id)
            .is_none()
    );
    assert_eq!(
        context
            .turn_timing_state
            .snapshot_path(std::time::Instant::now()),
        None
    );
}
