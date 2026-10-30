//! Compile-time compatibility for the root and direct-module contract surfaces.
//! Lists are frozen before RCM1; do not regenerate them to hide a missing export.

#[test]
fn model_diagnostics_keep_their_order_and_first_message() {
  let mut model = woml_engine::CompiledWorkflowDefinition::from_json(include_str!(
    "../../../woml/tests/fixtures/hello.compiled.v1.json"
  ))
  .unwrap();
  model.workflow_id.clear();
  let metadata = model.metadata.as_mut().unwrap();
  metadata.name = Some(String::new());
  metadata.version = Some(String::new());
  let error = model.validate_structure().unwrap_err();
  assert_eq!(
    error
      .issues
      .iter()
      .map(|issue| issue.code)
      .collect::<Vec<_>>(),
    [
      woml_engine::ModelIssueCode::InvalidIdentifier,
      woml_engine::ModelIssueCode::InvalidMetadata,
      woml_engine::ModelIssueCode::InvalidMetadata,
    ]
  );
  assert_eq!(
    error.to_string(),
    "compiled workflow model validation failed: workflowId must contain between 1 and 256 characters."
  );
}

#[test]
fn public_functions_and_protocol_constants_remain_compatible() {
  let _: fn(&str) -> bool = woml_engine::event::is_definition_hash;
  let _: fn(&str) -> bool = woml_engine::event::is_approval_request_id;
  let _: fn(&str, &str) -> bool = woml_engine::event::is_notification_delivery_id;
  assert_eq!(
    woml_engine::protocol::SCRIPT_HOST_PROTOCOL,
    "woml.script-host"
  );
  assert_eq!(woml_engine::protocol::SCRIPT_HOST_PROTOCOL_VERSION, 12);
}

