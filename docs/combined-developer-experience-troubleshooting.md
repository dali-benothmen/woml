# Combined Developer Experience Troubleshooting

## Project not found

WOML discovers `woml.project.json` only in the current directory. Change to the
project root or pass `--project /absolute/path/woml.project.json`.

## `WOML_PROJECT_OPTION_CONFLICT`

The project already declares a runtime, policy, reviewer directory, or remote
target input that the CLI invocation tries to replace. Update the reviewed
manifest or remove the conflicting flag; WOML will not choose one silently.

## Project check fails on a reference

Keep references relative to the project root, regular files, and free of symlink
escapes. Run `woml check --json` to get the source and corrective action in a
stable machine-readable result.

## A target is offline or unauthorized

```bash
woml target check production-private --registry ./ops/targets.json --trust ./ops/target-trust.json --json
woml target inspect production-private --registry ./ops/targets.json --trust ./ops/target-trust.json --json
```

Confirm worker identity, certificate trust, compatibility, advertised
capabilities, local policy, capacity, and drain state. Remote work never falls
back to local execution.

## A remote dispatch is ambiguous

Ambiguous means the worker may have produced an external effect but the
coordinator lacks a trustworthy terminal fact. Automatic retry is intentionally
blocked. Inspect the dispatch and the affected external system. Use
`woml dispatch reconcile` only after that review; never use reconciliation to
invent a success result.

## Logs are empty

Use the durable run id and the same state store as the runtime:

```bash
woml logs run_<id> --no-follow --state ./.woml/state.sqlite --json
```

Operational output is bounded and may contain a truncation marker. Workflow
history remains authoritative even when old output has been pruned.

## Recovery after a crash

Restart with the same project, state path, policies, and deployment identity.
WOML reattaches to durable waits, locks, compensation, artifacts, and remote
dispatch records. Restore a verified backup to a new path first when diagnosing
store damage.
