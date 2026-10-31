# Compensation and Cleanup

Use Compensation and Cleanup when a workflow performs several external
effects and later work can fail. WOML records which effects definitely
succeeded, runs their explicit compensations in reverse completion order, and
then runs mandatory cleanup before the saga settles.

This is a saga, not a distributed transaction. WOML can request a refund,
delete a preview database, release inventory, or revoke access. It cannot prove
that every outside system returned to an identical earlier state.

## First saga

```xml
<saga id="checkout" name="Complete checkout">
  <step id="capturePayment" name="Capture payment">
    <script>
      return { paymentId: "payment-42", amount: 149 };
    </script>

    <compensate timeout="30s" retry="5">
      <script>
        return {
          refundedPayment: context.steps.capturePayment.paymentId,
          idempotencyKey: attempt.idempotencyKey
        };
      </script>
    </compensate>
  </step>

  <step id="commitOrder" name="Commit order">
    <script>
      if (context.payload.simulateFailure === true) {
        throw new Error("order commit failed");
      }
      return { orderId: "order-42" };
    </script>
  </step>

  <cleanup>
    <step id="closeCheckoutSession" name="Close checkout session">
      <script>return { closed: true };</script>
    </step>
  </cleanup>
</saga>
```

If `commitOrder` fails, WOML compensates `capturePayment` and then closes the
checkout session. If all forward work succeeds, no compensation runs, but
cleanup still runs. A later step consumes the saga's forward result as
`context.steps.checkout`.

## Language rules

### `<saga>`

`<saga>` is a flow item accepted anywhere its contained flow is supported. It
requires a stable lower-camel-case `id` and accepts optional `name` and
`description`. Its body contains one or more forward flow items followed by an
optional final `<cleanup>`.

- Sagas cannot be nested.
- A fork inside a saga must join every branch; detached work is rejected.
- A committed saga is not reopened because a later step outside it fails.
- Selected choices, switches, parallels, joined forks, and `for-each`
  iterations register only the corrective work that actually became eligible.

### `<compensate>`

A forward `<step>` inside a saga may have one trailing `<compensate>` after its
`<script>`. It contains exactly one `<script>` and accepts:

| Attribute | Meaning |
| --- | --- |
| `timeout` | Maximum duration for one corrective attempt. |
| `retry` | Total attempts from 1 through 10. |

Successful forward output and its compensation registration are written in
one durable transaction. A step that did not definitely succeed registers
nothing. Compensation executes in reverse durable completion order, which is
also well-defined for concurrent work.

### `<cleanup>`

`<cleanup>` is the final child of a saga and contains one or more sequential
steps. Cleanup runs in source order after successful forward work, failed
forward work, cancellation, and compensation failure. Cleanup steps use the
normal step `timeout` and retry attributes.

Corrective bodies cannot contain waits, approvals, nested sagas, or multi-step
compensation in v1. These shapes are rejected during `woml check`.

## Corrective script bindings

A compensation script receives normal visible context, `attempt`, services,
and referenced secrets plus:

```js
context.compensation.sagaId
context.compensation.forStepId
context.compensation.reason // failure | cancel
```

A cleanup script receives:

```js
context.cleanup.sagaId
context.cleanup.reason
context.cleanup.compensationStatus
```

Managed services in these scripts use the corrective action's own durable
attempt identity—not the forward step's identity. Calls remain subject to the
same operator capabilities, infrastructure policies, and live lock ownership.
Cancellation still permits authorized compensation and cleanup, without
reopening forward work. A denied or unsupported operation is not implicitly
allowed just because it is corrective.

If a corrective script is interrupted, WOML settles any open operation records
before placing the action in manual recovery. Even proof that one state change
committed does not prove the whole script finished. Inspect the real system
before explicitly retrying or resolving the action.

The bindings are deeply read-only and exist only for that invocation. They do
not become durable workflow output. Use the original forward step result to
identify the exact resource being corrected, and use
`attempt.idempotencyKey` when the outside API supports idempotency.

## Durable execution model

