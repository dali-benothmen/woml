# WOML Run Events v8

Status: frozen by SC0 on 2026-08-09. Event versions v1-v7 remain immutable.

Run Event v8 retains the complete v7 vocabulary and adds:

```text
operation_started
operation_succeeded
operation_failed
```

The vocabulary is generic. `http.request`, future database calls, storage,
cache, event publication, queues, native Fetch, and custom capabilities do not
add event types.

## Fold rules

For each `{runId, invocationId, callId}`:

1. the run and matching step attempt must already be active;
2. exactly one `operation_started` is allowed;
3. at most one terminal operation event is allowed;
4. terminal identity, node, attempt, capability, method, mode, and operation
   key must equal the start;
5. IDs may not cross runs or invocations;
6. a managed operation still active prevents `step_attempt_succeeded`;
7. a terminal step prevents later operation events; and
8. recovery of a managed start without a terminal event marks the step attempt
   ambiguous unless a handler-specific contract proves the outcome.

Observed native Fetch follows the same structural fold. Its behavior remains
native; the event is an audit observation, not a replayable result cache.

Operation events never publish values to `context.steps`. Only the surrounding
`step_attempt_succeeded` event publishes the script result. Success events keep
only a byte count and digest; failures keep the shared bounded, redacted
Capability Call v1 failure.

If that failure escapes the script, the terminal attempt and run use Attempt
Failure v3 `service_failed`, which points back to the same call and retains the
same safe cause. This is a classification bridge, not a second service failure
taxonomy.
