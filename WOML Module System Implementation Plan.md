# WOML Module System Implementation Plan

Status: directional plan created on 2026-08-09. Implementation begins only
after Services and Capabilities reaches SC14. All source examples in this
document are illustrative until MS0 reviews and freezes the language contract.

## 1. Product Outcome

The WOML Module System lets an author write reusable code once and use it across
many steps and workflows without copying files or installing the legacy
JavaScript chaining SDK.

It delivers two related products:

1. **JavaScript/TypeScript code modules** — reusable functions that appear under
   the existing `services` namespace.
2. **WOML components** — reusable workflow graphs that can be embedded and
   composed inside another `.woml` workflow with explicit inputs and one
   predictable output.

The desired local code-module journey is conceptually:

```xml
<!-- Illustrative only; exact import grammar is an MS0 decision. -->
<imports>
  <module name="spreadsheet" from="./modules/spreadsheet.ts" />
</imports>
```

followed by normal JavaScript in any step:

```js
const rows = await services.spreadsheet.read(file);
const valid = services.spreadsheet.removeEmptyRows(rows);

return { valid };
```

The module is declared once for the workflow and is available in every script.
The author does not classify it as pure, effectful, Bun-backed, or Rust-backed.
WOML tracks actual calls to native Fetch and the built-in managed services, as
defined by Services and Capabilities.

The desired WOML-composition journey is conceptually:

```xml
<!-- One possible direction, not frozen syntax. -->
<customer.load
  id="customer"
  customer-id="{{context.trigger.customerId}}" />
```

The imported component's internal steps execute through the same Rust DAG and
publish one public result at:

```js
context.steps.customer
```

Internal step IDs remain private to that component instance. The calling
workflow depends only on its declared inputs and output.

## 2. What “Module” Means

WOML must keep three concepts distinct even if one package can contain all of
them later.

### 2.1 Code module

A local or packaged JavaScript/TypeScript module exports reusable functions.
Its code executes inside an isolated Bun Worker.

Examples:

- spreadsheet and CSV helpers;
- data validation and transformation;
- templates and date utilities;
- a company-specific CRM client built using `fetch()`;
- a business abstraction composed from `services.http`, `services.db`, or
  other built-ins.

### 2.2 WOML component

A WOML component is a triggerless reusable DAG fragment with declared inputs
and a declared output. The frontend instantiates it into the caller's compiled
DAG. Rust executes ordinary compiled nodes and does not parse or interpret
module markup.

Examples:

- load and normalize a customer;
- request approval and handle both decisions;
- enrich an order in parallel;
- validate, transform, and store an uploaded dataset.

### 2.3 WOML workflow

A workflow remains an independently activated definition with triggers. Running
a directory may activate several workflows, as it does today. Importing a
component does not automatically activate another workflow.

Starting a child workflow as a separate durable run is a different product
contract from compile-time component embedding. That child-run/subworkflow
feature is explicitly deferred unless MS0 determines it is required for the
first module release.

## 3. Product Principles

### 3.1 No engine terminology in ordinary authoring

The user does not write `kind="capability"`, execution backend names, event
types, handler names, or Rust tracking metadata. WOML handles those concerns.

### 3.2 One declaration, one stable namespace

A code module receives a local alias and appears as:

```js
services.<alias>.<exportedFunction>()
```

Aliases use the reviewed JavaScript-safe identifier grammar. The six built-in
service names are reserved:

```text
http
db
storage
cache
events
queue
```

An import cannot shadow a built-in, another import, a runtime binding, or a
component instance.

### 3.3 Track operations, not module categories

Pure module functions remain local Bun calls. A module's native `fetch()` is
instrumented. A module's `services.*` call crosses Capability Call v1. A module
may contain both kinds of function without declaring a type.

The surrounding script attempt remains the durable unit for arbitrary local
JavaScript. Rust does not record every `trim()`, CSV parse, or spreadsheet cell
read.

### 3.4 Imports are build-time dependencies, not runtime path lookups

`woml run` must never resume an old run by reopening whatever code currently
exists at `./modules/spreadsheet.ts`.

The frontend resolves, compiles, hashes, and packages the complete dependency
graph. The immutable workflow definition references content-addressed module
artifacts. A changed file produces a new definition hash; an existing run keeps
the exact code it started with.

### 3.5 Load code once; share no mutable run state

WOML may parse, bundle, transpile, and cache a module artifact once. Each
isolated invocation receives a fresh module instance or equivalent isolated
state. A module-level variable from one run cannot affect another run.

### 3.6 No effects during module initialization

Module top-level code may define functions and perform bounded pure
initialization. It may not call Fetch, a managed service, or another external
effect while the module is being loaded.

This is invalid behavior even though the exact diagnostic is an MS0 decision:

```js
const customer = await fetch("https://example.com/customer");
```

Effects become legal only after a workflow step invokes an exported function.

