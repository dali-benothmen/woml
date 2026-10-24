# Remote Execution Operations

Remote execution keeps workflow authority on the coordinator and runs one
integrity-bound step attempt on an authenticated worker. This runbook covers
the operator actions that are safe while work may still be in flight.

## Inspect a target and dispatch

Validate registry and trust material before activation:

```bash
woml target check production-private \
  --registry .woml/targets.json \
  --trust .woml/target-trust.json
```

Use JSON inspection for automation and incident records:

```bash
woml target inspect production-private \
  --registry .woml/targets.json \
  --trust .woml/target-trust.json \
  --json

woml dispatch inspect dispatch_<sha256> --state .woml/state.sqlite --json
```

Dispatch inspection reports the target, bound worker, state, heartbeat time,
output cursor, and ambiguity status without exposing certificates, secrets,
headers, or workflow values.

## Drain a target or worker

Drain before maintenance or certificate rotation. Drain intent rejects new
admission but does not cancel or abandon accepted attempts:

```bash
woml target drain production-private \
  --registry .woml/targets.json \
  --trust .woml/target-trust.json

woml target drain production-runner-01 \
  --registry .woml/targets.json \
  --trust .woml/target-trust.json
```

Wait until active capacity reaches zero, then stop the worker. Preserve its
worker journal until every accepted dispatch has a coordinator terminal fact.

## Rotate a worker certificate

1. Drain the worker and wait for active work to finish.
2. Issue a replacement certificate from the configured worker CA.
3. Update the target-trust certificate reference and worker-local certificate
   and key as one reviewed change.
4. Run `woml target check` and `woml worker check`.
5. Restart the worker and confirm a new authenticated session.
6. Revoke the old certificate only after the old session has expired.

Never copy a private key into the registry, workflow source, CLI arguments,
run state, or an incident ticket.

## Lost worker and target outage

Do not immediately retry an accepted or started attempt after a heartbeat
timeout. WOML marks loss after acceptance as ambiguous because the external
effect may have completed even if its terminal message was lost.

Inspect the dispatch and the worker journal. Restore connectivity when
possible: a trusted late terminal record can reconcile the ambiguity without
replaying the effect. For a complete target outage, drain the target, preserve
coordinator and worker stores, and restore at least one compatible authorized
worker before admitting new work.

## Reconcile an ambiguous attempt

If evidence proves failure, record it explicitly:

```bash
woml dispatch reconcile dispatch_<sha256> \
  --action record-failure \
  --request-id incident-2026-10-24 \
  --reason "Worker disk was recovered; journal proves the process failed"
```

If the effect cannot be proven and a possible duplicate is acceptable,
authorize that risk explicitly:

```bash
woml dispatch reconcile dispatch_<sha256> \
  --action authorize-possible-duplicate-retry \
  --request-id change-4821 \
  --reason "Operation is externally idempotent and the owner approved retry"
```

Reconciliation is durable and audited. WOML never lets an ambiguous result
silently release a lock, settle a run, or enter the ordinary retry path.

## Output and telemetry

Remote output remains in Operational Output Store v1 and keeps normal cursor,
retention, redaction, and truncation behavior. Duplicate worker batches are
deduplicated by dispatch and sequence; reordered or conflicting records fail
closed.

OpenTelemetry uses bounded dimensions and exports remote dispatch queue delay,
execution duration, losses, and ambiguities. Worker IDs and target IDs are not
metric labels, which prevents operator configuration from creating unbounded
series cardinality. Exporter failure never changes dispatch behavior.