#[test]
fn model_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowDefinition>();
  let _: fn(
    woml_engine::CompiledWorkflowDefinition,
  ) -> woml_engine::model::CompiledWorkflowDefinition = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledCommunicationRequirements>();
  let _ = std::any::type_name::<woml_engine::model::CompiledCommunicationProviderRequirement>();
  let _ = std::any::type_name::<woml_engine::model::CompiledRuntimePolicy>();
  let _: fn(woml_engine::CompiledRuntimePolicy) -> woml_engine::model::CompiledRuntimePolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledRateLimitPolicy>();
  let _: fn(woml_engine::CompiledRateLimitPolicy) -> woml_engine::model::CompiledRateLimitPolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::RateLimitAlgorithm>();
  let _: fn(woml_engine::RateLimitAlgorithm) -> woml_engine::model::RateLimitAlgorithm =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledQueuePolicy>();
  let _: fn(woml_engine::CompiledQueuePolicy) -> woml_engine::model::CompiledQueuePolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::QueueDiscipline>();
  let _: fn(woml_engine::QueueDiscipline) -> woml_engine::model::QueueDiscipline = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledLifecycleDefinition>();
  let _: fn(
    woml_engine::CompiledLifecycleDefinition,
  ) -> woml_engine::model::CompiledLifecycleDefinition = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledLifecycleHook>();
  let _: fn(woml_engine::CompiledLifecycleHook) -> woml_engine::model::CompiledLifecycleHook =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::LifecycleEventName>();
  let _: fn(woml_engine::LifecycleEventName) -> woml_engine::model::LifecycleEventName =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledLifecycleAction>();
  let _: fn(woml_engine::CompiledLifecycleAction) -> woml_engine::model::CompiledLifecycleAction =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledModuleRuntime>();
  let _: fn(woml_engine::CompiledModuleRuntime) -> woml_engine::model::CompiledModuleRuntime =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledModuleBinding>();
  let _: fn(woml_engine::CompiledModuleBinding) -> woml_engine::model::CompiledModuleBinding =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowMetadata>();
  let _ = std::any::type_name::<woml_engine::model::CompiledTrigger>();
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowGraph>();
  let _ = std::any::type_name::<woml_engine::model::CompiledCorrectionIdentity>();
  let _: fn(
    woml_engine::CompiledCorrectionIdentity,
  ) -> woml_engine::model::CompiledCorrectionIdentity = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledCorrectiveAction>();
  let _: fn(woml_engine::CompiledCorrectiveAction) -> woml_engine::model::CompiledCorrectiveAction =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledSaga>();
  let _: fn(woml_engine::CompiledSaga) -> woml_engine::model::CompiledSaga = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledLock>();
  let _: fn(woml_engine::CompiledLock) -> woml_engine::model::CompiledLock = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledWait>();
  let _: fn(woml_engine::CompiledWait) -> woml_engine::model::CompiledWait = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledWaitSignal>();
  let _: fn(woml_engine::CompiledWaitSignal) -> woml_engine::model::CompiledWaitSignal =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledForEachBody>();
  let _ = std::any::type_name::<woml_engine::model::CompiledForEach>();
  let _ = std::any::type_name::<woml_engine::model::CompiledForkBranch>();
  let _ = std::any::type_name::<woml_engine::model::CompiledFork>();
  let _ = std::any::type_name::<woml_engine::model::CompiledControlChoice>();
  let _ = std::any::type_name::<woml_engine::model::CompiledStringChoiceCase>();
  let _ = std::any::type_name::<woml_engine::model::CompiledContextVisibility>();
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowSettlement>();
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowNode>();
  let _: fn(woml_engine::CompiledWorkflowNode) -> woml_engine::model::CompiledWorkflowNode =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledExecutionPlacement>();
  let _ = std::any::type_name::<woml_engine::model::ScriptRuntimeBindings>();
  let _: fn(woml_engine::ScriptRuntimeBindings) -> woml_engine::model::ScriptRuntimeBindings =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledReusableInvocation>();
  let _: fn(
    woml_engine::CompiledReusableInvocation,
  ) -> woml_engine::model::CompiledReusableInvocation = |value| value;
  let _ = std::any::type_name::<woml_engine::model::CompiledReusableBoundProp>();
  let _ = std::any::type_name::<woml_engine::model::CompiledReusablePropExpression>();
  let _ = std::any::type_name::<woml_engine::model::CompiledReusableLifecycle>();
  let _ = std::any::type_name::<woml_engine::model::CompiledWorkflowEdge>();
  let _ = std::any::type_name::<woml_engine::model::ValueExpression>();
  let _: fn(woml_engine::ValueExpression) -> woml_engine::model::ValueExpression = |value| value;
  let _ = std::any::type_name::<woml_engine::model::TemplatePart>();
  let _ = std::any::type_name::<woml_engine::model::EdgeCondition>();
  let _ = std::any::type_name::<woml_engine::model::RetryPolicy>();
  let _ = std::any::type_name::<woml_engine::model::NotificationDefinition>();
  let _: fn(woml_engine::NotificationDefinition) -> woml_engine::model::NotificationDefinition =
    |value| value;
  let _ = std::any::type_name::<woml_engine::model::BackoffPolicy>();
  let _ = std::any::type_name::<woml_engine::model::ModelIssueCode>();
  let _: fn(woml_engine::ModelIssueCode) -> woml_engine::model::ModelIssueCode = |value| value;
  let _ = std::any::type_name::<woml_engine::model::ModelIssue>();
  let _: fn(woml_engine::ModelIssue) -> woml_engine::model::ModelIssue = |value| value;
  let _ = std::any::type_name::<woml_engine::model::ModelValidationError>();
  let _: fn(woml_engine::ModelValidationError) -> woml_engine::model::ModelValidationError =
    |value| value;
}

