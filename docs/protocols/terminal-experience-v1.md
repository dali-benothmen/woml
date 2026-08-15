# WOML Terminal Experience Contracts v1

Status: Frozen by TM0 and implemented through TM5. The renderer, durable Run
Presentation projection, foreground integration, interactive manual admission,
and secure retained/live background log following are implemented. Complex
control-flow presentation remains TM6 work.

This document freezes the interfaces shared by the future Rust projection,
the Bun CLI renderers, manual-trigger input, and background log following. It
does not make terminal text part of workflow execution truth.

## 1. Sources of truth

- The frozen compiled workflow definition owns workflow and authored-node
  metadata.
- The durable Event v13 history and its canonical fold own execution status,
  timing, attempts, selections, results, failures, and lifecycle settlement.
- `woml.run-presentation/v1` is a bounded, read-only projection of those two
  sources.
- Terminal text, `.woml/logs/runtime.log`, ANSI colors, and in-memory renderer
  state are never execution or recovery authority.
- TM0 does not require Event v14. TM2 must stop for review if an approved field
  cannot be derived from the frozen definition and Event v13.

The normative schemas are:

- `docs/schemas/run-presentation.v1.schema.json`
- `docs/schemas/run-presentation-list.v1.schema.json`
- `docs/schemas/manual-trigger-admission.v1.schema.json`

## 2. Run Presentation v1

Every snapshot uses `profile: "woml.run-presentation/v1"` and represents one
run. A snapshot contains:

- the exact workflow identity and trigger declarations from the run's frozen
  definition;
- the admitted trigger identity;
- run status and durable timestamps;
- authored presentation items in stable source order;
- business-step totals;
- lifecycle work separately from business work;
- a final result or final failure only when durably available; and
- bounded, non-sensitive warnings.

Internal scheduler tasks, compiler-generated nodes, join barriers, and storage
records do not become step rows. Authored control-flow operations may become a
row when they communicate a useful selection or outcome.

`depth` communicates authored visual nesting. It does not define execution
dependencies. The compiled DAG remains authoritative.

`STEPS COMPLETED` is derived from business-node settlement. `RUN COMPLETED`
is derived only after required lifecycle finalization settles. A renderer must
not infer either fact from elapsed time or an empty progress stream.

Lifecycle hook names cover the engine's complete existing vocabulary:
`on-start`, `on-success`, `on-failure`, `on-cancel`, `on-complete`,
`on-step-start`, `on-step-success`, `on-step-failure`, and
`on-step-complete`.

Historical snapshots use the definition bound to the run, never a currently
edited `.woml` file.

## 3. Presentation bounds and safety

The frozen v1 budgets are:

- at most 2 MiB of UTF-8 JSON for one complete Run Presentation snapshot;
- at most 10,000 authored presentation rows, 1,000 lifecycle rows, and 1,000
  warnings, with the smaller encoded-byte limit taking precedence;
- authored short text at most 2,048 characters and messages at most 8,192;
- each renderer result preview at most five nested levels, 20 properties per
  object, 20 items per array, 500 characters per string, and 2,000 visited
  values; and
- human rendering widths clamped to 32–160 columns.

Exceeding the snapshot budget is a projection/transport error, not permission
to emit a partial value that still claims to be a complete v1 snapshot. A
shortened result inside an otherwise valid snapshot is explicitly marked with
`resultTruncated: true`.

The TM1 renderer additionally:

- strips ANSI, OSC, cursor-control, and unsafe control characters from authored
  strings and returned values;
- redacts values under credential-shaped object keys such as `password`,
  `secret`, `token`, `authorization`, and `apiKey`;
- structurally bounds depth, property count, array count, and string length;
- never modifies the durable result merely to shorten its display; and
- labels shortened results and provides a full-result inspection command.

Capability URLs, approval tokens, secret props, credentials, idempotency
material, and internal administration tokens are forbidden from human, plain,
and JSON presentation surfaces.

## 4. Renderers

One presentation snapshot has three renderings:

- `tty`: Unicode layout with semantic color when supported;
- `plain`: deterministic human-readable text without ANSI; and
- `json`: one sanitized `woml.run-presentation/v1` JSON value without prose or
  ANSI. Live following uses one value per line.

Color defaults to `auto`. `NO_COLOR`, `TERM=dumb`, redirected output, and
`--color=never` disable it. `--color=always` is valid only for the human TTY
renderer. Meaning must never depend on color alone; every status has text and
an icon or ASCII marker.

The renderer adapts to 32–160 columns, wraps descriptions, aligns durations
when space permits, and uses the authored name with the authored ID as the
fallback. Missing optional metadata is omitted cleanly.

