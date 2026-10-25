# WOML Project Manifest

`workflow.json` is the portable entry point for a WOML project. It names the
workflow set and references operator-controlled configuration without copying
credentials or policy contents into workflow source.

Create a reviewed local project with:

```bash
woml init my-automation
cd my-automation
woml check
woml test
```

The minimal manifest is:

```json
{
  "$schema": "https://woml.dev/schemas/woml-project.v1.schema.json",
  "profile": "woml.project/v1",
  "version": 1,
  "name": "my-automation",
  "workflows": ["./workflows"]
}
```

## Fields

- `profile` must be `woml.project/v1`.
- `version` must be `1`. Future versions fail closed.
- `name` is a stable, portable project name.
- `workflows` contains relative workflow files or directories. References may
  not escape the project root, resolve through a symlink, or declare the same
  workflow twice.
- `runtime` optionally references a Runtime Configuration document.
- `authority` optionally references process, artifact, infrastructure-policy,
  policy-trust, emergency-deny, and reviewer-directory documents.
- `remoteExecution` optionally references the target registry and target trust
  document.

Referenced JSON files are resolved relative to the manifest, canonicalized,
validated, and included in the project digest. The manifest stores paths and
symbolic secret names only; secret values belong in WOML's secret store or the
execution environment.

## Discovery and precedence

With no workflow argument or `--project`, project-aware commands inspect only
the current directory for `workflow.json`. WOML intentionally does not walk
parent directories. Use `--project <path>` when running elsewhere.

An explicit CLI option cannot silently replace authority already declared by a
project. Conflicting runtime, policy, reviewer, or target inputs fail with
`WOML_PROJECT_OPTION_CONFLICT`.

## Commands

```bash
woml check --project ./workflow.json --json
woml types --project ./workflow.json --output ./generated/woml.d.ts
woml test --project ./workflow.json --state ./.woml/state.sqlite
woml run --project ./workflow.json
```

See [Migrating to Projects](migrating-to-projects.md) for converting long CLI
invocations and the [operator runbook](combined-developer-experience-operator-runbook.md)
for production checks.
