# WOML Run Events v3

Status: frozen for the parallel profile

The normative schema is `docs/schemas/run-event.v3.schema.json`. Event schemas
v1 and v2 remain immutable; one run never mixes schema versions.

Version 3 adds two durable control events:

- `parallel_group_started` records the fork point. Every child receives the
  context obtained by folding the event prefix before this event.
- `parallel_group_completed` records the group outcome plus failed and
  engine-cancelled child IDs in compiled document order.

Only successful step-attempt events publish `context.steps` values. Parallel
events never publish `context.steps.<parallelId>`.

`wait-all` completes after every child becomes terminal. `fail-fast` stops
scheduling unstarted children, cancels active siblings, and completes only
after every started attempt has a terminal event. Unstarted children appear in
neither failure list. Cancellation, interruption, Worker crash, and host crash
remain distinct outcomes.

Parallel run failure uses `failureScope: "parallel"`, records the policy,
primary failing child, failed children, cancelled children, and stable code
`WOML_PARALLEL_CHILD_FAILED`.

The reviewed success, wait-all failure, and fail-fast failure histories live
under `woml/tests/fixtures/run-events/`.