Foreground `woml run` accepts `--json`, `--verbose`, and
`--color=auto|always|never`. Human mode prints an immediate admission receipt
and one atomic durable block after settlement. JSON mode emits only reviewed
workflow metadata and `woml.run-presentation/v1` snapshots on stdout; warnings
and operational diagnostics remain on stderr. Raw trigger, scheduler, retry,
provider, and lifecycle chatter is hidden by default and available through
`--verbose` or structured background logging.

The semantic palette is:

| Meaning | Color |
| --- | --- |
| identity and result markers | cyan |
| succeeded and ready | green |
| running and finalizing | blue |
| waiting, retrying, and warnings | yellow |
| queued | magenta |
| failed and timed out | red |
| skipped, cancelled, and secondary text | dim/default |

## 5. Manual Trigger Admission v1

Normal `woml run` registers `<manual>` as an active trigger. It does not create
a startup run.

For one available target, an empty submitted line means: admit one manual
occurrence for that workflow and trigger. For several targets, the user enters
the displayed target number. `--trigger <id>` may reduce one workflow with
several manual triggers to the one-key experience.

The CLI sends one `request` value through the Rust runtime admission boundary.
The payload is exactly `{}`. Rust returns one correlated `accepted` or
`rejected` value. An accepted response contains the durable occurrence ID and
run ID. Each later Enter action gets a new request and occurrence identity;
normal concurrency, queue, rate-limit, timeout, and idempotency policies still
apply.

The CLI owns keyboard interpretation only. Rust owns admission, durable
identity, policy decisions, execution, cancellation, and recovery.

`woml test` remains the non-interactive one-shot surface. A foreground
manual-only runtime without a usable TTY fails before activation. A
manual-only background runtime is rejected because it has no ingress.

Stable manual diagnostics are:

- `WOML_MANUAL_TRIGGER_TTY_REQUIRED`
- `WOML_MANUAL_TRIGGER_SELECTION_REQUIRED`
- `WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE`
- `WOML_MANUAL_TRIGGER_ADMISSION_CLOSED`
- `WOML_POLICY_QUEUE_FULL` when the shared durable policy queue has reached its
  hard safety ceiling

Ctrl+C closes manual input, closes ingress, and starts the existing graceful
drain. It does not fabricate a run.

## 6. Background log following

The frozen commands are:

```bash
woml <run-id> --logs
woml <workflow-id> --logs
```

A run subject shows all retained presentation history, follows while the run
is active, and exits after terminal settlement. A workflow subject shows at
most the ten most recent retained runs before following future matching runs.
If no owned runtime is active, retained history is rendered and the command
exits with a clear note.

The viewer reconstructs snapshots from durable state and uses the authenticated
Operations Stream only as a change notification. It never parses
`.woml/logs/runtime.log` into workflow state.

Ctrl+C stops the viewer only. It does not stop the background runtime, close
workflow ingress, or cancel a run. Runtime ownership remains with `woml stop`.

Stable viewer diagnostics are:

- `WOML_LOG_SUBJECT_INVALID`
- `WOML_LOG_RUN_NOT_FOUND`
- `WOML_LOG_WORKFLOW_NOT_FOUND`
- `WOML_LOG_HISTORY_PRUNED`
- `WOML_LOG_RUNTIME_UNAVAILABLE`
- `WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED`

The native projection boundary additionally uses
`WOML_RUN_PRESENTATION_SIZE_LIMIT`, `WOML_RUN_PRESENTATION_LIMIT_INVALID`, and
`WOML_RUN_PRESENTATION_FAILED`. A missing individual run retains the existing
`WOML_RUN_NOT_FOUND` diagnostic.

The active runtime exposes the same bounded projection through its existing
loopback-only, capability-authenticated administration listener:

- `GET /v1/presentations/runs/<run-id>` returns one
  `woml.run-presentation/v1` snapshot; and
- `GET /v1/presentations/workflows/<workflow-id>?limit=10` returns one
  `woml.run-presentation-list/v1` value, with `limit` restricted to `1..10`.

These routes read the durable store through the Rust projection. They do not
parse runtime logs, expose a public network listener, or become a second source
of workflow truth.

Operational logging stays separate and ANSI-free. Background startup prints
copyable `--logs` commands but does not persist decorative terminal frames.

## 7. Compatibility

- Model v14, Event v13, Store v14, and existing execution protocols are not
  changed by TM0 or TM1.
- TM3 replaces legacy foreground progress strings with the versioned renderer;
  it does not change workflow execution semantics or durable events.
- Existing JSON commands retain their current shapes. Run Presentation v1 is a
  new profile rather than a silent mutation of Run Inspection v5.
- Permanent files, symbols, tests, and package commands use product names,
  never milestone shorthand such as `tm1_*`.
