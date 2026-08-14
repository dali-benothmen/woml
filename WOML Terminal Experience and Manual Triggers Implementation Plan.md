# WOML Terminal Experience and Manual Triggers Implementation Plan

Status: TM0, TM1, and TM2 completed on 2026-08-14; TM3 and TM4 completed on
2026-08-15. The versioned presentation contracts, pure color/plain/JSON
renderer, durable Rust projection, native/TypeScript query boundary,
professional foreground output, and Enter-driven durable manual trigger are
implemented. TM5 is next and adds background log following.

## 1. Product Outcome

This milestone gives `woml run` a polished terminal experience and makes
`<manual>` a real user-operated trigger.

After TM0-TM8 are complete, an author can:

- run a workflow and immediately understand its name, description, version,
  triggers, and current readiness;
- see every run as one organized block with its run ID and local date;
- see author-facing step names, descriptions, types, durations, statuses, and
  bounded results;
- understand selected switches, choices, forks, parallel work, approvals,
  retries, skipped work, and failures without reading internal engine logs;
- see workflow lifecycle execution after business steps settle;
- press Enter to create a real manual-trigger occurrence while the workflow
  remains active;
- press Enter again to create another independent run;
- start a runtime with `--background` and later follow one run or one
  workflow's execution from another terminal;
- leave a log-following session with Ctrl+C without stopping or cancelling the
  background runtime; and
- retain machine-readable output through `--json` without colors or decorative
  text.

The normal commands remain simple:

```bash
woml run workflow.woml
woml run workflows/ --background
woml run_8f21c4 --logs
woml order-processing --logs
```

This milestone does not create a second execution engine or make terminal text
authoritative. Rust remains the durable workflow authority. The CLI renders a
versioned, read-only presentation projection derived from the exact compiled
workflow definition and its durable event history.

## 2. Approved Product Experience

### 2.1 Workflow and trigger header

The default foreground TTY output begins with the workflow identity. Optional
metadata is omitted cleanly rather than replaced with `unknown`.

```text
╭─ WOML WORKFLOW ──────────────────────────────────────────────────────╮
│ Order Processing                                             v1.4.0 │
│ Validates an order, calculates its total, and selects delivery.     │
╰──────────────────────────────────────────────────────────────────────╯

TRIGGER

  ● Manual
    Press Enter to start a run
    Press Ctrl+C to stop the workflow
```

For a webhook, the trigger section contains the real method, address, and a
copyable example:

```text
TRIGGER

  ● Webhook
    POST  http://127.0.0.1:3000/webhooks/orders

    Try it
    curl --request POST 'http://127.0.0.1:3000/webhooks/orders' \
      --header 'Content-Type: application/json' \
      --data '{"orderId":"order-42"}'

  ● Listening for requests · Press Ctrl+C to stop
```

Schedule, interval, event, and Slack triggers use the same section with their
meaningful activation details:

- schedule: expression, timezone, and next due time;
- interval: interval duration and next due time;
- event: event name, authenticated endpoint, and copyable request example;
- Slack: workspace, event type, and configured channel/conversation scope; and
- manual: keyboard instructions and available target selection.

Several triggers appear as several aligned entries under one `TRIGGERS`
heading. The CLI never hides a trigger merely because another trigger also
exists.

### 2.2 Run and step output

Every occurrence creates one appended run block. Step numbers, icons, names,
durations, descriptions, and results use stable columns.

```text
RUN  run_8f21c4                              14 Aug 2026 · 10:42:18

  01  ✓  Prepare order                                      18 ms
          Normalize and validate the submitted order.
          → { orderId: "order-42", customer: "Dali", items: 3 }

  02  ✓  Calculate total                                    31 ms
          Reusable step · Calculate taxes and final price.
          → { subtotal: 120, tax: 24, total: 144 }

  03  ✓  Select delivery                                     4 ms
          Switch · Selected case "express".
          → { provider: "express", estimatedDays: 1 }

  04  ✓  Build confirmation                                 12 ms
          Generate the final customer response.
          → { message: "Order order-42 was accepted", total: 144 }


STEPS COMPLETED

  Duration    65 ms
  Steps       4 succeeded · 0 failed · 0 skipped
```

The renderer uses the author-provided `name`. When `name` is absent, it uses
the authored ID exactly. It prints `description` only when present. It does not
humanize internal compiler IDs or show synthetic join nodes.

Small results remain on one line. Larger results use indented JSON. Results
that exceed the display budget are summarized without changing the stored
result:

```text
          → Result
            {
              "records": [250 items],
              "nextPage": "page-2"
            }

            Complete result: woml get run_8f21c4 --json
```

### 2.3 Lifecycle and final outcome

Lifecycle is shown only when the workflow declares hooks or produces lifecycle
warnings. It appears after business steps because lifecycle finalization is a
separate phase of the run.

```text
LIFECYCLE

  ✓  on-success                                              6 ms
     console · Workflow completed successfully

  ✓  on-complete                                             2 ms
     console · Final outcome: succeeded


✓ RUN COMPLETED                                             73 ms

  Final result
  {
    "message": "Order order-42 was accepted",
    "total": 144,
    "delivery": "express"
  }


● Ready · Press Enter to run again
```

`STEPS COMPLETED` means business execution has settled. `RUN COMPLETED` means
required lifecycle finalization has also settled. The CLI must not print the
second statement early.

### 2.4 Failed run

Failures preserve the same layout and identify the responsible authored step:

```text
RUN  run_35ca91                              14 Aug 2026 · 10:44:03

  01  ✓  Prepare order                                      15 ms
          Normalize and validate the submitted order.
          → { orderId: "order-43", customer: "Sarah", items: 1 }

  02  ✓  Calculate total                                    26 ms
          Reusable step · Calculate taxes and final price.
          → { subtotal: 80, tax: 16, total: 96 }

  03  ✗  Charge customer                                   840 ms
          Submit the payment to the configured provider.

          Error
          WOML_HTTP_REQUEST_FAILED
          Payment provider returned HTTP 503.

          Attempts  3 · Retry exhausted

  04  ○  Build confirmation                                Skipped


STEPS COMPLETED

  Duration    881 ms
  Steps       2 succeeded · 1 failed · 1 skipped


LIFECYCLE

  ✓  on-failure                                              7 ms
     console · Payment processing failed

  ✓  on-complete                                             2 ms
     console · Final outcome: failed


✗ RUN FAILED                                               890 ms

  WOML_HTTP_REQUEST_FAILED
  Step "Charge customer" failed after 3 attempts.


● Ready · Press Enter to run again
```

