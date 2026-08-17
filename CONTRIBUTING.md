# Contributing to WOML

Thank you for helping build WOML. Contributions should preserve its central
boundary: TypeScript parses and compiles `.woml` documents, Rust owns durable
workflow execution, and isolated Bun workers execute authored JavaScript.

## Development setup

Install Bun 1.3.14 or later, the stable Rust toolchain, and Git:

```bash
git clone https://github.com/dali-benothmen/woml.git
cd woml/woml-cli
bun install --frozen-lockfile
bun run build
```

The active repository packages are:

- `woml` — the private `@woml/compiler` parser, validator, compiler, source
  diagnostics, and types;
- `woml-cli` — the public `@woml-org/woml` package, CLI, runtime hosting, Bun workers,
  and provider adapters;
- `core/woml-engine` — durable, language-neutral execution authority;
- `core/woml-native` — the narrow N-API adapter; and
- `woml-vscode` — editor grammar, snippets, and language configuration.

## Before opening a pull request

Run the checks relevant to your change. The minimum repository checks are:

```bash
# From woml-cli/
bun run typecheck
bun test ../woml/tests
bun run test:architecture-separation
bun run test:documentation
```

Before proposing a release, run `bun run release:check` from the repository
root. The checker installs from the frozen lockfile, builds once, runs the
portable language/CLI/Rust baseline serially, verifies architecture and
documentation, checks the guarded release automation, and proves that two clean
package builds are identical.
Use `bun run format:check` from the repository root for a quick formatting
gate.

For Rust engine or adapter changes, also run:

```bash
cargo test -j 1 --locked --release --manifest-path ../core/Cargo.toml --workspace
```

Use one Cargo build job (`-j 1`) so local verification has predictable memory
usage. Feature-specific release scripts in `woml-cli/package.json` remain the
source of truth for broader gates.

Cross-platform npm publication is documented in
[`docs/woml-release.md`](docs/woml-release.md). A release tag is intentionally
non-publishing, but only a release owner should push one after the CLI,
frontend, and native-adapter versions match exactly.

## Architecture rules

- Do not parse WOML/XML in Rust. The TypeScript frontend is the single compiler.
- Do not put markup, interpolation, or editor concerns into the engine.
- Do not make Bun responsible for graph advancement, retry, or durable truth.
- Keep schema and protocol changes explicit, versioned, and fixture-backed.
- Preserve source locations and actionable error codes for author-facing errors.
- Never persist resolved secrets or expose them in logs and diagnostics.
- Do not add a second execution path for tests or convenience.
- Do not restore the retired JavaScript-chaining package or combined Rust core.

Frozen schema identifiers are protocol identities. A historical domain in a
schema `$id` must not be renamed as a branding cleanup.

## Pull requests

Keep pull requests focused and explain:

1. the product behavior being changed;
2. the architecture boundary affected;
3. the contracts or schemas changed, if any;
4. how the result was tested; and
5. any compatibility, recovery, security, or performance impact.

Use conventional commit subjects where practical, for example:

```text
feat(frontend): validate reusable provider props
fix(engine): recover an admitted child workflow
docs(runtime): clarify background log following
```

Do not include generated build output, runtime databases, secrets, or local
`.woml/` state in a pull request.

User-facing behavior changes must update the appropriate permanent guide and a
validation-tested example. Do not add milestone plans or phase codes to public
documentation, test filenames, or package commands.

## License

By contributing, you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE).