Rust owns saga state, registration, ordering, attempts, retries, cancellation,
recovery, and settlement. The compiled Model v19 contains a separate
corrective plan; correction is not represented as a hidden branch in the
forward DAG. Event v18 is authoritative. Store v17 indexes are bounded and
rebuildable.

When forward work fails or cancellation is requested:

1. no new forward saga work is admitted;
2. active owned work settles or is cancelled;
3. eligible compensations run in reverse durable completion order;
4. every cleanup step runs in source order;
5. the original failure or cancellation remains the business outcome; and
6. the run finalizes only after correction is complete or explicitly requires
   manual recovery.

Correction continues after one corrective action fails so cleanup is never
silently skipped. A corrective failure is reported separately and does not
replace the original business failure.

## Crash recovery and ambiguity

If WOML recorded that a corrective attempt started but cannot prove its
outcome, it marks the action ambiguous and requires manual recovery. It never
blindly repeats a possibly completed refund, deletion, or revocation.

Inspect the run:

```bash
woml get run_82fc
woml get run_82fc --json
```

The correction section shows the original outcome, saga phase, completed
compensations, cleanup progress, attempts, safe diagnostic codes, and bounded
unresolved actions.

After verifying the outside system, choose exactly one action:

```bash
# The effect definitely did not happen; schedule another supervised attempt.
woml recover run_82fc --retry 'checkout:compensate:capturePayment'

# The operator completed or verified the effect outside WOML.
woml recover run_82fc \
  --resolve 'checkout:compensate:capturePayment' \
  --note 'Refund verified in payment console under ticket PAY-42'
```

External resolution is reported as `externally_resolved`; WOML never invents a
successful script-execution event. The note is validated and reduced to a
digest before durable storage, so ticket context can be correlated without
persisting raw operator text. An active runtime discovers the decision in a
bounded workflow-owned scan; otherwise the next activation resumes it.

## Cancellation

`woml cancel <runId>` stops new forward work, propagates cancellation through
supervised operations, compensates completed eligible effects, runs cleanup,
and only then settles the run as cancelled. Cancellation is not an unsafe
shortcut around cleanup.

## Composition

- **Parallel:** all active children settle before correction begins;
  compensation follows reverse durable completion, not source order.
- **Choose and switch:** unselected routes register nothing.
- **Fork:** every branch inside a saga must join.
- **For-each:** every successful iteration owns a stable corrective identity;
  compensation follows reverse completion while inspection retains iteration
  identity.
- **Locks:** place `<saga>` inside `<lock>` when the lease must remain held
  through correction and cleanup. A lock nested inside the saga can be
  released before later correction and is not silently reacquired.
- **Workflow calls:** WOML does not infer how to undo a child workflow. Declare
  a compensation that invokes a reviewed corrective workflow or service.

## Limits

| Boundary | v1 limit |
| --- | ---: |
| Sagas per workflow | 64 |
| Compensation registrations per run | 10,000 |
| Cleanup steps per saga | 64 |
| Corrective attempts per action | 10 |
| Unresolved recovery rows per store | 10,000 |
| Inspection rows | 100 |
| Inspection response | 1 MiB |
| Scheduler recovery batch | 100 |

These limits fail closed rather than truncating executable work. Inspection
explicitly reports truncation where its public contract permits it.

## Production checklist

- Make every compensation narrowly target the identifier returned by its
  forward step.
- Use stable idempotency keys with payment, provisioning, and messaging APIs.
- Set corrective timeouts and retries independently from forward attempts.
- Keep secrets out of results, exceptions, notes, logs, and resource names.
- Use `woml get` after any correction warning; never assume a red terminal line
  means an external effect did or did not occur.
- Back up the durable store; unresolved correction records are protected from
  retention.
- Monitor correction metrics and the age of the oldest manual-recovery item.
- Rehearse failure, cancellation, restart, retry, and external-resolution paths
  before production.

See the [operations runbook](woml-compensation-operations.md), [security and
threat model](woml-compensation-threat-model.md), and [examples](../examples/compensationAndCleanup/).