### 2.5 Repeated and concurrent runs

Every new occurrence appends a new `RUN` block with its own ID and local date.
Completed blocks are never rewritten after settlement.

For the common single-active-run case, step progress appears beneath the active
run. If several runs execute concurrently, WOML keeps output readable:

1. it prints each admission immediately with the run ID;
2. it maintains in-memory presentation state per run;
3. it emits each settled run block atomically instead of interleaving step
   lines from unrelated runs; and
4. it reports queued/waiting state immediately so the user knows the keypress or
   inbound request was accepted.

The engine is never serialized merely to make the terminal output simpler.
Existing workflow concurrency and queue policies remain authoritative.

## 3. Product Decisions

### 3.1 One presentation model, several renderers

Foreground output, historical inspection, and background log following target
one versioned `woml.run-presentation/v1` contract.

The contract is rendered in three modes:

1. **TTY mode** — colors, Unicode icons, adaptive width, and organized blocks;
2. **plain mode** — the same information without ANSI escape sequences or
   cursor control when output is redirected; and
3. **JSON mode** — machine-readable records only.

Business execution does not emit separately formatted strings for each mode.
This prevents foreground, background, and historical views from disagreeing.

### 3.2 Rust and durable history remain authoritative

The presentation projection is derived from:

- the exact frozen compiled workflow definition associated with the run;
- immutable Event v13 history and its existing fold;
- run policy, retry, approval, fork, choice, notification, and lifecycle
  projections; and
- safe runtime progress for live refresh.

Terminal state is never folded from previously printed terminal lines.
`runtime.log` is not parsed to reconstruct workflow truth. A renderer crash or
Ctrl+C cannot change a run.

No new durable event is introduced only to color or align terminal output. If
TM0 discovers that an approved fact cannot be deterministically derived from
existing events, that gap must be reviewed explicitly before Event v14 is
created.

### 3.3 Workflow and authored-node metadata

Model v14 already carries workflow metadata and authored-node metadata. The
projection exposes:

- workflow ID, name, description, and version;
- authored node ID, name, description, and operation kind;
- reusable definition alias when relevant;
- public control-flow identity and selected route; and
- source order for stable display numbering.

Compiler-generated joins, barriers, sentinels, lifecycle implementation nodes,
and namespaced module artifacts are hidden. They may appear only under an
explicit diagnostic/verbose surface, not normal output.

### 3.4 Color and icon contract

The default TTY palette is:

| Meaning | Color | Icons |
| --- | --- | --- |
| WOML identity and section labels | cyan | `◆`, `→` |
| Ready/running/active | blue | `●` |
| Success | green | `✓` |
| Waiting/retrying/warning/skipped | yellow | `◌`, `○`, `!` |
| Failure/cancellation | red | `✗`, `■` |
| Control flow and approval | purple | `◇` |
| IDs, dates, descriptions, secondary text | gray/dim | none |

Color defaults to automatic detection. WOML respects:

- whether stdout/stderr is a TTY;
- `NO_COLOR`;
- `TERM=dumb`; and
- `--color=auto`, `--color=always`, or `--color=never`.

Plain mode has readable textual status even after color is removed. Color is
never the only status signal.

### 3.5 Results are useful, bounded, and non-authoritative

The renderer may display a successful authored step result when it exists.
Display formatting never mutates the durable JSON result.

The first profile uses these display rules:

- one-line scalar/object/array previews when they fit the terminal width and
  preview byte budget;
- indented JSON for a small multiline value;
- structural summaries such as `[250 items]` or `{42 keys}` for larger values;
- complete final result when it fits the final-result budget; and
- a copyable `woml get <runId> --json` instruction when output is shortened.

Secrets remain governed by the existing secret boundary. Approval capability
URLs, provider credentials, secret props, idempotency material, and internal
control tokens never enter the presentation contract. User-authored business
results may contain sensitive business data, so documentation must explain that
terminal visibility and local state access are trust boundaries.

### 3.6 Lifecycle is separate from business steps

The presentation model distinguishes:

- business steps settled;
- workflow business outcome decided;
- lifecycle hook/action execution;
- lifecycle warnings; and
- final run settlement.

Lifecycle action failures remain warnings and do not rewrite business truth.
The renderer shows the warning and the truthful final outcome.

### 3.7 Default output stays quiet

Normal output does not print internal debug contracts, raw N-API messages,
queue polling, lease renewals, generated node IDs, or provider protocol frames.

Actionable warnings remain visible. Low-level diagnostics belong to
`--verbose` or structured operational logs. `--json` remains free of human
decoration.

### 3.8 Manual means a user-created occurrence

`<manual>` no longer means “run automatically once during activation.” In
foreground TTY mode it means:

- compile and activate the workflow;
- show the workflow and trigger header;
- wait without creating a run;
- create one unique durable trigger occurrence for each accepted Enter press;
- display the resulting run ID immediately; and
- return to ready state after the run settles while the runtime remains alive.

`woml test workflow.woml` remains the explicit one-shot command for tests,
scripts, and CI. It may admit one selected manual occurrence and exit with the
result as it does today.

### 3.9 Log following is a view, not runtime ownership

The public commands are:

```bash
woml run_8f21c4 --logs
woml order-processing --logs
```

`run_...` is unambiguously a run ID. A lower-kebab value is a workflow ID.

- Run ID: display that run's complete available history; follow it while it is
  active; exit after terminal settlement. Ctrl+C detaches early.
- Workflow ID: display the most recent bounded run history in chronological
  order, then follow future runs while the runtime is active. Ctrl+C detaches.
- Inactive runtime: display retained durable history and exit after explaining
  that no live runtime is available to follow.
- Pruned run: return a stable not-found/pruned diagnostic rather than inventing
  history.

Ctrl+C in a log viewer affects only the viewer process. It never sends
`woml stop`, never cancels a run, and never changes runtime ownership.

