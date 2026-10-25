# WOML Authoring Guide

This project treats workflows as code and operator policies as security
boundaries.

## Validate every change

Run `woml check --project workflow.json` after editing a workflow or project
reference. Run the relevant workflow with the reviewed local rehearsal inputs
before proposing a production change.

## Keep authority operator-owned

Process, artifact, infrastructure, reviewer, target, and trust policies belong
in the files referenced by `workflow.json`. Do not weaken, replace, inline,
or bypass those files to make a workflow pass. A policy change requires an
explicit operator review separate from workflow authoring.

## Keep secrets symbolic

Reference secrets as `secrets.NAME`. Never add secret values, access tokens,
private keys, credentials, or copied environment values to WOML, project JSON,
fixtures, logs, artifacts, tests, or generated files.

## Safe automation rules

- Keep remote targets literal and reviewed; never silently fall back to local.
- Preserve approvals, fencing tokens, compensation, and cleanup around effects.
- Use argument arrays for `services.process`; never construct a shell command.
- Do not edit generated `woml-env.d.ts`; regenerate it with `woml types --project workflow.json`.
- Do not modify policy, trust, certificate, or reviewer files unless the task
  explicitly requests an operator-authority change.

The starter workflow is harmless and local:

```bash
woml check --project workflow.json
woml run --project workflow.json
```
