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

// RCM2/RCM3 inventories frozen from commit 2c485ab before extraction.

#[test]
fn engine_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::engine::EngineError>();
  let _: fn(woml_engine::EngineError) -> woml_engine::engine::EngineError = |value| value;
  let _ = std::any::type_name::<woml_engine::engine::InMemoryDagEngine>();
  let _: fn(woml_engine::InMemoryDagEngine) -> woml_engine::engine::InMemoryDagEngine =
    |value| value;
  let _ = woml_engine::engine::step_effect_idempotency_key;
}

#[test]
fn projection_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::projection::RunStatus>();
  let _: fn(woml_engine::RunStatus) -> woml_engine::projection::RunStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LifecycleStatus>();
  let _: fn(woml_engine::LifecycleStatus) -> woml_engine::projection::LifecycleStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LifecycleActionStatus>();
  let _: fn(woml_engine::LifecycleActionStatus) -> woml_engine::projection::LifecycleActionStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LifecycleActionProjection>();
  let _: fn(
    woml_engine::LifecycleActionProjection,
  ) -> woml_engine::projection::LifecycleActionProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LifecycleHookStatus>();
  let _: fn(woml_engine::LifecycleHookStatus) -> woml_engine::projection::LifecycleHookStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LifecycleHookProjection>();
  let _: fn(
    woml_engine::LifecycleHookProjection,
  ) -> woml_engine::projection::LifecycleHookProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ReusableLifecycleStatus>();
  let _: fn(
    woml_engine::ReusableLifecycleStatus,
  ) -> woml_engine::projection::ReusableLifecycleStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ReusableLifecycleProjection>();
  let _: fn(
    woml_engine::ReusableLifecycleProjection,
  ) -> woml_engine::projection::ReusableLifecycleProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::WorkflowContext>();
  let _: fn(woml_engine::WorkflowContext) -> woml_engine::projection::WorkflowContext =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LockContext>();
  let _: fn(woml_engine::LockContext) -> woml_engine::projection::LockContext = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::AttemptIdentity>();
  let _ = std::any::type_name::<woml_engine::projection::AttemptStatus>();
  let _ = std::any::type_name::<woml_engine::projection::AttemptProjection>();
  let _ = std::any::type_name::<woml_engine::projection::OperationIdentity>();
  let _: fn(woml_engine::OperationIdentity) -> woml_engine::projection::OperationIdentity =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::OperationStatus>();
  let _: fn(woml_engine::OperationStatus) -> woml_engine::projection::OperationStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::OperationProjection>();
  let _: fn(woml_engine::OperationProjection) -> woml_engine::projection::OperationProjection =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RetryScheduleProjection>();
  let _: fn(
    woml_engine::RetryScheduleProjection,
  ) -> woml_engine::projection::RetryScheduleProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ParallelGroupStatus>();
  let _: fn(woml_engine::ParallelGroupStatus) -> woml_engine::projection::ParallelGroupStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ParallelGroupProjection>();
  let _: fn(
    woml_engine::ParallelGroupProjection,
  ) -> woml_engine::projection::ParallelGroupProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ForkJoinStatus>();
  let _: fn(woml_engine::ForkJoinStatus) -> woml_engine::projection::ForkJoinStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ForkBranchProjection>();
  let _: fn(woml_engine::ForkBranchProjection) -> woml_engine::projection::ForkBranchProjection =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ForkProjection>();
  let _: fn(woml_engine::ForkProjection) -> woml_engine::projection::ForkProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ForEachStatus>();
  let _ = std::any::type_name::<woml_engine::projection::ForEachIterationStatus>();
  let _ = std::any::type_name::<woml_engine::projection::ForEachProjection>();
  let _ = std::any::type_name::<woml_engine::projection::ForEachIterationExecutionProjection>();
  let _ = std::any::type_name::<woml_engine::projection::ApprovalRequestStatus>();
  let _: fn(woml_engine::ApprovalRequestStatus) -> woml_engine::projection::ApprovalRequestStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::ApprovalRequestProjection>();
  let _: fn(
    woml_engine::ApprovalRequestProjection,
  ) -> woml_engine::projection::ApprovalRequestProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RichApprovalRequestStatus>();
  let _: fn(
    woml_engine::RichApprovalRequestStatus,
  ) -> woml_engine::projection::RichApprovalRequestStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RichApprovalReviewerDecisionProjection>();
  let _: fn(
    woml_engine::RichApprovalReviewerDecisionProjection,
  ) -> woml_engine::projection::RichApprovalReviewerDecisionProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RichApprovalRequestProjection>();
  let _: fn(
    woml_engine::RichApprovalRequestProjection,
  ) -> woml_engine::projection::RichApprovalRequestProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::DurableWaitStatus>();
  let _: fn(woml_engine::DurableWaitStatus) -> woml_engine::projection::DurableWaitStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::DurableWaitProjection>();
  let _: fn(woml_engine::DurableWaitProjection) -> woml_engine::projection::DurableWaitProjection =
    |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LockStatus>();
  let _: fn(woml_engine::LockStatus) -> woml_engine::projection::LockStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::LockProjection>();
  let _: fn(woml_engine::LockProjection) -> woml_engine::projection::LockProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::SagaPhase>();
  let _: fn(woml_engine::SagaPhase) -> woml_engine::projection::SagaPhase = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::CorrectionActionStatus>();
  let _: fn(
    woml_engine::CorrectionActionStatus,
  ) -> woml_engine::projection::CorrectionActionStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::CorrectionActionProjection>();
  let _: fn(
    woml_engine::CorrectionActionProjection,
  ) -> woml_engine::projection::CorrectionActionProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::SagaProjection>();
  let _: fn(woml_engine::SagaProjection) -> woml_engine::projection::SagaProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::NotificationDeliveryStatus>();
  let _: fn(
    woml_engine::NotificationDeliveryStatus,
  ) -> woml_engine::projection::NotificationDeliveryStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::NotificationDeliveryProjection>();
  let _: fn(
    woml_engine::NotificationDeliveryProjection,
  ) -> woml_engine::projection::NotificationDeliveryProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::NotificationMessageUpdateStatus>();
  let _: fn(
    woml_engine::NotificationMessageUpdateStatus,
  ) -> woml_engine::projection::NotificationMessageUpdateStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::NotificationMessageUpdateProjection>();
  let _: fn(
    woml_engine::NotificationMessageUpdateProjection,
  ) -> woml_engine::projection::NotificationMessageUpdateProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RunFailure>();
  let _: fn(woml_engine::RunFailure) -> woml_engine::projection::RunFailure = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::RunProjection>();
  let _: fn(woml_engine::RunProjection) -> woml_engine::projection::RunProjection = |value| value;
  let _ = std::any::type_name::<woml_engine::projection::FoldError>();
  let _: fn(woml_engine::FoldError) -> woml_engine::projection::FoldError = |value| value;
  let _ = woml_engine::projection::fold_events;
}

