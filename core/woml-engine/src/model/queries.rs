//! model queries.

use super::{
  approval_notifications, reusable_provider_ids, ApprovalDefinition, CompiledFork,
  CompiledForkBranch, CompiledLifecycleHook, CompiledLock, CompiledSaga, CompiledTrigger,
  CompiledWait, CompiledWorkflowDefinition, CompiledWorkflowNode, LifecycleEventName,
  ParallelGroupDefinition, ValueExpression,
};
use serde_json::Value;
use serde_json_canonicalizer::to_vec as canonical_json;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::collections::HashSet;

impl CompiledWorkflowDefinition {
  pub fn runtime_policy_hash(&self) -> Option<String> {
    let policy = self.runtime_policy.as_ref()?;
    let bytes = canonical_json(policy).ok()?;
    Some(format!("sha256:{:x}", Sha256::digest(bytes)))
  }

  pub fn runtime_policy_queue_name(&self) -> Option<String> {
    let policy = self.runtime_policy.as_ref()?;
    Some(policy.queue.as_ref().map_or_else(
      || {
        let digest = Sha256::digest(self.workflow_id.as_bytes());
        format!("workflow-{:x}", digest)
      },
      |queue| queue.name.clone(),
    ))
  }

  pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
    serde_json::from_str(json)
  }

  pub fn node(&self, node_id: &str) -> Option<&CompiledWorkflowNode> {
    self.graph.nodes.iter().find(|node| node.id == node_id)
  }

  pub fn trigger(&self, trigger_id: &str) -> Option<&CompiledTrigger> {
    self
      .triggers
      .iter()
      .find(|trigger| trigger.id == trigger_id)
  }

  pub fn durable_wait(&self, wait_id: &str) -> Option<&CompiledWait> {
    self
      .graph
      .waits
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|wait| wait.wait_id() == wait_id)
  }

  pub fn durable_lock(&self, lock_id: &str) -> Option<&CompiledLock> {
    self
      .graph
      .locks
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|lock| lock.lock_id == lock_id)
  }

  pub fn saga(&self, saga_id: &str) -> Option<&CompiledSaga> {
    self
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|saga| saga.saga_id == saga_id)
  }

  pub fn saga_for_control_node(&self, node_id: &str) -> Option<&CompiledSaga> {
    self
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|saga| {
        saga.open_node_id == node_id
          || saga.commit_node_id == node_id
          || saga.result_node_id == node_id
      })
  }

  pub fn saga_for_body_node(&self, node_id: &str) -> Option<&CompiledSaga> {
    self
      .graph
      .sagas
      .as_deref()
      .unwrap_or_default()
      .iter()
      .find(|saga| {
        let mut pending = vec![saga.body_entry_node_id.as_str()];
        let mut visited = HashSet::new();
        while let Some(candidate) = pending.pop() {
          if !visited.insert(candidate) {
            continue;
          }
          if candidate == node_id {
            return true;
          }
          if candidate == saga.body_terminal_node_id {
            continue;
          }
          pending.extend(
            self
              .graph
              .edges
              .iter()
              .filter(|edge| edge.from == candidate)
              .map(|edge| edge.to.as_str()),
          );
        }
        false
      })
  }

  pub fn lifecycle_hook(&self, hook_id: &str) -> Option<&CompiledLifecycleHook> {
    self
      .lifecycle
      .as_ref()?
      .hooks
      .iter()
      .find(|hook| hook.hook_id == hook_id)
  }

  pub fn lifecycle_hook_for_event(
    &self,
    event: LifecycleEventName,
  ) -> Option<&CompiledLifecycleHook> {
    self
      .lifecycle
      .as_ref()?
      .hooks
      .iter()
      .find(|hook| hook.event == event)
  }

  pub fn lifecycle_hook_for_step_event(
    &self,
    event: LifecycleEventName,
    step_id: &str,
  ) -> Option<&CompiledLifecycleHook> {
    let hook = self.lifecycle_hook_for_event(event)?;
    if !event.is_step()
      || self
        .node(step_id)
        .or_else(|| {
          self
            .graph
            .for_each
            .iter()
            .flatten()
            .flat_map(|descriptor| descriptor.body.nodes.iter())
            .find(|node| node.id == step_id)
        })
        .is_none_or(|node| node.handler != "runtime.script")
      || hook
        .step_ids
        .as_ref()
        .is_some_and(|step_ids| !step_ids.iter().any(|id| id == step_id))
    {
      return None;
    }
    Some(hook)
  }

  pub(crate) fn approval(&self, approval_id: &str) -> Option<ApprovalDefinition> {
    let wait = self.node(approval_id)?;
    if wait.handler != "engine.approval-wait" {
      return None;
    }
    let ValueExpression::Object { fields } = &wait.inputs else {
      return None;
    };
    let timeout_ms = fields
      .get("timeoutMs")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_u64(),
        _ => None,
      });
    let on_timeout = fields
      .get("onTimeout")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_str().map(str::to_string),
        _ => None,
      })?;
    let review = fields.get("review").cloned();
    let required_approvals = fields
      .get("requiredApprovals")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_u64(),
        _ => None,
      })
      .and_then(|value| u32::try_from(value).ok());
    let required_roles = fields
      .get("requiredRoles")
      .and_then(|expression| match expression {
        ValueExpression::Array { items } => Some(
          items
            .iter()
            .filter_map(|item| match item {
              ValueExpression::Literal { value } => value.as_str().map(str::to_string),
              _ => None,
            })
            .collect(),
        ),
        _ => None,
      })
      .unwrap_or_default();
    let provider_ids = reusable_provider_ids(self);
    let notifications = approval_notifications(approval_id, fields, &provider_ids)?;
    Some(ApprovalDefinition {
      approval_id: approval_id.to_string(),
      name: wait
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string),
      description: wait
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("description"))
        .and_then(Value::as_str)
        .map(str::to_string),
      timeout_ms,
      on_timeout,
      review,
      required_approvals,
      required_roles,
      notifications,
    })
  }

  pub(crate) fn parallel_group(&self, parallel_id: &str) -> Option<ParallelGroupDefinition> {
    let start_node_id = format!("__woml_parallel__{parallel_id}__start");
    let start = self.node(&start_node_id)?;
    if start.handler != "engine.parallel-start" {
      return None;
    }
    let ValueExpression::Object { fields } = &start.inputs else {
      return None;
    };
    let concurrency = fields
      .get("concurrency")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => {
          value.as_u64().and_then(|value| usize::try_from(value).ok())
        }
        _ => None,
      })?;
    let on_error = fields
      .get("onError")
      .and_then(|expression| match expression {
        ValueExpression::Literal { value } => value.as_str().map(str::to_string),
        _ => None,
      })?;
    let mut child_edges = self
      .graph
      .edges
      .iter()
      .filter(|edge| edge.from == start_node_id && edge.parallel_id.as_deref() == Some(parallel_id))
      .collect::<Vec<_>>();
    child_edges.sort_by_key(|edge| {
      edge
        .id
        .strip_prefix(&format!("{parallel_id}:child:"))
        .and_then(|index| index.parse::<usize>().ok())
        .unwrap_or(usize::MAX)
    });
    Some(ParallelGroupDefinition {
      parallel_id: parallel_id.to_string(),
      start_node_id,
      child_node_ids: child_edges.iter().map(|edge| edge.to.clone()).collect(),
      concurrency,
      on_error,
    })
  }

  pub(crate) fn parallel_group_for_child(&self, node_id: &str) -> Option<ParallelGroupDefinition> {
    let parallel_id = self
      .graph
      .edges
      .iter()
      .find(|edge| edge.to == node_id && edge.parallel_id.is_some())?
      .parallel_id
      .as_deref()?;
    self.parallel_group(parallel_id)
  }

  pub(crate) fn fork_branch_owner(
    &self,
    node_id: &str,
  ) -> Option<(&CompiledFork, &CompiledForkBranch)> {
    for fork in self.graph.forks.as_deref().unwrap_or_default() {
      for branch in &fork.branches {
        let mut visited = HashSet::new();
        let mut pending = vec![branch.entry_node_id.as_str()];
        while let Some(candidate) = pending.pop() {
          if !visited.insert(candidate) {
            continue;
          }
          if candidate == node_id {
            return Some((fork, branch));
          }
          if candidate == branch.terminal_node_id {
            continue;
          }
          pending.extend(
            self
              .graph
              .edges
              .iter()
              .filter(|edge| edge.from == candidate)
              .map(|edge| edge.to.as_str()),
          );
        }
      }
    }
    None
  }

  pub fn terminal_node_id(&self) -> Option<&str> {
    let mut outgoing: HashMap<&str, usize> = self
      .graph
      .nodes
      .iter()
      .map(|node| (node.id.as_str(), 0))
      .collect();
    for edge in &self.graph.edges {
      if let Some(count) = outgoing.get_mut(edge.from.as_str()) {
        *count += 1;
      }
    }
    let mut terminals = outgoing
      .into_iter()
      .filter_map(|(id, count)| (count == 0).then_some(id));
    let first = terminals.next()?;
    terminals.next().is_none().then_some(first)
  }
}

impl CompiledWorkflowNode {
  pub fn script_source(&self) -> Option<&str> {
    let ValueExpression::Object { fields } = &self.inputs else {
      return None;
    };
    let ValueExpression::Literal { value } = fields.get("source")? else {
      return None;
    };
    value.as_str()
  }
}