### 3.7 Explicit component boundaries

A WOML component cannot silently capture the caller's trigger, steps, secrets,
or sibling component internals. The caller passes declared inputs. The component
publishes a declared output. This is what makes the component reusable and
reviewable.

### 3.8 Local-first, reproducible production later in the same milestone

The first vertical slice imports one local `.ts` file. Later phases add package
dependencies, deterministic bundles, permissions, and deployable module
packages. A public marketplace is not required to make local modules useful.

## 4. Dependency on Services and Capabilities

The Module System begins after SC14 because it reuses these completed contracts:

- read-only `services` and `secrets` runtime bindings;
- native Bun Fetch instrumentation;
- Capability Call v1;
- Script Host v4 full-duplex nested calls;
- generic operation events and errors;
- capability cancellation, limits, redaction, and idempotency metadata;
- the Rust capability registry; and
- built-in HTTP, database, storage, cache, events, and queue services.

Services and Capabilities must keep its protocol generic, but it must not guess
module syntax or package semantics. MS0 audits the implemented SC14 boundary.
If a new protocol field is required, the Module System creates a new version
rather than mutating a frozen Services artifact.

The intended runtime direction is:

```text
.woml + local/package dependencies
  -> TypeScript module resolver and WOML compiler
  -> immutable definition package + content-addressed bundles + source maps
  -> Rust stores/validates the exact artifact identities
  -> long-lived Bun host caches immutable bundles by digest
  -> isolated Worker instantiates modules for one script attempt
  -> module exports appear under services.<alias>
  -> native Fetch / built-in service effects use the SC capability boundary
  -> final script result returns to the existing Rust DAG
```

## 5. What “Done” Means

The Module System is complete when:

1. A workflow can declare a local `.js` or `.ts` dependency once and call its
   named exports through `services.<alias>` from multiple steps.
2. The exact import document shape, alias grammar, resolution rules, and export
   rules are frozen and source-located.
3. Pure module functions run locally in Bun without unnecessary Rust RPC.
4. Fetch and built-in service calls made inside a module use the same tracked
   operation boundaries as calls written directly in a `<script>`.
5. Module initialization cannot create external effects.
6. Module errors and stacks point to the original module path, line, and column.
7. The complete transitive dependency graph is hashed, locked, bundled, and
   stored as part of an immutable workflow definition package.
8. Restart recovery uses stored artifacts and never current filesystem code.
9. Package dependencies work without production runtime access to the npm
   registry or the author's `node_modules` directory.
10. Native addons, install scripts, remote imports, dynamic imports, and other
    unsupported dependency shapes fail with actionable diagnostics.
11. Third-party packages cannot silently enumerate every project secret or
    bypass reviewed capability/network policy.
12. A reusable `.woml` component declares inputs and one output, compiles into
    the caller's DAG, and publishes one predictable caller-visible result.
13. Component internal IDs are collision-free and private; nested components
    compose without reference leakage or DAG cycles.
14. Branch, parallel, approval, retry, triggers, service calls, storage
    references, and queue/event operations work inside component instances.
15. Definition hashes change when any transitive code or WOML dependency
    changes and remain stable when only irrelevant filesystem metadata changes.
16. CLI commands can check, inspect, bundle, test, and explain the dependency
    graph without executing a production workflow.
17. Older workflows with no imports compile and execute unchanged.
18. Local code modules and WOML components pass clean-package deployment and
    recovery journeys.

## 6. Scope

### Included

- Local JavaScript and TypeScript code modules.
- Named export discovery and one `services.<alias>` namespace per import.
- Static relative dependency resolution.
- Deterministic TypeScript/JavaScript compilation and source maps.
- A content-addressed immutable module artifact store.
- Module graph validation, cycle detection, collision detection, hashing, and
  lock data.
- Package dependencies that can be deterministically bundled without native
  addons or install scripts.
- Runtime bundle caching with per-invocation state isolation.
- Top-level effect prevention.
- Secret/capability permission and supply-chain rules for installed packages.
- Type declarations and editor autocomplete for imported services.
- Triggerless WOML components with explicit inputs and one output.
- Compile-time component instantiation into the caller's DAG.
- Nested component composition and mixed code/WOML packages.
- CLI inspection, validation, testing, packaging, documentation, and migration
  guidance.

### Not included

- A public hosted WOML marketplace in the first module release.
- Automatically trusting arbitrary npm lifecycle scripts.
- Node native addons, arbitrary executable binaries, child processes, or FFI.
- Remote `http://` or `https://` source imports.
- Runtime dependency installation or registry access while a workflow runs.
- Unpinned semver ranges in an activated production definition.
- Shared mutable JavaScript singletons across runs.
- Arbitrary raw filesystem, environment, or socket access.
- A user-facing `kind="pure"` or `kind="capability"` declaration.
- Allowing imported code to overwrite WOML built-ins.
- Implicit access from a component to caller context.
- A component that declares its own production trigger.
- A separate child workflow run, parent/child cancellation tree, or distributed
  subworkflow orchestration unless separately reviewed after component
  embedding works.
