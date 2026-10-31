//! Stable model contract facade. Private modules separate shapes and validation.

mod approval_validation;
mod branch_validation;
mod contracts;
mod control_flow;
mod expressions;
mod fork_validation;
mod issues;
mod iteration_validation;
mod lifecycle;
mod lifecycle_validation;
mod lock_validation;
mod parallel_validation;
mod placement_validation;
mod policies;
mod policy_validation;
mod queries;
mod reusable;
mod reusable_validation;
mod saga_validation;
mod trigger_validation;
mod validation;
mod wait_validation;

use approval_validation::{
  approval_notifications, approval_wait_inputs, inspect_approval_contract,
};
use branch_validation::{
  contains_lifecycle_reference, inspect_branch_contract, inspect_expression, parallel_join_inputs,
  parallel_start_id, parallel_start_inputs,
};
pub(crate) use contracts::{ApprovalDefinition, ParallelGroupDefinition};
pub use contracts::{
  CompiledCommunicationProviderRequirement, CompiledCommunicationRequirements,
  CompiledExecutionPlacement, CompiledModuleBinding, CompiledModuleRuntime, CompiledTrigger,
  CompiledWorkflowDefinition, CompiledWorkflowEdge, CompiledWorkflowGraph,
  CompiledWorkflowMetadata, CompiledWorkflowNode, NotificationDefinition, ScriptRuntimeBindings,
};
pub use control_flow::{
  CompiledContextVisibility, CompiledControlChoice, CompiledCorrectionIdentity,
  CompiledCorrectiveAction, CompiledForEach, CompiledForEachBody, CompiledFork, CompiledForkBranch,
  CompiledLock, CompiledSaga, CompiledStringChoiceCase, CompiledWait, CompiledWaitSignal,
  CompiledWorkflowSettlement,
};
pub use expressions::{EdgeCondition, TemplatePart, ValueExpression};
use fork_validation::inspect_fork_contract;
pub use issues::{ModelIssue, ModelIssueCode, ModelValidationError};
use iteration_validation::{inspect_for_each_contract, valid_for_each_path_segment};
pub use lifecycle::{
  CompiledLifecycleAction, CompiledLifecycleDefinition, CompiledLifecycleHook, LifecycleEventName,
};
use lifecycle_validation::{
  inspect_lifecycle_contract, reusable_provider_ids, valid_custom_provider_delivery,
};
use lock_validation::inspect_lock_contract;
use parallel_validation::inspect_parallel_contract;
use placement_validation::inspect_execution_placement_contract;
pub use policies::{
  BackoffPolicy, CompiledQueuePolicy, CompiledRateLimitPolicy, CompiledRuntimePolicy,
  QueueDiscipline, RateLimitAlgorithm, RetryPolicy,
};
use policy_validation::inspect_runtime_policy_contract;
pub use reusable::{
  CompiledReusableBoundProp, CompiledReusableInvocation, CompiledReusableLifecycle,
  CompiledReusablePropExpression,
};
use reusable_validation::inspect_reusable_contract;
use saga_validation::inspect_saga_contract;
use trigger_validation::{
  collect_custom_provider_ids, literal_string, object_fields, valid_discord_snowflake,
  valid_model_v7_trigger, valid_slack_destination, valid_whatsapp_language,
  valid_whatsapp_phone_number_id, valid_whatsapp_recipient, valid_whatsapp_template,
};
use validation::{
  empty_control_node, issue, reusable_prop_secrets, valid_id, valid_portable_path,
  valid_public_structural_id, valid_reusable_alias, valid_reusable_lifecycle, valid_secret_name,
  valid_sha256,
};
use wait_validation::inspect_durable_wait_contract;
