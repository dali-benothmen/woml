# Build a Governed Infrastructure Agent

This tutorial connects WOML's project experience to its infrastructure runtime.
The result can inspect a proposed change, require a human decision, execute on a
private worker, stream progress, and preserve enough durable evidence to recover
without guessing.

## 1. Initialize and validate

```bash
woml init infrastructure-agent
cd infrastructure-agent
woml check
```

Add your workflows beneath `workflows/`. Keep credentials outside the project;
workflows request symbolic secret names and workers resolve only the names their
local policy permits.

## 2. Add the safety boundaries

Reference these operator-owned documents from `workflow.json` as needed:

- a process policy for exact executable aliases, arguments, working roots, and
  timeouts;
- an artifact policy for durable plans, reports, and logs too large for workflow
  history;
- an infrastructure policy and trust file for effect-level admission;
- a reviewer directory for rich plan approvals;
- a target registry and trust file for private remote workers.

Run `woml check` before every deployment. It validates the joined project,
workflow, policy, reviewer, target, and capability requirements without running
the workflow.

## 3. Put placement and approval in the workflow

An executable step can select a logical target:

```xml
<step id="applyPlan" target="production-private">
  <script>
    await services.process.run({
      executable: "terraform",
      args: ["apply", "-input=false", "reviewed.tfplan"]
    });
    return { applied: true };
  </script>
</step>
```

The target is not a hostname. Operators decide which authenticated workers may
serve it and which policies apply. Put a rich plan approval before this step and
store the reviewed plan as a durable artifact.

## 4. Observe and recover

Use `woml logs <run-id> --follow` for bounded operational output and export
OpenTelemetry to the collector you already operate. Back up durable state before
high-risk maintenance:

```bash
woml backup ./backups/pre-change --state ./.woml/state.sqlite --json
```

If a remote worker disappears after an effect may have started, WOML marks the
dispatch **ambiguous**. Do not rerun it blindly. Inspect the external system and
the dispatch, then record the reviewed reconciliation:

```bash
woml dispatch inspect dispatch_<id> --state ./.woml/state.sqlite --json
woml dispatch reconcile dispatch_<id> \
  --action record-failure \
  --request-id incident-2026-104 \
  --reason "Operator verified that apply never completed" \
  --state ./.woml/state.sqlite --json
```

See [Remote Execution Operations](remote-execution-operations.md) before running
remote effects in production.
