# WOML Runtime Policies v1 Contracts

Status: frozen by RP0 on 2026-08-11. RP1 compiles `<config>` into Model v12;
policy execution remains deliberately unavailable until RP2 and RP3.

## Frozen boundaries

- Runtime Policy v1 normalizes WOML source into language-neutral concurrency,
  timeout, rolling-window rate-limit, and work-conserving FIFO queue data.
- Compiled Workflow Model v12 stores that policy outside the business DAG.
- Definition Package v7 carries Model v12 and unchanged local-module artifacts
  while `runtimeReady` remains false during RP1.
- Run Event v11 separates `run_admitted` from one
  `run_execution_started` fact and reserves `run_timeout_reached`.
- Store v12 owns transactional queue indexes, rate indexes, scheduler claims,
  and Run List v2 summaries. Events and immutable definitions remain the run
  authority; indexes/summaries are rebuildable.
- Scheduler Claim v1 is expiring coordination state, never proof that a script
  or external effect completed.
- Run Inspection v3 and Runtime Policy Progress v1 expose safe policy state
  without payloads, secrets, claim owners, or internal operation identities.

## Source and normalization

`<config>` is an optional singleton before `<lifecycle>`, `<triggers>`, and
`<steps>`. It is data-only and requires at least one attribute:

```xml
<config concurrency="4" timeout="10m" rate-limit="100/1m" queue="orders" />
```

- concurrency and rate counts are integers from 1 through 1,000,000;
- policy durations resolve to whole milliseconds from 1 ms through 365 days;
- a queue name is at most 128 characters and uses lowercase dot, underscore,
  or kebab segments; and
- unknown attributes, child content, interpolation, and secrets are invalid.

The frontend emits milliseconds, `rolling_window`, and
`work_conserving_fifo`; Rust never parses WOML duration/rate syntax.

## Policy scope

Concurrency and rate history key on the selected state location plus workflow
ID. Definition versions do not reset them. Every run retains its admitted
definition/policy identity. Conflicting active definitions for one workflow ID
must fail activation rather than enforce competing limits.

The queue name selects a scheduling lane. It does not make different workflow
IDs share concurrency capacity. The local scheduler may skip a blocked item to
start the oldest eligible item in the same lane.

## Execution slots and durable waits

Slots are held only while business/lifecycle work actively executes. Approval,
retry backoff, synchronous child-workflow waiting, and policy queueing release a
slot. Resumption must reacquire a slot. Rate capacity is consumed once at the
first execution start, never at resumption.

Claim/renew/release is transactional. An expired claim allows scheduler
recovery but never authorizes replay of an ambiguous started attempt.

## Timeout

The timeout deadline is `first execution started at + timeoutMs`. Policy queue
delay is excluded; later approval, retry, and child-workflow waits are included.
When timeout wins before a business outcome, Event v11 records
`run_timeout_reached`, then the run decides failure with
`WOML_WORKFLOW_TIMED_OUT`, runs `on-failure`, and finally `on-complete`.

A business outcome committed first cannot be rewritten by a later timer.
Timeout is failure and remains distinct from operator cancellation.

## Admission and recovery

Every Model v12 ingress uses one transaction to bind trigger occurrence,
definition, policy, queue, run identity, and `run_admitted`. The folded public
status is `queued` until `run_execution_started` commits. An immediately
eligible run still records both facts.

Recovery derives queued runs and rate history from Event v11. Store v12 may
rebuild queue/summary indexes. Scheduler leases are expired/reconciled using
the live owner boundary. Duplicate ingress observes the original run.

The bounded local queue ceiling uses one frozen fail-closed mapping:

| Ingress | `WOML_POLICY_QUEUE_FULL` behavior |
| --- | --- |
| Manual | Exit 1 with an actionable diagnostic. |
| Webhook | HTTP 503 with `Retry-After`; no run ID is invented. |
| Slack | Do not acknowledge, allowing reviewed provider redelivery. |
| Schedule/interval | Do not advance the occurrence cursor. |
| Named event | Mark that subscriber delivery retryable; preserve fan-out deduplication. |
| Workflow Call/Start | Return a retryable managed-operation failure. |

No run is created when the safety ceiling itself prevents admission. The
existing occurrence/source identity remains the deduplication key for a retry.

## Compatibility and RP1 staging

Models v1-v11, Events v1-v10, Store v11, Run List v1, and Run Inspection v2
remain immutable. RP1 emits Model v12 only when source contains `<config>`.
`woml check` can review it, including Definition Package v7 for local modules.
`woml run` and `woml test` return `WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE`
until the Event v11/Store v12 authority and scheduler are implemented.

`<config queue>` is run-admission policy. It does not implement the postponed
`services.queue` message producer/consumer feature.