#[test]
fn runtime_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::runtime::ExecutionProgress>();
  let _: fn(woml_engine::ExecutionProgress) -> woml_engine::runtime::ExecutionProgress =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::ExecutionProgressReporter>();
  let _: fn(
    woml_engine::ExecutionProgressReporter,
  ) -> woml_engine::runtime::ExecutionProgressReporter = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::LifecycleProgressPhase>();
  let _: fn(woml_engine::LifecycleProgressPhase) -> woml_engine::runtime::LifecycleProgressPhase =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::LifecycleProgress>();
  let _: fn(woml_engine::LifecycleProgress) -> woml_engine::runtime::LifecycleProgress =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::LifecycleProgressReporter>();
  let _: fn(
    woml_engine::LifecycleProgressReporter,
  ) -> woml_engine::runtime::LifecycleProgressReporter = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimePolicyProgressPhase>();
  let _: fn(
    woml_engine::RuntimePolicyProgressPhase,
  ) -> woml_engine::runtime::RuntimePolicyProgressPhase = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimePolicyProgress>();
  let _: fn(woml_engine::RuntimePolicyProgress) -> woml_engine::runtime::RuntimePolicyProgress =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimePolicyProgressReporter>();
  let _: fn(
    woml_engine::RuntimePolicyProgressReporter,
  ) -> woml_engine::runtime::RuntimePolicyProgressReporter = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimeModuleArtifact>();
  let _: fn(woml_engine::RuntimeModuleArtifact) -> woml_engine::runtime::RuntimeModuleArtifact =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::SystemEngineClock>();
  let _: fn(woml_engine::SystemEngineClock) -> woml_engine::runtime::SystemEngineClock =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::FixedEngineClock>();
  let _: fn(woml_engine::FixedEngineClock) -> woml_engine::runtime::FixedEngineClock =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimeExecutionOptions>();
  let _: fn(woml_engine::RuntimeExecutionOptions) -> woml_engine::runtime::RuntimeExecutionOptions =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WorkflowExecutionResult>();
  let _: fn(woml_engine::WorkflowExecutionResult) -> woml_engine::runtime::WorkflowExecutionResult =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WorkflowRuntimeOutcome>();
  let _: fn(woml_engine::WorkflowRuntimeOutcome) -> woml_engine::runtime::WorkflowRuntimeOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WaitingWorkflowApproval>();
  let _: fn(woml_engine::WaitingWorkflowApproval) -> woml_engine::runtime::WaitingWorkflowApproval =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WaitingRichApprovalAuthority>();
  let _: fn(
    woml_engine::WaitingRichApprovalAuthority,
  ) -> woml_engine::runtime::WaitingRichApprovalAuthority = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WaitingWorkflowTimer>();
  let _: fn(woml_engine::WaitingWorkflowTimer) -> woml_engine::runtime::WaitingWorkflowTimer =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::WaitingWorkflowSignal>();
  let _: fn(woml_engine::WaitingWorkflowSignal) -> woml_engine::runtime::WaitingWorkflowSignal =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::RuntimeExecutionError>();
  let _: fn(woml_engine::RuntimeExecutionError) -> woml_engine::runtime::RuntimeExecutionError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::CancelledRunDetails>();
  let _: fn(woml_engine::CancelledRunDetails) -> woml_engine::runtime::CancelledRunDetails =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::FailedRunDetails>();
  let _ = std::any::type_name::<woml_engine::runtime::FailedBranchDetails>();
  let _ = std::any::type_name::<woml_engine::runtime::FailedParallelDetails>();
  let _: fn(woml_engine::FailedParallelDetails) -> woml_engine::runtime::FailedParallelDetails =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::FailedApprovalDetails>();
  let _: fn(woml_engine::FailedApprovalDetails) -> woml_engine::runtime::FailedApprovalDetails =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::FailedNotificationDetails>();
  let _: fn(
    woml_engine::FailedNotificationDetails,
  ) -> woml_engine::runtime::FailedNotificationDetails = |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::BranchFailureSite>();
  let _: fn(woml_engine::BranchFailureSite) -> woml_engine::runtime::BranchFailureSite =
    |value| value;
  let _ = std::any::type_name::<woml_engine::runtime::ExternalSignalRuntimeOutcome>();
  let _: fn(
    woml_engine::ExternalSignalRuntimeOutcome,
  ) -> woml_engine::runtime::ExternalSignalRuntimeOutcome = |value| value;
  let _ = std::any::type_name::<dyn woml_engine::runtime::EngineClock>();
  let _ = woml_engine::runtime::execute_workflow;
  let _ = woml_engine::runtime::execute_workflow_durable;
  let _ = woml_engine::runtime::execute_workflow_durable_outcome;
  let _ = woml_engine::runtime::resume_workflow_durable;
  let _ = woml_engine::runtime::resume_workflow_durable_outcome;
  let _ = woml_engine::runtime::resume_workflow_durable_any_outcome;
  let _ = woml_engine::runtime::execute_admitted_trigger_run_durable;
  let _ = woml_engine::runtime::execute_reusable_provider_lifecycle_durable;
  let _ = woml_engine::runtime::recover_durable_runs;
  let _ = woml_engine::runtime::resume_due_timer_waits_durable;
  let _ = woml_engine::runtime::admit_external_signal_durable;
  let _ = woml_engine::runtime::resolve_human_approval_durable;
  let _ = woml_engine::runtime::resolve_rich_approval_durable;
  let _ = woml_engine::runtime::settle_approval_timeout_durable;
  let _ = woml_engine::runtime::EXECUTION_PROGRESS_CONTRACT;
  let _ = woml_engine::runtime::EXECUTION_PROGRESS_VERSION;
  let _ = woml_engine::runtime::LIFECYCLE_PROGRESS_PROFILE;
  let _ = woml_engine::runtime::RUNTIME_POLICY_PROGRESS_PROFILE;
  let _ = woml_engine::runtime::RUNTIME_OUTCOME_CONTRACT;
  let _ = woml_engine::runtime::RUNTIME_OUTCOME_VERSION;
}

