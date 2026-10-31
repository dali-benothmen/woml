//! Run-control identity, cancellation, outcome, and deadline transactions over one store authority.

mod cancellation;
mod identity;
mod outcomes;
mod timeouts;

pub use identity::derive_lifecycle_hook_invocation_id;