## 4. CLI Contract

### 4.1 Foreground run

```bash
woml run workflow.woml
woml run workflow.woml --trigger start
woml run workflows/
```

TTY mode uses the professional renderer. Redirected output uses plain mode.
`--json` remains explicit machine output. `--color` applies only to human
rendering.

### 4.2 Background run

```bash
woml run workflows/ --background
```

The successful handoff remains readiness-based and gains log-follow commands:

```text
WOML runtime started in the background.

Runtime    runtime_a8c1...
Workflows  2
Logs       woml order-processing --logs
           woml inventory-sync --logs
Inspect    woml inspect
Stop       woml stop
```

When many workflows are loaded, the startup receipt shows a bounded list and a
single `woml list` hint rather than flooding the terminal.

The detached process writes safe operational records without ANSI color.
Pretty history is rendered when a user attaches with `--logs`; raw terminal
escape sequences are never persisted.

### 4.3 Log subject options

```text
woml <run-id|workflow-id> --logs
  [--state <path>]
  [--config <path>]
  [--json]
  [--color=auto|always|never]
```

The default state/config discovery matches existing `woml list`, `woml get`,
`woml inspect`, and background-runtime behavior. An explicitly configured
non-default deployment must be targetable without guessing its state file.

Workflow log follow prints at most the ten most recent retained runs before
following new activity. Older history remains discoverable through `woml list
--workflow <workflowId>` followed by `woml <runId> --logs`.

JSON following uses newline-delimited `woml.run-presentation/v1` snapshots or
updates. It never mixes human headings with JSON.

### 4.4 Manual target selection

For one available manual trigger:

```text
  ● Manual
    Press Enter to start a run
```

For several available manual targets in one loaded foreground deployment:

```text
MANUAL TRIGGERS

  1  order-processing / start
  2  inventory-sync / refresh

  Type a number and press Enter to run
```

If a single workflow declares several manual triggers, `--trigger <id>` may
select one and restore the one-key Enter experience. Invalid or ambiguous
selection fails before admission.

Manual keyboard payload is the empty object `{}`. WOML does not prompt for
arbitrary JSON in this milestone. Workflows needing external payload data use
webhook, event, Slack, or another data-bearing trigger.

### 4.5 Non-interactive and background manual behavior

- `woml test` remains the one-shot non-interactive manual execution surface.
- Foreground `woml run` with only manual triggers and no usable TTY fails with
  `WOML_MANUAL_TRIGGER_TTY_REQUIRED` and recommends `woml test`.
- A background deployment never fabricates a startup manual occurrence.
- A deployment containing other production triggers may include manual
  triggers; keyboard manual admission is unavailable in the detached child.
- A manual-only `--background` invocation is rejected with an actionable
  message because it would have no user-operated ingress.

## 5. Run Presentation v1

### 5.1 Purpose

`woml.run-presentation/v1` is a read-only public projection for human and
machine presentation. It is not a new workflow event vocabulary, store
authority, or mutable cache.

The reviewed schema contains:

```text
profile
workflow
  id
  name?
  description?
  version?
run
  id
  admittedAt
  startedAt?
  settledAt?
  status
  businessOutcome
  lifecycleStatus
  durationMs?
trigger
  id
  type
  occurredAt
steps[]
  position
  id
  name?
  description?
  kind
  parentControlId?
  status
  startedAt?
  settledAt?
  durationMs?
  attempts
  result?
  selection?
  failure?
lifecycle[]
  hookId
  status
  durationMs?
  actions[]
summary
  succeeded
  failed
  skipped
  cancelled
  waiting
result?
failure?
warnings[]
```

Every public field has a byte/cardinality limit in the schema. Unknown future
profiles fail closed in JSON consumers and receive a compatibility diagnostic
in the human CLI.

### 5.2 Author-visible step kinds

The projection uses stable product kinds:

- `step`;
- `reusable-step`;
- `switch`;
- `choose`;
- `parallel`;
- `fork` and `branch`;
- `approval`;
- `workflow-call` and `workflow-start`; and
- a reviewed generic operation fallback for future official handlers.

Internal graph nodes are folded into their author-visible owner. For example,
switch selection and merge nodes appear as one switch item; fork barrier work
appears under the fork; approval resolution and join nodes appear as one
approval item.

### 5.3 Status and timing

Presentation status is derived, never guessed:

- `queued`;
- `running`;
- `waiting`;
- `retrying`;
- `succeeded`;
- `failed`;
- `skipped`;
- `cancelled`; and
- `finalizing` for run/lifecycle scope.

Durations use durable event timestamps. A live duration may use the monotonic
local clock for display, but the settled duration is recomputed from durable
timestamps and replaces the live estimate.

### 5.4 Ordering and hierarchy

Steps use stable authored source order for numbering. Nested work retains a
bounded hierarchy so parallel branches, fork branches, switch cases, choose
arms, and approval arms can be indented without creating a second graph
visualizer.

Only the selected switch/choice arm executes. Unselected author-visible work is
reported as skipped when that information improves understanding; synthetic
nodes are never counted as skipped steps.

### 5.5 Result and error formatting

The schema carries JSON values and safe structured failure facts. Text
formatting belongs to the renderer.

Error presentation includes, when available:

- stable WOML code;
- author-facing step name and ID;
- bounded safe message;
- attempt and retry exhaustion information;
- timeout/cancellation state; and
- source location under `--verbose`.

Rust/Bun stack traces are not printed in normal mode. They remain available to
diagnostic tooling where safe.

## 6. Manual Trigger Admission v1

### 6.1 Runtime boundary

The existing production trigger host gains an explicit manual-admission
operation. The CLI never bypasses Rust by calling the step executor directly.

One keyboard action creates a request containing:

```text
workflowId
triggerId
occurrenceId
occurredAt
payload: {}
```

Rust validates that the exact active definition owns the manual trigger,
applies trigger deduplication, runtime policy admission, queue/concurrency/rate
limits, durable run creation, and normal execution.

### 6.2 Uniqueness and repeated input

Every accepted keyboard submission receives a fresh occurrence ID. Pressing
Enter twice intentionally requests two runs. Terminal key repeat is treated as
user input, not idempotent replay.

