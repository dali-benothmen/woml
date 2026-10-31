# Compensation and Cleanup Security

Compensation can repeat sensitive effects such as refunds, deletion, access
revocation, and infrastructure teardown. Treat it as production authority, not
an error-handler convenience.

## Trust boundaries

- TypeScript validates syntax, visibility, limits, and the corrective plan.
- Rust independently validates Model v19 and exclusively owns scheduling,
  attempts, event history, cancellation, recovery, and settlement.
- Bun executes one isolated corrective script attempt with only compiled
  context, declared secrets, and approved capabilities.
- Operators may retry or externally resolve only an action already in manual
  recovery; the CLI cannot select arbitrary workflow code.

## Protected invariants

1. Forward success and compensation registration are atomic.
2. Unknown corrective outcomes are ambiguous and never replay automatically.
3. External resolution does not fabricate a success event.
4. The original business failure remains primary.
5. Cleanup cannot be skipped by cancellation or a failed compensation.
6. Corrective code receives no broader capabilities or secrets than its
   workflow.
7. Events, inspection, metrics, and recovery results exclude raw secrets,
   payloads, credentials, operation keys, and raw operator notes.
8. Future model, event, store, script-host, and inspection versions fail
   closed.

## Primary risks and controls

| Risk | Control |
| --- | --- |
| Duplicate refund or deletion after a crash | Started-without-outcome becomes manual recovery; no automatic replay |
| Wrong resource corrected | Stable saga/action identity and original forward result visibility |
| Operator resolves the wrong action | Exact run/action match, current-state validation, single-writer transaction |
| Secret copied into diagnostics | Redacted failure contracts, bounded inspection, static secret discovery |
| Malicious history changes ordering | Full event/model validation before folding; invalid history fails closed |
| Infinite retry or recovery storm | Ten-attempt action cap, 100-record scheduler batch, durable policy claims |
| Retention removes evidence | Runs with active correction or manual recovery are protected |
| Two runtimes execute the same correction | Workflow ownership filter plus the existing durable policy claim |
| Cleanup omitted after failure | Runtime continues through corrective failures and settles cleanup explicitly |
| Corrective code borrows a forward or stale attempt's authority | Exact active run/saga/action/attempt/invocation match and verified operation-key derivation |
| Recovery closes a correction while its operation remains open | Operation terminals commit before corrective interruption/manual recovery, in one transaction |

## Operator notes

External-resolution notes may contain ticket context but must never contain a
credential, payload, card number, access token, or personal record. WOML
validates length and control characters and stores only a SHA-256 digest. Put
the full investigation in the organization's approved incident or change
system.

## Review checklist

- Does each compensation use a precise resource ID rather than a broad query?
- Is the external API operation idempotent or protected by WOML attempt
  identity?
- Can a stale owner still mutate the resource? Use a surrounding lock and
  fencing token when necessary.
- Does cleanup remain safe when forward work only partly completed?
- Are all returned values bounded and free of credentials?
- Can operators verify the external state before choosing retry or resolve?