// RCM6/RCM7 paths frozen from commit 35ebd56 before durable extraction.
#[test]
fn durable_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::durable::RuntimeOwnerLease>();
  let _: fn(woml_engine::RuntimeOwnerLease) -> woml_engine::durable::RuntimeOwnerLease =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunDefinitionBinding>();
  let _: fn(woml_engine::RunDefinitionBinding) -> woml_engine::durable::RunDefinitionBinding =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::TriggerAdmissionRequest>();
  let _: fn(woml_engine::TriggerAdmissionRequest) -> woml_engine::durable::TriggerAdmissionRequest =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::TriggerOccurrence>();
  let _: fn(woml_engine::TriggerOccurrence) -> woml_engine::durable::TriggerOccurrence =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::TriggerAdmissionOutcome>();
  let _: fn(woml_engine::TriggerAdmissionOutcome) -> woml_engine::durable::TriggerAdmissionOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ScheduleCursorRegistration>();
  let _: fn(
    woml_engine::ScheduleCursorRegistration,
  ) -> woml_engine::durable::ScheduleCursorRegistration = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ScheduleCursor>();
  let _: fn(woml_engine::ScheduleCursor) -> woml_engine::durable::ScheduleCursor = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ScheduleCursorRegistrationOutcome>();
  let _: fn(
    woml_engine::ScheduleCursorRegistrationOutcome,
  ) -> woml_engine::durable::ScheduleCursorRegistrationOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::IntervalCursorRegistration>();
  let _: fn(
    woml_engine::IntervalCursorRegistration,
  ) -> woml_engine::durable::IntervalCursorRegistration = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::IntervalCursor>();
  let _: fn(woml_engine::IntervalCursor) -> woml_engine::durable::IntervalCursor = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::IntervalCursorRegistrationOutcome>();
  let _: fn(
    woml_engine::IntervalCursorRegistrationOutcome,
  ) -> woml_engine::durable::IntervalCursorRegistrationOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::TriggerRecoveryWork>();
  let _: fn(woml_engine::TriggerRecoveryWork) -> woml_engine::durable::TriggerRecoveryWork =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InternalEventAdmissionRequest>();
  let _: fn(
    woml_engine::InternalEventAdmissionRequest,
  ) -> woml_engine::durable::InternalEventAdmissionRequest = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InternalEventAdmissionOutcome>();
  let _: fn(
    woml_engine::InternalEventAdmissionOutcome,
  ) -> woml_engine::durable::InternalEventAdmissionOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::IssuedApprovalToken>();
  let _: fn(woml_engine::IssuedApprovalToken) -> woml_engine::durable::IssuedApprovalToken =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ApprovalTokenBinding>();
  let _: fn(woml_engine::ApprovalTokenBinding) -> woml_engine::durable::ApprovalTokenBinding =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::NotificationDeliveryWork>();
  let _: fn(
    woml_engine::NotificationDeliveryWork,
  ) -> woml_engine::durable::NotificationDeliveryWork = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::NotificationProviderDeliveryResult>();
  let _: fn(
    woml_engine::NotificationProviderDeliveryResult,
  ) -> woml_engine::durable::NotificationProviderDeliveryResult = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::NotificationUpdateWork>();
  let _: fn(woml_engine::NotificationUpdateWork) -> woml_engine::durable::NotificationUpdateWork =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::NotificationProviderUpdateResult>();
  let _: fn(
    woml_engine::NotificationProviderUpdateResult,
  ) -> woml_engine::durable::NotificationProviderUpdateResult = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::NotificationDispatchReport>();
  let _: fn(
    woml_engine::NotificationDispatchReport,
  ) -> woml_engine::durable::NotificationDispatchReport = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ApprovalDecisionOutcomeStatus>();
  let _: fn(
    woml_engine::ApprovalDecisionOutcomeStatus,
  ) -> woml_engine::durable::ApprovalDecisionOutcomeStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ApprovalDecisionOutcome>();
  let _: fn(woml_engine::ApprovalDecisionOutcome) -> woml_engine::durable::ApprovalDecisionOutcome =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ApprovalTimeoutSettlementStatus>();
  let _: fn(
    woml_engine::ApprovalTimeoutSettlementStatus,
  ) -> woml_engine::durable::ApprovalTimeoutSettlementStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ApprovalTimeoutSettlement>();
  let _: fn(
    woml_engine::ApprovalTimeoutSettlement,
  ) -> woml_engine::durable::ApprovalTimeoutSettlement = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RecoveryReport>();
  let _: fn(woml_engine::RecoveryReport) -> woml_engine::durable::RecoveryReport = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InfrastructurePolicyRunBindingV1>();
  let _: fn(
    woml_engine::InfrastructurePolicyRunBindingV1,
  ) -> woml_engine::durable::InfrastructurePolicyRunBindingV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::PublicRunStatus>();
  let _: fn(woml_engine::PublicRunStatus) -> woml_engine::durable::PublicRunStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunSummaryV1>();
  let _: fn(woml_engine::RunSummaryV1) -> woml_engine::durable::RunSummaryV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunListV1>();
  let _: fn(woml_engine::RunListV1) -> woml_engine::durable::RunListV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::PolicyWaitingFor>();
  let _: fn(woml_engine::PolicyWaitingFor) -> woml_engine::durable::PolicyWaitingFor =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunSummaryV2>();
  let _: fn(woml_engine::RunSummaryV2) -> woml_engine::durable::RunSummaryV2 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunListV2>();
  let _: fn(woml_engine::RunListV2) -> woml_engine::durable::RunListV2 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RuntimeObservationV1>();
  let _: fn(woml_engine::RuntimeObservationV1) -> woml_engine::durable::RuntimeObservationV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionHookV2>();
  let _: fn(woml_engine::RunInspectionHookV2) -> woml_engine::durable::RunInspectionHookV2 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionCancellationV2>();
  let _: fn(
    woml_engine::RunInspectionCancellationV2,
  ) -> woml_engine::durable::RunInspectionCancellationV2 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV2>();
  let _: fn(woml_engine::RunInspectionV2) -> woml_engine::durable::RunInspectionV2 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionPolicyV3>();
  let _: fn(woml_engine::RunInspectionPolicyV3) -> woml_engine::durable::RunInspectionPolicyV3 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV3>();
  let _: fn(woml_engine::RunInspectionV3) -> woml_engine::durable::RunInspectionV3 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedForkStatusV4>();
  let _: fn(woml_engine::InspectedForkStatusV4) -> woml_engine::durable::InspectedForkStatusV4 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedForkJoinModeV4>();
  let _: fn(woml_engine::InspectedForkJoinModeV4) -> woml_engine::durable::InspectedForkJoinModeV4 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForkCountsV4>();
  let _: fn(
    woml_engine::RunInspectionForkCountsV4,
  ) -> woml_engine::durable::RunInspectionForkCountsV4 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForkJoinV4>();
  let _: fn(woml_engine::RunInspectionForkJoinV4) -> woml_engine::durable::RunInspectionForkJoinV4 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForkV4>();
  let _: fn(woml_engine::RunInspectionForkV4) -> woml_engine::durable::RunInspectionForkV4 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForksV4>();
  let _: fn(woml_engine::RunInspectionForksV4) -> woml_engine::durable::RunInspectionForksV4 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV4>();
  let _: fn(woml_engine::RunInspectionV4) -> woml_engine::durable::RunInspectionV4 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedReusableKindV5>();
  let _: fn(woml_engine::InspectedReusableKindV5) -> woml_engine::durable::InspectedReusableKindV5 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedReusableStatusV5>();
  let _: fn(
    woml_engine::InspectedReusableStatusV5,
  ) -> woml_engine::durable::InspectedReusableStatusV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedReusableLifecycleStatusV5>();
  let _: fn(
    woml_engine::InspectedReusableLifecycleStatusV5,
  ) -> woml_engine::durable::InspectedReusableLifecycleStatusV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionReusableItemV5>();
  let _: fn(
    woml_engine::RunInspectionReusableItemV5,
  ) -> woml_engine::durable::RunInspectionReusableItemV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionReusableCountsV5>();
  let _: fn(
    woml_engine::RunInspectionReusableCountsV5,
  ) -> woml_engine::durable::RunInspectionReusableCountsV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionReusableDefinitionsV5>();
  let _: fn(
    woml_engine::RunInspectionReusableDefinitionsV5,
  ) -> woml_engine::durable::RunInspectionReusableDefinitionsV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV5>();
  let _: fn(woml_engine::RunInspectionV5) -> woml_engine::durable::RunInspectionV5 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedForEachStatusV6>();
  let _: fn(
    woml_engine::InspectedForEachStatusV6,
  ) -> woml_engine::durable::InspectedForEachStatusV6 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForEachCountsV6>();
  let _: fn(
    woml_engine::RunInspectionForEachCountsV6,
  ) -> woml_engine::durable::RunInspectionForEachCountsV6 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForEachItemV6>();
  let _: fn(
    woml_engine::RunInspectionForEachItemV6,
  ) -> woml_engine::durable::RunInspectionForEachItemV6 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionForEachV6>();
  let _: fn(woml_engine::RunInspectionForEachV6) -> woml_engine::durable::RunInspectionForEachV6 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV6>();
  let _: fn(woml_engine::RunInspectionV6) -> woml_engine::durable::RunInspectionV6 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionRichApprovalV11>();
  let _: fn(
    woml_engine::RunInspectionRichApprovalV11,
  ) -> woml_engine::durable::RunInspectionRichApprovalV11 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionRichApprovalsV11>();
  let _: fn(
    woml_engine::RunInspectionRichApprovalsV11,
  ) -> woml_engine::durable::RunInspectionRichApprovalsV11 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunInspectionV11>();
  let _: fn(woml_engine::RunInspectionV11) -> woml_engine::durable::RunInspectionV11 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ForkRecoveryBranchV1>();
  let _: fn(woml_engine::ForkRecoveryBranchV1) -> woml_engine::durable::ForkRecoveryBranchV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ForkRecoveryWorkV1>();
  let _: fn(woml_engine::ForkRecoveryWorkV1) -> woml_engine::durable::ForkRecoveryWorkV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::SchedulerClaimV1>();
  let _: fn(woml_engine::SchedulerClaimV1) -> woml_engine::durable::SchedulerClaimV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::DurableWaitRecordV1>();
  let _: fn(woml_engine::DurableWaitRecordV1) -> woml_engine::durable::DurableWaitRecordV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedWaitStatusV1>();
  let _: fn(woml_engine::InspectedWaitStatusV1) -> woml_engine::durable::InspectedWaitStatusV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedWaitV1>();
  let _: fn(woml_engine::InspectedWaitV1) -> woml_engine::durable::InspectedWaitV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::WaitInspectionV1>();
  let _: fn(woml_engine::WaitInspectionV1) -> woml_engine::durable::WaitInspectionV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionSagaInspectionV1>();
  let _: fn(
    woml_engine::CorrectionSagaInspectionV1,
  ) -> woml_engine::durable::CorrectionSagaInspectionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::UnresolvedCorrectionActionV1>();
  let _: fn(
    woml_engine::UnresolvedCorrectionActionV1,
  ) -> woml_engine::durable::UnresolvedCorrectionActionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionInspectionV1>();
  let _: fn(woml_engine::CorrectionInspectionV1) -> woml_engine::durable::CorrectionInspectionV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionRecoveryActionV1>();
  let _: fn(
    woml_engine::CorrectionRecoveryActionV1,
  ) -> woml_engine::durable::CorrectionRecoveryActionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionRecoveryRequestV1>();
  let _: fn(
    woml_engine::CorrectionRecoveryRequestV1,
  ) -> woml_engine::durable::CorrectionRecoveryRequestV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionRecoveryStatusV1>();
  let _: fn(
    woml_engine::CorrectionRecoveryStatusV1,
  ) -> woml_engine::durable::CorrectionRecoveryStatusV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionRecoveryResultV1>();
  let _: fn(
    woml_engine::CorrectionRecoveryResultV1,
  ) -> woml_engine::durable::CorrectionRecoveryResultV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionRecoveryCandidateV1>();
  let _: fn(
    woml_engine::CorrectionRecoveryCandidateV1,
  ) -> woml_engine::durable::CorrectionRecoveryCandidateV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::CorrectionObservationV1>();
  let _: fn(woml_engine::CorrectionObservationV1) -> woml_engine::durable::CorrectionObservationV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedLockStatusV1>();
  let _: fn(woml_engine::InspectedLockStatusV1) -> woml_engine::durable::InspectedLockStatusV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::LockInspectionV1>();
  let _: fn(woml_engine::LockInspectionV1) -> woml_engine::durable::LockInspectionV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::LockObservationV1>();
  let _: fn(woml_engine::LockObservationV1) -> woml_engine::durable::LockObservationV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::LockRecoveryKindV1>();
  let _: fn(woml_engine::LockRecoveryKindV1) -> woml_engine::durable::LockRecoveryKindV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::LockRecoveryCandidateV1>();
  let _: fn(woml_engine::LockRecoveryCandidateV1) -> woml_engine::durable::LockRecoveryCandidateV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ExternalSignalAdmissionRequest>();
  let _: fn(
    woml_engine::ExternalSignalAdmissionRequest,
  ) -> woml_engine::durable::ExternalSignalAdmissionRequest = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ExternalSignalAdmissionStatus>();
  let _: fn(
    woml_engine::ExternalSignalAdmissionStatus,
  ) -> woml_engine::durable::ExternalSignalAdmissionStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ExternalSignalResolution>();
  let _: fn(
    woml_engine::ExternalSignalResolution,
  ) -> woml_engine::durable::ExternalSignalResolution = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ExternalSignalAdmissionOutcome>();
  let _: fn(
    woml_engine::ExternalSignalAdmissionOutcome,
  ) -> woml_engine::durable::ExternalSignalAdmissionOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::SignalWaitTimeoutSettlement>();
  let _: fn(
    woml_engine::SignalWaitTimeoutSettlement,
  ) -> woml_engine::durable::SignalWaitTimeoutSettlement = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::TimerWaitSettlement>();
  let _: fn(woml_engine::TimerWaitSettlement) -> woml_engine::durable::TimerWaitSettlement =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::PolicyClaimWaitReason>();
  let _: fn(woml_engine::PolicyClaimWaitReason) -> woml_engine::durable::PolicyClaimWaitReason =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::PolicyExecutionClaimResult>();
  let _: fn(
    woml_engine::PolicyExecutionClaimResult,
  ) -> woml_engine::durable::PolicyExecutionClaimResult = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::InspectedBusinessOutcome>();
  let _: fn(
    woml_engine::InspectedBusinessOutcome,
  ) -> woml_engine::durable::InspectedBusinessOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunCancellationStatus>();
  let _: fn(woml_engine::RunCancellationStatus) -> woml_engine::durable::RunCancellationStatus =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunTimeoutSettlement>();
  let _: fn(woml_engine::RunTimeoutSettlement) -> woml_engine::durable::RunTimeoutSettlement =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunCancellationCode>();
  let _: fn(woml_engine::RunCancellationCode) -> woml_engine::durable::RunCancellationCode =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RunCancellationResult>();
  let _: fn(woml_engine::RunCancellationResult) -> woml_engine::durable::RunCancellationResult =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::DurableStoreError>();
  let _: fn(woml_engine::DurableStoreError) -> woml_engine::durable::DurableStoreError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::DurableEventStore>();
  let _: fn(woml_engine::DurableEventStore) -> woml_engine::durable::DurableEventStore =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalDecisionAdmission>();
  let _: fn(
    woml_engine::RichApprovalDecisionAdmission,
  ) -> woml_engine::durable::RichApprovalDecisionAdmission = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalIdentityDecision>();
  let _: fn(
    woml_engine::RichApprovalIdentityDecision,
  ) -> woml_engine::durable::RichApprovalIdentityDecision = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalDecisionStatus>();
  let _: fn(
    woml_engine::RichApprovalDecisionStatus,
  ) -> woml_engine::durable::RichApprovalDecisionStatus = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalDecisionOutcome>();
  let _: fn(
    woml_engine::RichApprovalDecisionOutcome,
  ) -> woml_engine::durable::RichApprovalDecisionOutcome = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalDecisionInspection>();
  let _: fn(
    woml_engine::RichApprovalDecisionInspection,
  ) -> woml_engine::durable::RichApprovalDecisionInspection = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::RichApprovalInspectionV1>();
  let _: fn(
    woml_engine::RichApprovalInspectionV1,
  ) -> woml_engine::durable::RichApprovalInspectionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::LockLeaseMaintenance>();
  let _: fn(woml_engine::LockLeaseMaintenance) -> woml_engine::durable::LockLeaseMaintenance =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ProgrammaticLockLease>();
  let _: fn(woml_engine::ProgrammaticLockLease) -> woml_engine::durable::ProgrammaticLockLease =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ProgrammaticLockAcquireState>();
  let _: fn(
    woml_engine::ProgrammaticLockAcquireState,
  ) -> woml_engine::durable::ProgrammaticLockAcquireState = |value| value;
  let _ = std::any::type_name::<woml_engine::durable::ProgrammaticLockAcquire>();
  let _: fn(woml_engine::ProgrammaticLockAcquire) -> woml_engine::durable::ProgrammaticLockAcquire =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::DurableEngineError>();
  let _: fn(woml_engine::DurableEngineError) -> woml_engine::durable::DurableEngineError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::StepFailureDisposition>();
  let _: fn(woml_engine::StepFailureDisposition) -> woml_engine::durable::StepFailureDisposition =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::StepFailureCommit>();
  let _: fn(woml_engine::StepFailureCommit) -> woml_engine::durable::StepFailureCommit =
    |value| value;
  let _ = std::any::type_name::<woml_engine::durable::DurableDagEngine>();
  let _: fn(woml_engine::DurableDagEngine) -> woml_engine::durable::DurableDagEngine =
    |value| value;
  let _ = std::any::type_name::<dyn woml_engine::durable::NotificationProviderAdapter>();
  let _ = woml_engine::durable::derive_lifecycle_hook_invocation_id;
  let _ = woml_engine::durable::correction_invocation_id;
  let _ = woml_engine::durable::RUNTIME_POLICY_QUEUE_CEILING;
  let _ = woml_engine::durable::DURABLE_STORE_SCHEMA_VERSION;
  let _ = woml_engine::durable::RUN_INSPECTION_RICH_APPROVAL_LIMIT;
  let _ = woml_engine::durable::CORRECTION_INSPECTION_PROFILE;
  let _ = woml_engine::durable::CORRECTION_RECOVERY_REQUEST_PROFILE;
  let _ = woml_engine::durable::CORRECTION_RECOVERY_RESULT_PROFILE;
  let _ = woml_engine::durable::CORRECTION_OBSERVATION_PROFILE;
  let _ = woml_engine::durable::CORRECTION_RECOVERY_BATCH_LIMIT;
  let _ = woml_engine::durable::CORRECTION_UNRESOLVED_STORE_LIMIT;
  let _ = woml_engine::durable::EXTERNAL_SIGNAL_ADMISSION_PROFILE;
  let _ = woml_engine::durable::EXTERNAL_SIGNAL_RESULT_PROFILE;
  let _ = woml_engine::durable::EXTERNAL_SIGNAL_MAX_PAYLOAD_BYTES;
  let _ = woml_engine::durable::EXTERNAL_SIGNAL_MAX_PAYLOAD_DEPTH;
  let _ = woml_engine::durable::EXTERNAL_SIGNAL_MAX_EARLY_PER_RUN;
  let _ = woml_engine::durable::RICH_APPROVAL_INSPECTION_DECISION_LIMIT;
  let _ = woml_engine::durable::LOCK_WAITERS_PER_KEY_LIMIT;
  let _ = woml_engine::durable::LOCK_WAITERS_PER_STORE_LIMIT;
  let _ = woml_engine::durable::LOCK_RECOVERY_BATCH_LIMIT;
  let _ = woml_engine::durable::MAX_MODULE_ARTIFACT_BYTES;
  let _ = woml_engine::durable::MAX_MODULE_ARTIFACT_SET_BYTES;
}