If an admitted run is queued by concurrency or rate-limit policy, the CLI shows
its run ID and waiting reason. It does not discard the second keypress or wait
for the previous run before asking Rust to admit it.

### 6.3 Input ownership

The foreground CLI owns line input and user prompts. Rust owns admission and
execution. The input reader:

- does not use raw terminal mode for the simple Enter prompt;
- treats Ctrl+C as graceful runtime shutdown;
- ignores non-empty input when there is one target and explains the accepted
  action;
- validates numbered selection before sending an admission request; and
- closes before runtime shutdown so no keypress races with closed admission.

## 7. Background Logs and Attachment

### 7.1 Data sources

The log viewer combines two reviewed sources:

1. a durable `Run Presentation v1` snapshot generated from the local store; and
2. the authenticated Operations Stream/current runtime descriptor for change
   notification while the runtime is alive.

The stream tells the viewer when to refresh. It is not the source of business
truth. Sequence gaps cause a snapshot refresh using the existing observability
rule.

### 7.2 Run attachment

`woml run_8f21c4 --logs`:

1. resolves the configured state;
2. reads the durable presentation snapshot;
3. renders all retained authored steps and lifecycle state;
4. attaches to the exact live runtime when the run is active;
5. refreshes on relevant progress; and
6. exits when the run settles or the user presses Ctrl+C.

### 7.3 Workflow attachment

`woml order-processing --logs`:

1. validates the workflow ID against retained/active definitions;
2. renders up to ten most recent retained runs oldest-to-newest;
3. follows newly admitted matching runs;
4. ignores unrelated workflow activity; and
5. remains attached until Ctrl+C or runtime loss.

If runtime ownership changes during restart, the viewer may reconnect through
the newly published descriptor after revalidating deployment identity. It
never sends control operations using a stale capability.

### 7.4 Operational logs remain separate

`.woml/logs/runtime.log` remains a safe operational log for startup,
readiness, ownership, providers, retention, and failures. It is not duplicated
into workflow state and is not parsed as execution truth.

`--logs` is the user-facing execution history/following experience. A future
explicit raw operational-log command can be considered separately if users
need it.

### 7.5 Retention, backup, and shutdown

- A viewer holds no workflow or maintenance lease.
- `woml prune` may remove eligible terminal run history according to the
  existing policy; attached viewers receive a history-unavailable update.
- Backup/restore preserves durable presentation inputs because it already
  preserves definitions and events; no terminal-render cache is backed up.
- Ctrl+C closes only the viewer's authenticated stream.
- `woml stop` remains the only normal command that stops the background
  runtime.

## 8. Error Contract

New stable diagnostics include at least:

| Code | Meaning |
| --- | --- |
| `WOML_MANUAL_TRIGGER_TTY_REQUIRED` | A manual-only foreground runtime has no interactive terminal. |
| `WOML_MANUAL_TRIGGER_SELECTION_REQUIRED` | More than one manual target exists and no valid selection was made. |
| `WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE` | A manual-only detached runtime would have no input surface. |
| `WOML_MANUAL_TRIGGER_ADMISSION_CLOSED` | Enter was pressed while admission was draining/stopped. |
| `WOML_POLICY_QUEUE_FULL` | The shared durable policy queue cannot admit another manual occurrence yet. |
| `WOML_LOG_SUBJECT_INVALID` | The value before `--logs` is neither a valid run ID nor workflow ID. |
| `WOML_LOG_RUN_NOT_FOUND` | The requested retained run cannot be found. |
| `WOML_LOG_WORKFLOW_NOT_FOUND` | No active or retained workflow matches the requested ID. |
| `WOML_LOG_HISTORY_PRUNED` | The subject existed but its requested retained history was pruned. |
| `WOML_LOG_RUNTIME_UNAVAILABLE` | Historical output was shown but no live runtime can be followed. |
| `WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED` | The CLI cannot render the returned presentation profile. |

Diagnostics carry code, message, and source/correlation details where
applicable. They never include payloads, results, secrets, approval URLs, or
provider credentials.

## 9. Compatibility Contract

- Existing WOML source syntax does not change.
- `<manual id="..." />` remains the source tag; only `woml run` activation
  behavior changes from automatic startup execution to user input.
- `woml test` preserves one-shot manual execution for automation and tests.
- Webhook, event, Slack, schedule, and interval trigger semantics remain
  unchanged.
- Model v14 and Event v13 remain readable and authoritative.
- Existing Store v14 data remains valid; Run Presentation v1 is derived.
- Existing run IDs, workflow IDs, definition hashes, results, retry behavior,
  approvals, workflow calls, and lifecycle outcomes remain unchanged.
- `woml list`, `woml get`, `woml cancel`, `woml inspect`, and `woml stop`
  remain available.
- `--json` contract tests prevent decorative output, ANSI sequences, or human
  labels from leaking into machine output.
- Existing `runtime.log` records remain readable; professional rendering does
  not require rewriting old logs.

The breaking user-visible change is intentional and documented:

```text
Before: woml run manual.woml immediately created one run.
After:  woml run manual.woml waits for Enter and remains active.
        woml test manual.woml creates one run and exits.
```

## 10. Architecture and Ownership

```text
WOML source
  └─ TypeScript compiler
       └─ Model v14 metadata and authored DAG

Durable run
  ├─ Event v13 history
  ├─ existing pure projections
  └─ frozen run definition
       └─ Rust Run Presentation v1 projection
            ├─ foreground TTY renderer
            ├─ plain redirected renderer
            ├─ JSON renderer
            └─ background --logs viewer

Keyboard Enter
  └─ Bun CLI input adapter
       └─ explicit Rust manual admission
            └─ normal trigger/runtime-policy/execution path
```

Ownership is explicit:

| Layer | Owns |
| --- | --- |
| TypeScript frontend | workflow/step display metadata and trigger display configuration |
| Rust core | manual occurrence admission, durable truth, presentation projection, timing/status/result facts |
| Bun CLI | terminal width, color, icons, JSON/text formatting, keyboard input, log attachment |
| Admin/observability | authenticated change notification and live attachment |
| SQLite Event Store | retained workflow/run truth used for historical presentation |

The CLI must not infer step success from a printed line, infer timing from log
arrival, or infer selected routes by reevaluating conditions.

