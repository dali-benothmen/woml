//! Frozen WOML-only N-API exports and engine delegation.
//!
//! Keep export attributes, argument order and function bodies here. Private
//! configuration, errors, progress and registries adapt the boundary; business
//! rules remain in woml-engine. Helper imports are private to this module tree.

mod configuration;
mod contracts;
mod errors;
mod progress;
mod registries;
mod remote;
mod validation;

use configuration::{
  apply_runtime_capabilities, decode_artifact_maintenance_authority, runtime_options,
  runtime_options_with_modules, runtime_options_with_secrets,
};
use contracts::{
  NativeApprovalDecisionOutcome, NativeManualAdmissionAccepted, NativeManualAdmissionRejected,
  NativeManualAdmissionRequest, NativeRichApprovalDecisionOutcome, NativeRunInspection,
  NativeStoredRunRequirements, NativeTimerWaitSettlement, NativeTriggerIngressAccepted,
  NativeTriggerIngressFailure, NativeTriggerIngressRejected, NativeTriggerIngressRequest,
  NativeWebhookRegistration, NativeWebhookRuntimeStarted,
};
use errors::{
  native_approval_error, native_backup_error, native_execution_error, native_notification_error,
  native_retention_error, native_run_inspection_error, native_run_management_error,
  native_run_presentation_error, native_trigger_runtime_error, operational_output_napi_error,
  run_failure_code, trigger_runtime_napi_error, NativeTriggerRuntimeError,
};
use progress::{
  native_runtime_progress_reporters, runtime_options_with_progress, NativeRuntimeInstanceProgress,
};
use registries::{
  remote_coordinators, webhook_runtimes, NativeManualTarget, NativeRemoteCoordinatorRuntime,
  NativeWebhookRuntimeCommand, NativeWebhookRuntimeThread,
};
use remote::native_prepared_remote_dispatch;
use validation::{encode_manual_rejection, operational_cursor, verify_durable_run_definition};

use chrono::DateTime;
use chrono::Utc;
use napi::Env;
use napi::JsFunction;
use napi::JsObject;
use napi_derive::napi;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use woml_engine::admit_external_signal_durable;
use woml_engine::create_online_backup;
use woml_engine::execute_retention;
use woml_engine::execute_workflow;
use woml_engine::execute_workflow_durable;
use woml_engine::execute_workflow_durable_outcome;
use woml_engine::export_artifact;
use woml_engine::export_artifact_backup;
use woml_engine::inspect_artifact;
use woml_engine::inspect_backup_store;
use woml_engine::inspect_run_artifacts;
use woml_engine::last_retention_result;
use woml_engine::maintain_artifacts;
use woml_engine::model::ValueExpression;
use woml_engine::observe_artifacts;
use woml_engine::parse_target_registry;
use woml_engine::plan_retention;
use woml_engine::prepare_restored_store;
use woml_engine::recent_run_presentations_from_store_v1;
use woml_engine::record_verified_backup;
use woml_engine::recover_durable_runs;
use woml_engine::resolve_human_approval_durable;
use woml_engine::resolve_rich_approval_durable;
use woml_engine::restore_artifact_backup;
use woml_engine::resume_workflow_durable;
use woml_engine::resume_workflow_durable_any_outcome;
use woml_engine::resume_workflow_durable_outcome;
use woml_engine::run_notification_provider_journey_with_custom;
use woml_engine::run_presentation_from_store_v1;
use woml_engine::settle_approval_timeout_durable;
use woml_engine::verify_artifact_backup;
use woml_engine::ApprovalDecision;
use woml_engine::ArtifactPolicyV1;
use woml_engine::ArtifactRuntimeMode;
use woml_engine::CompiledReusableInvocation;
use woml_engine::CompiledWorkflowDefinition;
use woml_engine::CorrectionRecoveryRequestV1;
use woml_engine::CustomNotificationJourneyOptions;
use woml_engine::CustomProviderScriptArtifact;
use woml_engine::DurableEventStore;
use woml_engine::DurableStoreError;
use woml_engine::ExternalSignalAdmissionRequest;
use woml_engine::ExternalTriggerAdmissionCommand;
use woml_engine::NotificationHostProcessOptions;
use woml_engine::NotificationJourneyError;
use woml_engine::RemoteCoordinatorAuthorityV1;
use woml_engine::RemoteCoordinatorServerV1;
use woml_engine::RemoteCoordinatorTlsOptionsV1;
use woml_engine::RemoteDispatchPackageV1;
use woml_engine::RemoteDispatchResolutionV1;
use woml_engine::RemoteDispatchTransportV1;
use woml_engine::RemoteSessionAuthorityV1;
use woml_engine::RemoteWorkerAttemptRuntimeV1;
use woml_engine::ResolvedRemoteSecretProvider;
use woml_engine::RetentionPolicyV1;
use woml_engine::ReviewerIdentityClaimV1;
use woml_engine::ReviewerProviderV1;
use woml_engine::RichApprovalDecisionStatus;
use woml_engine::RichApprovalIdentityDecision;
use woml_engine::RunPresentationError;
use woml_engine::RuntimeExecutionError;
use woml_engine::RuntimeModuleArtifact;
use woml_engine::SystemEngineClock;
use woml_engine::TimerWaitSettlement;
use woml_engine::TriggerAdmissionRequest;
use woml_engine::TrustedWorkerIdentityV1;
use woml_engine::WebhookDefinitionRegistration;
use woml_engine::WebhookRuntimeError;
use woml_engine::WomlWebhookServer;
use woml_engine::WomlWebhookServerConfig;
use woml_engine::WorkerAttemptStateV1;
use woml_engine::WorkerJournalV1;
use woml_engine::COMPILED_MODEL_SCHEMA_VERSION_V19;

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_modules(
    bun_executable,
    script_host_path,
    script_timeout_ms,
    resolved_secrets_json,
    runtime_modules_json,
    runtime_capabilities_json,
  )?;
  let result = execute_workflow(workflow, definition_hash, trigger, options)
    .await
    .map_err(native_execution_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow_durable(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_durable_request",
  );
  decode_span.bytes("model", compiled_model_json.len());
  decode_span.bytes("trigger", trigger_json.len());
  decode_span.bytes(
    "modules",
    runtime_modules_json.as_deref().unwrap_or("[]").len(),
  );
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_modules(
    bun_executable,
    script_host_path,
    script_timeout_ms,
    resolved_secrets_json,
    runtime_modules_json,
    runtime_capabilities_json,
  )?;
  decode_span.count("nodes", workflow.graph.nodes.len());
  decode_span.count("modules", options.runtime_modules.len());
  decode_span.succeed();
  drop(decode_span);
  let mut execute_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.execute_durable",
  );
  let result = execute_workflow_durable(
    workflow,
    definition_hash,
    trigger,
    options,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  execute_span.run_id(result.run_id.clone());
  execute_span.succeed();
  drop(execute_span);
  let mut encode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.encode_durable_result",
  );
  encode_span.run_id(result.run_id.clone());
  let encoded = serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))?;
  encode_span.bytes("result", encoded.len());
  encode_span.succeed();
  Ok(encoded)
}

