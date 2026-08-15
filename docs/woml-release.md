# Releasing WOML to npm

WOML publishes one platform-neutral `woml-cli` package and one small native
engine package for each supported operating-system, CPU, and Linux libc pair.
Users install only `woml-cli`; platform metadata limits its optional native
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
and Apache-2.0 license. The main package contains the CLI and Bun hosts but no
native binary. Its exact-version optional dependencies select the platform
package. Linux selection distinguishes glibc from musl at runtime.

Local development remains simple: `bun run build` stages the current machine's
binary directly under `woml-cli/dist`, and that colocated binary takes priority.
`WOML_RUST_CORE_PATH` remains the explicit development/test override.

## One-time npm setup

1. Own or create the `@woml` npm scope.
2. Give the npm identity used by GitHub Actions permission to publish
   `woml-cli` and every `@woml/cli-*` package.
3. Add an npm automation token as the repository secret `NPM_TOKEN`.
4. Keep GitHub Actions enabled for tag builds and allow the release workflow to
   create GitHub releases.

The workflow requests `id-token: write` and publishes with npm provenance.
Never place the token in source, workflow arguments, artifacts, or logs.

## Release procedure

Use one version across:

- `woml-cli/package.json`;
- `woml/package.json`; and
- `core/woml-native/Cargo.toml`.

Run the local contract check:

```bash
cd woml-cli
bun run test:native-platform-release
```

Commit the version change, then create and push the exact matching tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow rejects a tag that differs from the package version. It validates
the frontend, builds the portable Bun files, builds and load-tests all eight
native targets, verifies the collected package set and licenses, publishes the
native packages first, publishes `woml-cli` last, and creates the GitHub
release. A rerun safely skips package versions already present on npm.

## Supported build boundary

The release builds only `core/woml-native`, which depends locally only on
`woml-engine`. It never restores the retired combined Rust package. Cargo uses
the committed workspace lockfile and one build job per runner. macOS, Windows,
Linux glibc, and Linux musl artifacts are tested on matching runtime families
before publication.
