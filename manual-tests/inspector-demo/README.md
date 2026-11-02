# Inspector demo: four small workflows

This local-only project gives the new inspector real runs to display. It does
not send emails, create accounts, call external services, or change real
infrastructure. All delays and failures are intentional.

Use the rebuilt CLI from this branch, **not the globally installed `woml`**.
An older CLI runtime will not serve the new analytics endpoint.

## 1. Build once

From the repository root:

```bash
bun run --cwd woml-cli build:js
cd manual-tests/inspector-demo
bun ../../woml-cli/dist/cli.js check --project workflow.json
```

This only rebuilds JavaScript; it uses your existing native engine. If WOML
reports a missing native package, build the native engine separately using
the repository's normal build instructions. Do not start a full release gate
just to try this UI.

## 2. Start the runtime — terminal A

Run from this demo folder:

```bash
bun ../../woml-cli/dist/cli.js run --project workflow.json
```

Wait for the runtime's ready message. Leave this terminal running. Public demo
webhooks bind to `127.0.0.1:4140`; the authenticated local admin listener uses
`127.0.0.1:4141`. State is isolated in this folder's `.woml/` directory, which
Git ignores. There are only two configured workers and no native rebuild.

If either port is already occupied, change it in `woml.runtime.json` before
starting. `seed.ts` reads the public port from that file. Keep both hosts on
loopback: these demonstration webhooks intentionally use `auth="none"`.

## 3. Open the inspector — terminal B

Open another terminal in the same demo folder:

```bash
bun ../../woml-cli/dist/cli.js inspect --state .woml/state.sqlite
```

For the closest layout to the screenshots, make the terminal approximately
140 columns wide and 40–45 rows tall. A smaller terminal also works, but
analytics sections stack and require scrolling.

## 4. Add finished history — terminal C

In a third terminal, also in this folder:

```bash
bun seed.ts history
```

This adds exactly four runs and waits for their terminal states:

| Workflow | What happens | What to inspect |
| --- | --- | --- |
| Customer onboarding | Two successes and one deliberate failure at `provisionWorkspace` | Select a run, then Right: success rate, three step names, timings, recent failure code |
| Service health | Two failed attempts, then success on attempt three | Right: one retried run and two extra attempts |

On a fresh demo store, onboarding should show **3 finished runs**, **2
succeeded**, **1 failed**, and **66.7% success**. Service health should show
**1 finished run**, **100% success**, **1 retried run**, and **2 retry attempts**,
provided its detailed presentation is available. The screen reports detail
coverage if a read was skipped by its time budget.

The health workflow deliberately waits four seconds between attempts, so this
command normally takes about 10–15 seconds, plus local startup/storage overhead.
Every invocation adds new runs; counts accumulate. Analytics caches results
for up to 15 seconds, so wait for the capture timestamp to advance if needed.

## 5. Add live activity

Keep the inspector open and run:

```bash
bun seed.ts live
```

This admits five runs, then returns immediately:

- Three **send-welcome** runs. One runs while the others queue because the
  workflow declares concurrency one. Each has three three-second simulated
  tasks, so the batch normally takes roughly 30 seconds plus overhead.
- One **service-health** run that visibly enters retry delay before succeeding.
- One **document-review** run that waits durably for two minutes, then completes.

States change quickly and need not all appear in one frame. Start the
inspector **before** sending this batch. Retry releases execution capacity;
the durable document timer does not keep a script worker occupied.

Try these controls:

1. Up/Down selects a run; its identity appears below the table.
2. Right opens analytics for that workflow. Left returns to the list.
3. In analytics, Up/Down or Page Up/Page Down scrolls. R retries refresh;
   the server may still return its current cached result.
4. Select a queued welcome run or waiting document review, press **c**, then
   **y** to request cancellation. Cancelling changes demo state only. Wait for
   a terminal status before stopping the runtime; this is a separate check,
   not proof supplied by the successful history batch.
5. Press **V** to choose other views: Overview, Triggers, Approvals, Queues,
   Failures, or Runtime. A timer wait is not a human approval.
6. Press `/`, type `service-health`, then Enter to search. Repeat `/`, erase
   the query, and Enter to show everything again.
7. Resize the terminal; selection and the footer should remain usable.

To print statuses without the full-screen UI:

```bash
bun seed.ts status
```

To inspect only one workflow:

```bash
bun ../../woml-cli/dist/cli.js inspect workflows/customer-onboarding.woml --state .woml/state.sqlite
```

For a color-free pass, close the inspector with Q and reopen it with
`--no-color`.

## Optional: send requests yourself

These commands only admit runs; an HTTP 202 does not mean the workflow has
finished or succeeded.

```bash
curl -X POST http://127.0.0.1:4140/demo/onboarding -H 'content-type: application/json' -d '{}'
curl -X POST http://127.0.0.1:4140/demo/onboarding -H 'content-type: application/json' -d '{"fail":true}'
curl -X POST http://127.0.0.1:4140/demo/health -H 'content-type: application/json' -d '{}'
curl -X POST http://127.0.0.1:4140/demo/welcome -H 'content-type: application/json' -d '{}'
curl -X POST http://127.0.0.1:4140/demo/review -H 'content-type: application/json' -d '{}'
```

## Stop cleanly

Q closes only the inspector. To stop this demo runtime, press Ctrl+C in
terminal A, or run from the demo folder:

```bash
bun ../../woml-cli/dist/cli.js stop --state .woml/state.sqlite
```

Prefer letting running welcome tasks finish, or cancelling demo work first.
Durable waits and history survive a clean stop. Restart with the same command
to keep the existing history. Never delete or replace state while its runtime
is active.

## Initial verification note

The four workflows passed project validation. A real local runtime produced
the expected history, analytics counts, and running/queued/retrying/waiting
states. A separate document-timer cancellation remained `finalizing` before
shutdown, so cancellation settlement is **not certified** by this demo's
initial verification. If it persists during your test, keep the state and
report the run ID instead of treating that display as a completed cancellation.

The verification runtime was stopped. Its generated state was moved outside
the project to a private temporary directory, rather than deleted, so your
first manual run starts with an empty demo history.