#[napi(ts_return_type = "Promise<string>")]
pub fn execute_woml_workflow_durable_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_durable_request",
  );
  decode_span.bytes("model", compiled_model_json.len());
  decode_span.bytes("trigger", trigger_json.len());
  decode_span.bytes(
    "modules",
    runtime_modules_json.as_deref().unwrap_or("[]").len(),
  );
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  decode_span.count("nodes", workflow.graph.nodes.len());
  decode_span.count("modules", options.runtime_modules.len());
  decode_span.succeed();
  drop(decode_span);
  env.spawn_future(async move {
    let mut execute_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.execute_durable",
    );
    let result = execute_workflow_durable(
      workflow,
      definition_hash,
      trigger,
      options,
      PathBuf::from(event_store_path),
    )
    .await
    .map_err(native_execution_error)?;
    execute_span.run_id(result.run_id.clone());
    execute_span.succeed();
    drop(execute_span);
    let mut encode_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.encode_durable_result",
    );
    encode_span.run_id(result.run_id.clone());
    let encoded = serde_json::to_string(&result).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML result: {error}"))
    })?;
    encode_span.bytes("result", encoded.len());
    encode_span.succeed();
    Ok(encoded)
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_workflow_durable_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  verify_durable_run_definition(&event_store_path, &run_id, &definition_hash, &workflow)?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let result = resume_workflow_durable(PathBuf::from(event_store_path), &run_id, options)
      .await
      .map_err(native_execution_error)?;
    serde_json::to_string(&result)
      .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML result: {error}")))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn execute_woml_workflow_durable_outcome(
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let outcome = execute_workflow_durable_outcome(
    workflow,
    definition_hash,
    trigger,
    runtime_options_with_modules(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
      runtime_modules_json,
      runtime_capabilities_json,
    )?,
    PathBuf::from(event_store_path),
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn execute_woml_workflow_durable_outcome_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  trigger_json: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let trigger: Map<String, Value> = serde_json::from_str(&trigger_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger JSON: {error}")))?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let outcome = execute_workflow_durable_outcome(
      workflow,
      definition_hash,
      trigger,
      options,
      PathBuf::from(event_store_path),
    )
    .await
    .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn resume_woml_workflow_durable_outcome(
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  let store = DurableEventStore::open(PathBuf::from(&event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let stored_workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  if binding.definition_hash != definition_hash || stored_workflow != workflow {
    return Err(native_execution_error(
      RuntimeExecutionError::InvalidConfiguration(
        "the supplied WOML definition does not match the durable run definition".to_string(),
      ),
    ));
  }
  let outcome = resume_workflow_durable_outcome(
    PathBuf::from(event_store_path),
    &run_id,
    runtime_options_with_modules(
      bun_executable,
      script_host_path,
      script_timeout_ms,
      resolved_secrets_json,
      runtime_modules_json,
      runtime_capabilities_json,
    )?,
  )
  .await
  .map_err(native_execution_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_workflow_durable_outcome_with_progress(
  env: Env,
  compiled_model_json: String,
  definition_hash: String,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_modules_json: Option<String>,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let workflow: CompiledWorkflowDefinition =
    serde_json::from_str(&compiled_model_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid compiled workflow JSON: {error}"))
    })?;
  verify_durable_run_definition(&event_store_path, &run_id, &definition_hash, &workflow)?;
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  let modules: Vec<RuntimeModuleArtifact> =
    serde_json::from_str(runtime_modules_json.as_deref().unwrap_or("[]")).map_err(|error| {
      napi::Error::from_reason(format!("Invalid runtime modules JSON: {error}"))
    })?;
  let options = options.with_runtime_modules(modules);
  env.spawn_future(async move {
    let outcome =
      resume_workflow_durable_outcome(PathBuf::from(event_store_path), &run_id, options)
        .await
        .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi]
pub fn inspect_woml_stored_run_requirements(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let mut required_secrets = workflow
    .graph
    .nodes
    .iter()
    .flat_map(|node| {
      node
        .script_runtime
        .iter()
        .flat_map(|runtime| runtime.required_secrets.iter().cloned())
    })
    .collect::<Vec<_>>();
  required_secrets.extend(
    workflow
      .lifecycle
      .iter()
      .flat_map(|lifecycle| &lifecycle.hooks)
      .flat_map(|hook| &hook.actions)
      .flat_map(|action| action.script_runtime.iter())
      .flat_map(|runtime| runtime.required_secrets.iter().cloned()),
  );
  required_secrets.extend(
    workflow
      .reusable_definitions
      .iter()
      .flatten()
      .filter_map(|definition| match definition {
        CompiledReusableInvocation::NotificationProvider { props, .. } => Some(props),
        _ => None,
      })
      .flatten()
      .filter_map(|prop| match &prop.expression {
        woml_engine::model::CompiledReusablePropExpression::Secret { name } => Some(name.clone()),
        _ => None,
      }),
  );
  required_secrets.sort();
  required_secrets.dedup();
  let has_approval = workflow
    .graph
    .nodes
    .iter()
    .any(|node| node.handler == "engine.approval-wait");
  let has_notifications = workflow.graph.nodes.iter().any(|node| {
    node.handler == "engine.approval-wait"
      && matches!(
        &node.inputs,
        ValueExpression::Object { fields } if fields.contains_key("notifications")
      )
  });
  let requirements = NativeStoredRunRequirements {
    contract: "woml.stored-run-requirements",
    version: 1,
    workflow_id: binding.workflow_id,
    definition_hash: binding.definition_hash,
    required_secrets,
    module_count: workflow
      .module_runtime
      .as_ref()
      .map_or(0, |runtime| runtime.modules.len()),
    has_approval,
    has_notifications,
  };
  serde_json::to_string(&requirements).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode stored run requirements: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn resume_woml_stored_run_with_progress(
  env: Env,
  run_id: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  event_store_path: String,
  progress_callback: JsFunction,
  resolved_secrets_json: String,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<JsObject> {
  let options = runtime_options_with_progress(
    &env,
    bun_executable,
    script_host_path,
    script_timeout_ms,
    progress_callback,
    resolved_secrets_json,
    runtime_capabilities_json,
  )?;
  env.spawn_future(async move {
    let outcome =
      resume_workflow_durable_any_outcome(PathBuf::from(event_store_path), &run_id, options)
        .await
        .map_err(native_execution_error)?;
    serde_json::to_string(&outcome).map_err(|error| {
      napi::Error::from_reason(format!("Could not encode WOML runtime outcome: {error}"))
    })
  })
}

#[napi]
pub fn resolve_woml_approval(
  event_store_path: String,
  token: String,
  decision: String,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let outcome = resolve_human_approval_durable(
    PathBuf::from(event_store_path),
    &token,
    decision,
    &SystemEngineClock,
  )
  .map_err(native_approval_error)?;
  serde_json::to_string(&NativeApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 1,
    outcome,
  })
  .map_err(|error| napi::Error::from_reason(format!("Could not encode approval decision: {error}")))
}

#[napi]
pub fn resolve_woml_notification_approval(
  event_store_path: String,
  capability: String,
  decision: String,
  provider_actor_id: Option<String>,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_approval_error(RuntimeExecutionError::DurableStore(error)))?;
  let provider_actor_id = provider_actor_id
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "custom-provider".to_string());
  let outcome = store
    .resolve_notification_approval(&capability, &provider_actor_id, decision, Utc::now())
    .map_err(|error| native_approval_error(RuntimeExecutionError::DurableStore(error)))?;
  serde_json::to_string(&NativeApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 1,
    outcome,
  })
  .map_err(|error| napi::Error::from_reason(format!("Could not encode approval decision: {error}")))
}

/// Internal ingress used only after the provider host or trusted HTTP gateway
/// has authenticated the provider account and actor. The active Reviewer
/// Directory is runtime-owned and cannot be supplied by this decision call.
#[napi]
#[allow(clippy::too_many_arguments)]
pub fn resolve_woml_rich_approval(
  event_store_path: String,
  run_id: String,
  approval_id: String,
  request_id: String,
  review_digest: String,
  decision: String,
  provider: String,
  provider_account_id: String,
  provider_actor_id: String,
) -> napi::Result<String> {
  let decision = match decision.as_str() {
    "approved" => ApprovalDecision::Approved,
    "rejected" => ApprovalDecision::Rejected,
    _ => {
      return Err(napi::Error::from_reason(
        "Approval decision must be approved or rejected.".to_string(),
      ))
    }
  };
  let provider = match provider.as_str() {
    "slack" => ReviewerProviderV1::Slack,
    "telegram" => ReviewerProviderV1::Telegram,
    "discord" => ReviewerProviderV1::Discord,
    "trusted-http" => ReviewerProviderV1::TrustedHttp,
    _ => {
      return Err(napi::Error::from_reason(
        "Rich approval provider identity is unsupported.".to_string(),
      ))
    }
  };
  let outcome = resolve_rich_approval_durable(
    PathBuf::from(event_store_path),
    RichApprovalIdentityDecision {
      run_id: run_id.clone(),
      request_id: request_id.clone(),
      review_digest: review_digest.clone(),
      decision,
      identity: ReviewerIdentityClaimV1 {
        provider,
        account_id: provider_account_id,
        actor_id: provider_actor_id,
      },
      decided_at: Utc::now(),
    },
  )
  .map_err(native_approval_error)?;
  let status = match outcome.status {
    RichApprovalDecisionStatus::Pending => "recorded",
    RichApprovalDecisionStatus::Approved | RichApprovalDecisionStatus::Rejected => "resolved",
    RichApprovalDecisionStatus::Idempotent => "already_recorded",
    RichApprovalDecisionStatus::AlreadyResolved => "already_resolved",
  };
  serde_json::to_string(&NativeRichApprovalDecisionOutcome {
    contract: "woml.approval-http",
    version: 2,
    status,
    run_id,
    approval_id,
    request_id,
    review_digest,
  })
  .map_err(|error| {
    napi::Error::from_reason(format!("Could not encode rich approval decision: {error}"))
  })
}

#[napi]
pub fn settle_woml_approval_timeout(
  event_store_path: String,
  run_id: String,
  approval_id: String,
) -> napi::Result<String> {
  let outcome = settle_approval_timeout_durable(
    PathBuf::from(event_store_path),
    &run_id,
    &approval_id,
    &SystemEngineClock,
  )
  .map_err(native_approval_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode approval timeout: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub fn start_woml_webhook_runtime(
  env: Env,
  registrations_json: String,
  startup_manual_triggers_json: String,
  bind_address: String,
  event_store_path: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  shutdown_timeout_ms: u32,
  deployment_id: String,
  activation_id: String,
  start_suspended: bool,
  runtime_capabilities_json: Option<String>,
  signal_ingress_token: Option<String>,
  telemetry_enabled: bool,
  telemetry_propagation: bool,
  telemetry_sampling_ratio: f64,
  progress_callback: JsFunction,
) -> napi::Result<JsObject> {
  let mut decode_span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.decode_runtime_registration",
  );
  decode_span.bytes("registrations", registrations_json.len());
  decode_span.bytes("manual_triggers", startup_manual_triggers_json.len());
  let registrations: Vec<NativeWebhookRegistration> = serde_json::from_str(&registrations_json)
    .map_err(|error| {
      napi::Error::from_reason(format!("Invalid webhook registration JSON: {error}"))
    })?;
  let startup_manual_triggers: BTreeMap<String, String> =
    serde_json::from_str(&startup_manual_triggers_json).map_err(|error| {
      napi::Error::from_reason(format!("Invalid startup manual trigger JSON: {error}"))
    })?;
  let bind_address: SocketAddr = bind_address
    .parse()
    .map_err(|error| napi::Error::from_reason(format!("Invalid webhook bind address: {error}")))?;
  let runtime_id = format!("runtime_{}", uuid::Uuid::new_v4().simple());
  let (
    progress_reporter,
    schedule_progress_reporter,
    interval_progress_reporter,
    workflow_call_progress_reporter,
    runtime_policy_progress_reporter,
    runtime_instance_reporter,
    operational_output_reporter,
    telemetry_reporter,
    telemetry_admission_link_reporter,
  ) = native_runtime_progress_reporters(&env, progress_callback, telemetry_enabled)?;
  let mut runtime_secrets = BTreeMap::new();
  for registration in &registrations {
    for (name, value) in &registration.resolved_secrets {
      if runtime_secrets
        .insert(name.clone(), value.clone())
        .is_some_and(|existing| existing != *value)
      {
        return Err(napi::Error::from_reason(format!(
          "Resolved secret {name:?} has conflicting values across workflow registrations."
        )));
      }
    }
  }
  let manual_targets = registrations
    .iter()
    .flat_map(|registration| {
      registration
        .workflow
        .triggers
        .iter()
        .filter(|trigger| trigger.handler == "trigger.manual")
        .map(|trigger| {
          (
            (
              registration.workflow.workflow_id.clone(),
              trigger.id.clone(),
            ),
            NativeManualTarget {
              definition_hash: registration.definition_hash.clone(),
              admission_status: if registration.workflow.schema_version
                >= woml_engine::COMPILED_MODEL_SCHEMA_VERSION_V12
              {
                "queued"
              } else {
                "running"
              },
            },
          )
        })
    })
    .collect::<BTreeMap<_, _>>();
  decode_span.count("workflows", registrations.len());
  let registrations = registrations
    .into_iter()
    .map(|registration| WebhookDefinitionRegistration {
      workflow: registration.workflow,
      definition_hash: registration.definition_hash,
      resolved_secrets: registration.resolved_secrets,
      runtime_modules: registration.runtime_modules,
    })
    .collect();
  let execution = runtime_options(bun_executable, script_host_path, script_timeout_ms)
    .with_resolved_secrets(runtime_secrets)
    .with_schedule_progress_reporter(schedule_progress_reporter)
    .with_interval_progress_reporter(interval_progress_reporter)
    .with_workflow_call_progress_reporter(workflow_call_progress_reporter)
    .with_runtime_policy_progress_reporter(runtime_policy_progress_reporter)
    .with_operational_output_reporter(operational_output_reporter);
  let execution = if let Some(telemetry_reporter) = telemetry_reporter {
    execution.with_telemetry_reporter(
      woml_engine::TelemetrySignalContext {
        runtime_instance_id: runtime_id.clone(),
        deployment_id: deployment_id.clone(),
      },
      telemetry_reporter,
    )
  } else {
    execution
  };
  let execution = execution.with_telemetry_propagation(
    telemetry_enabled && telemetry_propagation,
    telemetry_sampling_ratio,
    telemetry_admission_link_reporter,
  );
  let execution = apply_runtime_capabilities(execution, runtime_capabilities_json)?
    .with_signal_ingress_token(signal_ingress_token)
    .with_shared_script_hosts();
  let signal_execution = execution.clone();
  let config = WomlWebhookServerConfig {
    bind_address,
    database_path: PathBuf::from(event_store_path),
    registrations,
    startup_manual_triggers,
    execution,
    progress_reporter: Some(progress_reporter),
  };
  decode_span.succeed();
  drop(decode_span);

  env.spawn_future(async move {
    let mut start_span = woml_engine::performance::PerformanceSpan::new_for_process(
      "native",
      "napi",
      "napi.start_trigger_runtime",
    );
    let started_at = Utc::now();
    let shutdown_deadline = std::time::Duration::from_millis(u64::from(shutdown_timeout_ms));
    let (startup_sender, startup_receiver) =
      mpsc::sync_channel::<Result<SocketAddr, NativeTriggerRuntimeError>>(1);
    let (control_sender, mut control_receiver) = mpsc::channel::<NativeWebhookRuntimeCommand>();
    let ownership_control = control_sender.clone();
    let (ingress_sender, ingress_receiver) = tokio::sync::mpsc::unbounded_channel();
    let thread_runtime_id = runtime_id.clone();
    let join = std::thread::Builder::new()
      .name(format!("woml-trigger-{runtime_id}"))
      .spawn(move || {
        actix_web::rt::System::new().block_on(async move {
          let ownership = (|| {
            let now = Utc::now();
            let mut store = DurableEventStore::open(&config.database_path)?;
            store.audit_integrity()?;
            store.acquire_runtime_owner(
              &deployment_id,
              &activation_id,
              &thread_runtime_id,
              now,
              now + chrono::Duration::seconds(10),
            )
          })();
          if let Err(error) = ownership {
            let _ = startup_sender.send(Err(native_trigger_runtime_error(
              WebhookRuntimeError::DurableStore(error),
            )));
            return;
          }
          let runtime_database_path = config.database_path.clone();
          match WomlWebhookServer::prepare_with_external_ingress(config, Some(ingress_receiver))
            .await
          {
            Ok(mut server) => {
              if !start_suspended {
                if let Err(error) = server.activate().await {
                  let _ = startup_sender.send(Err(native_trigger_runtime_error(error)));
                  server.stop_with_deadline(shutdown_deadline).await;
                  let _ = DurableEventStore::open(&runtime_database_path).and_then(|mut store| {
                    store.release_runtime_owner(&thread_runtime_id).map(|_| ())
                  });
                  return;
                }
              }
              let address = server.local_address();
              if startup_sender.send(Ok(address)).is_err() {
                server.stop_with_deadline(shutdown_deadline).await;
                return;
              }
              let heartbeat_path = runtime_database_path.clone();
              let heartbeat_runtime_id = thread_runtime_id.clone();
              let heartbeat_control = ownership_control.clone();
              let heartbeat = actix_web::rt::spawn(async move {
                loop {
                  tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                  let now = Utc::now();
                  let renewed = DurableEventStore::open(&heartbeat_path).and_then(|mut store| {
                    store.renew_runtime_owner(
                      &heartbeat_runtime_id,
                      now,
                      now + chrono::Duration::seconds(10),
                    )
                  });
                  if !renewed.is_ok_and(|renewed| renewed) {
                    let _ = heartbeat_control.send(NativeWebhookRuntimeCommand::OwnershipLost);
                    break;
                  }
                }
              });
              loop {
                let received = actix_web::rt::task::spawn_blocking(move || {
                  let command = control_receiver.recv();
                  (control_receiver, command)
                })
                .await;
                let (receiver, command) = match received {
                  Ok(value) => value,
                  Err(_) => break,
                };
                control_receiver = receiver;
                match command {
                  Ok(NativeWebhookRuntimeCommand::Activate(response)) => {
                    let outcome = server
                      .activate()
                      .await
                      .map_err(native_trigger_runtime_error);
                    let _ = response.send(outcome);
                  }
                  Ok(NativeWebhookRuntimeCommand::Signal(request, response)) => {
                    let outcome = admit_external_signal_durable(
                      runtime_database_path.clone(),
                      request,
                      signal_execution.clone(),
                    )
                    .await
                    .map(|outcome| outcome.admission);
                    let _ = response.send(outcome);
                  }
                  Ok(NativeWebhookRuntimeCommand::OwnershipLost) => {
                    let now = Utc::now();
                    runtime_instance_reporter(NativeRuntimeInstanceProgress {
                      profile: "woml.runtime-instance/v1",
                      deployment_id: deployment_id.clone(),
                      activation_id: activation_id.clone(),
                      runtime_instance_id: thread_runtime_id.clone(),
                      runtime_version: env!("CARGO_PKG_VERSION"),
                      native_version: env!("CARGO_PKG_VERSION"),
                      lifecycle: "degraded",
                      started_at,
                      heartbeat_at: now,
                      lease_expires_at: now,
                    });
                    break;
                  }
                  Ok(NativeWebhookRuntimeCommand::Stop) | Err(_) => break,
                }
              }
              heartbeat.abort();
              server.stop_with_deadline(shutdown_deadline).await;
              let _ = DurableEventStore::open(&runtime_database_path)
                .and_then(|mut store| store.release_runtime_owner(&thread_runtime_id).map(|_| ()));
            }
            Err(error) => {
              let _ = startup_sender.send(Err(native_trigger_runtime_error(error)));
              let _ = DurableEventStore::open(&runtime_database_path)
                .and_then(|mut store| store.release_runtime_owner(&thread_runtime_id).map(|_| ()));
            }
          }
        });
      })
      .map_err(|error| {
        napi::Error::from_reason(format!("Could not start WOML webhook runtime: {error}"))
      })?;

    let startup = tokio::task::spawn_blocking(move || startup_receiver.recv())
      .await
      .map_err(|error| napi::Error::from_reason(format!("Webhook startup task failed: {error}")))?
      .map_err(|_| napi::Error::from_reason("Webhook startup channel closed.".to_string()))?;
    let address = match startup {
      Ok(address) => address,
      Err(error) => {
        let _ = tokio::task::spawn_blocking(move || join.join()).await;
        return Err(trigger_runtime_napi_error(error));
      }
    };

    webhook_runtimes()
      .lock()
      .map_err(|_| {
        napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string())
      })?
      .insert(
        runtime_id.clone(),
        NativeWebhookRuntimeThread {
          control: control_sender,
          ingress: ingress_sender,
          manual_targets,
          join,
        },
      );
    let encoded = serde_json::to_string(&NativeWebhookRuntimeStarted {
      runtime_id,
      host: address.ip().to_string(),
      port: address.port(),
    })
    .map_err(|error| {
      napi::Error::from_reason(format!("Could not encode webhook runtime startup: {error}"))
    })?;
    start_span.bytes("result", encoded.len());
    start_span.succeed();
    Ok(encoded)
  })
}

#[napi(ts_return_type = "Promise<void>")]
pub async fn activate_woml_webhook_runtime(runtime_id: String) -> napi::Result<()> {
  let mut span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.activate_trigger_runtime",
  );
  let control = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.control.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML webhook runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = mpsc::sync_channel(1);
  control
    .send(NativeWebhookRuntimeCommand::Activate(response_sender))
    .map_err(|_| napi::Error::from_reason("WOML webhook runtime is stopping.".to_string()))?;
  let result = tokio::task::spawn_blocking(move || response_receiver.recv())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Runtime activation task failed: {error}")))?
    .map_err(|_| napi::Error::from_reason("Runtime activation response was lost.".to_string()))?;
  result.map_err(trigger_runtime_napi_error)?;
  span.succeed();
  Ok(())
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_external_signal(
  runtime_id: String,
  request_json: String,
) -> napi::Result<String> {
  let request: ExternalSignalAdmissionRequest = serde_json::from_str(&request_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid external signal JSON: {error}")))?;
  let control = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.control.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML trigger runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = mpsc::sync_channel(1);
  control
    .send(NativeWebhookRuntimeCommand::Signal(
      request,
      response_sender,
    ))
    .map_err(|_| napi::Error::from_reason("WOML trigger runtime is stopping.".to_string()))?;
  let outcome = tokio::task::spawn_blocking(move || response_receiver.recv())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Signal admission task failed: {error}")))?
    .map_err(|_| napi::Error::from_reason("Signal admission response was lost.".to_string()))?
    .map_err(native_execution_error)?;
  serde_json::to_string(&outcome)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode signal outcome: {error}")))
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_trigger_occurrence(
  runtime_id: String,
  ingress_json: String,
) -> napi::Result<String> {
  let ingress: NativeTriggerIngressRequest = serde_json::from_str(&ingress_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid trigger ingress JSON: {error}")))?;
  if ingress.contract != "woml.trigger-ingress"
    || ingress.contract_version != 1
    || ingress.message_type != "admit"
    || ingress.request_id.is_empty()
    || !matches!(
      ingress.trigger_handler.as_str(),
      "trigger.slack" | "trigger.telegram" | "trigger.discord"
    )
  {
    return Err(napi::Error::from_reason(
      "Invalid communication trigger ingress contract.".to_string(),
    ));
  }
  let request_id = ingress.request_id;
  let sender = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| runtime.ingress.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML trigger runtime does not exist.".to_string()))?;
  let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
  sender
    .send(ExternalTriggerAdmissionCommand {
      request: TriggerAdmissionRequest {
        workflow_id: ingress.workflow_id,
        definition_hash: ingress.definition_hash,
        trigger_id: ingress.trigger_id,
        trigger_handler: ingress.trigger_handler,
        source_identity: ingress.source_identity,
        payload: ingress.payload,
        received_at: ingress.received_at,
      },
      response: response_sender,
    })
    .map_err(|_| napi::Error::from_reason("WOML trigger runtime is stopping.".to_string()))?;
  let outcome = response_receiver
    .await
    .map_err(|_| napi::Error::from_reason("WOML trigger ingress response was lost.".to_string()))?;
  let json = match outcome {
    Ok(outcome) => serde_json::to_string(&NativeTriggerIngressAccepted {
      contract: "woml.trigger-ingress",
      contract_version: 1,
      message_type: "accepted",
      request_id,
      occurrence_id: outcome.occurrence_id,
      run_id: outcome.run_id,
      duplicate: outcome.duplicate,
    }),
    Err(DurableStoreError::TriggerIdempotencyConflict) => {
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code: "WOML_TRIGGER_IDEMPOTENCY_CONFLICT",
          message: "The source identity is already bound to a different payload.",
          retryable: false,
        },
      })
    }
    Err(DurableStoreError::RuntimePolicyQueueFull) => {
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code: "WOML_POLICY_QUEUE_FULL",
          message: "The durable WOML policy queue is full; Slack may redeliver this event.",
          retryable: true,
        },
      })
    }
    Err(error) => {
      let (code, message, retryable) = match error {
        DurableStoreError::DefinitionNotFound(_)
        | DurableStoreError::TriggerDefinitionMismatch => (
          "WOML_TRIGGER_DEFINITION_MISMATCH",
          "The provider event does not match the active workflow definition; restart the WOML runtime.",
          false,
        ),
        DurableStoreError::TriggerNotFound { .. } => (
          "WOML_TRIGGER_NOT_FOUND",
          "The provider event references a trigger that is not registered in the active workflow.",
          false,
        ),
        DurableStoreError::TriggerHandlerMismatch => (
          "WOML_TRIGGER_HANDLER_MISMATCH",
          "The provider event type does not match the compiled workflow trigger.",
          false,
        ),
        DurableStoreError::Contract(_) | DurableStoreError::InvalidModel(_) => (
          "WOML_TRIGGER_CONTRACT_INVALID",
          "The compiled workflow and durable trigger contracts are incompatible; update or rebuild WOML.",
          false,
        ),
        _ => (
          "WOML_TRIGGER_UNAVAILABLE",
          "The durable WOML trigger authority is temporarily unavailable.",
          true,
        ),
      };
      serde_json::to_string(&NativeTriggerIngressRejected {
        contract: "woml.trigger-ingress",
        contract_version: 1,
        message_type: "rejected",
        request_id,
        failure: NativeTriggerIngressFailure {
          code,
          message,
          retryable,
        },
      })
    }
  };
  json.map_err(|error| {
    napi::Error::from_reason(format!("Could not encode trigger ingress outcome: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn submit_woml_manual_trigger(
  runtime_id: String,
  request_json: String,
) -> napi::Result<String> {
  let mut span = woml_engine::performance::PerformanceSpan::new_for_process(
    "native",
    "napi",
    "napi.submit_manual_trigger",
  );
  span.bytes("request", request_json.len());
  let request: NativeManualAdmissionRequest = serde_json::from_str(&request_json)
    .map_err(|error| napi::Error::from_reason(format!("Invalid manual admission JSON: {error}")))?;
  if request.profile != "woml.manual-trigger-admission/v1"
    || request.message_type != "request"
    || request.request_id.is_empty()
    || request.request_id.len() > 256
    || request.workflow_id.is_empty()
    || request.workflow_id.len() > 256
    || request.trigger_id.is_empty()
    || request.trigger_id.len() > 256
    || !request.payload.is_empty()
  {
    return Err(napi::Error::from_reason(
      "Invalid Manual Trigger Admission v1 request.".to_string(),
    ));
  }

  let request_id = request.request_id;
  span.invocation_id(request_id.clone());
  let target_key = (request.workflow_id.clone(), request.trigger_id.clone());
  let runtime = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Trigger runtime registry is unavailable.".to_string()))?
    .get(&runtime_id)
    .map(|runtime| {
      (
        runtime.ingress.clone(),
        runtime.manual_targets.get(&target_key).cloned(),
      )
    });
  let Some((sender, target)) = runtime else {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The runtime is draining and no longer accepts manual triggers.",
    );
  };
  let Some(target) = target else {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_SELECTION_REQUIRED",
      "Select one registered manual workflow trigger.",
    );
  };

  let (response_sender, response_receiver) = tokio::sync::oneshot::channel();
  if sender
    .send(ExternalTriggerAdmissionCommand {
      request: TriggerAdmissionRequest {
        workflow_id: request.workflow_id,
        definition_hash: target.definition_hash,
        trigger_id: request.trigger_id,
        trigger_handler: "trigger.manual".to_string(),
        source_identity: format!("manual:{request_id}"),
        payload: request.payload,
        received_at: request.requested_at,
      },
      response: response_sender,
    })
    .is_err()
  {
    return encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The runtime is draining and no longer accepts manual triggers.",
    );
  }

  let outcome = match response_receiver.await {
    Ok(outcome) => outcome,
    Err(_) => {
      return encode_manual_rejection(
        request_id,
        "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
        "The runtime stopped before the manual trigger could be admitted.",
      );
    }
  };
  if let Ok(admitted) = &outcome {
    span.run_id(admitted.run_id.clone());
  }
  let encoded = match outcome {
    Ok(outcome) => serde_json::to_string(&NativeManualAdmissionAccepted {
      profile: "woml.manual-trigger-admission/v1",
      message_type: "accepted",
      request_id,
      occurrence_id: outcome.occurrence_id,
      run_id: outcome.run_id,
      status: target.admission_status,
    })
    .map_err(|error| {
      napi::Error::from_reason(format!(
        "Could not encode manual admission outcome: {error}"
      ))
    }),
    Err(DurableStoreError::RuntimePolicyQueueFull) => encode_manual_rejection(
      request_id,
      "WOML_POLICY_QUEUE_FULL",
      "The durable workflow queue is full; try the manual trigger again later.",
    ),
    Err(_) => encode_manual_rejection(
      request_id,
      "WOML_MANUAL_TRIGGER_ADMISSION_CLOSED",
      "The durable runtime cannot accept this manual trigger right now.",
    ),
  };
  if encoded.is_ok() {
    span.succeed();
  }
  encoded
}

#[napi(ts_return_type = "Promise<void>")]
pub async fn stop_woml_webhook_runtime(runtime_id: String) -> napi::Result<()> {
  let runtime = webhook_runtimes()
    .lock()
    .map_err(|_| napi::Error::from_reason("Webhook runtime registry is unavailable.".to_string()))?
    .remove(&runtime_id)
    .ok_or_else(|| napi::Error::from_reason("WOML webhook runtime does not exist.".to_string()))?;
  drop(runtime.ingress);
  let _ = runtime.control.send(NativeWebhookRuntimeCommand::Stop);
  tokio::task::spawn_blocking(move || runtime.join.join())
    .await
    .map_err(|error| napi::Error::from_reason(format!("Webhook shutdown task failed: {error}")))?
    .map_err(|_| {
      napi::Error::from_reason("WOML webhook runtime panicked during shutdown.".to_string())
    })?;
  Ok(())
}

#[napi]
pub fn inspect_woml_run(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(native_run_inspection_error)?;
  let projection = store
    .projection(&run_id)
    .map_err(native_run_inspection_error)?;
  let workflow_calls = store
    .workflow_call_relations_for_run(&run_id)
    .map_err(native_run_inspection_error)?;
  let inspection = NativeRunInspection {
    run_id: projection.run_id.unwrap_or(run_id),
    workflow_id: projection.workflow_id.unwrap_or_default(),
    status: projection.status,
    terminal_node_id: projection.terminal_node_id,
    result: projection.result,
    failure_code: projection.failure.as_ref().map(run_failure_code),
    workflow_calls,
  };
  serde_json::to_string(&inspection)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML run: {error}")))
}

#[napi]
pub fn inspect_woml_remote_dispatch(
  event_store_path: String,
  dispatch_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_TARGET_UNAVAILABLE", error))?;
  let inspection = store
    .inspect_remote_dispatch(&dispatch_id)
    .map_err(|error| napi::Error::from_reason(format!("{}: {}", error.code(), error)))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode remote dispatch inspection: {error}"
    ))
  })
}