#[test]
fn event_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::event::RunEvent>();
  let _: fn(woml_engine::RunEvent) -> woml_engine::event::RunEvent = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunEventPayload>();
  let _: fn(woml_engine::RunEventPayload) -> woml_engine::event::RunEventPayload = |value| value;
  let _ = std::any::type_name::<woml_engine::event::InfrastructurePolicyDecisionRecordedData>();
  let _: fn(
    woml_engine::InfrastructurePolicyDecisionRecordedData,
  ) -> woml_engine::event::InfrastructurePolicyDecisionRecordedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RemoteDispatchTransitionedData>();
  let _: fn(
    woml_engine::RemoteDispatchTransitionedData,
  ) -> woml_engine::event::RemoteDispatchTransitionedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::DurableWaitKind>();
  let _: fn(woml_engine::DurableWaitKind) -> woml_engine::event::DurableWaitKind = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CorrectiveActionKind>();
  let _: fn(woml_engine::CorrectiveActionKind) -> woml_engine::event::CorrectiveActionKind =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::SagaCorrectionReason>();
  let _: fn(woml_engine::SagaCorrectionReason) -> woml_engine::event::SagaCorrectionReason =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::SagaIdentityData>();
  let _: fn(woml_engine::SagaIdentityData) -> woml_engine::event::SagaIdentityData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CompensationRegisteredData>();
  let _: fn(
    woml_engine::CompensationRegisteredData,
  ) -> woml_engine::event::CompensationRegisteredData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::SagaCorrectionStartedData>();
  let _: fn(
    woml_engine::SagaCorrectionStartedData,
  ) -> woml_engine::event::SagaCorrectionStartedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CorrectionAttemptIdentityData>();
  let _: fn(
    woml_engine::CorrectionAttemptIdentityData,
  ) -> woml_engine::event::CorrectionAttemptIdentityData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CorrectionAttemptFailedData>();
  let _: fn(
    woml_engine::CorrectionAttemptFailedData,
  ) -> woml_engine::event::CorrectionAttemptFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CorrectionRetryScheduledData>();
  let _: fn(
    woml_engine::CorrectionRetryScheduledData,
  ) -> woml_engine::event::CorrectionRetryScheduledData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CorrectionInterruptedData>();
  let _: fn(
    woml_engine::CorrectionInterruptedData,
  ) -> woml_engine::event::CorrectionInterruptedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ManualRecoveryRequiredData>();
  let _: fn(
    woml_engine::ManualRecoveryRequiredData,
  ) -> woml_engine::event::ManualRecoveryRequiredData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::WaitRegisteredData>();
  let _: fn(woml_engine::WaitRegisteredData) -> woml_engine::event::WaitRegisteredData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ExternalSignalReceivedData>();
  let _: fn(
    woml_engine::ExternalSignalReceivedData,
  ) -> woml_engine::event::ExternalSignalReceivedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::DurableWaitWinner>();
  let _: fn(woml_engine::DurableWaitWinner) -> woml_engine::event::DurableWaitWinner =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::WaitSatisfiedData>();
  let _: fn(woml_engine::WaitSatisfiedData) -> woml_engine::event::WaitSatisfiedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::DurableWaitTimeoutBehavior>();
  let _: fn(
    woml_engine::DurableWaitTimeoutBehavior,
  ) -> woml_engine::event::DurableWaitTimeoutBehavior = |value| value;
  let _ = std::any::type_name::<woml_engine::event::WaitTimedOutData>();
  let _: fn(woml_engine::WaitTimedOutData) -> woml_engine::event::WaitTimedOutData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::WaitCancelledData>();
  let _: fn(woml_engine::WaitCancelledData) -> woml_engine::event::WaitCancelledData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockWaitRegisteredData>();
  let _: fn(woml_engine::LockWaitRegisteredData) -> woml_engine::event::LockWaitRegisteredData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockAcquiredData>();
  let _: fn(woml_engine::LockAcquiredData) -> woml_engine::event::LockAcquiredData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockReleasedData>();
  let _: fn(woml_engine::LockReleasedData) -> woml_engine::event::LockReleasedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockAcquireTimedOutData>();
  let _: fn(woml_engine::LockAcquireTimedOutData) -> woml_engine::event::LockAcquireTimedOutData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockAcquisitionCancelledData>();
  let _: fn(
    woml_engine::LockAcquisitionCancelledData,
  ) -> woml_engine::event::LockAcquisitionCancelledData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LockLeaseLostData>();
  let _: fn(woml_engine::LockLeaseLostData) -> woml_engine::event::LockLeaseLostData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunAdmissionTrigger>();
  let _: fn(woml_engine::RunAdmissionTrigger) -> woml_engine::event::RunAdmissionTrigger =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunAdmissionQueue>();
  let _: fn(woml_engine::RunAdmissionQueue) -> woml_engine::event::RunAdmissionQueue =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunAdmittedData>();
  let _: fn(woml_engine::RunAdmittedData) -> woml_engine::event::RunAdmittedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunExecutionStartedData>();
  let _: fn(woml_engine::RunExecutionStartedData) -> woml_engine::event::RunExecutionStartedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunTimeoutReachedData>();
  let _: fn(woml_engine::RunTimeoutReachedData) -> woml_engine::event::RunTimeoutReachedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunCancellationRequestedData>();
  let _: fn(
    woml_engine::RunCancellationRequestedData,
  ) -> woml_engine::event::RunCancellationRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleSubjectKind>();
  let _: fn(woml_engine::LifecycleSubjectKind) -> woml_engine::event::LifecycleSubjectKind =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleSubject>();
  let _: fn(woml_engine::LifecycleSubject) -> woml_engine::event::LifecycleSubject = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleHookRequestedData>();
  let _: fn(
    woml_engine::LifecycleHookRequestedData,
  ) -> woml_engine::event::LifecycleHookRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleActionIdentityData>();
  let _: fn(
    woml_engine::LifecycleActionIdentityData,
  ) -> woml_engine::event::LifecycleActionIdentityData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleFailureKind>();
  let _: fn(woml_engine::LifecycleFailureKind) -> woml_engine::event::LifecycleFailureKind =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleFailure>();
  let _: fn(woml_engine::LifecycleFailure) -> woml_engine::event::LifecycleFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleActionFailedData>();
  let _: fn(
    woml_engine::LifecycleActionFailedData,
  ) -> woml_engine::event::LifecycleActionFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleHookCompletionStatus>();
  let _: fn(
    woml_engine::LifecycleHookCompletionStatus,
  ) -> woml_engine::event::LifecycleHookCompletionStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleHookCompletedData>();
  let _: fn(
    woml_engine::LifecycleHookCompletedData,
  ) -> woml_engine::event::LifecycleHookCompletedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleHook>();
  let _: fn(woml_engine::ReusableLifecycleHook) -> woml_engine::event::ReusableLifecycleHook =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleOutcome>();
  let _: fn(woml_engine::ReusableLifecycleOutcome) -> woml_engine::event::ReusableLifecycleOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleRequestedData>();
  let _: fn(
    woml_engine::ReusableLifecycleRequestedData,
  ) -> woml_engine::event::ReusableLifecycleRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleActionStartedData>();
  let _: fn(
    woml_engine::ReusableLifecycleActionStartedData,
  ) -> woml_engine::event::ReusableLifecycleActionStartedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleActionSucceededData>();
  let _: fn(
    woml_engine::ReusableLifecycleActionSucceededData,
  ) -> woml_engine::event::ReusableLifecycleActionSucceededData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleActionFailedData>();
  let _: fn(
    woml_engine::ReusableLifecycleActionFailedData,
  ) -> woml_engine::event::ReusableLifecycleActionFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ReusableLifecycleCompletedData>();
  let _: fn(
    woml_engine::ReusableLifecycleCompletedData,
  ) -> woml_engine::event::ReusableLifecycleCompletedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::BusinessOutcome>();
  let _: fn(woml_engine::BusinessOutcome) -> woml_engine::event::BusinessOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunOutcomeDecidedData>();
  let _: fn(woml_engine::RunOutcomeDecidedData) -> woml_engine::event::RunOutcomeDecidedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::FinalLifecycleStatus>();
  let _: fn(woml_engine::FinalLifecycleStatus) -> woml_engine::event::FinalLifecycleStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::LifecycleWarning>();
  let _: fn(woml_engine::LifecycleWarning) -> woml_engine::event::LifecycleWarning = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFinalizedData>();
  let _: fn(woml_engine::RunFinalizedData) -> woml_engine::event::RunFinalizedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::OperationExecutionMode>();
  let _: fn(woml_engine::OperationExecutionMode) -> woml_engine::event::OperationExecutionMode =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::OperationStartedData>();
  let _: fn(woml_engine::OperationStartedData) -> woml_engine::event::OperationStartedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::OperationSucceededData>();
  let _: fn(woml_engine::OperationSucceededData) -> woml_engine::event::OperationSucceededData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::OperationFailedData>();
  let _: fn(woml_engine::OperationFailedData) -> woml_engine::event::OperationFailedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunStartedData>();
  let _: fn(woml_engine::RunStartedData) -> woml_engine::event::RunStartedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunIngress>();
  let _: fn(woml_engine::RunIngress) -> woml_engine::event::RunIngress = |value| value;
  let _ = std::any::type_name::<woml_engine::event::StepAttemptStartedData>();
  let _: fn(woml_engine::StepAttemptStartedData) -> woml_engine::event::StepAttemptStartedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::StepAttemptSucceededData>();
  let _: fn(woml_engine::StepAttemptSucceededData) -> woml_engine::event::StepAttemptSucceededData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::StepAttemptFailedData>();
  let _: fn(woml_engine::StepAttemptFailedData) -> woml_engine::event::StepAttemptFailedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::StepRetryScheduledData>();
  let _: fn(woml_engine::StepRetryScheduledData) -> woml_engine::event::StepRetryScheduledData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::BranchSelectedData>();
  let _: fn(woml_engine::BranchSelectedData) -> woml_engine::event::BranchSelectedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ChoiceSelectedData>();
  let _: fn(woml_engine::ChoiceSelectedData) -> woml_engine::event::ChoiceSelectedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForkOpenedData>();
  let _: fn(woml_engine::ForkOpenedData) -> woml_engine::event::ForkOpenedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForkBranchOutcome>();
  let _: fn(woml_engine::ForkBranchOutcome) -> woml_engine::event::ForkBranchOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForkBranchSettledData>();
  let _: fn(woml_engine::ForkBranchSettledData) -> woml_engine::event::ForkBranchSettledData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForkJoinOutcome>();
  let _: fn(woml_engine::ForkJoinOutcome) -> woml_engine::event::ForkJoinOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForkJoinSettledData>();
  let _: fn(woml_engine::ForkJoinSettledData) -> woml_engine::event::ForkJoinSettledData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationScope>();
  let _ = std::any::type_name::<woml_engine::event::ForEachOpenedData>();
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationIdentityData>();
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationSucceededData>();
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationFailureKind>();
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationFailedData>();
  let _ = std::any::type_name::<woml_engine::event::ForEachSkipReason>();
  let _ = std::any::type_name::<woml_engine::event::ForEachIterationSkippedData>();
  let _ = std::any::type_name::<woml_engine::event::ForEachSettledData>();
  let _ = std::any::type_name::<woml_engine::event::ParallelGroupStartedData>();
  let _: fn(woml_engine::ParallelGroupStartedData) -> woml_engine::event::ParallelGroupStartedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ParallelGroupOutcome>();
  let _: fn(woml_engine::ParallelGroupOutcome) -> woml_engine::event::ParallelGroupOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ParallelGroupCompletedData>();
  let _: fn(
    woml_engine::ParallelGroupCompletedData,
  ) -> woml_engine::event::ParallelGroupCompletedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalTimeoutPolicy>();
  let _: fn(woml_engine::ApprovalTimeoutPolicy) -> woml_engine::event::ApprovalTimeoutPolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalRequestedData>();
  let _: fn(woml_engine::ApprovalRequestedData) -> woml_engine::event::ApprovalRequestedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalDecision>();
  let _: fn(woml_engine::ApprovalDecision) -> woml_engine::event::ApprovalDecision = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalDecisionSource>();
  let _: fn(woml_engine::ApprovalDecisionSource) -> woml_engine::event::ApprovalDecisionSource =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalResolution>();
  let _: fn(woml_engine::ApprovalResolution) -> woml_engine::event::ApprovalResolution =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalResolvedData>();
  let _: fn(woml_engine::ApprovalResolvedData) -> woml_engine::event::ApprovalResolvedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RichApprovalResolutionReason>();
  let _: fn(
    woml_engine::RichApprovalResolutionReason,
  ) -> woml_engine::event::RichApprovalResolutionReason = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalReviewRequestedData>();
  let _: fn(
    woml_engine::ApprovalReviewRequestedData,
  ) -> woml_engine::event::ApprovalReviewRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalReviewerDecisionRecordedData>();
  let _: fn(
    woml_engine::ApprovalReviewerDecisionRecordedData,
  ) -> woml_engine::event::ApprovalReviewerDecisionRecordedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalQuorumProgressedData>();
  let _: fn(
    woml_engine::ApprovalQuorumProgressedData,
  ) -> woml_engine::event::ApprovalQuorumProgressedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalInvalidatedData>();
  let _: fn(woml_engine::ApprovalInvalidatedData) -> woml_engine::event::ApprovalInvalidatedData =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationDeliveryRequestedData>();
  let _: fn(
    woml_engine::NotificationDeliveryRequestedData,
  ) -> woml_engine::event::NotificationDeliveryRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationDeliveryAttemptStartedData>();
  let _: fn(
    woml_engine::NotificationDeliveryAttemptStartedData,
  ) -> woml_engine::event::NotificationDeliveryAttemptStartedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ProviderMessageIdentity>();
  let _: fn(woml_engine::ProviderMessageIdentity) -> woml_engine::event::ProviderMessageIdentity =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::SlackProviderMessageIdentity>();
  let _: fn(
    woml_engine::SlackProviderMessageIdentity,
  ) -> woml_engine::event::SlackProviderMessageIdentity = |value| value;
  let _ = std::any::type_name::<woml_engine::event::CommunicationProviderMessageIdentity>();
  let _: fn(
    woml_engine::CommunicationProviderMessageIdentity,
  ) -> woml_engine::event::CommunicationProviderMessageIdentity = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationDeliverySucceededData>();
  let _: fn(
    woml_engine::NotificationDeliverySucceededData,
  ) -> woml_engine::event::NotificationDeliverySucceededData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationSafeFailure>();
  let _: fn(woml_engine::NotificationSafeFailure) -> woml_engine::event::NotificationSafeFailure =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationDeliveryFailedData>();
  let _: fn(
    woml_engine::NotificationDeliveryFailedData,
  ) -> woml_engine::event::NotificationDeliveryFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationDecisionAcceptedData>();
  let _: fn(
    woml_engine::NotificationDecisionAcceptedData,
  ) -> woml_engine::event::NotificationDecisionAcceptedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationResolution>();
  let _: fn(woml_engine::NotificationResolution) -> woml_engine::event::NotificationResolution =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationMessageUpdateRequestedData>();
  let _: fn(
    woml_engine::NotificationMessageUpdateRequestedData,
  ) -> woml_engine::event::NotificationMessageUpdateRequestedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationMessageUpdateAttemptStartedData>();
  let _: fn(
    woml_engine::NotificationMessageUpdateAttemptStartedData,
  ) -> woml_engine::event::NotificationMessageUpdateAttemptStartedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationMessageUpdatedData>();
  let _: fn(
    woml_engine::NotificationMessageUpdatedData,
  ) -> woml_engine::event::NotificationMessageUpdatedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationMessageUpdateFailedData>();
  let _: fn(
    woml_engine::NotificationMessageUpdateFailedData,
  ) -> woml_engine::event::NotificationMessageUpdateFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunSucceededData>();
  let _: fn(woml_engine::RunSucceededData) -> woml_engine::event::RunSucceededData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedDataV1>();
  let _: fn(woml_engine::RunFailedDataV1) -> woml_engine::event::RunFailedDataV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedDataV2>();
  let _: fn(woml_engine::RunFailedDataV2) -> woml_engine::event::RunFailedDataV2 = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ParallelFailurePolicy>();
  let _: fn(woml_engine::ParallelFailurePolicy) -> woml_engine::event::ParallelFailurePolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::ParallelFailure>();
  let _: fn(woml_engine::ParallelFailure) -> woml_engine::event::ParallelFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedDataV3>();
  let _: fn(woml_engine::RunFailedDataV3) -> woml_engine::event::RunFailedDataV3 = |value| value;
  let _ = std::any::type_name::<woml_engine::event::ApprovalFailure>();
  let _: fn(woml_engine::ApprovalFailure) -> woml_engine::event::ApprovalFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedDataV4>();
  let _: fn(woml_engine::RunFailedDataV4) -> woml_engine::event::RunFailedDataV4 = |value| value;
  let _ = std::any::type_name::<woml_engine::event::NotificationRunFailure>();
  let _: fn(woml_engine::NotificationRunFailure) -> woml_engine::event::NotificationRunFailure =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedDataV5>();
  let _: fn(woml_engine::RunFailedDataV5) -> woml_engine::event::RunFailedDataV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::event::RunFailedData>();
  let _: fn(woml_engine::RunFailedData) -> woml_engine::event::RunFailedData = |value| value;
  let _ = std::any::type_name::<woml_engine::event::AttemptFailureKind>();
  let _: fn(woml_engine::AttemptFailureKind) -> woml_engine::event::AttemptFailureKind =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::FailureSizeDetails>();
  let _: fn(woml_engine::FailureSizeDetails) -> woml_engine::event::FailureSizeDetails =
    |value| value;
  let _ = std::any::type_name::<woml_engine::event::AttemptFailure>();
  let _: fn(woml_engine::AttemptFailure) -> woml_engine::event::AttemptFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::event::JsonValueType>();
  let _: fn(woml_engine::JsonValueType) -> woml_engine::event::JsonValueType = |value| value;
  let _ = std::any::type_name::<woml_engine::event::BranchFailure>();
  let _: fn(woml_engine::BranchFailure) -> woml_engine::event::BranchFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::event::EventValidationError>();
}

