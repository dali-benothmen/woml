# WOML Run Events v6

Status: frozen, implemented, and publishable as part of the RI7 retry profile.

The normative schema is `docs/schemas/run-event.v6.schema.json`. Event schemas
v1 through v5 remain immutable, and one run never mixes schema versions.

Version 6 changes two event shapes:

- `step_attempt_started` now requires the stable step-effect
  `idempotencyKey`.
- `step_retry_scheduled` is now executable and records `nodeId`,
  `failedAttempt`, `nextAttempt`, and the absolute RFC 3339 `scheduledAt` time.

The attempt failure and retry schedule are one atomic persistence decision.
`nextAttempt` must equal `failedAttempt + 1`, attempt numbers must be contiguous,
and the next attempt must not start before `scheduledAt`.

Only `script_threw` may be followed by `step_retry_scheduled` in the first
profile. A final or non-retryable failure is committed atomically with the
applicable run or parallel terminal event instead.

Recovery rules are deliberately asymmetric:

- A durable schedule with no next-attempt start is safe to resume.
- A started attempt with no terminal event is resolved as `interrupted` and the
  run fails closed.
- A succeeded step is never replayed.
- A final failed attempt never schedules an attempt beyond `maxAttempts`.

The reviewed histories are:

- `retry-success.events.v6.json`
- `retry-exhausted.events.v6.json`
- `retry-scheduled-recovery.events.v6.json`
- `retry-ambiguous-recovery.events.v6.json`

All live under `woml/tests/fixtures/run-events/`.