#[napi]
#[allow(clippy::too_many_arguments)]
pub fn start_woml_remote_loopback_coordinator(
  runtime_id: String,
  target_registry_json: String,
  trusted_workers_json: String,
  supported_capabilities_json: String,
  event_store_path: String,
  certificate_path: String,
  private_key_path: String,
  client_ca_path: String,
) -> napi::Result<String> {
  if runtime_id.is_empty() || runtime_id.len() > 256 {
    return Err(napi::Error::from_reason(
      "WOML_REMOTE_PROTOCOL_INVALID: invalid coordinator runtime id".to_string(),
    ));
  }
  let registry = parse_target_registry(target_registry_json.as_bytes())
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  let trust: Vec<TrustedWorkerIdentityV1> =
    serde_json::from_str(&trusted_workers_json).map_err(|_| {
      napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid worker trust".to_string())
    })?;
  let capabilities: BTreeMap<String, u32> = serde_json::from_str(&supported_capabilities_json)
    .map_err(|_| {
      napi::Error::from_reason(
        "WOML_REMOTE_PROTOCOL_INVALID: invalid coordinator capabilities".to_string(),
      )
    })?;
  let authority = RemoteSessionAuthorityV1::new_dispatching(registry, trust, capabilities)
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  let transport = RemoteDispatchTransportV1::new(event_store_path);
  let server = RemoteCoordinatorServerV1::start_dispatching(
    authority,
    transport.clone(),
    RemoteCoordinatorTlsOptionsV1 {
      bind_address: "127.0.0.1:0"
        .parse::<SocketAddr>()
        .expect("fixed loopback address"),
      certificate_path: PathBuf::from(certificate_path),
      private_key_path: PathBuf::from(private_key_path),
      client_ca_path: PathBuf::from(client_ca_path),
    },
  )
  .map_err(|error| napi::Error::from_reason(format!("WOML_TARGET_UNAVAILABLE: {error}")))?;
  let local_address = server.local_address();
  let mut runtimes = remote_coordinators().lock().map_err(|_| {
    napi::Error::from_reason(
      "WOML_TARGET_UNAVAILABLE: coordinator registry unavailable".to_string(),
    )
  })?;
  if runtimes.contains_key(&runtime_id) {
    return Err(napi::Error::from_reason(
      "WOML_REMOTE_DISPATCH_CONFLICT: coordinator runtime already exists".to_string(),
    ));
  }
  runtimes.insert(
    runtime_id.clone(),
    NativeRemoteCoordinatorRuntime { server, transport },
  );
  serde_json::to_string(&serde_json::json!({
    "profile": "woml.remote-loopback-coordinator/v1",
    "runtimeId": runtime_id,
    "address": local_address.to_string(),
    "dispatchEnabled": true
  }))
  .map_err(|_| napi::Error::from_reason("Could not encode loopback coordinator startup."))
}