## 11. Implementation Phases

### 11.1 Phase summary

| Phase | What changes | Result after the phase |
| --- | --- | --- |
| TM0 — completed | Freeze presentation, color, result, manual-admission, log-follow, error, security, and compatibility contracts with reviewed fixtures. | Every layer targets one approved terminal and runtime behavior before code changes. |
| TM1 — completed | Build the pure terminal design system and render reviewed presentation fixtures in color/plain/JSON modes. | The approved output exists as a deterministic renderer independent of execution. |
| TM2 — completed | Add Rust Run Presentation v1 projection and native/TypeScript decoding. | One durable run can be queried with names, descriptions, steps, results, lifecycle, timing, and summaries. |
| TM3 — completed | Replace foreground trigger/run output with the professional renderer. | Webhook, Slack, schedule, interval, event, and existing one-shot runs are readable and organized. |
| TM4 — completed | Implement explicit Rust manual admission and the Enter-driven foreground input loop. | `<manual>` waits for the user and can create repeated durable runs. |
| TM5 | Add `woml <run-id|workflow-id> --logs` and background-runtime attachment. | Users can inspect and follow background execution safely from another terminal. |
| TM6 | Complete complex control-flow, retry, approval, lifecycle, concurrency, and multi-run presentation. | Large real workflows remain organized instead of exposing internal engine noise. |
| TM7 | Harden accessibility, security, compatibility, retention, restart, and failure behavior. | Output remains safe and truthful across terminals, pipes, crashes, and historical stores. |
| TM8 | Document, benchmark, package, and publish the terminal/manual milestone. | Professional output and real manual triggers are supported WOML features. |

Phase labels are planning shorthand only. Permanent files, symbols, fixtures,
tests, and package scripts use descriptive product names rather than names such
as `tm4_*`.

### TM0 — Freeze contracts and reviewed fixtures — completed

Changes:

- Freeze the approved workflow header, trigger section, run block, step rows,
  business summary, lifecycle section, final outcome, failure, and ready prompt.
- Freeze color meanings, icons, TTY detection, `NO_COLOR`, narrow-terminal,
  plain, and JSON behavior.
- Freeze Run Presentation v1 schema, cardinality/byte limits, status mapping,
  author-visible kinds, hierarchy, result/failure shape, and future-version
  behavior.
- Freeze how names/descriptions/version fall back when metadata is absent.
- Freeze result preview, multiline formatting, structural truncation, and full
  result hints.
- Freeze exact manual Enter, numbered selection, empty payload, admission,
  repeated keypress, queue, shutdown, non-TTY, and background behavior.
- Freeze `woml <run-id|workflow-id> --logs`, history bounds, follow/exit,
  runtime restart, pruned history, and Ctrl+C behavior.
- Decide from reviewed Event v13 histories whether every approved presentation
  fact is derivable without Event v14.
- Add reviewed source/model/event/presentation/text/plain/JSON fixtures for
  success, failure, retry, skip, cancellation, waiting, approval, switch,
  parallel, fork, lifecycle warning, and repeated runs.
- Add historical Model/Event/Store fixtures and current background descriptor,
  log, and operations-stream fixtures.

Reuse:

- Model v14 workflow/node metadata and authored DAG.
- Event v13 folding, Store v14, Run Inspection v5, trigger progress, runtime
  policy progress, lifecycle progress, and Operations Stream v1.
- Existing webhook/event curl example generation and Slack/scheduler metadata.
- Existing background descriptor, authenticated admin listener, runtime
  ownership, and state/config discovery.
- Existing terminal inspector input/signal patterns where they fit the simpler
  line-input contract.

Result:

The output, manual trigger, and log-follow experience are reviewable as frozen
contracts instead of being invented while modifying `cli.ts`.

Gate:

TM1 does not begin until reviewed fixtures answer every field, color, status,
result, trigger, keypress, lifecycle, history, security, and compatibility
question without relying on terminal strings as truth.

### TM1 — Build the terminal presentation system — completed

Changes:

- Add a focused pure presentation module rather than adding more formatting
  branches to `cli.ts`.
- Implement theme tokens, status icons, ANSI application, color disabling, and
  ASCII/plain fallbacks.
- Implement adaptive workflow header width and wrapped descriptions.
- Implement aligned run/step/lifecycle rows and terminal-width-aware JSON
  previews.
- Implement success/failure/waiting/retry/cancelled/skipped/finalizing states.
- Implement pure renderers for TTY text, plain text, and JSON/NDJSON.
- Add golden snapshots at wide, normal, narrow, no-color, redirected, and
  `TERM=dumb` widths.
- Ensure arbitrary user strings cannot inject untrusted ANSI control sequences
  or terminal cursor commands.

Result:

Reviewed Run Presentation fixtures render exactly like the approved output,
with colors in real terminals and equally readable plain output elsewhere.

Gate:

Golden tests cover every status and optional-field combination; stripping ANSI
from colored output produces the same semantic text as plain mode; malicious
names/descriptions/results cannot control the terminal.

### TM2 — Build Run Presentation v1 in Rust — completed

Changes:

- Add a pure Rust presentation projection over the frozen definition and
  durable event/projection state.
- Map Model v14 nodes/control groups to author-visible presentation items.
- Fold names, descriptions, kinds, source order, selected routes, skipped work,
  attempts, statuses, timestamps, durations, results, failures, lifecycle, and
  warnings.
- Hide internal compiler/runtime nodes by construction.
- Add a bounded native query for one run and a workflow-scoped recent-run query.
- Add N-API functions and strict TypeScript decoders for Run Presentation v1.
- Expose the same query through authenticated runtime administration for live
  attachment.
- Add schema and conformance fixtures accepted independently by Rust and
  TypeScript.
- Prove projection purity and deterministic output across process restart.

Result:

Any retained run can be transformed into one truthful, versioned presentation
snapshot without parsing logs or rerunning workflow logic.

Gate:

Rust/TypeScript conformance, historical store, restart, malformed/future
version, limits, result redaction, selected-route, lifecycle, and timing tests
pass.

### TM3 — Replace foreground output — completed

Changes:

- Render workflow name, description, version, ID, and trigger instructions once
  activation is ready.