#[test]
fn capability_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::capability::CapabilityIdentityMode>();
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallIdentity>();
  let _: fn(
    woml_engine::CapabilityCallIdentity,
  ) -> woml_engine::capability::CapabilityCallIdentity = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallLimits>();
  let _: fn(woml_engine::CapabilityCallLimits) -> woml_engine::capability::CapabilityCallLimits =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallRequest>();
  let _: fn(woml_engine::CapabilityCallRequest) -> woml_engine::capability::CapabilityCallRequest =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityFailureKind>();
  let _: fn(woml_engine::CapabilityFailureKind) -> woml_engine::capability::CapabilityFailureKind =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityFailure>();
  let _: fn(woml_engine::CapabilityFailure) -> woml_engine::capability::CapabilityFailure =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallSucceeded>();
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallFailed>();
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCallResult>();
  let _: fn(woml_engine::CapabilityCallResult) -> woml_engine::capability::CapabilityCallResult =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityEffect>();
  let _: fn(woml_engine::CapabilityEffect) -> woml_engine::capability::CapabilityEffect =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityDescriptor>();
  let _: fn(woml_engine::CapabilityDescriptor) -> woml_engine::capability::CapabilityDescriptor =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityCancellationToken>();
  let _: fn(
    woml_engine::CapabilityCancellationToken,
  ) -> woml_engine::capability::CapabilityCancellationToken = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityRegistryError>();
  let _: fn(
    woml_engine::CapabilityRegistryError,
  ) -> woml_engine::capability::CapabilityRegistryError = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::CapabilityRegistry>();
  let _: fn(woml_engine::CapabilityRegistry) -> woml_engine::capability::CapabilityRegistry =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::DurableCapabilityAuthorityError>();
  let _: fn(
    woml_engine::DurableCapabilityAuthorityError,
  ) -> woml_engine::capability::DurableCapabilityAuthorityError = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::NativeFetchInvocationIdentity>();
  let _: fn(
    woml_engine::NativeFetchInvocationIdentity,
  ) -> woml_engine::capability::NativeFetchInvocationIdentity = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::NativeFetchFailureKind>();
  let _: fn(
    woml_engine::NativeFetchFailureKind,
  ) -> woml_engine::capability::NativeFetchFailureKind = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::NativeFetchFailure>();
  let _: fn(woml_engine::NativeFetchFailure) -> woml_engine::capability::NativeFetchFailure =
    |value| value;
  let _ = std::any::type_name::<woml_engine::capability::NativeFetchObservation>();
  let _: fn(
    woml_engine::NativeFetchObservation,
  ) -> woml_engine::capability::NativeFetchObservation = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::DurableCapabilityAuthority>();
  let _: fn(
    woml_engine::DurableCapabilityAuthority,
  ) -> woml_engine::capability::DurableCapabilityAuthority = |value| value;
  let _ = std::any::type_name::<woml_engine::capability::TestCapabilityHandler>();
  let _: fn(woml_engine::TestCapabilityHandler) -> woml_engine::capability::TestCapabilityHandler =
    |value| value;
  let _ = woml_engine::capability::CAPABILITY_CALL_CONTRACT;
  let _ = woml_engine::capability::CAPABILITY_CALL_CONTRACT_VERSION;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_INPUT_BYTES;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_RESULT_BYTES;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_FRAME_BYTES;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_TIMEOUT_MS;
  let _ = woml_engine::capability::MAX_CAPABILITY_TIMEOUT_MS;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_IN_FLIGHT_PER_INVOCATION;
  let _ = woml_engine::capability::DEFAULT_CAPABILITY_IN_FLIGHT_GLOBAL;
  let _ = woml_engine::capability::NATIVE_FETCH_OBSERVATION_CONTRACT;
  let _ = woml_engine::capability::NATIVE_FETCH_OBSERVATION_CONTRACT_VERSION;
  let _ = woml_engine::capability::derive_operation_key;
  let _ = woml_engine::capability::validate_safe_metadata;
  let _ = woml_engine::capability::capability_transport_failure;
  let _: Option<&dyn woml_engine::capability::CapabilityHandler> = None;
}