#[napi]
pub fn enqueue_woml_remote_dispatch(
  runtime_id: String,
  dispatch_package_json: String,
  coordinator_authority_json: String,
) -> napi::Result<String> {
  let package: RemoteDispatchPackageV1 =
    serde_json::from_str(&dispatch_package_json).map_err(|_| {
      napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid dispatch package")
    })?;
  let authority: RemoteCoordinatorAuthorityV1 = serde_json::from_str(&coordinator_authority_json)
    .map_err(|_| {
    napi::Error::from_reason("WOML_REMOTE_PROTOCOL_INVALID: invalid coordinator authority")
  })?;
  let transport = remote_coordinators()
    .lock()
    .map_err(|_| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE"))?
    .get(&runtime_id)
    .map(|runtime| runtime.transport.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE: coordinator not found"))?;
  transport
    .enqueue(package, authority, Utc::now())
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))
}

#[napi]
pub fn maintain_woml_remote_dispatches(runtime_id: String) -> napi::Result<String> {
  let transport = remote_coordinators()
    .lock()
    .map_err(|_| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE"))?
    .get(&runtime_id)
    .map(|runtime| runtime.transport.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE: coordinator not found"))?;
  let now = Utc::now();
  let changed = transport
    .mark_lost(now, now)
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  serde_json::to_string(&changed)
    .map_err(|_| napi::Error::from_reason("Could not encode remote maintenance result."))
}

#[napi]
pub fn read_woml_remote_terminal(
  runtime_id: String,
  dispatch_id: String,
) -> napi::Result<Option<String>> {
  let transport = remote_coordinators()
    .lock()
    .map_err(|_| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE"))?
    .get(&runtime_id)
    .map(|runtime| runtime.transport.clone())
    .ok_or_else(|| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE: coordinator not found"))?;
  transport
    .terminal_fact(&dispatch_id)
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?
    .map(|terminal| {
      serde_json::to_string(&terminal)
        .map_err(|_| napi::Error::from_reason("Could not encode remote terminal fact."))
    })
    .transpose()
}

#[napi]
pub fn stop_woml_remote_loopback_coordinator(runtime_id: String) -> napi::Result<()> {
  let runtime = remote_coordinators()
    .lock()
    .map_err(|_| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE"))?
    .remove(&runtime_id)
    .ok_or_else(|| napi::Error::from_reason("WOML_TARGET_UNAVAILABLE: coordinator not found"))?;
  runtime.server.shutdown();
  Ok(())
}

#[napi]
#[allow(clippy::too_many_arguments)]
pub fn prepare_woml_remote_attempt(
  dispatch_package_json: String,
  coordinator_authority_json: String,
  worker_authority_json: String,
  worker_id: String,
  session_id: String,
  target: String,
  worker_journal_path: String,
) -> napi::Result<String> {
  let prepared = native_prepared_remote_dispatch(
    &dispatch_package_json,
    &coordinator_authority_json,
    &worker_authority_json,
    &worker_id,
    &target,
  )?;
  let journal_path = PathBuf::from(worker_journal_path);
  let mut journal = WorkerJournalV1::open(&journal_path)
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  let record = journal
    .accept(&prepared, &worker_id, &session_id, Utc::now())
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  if let Some(terminal) = record.terminal {
    return serde_json::to_string(&serde_json::json!({
      "profile": "woml.remote-attempt-preparation/v1",
      "status": "terminal",
      "dispatchId": prepared.package.dispatch_id,
      "terminal": terminal
    }))
    .map_err(|_| napi::Error::from_reason("Could not encode remote attempt preparation."));
  }
  if record.state != WorkerAttemptStateV1::Accepted {
    return Err(napi::Error::from_reason(
      "WOML_REMOTE_DISPATCH_AMBIGUOUS: remote attempt is not safely restartable".to_string(),
    ));
  }
  serde_json::to_string(&serde_json::json!({
    "profile": "woml.remote-attempt-preparation/v1",
    "status": "accepted",
    "dispatchId": prepared.package.dispatch_id,
    "terminal": null
  }))
  .map_err(|_| napi::Error::from_reason("Could not encode remote attempt preparation."))
}

#[napi]
pub fn start_woml_remote_attempt(
  worker_journal_path: String,
  dispatch_id: String,
  session_id: String,
) -> napi::Result<()> {
  let journal_path = PathBuf::from(worker_journal_path);
  let mut journal = WorkerJournalV1::open(&journal_path)
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  let record = journal
    .start(&dispatch_id, &session_id, Utc::now())
    .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  if record.state != WorkerAttemptStateV1::Started
    || !journal
      .was_started_by_session(&dispatch_id, &session_id)
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?
  {
    return Err(napi::Error::from_reason(
      "WOML_REMOTE_DISPATCH_AMBIGUOUS: remote attempt start is not bound to this session"
        .to_string(),
    ));
  }
  Ok(())
}

#[napi(ts_return_type = "Promise<string>")]
#[allow(clippy::too_many_arguments)]
pub async fn execute_woml_remote_attempt(
  dispatch_package_json: String,
  coordinator_authority_json: String,
  worker_authority_json: String,
  worker_id: String,
  session_id: String,
  target: String,
  capacity: u32,
  prestarted: bool,
  worker_journal_path: String,
  bun_executable: String,
  script_host_path: String,
  script_timeout_ms: u32,
  resolved_secrets_json: String,
  runtime_capabilities_json: Option<String>,
) -> napi::Result<String> {
  let secrets: BTreeMap<String, String> = serde_json::from_str(&resolved_secrets_json)
    .map_err(|_| napi::Error::from_reason("WOML_SECRET_UNAVAILABLE: invalid resolved secrets"))?;
  let prepared = native_prepared_remote_dispatch(
    &dispatch_package_json,
    &coordinator_authority_json,
    &worker_authority_json,
    &worker_id,
    &target,
  )?;
  let journal_path = PathBuf::from(worker_journal_path);
  let mut options = runtime_options_with_modules(
    bun_executable,
    script_host_path,
    script_timeout_ms,
    resolved_secrets_json,
    None,
    runtime_capabilities_json,
  )?;
  if prepared
    .effective_authority
    .capabilities
    .iter()
    .any(|capability| capability.id != "runtime.script")
  {
    options = options
      .prepare_remote_worker_capabilities(&journal_path)
      .map_err(native_execution_error)?;
  }
  let mut runtime = RemoteWorkerAttemptRuntimeV1::new(
    WorkerJournalV1::open(&journal_path)
      .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?,
    worker_id,
    session_id,
    capacity as usize,
    options.script_host.clone(),
    options.remote_worker_capability_authority(),
    Arc::new(ResolvedRemoteSecretProvider::new(secrets)),
  )
  .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  let terminal = if prestarted {
    runtime.execute_prestarted(&prepared, Utc::now()).await
  } else {
    runtime.execute(&prepared, Utc::now()).await
  }
  .map_err(|error| napi::Error::from_reason(format!("{}: {error}", error.code())))?;
  serde_json::to_string(&terminal).map_err(|_| {
    napi::Error::from_reason("WOML_REMOTE_RESULT_INVALID: terminal fact could not be encoded")
  })
}

#[napi]
pub fn reconcile_woml_remote_dispatch(
  event_store_path: String,
  dispatch_id: String,
  resolution: String,
  actor: String,
  evidence_digest: String,
) -> napi::Result<String> {
  let resolution = match resolution.as_str() {
    "record-failure" => RemoteDispatchResolutionV1::RecordFailure,
    "authorize-possible-duplicate-retry" => {
      RemoteDispatchResolutionV1::AuthorizePossibleDuplicateRetry
    }
    _ => {
      return Err(napi::Error::from_reason(
        "WOML_REMOTE_PROTOCOL_INVALID: invalid reconciliation resolution".to_string(),
      ))
    }
  };
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_TARGET_UNAVAILABLE", error))?;
  let record = store
    .reconcile_remote_dispatch(
      &dispatch_id,
      resolution,
      &actor,
      &evidence_digest,
      Utc::now(),
    )
    .map_err(|error| napi::Error::from_reason(format!("{}: {}", error.code(), error)))?;
  serde_json::to_string(&record).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode remote dispatch reconciliation: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_run_presentation(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  let presentation =
    run_presentation_from_store_v1(&store, &run_id).map_err(native_run_presentation_error)?;
  serde_json::to_string(&presentation).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run presentation: {error}"))
  })
}

#[napi]
pub fn list_woml_run_presentations(
  event_store_path: String,
  workflow_id: String,
  limit: u32,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  let presentations = recent_run_presentations_from_store_v1(
    &store,
    &workflow_id,
    usize::try_from(limit).unwrap_or(usize::MAX),
  )
  .map_err(native_run_presentation_error)?;
  serde_json::to_string(&presentations).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run presentations: {error}"))
  })
}

#[napi]
pub fn has_woml_workflow_definition(
  event_store_path: String,
  workflow_id: String,
) -> napi::Result<bool> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))?;
  store
    .has_definition_for_workflow(&workflow_id)
    .map_err(|error| native_run_presentation_error(RunPresentationError::Store(error)))
}