- Reuse existing curl/schema examples inside the new trigger layout.
- Route trigger, policy, retry, approval, lifecycle, and terminal progress into
  per-run presentation state.
- Print the run ID/date immediately after admission.
- Print one atomic organized block when each run settles; show queued/waiting
  notices while it is active.
- Render final result only after lifecycle finalization.
- Keep actionable runtime warnings outside run blocks with clear severity.
- Remove raw default progress chatter and preserve it under `--verbose` or
  structured operational logging.
- Preserve strict stdout/stderr and `--json` behavior for pipelines.

Result:

Existing production triggers produce the approved professional output without
changing their execution semantics.

Gate:

Packaged foreground acceptance covers webhook, Slack, schedule, interval,
event, success, failure, lifecycle, warning, and Ctrl+C shutdown with colored,
plain, and JSON snapshots.

### TM4 — Make `<manual>` interactive — completed

Changes:

- Treat manual workflows as active production-runtime registrations rather
  than one-shot startup work.
- Remove automatic startup admission for normal `woml run` manual triggers.
- Add explicit manual occurrence admission to the Rust trigger runtime and
  native bridge.
- Add the foreground line-input adapter and ready prompt.
- Admit one run for every accepted Enter submission.
- Support numbered manual-target selection for multi-workflow foreground
  activation and `--trigger` for one workflow with several manual triggers.
- Show queued/rate-limited/concurrent admission truth using existing runtime
  policy progress.
- Keep `woml test` one-shot and non-interactive.
- Fail manual-only non-TTY/background cases with the frozen actionable
  diagnostics.
- Close input before graceful runtime draining.

Result:

The user controls when a manual run begins and can trigger the same active
workflow repeatedly without restarting WOML.

Gate:

Real PTY acceptance proves no run exists before Enter, one Enter creates one
run, repeated Enter creates distinct runs, policies still apply, invalid input
does not admit, Ctrl+C drains, and `woml test` remains scriptable.

Completion evidence:

- The frozen Manual Trigger Admission v1 contract is implemented through the
  native Rust runtime boundary; accepted occurrences use the same durable
  scheduler and policy authority as every production trigger.
- Single-target Enter, numbered multi-workflow selection, `--trigger`, empty
  payload, repeated independent runs, queue notices, and ready-state return are
  implemented in the foreground CLI.
- Manual-only non-TTY and background activation fail with their stable,
  actionable diagnostics before workflow admission; `woml test` remains the
  explicit non-interactive one-shot surface.
- The consolidated `test:manual-triggers` gate passes 22 contract,
  native-runtime, renderer, packaged CLI, and real PTY tests with 91
  assertions, followed by TypeScript and single-job Rust checks.

### TM5 — Add background log following

Changes:

- Parse the direct subject syntax `woml <run-id|workflow-id> --logs`.
- Resolve default/configured state and active runtime descriptor securely.
- Render retained Run Presentation snapshots before attaching live.
- Add authenticated workflow/run filtered follow behavior using Operations
  Stream change notifications plus snapshot refresh.
- Reconnect safely across runtime restart and resynchronize on sequence gaps.
- Exit run attachment at terminal settlement and workflow attachment on Ctrl+C.
- Guarantee Ctrl+C never stops the runtime or cancels work.
- Add plain and NDJSON behavior for redirected/machine consumers.
- Update background startup receipt with copyable per-workflow log commands.
- Handle inactive runtimes, pruned history, rotated descriptors, runtime loss,
  and permission errors clearly.

Result:

A user can leave a workflow running in the background, open another terminal,
and see the same organized run history and live updates at any time.

Gate:

Two-process acceptance starts a packaged background runtime, triggers several
runs, attaches by run ID and workflow ID, verifies colors/history/live updates,
detaches with Ctrl+C, proves the runtime remains ready, then stops it with
`woml stop`.

### TM6 — Complete automation/control-flow presentation

Changes:

- Render switch/choose selection and stable results.
- Render parallel children and final policy without interleaving unrelated
  runs.
- Render fork branches, joined/unjoined status, and join release.
- Render approval waiting, deadline, notification delivery, decision, and
  selected arm without exposing capability URLs.
- Render retry attempt failure, scheduled delay, eventual success/exhaustion,
  and idempotent reattachment.
- Render workflow calls/starts with child run ID and wait/detached status.
- Render lifecycle hooks/actions/providers and warnings after business steps.
- Render cancellation, workflow timeout, queue/concurrency/rate-limit waiting,
  recovery, and skipped downstream work.
- Keep compiler-generated nodes absent from counts and display.

Result:

The simple visual language remains understandable for real automation graphs,
not only sequential hello-world workflows.

Gate:

Golden and packaged acceptance cover every official flow construct in nested
positions, reversed concurrency timing, waiting/recovery, failures, and more
than one simultaneous run.

### TM7 — Harden security, accessibility, and compatibility

Changes:

- Fuzz unsafe terminal text, control characters, huge Unicode, narrow widths,
  malformed JSON, deeply nested results, and oversized collections.
- Enforce presentation query/cardinality/byte/time budgets.
- Verify secrets, approval URLs, credentials, tokens, secret props, and
  idempotency material never enter snapshots, logs, errors, or golden output.
- Verify business results are shortened only for display and never silently
  rewritten in durable state.
- Test `NO_COLOR`, every `--color` mode, non-TTY pipes, `TERM=dumb`, common
  Linux/macOS/Windows terminal behavior, and missing Unicode capability.
- Test background descriptor replacement, runtime crash/restart, viewer crash,
  sequence gaps, state permissions, backup/restore, retention, and prune.
- Verify historical Model/Event/Store runs render or fail with reviewed
  compatibility diagnostics.
- Verify old CLI machine commands and JSON outputs are unchanged.

Result:

The terminal experience is safe, accessible, bounded, recoverable, and
backward-compatible rather than a fragile layer of colored print statements.

Gate:

Security scan, adversarial snapshots, compatibility fixtures, crash/restart,
retention, backup/restore, cross-terminal, and machine-output suites pass.

### TM8 — Package, document, benchmark, and publish

Changes:

- Add manually runnable sequential, failure, manual, webhook, control-flow,
  lifecycle, and background-log examples.
