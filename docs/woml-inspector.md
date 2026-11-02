# Terminal inspector

`woml inspect` is a full-screen terminal dashboard for an already-running WOML
runtime. Opening or closing it does not start, stop, or resume workflows.

```bash
woml inspect
woml inspect customer-onboarding.woml
woml inspect --state .woml/production.sqlite
woml inspect --no-color
```

The optional source file selects its declared workflow ID, not its filename.
WOML reads the markup without loading modules or executing scripts. The
workflow must already be loaded by the runtime that owns the selected state
store. Inspection does not discover another store from the file or replace
project configuration. Use `--state` when the runtime uses a non-default store.

An interactive terminal is required. For scripts, use `woml list --json` and
`woml get <run-id> --json` instead.

## Runs dashboard

The dashboard shows four status meters, runtime health and uptime, workflow
and trigger-type counts, and the recent run table. The green table header
contains Status, Workflow, Run ID, Work, and Duration. The selected row has a
gray background; its full identity and execution are shown below the table.
Color is supplementary: status words and the selection marker remain visible
with `--no-color` or `NO_COLOR`.

Meters and run counts describe the snapshot's recent run window, not all runs
ever stored. The current runtime supplies at most 200 recently updated runs
across all workflows. Active includes running and retrying runs. Triggers
counts the reported trigger types per loaded workflow, not individual trigger
registrations. Work displays the current node or bounded for-each progress
when the owning runtime has observed it; unknown work is shown as `—`.

| Key | Action |
| --- | --- |
| Up / Down | Select a run; the table scrolls to keep it visible |
| Right | Open analytics for the selected run's workflow |
| Left / Esc in analytics | Return to the same list selection and search |
| Tab / Shift+Tab | Cycle through the existing operational views |
| V | Open the View selector; use Up/Down and Enter, or 1–7 |
| 1–7 | Overview, runs, triggers, approvals, queues, failures, runtime |
| Enter | Toggle selected-run details |
| / | Search run IDs, workflow IDs, and statuses |
| c | Request cancellation of the selected active run, after confirmation |
| R | Refresh (`r` also works) |
| l | Show recent live events |
| ? | Toggle keyboard help |
| Q / Ctrl+C | Close the inspector; workflows keep running |

In analytics, Up/Down scroll by one line and Page Up/Page Down scroll by a
page. The footer remains visible. Search and cancellation are only available
from the operational views, not the analytics screen.

## Workflow analytics

The analytics screen shows the workflow's authored name and description, source
basename, trigger types, four summary statistics, six activity intervals,
step performance, and recent failure codes. Wide terminals show four KPI
columns and side-by-side step/failure sections. Narrow terminals stack these
sections and allow vertical scrolling.

All numbers come from durable native inspection, never sample data:

- The window is the preceding 24 hours at the displayed capture time. Activity
  intervals are six consecutive four-hour buckets; their labels use UTC.
- Run summaries are restricted to the selected workflow's latest 200 updated
  runs. Reaching that limit is conservatively labelled **limited**, even if the
  store happens to contain exactly 200. Old history may also have been pruned.
- Finished runs means succeeded plus failed runs in that window. Cancelled,
  queued, waiting, and active runs do not enter the success-rate denominator.
- Average, median, p95, and longest duration use finished runs with a valid
  execution start time. Duration is wall time from execution start to the
  summary's terminal update, including waits and retries but excluding time
  before execution started. Percentiles use the nearest-rank method.
- Retry and step statistics use at most the latest ten finished run
  presentations in the window. Retried runs counts presentations with an
  extra attempt; retry attempts sums `max(attempts - 1, 0)` from their displayed
  steps. These are presentation-level statistics, not an exhaustive count of
  hidden or truncated loop iterations. The coverage note always gives the
  number of detailed runs and labels incomplete detail coverage.
- Step averages use available per-step durations. Fail counts are failed or
  timed-out step presentations, not every failed attempt preceding a later
  success. Recent failures show at most three step failure codes from those
  detailed runs, never arbitrary script error text.
- Missing durations or unavailable detail statistics appear as `—`, not an
  invented zero. A run removed by retention between reads contributes to the
  unavailable-detail count without failing the runtime.

The capture time and coverage notes are part of the screen; scroll down to
read them in a short terminal. `NOW` counts are from the captured recent
workflow summary window, not an unbounded global census.

## Refresh, privacy, and compatibility

Live status updates retain the existing authenticated operations stream and
five-second snapshot refresh. Analytics is fetched only while its screen is
open, with a five-second client request timeout. The runtime caches each
workflow result for 15 seconds, coalesces concurrent requests for that
workflow, and retains at most eight results. Other workflow requests fail busy
while a detail build is in progress. Press R to retry; refreshing may still
return the current cached result.

A build reads one bounded summary list and at most ten presentations
sequentially, yielding between native calls. Once 250 ms has elapsed, it skips
additional detail reads and labels the result partial. This is a soft budget
between calls, not cancellation of a synchronous native read already underway.
Each presentation is bounded by the existing native contract; analytics uses
at most 200 step presentations per run and returns at most 100 aggregated
steps. These limits prevent repeated full-history work on every terminal
redraw. They do not claim complete historical analytics for a busy deployment.

`GET /v1/inspector/analytics?workflow=<id>` returns
`woml.inspector-analytics/v1` through the existing loopback admin listener. It
requires the current owner capability and inherits the admin response-size,
concurrency, and rate limits. It reports workflow metadata, capture/window
times, coverage, totals, activity, step aggregates, and stable failure codes.
It does not return payloads, step results, context, secret values, or raw error
messages. Workflow text is sanitized before terminal rendering.

The durable engine, native exports, workflow events, and frozen snapshot
contract are unchanged. An older running runtime does not have the analytics
endpoint: its run dashboard still works, while analytics shows an unavailable
message. Restart that runtime using the updated CLI to enable analytics.

See [runtime observability](woml-observability.md) for authentication and
telemetry boundaries, and [streaming output](woml-streaming-output-and-progress.md)
for following actual process output and progress records.