#[napi]
pub fn list_woml_runs(
  event_store_path: String,
  limit: u32,
  workflow_id: Option<String>,
  status: Option<String>,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_LIST_FAILED", error))?;
  let list = store
    .list_runs_v2_filtered(
      usize::try_from(limit).unwrap_or(usize::MAX),
      workflow_id.as_deref(),
      status.as_deref(),
    )
    .map_err(|error| native_run_management_error("WOML_RUN_LIST_FAILED", error))?;
  serde_json::to_string(&list)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode WOML run list: {error}")))
}

#[napi]
pub fn observe_woml_runtime(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_OBSERVABILITY_UNAVAILABLE", error))?;
  let observation = store
    .runtime_observation_v1()
    .map_err(|error| native_run_management_error("WOML_OBSERVABILITY_UNAVAILABLE", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML runtime observation: {error}"
    ))
  })
}

#[napi]
pub fn read_woml_operational_output_page(
  event_store_path: String,
  run_id: Option<String>,
  after: f64,
  limit: u32,
) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let after = operational_cursor(after)?;
  let page = match run_id {
    Some(run_id) => {
      store.read_run_page(&run_id, after, usize::try_from(limit).unwrap_or(usize::MAX))
    }
    None => store.read_page(after, usize::try_from(limit).unwrap_or(usize::MAX)),
  }
  .map_err(operational_output_napi_error)?;
  serde_json::to_string(&page).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational output: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_operational_progress(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let progress = store
    .latest_progress(&run_id)
    .map_err(operational_output_napi_error)?;
  serde_json::to_string(&progress).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational progress: {error}"
    ))
  })
}

