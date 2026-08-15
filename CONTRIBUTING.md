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

The active product packages are:

- `woml` — parser, validator, compiler, source diagnostics, and public types;
- `woml-cli` — CLI, runtime hosting, Bun workers, and provider adapters;
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
```

For Rust engine or adapter changes, also run:

```bash
cargo test -j 1 --locked --release --manifest-path ../core/Cargo.toml --workspace
```

Use one Cargo build job (`-j 1`) so local verification has predictable memory
usage. Feature-specific release scripts in `woml-cli/package.json` remain the
source of truth for broader gates.

Cross-platform npm publication is documented in
[`docs/woml-release.md`](docs/woml-release.md). Do not push a release tag unless
the CLI, frontend, and native-adapter versions match exactly.

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

## License

By contributing, you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE).
