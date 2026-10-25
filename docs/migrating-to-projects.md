# Migrating Long CLI Commands to WOML Projects

Projects replace repeated flag sets with one reviewable manifest. Existing
explicit-file commands remain supported, so migration can be incremental.

Before:

```bash
woml run ./workflows \
  --config ./ops/runtime.json \
  --process-policy ./ops/process-policy.json \
  --artifact-policy ./ops/artifact-policy.json \
  --policy ./ops/infrastructure-policy.json \
  --policy-trust ./ops/policy-trust.json \
  --reviewer-directory ./ops/reviewers.json \
  --target-registry ./ops/targets.json \
  --target-trust ./ops/target-trust.json
```

After, record those paths in `workflow.json`, run `woml check`, and use:

```bash
woml run --project ./workflow.json
```

## Safe migration

1. Run `woml init` in a new directory and copy workflow files into `workflows/`.
2. Add runtime and authority references one at a time.
3. Run `woml check --project ./workflow.json --json` after every change.
4. Commit the manifest and policy documents, but never secret values, private
   keys, runtime state, worker journals, or generated capability tokens.
5. Compare a `woml test` result with the existing explicit-file invocation.
6. Change CI and deployment commands only after the project check is clean.

CLI values do not override project authority. A collision is an error so that a
deployment script cannot quietly weaken a reviewed project.
