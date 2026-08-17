# HTTP Capability Release Gate

SC6 publishes Capability Call v1, Script Host v4, observed native Fetch, and
Managed HTTP v1 as one independently testable foundation. This file maps each
failure boundary to executable evidence; it does not introduce another
protocol.

## Crash and persistence matrix

| Boundary | Required behavior | Executable evidence |
|---|---|---|
| Before `operation_started` persistence | Do not dispatch the external request; return a safe failure. | `capability_authority::durable_authority_commits_terminal_event_before_returning_result` proves the authority owns ordering; invalid request/metadata tests stop before registry dispatch. |
| After persisted start, before or during send | Recovery closes the operation and attempt as `interrupted`, ambiguous, and final. | `capability_authority::recovery_closes_interrupted_managed_operation_as_ambiguous_before_the_attempt`. |
| HTTP server disconnects before a response | Return `WOML_HTTP_TRANSPORT_FAILED`; never persist a body or credential. | `managed_http::managed_http_classifies_timeout_status_and_invalid_json_without_leaking_bodies`. |
| Handler panics | Convert panic to `handler_crashed`; keep the long-lived authority alive. | `capability_authority::registry_enforces_results_sizes_cancellation_crashes_and_concurrency`. |
| Worker crashes or is cancelled with calls active | Emit one terminal invocation outcome, cancel active calls, and discard known late replies. | Script Host tests `Worker crash racing with cancel` and `cancellation drops a known late reply`. |
| Bun host exits during an attempt | Rust reports `host_crashed`; retry/recovery policy fails closed. | `script_host_protocol`, `retry_hardening`, and Script Host crash fixtures. |
| Terminal event persistence | Persist the terminal operation before releasing the result to Bun. | `capability_authority::durable_authority_commits_terminal_event_before_returning_result`. |
| Terminal persisted, reply lost | Recovery observes a terminal operation, never dispatches it again, and closes only the interrupted script attempt. | Capability duplicate-correlation and RI7 interrupted-attempt recovery tests. |
| Process stops after a successful step | Fold the durable result and continue only downstream work. | `durable_event_store`, `parallel_recovery`, and CLI native recovery tests. |
| Oversized/malformed response | Fail with a bounded typed error without exposing the provider body. | `managed_http` oversized, rejected-status, and invalid-JSON cases. |

The durable authority is deliberately ordered:

```text
validate safe request metadata
  -> append operation_started
  -> dispatch handler
  -> append operation_succeeded / operation_failed
  -> release correlated reply to Bun
```

There is no supported code path that sends before the start append or replies
before the terminal append. An unclosed start is therefore the only durable
ambiguity and recovery handles it explicitly.

## Composition matrix

The SC6 frontend conformance suite injects native Fetch and managed HTTP into
the reviewed retry/branch/parallel, approval, webhook, Slack, schedule,
interval, and named-event fixtures. It asserts Model v8 and Script Bindings v1
without changing their structural contracts. The public CLI additionally runs
manual + retry + branch + parallel with both HTTP paths against one deterministic
server. Existing approval and Production Trigger runtime suites exercise the
same Model v8 script runtime after their control boundary selects a node.

## Release commands

From `woml-cli`:

```bash
bun run test:http
bun run benchmark:http
```

`test:http` includes the transitive T13/N6 gate: clean build, frontend tests,
Rust tests and lint, isolated CLI suites, clean-package execution, crash and
recovery suites, compatibility checks, type checks, and secret scanning. The
clean-package test copies and executes `examples/httpComparisonWorkflow.woml`
against a local server using the installed package's native Rust binary.
