# Releasing WOML to npm

WOML publishes one platform-neutral `woml-cli` package and one small native
engine package for each supported operating-system, CPU, and Linux libc pair.
Users install only `woml-cli`; platform metadata limits its optional native
dependencies and the WOML loader selects the exact runtime package.

## Published package family

| Runtime | Native package |
| --- | --- |
| macOS x64 | `@woml-org/cli-darwin-x64` |
| macOS ARM64 | `@woml-org/cli-darwin-arm64` |
| Windows x64 | `@woml-org/cli-win32-x64-msvc` |
| Windows ARM64 | `@woml-org/cli-win32-arm64-msvc` |
| Linux x64 glibc | `@woml-org/cli-linux-x64-gnu` |
| Linux ARM64 glibc | `@woml-org/cli-linux-arm64-gnu` |

Linux GNU packages are built against glibc 2.31 and run on glibc 2.31 or
newer. The release workflow inspects every produced Linux binary and rejects
it if any required `GLIBC_*` symbol exceeds that compatibility ceiling.

The native packages contain only one `.node` binary, package metadata, README,
and Apache-2.0 license. The main package contains the CLI, script hosts,
communication-provider host, and built-in Slack/Telegram/Discord/WhatsApp
adapters but no native binary. Its exact-version optional dependencies select
the platform package.

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

1. Own or create the `@woml-org` npm scope.
2. Own or create `woml-cli` and every `@woml-org/cli-*` package.
3. Create one granular npm access token with read/write access to `woml-cli`
   and every `@woml-org/cli-*` package. Enable bypass 2FA for package
   publication when required by the npm account/package policy, then store it
   as the GitHub Actions secret `NPM_TOKEN`.
4. Create the protected GitHub environment `npm-production` and optionally require an
   owner review before deployment.
5. Keep GitHub Actions enabled for release builds and allow the protected
   publish job to create GitHub releases.

The token is exposed as `NODE_AUTH_TOKEN` only to the credential check and the
two npm publication steps. Build, test, collection, and artifact-verification
jobs cannot read it. The publish job retains `id-token: write` and
`--provenance`, so public packages carry verifiable GitHub build provenance.
The workflow fails with an explicit error before publishing when `NPM_TOKEN`
is missing.

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
`release/packages/woml-cli-1.0.1.tgz`. Publishing directly from `woml-cli/` is
blocked intentionally; only the verified staging directory may become the
main npm package.

Commit the version change, then create and push the exact matching tag:

```bash
git tag v1.0.0
git push origin v1.0.0
```

The tag workflow rejects a tag that differs from the package version. It builds
and load-tests all six native targets on matching runtime families, packs and
seals each package, verifies the complete collected family, and uploads a
30-day release artifact. After every required job succeeds, the same run
publishes the six native packages first and `woml-cli` last. It creates the
GitHub release only after npm succeeds.

If `npm-production` requires reviewers, GitHub pauses immediately before the
publish job and waits for an owner approval; no `.tgz` download or local
`npm publish` command is needed. A rerun safely skips package versions already
present on npm and continues with any missing family member.

For recovery, an owner may manually run `release.yml` from the exact tag with
`publish_to_npm=true`. Running it with `publish_to_npm=false` rebuilds and
verifies the release family without publishing.

## Supported build boundary

The release builds only `core/woml-native`, which depends locally only on
`woml-engine`. It never restores the retired combined Rust package. Cargo uses
the committed workspace lockfile and one build job per runner. Linux builds run
inside the pinned multi-architecture `rust:1.88-bullseye` compatibility image,
while their resulting packages are still load-tested on matching x64 and ARM64
GitHub runners. macOS, Windows, and Linux artifacts all pass their matching
runtime tests before publication.

## CI and artifact retention

Pull requests and main-branch pushes expose six named gates: language frontend,
CLI/runtime journeys, Rust engine, architecture separation, documentation and
examples, and package contract. Rust compilation and tests stay single-job and
serial where shared durable stores require it.

CI keeps its inspectable main-package artifact for 7 days. A release candidate
keeps individual main/native artifacts for 14 days and the verified collected
family for 30 days. Every family member contains an npm tarball and a checksum
manifest; native packages additionally contain a matching-runtime load receipt
with the exact N-API export set.
