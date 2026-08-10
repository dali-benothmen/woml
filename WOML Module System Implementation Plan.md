# WOML Module System Implementation Plan

Status: MS0 through MS4 completed on 2026-08-10. The canonical `<woml>`
document, local-module source profile, diagnostics, resolver, deterministic ESM
bundles/source maps, generated declarations, Definition Package v2, Model v9,
`woml check`, Package v3, Script Host v5, isolated module execution, and
reviewed fixtures are frozen and implemented. Durable artifact recovery, Script
Host v6 re-registration/source maps, bounded caches, safe progress, secret
redaction, and control-flow/trigger composition are implemented. MS5 locked
package dependencies are next.

## 1. Product Outcome

The WOML Module System lets an author write reusable JavaScript or TypeScript
once and call it from any workflow step without copying functions or installing
the legacy JavaScript chaining SDK.

The canonical authoring journey is:

```xml
<woml>
  <imports>
    <module name="spreadsheet" from="./modules/spreadsheet.ts" />
  </imports>

  <workflow id="customer-import" name="Customer import" version="1.0.0">
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="cleanRows" name="Clean spreadsheet rows">
        <script>
          const rows = await services.spreadsheet.read(context.trigger.file);
          return services.spreadsheet.removeEmptyRows(rows);
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

The imported module uses ordinary named ESM exports:

```ts
export async function read(file: string) {
  // Read and parse the supplied file.
}

export function removeEmptyRows(rows: unknown[][]) {
  return rows.filter(row => row.some(value => value !== null && value !== ''));
}
```

The module is declared once and appears beneath one deeply read-only
`services.<moduleName>` namespace in every script in that workflow. Authors do
not declare whether a module is pure, effectful, Bun-backed, or Rust-backed.
WOML observes actual calls to native Fetch and managed built-ins at the runtime
boundaries that already exist.

This milestone imports code, not workflows. A `.woml` workflow remains an
independently activated definition and cannot be inserted into `<steps>` through
`<module>`. Independent workflows communicate through events today. Durable
call-and-return workflow communication is the next roadmap milestone through
`services.workflows.call()`.

## 2. Frozen Product Direction for MS0

These product choices are no longer open alternatives. MS0 freezes their exact
contracts and rejects competing spellings or shapes.

### 2.1 Canonical WOML document

Every WOML source uses `<woml>` as its document root:

```xml
<woml>
  <imports>...</imports>
  <workflow ...>...</workflow>
</woml>
```

Rules:

- `<woml>` has no attributes in the first profile.
- `<imports>` is optional and, when present, appears before `<workflow>`.
- Exactly one `<workflow>` is required.
- The workflow keeps its `version` attribute as business metadata.
- `<woml>` does not introduce a competing language or document `version`.
- Direct `<workflow>` roots are migrated while WOML is pre-release rather than
  becoming a permanent second document grammar.
- The wrapper is used even when a document has no imports, so authors and tools
  do not need to understand two source shapes.

### 2.2 Canonical module declaration

Imports use exactly:

```xml
<imports>
  <module name="spreadsheet" from="./modules/spreadsheet.ts" />
