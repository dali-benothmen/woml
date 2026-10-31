//! event policy.

use crate::infrastructure_policy_runtime::InfrastructurePolicyDecisionRecordV1;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InfrastructurePolicyDecisionRecordedData {
  pub invocation_id: String,
  pub call_id: String,
  pub operation_key: String,
  pub decision: InfrastructurePolicyDecisionRecordV1,
}