- Document every trigger header and status/icon meaning.
- Document `woml run` versus `woml test`, repeated manual runs, multi-target
  selection, `--logs`, Ctrl+C, background operation, color control, and JSON.
- Update architecture, production runtime, observability, trigger, lifecycle,
  security, CLI, and migration documentation.
- Add descriptive renderer, presentation-projection, manual-trigger,
  background-log, PTY, clean-package, and compatibility release commands.
- Benchmark renderer cost, projection latency, attach latency, live refresh,
  memory per active run, large result summaries, and high-rate run streams.
- Run the milestone through a clean installed package with native Rust and Bun
  hosts.

Result:

Professional terminal output, real manual triggers, and background execution
following become supported, documented WOML behavior.

Gate:

Frontend, Rust, N-API, terminal snapshots, PTY, two-process background,
security, compatibility, typecheck, Clippy, package, docs, and benchmark gates
pass without skipped native coverage.

## 12. Verification Matrix

| Area | Required proof |
| --- | --- |
| Workflow header | Name, description, version, ID, optional fields, wrapping, and narrow widths render correctly. |
| Trigger section | Manual, webhook, event, Slack, schedule, and interval show correct activation instructions. |
| Step identity | Author name/description/ID/type appear; internal nodes never appear. |
| Status | Queued, running, waiting, retrying, succeeded, failed, skipped, cancelled, and finalizing are truthful. |
| Results | Small/multiline/large/scalar/array/object results format predictably without changing stored JSON. |
| Control flow | Switch, choose, parallel, fork, branch, approval, calls, and starts render selected/executed work correctly. |
| Lifecycle | Business settlement and lifecycle finalization remain distinct; warnings do not rewrite truth. |
| Manual | No startup run; Enter admits once; repeated Enter admits distinct runs; selection and policies work. |
| Background logs | Run/workflow history and live following work; Ctrl+C detaches only the viewer. |
| Concurrency | Simultaneous runs execute concurrently while terminal blocks remain readable. |
| Color/accessibility | Auto/always/never, `NO_COLOR`, plain, ASCII fallback, and contrast/status icons work. |
| Machine output | `--json`/NDJSON contain no ANSI or human prose and preserve reviewed schemas. |
| Security | Secrets/capabilities/credentials/internal tokens never enter presentation or operational logs. |
| Recovery | Restart derives identical settled presentation and reconnects live viewers safely. |
| Retention | Pruned history fails clearly; viewers do not block prune or backup. |
| Compatibility | Historical models/events/stores and existing CLI commands remain readable and stable. |
| Package | A clean installed CLI passes real PTY and two-process native acceptance. |
| Performance | Rendering/projection/following stay within frozen budgets and do not slow execution materially. |

## 13. Explicit Non-Goals

This milestone does not add:

- a browser dashboard;
- a full-screen TUI replacement for the existing `woml inspect` surface;
- mouse interaction or terminal menus requiring a special font;
- a custom `.woml` editor theme or VS Code extension;
- arbitrary manual-trigger JSON prompting;
- remote/public manual-trigger HTTP endpoints;
- remote administration beyond the existing reviewed local admin boundary;
- raw operational-log search/query language;
- distributed log storage or hosted log aggregation;
- a new durable event merely to support decoration;
- persistence of ANSI-colored terminal strings;
- changes to workflow business semantics, retries, approvals, policies, or
  results;
- performance optimization of unrelated engine hot paths;
- removal of legacy Cronflow core/SDK code;
- additional Discord, WhatsApp, or Telegram built-ins; or
- retirement of the JavaScript chaining SDK.

These remain separately reviewed roadmap work.

## 14. Expected File Areas

| Area | Expected locations |
| --- | --- |
| Pure terminal renderer | new focused modules under `woml-cli/src`, not more monolithic `cli.ts` formatting |
| CLI parsing/orchestration | `woml-cli/src/cli.ts` plus focused argument/input/log-follow modules |
| Trigger summaries | existing webhook/event/schedule/interval/Slack summary helpers in the CLI |
| Rust presentation projection | `core/woml-engine/src/durable.rs`, projection/model helpers, and a focused presentation module |
| Manual admission | `core/woml-engine/src/webhook.rs` or renamed production-trigger runtime authority |
| Native bridge | `core/src/woml_bridge.rs`, `woml-cli/src/rust-executor.ts` |
| Runtime administration | `woml-cli/src/runtime-control.ts`, `runtime-observability.ts`, reviewed admin schemas |
| Schemas/protocols | `docs/schemas`, `docs/protocols`, reviewed presentation/manual fixtures |
| PTY tests | focused `woml-cli/tests` helpers and packaged terminal acceptance |
| Rust tests | focused presentation/manual/store/recovery tests in `core/woml-engine/tests` |
| Examples/docs | `examples`, CLI/runtime/trigger/observability/security/migration documentation |
| Release gates | descriptive renderer, PTY, background attachment, package, compatibility, and benchmark scripts |

Implementation reuses the existing compiler, Model v14 metadata, Event v13
fold, Store v14, runtime-policy scheduler, production trigger host, runtime
descriptor, authenticated admin stream, and background child. It must not add a
parallel executor or a mutable “current terminal context” authority.

## 15. Risks and Guardrails

### Pretty output can become a second truth source

The renderer consumes Run Presentation v1. It never infers business state from
printed strings, callback arrival order, or JavaScript logs.

### Step results can flood or expose the terminal

Results use explicit preview budgets and structural summaries. Existing secret
boundaries and capability redaction remain mandatory. Full business data is
shown only through trusted local operator surfaces.

### Colors can make output inaccessible or break pipelines

Status always has an icon/text equivalent. Auto-detection, `NO_COLOR`, plain
mode, and JSON mode are tested as first-class outputs.

### Manual input can bypass normal trigger guarantees

Enter calls explicit Rust trigger admission. It does not execute the workflow
directly in Bun and cannot bypass deduplication, policy admission, persistence,
or durable event creation.

### Repeated Enter can accidentally create many runs

Every accepted keypress is intentional and visible. Existing concurrency,
rate-limit, queue, and overflow policies remain the control mechanism; WOML
does not silently debounce requested automations.

### Background log following can accidentally gain control authority