</imports>
```

Rules:

- `<module>` is empty and accepts exactly `name` and `from` in v1.
- `name` is a JavaScript-safe alias and must be unique in the document.
- `from` initially resolves a local `.js` or `.ts` source.
- Paths are static and relative to the importing WOML source.
- Absolute paths, home-relative paths, remote URLs, runtime-computed paths, and
  paths escaping the reviewed project boundary are rejected.
- `<require>`, `<import>`, and a workflow-owned `<imports>` container are not
  aliases.
- `.woml` files are not accepted by `<module>`.

The five published built-in service names cannot be used as module aliases:

```text
http
db
storage
cache
events
```

`queue` and `workflows` are also reserved for future WOML-owned services. They
are not custom-module aliases.

### 2.3 Export surface

The first profile exposes named function exports only:

```js
export function parse(input) { ... }
export async function write(output) { ... }
```

The following are rejected in v1 with source-located diagnostics:

- `export default`;
- exported mutable values or arbitrary JSON constants;
- classes and constructors;
- nested namespace objects;
- generators, symbols, or callable proxies; and
- CommonJS `module.exports` or `exports.*` surfaces.

Type-only TypeScript exports are allowed in source but do not become runtime
members of `services.<moduleName>`. A module with no accepted runtime function
exports is invalid.

### 2.4 Calling a module

An alias is available only through `services`:

```js
const rows = await services.spreadsheet.read(file);
```

Direct globals such as `spreadsheet.read()` are not injected. Imported
namespaces are deeply read-only and cannot shadow built-ins, `context`,
`attempt`, `secrets`, another alias, or a future reserved binding.

### 2.5 Workflow boundary

A `.woml` file describes an independently activatable workflow definition. A
trigger occurrence creates a durable run instance; the source file itself is
not a run instance.

The Module System does not import or inline runnable workflows. That avoids
confusing three different operations:

- importing reusable code;
- publishing an event that starts zero, one, or many independent workflows;
  and
- calling exactly one workflow and durably waiting for its result.

`services.events.emit()` uses the fast direct-Rust path for subscribers loaded
in the same runtime, passes its payload through each subscriber's
`context.trigger`, and does not wait for subscriber results. Separate runtime
processes can currently receive events through their authenticated public event
endpoints, but internal emit does not discover them automatically.

The final operation belongs to the Durable Workflow Calls milestone after this
plan. It adds automatic same-runtime and cross-process routing without treating
a workflow as an imported file.

## 3. Product Principles

### 3.1 Imports are build-time dependencies

`woml run` must never recover an old run by reopening whatever currently exists
at `./modules/spreadsheet.ts`. The frontend resolves, compiles, hashes, and
packages the complete dependency graph before activation.

A changed source produces a new definition package and definition hash. An
existing run stays bound to the exact module artifact it started with.

### 3.2 One namespace, no engine vocabulary

Authors use `services.<moduleName>.<function>()`. They never write
`kind="capability"`, an execution backend, handler name, event type, or Rust
tracking field.

### 3.3 Track operations, not module categories

Pure module functions remain local Bun calls. A module's native `fetch()` is
instrumented. Calls from a module to a built-in `services.*` capability cross
the existing managed-operation boundary. A module may contain both without a
user-authored classification.

The surrounding script attempt remains the durable unit for arbitrary local
JavaScript. Rust does not record every string transform, CSV cell, or helper
function call.

### 3.4 Load artifacts once; share no mutable run state

WOML may compile and cache an immutable module artifact once. Every isolated
script invocation receives fresh module state or an equivalent isolation
guarantee. A module-level variable from one attempt or run cannot affect
another.

### 3.5 No effects during initialization

Top-level module code may define functions and perform bounded pure
initialization. It may not call Fetch, a managed service, or another external
effect while the module loads.

This is invalid:

```js
const customer = await fetch('https://example.com/customer');
```

Effects become available only while an exported function is invoked from an
active script attempt.

### 3.6 Local first, reproducible packages later

The vertical slice imports one local `.ts` file. Later phases add transitive
dependencies, deterministic bundles, permissions, offline deployment
artifacts, and package inspection. A public marketplace is not required for
local modules to become useful.

### 3.7 Useful compatibility, not arbitrary npm compatibility

WOML supports a deterministic ESM-oriented JavaScript/TypeScript subset.
Packages that require install scripts, native addons, unrestricted processes,
or runtime registry access fail before workflow activation.

## 4. Baseline Entering This Milestone

SC14 provides the runtime boundaries reused by modules:

- read-only `services` and source-proven `secrets` bindings;
- native Bun Fetch instrumentation;
- Capability Call v1 and Script Host v4 full-duplex calls;
- durable generic operation events and `WomlServiceError`;
- cancellation, limits, redaction, and stable operation identity;
- the Rust capability registry; and
- built-in HTTP, database, storage, cache, and internal events services.

The intended direction is:

```text
.woml + local/package JS/TS dependencies
  -> TypeScript resolver and WOML compiler
  -> immutable definition package + bundles + source maps
  -> Rust stores and validates exact artifact identities
  -> long-lived Bun host caches immutable bundles by digest
  -> isolated Worker instantiates modules for one script attempt
  -> named exports appear under services.<alias>
  -> native Fetch / built-in effects reuse the SC14 boundaries
  -> final script result returns to the existing Rust DAG
