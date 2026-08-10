# WOML Local Module Runtime v1

Status: frozen and implemented for Module System MS3 on 2026-08-10.

This contract activates the immutable ESM artifacts produced by Module
Compilation v1. Rust remains the workflow and effect authority; Bun evaluates
local JavaScript inside the existing isolated step Worker.

## Definition Package v3

Definition Package v3 promotes an unchanged Package v2 compilation into the
runtime profile. It sets `runtimeReady: true`, retains the Package v2 root as
`compilationRootHash`, and receives a new canonical `rootHash`. Model v9,
bundle bytes, source maps, exports, and their digests do not change during
promotion.

## Script Host protocol v5

Protocol v5 adds two operations without changing Capability Call v1, tracked
Fetch, cancellation, or completion semantics:

1. Rust sends `register_module` with one bundle and its SHA-256 digest. The
   long-lived host verifies and caches immutable bytes by digest, then returns
   `module_registered`.
2. Every `execute` identifies the sorted module aliases, bundle digests, and
   accepted exports needed by that invocation. Unregistered or mismatched
   artifacts fail closed.

Registration contains no source paths, source maps, context, secrets, service
credentials, or mutable workflow state. Frames retain the frozen UTF-8
`Content-Length` framing and eight-MiB transport ceiling. The MS3 activation
profile further limits each bundle and source map to three MiB.

## Worker lifecycle

For every step attempt the host starts a fresh Worker. The Worker:

1. verifies each bundle digest again;
2. installs guarded Fetch and managed-service doorways;
3. evaluates module initialization with effects disabled;
4. verifies every declared export is a function;
5. freezes each `services.<alias>` namespace;
6. runs the inline step with native Fetch and the merged services facade; and
7. terminates at success, failure, timeout, cancellation, or crash.

Module functions are direct local calls, so ordinary JavaScript values remain
in memory until an existing JSON/capability boundary. Module-level state never
crosses Workers, steps, attempts, or runs.

Modules receive no automatic `context`, `attempt`, or `secrets` binding.
Callers pass workflow values and individual secrets explicitly. A module may
use native `fetch` or global built-in `services.*` only while one of its
exported functions is actively executing; top-level effects are rejected.

## Failure and supervision

Module loading and calls share the step attempt's timeout, cancellation,
Worker-crash, host-crash, context-size, result-size, durable capability, retry,
and secret-redaction boundaries. A module exception is a failure of the
calling step. Artifact recovery and source-mapped diagnostics are MS4 work;
MS3 requires the exact Package v3 artifacts to be supplied at activation.
