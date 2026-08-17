# Releasing WOML to npm

WOML publishes one platform-neutral `woml` package and one small native
engine package for each supported operating-system, CPU, and Linux libc pair.
Users install only `woml`; platform metadata limits its optional native
dependencies and the WOML loader selects the exact runtime package. On Linux,
the loader explicitly distinguishes glibc from musl.

## Published package family

| Runtime | Native package |
| --- | --- |
| macOS x64 | `@woml-org/cli-darwin-x64` |
| macOS ARM64 | `@woml-org/cli-darwin-arm64` |
| Windows x64 | `@woml-org/cli-win32-x64-msvc` |
| Windows ARM64 | `@woml-org/cli-win32-arm64-msvc` |
| Linux x64 glibc | `@woml-org/cli-linux-x64-gnu` |
| Linux x64 musl | `@woml-org/cli-linux-x64-musl` |
| Linux ARM64 glibc | `@woml-org/cli-linux-arm64-gnu` |
| Linux ARM64 musl | `@woml-org/cli-linux-arm64-musl` |

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

1. Own or create the `@woml-org` npm scope.
2. Own or create `woml` and every `@woml-org/cli-*` package.
3. Configure npm trusted publishing for `.github/workflows/release.yml` in this
   repository for every package in the family.
4. Create the protected GitHub environment `npm-production` and require an
   owner review before deployment.
5. After those controls are verified in V1R9, set the repository variable
   `WOML_NPM_PUBLISH_ENABLED=true` as the final publication safety latch.
6. Keep GitHub Actions enabled for candidate builds and allow the approved
   publish job to create GitHub releases.

The publish job uses GitHub OIDC (`id-token: write`) with npm provenance. It
does not read `NPM_TOKEN`, `NODE_AUTH_TOKEN`, or another long-lived registry
credential. Trusted-publisher ownership and the protected environment are
external V1R9 setup gates; do not invoke publication until both are verified.
The release workflow also fails before building publication intent unless the
V1R9 safety-latch variable is enabled.

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

The tag workflow rejects a tag that differs from the package version. It builds
and load-tests all eight native targets on matching runtime families, packs and
seals each package, verifies the complete collected family, and uploads a
30-day release-candidate artifact. **A tag push does not publish anything.**

After V1R7 and V1R8 pass, and only after the V1R9 npm/environment setup is
complete, an owner starts the same workflow manually from the exact tag:

```bash
gh workflow run release.yml --ref v1.0.0 -f publish_to_npm=true
```

The `npm-production` approval is the final human gate. The job downloads and
reverifies the already-tested candidate rather than rebuilding it, publishes
the eight native packages first and `woml` last, then creates the GitHub release
only after npm succeeds. A rerun safely skips package versions already present
on npm.

Running the workflow manually with `publish_to_npm=false`, or pushing the exact
tag normally, is always non-publishing and is safe for release-candidate proof.

## Supported build boundary

The release builds only `core/woml-native`, which depends locally only on
`woml-engine`. It never restores the retired combined Rust package. Cargo uses
the committed workspace lockfile and one build job per runner. macOS, Windows,
Linux glibc, and Linux musl artifacts are tested on matching runtime families
before publication.

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
