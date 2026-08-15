//! Temporary compatibility shim for the combined legacy `core` addon.
//!
//! The canonical WOML N-API adapter now belongs to `core/woml-native`. The old
//! addon includes the same source until the CLI switches to the dedicated
//! artifact in Audit 2. Do not add WOML implementation code to this shim.

include!("../woml-native/src/bridge.rs");