The viewer receives read-only snapshot/stream access. Ctrl+C closes the viewer
only. Stop and cancellation remain separate explicit commands.

### Concurrent runs can interleave into unreadable output

The renderer buffers state per run and publishes settled run blocks atomically.
Execution concurrency is never reduced for display convenience.

### Historical presentation can drift after source files change

Presentation reads the exact frozen run definition/artifacts, never current
source files. Restart reproduces the same settled view.

### `cli.ts` can become harder to maintain

Formatting, terminal capabilities, manual input, presentation decoding, and
log following live in focused modules with pure tests. `cli.ts` only composes
them.

## 16. Global Roadmap After This Milestone

1. **WOML Editor Experience and Theme** — ship a WOML language extension and
   TextMate grammar for tags, attributes, `{{...}}` references, embedded
   JavaScript, diagnostics, custom imported tags, autocomplete, and a polished
   color theme while respecting the user's editor theme.
2. **Performance Profiling and Optimization** — establish end-to-end startup,
   compilation, N-API, serialization, worker-host, short-workflow,
   large-workflow, and concurrency profiles; optimize measured bottlenecks
   without weakening durability or isolation. As an optional, benchmark-gated
   investigation, prototype WOML XML parsing with Rust `quick-xml` and compare
   it with the current Bun/TypeScript `fast-xml-parser` frontend using real
   small, large, module-heavy, and control-flow-heavy workflows. Keep N-API as
   the stable boundary. Adopt Rust parsing or compilation only when it produces
   a meaningful end-to-end improvement while preserving exact diagnostics,
   source locations, raw `<script>` behavior, modules, and compiled-model
   conformance. TypeScript/Bun remains authoritative unless a reviewed complete
   migration makes Rust the single compiler; WOML must never maintain two
   competing compiler implementations.
3. **Legacy Cronflow Core Audit and Removal Map** — produce a dependency-backed
   inventory of old JavaScript-chaining bridge, dispatcher, trigger executor,
   step orchestrator, state machine, database, and compatibility paths;
   identify what WOML still uses, what needs replacement, and what can be
   removed later without deleting code during the audit.
4. **Additional Communication Providers** — add built-in Discord, WhatsApp, and
   Telegram triggers, notifications, and messaging capabilities when product
   demand justifies them; keep custom providers available for project-specific
   integrations.
5. **Retire the JavaScript Chaining SDK** — remove the old SDK and approved
   legacy core paths only after WOML reaches sufficient parity, the legacy
   audit is reviewed, migration guidance exists, and replacement coverage
   passes.

Completed milestones—including choices/switch, parallel, fork/branch, Human
Approval, retries/idempotency, production triggers, services/capabilities,
essential modules, reusable steps/providers, Durable Workflow Calls/Start,
lifecycle/engine controls, runtime policies, Durable State, and Production
Runtime/Operations—remain the baseline and are not repeated as future work.

## 17. Definition of Done

The milestone is complete only when:

- normal TTY output matches the approved workflow, trigger, run, step,
  lifecycle, and final-outcome structure with colors;
- workflow name, description, version, and ID derive from the exact frozen
  definition;
- every supported trigger prints truthful activation instructions;
- every run has an appended run ID and local date;
- authored steps show stable order, name/ID, optional description, type,
  duration, status, attempts, and bounded result when available;
- internal compiler/runtime nodes never appear in normal output or counts;
- control-flow selection, waiting, skipped work, retries, approvals,
  cancellation, lifecycle, and warnings render truthfully;
- business step settlement and final lifecycle settlement remain distinct;
- colors are accessible and fully removable without losing meaning;
- `--json` and redirected output remain deterministic and ANSI-free;
- Run Presentation v1 is versioned, bounded, pure, independently validated,
  and reconstructable after restart;
- `woml run` manual workflows create no run before input;
- each accepted Enter action creates one normal Rust-admitted durable run;
- repeated manual runs do not require restarting WOML;
- `woml test` remains the one-shot non-interactive command;
- background startup prints copyable log-follow commands;
- `woml <run-id|workflow-id> --logs` renders retained history and follows live
  matching execution;
- Ctrl+C in the viewer never stops the background runtime or cancels work;
- secrets, capability URLs, credentials, and internal tokens never leak;
- old source, Model v14, Event v13, Store v14, run management, background,
  and machine-output behavior remains compatible; and
- the independent terminal/manual release gate and full repository release
  gate pass from a clean installed package.

## 18. TM0 Review Gate

Before TM1 begins, review these artifacts together:

- colored and plain workflow-header fixtures at wide/normal/narrow widths;
- trigger fixtures for manual, webhook, event, Slack, schedule, and interval;
- sequential success and failure run fixtures matching Section 2;
- result fixtures for scalar, object, array, multiline, large, Unicode, and
  unsafe terminal control input;
- control-flow fixtures for switch, choose, parallel, fork/branch, approval,
  Workflow Call, and Workflow Start;
- retry, queue, rate-limit, cancellation, timeout, waiting, recovery, and
  lifecycle-warning histories;
- Run Presentation v1 JSON schema and Rust/TypeScript reviewed examples;
- exact Event v13 histories proving every presented fact is derivable;
- manual single-target, multi-target, repeated Enter, invalid input, non-TTY,
  background, policy, and shutdown fixtures;
- run-ID and workflow-ID `--logs` histories, live stream, runtime restart,
  sequence gap, inactive runtime, prune, and Ctrl+C fixtures;
- machine JSON/NDJSON snapshots with zero ANSI/human text;
- secret/capability redaction fixtures;
- historical Model/Event/Store/background descriptor/log fixtures; and
- benchmark budgets for projection, rendering, attachment, and active-run
  memory.

TM0 is approved only when the same artifacts answer:

1. What exact workflow and trigger information does the user see before a run?
2. Which authored operation does each displayed row represent?
3. Where does every displayed status, duration, result, and lifecycle fact come
   from?
4. What does Enter do, and through which durable admission authority?
5. How does a second terminal reconstruct and follow a background run?
6. What does Ctrl+C stop in each terminal?
7. Which output is human, which is machine-readable, and where can color occur?
8. Which data is forbidden from every presentation and log surface?
9. Can old durable runs render without reopening current source files?
10. Can presentation fail without changing workflow execution truth?
