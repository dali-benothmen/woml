# WOML Retry and Idempotency v1

Status: frozen and implemented. RI0–RI7 completed the frontend, Rust runtime,
Bun host, durable recovery, CLI diagnostics, hardening, and packaging gates.

The normative compiled contract is
`docs/schemas/compiled-workflow-model.v6.schema.json`.

## Authoring contract

Retry is a `<step>` attribute. WOML has no `<retry>` element.

```xml
<step
  id="loadProfile"
  retry="4"
  retry-backoff="exponential"
  retry-delay="1s"
  retry-max-delay="30s">
  <script>
    return await loadProfile(attempt.idempotencyKey);
  </script>
</step>
```

`retry` is the maximum total attempt count, including the first attempt. Its
range is 1 through 10. Omission and `retry="1"` both produce no compiled
`retryPolicy` and preserve the older model profile.

Backoff attributes require `retry` greater than 1. The default strategy is
`exponential`, the default initial delay is 1 second, and the default maximum
delay is the greater of 30 seconds and the initial delay. Fixed backoff rejects
`retry-max-delay`. Delays must resolve to a positive whole number of
milliseconds no greater than 24 hours. Exponential backoff uses multiplier 2
and no jitter.

The delay before attempt `n`, for `n >= 2`, is:

```text
fixed:       retry-delay
exponential: min(retry-delay × 2^(n - 2), retry-max-delay)
```

## Retryable failures

The first profile retries only `script_threw`, and only when another compiled
attempt remains. Timeouts, invalid results, size-limit failures, worker crashes,
host crashes, interrupted attempts, and cancellation fail closed because the
status of external effects is ambiguous or retry cannot correct the failure.

Rust owns this decision. Bun reports the attempt outcome but never schedules a
retry.

## Stable step-effect identity

Every Model v6 script receives an immutable `attempt` binding:

```ts
interface WomlAttempt {
  readonly number: number;
  readonly maxAttempts: number;
  readonly idempotencyKey: string;
}
```

The logical effect key is:

```text
sha256(RFC8785({
  contract: "woml.step-effect",
  version: 1,
  runId,
  definitionHash,
  nodeId
}))
```

Its public representation is `sha256:` followed by 64 lowercase hexadecimal
characters. Attempt number and invocation ID are excluded, so every retry of
one logical step shares the key. Other runs and nodes receive different keys.

WOML does not claim exactly-once behavior for arbitrary JavaScript side
effects. An external API must honor the supplied key to deduplicate the effect.
Authors who enable multiple attempts permit the script body to run again after
a definitive retryable failure.

## Durable boundary

The atomic durable write for a retryable failure is:

```text
step_attempt_failed + step_retry_scheduled
```

A scheduled but unstarted retry may resume safely. An attempt with a recorded
start and no terminal event is resolved as `interrupted`; it is never replayed
automatically. A successful step publishes one output, and that node never runs
again.

The stable key is operational metadata. It is not a credential, capability, or
workflow-context value. `context.run` remains undefined and unavailable.

## CLI and recovery contract

Retry progress is operational output and is written to stderr. The final
workflow result remains the only JSON value written to stdout, so scripts may
consume it safely.

The CLI reports the failed attempt, the next attempt number, and the durable
scheduled time. After the first retry is scheduled it also prints a complete
recovery command containing the state path and run ID:

```text
woml run "workflow.woml" --state ".woml/state.sqlite" --resume "run_..."
```

Recovery uses the workflow definition already bound to that run. It resumes a
durably scheduled attempt without replaying completed work. A recorded active
attempt with no terminal outcome is ambiguous and becomes `interrupted`; it is
never retried automatically. Exhaustion is reported as
`WOML_STEP_RETRIES_EXHAUSTED` while preserving the safe underlying failure code
and the authored script location.

## Guarantee boundary

WOML guarantees deterministic attempt identity, durable scheduling, safe
recovery, and one published successful step result. WOML does not claim that an
arbitrary external side effect happens exactly once. The receiving service must
deduplicate `attempt.idempotencyKey`; otherwise authors must treat a retried
script as potentially executing its external effect more than once.