#[test]
fn process_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::process::ProcessRuntimeMode>();
  let _: fn(woml_engine::ProcessRuntimeMode) -> woml_engine::process::ProcessRuntimeMode =
    |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyV1>();
  let _: fn(woml_engine::ProcessPolicyV1) -> woml_engine::process::ProcessPolicyV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyModes>();
  let _: fn(woml_engine::ProcessPolicyModes) -> woml_engine::process::ProcessPolicyModes =
    |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyExecutable>();
  let _: fn(woml_engine::ProcessPolicyExecutable) -> woml_engine::process::ProcessPolicyExecutable =
    |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyEnvironment>();
  let _: fn(
    woml_engine::ProcessPolicyEnvironment,
  ) -> woml_engine::process::ProcessPolicyEnvironment = |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyLimits>();
  let _: fn(woml_engine::ProcessPolicyLimits) -> woml_engine::process::ProcessPolicyLimits =
    |value| value;
  let _ = std::any::type_name::<woml_engine::process::ProcessPolicyError>();
  let _: fn(woml_engine::ProcessPolicyError) -> woml_engine::process::ProcessPolicyError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::process::ManagedProcessHandler>();
  let _: fn(woml_engine::ManagedProcessHandler) -> woml_engine::process::ManagedProcessHandler =
    |value| value;
  let _ = woml_engine::process::PROCESS_CONTRACT;
  let _ = woml_engine::process::PROCESS_CONTRACT_VERSION;
  let _ = woml_engine::process::PROCESS_CONTRACT_VERSION_V2;
  let _ = woml_engine::process::PROCESS_POLICY_CONTRACT;
  let _ = woml_engine::process::PROCESS_POLICY_CONTRACT_VERSION;
}