- Hot reloading a running definition. A future `--watch` still creates a new
  immutable definition while existing runs retain their old artifacts.
- General production multi-tenant OS isolation. This milestone creates the
  package permission contract; Production Runtime owns tenant sandboxing.

## 7. Provisional Authoring Directions

This section frames the choices MS0 must review. It does not amend the current
WOML grammar. Today, one `.woml` document still has exactly one `<workflow>`
root containing its existing ordered children.

### 7.1 Where imports live

MS0 must choose one canonical document shape from directions such as:

**Workflow-owned imports:**

```xml
<workflow ...>
  <imports>...</imports>
  <triggers>...</triggers>
  <steps>...</steps>
</workflow>
```

**Package document:**

```xml
<woml ...>
  <imports>...</imports>
  <workflow ...>...</workflow>
</woml>
```

**External project manifest:** imports stay outside the workflow source.

The choice affects one-workflow-per-file behavior, shared imports, component
exports, editor tooling, and future packages. No implementation phase may
quietly choose a shape because it is easiest to parse.

### 7.2 Import element naming

Possible directions include:

```xml
<import name="spreadsheet" from="./spreadsheet.ts" />
```

or:

```xml
<module name="spreadsheet" from="./spreadsheet.ts" />
```

`<require>` is not assumed: it carries CommonJS meaning while WOML modules use
modern ESM/TypeScript semantics. MS0 freezes the one canonical spelling and
rejects aliases.

### 7.3 Calling a code module

The agreed public direction is:

```js
const rows = await services.spreadsheet.read(file);
```

Direct globals such as `spreadsheet.read()` are not the default because they
collide easily and make runtime bindings harder to discover. Named function
exports are the likely v1 contract. MS0 decides whether JSON constants, nested
namespaces, default exports, classes, generators, and symbol exports are
supported or rejected.

### 7.4 Module implementation

An ordinary local module should look like ordinary JavaScript:

```js
export function removeEmptyRows(rows) {
  return rows.filter(row => row.some(value => value !== null && value !== ""));
}

export async function loadExchangeRates(base) {
  const response = await fetch(
    `https://rates.example.com/latest?base=${encodeURIComponent(base)}`
  );

  if (!response.ok) {
    throw new Error(`Rates API returned ${response.status}`);
  }

  return response.json();
}
```

No WOML npm package is required merely to define functions. The runtime installs
native Fetch instrumentation and the built-in `services` binding before an
exported function is invoked.

For reuse and security, module functions should normally receive workflow data
and credential values as arguments instead of silently reading `context`:

```js
export async function loadCustomer(customerId, token) {
  return fetch(`https://crm.example.com/customers/${customerId}`, {
    headers: { authorization: `Bearer ${token}` }
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

MS0 must decide whether local modules may use the `secrets` global directly,
whether installed packages must declare secret inputs, and how editor types
represent those choices. A package must never enumerate the whole project
secret store.

### 7.5 Instantiating a WOML component

Possible invocation directions include a React-like custom element:

```xml
<customer.load
  id="customer"
  customer-id="{{context.trigger.customerId}}" />
```

or an explicit neutral element:

```xml
<use
  id="customer"
  module="customer"
  component="load">
  <input name="customerId" value="{{context.trigger.customerId}}" />
</use>
```

The custom-element form is concise and visually component-like. The explicit
form is easier for XML tooling and makes dynamic names impossible. MS0 reviews
both against parser behavior, errors, schemas, autocomplete, and AI-generated
WOML before selecting one.

## 8. Code Module Runtime Contract

### 8.1 Export surface

The first profile should favor named function exports:

```js
export function parse(...) { ... }
export async function write(...) { ... }
```

The build stage validates the public names. At runtime, WOML constructs one
deeply read-only namespace:

```js
services.spreadsheet.parse
services.spreadsheet.write
```

The namespace cannot be mutated, extended by user code, or reassigned.

### 8.2 Initialization and invocation

The Bun host may cache a compiled bundle by content hash. Each isolated Worker:

1. installs the restricted initialization environment;
2. instantiates the bundle with effects disabled;
3. validates the expected named exports;
4. attaches callable exports under the imported alias;
5. enables invocation-scoped Fetch/capability calls;
6. executes the authored script; and
7. destroys all module state with the Worker.

An effect attempted during steps 1-4 fails before the workflow operation is
sent. Pure top-level initialization must be bounded by the same worker startup
and memory limits.

### 8.3 Local values versus protocol values

A pure module function may return an in-memory object, class instance, typed
array, or library value for further local processing. It does not cross Rust
merely because it was returned by a module function.

Only these boundaries require the frozen wire types:

- arguments/results of a managed `services.*` capability call;
- operation metadata;
- the final `<script>` result; and
- stored component inputs/outputs.

The final step result remains JSON-only unless Services and Capabilities
provides an explicit portable storage reference.

### 8.4 Errors and source maps

An exception must preserve:

- original module path or package identity;
- original TypeScript/JavaScript line and column;
- exported function name;
- calling WOML step ID and source location; and
- a secret-safe stack/cause chain.

The CLI should show the shortest useful error first and offer verbose dependency
and stack information without printing generated bundle internals by default.

## 9. Resolution, Hashing, and Immutable Artifacts

### 9.1 Static graph

The initial resolver accepts static imports only. It rejects:

- missing files;
- paths outside reviewed project/package boundaries;
- ambiguous case/symlink identities;
- import cycles in the first profile;
- dynamic `import()`;
- CommonJS runtime `require()` unless explicitly added later;
- remote URL imports;
- native addons and executable binaries; and
- packages that require install/postinstall scripts.

Relative paths resolve from the importing source file, not the terminal's
current working directory.

### 9.2 Definition package

An activated definition package contains or addresses immutable copies of:

- compiled workflow Model;
- compiled JavaScript bundles;
- source maps;
- original source identity and safe display paths;
- transitive dependency manifest;
- package versions and integrity digests;
- compiler/Bun compatibility version;
- imported WOML component source/model artifacts; and
- a root definition hash over the canonical package manifest.

Filesystem timestamps, absolute machine-specific paths, and dependency cache
locations do not affect the canonical hash.

### 9.3 Runtime cache

The long-lived Bun host may retain compiled artifacts keyed by digest. Rust
registers/validates the artifact identity and can resend it after a host crash.
Cache eviction affects performance only; the durable definition package remains
the recovery authority.

### 9.4 Locking and package dependencies

MS0/MS5 must decide whether WOML consumes an existing `bun.lock`, emits a
WOML-specific lock artifact, or records both. Regardless of filename:

- activated versions are exact;
- integrity hashes are mandatory;
- resolution never changes during a run;
- production execution does not access a package registry; and
- bundle output plus toolchain compatibility is part of the immutable
  definition.

## 10. WOML Component Contract

The exact markup remains MS7 work, but these semantics are required.

### 10.1 Triggerless reusable graph

A component may contain supported executable structures such as steps,
branches, parallels, approvals, and nested components. It does not own manual,
webhook, Slack, schedule, interval, event, or queue triggers.

### 10.2 Declared inputs

Inputs have stable names and may eventually declare:

- required/optional status;
- JSON Schema or primitive type;
- default JSON value;
- ordinary data versus secret/capability input; and
- source documentation.

The first profile must choose a clear JavaScript binding such as `inputs` or an
explicit component-local context field. It must not smuggle inputs into
`context.trigger`, silently expose caller context, or resolve an undefined
`context.run` contract.

References at the call site may depend only on values legally available before
the component instance in the caller DAG.

### 10.3 One public output

A component declares one output expression/schema. After successful execution,
the caller sees that output at:

```js
context.steps.<instanceId>
```

Internal outputs remain component-private unless explicitly included in the
declared output. A component with no meaningful data may return `null`, but the
behavior must be explicit.

### 10.4 ID namespaces

The frontend creates deterministic compiled IDs from the import identity,
component identity, instance ID, nesting path, and internal node ID. Two
instances of the same component never collide.

Author-facing diagnostics use readable component paths. Rust validates the
flattened DAG IDs but does not need to understand XML alias syntax.

### 10.5 Compile-time instantiation

Component embedding is compile-time composition in v1:

```text
component source
  -> validate its local DAG and interface
  -> bind caller inputs
  -> instantiate/prefix internal nodes
  -> connect caller predecessor/component entry edges
  -> connect component exit/output/caller successor edges
  -> validate the complete DAG
```

The result is one durable run and one event log. A component is not a hidden
second executor or mutable context object.

### 10.6 Observability

Compiled nodes retain safe source/module/instance metadata so the CLI and a
future UI can group them by component. The first implementation should avoid a
new durable module-event vocabulary unless execution semantics genuinely need
it; visual grouping can usually be derived from the immutable Model.

## 11. Permissions, Secrets, and Supply Chain

The module system does not ask the user to label code as a capability. That does
not remove the need for permissions when installing third-party code.

The product distinction is:

- **module kind** is an internal implementation concern and is not authored;
- **requested permission** is a security decision and must be visible before
  third-party code receives access.

MS0/MS6 must freeze:

- whether local project modules are treated as trusted authored code;
- how installed packages request native Fetch origins and built-in services;
- how secret values are passed or explicitly granted;
- whether package permissions are recorded in the lockfile/definition package;
- what happens when a dependency update requests new permissions;
- how filesystem/storage scopes are expressed;
- how transitive dependency permissions are summarized; and
- how CI/non-interactive builds approve an exact reviewed permission set.

Guardrails:

1. A package cannot enumerate the secret store.
2. A package update cannot silently gain a new permission.
3. Only the exact locked artifact executes.
4. Runtime registry access and install scripts are forbidden.
5. Raw environment, child process, FFI, and native addon access remain denied.
6. Known secrets are rejected from final results, logs, events, source maps, and
   bundled artifacts.
7. A package's generated code and source maps are scanned for embedded
   credentials before activation.
8. Hosted multi-tenant enforcement requires the later Production Runtime
   sandbox; local permissions are still useful product policy, not a false OS
   boundary.

## 12. CLI and Developer Experience

The exact command names are an MS0 decision, but the product must support these
journeys:

- validate a workflow and its full module graph without executing it;
- show resolved aliases, paths/packages, versions, hashes, exports, component
  interfaces, and permissions;
- explain why a module or export cannot be resolved;
- build an immutable deployable definition package;
- reproduce a build from its lock data;
- test a code module export with mocked built-in services;
- test a WOML component with supplied inputs and inspect its single output;
- identify unused imports and dependency cycles;
- show which transitive file caused a definition hash to change; and
- print source-mapped errors from installed packages safely.

Potential commands such as `woml check`, `woml inspect`, `woml bundle`, or
`woml modules` remain provisional. The module system should reuse existing
`woml run` and `woml test` rather than invent a second runtime.

Editor support should generate or expose TypeScript declarations so this works
with autocomplete:

```ts
services.spreadsheet.read
services.crm.loadCustomer
```

WOML component schemas should provide attribute/input autocomplete and
source-located missing/unknown input diagnostics.

## 13. Versioned Artifacts to Review in MS0

MS0 determines the exact version numbers after auditing SC14. At minimum the
Module System needs reviewed artifacts for:

1. module import/document syntax;
2. code module manifest and named-export contract;
3. module resolution and canonical identity;
4. immutable definition-package manifest;
5. dependency lock and integrity data;
6. Bun bundle/runtime compatibility;
7. Script Host module registration/cache/invocation messages, using a new host
   protocol version if SC14's frozen version cannot express them;
8. package permission and secret-grant contract;
9. WOML component definition, input, output, and invocation syntax;
10. compiled Model module/component metadata and flattened-node identity;
11. module/component diagnostics and source-location schema;
12. representative source, bundle, source-map, model, package, nested graph,
    recovery, and invalid-cycle fixtures.

Run Event changes are not assumed. If the compiled DAG and existing generic
operation events are sufficient, module provenance remains immutable Model
metadata. Any new durable event requires its own explicit review and version.

## 14. Implementation Phases

### MS0 — Freeze the Module System contracts after SC14

Changes:

- Audit the completed capability registry, Script Host, secrets, Fetch, and
  artifact persistence boundaries.
- Select the WOML document/import shape and canonical import element syntax.
- Freeze alias/export rules, path/package resolution, graph/cycle rules,
  definition-package hashing, lock data, runtime artifact registration, and
  compatibility versions.
- Freeze the local-versus-installed permission and secret contract.
- Choose the component invocation syntax, input binding, output contract, and
  flattened ID algorithm.
- Add reviewed code-module and WOML-component fixtures before runtime code.
- Keep child runs, public registry UX, hot reload, and multi-node ownership
  explicitly deferred.

Result:

Every layer agrees on what a module is and how it remains reproducible, but no
module is executable yet.

Gate:

Schemas and fixtures resolve every syntax/protocol/hash/permission shape listed
in Sections 7-13, reject ambiguous alternatives, and do not modify any frozen
SC artifact in place.

### MS1 — Build the static resolver and immutable definition package

Changes:

- Resolve local files from the importing source with canonical safe identities.
- Build a deterministic acyclic dependency graph.
- Validate aliases, extensions, paths, missing files, collisions, and exports.
- Canonicalize and hash sources, manifests, compiler version, and graph edges.
- Create/store a definition package containing module artifacts and source
  maps rather than runtime filesystem paths.
- Add cache reuse by content digest.

Result:

WOML can explain and package a module graph without executing its code.

Gate:

Same content in different safe project locations produces the reviewed
portable identities; content changes alter the root hash; timestamps, graph
discovery order, and cache paths do not.

### MS2 — Compile local JavaScript and TypeScript imports

Changes:

- Activate the frozen import syntax for local `.js` and `.ts` files.
- Parse ESM imports/exports and accept the reviewed named-function surface.
- Transpile/bundle TypeScript deterministically and retain source maps.
- Lower aliases and artifact digests into the new compiled Model profile.
- Generate TypeScript declarations for `services.<alias>`.
- Keep frontend errors attached to exact WOML/import/module locations.

Result:

A WOML workflow with a local code module compiles into a complete immutable
definition package, but Rust may still reject module execution.

Gate:

Reviewed source deep-equals the Model/package fixtures; syntax, export,
collision, dynamic import, cycle, and TypeScript error cases are source-mapped.

### MS3 — Execute local modules under `services.*`

Changes:

- Register immutable bundles with the long-lived Bun host by digest.
- Instantiate a fresh module environment in each isolated Worker.
- Attach named exports to a deeply read-only `services.<alias>` namespace.
- Disable Fetch/capability calls during module initialization and enable them
  during exported-function invocation.
- Let module-internal Fetch and built-in service calls reuse SC contracts.
- Preserve local in-memory values until the final script JSON boundary.

Result:

One local TypeScript file is declared once and used from two sequential WOML
steps through `services.<alias>`.

Gate:

The vertical slice proves pure sync/async functions, Fetch, managed services,
multiple exports, multiple steps, no shared mutable state, top-level effect
rejection, Worker timeout, and exact final output.

### MS4 — Make local module execution recoverable and observable

Changes:

- Recover using stored definition artifacts after original files are changed or
  removed.
- Re-register bundles after Bun host restart and verify every digest.
- Add source-mapped module stacks and safe CLI progress.
- Enforce artifact/frame/memory/startup limits and cache eviction.
- Test branch, parallel, approval, retry, all triggers, and every built-in
  service from imported functions.
- Prevent module artifacts, maps, errors, and logs from containing secrets.

Result:

Local code modules have the same restart, isolation, composition, and diagnostic
quality as inline WOML scripts.

Gate:

Crash tests cover before/after artifact registration, during initialization,
during pure code, during Fetch/service calls, and after script success; recovery
never reads current project files.

### MS5 — Add locked package dependencies and deterministic bundles

Changes:

- Freeze and implement the reviewed package/lockfile integration.
- Resolve exact versions and integrity digests before activation.
- Bundle dependencies into immutable artifacts for offline production runtime.
- Reject install scripts, native addons, executable binaries, dynamic/remote
  imports, and unsupported CommonJS edges.
- Add dependency license/provenance metadata without making it workflow context.
- Build the spreadsheet acceptance fixture using a real locked library.

Result:

A user can write a small local spreadsheet module backed by a package dependency
and deploy it without `npm install` on the production runtime.

Gate:

Offline clean-package execution succeeds; tampered integrity, missing lock,
range drift, lifecycle scripts, native addons, registry outage, and transitive
cycle cases fail before activation.

### MS6 — Enforce module permissions and complete code-module DX

Changes:

- Implement the reviewed permission/grant contract for installed packages.
- Prevent secret enumeration and require reviewed secret flow.
- Enforce capability/network/storage policy at controlled runtime doorways.
- Detect permission expansion after a dependency update.
- Add module graph inspection, build/check output, unit testing with mocked
  built-ins, autocomplete, unused-import diagnostics, and documentation.
- Add local and package code-module examples to clean-package smoke tests.

Result:

JavaScript/TypeScript code modules become independently publishable and pleasant
to author, inspect, test, and deploy.

Gate:

Permission denial/grant/update, secret leakage, malicious dependency, type
generation, mocked service, packaging, offline, and compatibility tests pass.

### MS7 — Freeze the WOML component language and fixtures

Changes:

- Freeze the component document/export and invocation syntax selected in MS0.
- Freeze input names, required/default/schema behavior, the script binding for
  inputs, and secret input behavior.
- Freeze the one-output contract and caller `context.steps.<instanceId>` shape.
- Freeze local/private IDs, instance identity, nested identity, and source maps.
- Define allowed component structures and explicitly forbid triggers/caller
  context capture.
- Add simple, branch, parallel, approval, nested, and invalid-cycle fixtures.

Result:

A reusable WOML graph has one precise public interface before it is expanded
into a caller DAG.

Gate:

Every fixture has an exact interface and expanded-ID expectation; missing,
unknown, forward, secret, collision, empty-output, and recursive cases produce
source-located errors with no unresolved defaults.

### MS8 — Compile component instances into the caller DAG

Changes:

- Resolve imported WOML component definitions through the MS1 graph.
- Validate component-local DAGs independently.
- Bind caller values to declared inputs and insert reviewed boundary nodes or
  metadata.
- Prefix/instantiate internal nodes deterministically.
- Connect entry, exit, output, and downstream edges.
- Revalidate the complete expanded DAG and reference legality.
- Store component/interface/source provenance in the immutable Model/package.

Result:

A caller using one component compiles into the exact reviewed flat DAG while
retaining enough metadata for component-level diagnostics and visualization.

Gate:

Expanded models deep-equal fixtures for one/two/nested instances, branches,
parallels, approvals, repeated imports, and mixed code-module dependencies;
cycles, ID collisions, and reference leaks are rejected.

### MS9 — Execute and recover WOML components end to end

Changes:

- Execute expanded components through the existing Rust DAG without a second
  executor or persistence authority.
- Publish exactly one instance result at the caller-visible step ID.
- Keep internal outputs private outside component diagnostics.
- Compose retries, branch selection, parallel cancellation, approval waiting,
  service calls, events, queues, storage references, and process recovery.
- Group CLI progress by component instance using immutable Model metadata.

Result:

A reusable `.woml` component works like a native workflow building block and
survives every existing durable pause/restart boundary.

Gate:

End-to-end and crash-boundary tests prove exact output, private internals,
single event authority, approval resume, retry safety, and compatibility with
non-module workflows.

### MS10 — Compose nested and mixed local module packages

Changes:

- Support packages containing code modules, WOML components, or both.
- Resolve nested components and code dependencies through one acyclic graph.
- Deduplicate identical artifacts by digest while preserving aliases/interfaces.
- Enforce package export maps so private files/components cannot be imported.
- Add package-level documentation, types, component schemas, and provenance.

Result:

A team can maintain a reusable local WOML package containing a component and
the TypeScript services that implement its domain logic.

Gate:

Mixed/nested/re-export/private-export/duplicate-version tests prove deterministic
resolution, isolation, hashing, permission aggregation, and source diagnostics.

### MS11 — Complete packaging, inspection, and distribution boundaries

Changes:

- Finalize CLI check/inspect/build/test journeys and machine-readable output.
- Produce portable definition/module bundles for deployment and CI artifacts.
- Support reviewed local path and packaged archive dependencies.
- Define, but do not require, the future registry identity/signature boundary.
- Document versioning, upgrades, lock updates, permission changes, cache cleanup,
  and incident response for a compromised dependency.
- Add migration guidance from copied scripts and the legacy SDK service pattern.

Result:

Modules can move from a developer project to CI and production reproducibly,
without a public marketplace.

Gate:

Builds reproduce offline across clean directories; inspection explains every
artifact/version/permission; corrupted/tampered archives fail before execution.

### MS12 — Harden and publish the WOML Module System

Changes:

- Run adversarial graph, bundle, source-map, permission, secret, resource,
  cache, crash, and recovery suites.
- Benchmark bundle build, cold/warm Worker startup, cache reuse, pure function
  calls, nested components, and large graphs.
- Update the WOML language, architecture, CLI, security, deployment, recovery,
  migration, and AI-authoring documentation.
- Add local code, package-backed spreadsheet, simple component, nested
  component, and mixed package examples to the release suite.
- Run the entire pre-module WOML compatibility suite unchanged.

Result:

Local/package JavaScript and TypeScript modules plus reusable WOML components
are supported, documented, reproducible, and publishable.

Gate:

Frontend, Rust, Bun host, schemas/protocols, typecheck, Clippy, package,
permission, offline, source-map, compatibility, performance-regression, and
secret/artifact scans pass from a clean installation.

## 15. Expected File Areas

| Area | Expected locations |
|---|---|
| WOML document/import grammar | `woml/src/parser.ts`, `compiler.ts`, new module syntax helpers |
| Module graph/resolution | new `woml/src/modules/*` resolver, identity, graph, lock, diagnostics |
| Model/package artifacts | `woml/src/model.ts`, new schemas/manifests and artifact builders |
| JS/TS analysis/bundling | new frontend/CLI bundler integration, source maps, type generation |
| Bun module runtime | `woml-cli/src/script-host/*`, new module loader/cache/runtime helpers |
| Rust artifact authority | `core/woml-engine` definition/artifact storage and host registration |
| Permissions/secrets | frontend dependency analysis, CLI grants, runtime capability policy |
| WOML components | new component interface/lowering/instantiation modules and fixtures |
| CLI DX | `woml-cli/src/cli.ts`, module check/inspect/build/test commands |
| Versioned contracts | `docs/schemas/*`, `docs/protocols/*`, model/package/module fixtures |
| Examples | local TS module, spreadsheet package, simple/nested WOML components |

Exact files depend on the SC14 implementation. The ownership boundary remains:
the TypeScript frontend understands module syntax and builds immutable artifacts,
Rust validates/stores definitions and executes the DAG, and Bun evaluates
JavaScript/TypeScript bundles in isolated invocation state.

## 16. Verification Matrix

| Area | Required proof |
|---|---|
| Syntax | One canonical import/component grammar; alternatives and misplaced declarations fail clearly. |
| Resolution | Relative/package identities are deterministic, bounded, acyclic, and source-located. |
| Hashing | Every transitive content change changes the definition; timestamps/absolute paths do not. |
| Recovery | Stored artifacts execute after source deletion/change; current filesystem code is never substituted. |
| Isolation | No mutable module state crosses invocation/run boundaries. |
| Initialization | Fetch and managed effects are impossible before exported-function invocation. |
| Namespace | Built-ins, aliases, exports, instances, and internal IDs cannot collide. |
| Operations | Module Fetch/services calls use the SC operation protocol; pure calls avoid RPC. |
| Secrets | Packages cannot enumerate secrets; grants and values never leak into artifacts/history. |
| Permissions | Dependency updates cannot silently gain network/service/storage access. |
| Packages | Exact offline bundles reproduce; scripts/native addons/remote imports are rejected. |
| Errors | Stacks map to original module/component line and call-site step. |
| Components | Inputs are explicit, internals private, output singular, expanded DAG acyclic. |
| Composition | Branch, parallel, approval, retry, triggers, services, events, and queue work inside instances. |
| Compatibility | Workflows without imports compile and execute unchanged. |
| Packaging | Every reviewed example works from a clean offline artifact. |

## 17. Risks and Guardrails

### “React-like” must not mean hidden state

The desired similarity is readable composition, explicit props/inputs, stable
components, and good tooling. WOML components must not inherit React's mutable
client-state assumptions or introduce a virtual DOM-style runtime.

### One `services` namespace contains different execution costs

A pure `services.spreadsheet.parse()` may be a local function while
`services.db.query()` crosses Rust. Editor documentation and inspection should
show effect/cost information where known without requiring author-written
module kinds.

### Arbitrary npm compatibility would destroy reliability

Many packages assume unrestricted filesystem, environment, sockets, native
addons, or install scripts. WOML v1 intentionally supports a reproducible safe
subset and produces helpful incompatibility errors instead of claiming every
npm package works.

### Bundles are part of durable workflow history

Storing only a path or package version is insufficient. Artifact retention,
size, cleanup, and integrity must be designed with run retention so old active
runs do not lose executable code.

### Component inlining can create enormous DAGs

The frontend needs maximum graph depth/node/artifact limits, source-preserving
diagnostics, and expansion summaries. Nested components cannot expand
recursively without bound.

### Secrets passed as strings can be exfiltrated

Native Fetch requires real values in Bun. Permissions and explicit argument
flow reduce accidental exposure but cannot stop deliberately authored trusted
code. Hosted untrusted-code guarantees wait for OS isolation.

### Package permissions must not become `kind="capability"` in disguise

The user should approve concrete access such as a network origin or named
secret, not explain the engine's execution category. Local authored modules and
third-party packages may receive different trust defaults, but the behavior
must be transparent.

### Component output is an interface, not an internal step shortcut

Downstream callers depend on the declared component result only. Allowing
references into internal node IDs would permanently prevent safe refactoring.

## 18. Global Roadmap After the Module System

1. **Retries and idempotency** — completed in RI7.
2. **Production triggers** — completed in T13.
3. **Services and capabilities** — SC0-SC14, built before this milestone.
4. **WOML Module System** — this MS0-MS12 milestone: local/package JS/TS
   modules, reusable WOML components, immutable bundles, permissions, and
   deployment packaging.
5. **Lifecycle and engine controls** — workflow cancellation, lifecycle hooks,
   workflow-level concurrency/rate limits/timeouts, durable user state,
   advanced queue controls, and any separately reviewed child-workflow runs.
6. **Production runtime and operations** — hosting, deployment, multi-node
   ownership, OS-level isolation, observability, retention, administration,
   scaling, and artifact lifecycle management.
7. **WOML package registry/community ecosystem** — signed publication,
   discovery, trust/provenance, moderation, compatibility, and deprecation once
   local/package artifacts are proven.
8. **Additional infrastructure and communication providers** — external
   database/storage/cache/broker adapters plus Discord, WhatsApp, and Telegram
   according to demand.
9. **Retire the JavaScript chaining SDK** — only after WOML reaches sufficient
   parity and users have a supported migration path.

After MS12, the next architecture milestone is Lifecycle and Engine Controls.

## 19. MS0 Review Gate

Implementation must not begin until MS0 explicitly resolves:

- workflow-owned imports versus a `<woml>` package root versus an external
  manifest;
- canonical `<import>`/`<module>` spelling and ordering;
- alias and named-export grammar;
- JavaScript/TypeScript ESM subset and CommonJS decision;
- local path, symlink, case, project-root, and cycle rules;
- canonical module graph, definition package, hash, lock, and artifact retention;
- Bun compiler/runtime compatibility and source-map format;
- host artifact registration/cache protocol version;
- top-level initialization restrictions and timeout/memory limits;
- local authored-code trust versus installed-package permissions;
- secret access/passing/grant behavior;
- npm/package manager and lockfile integration;
- component definition and invocation syntax;
- component input binding without abusing `context.trigger` or defining
  `context.run`;
- component output, private IDs, nested instances, and flattened DAG identity;
- whether compile-time components are sufficient for v1 or child runs require
  a later separate contract; and
- exact errors, CLI journeys, limits, and clean-package fixtures.

The following remain explicitly deferred unless the user later expands scope:
a public hosted registry, marketplace economics, remote source imports,
arbitrary npm compatibility, hot reload semantics, distributed artifact
ownership, OS-level tenant isolation, and JavaScript SDK retirement.
