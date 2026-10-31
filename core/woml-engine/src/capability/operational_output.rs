//! Invocation-authorized, redacted operational records and progress coalescing, separate from workflow event authority.

use crate::capability::failure;
use crate::capability::operational_invocation_is_active;
use crate::capability::CapabilityFailure;
use crate::capability::CapabilityFailureKind;
use crate::capability::DurableCapabilityAuthority;
use crate::capability::NativeFetchInvocationIdentity;

fn operational_output_failure(code: &str, ambiguous: bool) -> CapabilityFailure {
  failure(
    CapabilityFailureKind::TransportFailed,
    code,
    "The operational-output authority could not accept this script record.",
    false,
    ambiguous,
  )
}

fn redact_operational_body<'a>(
  body: &mut crate::OperationalBodyV1,
  values: impl Iterator<Item = &'a String>,
) {
  let mut redactions = values
    .filter(|value| !value.is_empty())
    .cloned()
    .collect::<Vec<_>>();
  redactions.sort_by_key(|value| std::cmp::Reverse(value.len()));
  let redact = |text: &mut String| {
    for value in &redactions {
      *text = text.replace(value, "[redacted]");
    }
  };
  match body {
    crate::OperationalBodyV1::Output { text, .. } => redact(text),
    crate::OperationalBodyV1::Progress {
      message: Some(message),
      ..
    } => redact(message),
    _ => {}
  }
}

impl DurableCapabilityAuthority {
  pub async fn append_script_operational_record(
    &self,
    identity: &NativeFetchInvocationIdentity,
    body: crate::OperationalBodyV1,
  ) -> Result<(), CapabilityFailure> {
    self
      .append_script_operational_record_inner(identity, body, false)
      .await
  }

  pub async fn force_append_script_operational_record(
    &self,
    identity: &NativeFetchInvocationIdentity,
    body: crate::OperationalBodyV1,
  ) -> Result<(), CapabilityFailure> {
    self
      .append_script_operational_record_inner(identity, body, true)
      .await
  }

  pub(in crate::capability) async fn append_script_operational_record_inner(
    &self,
    identity: &NativeFetchInvocationIdentity,
    mut body: crate::OperationalBodyV1,
    force_append: bool,
  ) -> Result<(), CapabilityFailure> {
    let workflow_id = {
      let store = self.store.lock().await;
      let projection = store
        .projection(&identity.run_id)
        .map_err(|_| operational_output_failure("WOML_OPERATIONAL_IDENTITY_INVALID", false))?;
      if !operational_invocation_is_active(&projection, identity) {
        return Err(operational_output_failure(
          "WOML_OPERATIONAL_INVOCATION_CLOSED",
          false,
        ));
      }
      store
        .run_binding(&identity.run_id)
        .map_err(|_| operational_output_failure("WOML_OPERATIONAL_IDENTITY_INVALID", false))?
        .workflow_id
    };
    let mut redactions = self
      .operational_redactions
      .values()
      .cloned()
      .collect::<Vec<_>>();
    if let Some(artifacts) = self.artifact_store.as_ref() {
      redactions.extend(artifacts.operational_redactions(&identity.invocation_id));
    }
    redact_operational_body(&mut body, redactions.iter());
    let store = self
      .operational_output_store
      .as_ref()
      .ok_or_else(|| operational_output_failure("WOML_OUTPUT_STORE_UNAVAILABLE", false))?
      .activate()
      .map_err(|_| operational_output_failure("WOML_OUTPUT_STORE_UNAVAILABLE", false))?;
    let draft = crate::OperationalRecordDraftV1 {
      identity: crate::OperationalIdentityV1 {
        workflow_id,
        run_id: identity.run_id.clone(),
        node_id: Some(identity.node_id.clone()),
        attempt: Some(identity.attempt_number),
        invocation_id: Some(identity.invocation_id.clone()),
        phase: if identity.invocation_id.starts_with("corr_") {
          crate::OperationalExecutionPhaseV1::Corrective
        } else {
          crate::OperationalExecutionPhaseV1::Forward
        },
      },
      body,
    };
    if !force_append
      && matches!(&draft.body, crate::OperationalBodyV1::Progress { .. })
      && store
        .coalesce_progress_snapshot(&draft)
        .map_err(|_| operational_output_failure("WOML_OUTPUT_STORE_UNAVAILABLE", true))?
    {
      return Ok(());
    }
    store
      .append_bounded(draft)
      .map(|_| ())
      .map_err(|_| operational_output_failure("WOML_OUTPUT_STORE_UNAVAILABLE", true))
  }
}