#[test]
fn artifacts_contract_paths_remain_available() {
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactRuntimeMode>();
  let _: fn(woml_engine::ArtifactRuntimeMode) -> woml_engine::artifacts::ArtifactRuntimeMode =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactPolicyV1>();
  let _: fn(woml_engine::ArtifactPolicyV1) -> woml_engine::artifacts::ArtifactPolicyV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactOperationalOutputPolicy>();
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactPolicyModes>();
  let _: fn(woml_engine::ArtifactPolicyModes) -> woml_engine::artifacts::ArtifactPolicyModes =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactMaterializationPolicy>();
  let _: fn(
    woml_engine::ArtifactMaterializationPolicy,
  ) -> woml_engine::artifacts::ArtifactMaterializationPolicy = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactLimits>();
  let _: fn(woml_engine::ArtifactLimits) -> woml_engine::artifacts::ArtifactLimits = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactRetentionPolicy>();
  let _: fn(
    woml_engine::ArtifactRetentionPolicy,
  ) -> woml_engine::artifacts::ArtifactRetentionPolicy = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::KnownSecretAction>();
  let _: fn(woml_engine::KnownSecretAction) -> woml_engine::artifacts::KnownSecretAction =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactSecretScanningPolicy>();
  let _: fn(
    woml_engine::ArtifactSecretScanningPolicy,
  ) -> woml_engine::artifacts::ArtifactSecretScanningPolicy = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactBackendPolicy>();
  let _: fn(woml_engine::ArtifactBackendPolicy) -> woml_engine::artifacts::ArtifactBackendPolicy =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactReferenceV1>();
  let _: fn(woml_engine::ArtifactReferenceV1) -> woml_engine::artifacts::ArtifactReferenceV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactMaintenanceV1>();
  let _: fn(woml_engine::ArtifactMaintenanceV1) -> woml_engine::artifacts::ArtifactMaintenanceV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactBackupInventoryV1>();
  let _: fn(
    woml_engine::ArtifactBackupInventoryV1,
  ) -> woml_engine::artifacts::ArtifactBackupInventoryV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactInspectionV1>();
  let _: fn(woml_engine::ArtifactInspectionV1) -> woml_engine::artifacts::ArtifactInspectionV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactProvenanceV1>();
  let _: fn(woml_engine::ArtifactProvenanceV1) -> woml_engine::artifacts::ArtifactProvenanceV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactRunInspectionV1>();
  let _: fn(
    woml_engine::ArtifactRunInspectionV1,
  ) -> woml_engine::artifacts::ArtifactRunInspectionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactExportV1>();
  let _: fn(woml_engine::ArtifactExportV1) -> woml_engine::artifacts::ArtifactExportV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactObservationV1>();
  let _: fn(woml_engine::ArtifactObservationV1) -> woml_engine::artifacts::ArtifactObservationV1 =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ArtifactPolicyError>();
  let _: fn(woml_engine::ArtifactPolicyError) -> woml_engine::artifacts::ArtifactPolicyError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ManagedArtifactStore>();
  let _: fn(woml_engine::ManagedArtifactStore) -> woml_engine::artifacts::ManagedArtifactStore =
    |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ManagedArtifactPutHandler>();
  let _: fn(
    woml_engine::ManagedArtifactPutHandler,
  ) -> woml_engine::artifacts::ManagedArtifactPutHandler = |value| value;
  let _ = std::any::type_name::<woml_engine::artifacts::ManagedArtifactReadHandler>();
  let _ = woml_engine::artifacts::ARTIFACTS_CONTRACT;
  let _ = woml_engine::artifacts::ARTIFACTS_CONTRACT_VERSION;
  let _ = woml_engine::artifacts::ARTIFACT_POLICY_CONTRACT;
  let _ = woml_engine::artifacts::ARTIFACT_POLICY_CONTRACT_VERSION;
  // These closures are compiled, never invoked: the check must not open a
  // database, publish bytes, or run maintenance just to verify an API path.
  let _ = |path: std::path::PathBuf, policy, mode, secrets, now, dry_run| {
    woml_engine::artifacts::maintain_artifacts(path, policy, mode, secrets, now, dry_run)
  };
  let _ = |path: std::path::PathBuf, id| woml_engine::artifacts::inspect_artifact(path, id);
  let _ = |path: std::path::PathBuf, run| woml_engine::artifacts::inspect_run_artifacts(path, run);
  let _ = |path: std::path::PathBuf, destination: std::path::PathBuf, id, policy, mode, secrets| {
    woml_engine::artifacts::export_artifact(path, destination, id, policy, mode, secrets)
  };
  let _ = |path: std::path::PathBuf, policy, mode, secrets| {
    woml_engine::artifacts::observe_artifacts(path, policy, mode, secrets)
  };
  let _ = |path: std::path::PathBuf, destination: std::path::PathBuf, policy, mode, secrets| {
    woml_engine::artifacts::export_artifact_backup(path, destination, policy, mode, secrets)
  };
  let _ = |path: std::path::PathBuf, source: std::path::PathBuf, policy, mode, secrets| {
    woml_engine::artifacts::restore_artifact_backup(path, source, policy, mode, secrets)
  };
  let _ = |path: std::path::PathBuf, source: std::path::PathBuf| {
    woml_engine::artifacts::verify_artifact_backup(path, source)
  };
}

