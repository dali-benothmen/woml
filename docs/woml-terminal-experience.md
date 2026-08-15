# WOML Terminal Experience

WOML's terminal output is a read-only view of the frozen workflow definition
and its durable event history. It is designed for people first, remains useful
without color or Unicode, and never becomes a second workflow state authority.

## Start an automation

Use `woml run` for an active automation:

```bash
woml run workflow.woml
woml run workflow.woml --color=never
woml run workflows/ --background
```

For a manual workflow, startup displays the workflow and trigger information
and then waits. It does not create a run until you press Enter. Every later
Enter creates another independent, durably admitted run. Ctrl+C stops the
foreground runtime after in-flight work drains.

If one deployment has several manual targets, WOML prints a numbered list.
Type a number and press Enter. Select one target permanently with:

```bash
woml run workflows/ --trigger start-order
```

Use `woml test` only when a script, CI job, or shell command needs exactly one
manual execution and then an exit:

```bash
woml test workflow.woml
```

`woml run` is the automation host; `woml test` is the one-shot testing tool.
A manual-only `woml run` requires an interactive terminal and cannot be
detached because no user would remain to press Enter.

## What the workflow header shows

The header uses the exact frozen definition:

```text
╭─ WOML WORKFLOW ──────────────────────────────────────────────────────╮
│ Order Processing                                             v1.4.0 │
│ Validates an order, calculates its total, and selects delivery.     │
╰──────────────────────────────────────────────────────────────────────╯
```

Optional names, descriptions, and versions disappear cleanly when omitted.
Below the header, every configured trigger explains how the automation is
activated:

| Trigger | Terminal instruction |
| --- | --- |
| Manual | Press Enter, choose a numbered target, and use Ctrl+C to stop. |
| Webhook | HTTP method, bound URL, authentication warning, and copyable `curl`. |
| Event | Event name, authenticated publication URL, and copyable request. |
| Slack | Workspace, event type, and channel or conversation scope. |
| Schedule | Cron expression, timezone, and next due instant. |
| Interval | Fixed interval and next due instant. |

The printed URL is the actual bound address. An unauthenticated webhook is
marked as a warning rather than silently presented as production-safe.

## Run blocks and status symbols

One run is printed as one organized block. Concurrent runs can execute at the
same time, but their completed blocks are not interleaved.

```text
RUN  run_8f21c4                              14 Aug 2026 · 10:42:18

  01  ✓  Prepare order                                      18 ms
          Normalize and validate the submitted order.
          → { orderId: "order-42", items: 3 }

  02  ✓  Calculate total                                    31 ms
          → { subtotal: 120, tax: 24, total: 144 }

STEPS COMPLETED

  Duration    49 ms
  Steps       2 succeeded · 0 failed · 0 skipped

✓ RUN COMPLETED                                             49 ms
```

| Symbol | Text fallback | Meaning |
| --- | --- | --- |
| `✓` | `OK` | Succeeded. |
| `✗` | `X` | Failed or timed out; the failure code follows. |
| `●` | `RUN` | Currently running. |
| `◇` | `QUEUE` | Durably queued and waiting for runtime capacity. |
| `◐` | `WAIT` | Waiting for approval, cancellation, or another durable condition. |
| `↻` | `RETRY` | A durable retry is scheduled or executing. |
| `◌` | `FINAL` | Business work settled and lifecycle finalization remains. |
| `○` | `-` | Skipped or cancelled. |

Names and descriptions come from authored WOML. `Script`, `Reusable step`,
`Switch`, `Choose`, `Parallel`, `Fork`, `Branch`, `Approval`, `Workflow call`,
and `Workflow start` labels explain structural work. Compiler joins and other
engine-only nodes never appear as user steps.

Small results are displayed inline. Deep, wide, or large values are shortened
only in the presentation and marked as truncated. Inspect the durable command
shown beside the preview for the complete result. Display shortening never
rewrites SQLite or the workflow result.

Lifecycle activity appears after business steps. A lifecycle warning is not a
business failure, and a finalizing run is not presented as completed early.

## Color, Unicode, pipes, and JSON

Color defaults to `auto` and is used only for an interactive capable terminal:

```bash
woml run workflow.woml --color=auto
woml run workflow.woml --color=always
woml run workflow.woml --color=never
NO_COLOR=1 woml run workflow.woml
```

`TERM=dumb`, redirected output, and `NO_COLOR` disable automatic color. Plain
and JSON formats never contain ANSI sequences. Status meaning never depends on
color alone. When Unicode is unavailable, WOML uses the text fallbacks in the
table above.

`--json` emits versioned Run Presentation JSON on stdout and warnings on
stderr. Live workflow following emits one JSON object per line (NDJSON), which
can be consumed incrementally:

```bash
woml order-processing --logs --json | jq -c '.status'
```

## Follow a background automation

Start the runtime once:

```bash
woml run workflows/ --background
```

The startup receipt prints copyable commands for each workflow. Follow one
workflow or one exact run from another terminal:

```text
woml <run-id|workflow-id> --logs
```

```bash
woml order-processing --logs
woml run_8f21c4 --logs
woml order-processing --logs --state /srv/woml/state.sqlite
```

WOML first renders retained durable history and then follows authenticated
change notifications from the local runtime. Notifications cause a fresh
durable projection read; runtime log text is never parsed into workflow truth.

Ctrl+C detaches only the viewer. It does not stop the runtime or cancel a run.
Use `woml stop` to stop the deployment and `woml cancel run_...` to request
run cancellation.

The viewer reconnects after a same-deployment runtime restart. It fails closed
if the descriptor is replaced by another deployment, resynchronizes after a
stream sequence gap, and clearly reports retained history that was pruned.

## Security and bounds

Presentation output has fixed row, history, nesting, collection, string, and
2 MiB transport limits. Unsafe terminal controls and bidi overrides are
neutralized. Credential-shaped properties, capability and approval URLs,
Bearer/Basic values, Slack-token shapes, sensitive query parameters, and
idempotency material are redacted from previews and diagnostics.

Do not deliberately return credentials as business results. Secret values
belong in `{{secrets.NAME}}` and are never intended to become workflow output.
Use `woml get ... --json` only as a trusted local operator because complete
business results may contain private customer data even when they contain no
credentials.

## Troubleshooting

| Diagnostic or symptom | Meaning and action |
| --- | --- |
| `WOML_MANUAL_TRIGGER_TTY_REQUIRED` | `woml run` cannot read Enter from this environment. Use a terminal or use `woml test` for one execution. |
| `WOML_LOG_RUN_NOT_FOUND` | The run ID is unknown or no retained history remains. Verify `--state` and retention. |
| `WOML_LOG_WORKFLOW_NOT_FOUND` | The workflow is neither active nor retained in that store. |
| `WOML_LOG_STATE_UNAVAILABLE` | The current user cannot read the durable state file. Check ownership and permissions. |
| `WOML_LOG_RUNTIME_UNAVAILABLE` | Retained history may still be shown, but no matching live runtime can be followed. |
| `WOML_LOG_HISTORY_PRUNED` | Retention removed history while it was being viewed. |
| `WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED` | The store or presentation is newer than this CLI. Upgrade WOML; do not downgrade the store. |
| Output contains no colors | Check `NO_COLOR`, `TERM`, redirection, and `--color`; status text remains authoritative. |
| Results are shortened | Run the displayed `woml get run_... --json` command for the durable result. |

Runnable journeys are collected in
[`examples/terminalExperience`](../examples/terminalExperience/README.md).
The normative machine contracts remain in
[`protocols/terminal-experience-v1.md`](protocols/terminal-experience-v1.md).
