//! Regression coverage for conditional interruption and detached abort tracing.
#![cfg(test)]

use super::*;
use crate::session::tests::HeldStepTask;
use crate::session::tests::make_session_and_context_with_rx;
use crate::session::tests::make_thread_for_session;
use crate::state::TaskKind;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tokio::sync::Notify;

#[tokio::test]
async fn interrupt_if_no_pending_input_handles_cancelled_submission() {
    let (session, turn_context, rx_event) = make_session_and_context_with_rx().await;
    session
        .spawn_task(
            Arc::clone(&turn_context),
            Vec::new(),
            HeldStepTask {
                kind: TaskKind::Regular,
                finish: Arc::new(Notify::new()),
            },
        )
        .await;
    let cancellation_token = session
        .active_turn
        .lock()
        .await
        .as_ref()
        .expect("active turn")
        .task
        .as_ref()
        .expect("active task")
        .cancellation_token
        .clone();
    let (thread, rx_sub) = make_thread_for_session(
        Arc::clone(&session),
        &turn_context,
        rx_event,
        SessionSource::Cli,
    )
    .await;
    thread.submit(Op::Interrupt).await.expect("fill the queue");

    let interrupt = thread.interrupt_if_no_pending_input(&turn_context.sub_id);
    tokio::pin!(interrupt);
    assert!(futures::poll!(interrupt.as_mut()).is_pending());
    // The pending submission still owns its reply sender when cancellation wins.
    cancellation_token.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(/*secs*/ 1), interrupt)
            .await
            .expect("cancellation should unblock the submission")
            .expect("cancelling a submission does not mean the agent died"),
        false,
    );
    assert!(!rx_sub.is_closed());
    assert_eq!(rx_sub.len(), 1);
    session.abort_all_tasks(TurnAbortReason::Interrupted).await;
}