#[test]
fn remote_ingress_and_notification_contract_paths_remain_available() {
  let _ =
    std::any::type_name::<woml_engine::remote_worker_journal::EnvironmentRemoteSecretProvider>();
  let _: fn(
    woml_engine::EnvironmentRemoteSecretProvider,
  ) -> woml_engine::remote_worker_journal::EnvironmentRemoteSecretProvider = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::PreparedRemoteDispatchV1>();
  let _: fn(
    woml_engine::PreparedRemoteDispatchV1,
  ) -> woml_engine::remote_worker_journal::PreparedRemoteDispatchV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::RemoteWorkerAttemptRuntimeV1>();
  let _: fn(
    woml_engine::RemoteWorkerAttemptRuntimeV1,
  ) -> woml_engine::remote_worker_journal::RemoteWorkerAttemptRuntimeV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::RemoteWorkerAuthorityV1>();
  let _: fn(
    woml_engine::RemoteWorkerAuthorityV1,
  ) -> woml_engine::remote_worker_journal::RemoteWorkerAuthorityV1 = |value| value;
  let _ =
    std::any::type_name::<woml_engine::remote_worker_journal::RemoteWorkerCancellationTokenV1>();
  let _: fn(
    woml_engine::RemoteWorkerCancellationTokenV1,
  ) -> woml_engine::remote_worker_journal::RemoteWorkerCancellationTokenV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::RemoteWorkerError>();
  let _: fn(
    woml_engine::RemoteWorkerError,
  ) -> woml_engine::remote_worker_journal::RemoteWorkerError = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::RemoteWorkerTerminalV1>();
  let _: fn(
    woml_engine::RemoteWorkerTerminalV1,
  ) -> woml_engine::remote_worker_journal::RemoteWorkerTerminalV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::ResolvedRemoteSecretProvider>();
  let _: fn(
    woml_engine::ResolvedRemoteSecretProvider,
  ) -> woml_engine::remote_worker_journal::ResolvedRemoteSecretProvider = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::WorkerAttemptRecordV1>();
  let _: fn(
    woml_engine::WorkerAttemptRecordV1,
  ) -> woml_engine::remote_worker_journal::WorkerAttemptRecordV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::WorkerAttemptStateV1>();
  let _: fn(
    woml_engine::WorkerAttemptStateV1,
  ) -> woml_engine::remote_worker_journal::WorkerAttemptStateV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_worker_journal::WorkerJournalV1>();
  let _: fn(woml_engine::WorkerJournalV1) -> woml_engine::remote_worker_journal::WorkerJournalV1 =
    |value| value;
  let _ = std::any::type_name::<dyn woml_engine::remote_worker_journal::RemoteSecretProvider>();
  let _ = woml_engine::remote_worker_journal::REMOTE_CANCELLATION_CONFIRMATION_TIMEOUT_MS;
  let _ = woml_engine::remote_worker_journal::WORKER_JOURNAL_RECOVERY_LIMIT;
  let _ = woml_engine::remote_worker_journal::WORKER_JOURNAL_SCHEMA_VERSION;
  let _ = woml_engine::remote_worker_journal::prepare_remote_dispatch;
  let _ = woml_engine::remote_worker_journal::verify_remote_artifact_transfer;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchError>();
  let _: fn(woml_engine::RemoteDispatchError) -> woml_engine::remote_dispatch::RemoteDispatchError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchInspectionV1>();
  let _: fn(
    woml_engine::RemoteDispatchInspectionV1,
  ) -> woml_engine::remote_dispatch::RemoteDispatchInspectionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchRecordV1>();
  let _: fn(
    woml_engine::RemoteDispatchRecordV1,
  ) -> woml_engine::remote_dispatch::RemoteDispatchRecordV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchRequestV1>();
  let _: fn(
    woml_engine::RemoteDispatchRequestV1,
  ) -> woml_engine::remote_dispatch::RemoteDispatchRequestV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchResolutionV1>();
  let _: fn(
    woml_engine::RemoteDispatchResolutionV1,
  ) -> woml_engine::remote_dispatch::RemoteDispatchResolutionV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteDispatchStateV1>();
  let _: fn(
    woml_engine::RemoteDispatchStateV1,
  ) -> woml_engine::remote_dispatch::RemoteDispatchStateV1 = |value| value;
  let _ = std::any::type_name::<woml_engine::remote_dispatch::RemoteOutputBindingV1>();
  let _: fn(
    woml_engine::RemoteOutputBindingV1,
  ) -> woml_engine::remote_dispatch::RemoteOutputBindingV1 = |value| value;
  let _ = woml_engine::remote_dispatch::REMOTE_DISPATCH_HEARTBEAT_MAX_BYTES;
  let _ = woml_engine::remote_dispatch::REMOTE_DISPATCH_RECOVERY_BATCH_LIMIT;
  let _ = woml_engine::remote_dispatch::derive_remote_dispatch_id;
  let _ =
    std::any::type_name::<woml_engine::remote_coordinator_server::RemoteCoordinatorServerError>();
  let _: fn(
    woml_engine::RemoteCoordinatorServerError,
  ) -> woml_engine::remote_coordinator_server::RemoteCoordinatorServerError = |value| value;
  let _ =
    std::any::type_name::<woml_engine::remote_coordinator_server::RemoteCoordinatorServerV1>();
  let _: fn(
    woml_engine::RemoteCoordinatorServerV1,
  ) -> woml_engine::remote_coordinator_server::RemoteCoordinatorServerV1 = |value| value;
  let _ =
    std::any::type_name::<woml_engine::remote_coordinator_server::RemoteCoordinatorTlsOptionsV1>();
  let _: fn(
    woml_engine::RemoteCoordinatorTlsOptionsV1,
  ) -> woml_engine::remote_coordinator_server::RemoteCoordinatorTlsOptionsV1 = |value| value;
  let _ = woml_engine::remote_coordinator_server::REMOTE_COORDINATOR_IO_TIMEOUT_SECONDS;
  let _ = woml_engine::remote_coordinator_server::REMOTE_COORDINATOR_MAX_CONNECTIONS;
  let _ = woml_engine::remote_coordinator_server::REMOTE_COORDINATOR_MAX_CONNECTIONS_PER_SECOND;
  let _ = woml_engine::remote_coordinator_server::REMOTE_COORDINATOR_MAX_HEADER_BYTES;
  let _ = std::any::type_name::<woml_engine::webhook::ExternalTriggerAdmissionCommand>();
  let _: fn(
    woml_engine::ExternalTriggerAdmissionCommand,
  ) -> woml_engine::webhook::ExternalTriggerAdmissionCommand = |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::ExternalTriggerAdmissionReceiver>();
  let _: fn(
    woml_engine::ExternalTriggerAdmissionReceiver,
  ) -> woml_engine::webhook::ExternalTriggerAdmissionReceiver = |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::TriggerProgress>();
  let _: fn(woml_engine::TriggerProgress) -> woml_engine::webhook::TriggerProgress = |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::TriggerProgressReporter>();
  let _: fn(woml_engine::TriggerProgressReporter) -> woml_engine::webhook::TriggerProgressReporter =
    |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::WebhookDefinitionRegistration>();
  let _: fn(
    woml_engine::WebhookDefinitionRegistration,
  ) -> woml_engine::webhook::WebhookDefinitionRegistration = |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::WebhookRuntimeError>();
  let _: fn(woml_engine::WebhookRuntimeError) -> woml_engine::webhook::WebhookRuntimeError =
    |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::WomlWebhookServer>();
  let _: fn(woml_engine::WomlWebhookServer) -> woml_engine::webhook::WomlWebhookServer =
    |value| value;
  let _ = std::any::type_name::<woml_engine::webhook::WomlWebhookServerConfig>();
  let _: fn(woml_engine::WomlWebhookServerConfig) -> woml_engine::webhook::WomlWebhookServerConfig =
    |value| value;
  let _ = woml_engine::webhook::COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS;
  let _ = woml_engine::webhook::COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES;
  let _ = woml_engine::webhook::EVENT_MAX_SUBSCRIBERS;
  let _ = woml_engine::webhook::TRIGGER_PROGRESS_CONTRACT;
  let _ = woml_engine::webhook::TRIGGER_PROGRESS_CONTRACT_VERSION;
  let _ = woml_engine::webhook::WEBHOOK_MAX_BODY_BYTES;
  let _ = woml_engine::webhook::WHATSAPP_CALLBACK_PATH;
  let _ =
    std::any::type_name::<woml_engine::notification_runtime::CustomNotificationJourneyOptions>();
  let _: fn(
    woml_engine::CustomNotificationJourneyOptions,
  ) -> woml_engine::notification_runtime::CustomNotificationJourneyOptions = |value| value;
  let _ =
    std::any::type_name::<woml_engine::notification_runtime::NotificationDeliveryDiagnostic>();
  let _: fn(
    woml_engine::NotificationDeliveryDiagnostic,
  ) -> woml_engine::notification_runtime::NotificationDeliveryDiagnostic = |value| value;
  let _ =
    std::any::type_name::<woml_engine::notification_runtime::NotificationJourneyDiagnostics>();
  let _: fn(
    woml_engine::NotificationJourneyDiagnostics,
  ) -> woml_engine::notification_runtime::NotificationJourneyDiagnostics = |value| value;
  let _ = std::any::type_name::<woml_engine::notification_runtime::NotificationJourneyError>();
  let _: fn(
    woml_engine::NotificationJourneyError,
  ) -> woml_engine::notification_runtime::NotificationJourneyError = |value| value;
  let _ = std::any::type_name::<woml_engine::notification_runtime::NotificationJourneyResult>();
  let _: fn(
    woml_engine::NotificationJourneyResult,
  ) -> woml_engine::notification_runtime::NotificationJourneyResult = |value| value;
  let _ = woml_engine::notification_runtime::NOTIFICATION_JOURNEY_DIAGNOSTICS_VERSION;
  // impl Trait path parameters are checked through uncalled typed closures;
  // this compile-only contract test must not start a provider journey.
  let _ = |path: std::path::PathBuf, host: woml_engine::NotificationHostProcessOptions| {
    std::mem::drop(
      woml_engine::notification_runtime::run_notification_provider_journey(
        path,
        "",
        host,
        std::time::Duration::ZERO,
      ),
    );
  };
  let _ = |path: std::path::PathBuf, host: woml_engine::NotificationHostProcessOptions| {
    std::mem::drop(
      woml_engine::notification_runtime::run_notification_provider_journey_with_custom(
        path,
        "",
        host,
        std::time::Duration::ZERO,
        None,
      ),
    );
  };
}
