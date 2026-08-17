# Terminal Experience Examples

Build and link the local CLI once, then run these journeys from the repository
root. `woml run` stays active; press Enter to start each manual run and Ctrl+C
to stop it.

## Sequential and repeated manual runs

```bash
woml run examples/terminalExperience/sequential.woml
```

Press Enter twice. The two run IDs and their step blocks remain separate.

## Intentional failure

```bash
woml run examples/terminalExperience/failure.woml --color=never
```

Press Enter. The first step succeeds, the second fails, and the final run block
shows the durable error separately from step output.

## One-shot manual test

```bash
woml test examples/terminalExperience/sequential.woml
```

This runs once and exits. It is intended for scripts and CI, not long-lived
automation.

## Control flow

```bash
woml run examples/manualForkWorkflow.woml
```

Press Enter to see a fork with three branches, selected joined routes, a
condition inside a branch, and a condition on the continuation.

## Lifecycle

```bash
woml run examples/lifecycleWorkflow.woml
```

Press Enter to see business steps and the workflow lifecycle as separate
sections.

## Webhook and professional trigger instructions

```bash
woml run examples/webhookWorkflow.woml
```

Copy the `curl` command printed under the webhook trigger. Each call produces
a new organized run block without restarting WOML.

## Background logs

```bash
woml run examples/webhookWorkflow.woml --background
woml webhook-demo --logs
```

Trigger the printed webhook from another terminal. Ctrl+C leaves the log
viewer but keeps the background runtime active. Stop it explicitly:

```bash
woml stop
```

Use `--json` for NDJSON and `--color=never` or `NO_COLOR=1` for a color-free
human view. See the complete
[terminal guide](../../docs/woml-terminal-experience.md).