#[napi]
pub fn observe_woml_operational_output(event_store_path: String) -> napi::Result<String> {
  let store = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .and_then(|store| store.activate())
    .map_err(operational_output_napi_error)?;
  let observation = store.inspect().map_err(operational_output_napi_error)?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode operational observation: {error}"
    ))
  })
}

#[napi]
pub fn plan_woml_operational_output_prune(
  event_store_path: String,
  cutoff: String,
) -> napi::Result<String> {
  let cutoff = DateTime::parse_from_rfc3339(&cutoff)
    .map_err(|_| {
      napi::Error::from_reason(
        "WOML_OUTPUT_RETENTION_INVALID: the output retention cutoff is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  let lazy = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .map_err(operational_output_napi_error)?;
  let plan = match lazy
    .activate_if_exists()
    .map_err(operational_output_napi_error)?
  {
    Some(store) => store
      .plan_prune_terminal_before(cutoff, 1_000)
      .map_err(operational_output_napi_error)?,
    None => woml_engine::OperationalOutputPrunePlanV1 {
      profile: "woml.operational-output-prune-plan/v1".to_string(),
      eligible_runs: 0,
      eligible_records: 0,
      eligible_output_bytes: 0,
    },
  };
  serde_json::to_string(&plan).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode output retention plan: {error}"
    ))
  })
}

