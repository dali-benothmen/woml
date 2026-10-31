//! Stable lifecycle-hook identity derivation with unchanged domain separation and inputs.

use sha2::Digest;
use sha2::Sha256;

pub fn derive_lifecycle_hook_invocation_id(
  run_id: &str,
  hook_id: &str,
  subject_kind: crate::event::LifecycleSubjectKind,
  subject_id: &str,
) -> String {
  let subject_kind = match subject_kind {
    crate::event::LifecycleSubjectKind::Workflow => "workflow",
    crate::event::LifecycleSubjectKind::Step => "step",
  };
  let canonical = [run_id, hook_id, subject_kind, subject_id]
    .into_iter()
    .map(|part| format!("{}:{part}", part.len()))
    .collect::<Vec<_>>()
    .join("|");
  format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}
