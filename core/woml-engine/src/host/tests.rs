use super::{
  cancel_invocation_calls, ActiveCapabilityCall, ScriptHostClientError, SharedState,
  CAPABILITY_SETTLEMENT_GRACE,
};
use crate::CapabilityCancellationToken;
use std::{sync::Arc, time::Duration};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

#[tokio::test]
async fn completion_waits_for_all_owned_settlements_without_holding_shared_state() {
  let shared = Arc::new(Mutex::new(SharedState::default()));
  let mut acknowledgements = Vec::new();
  let mut cancellations = Vec::new();
  for name in ["first", "second", "other-invocation"] {
    let cancellation = CapabilityCancellationToken::default();
    let (sender, settled) = oneshot::channel();
    shared.lock().await.active_calls.insert(
      (
        if name == "other-invocation" {
          "other"
        } else {
          "completed"
        }
        .into(),
        name.into(),
      ),
      ActiveCapabilityCall {
        cancellation: cancellation.clone(),
        settled,
      },
    );
    cancellations.push(cancellation);
    acknowledgements.push(sender);
  }
  let pending_shared = Arc::clone(&shared);
  let completion =
    tokio::spawn(async move { cancel_invocation_calls(&pending_shared, "completed").await });
  timeout(Duration::from_secs(1), async {
    while !cancellations[0].is_cancelled() || !cancellations[1].is_cancelled() {
      tokio::task::yield_now().await;
    }
  })
  .await
  .unwrap();
  assert!(!cancellations[2].is_cancelled());
  assert_eq!(shared.lock().await.active_calls.len(), 1);
  assert!(!completion.is_finished());
  acknowledgements.remove(0).send(()).unwrap();
  tokio::task::yield_now().await;
  assert!(!completion.is_finished());
  acknowledgements.remove(0).send(()).unwrap();
  completion.await.unwrap().unwrap();
  assert_eq!(shared.lock().await.active_calls.len(), 1);
}

#[tokio::test]
async fn a_lost_settlement_acknowledgement_is_not_treated_as_completion() {
  let shared = Arc::new(Mutex::new(SharedState::default()));
  let (sender, settled) = oneshot::channel();
  let cancellation = CapabilityCancellationToken::default();
  shared.lock().await.active_calls.insert(
    ("invocation".into(), "call".into()),
    ActiveCapabilityCall {
      cancellation: cancellation.clone(),
      settled,
    },
  );
  drop(sender);
  let error = cancel_invocation_calls(&shared, "invocation")
    .await
    .unwrap_err();
  assert!(matches!(error, ScriptHostClientError::HostCrashed(_)));
  assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn an_invocation_without_managed_calls_completes_without_a_settlement_wait() {
  cancel_invocation_calls(&Arc::new(Mutex::new(SharedState::default())), "empty")
    .await
    .unwrap();
}

#[tokio::test]
async fn a_missing_settlement_is_bounded_and_never_reported_as_success() {
  let shared = Arc::new(Mutex::new(SharedState::default()));
  let (_sender, settled) = oneshot::channel();
  shared.lock().await.active_calls.insert(
    ("stuck".into(), "call".into()),
    ActiveCapabilityCall {
      cancellation: CapabilityCancellationToken::default(),
      settled,
    },
  );
  let error = timeout(
    CAPABILITY_SETTLEMENT_GRACE + Duration::from_secs(2),
    cancel_invocation_calls(&shared, "stuck"),
  )
  .await
  .unwrap()
  .unwrap_err();
  assert!(
    matches!(error, ScriptHostClientError::HostCrashed(ref message)
    if message.contains("settlement exceeded"))
  );
}
