//! Native Node/Bun boundary for WOML.
//!
//! This crate deliberately depends on `woml-engine` and adapter libraries
//! only. Retired orchestration paths must never become reachable from here.

// N-API function arity is part of the frozen JavaScript boundary. Audit 1
// preserves it exactly; consolidating parameters would require a protocol bump.
#![allow(clippy::too_many_arguments)]

pub mod bridge;
