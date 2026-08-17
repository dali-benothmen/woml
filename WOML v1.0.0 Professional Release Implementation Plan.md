# WOML v1.0.0 Professional Release Implementation Plan

Status: V1R0 through V1R4 completed on 2026-08-17. V1R5 through V1R9 are planned. No npm
package has been published by this plan. Publishing is the final gated phase
and will be performed only with the product owner present.

## 1. Product Outcome

This milestone turns the completed WOML implementation into a professional,
public v1.0.0 product.

After the milestone is complete, a new user can:

1. install one public package globally with `bun add --global woml`;
2. run `woml --version` and receive `1.0.0`;
3. create or copy a `.woml` workflow and execute it with `woml run`;
4. use the correct native Rust engine automatically on a supported platform;
5. understand WOML from a concise README, a complete language reference, and
   working examples; and
6. trust that the published package was built, tested, and traced back to the
   public GitHub source.

The repository will also be ready for contributors: files and tests use
descriptive names, completed planning residue is removed, permanent contracts
are kept, documentation has one clear structure, and CI represents the actual
release boundary.

## 2. Approved Release Shape

### 2.1 Public product

WOML has one user-facing npm package:

```text
woml@1.0.0
  -> installs the `woml` command
  -> includes the Bun CLI, compiler bundle, workers, and built-in hosts
  -> selects one optional native package for the current platform
```

Users do not install a separate compiler or CLI package. The existing
`woml-cli` directory can remain an internal repository boundary, but its
published package name becomes `woml`.

The TypeScript frontend remains an internal package named
`@woml/compiler`, marked `private: true`, and is bundled into the public CLI.
It is not an independently published product.

The native Rust artifacts remain implementation packages:

- `@woml/cli-darwin-x64`
- `@woml/cli-darwin-arm64`
- `@woml/cli-win32-x64-msvc`
- `@woml/cli-win32-arm64-msvc`
- `@woml/cli-linux-x64-gnu`
- `@woml/cli-linux-x64-musl`
- `@woml/cli-linux-arm64-gnu`
- `@woml/cli-linux-arm64-musl`

They are exact-version optional dependencies of `woml`; users never choose or
install one manually.

### 2.2 Repository identity

The public repository becomes `dali-benothmen/woml`. Product metadata,
documentation, package manifests, issue links, release workflows, and editor
metadata use that identity. The local directory name does not affect the
published product and does not need to block v1.0.0.

The existing `cronflow` npm package is not unpublished. After `woml@1.0.0` is
successfully installed and verified, it can receive a short deprecation notice
that directs users to WOML.

### 2.3 Source and history policy

Cleanup must not destroy real product coverage or durable compatibility:

- tests are renamed or consolidated, not removed because their current names
  contain phase codes;
- examples remain committed and become part of the product documentation;
- versioned schemas, protocols, migrations, and compatibility fixtures remain;
- immutable schema `$id` values are not renamed for branding;
- completed implementation plans may be removed only after their still-valid
  decisions are present in permanent documentation; and
- generated output, runtime state, secrets, databases, and local scratch files
  remain ignored.

### 2.4 Publication safety

Build and release-candidate workflows must be safe to run without publishing.
The npm publication job is separate, explicit, and protected by a GitHub
release environment. npm trusted publishing through GitHub Actions is the
preferred final setup so the release does not depend on a long-lived npm token
and provenance is generated automatically for eligible public packages.

The current official references for that setup are:

- [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/)
- [npm provenance](https://docs.npmjs.com/generating-provenance-statements/)
- [GitHub deployment environments](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments)

Their exact registry steps will be rechecked during the final publication
phase because registry and CI requirements can change.

## 3. Release Order

```text
Audit -> human-readable names -> reviewed deletion -> package identity
      -> documentation -> deterministic build -> cross-platform release gate
      -> clean release candidate -> npm setup and publication
```

No phase may publish to npm before V1R9.

## 4. Scope

This release milestone includes:

- a dependency-backed v1.0.0 release-readiness audit;
- professional file, test, script, and command naming;
- reviewed removal of completed planning and obsolete repository files;
- one public npm package identity and aligned v1.0.0 versions;
- user, operator, contributor, editor, and release documentation;
- deterministic Bun and Rust builds;
- Linux, macOS, and Windows native packages;
- CI and release workflow hardening;
- clean-package and clean-machine smoke tests;
- npm trusted-publisher setup and final publication; and
- post-publication installation verification.

This milestone does not add language features, provider features, a package
registry, new runtime architecture, or performance optimizations. Functional
bugs discovered by release verification are fixed, but feature expansion is
recorded for a later release.

## 5. Release Artifacts

The work produces these durable artifacts:

| Artifact | Purpose |
| --- | --- |
| `docs/release/v1.0.0-readiness-audit.md` | Temporary source of truth for every release finding, owner, action, and verification result |
| `README.md` | Product landing page, quick start, core concepts, and documentation map |
| `docs/language-reference.md` | Stable v1.0.0 WOML authoring reference |
| `docs/cli-reference.md` | Complete command and option reference |
| `docs/getting-started.md` | First successful workflow from install to run |
| `docs/woml-release.md` | Maintainer-only build and publication runbook |
| `.github/workflows/ci.yml` | Pull-request and branch quality gates |
| `.github/workflows/release.yml` | Explicit release-candidate and protected publication pipeline |
| `woml@1.0.0` package | The one public package users install |
| eight `@woml/cli-*` packages | Platform-native Rust engine artifacts |

The release-readiness audit remains until the release is verified. After all
items are closed, it is either converted into a concise v1.0.0 release record
or removed before the final tag; it must not become a permanent unfinished
checklist in the repository root.

## 6. Phase Summary

| Phase | Work | Result |
| --- | --- | --- |
| V1R0 | Freeze release contracts and create the audit | One reviewed definition of what v1.0.0 publishes and what must be cleaned |
| V1R1 | Rename phase-coded files, tests, scripts, and commands | A human-readable codebase without changing behavior |
| V1R2 | Remove obsolete files and completed planning residue | A smaller repository containing product code, contracts, tests, examples, and permanent docs |
| V1R3 | Finalize WOML package, repository, and version identity | One public `woml` package shape at version 1.0.0, still unpublished |
| V1R4 | Rebuild the documentation and example experience | A new user can install, learn, run, operate, and troubleshoot WOML from the repository |
| V1R5 | Make local builds deterministic and release-shaped | Local builds produce exactly the files intended for publication |
| V1R6 | Harden CI and cross-platform release automation | Every supported native target is built and verified without accidental publication |
| V1R7 | Produce and test the v1.0.0 release candidate | A clean packed artifact passes real installation and workflow journeys |
| V1R8 | Perform the final security, legal, and product review | A signed release checklist with no unresolved blocker |
| V1R9 | Configure npm and publish together | Verified `woml@1.0.0` and native packages available to users |

## 7. Implementation Phases

### V1R0 — Freeze release contracts and create the audit

Status: completed on 2026-08-17. The evidence-backed release ledger is
[`docs/release/v1.0.0-readiness-audit.md`](docs/release/v1.0.0-readiness-audit.md).

Changes:

1. Create `docs/release/v1.0.0-readiness-audit.md`.
2. Record the current state of manifests, versions, public names, package
   contents, GitHub remotes, workflows, documentation, test suites, examples,
   ignored files, licenses, and product-name residue.
3. Classify every finding as `keep`, `rename`, `rewrite`, `move`, `delete`, or
   `defer` and give it an evidence path.
4. Freeze the supported v1.0.0 user journeys and platform matrix.
5. Freeze the deferred list so missing future work is not accidentally
   advertised as part of v1.0.0.
6. Record baseline commands and results before cleanup, including frontend
   tests, CLI tests, Rust tests, type-checking, architecture separation, and
   current package dry-run contents.

The audit explicitly covers the current 75 Rust integration-test files, 86 CLI
test files, 25 frontend test files, 49 verification scripts, 17 completed root
implementation plans, and all phase-coded package scripts. Counts may change
as the audit refines the inventory; file identity and coverage matter more than
the initial numbers.

Result:

The repository has one reviewed release ledger. No file is renamed or deleted
until it has a recorded disposition, and no npm action occurs.

Gate:

- every tracked and ignored root-level artifact is classified;
- every user-facing feature is represented in the release matrix;
- package and platform names are explicit; and
- baseline verification results are attached to the audit.

### V1R1 — Rename files, tests, scripts, and commands professionally

Status: completed on 2026-08-17. The codebase now uses behavior-based test,
script, fixture, protocol-note, and package-command names. Compatibility
versions and immutable fixture bytes were preserved.

Changes:

1. Rename phase-coded Rust tests such as `ri4_retry_runtime.rs` and
   `wc7_workflow_calls.rs` to behavior names such as
   `retry_runtime.rs` and `workflow_call_release.rs`.
2. Rename phase-coded CLI tests such as `pro6-runtime-inspector.test.ts` to
   names such as `runtime-inspector.test.ts`.
3. Rename milestone-based verification and benchmark scripts to feature names,
   for example `verify-production-runtime-release.ts` to `verify-production-release.ts`.
4. Replace package commands such as `test:retry-release`, `test:services-release`, and `test:production-runtime-release`
   with stable feature commands such as `test:retries`, `test:services`, and
   `test:production-runtime`.
5. Remove phase labels from test titles, diagnostic comments, and CI step names
   where they have no runtime or compatibility meaning.
6. Rename phase-residue documentation such as protocol notes containing
   `sc8`/`sc9` when the phase code is not part of a frozen external identifier.
7. Update all manifest, script, workflow, and documentation references in the
   same change as each rename.

Names that represent real protocol versions, event versions, model versions,
schema versions, or migration generations are retained. A file named
`run-event.v14.schema.json` is professional compatibility history; a file named
`verify-production-runtime-release.ts` is implementation-phase residue.

Result:

Tests and scripts explain what they verify without requiring knowledge of the
project roadmap. Coverage and behavior remain unchanged.

Gate:

- the old and new test inventories contain the same behavioral coverage;
- no manifest or workflow points to a missing file;
- `rg` finds no unapproved phase-code filenames or test titles; and
- frontend, CLI, and Rust test discovery still finds every expected suite.

### V1R2 — Remove obsolete files and completed planning residue

Status: completed on 2026-08-17.

Changes:

1. Review every root `*Implementation Plan.md` file against permanent docs.
2. Move still-authoritative language, runtime, security, recovery, and protocol
   decisions into `docs/`, then remove the completed plan from the release
   tree.
3. Review the completed legacy-removal audit and provider-extension notes.
   Preserve useful architecture facts in permanent docs; delete or move only
   after that preservation is verified.
4. Remove obsolete local scratch documents, retired scripts, duplicated docs,
   dead fixtures, and generated artifacts identified by V1R0.
5. Keep `CONTRIBUTING.md`, `LICENSE`, `NOTICE.md`, editor assets, active GitHub
   workflows, source code, schemas, protocol documents, tests, and curated
   examples.
6. Simplify `.gitignore` around actual generated/runtime data. Do not add
   `examples/` to `.gitignore`.
7. Verify that no removed file is imported, executed by a package script,
   referenced by CI, or linked from permanent documentation.

Deletion is performed as a separate reviewed change after the audit. Git
history provides historical implementation plans; the release branch should
show the product as it exists, not every milestone used to build it.

Result:

The repository root is concise and professional. Examples remain visible and
copyable, while completed internal planning and dead artifacts no longer
distract users or contributors.

Gate:

- every deletion has audit evidence;
- all documentation links resolve;
- all builds and tests pass after deletion; and
- a clean clone contains no runtime state, secrets, build output, or local
  databases.

### V1R3 — Finalize package, repository, and version identity

Status: completed on 2026-08-17. The GitHub repository rename and local
`origin` cutover remain an owner-controlled prerequisite before V1R9.

Changes:

1. Rename the public package in `woml-cli/package.json` from `woml-cli` to
   `woml` and keep the executable mapping `woml -> dist/cli.js`.
2. Rename the private frontend package in `woml/package.json` to
   `@woml/compiler`, retain `private: true`, and update internal dependency and
   import references.
3. Keep the root package private and non-publishable.
4. Align the release family to `1.0.0`: the public package, generated native
   packages, native crate release metadata, engine crate metadata where exposed,
   release checks, and user-visible CLI version.
5. Give the VS Code extension its independently reviewed v1 release version
   and verify its WOML repository and documentation links.
6. Replace live `woml-cli` install instructions and stale `v0.1` product
   references. Do not rewrite immutable schema identities.
7. Prepare repository rename instructions and update the local `origin` only
   after the GitHub repository has actually been renamed by the owner.
8. Regenerate lockfiles only through their package managers and review the
   dependency diff.

Result:

The source tree consistently describes WOML v1.0.0 and builds one public
package named `woml`, but nothing has been sent to npm.

Gate:

- `woml --version` reports `1.0.0` from a local release build;
- tag/version validation accepts `v1.0.0` and rejects mismatches;
- the private compiler cannot be published accidentally;
- the root package cannot be published accidentally; and
- no user-facing install command mentions `woml-cli`.

### V1R4 — Rebuild documentation and examples for users

Status: completed on 2026-08-17. The repository now has one validated newcomer
path, a v1 language and CLI reference, a curated example progression, permanent
support/security routes, and an executable documentation gate.

Changes:

1. Rewrite `README.md` as the product entry point: what WOML is, install,
   five-minute workflow, run behavior, key capabilities, architecture summary,
   editor extension, documentation map, support, and license.
2. Turn the current v0.1 language document into
   `docs/language-reference.md`, reflecting the actual v1 syntax and runtime.
3. Add `docs/getting-started.md` and `docs/cli-reference.md` using commands
   verified against the CLI parser and help output.
4. Consolidate overlapping service, trigger, runtime, deployment, security,
   recovery, and provider documents into a navigable documentation structure.
5. Keep low-level protocol and schema documents for maintainers, but separate
   them from the first-time author path.
6. Curate examples into a small progression: hello/manual, webhook, conditions,
   parallel/fork, retries, approval, modules, services, workflow calls, durable
   state, communication, and production deployment.
7. Make every documented example validation-tested. Remove placeholder IDs,
   tokens, personal values, future dates, and machine-specific paths.
8. Update `CONTRIBUTING.md`, issue templates, support/security guidance, and the
   VS Code extension README to match the final package and repository identity.

Result:

A first-time author can move from installation to a running workflow without
reading architecture or implementation plans. An operator and contributor can
find the deeper information without searching the repository.

Gate:

- all internal Markdown links pass;
- every `.woml` example passes `woml check` without private secrets;
- commands in the quick start work from a clean temporary project; and
- README installation uses only `bun add --global woml`.

### V1R5 — Make local builds deterministic and release-shaped

Changes:

1. Make the top-level build, type-check, test, lint/format, architecture, pack,
   and release-check commands concise and descriptive.
2. Preserve one Cargo job (`-j 1`) in repository scripts and CI to avoid the
   memory pressure already observed during development.
3. Ensure `bun install --frozen-lockfile` and Cargo `--locked` are used at
   release boundaries.
4. Build the TypeScript frontend once into the public bundle; do not publish or
   load a second compiler implementation.
5. Verify that source maps, shebangs, executable permissions, worker paths,
   provider hosts, Slack assets, licenses, and native loader metadata survive
   packaging.
6. Replace broad package inclusion with an explicit `files` allowlist and
   inspect the packed file list, unpacked size, dependency graph, and absence of
   secrets/runtime data.
7. Add one release-check command that runs the required suites in a stable
   order and reports a readable failure summary.

Result:

Local release preparation creates the same platform-neutral package layout used
by CI. `npm pack --dry-run`/`bun pm pack --dry-run` shows only intended files.

Gate:

- two clean builds from the same commit have equivalent package contents apart
  from accepted archive metadata;
- the packed CLI starts without the repository source tree;
- package contents contain no `.woml` state, databases, tokens, or tests; and
- architecture separation remains enforced.

### V1R6 — Harden CI and cross-platform release automation

Changes:

1. Expand `.github/workflows/ci.yml` into named frontend, CLI, Rust, architecture,
   documentation/example, and package-contract gates.
2. Refactor `.github/workflows/release.yml` so validation and the complete
   artifact matrix can run without publishing.
3. Build and load-test all eight native targets on matching Windows, macOS,
   Linux glibc, and Linux musl environments.
4. Verify the collected package family has one exact version, the correct
   `os`/`cpu`/libc constraints, licenses, checksums, and expected N-API exports.
5. Upload immutable main/native artifacts so the publish job consumes the
   already-tested artifacts rather than rebuilding source.
6. Add concurrency protection, minimal permissions, pinned major action
   versions, failure diagnostics, and artifact retention appropriate for a
   public release.
7. Add an `npm-production` environment to the publish job. Publication requires
   an explicit final invocation and environment approval; tag pushes alone do
   not publish accidentally.
8. Create the GitHub release only after the npm family succeeds.

Result:

GitHub Actions can prove cross-platform release readiness repeatedly without
changing npm. The publish path exists but remains unconfigured/uninvoked until
V1R9.

Gate:

- the full matrix passes from the intended release commit;
- every artifact is downloadable and locally inspectable;
- a failed target prevents publication;
- the workflow has no release token embedded in source; and
- a normal push or pull request cannot invoke `npm publish`.

### V1R7 — Produce and test the v1.0.0 release candidate

Changes:

1. Produce the complete main/native package family from one commit through the
   non-publishing release workflow.
2. Install the packed `woml` package in clean temporary environments for every
   platform available in CI.
3. Verify `woml --version`, `woml --help`, `woml check`, foreground manual run,
   webhook/event activation, background runtime, logs, inspect/list/get/cancel,
   secrets, backup, and prune.
4. Run one representative workflow for control flow, retry, approval,
   modules/custom definitions, services, workflow call/start, lifecycle,
   durable state, and each supported built-in communication adapter contract.
5. Confirm errors have stable codes, source locations where applicable, safe
   secret redaction, and useful recovery guidance.
6. Test upgrade/reinstall behavior and unsupported-platform diagnostics.
7. Record all results and remaining accepted limitations in the release audit.

Result:

There is a release candidate that behaves like the package a user will install,
without having published any version to npm.

Gate:

- clean installation does not depend on the monorepo;
- all supported user journeys pass;
- all known limitations are documented rather than hidden; and
- no release-blocking item remains open in the audit.

### V1R8 — Final security, legal, and product review

Changes:

1. Scan tracked source, examples, package archives, source maps, and workflow
   logs for secrets, personal IDs, private URLs, machine paths, and credentials.
2. Review dependency advisories and licenses; document accepted risk rather
   than silently ignoring it.
3. Verify `LICENSE`, `NOTICE.md`, package license fields, repository links,
   author/publisher metadata, support route, and security-reporting route.
4. Review default network exposure, webhook/event control-token behavior,
   provider callback verification, secret storage/redaction, file permissions,
   and runtime-data defaults.
5. Freeze the exact release commit and final artifact checksums.
6. Convert or remove the temporary readiness audit only after every item is
   closed and the permanent documentation carries all user-facing limitations.
7. Prepare the final release notes from actual v1.0.0 behavior.

Result:

The exact v1.0.0 commit and artifact set are approved for publication. This is
the final stop point before any npm-side action.

Gate:

- product owner approves the release candidate;
- security/legal review has no unresolved blocker;
- the working tree is clean;
- CI and the release-candidate matrix are green; and
- artifact hashes and versions match the approved commit.

### V1R9 — Configure npm and publish together

This is intentionally the last phase. It requires the product owner and will
not be executed automatically while earlier phases are being implemented.

Changes:

1. Recheck that `woml` and the required `@woml/cli-*` names are available or
   owned by the correct npm account.
2. Create/confirm the `@woml` scope and package permissions.
3. Configure npm trusted publishing for the exact GitHub repository and
   `.github/workflows/release.yml`; use the protected `npm-production`
   environment.
4. Run the publication workflow for the approved v1.0.0 commit and artifacts.
5. Publish native packages first, then publish `woml@1.0.0` only after the full
   native family succeeds.
6. Install from the public registry on clean Linux, macOS, and Windows runners
   and repeat the version/help/hello-workflow smoke test.
7. Verify npm provenance, package links, README rendering, executable mapping,
   optional native dependency resolution, and public artifact contents.
8. Create the GitHub v1.0.0 release and publish the reviewed VS Code extension.
9. Only after public WOML installation succeeds, deprecate `cronflow` with a
   concise migration message; do not unpublish it.
10. Monitor first-install failures and prepare v1.0.1 only for concrete release
    defects.

Result:

`bun add --global woml` installs the public v1.0.0 product and `woml run`
executes a real workflow with the correct native engine.

Gate:

- `npm view woml@1.0.0` shows the approved metadata and provenance;
- registry installs select the correct native package on supported platforms;
- the public hello workflow succeeds from a clean directory;
- GitHub and VS Code release pages point to the final documentation; and
- the old `cronflow` package redirects users without being deleted.

## 8. Release Verification Matrix

The final candidate must cover these product areas at least once:

| Area | Required evidence |
| --- | --- |
| Language frontend | parse, validate, compile, references, diagnostics, source locations |
| Execution | sequential DAG, script worker, context, results, durable events, recovery |
| Control flow | choose, switch, parallel, fork/branch/join |
| Reliability | retry/idempotency, timeout, approval, cancellation, lifecycle |
| Triggers | manual, webhook, schedule, interval, event, supported communication triggers |
| Services | HTTP/fetch, database, storage, cache, events, durable state |
| Composition | modules, custom steps/providers, workflow `call` and `start` |
| Operations | background runtime, logs, inspect, list/get/cancel, backup, prune |
| Presentation | color/plain/JSON output, readable failures, bounded results |
| Editor | VSIX install, `.woml` grammar, snippets, file icon, keyword highlighting |
| Packaging | clean pack/install, licenses, native selection, no source-tree dependency |
| Security | secret redaction, callback verification, auth warnings, no leaked credentials |

## 9. Deferred Beyond v1.0.0

The following items remain deliberate post-release work and are not blockers:

- provider extension architecture for third-party communication providers;
- additional communication providers beyond the v1 built-in set;
- postponed module-system package/security phases;
- package registry and community ecosystem;
- additional infrastructure adapters such as external queues and stores;
- advanced workflow control and remote administration;
- advanced web observability and analytics;
- performance profiling and benchmark-gated Rust `quick-xml` investigation; and
- any complete compiler migration from TypeScript/Bun to Rust.

TypeScript/Bun remains the single authoritative WOML compiler for v1.0.0. Rust
remains the durable execution authority. No release cleanup may weaken that
boundary.

## 10. Definition of Done

WOML v1.0.0 is complete only when:

- the release audit has no unresolved blocker;
- phase-coded implementation residue is absent from public-facing names;
- obsolete files and completed plans have been reviewed and removed safely;
- one public package named `woml` owns the `woml` executable;
- all release-family versions are correct and traceable;
- documentation and examples describe the shipped product accurately;
- local, CI, package, cross-platform, security, and clean-install gates pass;
- npm publication was performed only in V1R9 with the product owner present;
- a fresh registry installation executes a workflow successfully; and
- first-release feedback has a clear issue/support path.
