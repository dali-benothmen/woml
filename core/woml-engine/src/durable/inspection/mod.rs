//! Bounded operator read models without workflow scheduling or new authority.
//!
//! Run, approval, wait, lock, and corrective views retain their original limits,
//! versioned shapes, projection validation, and credential/payload exclusions.

mod approvals;
mod correction;
mod locks;
mod runs;
mod waits;