#[test]
fn protocol_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::protocol::ScriptLogLevelV1>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptLogMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::ProgressReportMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::ProgressAckMessage<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ReadyMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::CancelMessage<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ExecuteMessage<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptExecutionContext<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::CorrectionReasonV1>();
  let _ = std::any::type_name::<woml_engine::protocol::CompensationBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::CleanupReasonV1>();
  let _ = std::any::type_name::<woml_engine::protocol::CompensationStatusV1>();
  let _ = std::any::type_name::<woml_engine::protocol::CleanupBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::CorrectionBindingRef<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptIterationBinding>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptExecutionMode>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableScriptBindingV3>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableDefinitionBindingV3>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableDefinitionKind>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableLifecycleBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableLifecycleHook>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableInvocationOutcome>();
  let _ = std::any::type_name::<woml_engine::protocol::ReusableLifecycleErrorV1>();
  let _ = std::any::type_name::<woml_engine::protocol::LifecycleBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::LifecycleWorkflowBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::LifecycleStepBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::LifecycleFailureBindingV1>();
  let _ = std::any::type_name::<woml_engine::protocol::RuntimeModuleBinding>();
  let _ = std::any::type_name::<woml_engine::protocol::RegisterModuleMessage<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ModuleRegisteredMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptBindings<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::ScriptAttempt<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::CompletedMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::HostOutcome>();
  let _ = std::any::type_name::<woml_engine::protocol::HostReportedFailureKind>();
  let _ = std::any::type_name::<woml_engine::protocol::HostReportedFailure>();
  let _ = std::any::type_name::<woml_engine::protocol::CapabilityCallMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::CapabilityResultMessage<'static>>();
  let _ = std::any::type_name::<woml_engine::protocol::FetchObservationMessage>();
  let _ = std::any::type_name::<woml_engine::protocol::FetchObservationAckMessage<'static>>();
}