#[napi]
pub fn prune_woml_operational_output(
  event_store_path: String,
  cutoff: String,
) -> napi::Result<String> {
  let cutoff = DateTime::parse_from_rfc3339(&cutoff)
    .map_err(|_| {
      napi::Error::from_reason(
        "WOML_OUTPUT_RETENTION_INVALID: the output retention cutoff is invalid".to_string(),
      )
    })?
    .with_timezone(&Utc);
  let lazy = woml_engine::operational_output_store_for_state(&PathBuf::from(event_store_path))
    .map_err(operational_output_napi_error)?;
  let result = match lazy
    .activate_if_exists()
    .map_err(operational_output_napi_error)?
  {
    Some(store) => store
      .prune_terminal_before(cutoff, 1_000)
      .map_err(operational_output_napi_error)?,
    None => woml_engine::OperationalOutputPruneResultV1 {
      profile: "woml.operational-output-prune/v1".to_string(),
      pruned_runs: 0,
      pruned_records: 0,
      pruned_output_bytes: 0,
      pruned_through: 0,
    },
  };
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "WOML_OUTPUT_STORE_UNAVAILABLE: could not encode output retention result: {error}"
    ))
  })
}

#[napi]
pub fn create_woml_backup(
  event_store_path: String,
  destination_path: String,
  lease_id: String,
  owner_id: String,
  fallback_deployment_id: String,
) -> napi::Result<String> {
  let inspection = create_online_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    &lease_id,
    &owner_id,
    &fallback_deployment_id,
  )
  .map_err(|error| native_backup_error(error, "WOML_BACKUP_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode backup inventory: {error}"))
  })
}

#[napi]
pub fn inspect_woml_backup_store(event_store_path: String) -> napi::Result<String> {
  let inspection = inspect_backup_store(PathBuf::from(event_store_path))
    .map_err(|error| native_backup_error(error, "WOML_BACKUP_VERIFICATION_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode backup inspection: {error}"))
  })
}

#[napi]
pub fn record_woml_verified_backup(
  event_store_path: String,
  backup_id: String,
  completed_at: String,
) -> napi::Result<()> {
  let completed_at = DateTime::parse_from_rfc3339(&completed_at)
    .map_err(|_| napi::Error::from_reason("Backup completion time is invalid.".to_string()))?
    .with_timezone(&Utc);
  record_verified_backup(PathBuf::from(event_store_path), &backup_id, completed_at)
    .map_err(|error| native_backup_error(error, "WOML_BACKUP_FAILED"))
}

#[napi]
pub fn prepare_woml_restored_store(
  event_store_path: String,
  expected_definition_hashes_json: String,
  backup_id: String,
  restored_at: String,
) -> napi::Result<String> {
  let expected: Vec<String> =
    serde_json::from_str(&expected_definition_hashes_json).map_err(|_| {
      napi::Error::from_reason("Restore definition inventory is invalid.".to_string())
    })?;
  let restored_at = DateTime::parse_from_rfc3339(&restored_at)
    .map_err(|_| napi::Error::from_reason("Restore time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let inspection = prepare_restored_store(
    PathBuf::from(event_store_path),
    &expected,
    &backup_id,
    restored_at,
  )
  .map_err(|error| native_backup_error(error, "WOML_RESTORE_FAILED"))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode restore inspection: {error}"))
  })
}

#[napi]
pub fn plan_woml_retention(
  event_store_path: String,
  policy_json: String,
  now: String,
) -> napi::Result<String> {
  let policy: RetentionPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Retention Policy v1 is invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Retention planning time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let plan = plan_retention(PathBuf::from(event_store_path), &policy, now)
    .map_err(native_retention_error)?;
  serde_json::to_string(&plan)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode retention plan: {error}")))
}

#[napi]
pub fn execute_woml_retention(
  event_store_path: String,
  policy_json: String,
  lease_id: String,
  owner_id: String,
  compact: bool,
  now: String,
) -> napi::Result<String> {
  let policy: RetentionPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Retention Policy v1 is invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Retention execution time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let outcome = execute_retention(
    PathBuf::from(event_store_path),
    &policy,
    &lease_id,
    &owner_id,
    compact,
    now,
  )
  .map_err(native_retention_error)?;
  serde_json::to_string(&outcome).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode retention result: {error}"))
  })
}

#[napi]
pub async fn execute_woml_retention_async(
  event_store_path: String,
  policy_json: String,
  lease_id: String,
  owner_id: String,
  compact: bool,
  now: String,
) -> napi::Result<String> {
  tokio::task::spawn_blocking(move || {
    execute_woml_retention(
      event_store_path,
      policy_json,
      lease_id,
      owner_id,
      compact,
      now,
    )
  })
  .await
  .map_err(|error| {
    napi::Error::from_reason(format!("Retention worker could not complete: {error}"))
  })?
}

#[napi]
pub fn read_woml_last_retention_result(event_store_path: String) -> napi::Result<String> {
  let result =
    last_retention_result(PathBuf::from(event_store_path)).map_err(native_retention_error)?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode retention audit: {error}")))
}

#[napi]
pub fn maintain_woml_artifacts(
  event_store_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
  now: String,
  dry_run: bool,
) -> napi::Result<String> {
  let policy: ArtifactPolicyV1 = serde_json::from_str(&policy_json)
    .map_err(|_| napi::Error::from_reason("Artifact Policy v1 is invalid.".to_string()))?;
  let mode: ArtifactRuntimeMode = serde_json::from_str(&mode_json)
    .map_err(|_| napi::Error::from_reason("Artifact runtime mode is invalid.".to_string()))?;
  let secrets: BTreeMap<String, String> = serde_json::from_str(&resolved_secrets_json)
    .map_err(|_| napi::Error::from_reason("Resolved artifact secrets are invalid.".to_string()))?;
  let now = DateTime::parse_from_rfc3339(&now)
    .map_err(|_| napi::Error::from_reason("Artifact maintenance time is invalid.".to_string()))?
    .with_timezone(&Utc);
  let result = maintain_artifacts(
    PathBuf::from(event_store_path),
    policy,
    mode,
    &secrets,
    now,
    dry_run,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact maintenance: {error}"))
  })
}

#[napi]
pub fn inspect_woml_artifact(
  event_store_path: String,
  artifact_id: String,
) -> napi::Result<String> {
  let result =
    inspect_artifact(PathBuf::from(event_store_path), &artifact_id).map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_run_artifacts(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let result =
    inspect_run_artifacts(PathBuf::from(event_store_path), &run_id).map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode run artifact inspection: {error}"))
  })
}

#[napi]
pub fn export_woml_artifact(
  event_store_path: String,
  destination_path: String,
  artifact_id: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = export_artifact(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    &artifact_id,
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode artifact export: {error}")))
}

#[napi]
pub fn observe_woml_artifacts(
  event_store_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = observe_artifacts(PathBuf::from(event_store_path), policy, mode, &secrets).map_err(
    |failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)),
  )?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode artifact observation: {error}"))
  })
}

#[napi]
pub fn export_woml_artifact_backup(
  event_store_path: String,
  destination_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = export_artifact_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(destination_path),
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact backup inventory: {error}"
    ))
  })
}

#[napi]
pub fn restore_woml_artifact_backup(
  event_store_path: String,
  source_path: String,
  policy_json: String,
  mode_json: String,
  resolved_secrets_json: String,
) -> napi::Result<String> {
  let (policy, mode, secrets) =
    decode_artifact_maintenance_authority(&policy_json, &mode_json, &resolved_secrets_json)?;
  let result = restore_artifact_backup(
    PathBuf::from(event_store_path),
    PathBuf::from(source_path),
    policy,
    mode,
    &secrets,
  )
  .map_err(|failure| napi::Error::from_reason(format!("{}: {}", failure.code, failure.message)))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact restore inventory: {error}"
    ))
  })
}

