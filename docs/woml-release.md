# Releasing WOML to npm

WOML publishes one platform-neutral `woml` package and one small native
engine package for each supported operating-system, CPU, and Linux libc pair.
Users install only `woml`; platform metadata limits its optional native
dependencies and the WOML loader selects the exact runtime package. On Linux,
the loader explicitly distinguishes glibc from musl.

## Published package family

| Runtime | Native package |
| --- | --- |
| macOS x64 | `@woml/cli-darwin-x64` |
| macOS ARM64 | `@woml/cli-darwin-arm64` |
| Windows x64 | `@woml/cli-win32-x64-msvc` |
| Windows ARM64 | `@woml/cli-win32-arm64-msvc` |
| Linux x64 glibc | `@woml/cli-linux-x64-gnu` |
| Linux x64 musl | `@woml/cli-linux-x64-musl` |
| Linux ARM64 glibc | `@woml/cli-linux-arm64-gnu` |
| Linux ARM64 musl | `@woml/cli-linux-arm64-musl` |

The native packages contain only one `.node` binary, package metadata, README,
and Apache-2.0 license. The main package contains the CLI, script hosts,
communication-provider host, and built-in Slack/Telegram/Discord/WhatsApp
adapters but no native binary. Its exact-version optional dependencies select
the platform package. Linux selection distinguishes glibc from musl at runtime.

Local development remains simple: `bun run build` stages the current machine's
binary directly under the internal `woml-cli/dist` development directory, and
that colocated binary takes priority. The public package is never created from
that directory directly: `bun run build:release` copies only the frozen public
allowlist into `release/main` and deliberately excludes every `.node` file.
`WOML_RUST_CORE_PATH` remains the explicit development/test override.

The public JavaScript build produces six entrypoints and their source maps in
one deterministic build operation. The package verifier checks the CLI
shebang and executable bit, worker and provider-host paths, Slack assets,
license, native-loader dependency metadata, forbidden runtime files, and the
exact `npm pack --dry-run` inventory. Two builds must produce the same content
hash before the package is accepted.

## One-time npm setup

1. Own or create the `@woml` npm scope.
2. Give the npm identity used by GitHub Actions permission to publish
   `woml` and every `@woml/cli-*` package.
3. Add an npm automation token as the repository secret `NPM_TOKEN`.
4. Keep GitHub Actions enabled for tag builds and allow the release workflow to
   create GitHub releases.

The workflow requests `id-token: write` and publishes with npm provenance.
Never place the token in source, workflow arguments, artifacts, or logs.

## Release procedure

Use one version across the repository root, public CLI, private compiler,
native crates, and editor extension. Before creating a release candidate, run
the locked local gate from the repository root:

```bash
bun run install:check
bun run release:check
bun run build:release
```

To create a local tarball for inspection, use `bun run pack`. It writes
`release/packages/woml-1.0.0.tgz`. Publishing directly from `woml-cli/` is
blocked intentionally; only the verified staging directory may become the
main npm package.

Commit the version change, then create and push the exact matching tag:

```bash
git tag v1.0.0
git push origin v1.0.0
```

The workflow rejects a tag that differs from the package version. It validates
the frontend, builds the portable Bun files, builds and load-tests all eight
native targets, verifies the collected package set and licenses, publishes the
native packages first, publishes `woml` last, and creates the GitHub
release. A rerun safely skips package versions already present on npm.

## Supported build boundary

The release builds only `core/woml-native`, which depends locally only on
`woml-engine`. It never restores the retired combined Rust package. Cargo uses
the committed workspace lockfile and one build job per runner. macOS, Windows,
Linux glibc, and Linux musl artifacts are tested on matching runtime families
before publication.