```

If modules require a protocol shape that SC14 cannot express, this milestone
creates a new version. It does not mutate a frozen Services artifact.

## 5. What “Done” Means

The Module System is complete when:

1. Every WOML source uses the canonical `<woml>` document root.
2. A workflow can declare a local `.js` or `.ts` module once and call its named
   functions through `services.<alias>` from multiple steps.
3. Parser, alias, path, export, and source-location rules are frozen and
   conformance-tested.
4. Pure module functions execute locally in Bun without unnecessary Rust RPC.
5. Fetch and built-in service calls inside modules use the same tracked
   boundaries as inline script calls.
6. Module initialization cannot create external effects.
7. Every invocation has isolated mutable module state.
8. Module failures and stacks point to the original module path, line, and
   column.
9. The complete transitive dependency graph is hashed, locked, bundled, and
   stored in an immutable definition package.
10. Recovery uses stored artifacts and never current filesystem code.
11. Supported package dependencies execute without production access to the npm
    registry or the author's `node_modules` directory.
12. Native addons, install scripts, remote/dynamic imports, default exports, and
    unsupported package shapes fail with actionable diagnostics.
13. Installed packages cannot silently enumerate project secrets or bypass
    reviewed capability and network policy.
14. CLI commands can check, inspect, bundle, test, and explain the dependency
    graph without activating a production workflow.
15. Workflows with no imports preserve their existing compiled execution
    semantics after the document-root migration.
16. Local and package-backed modules pass clean-package deployment, crash,
    recovery, compatibility, and secret/artifact scans.

## 6. Scope

### Included

- The canonical `<woml>` document wrapper.
- `<imports><module name="..." from="..." /></imports>`.
- Local JavaScript and TypeScript modules.
- Named function export discovery.
- One deeply read-only `services.<alias>` namespace per module.
- Static relative dependency resolution.
- Deterministic TypeScript/JavaScript compilation and source maps.
- Module graph validation, cycle/collision detection, hashing, and lock data.
- A content-addressed immutable artifact store and runtime bundle cache.
- Per-invocation module-state isolation.
- Top-level effect prevention.
- Package dependencies that can be deterministically bundled without native
  addons or install scripts.
- Secret, capability, network, storage, and supply-chain permissions for
  installed packages.
- Type declarations and autocomplete for imported namespaces.
- CLI validation, inspection, testing, bundling, packaging, documentation, and
  migration guidance.

### Not included

- Importing `.woml` files.
- Reusable WOML components or custom workflow tags.
- Inline DAG expansion from another workflow definition.
- `services.workflows.call()` or parent/child workflow runs; that is the next
  milestone.
- A public hosted WOML package marketplace.
- Arbitrary npm lifecycle scripts or unreviewed CommonJS compatibility.
- Native addons, executable binaries, child processes, raw FFI, or unrestricted
  filesystem/environment/socket access.
- Remote `http://` or `https://` module sources.
- Runtime dependency installation or registry access.
- Unpinned semver ranges in an activated definition.
- Shared mutable module singletons across runs.
- Default exports, classes, runtime namespace objects, or dynamic imports.
- A user-facing pure/capability module classification.
- Hot replacement of an active definition. A future watch experience still
  produces a new immutable definition while existing runs keep old artifacts.
- General multi-tenant OS isolation; Production Runtime owns that boundary.

## 7. Code Module Runtime Contract

### 7.1 Export discovery

The frontend parses the complete ESM graph and records accepted named runtime
functions. Export discovery is static and deterministic. Module profile v1
rejects star re-exports, renamed re-exports, export lists, default exports, and
package export maps; public functions use direct declarations only.

Runtime namespaces are frozen:

```js
services.spreadsheet.parse;
services.spreadsheet.write;
```

They cannot be mutated, extended, reassigned, or returned as JSON workflow
data.

### 7.2 Initialization and invocation

For each script attempt, the Worker:

1. verifies the artifact digest;
2. creates fresh isolated module state;
3. evaluates bounded top-level initialization with effects disabled;
4. creates the read-only namespace from accepted exports;
5. invokes functions when the script calls them;
6. enables Fetch and built-in capabilities only during active invocation; and
7. destroys invocation-local state when the attempt ends.

Module initialization and function execution share the script attempt's
timeout, cancellation, memory, and host-crash boundary. A module cannot keep
background work alive after the attempt completes.

### 7.3 Arguments, results, and bindings

Local module calls may temporarily use ordinary in-memory JavaScript values.
JSON constraints apply when data crosses an existing durable boundary:

- the final script return;
- native Fetch observations;
- managed `services.*` inputs and results; and
- durable workflow context.

Module functions should receive workflow data and secrets as explicit
arguments:

```js
export async function loadCustomer(customerId, token) {
  return fetch(`https://crm.example.com/customers/${customerId}`, {
    headers: { authorization: `Bearer ${token}` },
  }).then(response => response.json());
}
```

Caller:

```js
const customer = await services.crm.loadCustomer(
  context.trigger.customerId,
  secrets.CRM_TOKEN
);
```

Modules do not automatically receive `context`, `attempt`, an enumerable
`secrets` object, or a hidden runtime helper in v1. Callers pass values and
individual secrets explicitly. Installed packages must never enumerate the
project secret store.

### 7.4 Errors and source maps

A module failure is still a failure of the invoking step attempt. Diagnostics
must identify:

- importing WOML file and `<module>` declaration;
- module path and original line/column;
- exported function;
- caller workflow and step; and
- safe cause/code without source, secret, or body leakage.

Bundling and TypeScript transpilation retain deterministic source maps. Rust
stores safe artifact identity; Bun maps runtime frames back to original source.

## 8. Resolution, Hashing, and Immutable Artifacts

### 8.1 Static dependency graph

The resolver begins at the WOML document and follows only reviewed static
edges. It rejects missing files, unsafe paths, case collisions, symlink escapes,
unsupported extensions, ambiguous aliases, graph cycles, and configured graph
or byte-limit violations.

Absolute developer-machine paths never become portable artifact identity.

### 8.2 Definition package

Before activation, the frontend creates a deterministic package containing:

- canonical compiled workflow model;
- canonical module graph and aliases;
- source and bundle content digests;
- immutable executable bundles;
- source maps and safe source identities;
- compiler/Bun compatibility metadata;
- lock and package integrity data; and
- reviewed permission requirements.

Rust validates and stores this package with the workflow definition. Existing
runs remain bound to it after source files change or disappear.

### 8.3 Runtime artifact cache

The Bun host may cache verified immutable bundles by digest. Cache eviction may
affect performance but not correctness. Rust or durable definition storage can
re-register the exact artifact without consulting current project files.

Cache size, artifact size, graph size, compile time, initialization time, and
memory are bounded before publication.

### 8.4 Package dependencies

Supported packages resolve to exact versions and integrity hashes. Production
execution uses prebuilt artifacts and performs no registry lookup or install.

Local-only MS1 has no package lock input. MS5 consumes exact package resolution
from `bun.lock` and copies the resolved versions and integrity into the
immutable definition package; it does not introduce a second author-maintained
lockfile in the first package profile. Unpinned, tampered, script-requiring,
native-addon, or unsupported dependency graphs fail before activation.

## 9. Permissions, Secrets, and Supply Chain

Local author code and installed packages may use different trust defaults, but
authors never declare an engine category.

The permission contract must cover controlled access to:

- native Fetch/network origins;
- managed HTTP;
- database connections or connection classes;
- storage namespaces;
- cache namespaces;
- internal event names; and
- specifically named secrets.

Requirements:

- packages cannot enumerate secrets;
- declared secret names and permission requirements may be stored, values may
  not;
- dependency updates cannot silently widen permissions;
- denied permissions fail before or at the controlled doorway;
- module errors, source maps, bundles, locks, logs, and events are scanned for
  credential leakage; and
- package provenance and integrity are inspectable before activation.

Permissions reduce accidental and supply-chain access. They do not make trusted
JavaScript a complete multi-tenant sandbox; hosted isolation remains a
Production Runtime responsibility.

## 10. CLI and Developer Experience

MS0 freezes `woml check <file> [--json]`; later phases may add dedicated build,
test, and dependency commands without changing that read-only contract. The
completed module experience must support:

- checking WOML/module syntax without executing a workflow;
- explaining aliases, paths, exports, transitive dependencies, digests, and
  permissions;
- building a deterministic deployment artifact;
- testing an exported module function with mocked built-ins;
- updating and reviewing locked dependencies;
- showing why a package or export is unsupported;
- identifying unused module declarations; and
- mapping runtime errors to original TypeScript/JavaScript source.

Editor support should generate declarations similar to:

```ts
services.spreadsheet.read;
services.crm.loadCustomer;
```

AI-generated WOML receives the same source-located diagnostics as human-authored
files. Unknown elements, attributes, aliases, and exports are never guessed.

## 11. Versioned Artifacts to Review in MS0

At minimum, MS0 reviews and pins:

1. canonical `<woml>/<imports>/<module>/<workflow>` document schema;
2. module alias, path, and extension grammar;
3. accepted ESM/named-function export surface;
4. module graph identity, limits, and cycle rules;
5. definition-package manifest and canonical hash inputs;
6. artifact, source-map, and cache identity;
7. dependency lock and integrity data;
8. Bun compiler/runtime compatibility contract;
9. Script Host artifact registration/cache/invocation messages, using a new
   protocol version when necessary;
10. package permission and secret-grant contract;
11. module diagnostics/source-location schema; and
12. representative source, bundle, source-map, model, package, recovery, and
    invalid-graph fixtures.

Run Event changes are not assumed. If existing generic operation events are
sufficient, module provenance stays immutable definition metadata. Any new
durable event receives its own versioned review.

## 12. Implementation Phases

### MS0 — Freeze the Module System contracts — completed

Changes:

- Audit SC14's model, Script Host, capability, secrets, Fetch, and artifact
  persistence boundaries.
- Freeze the canonical document, module declaration, named-export, alias, path,
  resolution, hashing, locking, permission, error, and limit contracts.
- Freeze the migration from direct `<workflow>` roots to `<woml>` documents.
- Add reviewed valid and invalid fixtures before runtime code.
- Keep `.woml` imports and Durable Workflow Calls explicitly outside scope.

Result:

Every layer agrees on exactly what a module is and how it remains reproducible,
but modules are not executable yet.

Gate:

Schemas and fixtures resolve every syntax/protocol/hash/permission shape in
Sections 2–11, reject competing alternatives, and do not mutate frozen SC14
artifacts.

### MS1 — Activate the document grammar and immutable resolver — completed

Changes:

- Parse and validate the canonical `<woml>` root, optional ordered `<imports>`,
  `<module>` declarations, and one `<workflow>`.
- Migrate project examples and fixtures from direct workflow roots.
- Resolve local files from their importing source with safe portable identity.
- Build a deterministic acyclic dependency graph.
- Validate aliases, extensions, paths, files, collisions, and exports.
- Canonicalize and hash source, compiler, graph, and permission inputs.
- Produce the reviewed definition-package manifest without executing code.

Result:

WOML can validate, explain, and package a local module graph.

Gate:

Reviewed sources deep-equal their parser/graph/package fixtures. Content changes
alter the root hash; timestamps, discovery order, cache location, and irrelevant
filesystem metadata do not.

### MS2 — Compile local JavaScript and TypeScript modules — completed

Changes:

- Parse the reviewed ESM subset and named function exports.
- Reject default/CommonJS/dynamic/unsupported exports precisely.
- Transpile and bundle TypeScript deterministically.
- Retain verified source maps.
- Lower aliases and artifact digests into the new compiled Model profile.
- Generate declarations for `services.<alias>`.

Result:

A WOML document with a local module compiles into an immutable executable
definition package, while Rust still rejects module execution until MS3.

Gate:

Source, model, bundle, map, and package fixtures match exactly; syntax, export,
collision, dynamic import, cycle, and TypeScript failures point to their source.

### MS3 — Execute local modules under `services.*` — completed

Changes:

- Register immutable bundles with the long-lived Bun host by digest.
- Instantiate fresh module state in each isolated Worker.
- Attach named exports to a deeply read-only alias namespace.
- Disable effects during initialization and enable tracked Fetch/built-ins only
  during function invocation.
- Preserve local JavaScript values until an existing JSON boundary.
- Apply script timeout, cancellation, crash, input/result, and memory limits.

Result:

One local TypeScript module is declared once and called from two sequential
workflow steps through `services.<alias>`.

Gate:

The vertical slice proves sync/async functions, multiple exports/steps, Fetch,
managed services, no shared state, initialization rejection, timeout,
cancellation, host/worker crash distinction, and exact output.

### MS4 — Make local modules recoverable and observable — completed

Changes:

- Recover using stored artifacts after original sources change or disappear.
- Re-register bundles after Bun host restart and verify every digest.
- Add source-mapped failures and safe CLI progress.
- Enforce graph/artifact/frame/memory/startup/cache bounds.
- Compose imported functions with sequential, branch, parallel, approval,
  retry, every production trigger, and all five published services.
- Scan artifacts, maps, failures, logs, and history for secrets.

Result:

Local modules have the same recovery, isolation, composition, and diagnostic
quality as inline scripts.

Gate:

Crash tests cover registration, initialization, pure execution, Fetch/service
calls, and post-script completion. Recovery never reads current project files.

### MS5 — Add locked package dependencies

Changes:

- Implement the reviewed package-manager and lock integration.
- Resolve exact versions and integrity digests before activation.
- Bundle supported dependencies for offline production execution.
- Reject install scripts, native addons, binaries, remote/dynamic imports, and
  unsupported CommonJS edges.
- Record safe license/provenance metadata.
- Build a spreadsheet acceptance module backed by a real locked package.

Result:

A local module can use a supported package dependency and deploy without
running `npm install` in production.

Gate:

Offline execution succeeds; tampering, missing lock data, range drift,
lifecycle scripts, native addons, registry outage, and transitive cycles fail
before activation.

### MS6 — Enforce permissions and complete module DX

Changes:

- Implement installed-package permission and secret grants.
- Prevent secret enumeration and detect permission expansion after updates.
- Enforce capability/network/storage policies at controlled doorways.
- Add graph inspection, build/check output, module unit testing with mocked
  built-ins, autocomplete, unused-import diagnostics, and guidance.
- Add local and package-backed examples to clean-package smoke tests.

Result:

Modules are safe to inspect and pleasant to author, test, review, and deploy.

Gate:

Permission, secret, malicious dependency, type-generation, mocked-service,
packaging, offline, and backwards-compatibility suites pass.

### MS7 — Complete packaging and distribution boundaries

Changes:

- Finalize CLI check/inspect/build/test journeys and machine-readable output.
- Produce portable definition/module bundles for CI and deployment.
- Support reviewed local path and packaged archive dependencies.
- Define, without requiring, a future registry signature/identity boundary.
- Document versions, upgrades, lock updates, permission changes, cache cleanup,
  incident response, and migration from copied scripts/legacy SDK services.

Result:

Modules move reproducibly from a developer project to CI and production without
a public marketplace.

Gate:

Builds reproduce offline across clean directories; inspection explains every
artifact, version, export, and permission; corruption fails before execution.

### MS8 — Harden and publish the Module System

Changes:

- Run adversarial graph, bundle, source-map, permission, secret, resource,
  cache, crash, and recovery suites.
- Benchmark build, cold/warm Worker startup, cache reuse, pure calls, and large
  dependency graphs.
- Update language, architecture, CLI, security, deployment, recovery,
  migration, and AI-authoring documentation.
- Add local TypeScript and package-backed spreadsheet examples to the release
  suite.
- Run the entire pre-module WOML compatibility suite unchanged after source
  fixtures are migrated to the canonical wrapper.

Result:

Local and package-backed JavaScript/TypeScript modules are supported,
documented, reproducible, and publishable.

Gate:

Frontend, Rust, Bun host, schemas/protocols, typecheck, Clippy, package,
permission, offline, source-map, compatibility, benchmark-regression, and
secret/artifact scans pass from a clean installation.

## 13. Expected File Areas

| Area                         | Expected locations                                                    |
| ---------------------------- | --------------------------------------------------------------------- |
| WOML document/import grammar | `woml/src/parser.ts`, `compiler.ts`, module syntax helpers            |
| Module graph/resolution      | new `woml/src/modules/*` resolver, identity, graph, lock, diagnostics |
| Model/package artifacts      | `woml/src/model.ts`, schemas/manifests and artifact builders          |
| JS/TS analysis/bundling      | frontend/CLI bundler integration, source maps, type generation        |
| Bun module runtime           | `woml-cli/src/script-host/*`, module loader/cache/runtime helpers     |
| Rust artifact authority      | `core/woml-engine` definition/artifact storage and host registration  |
| Permissions/secrets          | frontend analysis, CLI grants, runtime capability policy              |
| CLI DX                       | `woml-cli/src/cli.ts`, module check/inspect/build/test commands       |
| Versioned contracts          | `docs/schemas/*`, `docs/protocols/*`, model/package/module fixtures   |
| Examples                     | local TS module and package-backed spreadsheet module                 |

The TypeScript frontend understands module syntax and creates immutable
artifacts. Rust validates and stores definitions and orchestrates attempts. Bun
loads and evaluates JavaScript/TypeScript bundles in isolated invocation state.

## 14. Verification Matrix

| Area           | Required proof                                                                              |
| -------------- | ------------------------------------------------------------------------------------------- |
| Document       | One canonical root/order; legacy and competing shapes fail with migration guidance.         |
| Syntax         | One `<module>` grammar; misplaced declarations and unsupported attributes fail clearly.     |
| Resolution     | Relative/package identities are deterministic, bounded, acyclic, and source-located.        |
| Exports        | Named functions work; default/CommonJS/dynamic/unsupported exports fail before activation.  |
| Hashing        | Every transitive content change changes the definition; timestamps/absolute paths do not.   |
| Recovery       | Stored artifacts execute after source deletion/change; current files are never substituted. |
| Isolation      | No mutable module state crosses invocation or run boundaries.                               |
| Initialization | Fetch and managed effects are impossible before exported-function invocation.               |
| Namespace      | Built-ins, reserved names, aliases, and exports cannot collide or be mutated.               |
| Operations     | Module Fetch/service calls reuse SC14; pure function calls avoid Rust RPC.                  |
| Secrets        | Packages cannot enumerate secrets; values never leak into artifacts or history.             |
| Permissions    | Dependency updates cannot silently gain network/service/storage access.                     |
| Packages       | Offline bundles reproduce; scripts, native addons, and remote imports are rejected.         |
| Errors         | Stacks map to original module line and importing/caller WOML step.                          |
| Composition    | Modules work through branch, parallel, approval, retry, all triggers, and five services.    |
| Compatibility  | Workflows without imports preserve execution semantics after wrapper migration.             |
| Packaging      | Every reviewed example runs from a clean offline artifact.                                  |

## 15. Risks and Guardrails

### A familiar namespace can hide different execution costs

A pure `services.spreadsheet.parse()` stays in Bun while
`services.db().query()` crosses Rust. Documentation and inspection should show
known effect and cost information without requiring module kinds.

### Arbitrary npm compatibility would destroy reliability

Many packages assume unrestricted files, environment, sockets, native addons,
or install scripts. WOML supports a reproducible subset and gives actionable
incompatibility errors instead of pretending every package works.

### Bundles are durable workflow dependencies

Storing only a path or package version is insufficient. Artifact retention,
size, cleanup, and integrity must align with run retention so an active old run
never loses its executable code.

### Top-level module execution can hide effects

The runtime must disable controlled effect doorways during initialization, not
merely document a convention. Initialization also needs strict time and memory
bounds.

### Secrets passed as strings can be exfiltrated

Native Fetch needs real values in Bun. Explicit argument flow and permissions
reduce accidental exposure but cannot stop deliberately authored trusted code.
Hosted untrusted-code guarantees require OS isolation.

### Package permissions must stay product-oriented

Authors approve concrete access such as a network origin or named secret, not
an internal capability classification. Dependency updates that expand access
must require visible review.

### The wrapper migration is intentionally breaking

WOML is pre-release, so adopting one clean `<woml>` grammar now is cheaper than
supporting two roots forever. Migration diagnostics and mechanical tooling must
make the change straightforward.

## 16. Global Roadmap After the Module System

1. **Durable Workflow Calls** — add
   `services.workflows.call(workflowId, payload, options?)` so one workflow can
   target exactly one activated workflow by ID, create an independent durable
   child run, pass payload through the child's `context.trigger`, wait durably,
   and receive its final JSON result. No `<call>` trigger tag is required.
   Same-runtime calls use a direct Rust path; cross-process calls require
   authenticated runtime discovery/routing, unique target ownership, stable
   call identity, accepted definition hashes, timeouts, cancellation,
   idempotency, cycle/depth protection, and crash recovery. Explicit
   `return null` represents intentional no-result success; missing
   return/`undefined` fails the child call.
2. **Lifecycle and engine controls** — workflow cancellation, lifecycle hooks,
   workflow-level concurrency/rate limits/timeouts, and durable user state.
3. **Production runtime and operations** — hosting, deployment, multi-node
   ownership, OS-level isolation, observability, retention, administration,
   scaling, artifact lifecycle, and the production workflow-call router.
4. **WOML package registry/community ecosystem** — signed publication,
   discovery, trust/provenance, moderation, compatibility, and deprecation after
   local/package artifacts are proven.
5. **Additional infrastructure adapters** — the postponed durable queue and
   external broker profile, document databases, external object storage, and
   distributed caches according to demand.
6. **Additional communication providers** — Discord, WhatsApp, and Telegram
   triggers, notifications, and messaging capabilities when justified.
7. **Retire the JavaScript chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

After MS8, Durable Workflow Calls is the next product milestone.

## 17. MS0 Review Gate — passed 2026-08-10

MS0 froze the author-facing and MS1 package boundary in
`docs/protocols/module-system-v1.md`,
`docs/schemas/woml-definition-package.v1.schema.json`, and the reviewed module
fixtures. Runtime-only shapes that cannot exist before bundling retain explicit
version gates in MS2–MS5 rather than changing any frozen SC14 artifact.

The review covered:

- exact `<woml>`, `<imports>`, `<module>`, and `<workflow>` schema/order rules;
- migration behavior and diagnostics for direct `<workflow>` roots;
- module alias and named-function export grammar;
- JavaScript/TypeScript ESM subset and CommonJS rejection boundary;
- local path, symlink, case, project-root, extension, and cycle rules;
- canonical graph, definition package, hash, lock, and retention inputs;
- Bun compiler/runtime compatibility and source-map format;
- host artifact registration/cache protocol version;
- top-level initialization restrictions and resource limits;
- local-code trust versus installed-package permissions;
- explicit secret argument/grant behavior;
- package-manager and lockfile integration;
- namespace/reserved-name collision rules;
- error shape and source locations; and
- all size, graph, compile, initialization, cache, and invocation limits.

Runtime artifacts reserved for MS2–MS5 receive a reviewed new version before
their phase writes code; they cannot silently widen Definition Package v1 or a
frozen SC14 protocol. `.woml` imports and Durable Workflow Calls remain
separate milestones and cannot be introduced by convenience while the Module
System is being built.
