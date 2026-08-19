# Maintainer automations

This folder contains scripts the maintainers run to build, version, and ship WOML. They are **not** user-facing examples — for those, see `examples/` at the repo root.

## `bump-version.ts`

A prep tool that edits every place in the repo where the WOML version lives, regenerates the Cargo lockfile, and runs the full release validation suite. After it finishes, it prints the exact `git commit`, `git tag`, and `git push` commands the human should run.

### Toolchain

Requires the same toolchain as the rest of the project: `bun`, `git`, `cargo`, and `npm`. All are already hard dependencies.

### Usage

```bash
# From anywhere inside the repo
bun automations/bump-version.ts --from 1.0.4 --to 1.0.5

# Preview what would change without editing anything
bun automations/bump-version.ts --from 1.0.4 --to 1.0.5 --dry-run
```

The tool auto-detects the repository root by walking up from the current directory looking for the `woml-repository` `package.json`, so it works from any subdirectory.

### What it edits

The full list of files the tool knows how to bump:

- `woml-cli/package.json` — public `woml-cli`
- `woml/package.json` — private `@woml/compiler`
- `package.json` (repo root) — private `woml-repository`
- `woml-vscode/package.json` — private `woml-language`
- `core/woml-engine/Cargo.toml` — package version
- `core/woml-native/Cargo.toml` — package version + `woml-engine` dependency version
- `examples/production/deployment/Dockerfile` — `ARG WOML_VERSION=`
- `core/Cargo.lock` — regenerated via `cargo update --workspace`
- `woml-cli/scripts/release-package.ts` — identity check + log line
- `woml-cli/scripts/verify-production-release.ts` — identity check
- `woml-cli/scripts/verify-final-release-review.ts` — identity check
- `woml-cli/tests/release-identity.test.ts` — fixtures + test name
- `woml-cli/tests/release-family.test.ts` — fixtures
- `woml-cli/tests/native-platform-release.test.ts` — fixture
- `core/woml-native/tests/separation.rs` — Rust string literal

### What it does **not** do

The tool is prep-only. It deliberately does not:

- Run `git commit`
- Run `git tag`
- Run `git push`
- Run `npm publish`

After the tool finishes successfully, it prints the exact commands the human should run. This is a safety net: every irreversible action (commit, tag, push, publish) remains a deliberate human choice.

### When to update this tool

If a new manifest is added to the project (a new `package.json`, `Cargo.toml`, hardcoded version string), the maintainer must add it to `bump-version.ts` so the next bump catches it. The `git diff --stat` summary at the end of each run makes any missing bump visible — if the expected files don't appear in the diff, the tool needs updating.

### Validation it runs

After all edits, the tool runs:

1. `bun test tests/release-identity.test.ts tests/native-platform-release.test.ts --max-concurrency=1`
2. `bun test tests/release-automation.test.ts tests/release-artifact.test.ts tests/release-family.test.ts --max-concurrency=1`
3. `bun scripts/verify-native-platform-release.ts`
4. `bun scripts/verify-documentation.ts`
5. `bun run typecheck`

If any step fails, the tool halts and prints the failing step's stderr. No edits are rolled back — the human can fix the issue and re-run the tool; it's idempotent for the version-string edits (replace is a no-op if the new value already matches).