#[napi]
pub fn verify_woml_artifact_backup(
  event_store_path: String,
  source_path: String,
) -> napi::Result<String> {
  let result = verify_artifact_backup(PathBuf::from(event_store_path), PathBuf::from(source_path))
    .map_err(|failure| {
      napi::Error::from_reason(format!("{}: {}", failure.code, failure.message))
    })?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode artifact backup verification: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_run_v2(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
  let binding = store.run_binding(&run_id).map_err(|error| {
    let code = if matches!(error, DurableStoreError::RunNotFound(_)) {
      "WOML_RUN_NOT_FOUND"
    } else {
      "WOML_RUN_INSPECTION_FAILED"
    };
    native_run_management_error(code, error)
  })?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
  let mut inspection = if workflow.schema_version >= 20 {
    serde_json::to_value(
      store
        .inspect_run_v11(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 16 {
    serde_json::to_value(
      store
        .inspect_run_v6(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 14 {
    serde_json::to_value(
      store
        .inspect_run_v5(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 13 {
    serde_json::to_value(
      store
        .inspect_run_v4(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else if workflow.schema_version >= 12 {
    serde_json::to_value(
      store
        .inspect_run_v3(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  } else {
    serde_json::to_value(
      store
        .inspect_run_v2(&run_id)
        .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?,
    )
  }
  .map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run inspection: {error}"))
  })?;
  if let Some(policy) = store
    .infrastructure_policy_binding(&run_id)
    .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?
  {
    let projection = store
      .projection(&run_id)
      .map_err(|error| native_run_management_error("WOML_RUN_INSPECTION_FAILED", error))?;
    let total_decisions = projection.infrastructure_policy_decisions.len();
    let decisions = projection
      .infrastructure_policy_decisions
      .iter()
      .rev()
      .take(100)
      .map(|event| event.decision.clone())
      .collect::<Vec<_>>();
    if let Some(object) = inspection.as_object_mut() {
      object.insert(
        "infrastructurePolicy".to_string(),
        serde_json::json!({
          "profile": "woml.infrastructure-policy-run-inspection/v1",
          "binding": policy,
          "decisions": decisions,
          "totalDecisions": total_decisions,
          "truncated": total_decisions > 100,
        }),
      );
    }
  }
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML run inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_waits_v1(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_WAIT_INSPECTION_FAILED", error))?;
  let inspection = store
    .inspect_waits_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_WAIT_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML wait inspection: {error}"))
  })
}

#[napi]
pub fn inspect_woml_correction_v1(
  event_store_path: String,
  run_id: String,
) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  if workflow.schema_version != COMPILED_MODEL_SCHEMA_VERSION_V19 {
    return Ok("null".to_string());
  }
  let inspection = store
    .inspect_correction_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_CORRECTION_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction inspection: {error}"
    ))
  })
}

#[napi]
pub fn recover_woml_correction(
  event_store_path: String,
  request_json: String,
) -> napi::Result<String> {
  let request: CorrectionRecoveryRequestV1 = serde_json::from_str(&request_json).map_err(|_| {
    napi::Error::from_reason(
      "WOML_CORRECTION_NOT_RECOVERABLE: invalid Correction Recovery v1 request.".to_string(),
    )
  })?;
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_RECOVERY_FAILED", error))?;
  let result = store
    .recover_correction_v1(request, Utc::now())
    .map_err(|error| native_run_management_error("WOML_CORRECTION_RECOVERY_FAILED", error))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction recovery: {error}"
    ))
  })
}

#[napi]
pub fn observe_woml_corrections_v1(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_CORRECTION_OBSERVATION_FAILED", error))?;
  let observation = store
    .correction_observation_v1()
    .map_err(|error| native_run_management_error("WOML_CORRECTION_OBSERVATION_FAILED", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML correction observation: {error}"
    ))
  })
}

#[napi]
pub fn inspect_woml_locks_v1(event_store_path: String, run_id: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_LOCK_INSPECTION_FAILED", error))?;
  let inspection = store
    .inspect_locks_v1(&run_id)
    .map_err(|error| native_run_management_error("WOML_LOCK_INSPECTION_FAILED", error))?;
  serde_json::to_string(&inspection).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML lock inspection: {error}"))
  })
}

#[napi]
pub fn observe_woml_locks_v1(event_store_path: String) -> napi::Result<String> {
  let store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_LOCK_OBSERVATION_FAILED", error))?;
  let observation = store
    .lock_observation_v1()
    .map_err(|error| native_run_management_error("WOML_LOCK_OBSERVATION_FAILED", error))?;
  serde_json::to_string(&observation).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML lock observation: {error}"))
  })
}

#[napi]
pub fn cancel_woml_run(
  event_store_path: String,
  run_id: String,
  command_id: String,
) -> napi::Result<String> {
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_run_management_error("WOML_RUN_CANCELLATION_FAILED", error))?;
  let result = store
    .request_run_cancellation(&run_id, &command_id, Utc::now())
    .map_err(|error| native_run_management_error("WOML_RUN_CANCELLATION_FAILED", error))?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode WOML cancellation result: {error}"
    ))
  })
}

#[napi]
pub fn recover_woml_runs(event_store_path: String) -> napi::Result<String> {
  let report = recover_durable_runs(PathBuf::from(event_store_path))
    .map_err(|error| napi::Error::from_reason(error.to_string()))?;
  serde_json::to_string(&report)
    .map_err(|error| napi::Error::from_reason(format!("Could not encode recovery report: {error}")))
}

#[napi]
pub fn settle_woml_timer_wait(
  event_store_path: String,
  run_id: String,
  wait_id: String,
) -> napi::Result<String> {
  let mut store = DurableEventStore::open(PathBuf::from(event_store_path))
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?;
  let settlement = match store
    .settle_timer_wait(&run_id, &wait_id, Utc::now())
    .map_err(|error| native_execution_error(RuntimeExecutionError::DurableStore(error)))?
  {
    TimerWaitSettlement::Resolved { .. } => NativeTimerWaitSettlement::Resolved,
    TimerWaitSettlement::AlreadyResolved { .. } => NativeTimerWaitSettlement::AlreadyResolved,
    TimerWaitSettlement::NotDue { deadline } => NativeTimerWaitSettlement::NotDue { deadline },
  };
  serde_json::to_string(&settlement).map_err(|error| {
    napi::Error::from_reason(format!("Could not encode WOML timer settlement: {error}"))
  })
}

#[napi(ts_return_type = "Promise<string>")]
pub async fn run_woml_notification_provider_journey(
  event_store_path: String,
  run_id: String,
  bun_executable: String,
  notification_host_path: String,
  interaction_timeout_ms: u32,
  custom_notification_host_path: Option<String>,
  script_host_path: Option<String>,
  approval_base_url: Option<String>,
  resolved_secrets_json: Option<String>,
) -> napi::Result<String> {
  if interaction_timeout_ms == 0 {
    return Err(napi::Error::from_reason(
      "Notification interaction timeout must be positive.".to_string(),
    ));
  }
  let event_store_path = PathBuf::from(event_store_path);
  let store = DurableEventStore::open(&event_store_path)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let binding = store
    .run_binding(&run_id)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let workflow = store
    .definition(&binding.definition_hash)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let stored_artifacts = store
    .definition_module_artifacts(&binding.definition_hash)
    .map_err(|error| native_notification_error(NotificationJourneyError::Store(error)))?;
  let custom_descriptors: HashMap<&str, &str> = workflow
    .reusable_definitions
    .iter()
    .flatten()
    .filter_map(|definition| match definition {
      CompiledReusableInvocation::NotificationProvider {
        definition_digest,
        script_artifact_id,
        ..
      } => Some((script_artifact_id.as_str(), definition_digest.as_str())),
      _ => None,
    })
    .collect::<HashMap<_, _>>();
  let custom_artifacts = stored_artifacts
    .iter()
    .filter_map(|artifact| {
      let artifact_id = artifact.name.strip_prefix("__woml_provider__")?;
      Some(CustomProviderScriptArtifact {
        script_artifact_id: artifact_id.to_string(),
        definition_digest: custom_descriptors.get(artifact_id)?.to_string(),
        source: artifact.bundle.clone(),
      })
    })
    .collect::<Vec<_>>();
  let has_custom = !custom_descriptors.is_empty();
  let custom = if has_custom {
    let host_script_path = custom_notification_host_path.ok_or_else(|| {
      napi::Error::from_reason("Custom notification provider host path is required.".to_string())
    })?;
    let lifecycle_script_host_path = script_host_path.map(PathBuf::from).unwrap_or_else(|| {
      let host = PathBuf::from(&host_script_path);
      let extension = host
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("js");
      host
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("script-host.{extension}"))
    });
    let approval_base_url = approval_base_url.ok_or_else(|| {
      napi::Error::from_reason("Custom provider approval base URL is required.".to_string())
    })?;
    let resolved_secrets = serde_json::from_str(resolved_secrets_json.as_deref().unwrap_or("{}"))
      .map_err(|_| {
      napi::Error::from_reason("Resolved provider secrets JSON is invalid.".to_string())
    })?;
    Some(CustomNotificationJourneyOptions {
      bun_executable: PathBuf::from(&bun_executable),
      host_script_path: PathBuf::from(host_script_path),
      script_host_path: lifecycle_script_host_path,
      approval_base_url,
      resolved_secrets,
      artifacts: custom_artifacts,
    })
  } else {
    None
  };
  let result = run_notification_provider_journey_with_custom(
    event_store_path,
    &run_id,
    NotificationHostProcessOptions::new(
      PathBuf::from(bun_executable),
      PathBuf::from(notification_host_path),
    ),
    std::time::Duration::from_millis(u64::from(interaction_timeout_ms)),
    custom,
  )
  .await
  .map_err(native_notification_error)?;
  serde_json::to_string(&result).map_err(|error| {
    napi::Error::from_reason(format!(
      "Could not encode notification provider journey: {error}"
    ))
  })
}
