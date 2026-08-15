# Cronflow JavaScript SDK Retirement Contract

Status: **deprecated, maintenance only**  
Contract: `cronflow.sdk-retirement/v1`  
Published: **2026-08-15**  
Support ends: **2027-02-15**

This is the user-facing product contract for retiring the JavaScript-chaining
Cronflow SDK in favor of WOML. It applies to both public entry points of the
`cronflow` npm package: `cronflow` and `cronflow/sdk`.

The machine-readable contract is
[`contracts/cronflow-sdk-retirement.v1.json`](contracts/cronflow-sdk-retirement.v1.json).

## The promise in one minute

- `cronflow@0.11.6` is the final release that may add features to the chained
  JavaScript SDK.
- The `0.11.x` line receives only critical security, data-loss/corruption,
  critical-regression, and migration-documentation fixes through
  **2027-02-15**.
- New workflow features are implemented only in WOML.
- Removing the chained SDK is a breaking change. It cannot happen in a
  `0.11.x` patch and cannot happen before **2027-02-16**. The earliest
  `cronflow` version allowed to remove it is `1.0.0`.
- No release automatically rewrites JavaScript workflows, converts in-flight
  runs, deletes `.cronflow` data, or points WOML at a legacy database.
- Internal crate, native-binary, and platform-package version numbers are
  implementation identities. The supported product identity in this contract
  is the public `cronflow` npm package.

If a maintenance release is required during the support window, it remains in
`0.11.x`. The newest published `0.11.x` at the end of the window becomes the
final supported build; `0.11.6` remains the frozen feature baseline.

## Why the SDK is retiring

The chained SDK and WOML are two complete authoring and execution paths. Keeping
both indefinitely would duplicate triggers, scheduling, retries, state,
execution, tests, and documentation. WOML is now the product's authoring
surface: workflow structure is readable markup, JavaScript remains available
inside `<script>`, and the Rust engine owns durable execution.

Retirement does not mean that JavaScript disappears. It means JavaScript no
longer constructs the workflow graph through `cronflow.define()` and chaining.

## Support policy

During the support window, the SDK may receive:

- fixes for critical security vulnerabilities;
- fixes that prevent data loss or corruption;
- fixes for critical regressions in behavior that already worked; and
- documentation corrections needed to migrate safely.

It will not receive new triggers, services, control-flow operators, providers,
performance features, or parity work. Feature requests target WOML.

Existing SDK applications may continue running after the support end date, but
they do so without a maintenance promise. End of support does not uninstall the
package or remove local files.

## Breaking-release boundary

The following actions require a reviewed breaking release no earlier than
2027-02-16:

- removing `sdk/` or the root `cronflow` compatibility exports;
- removing generated chaining types or SDK-specific dependencies;
- removing the legacy Rust graph needed only by the SDK; and
- changing the `cronflow` package from chaining semantics to a different API.

WOML is not a silent replacement behind `import { cronflow } from "cronflow"`.
Users migrate source deliberately and activate it with `woml run`.

## Feature-equivalence table

The status terms are intentional:

- **Direct replacement**: WOML has a supported product equivalent.
- **Migration recipe**: the goal is supported, but the shape or semantics must
  be redesigned.
- **Retired legacy API**: the SDK surface is a helper, simulation, or incomplete
  API that WOML does not reproduce as an author-facing API.
- **No equivalent yet**: do not cut over a workflow that depends on the exact
  behavior without redesigning it first.

| Cronflow SDK capability | WOML path | Status |
| --- | --- | --- |
| `cronflow.define()` and chained steps | `<workflow>`, `<steps>`, `<step>`, `<script>` | **Direct replacement** |
| `ctx.payload`, previous results | `context.payload`, `context.steps.<id>` | **Direct replacement** |
| Manual, webhook, schedule, interval, and event triggers | Tags inside `<triggers>` | **Direct replacement** |
| Slack trigger and approval notification | `<slack>` trigger/provider | **Direct replacement** |
| If/else, exact-value decisions | `<choose>` and `<switch>` | **Direct replacement** |
| Parallel work and independent routes | `<parallel>` and `<fork><branch>` | **Direct replacement** |
| Retry, timeout, human approval | Step retry attributes, runtime timeout, `<approval>` | **Direct replacement** |
| HTTP, database, storage, cache, event, and durable state helpers | `services.http`, `db`, `storage`, `cache`, `events`, `state` | **Direct replacement** |
| Reusable JS/TS helpers and SDK abstractions | Imported modules, reusable steps, custom providers | **Direct replacement** |
| Await or asynchronously start another workflow | `services.workflows.call()` / `.start()` | **Direct replacement** |
| Workflow hooks and run cancellation | `<lifecycle>`, `woml cancel` | **Direct replacement** |
| Concurrency, rate limit, timeout, queue admission | Workflow `<config>` | **Direct replacement** |
| Background hosting, inspection, logs, backup, restore, retention | WOML production runtime and CLI operations | **Direct replacement** for the single-machine profile |
| `.action()` fire-and-continue work | `services.workflows.start()` or `<fork join="none">` | **Migration recipe**; choose ownership and failure semantics explicitly |
| Step `.cache()` | `services.cache` | **Migration recipe**; TTL and non-authoritative semantics are explicit |
| Step `.delay()` | Script delay or a scheduled workflow boundary | **Migration recipe**; there is no durable mid-step sleep contract |
| `.forEach()`, `.while()`, `.batch()`, `.race()` | Script logic, explicit steps, or child workflow calls | **Migration recipe**; script loops do not create durable per-item steps |
| Step-local `.onError()` and `.log()` | Workflow/step lifecycle hooks and script logging | **Migration recipe** |
| Framework-embedded webhook adapters and middleware | WOML-owned trigger host behind a reverse proxy | **Migration recipe**; WOML does not mount into an Express/Fastify route tree |
| SDK global/process-local state | `services.state`, database, storage, or cache according to ownership | **Migration recipe** |
| SDK test harnesses, benchmark helpers, and performance classes | `woml check`, normal integration tests, CLI inspection | **Retired legacy API**; no matching runtime library API is promised |
| Standalone circuit-breaker helper | Application/module policy around supervised HTTP | **No equivalent yet** |
| SDK `subflow()` and `waitForEvent()` simulations | Durable workflow call/start and event trigger/service | **Retired legacy API**; migrate to the real WOML primitives |
| SDK `cancelRun()` placeholder | `woml cancel` | **Direct replacement**; the incomplete legacy method is not a parity blocker |
| External durable queue service | Not currently published | **No equivalent yet** |
| Additional communication providers | Custom notification provider or future built-ins | **Migration recipe** for notifications; built-in parity is not promised |
| Cross-machine workflow calls and multi-node hosting | Current runtime is single-machine | **No equivalent yet** |
| Remote public run-control API | Current admin surface is local and authenticated | **No equivalent yet** |

The complete source-rewrite checklist is in
[`woml-sdk-migration.md`](woml-sdk-migration.md).

## Safe migration and cutover

1. Inventory every SDK workflow, trigger endpoint, provider, state path,
   environment variable, secret name, and deployment owner.
2. Use the table above to classify dependencies. Do not schedule cutover while
   a required capability is marked **No equivalent yet** without an approved
   redesign.
3. Rewrite and validate `.woml` definitions. Preserve public webhook paths and
   externally visible idempotency keys where compatibility requires them.
4. Test WOML with a separate `.woml/state.sqlite`; never point it at a legacy
   `.cronflow` database.
5. Stop legacy trigger admission, then allow active legacy runs to settle.
   The SDK's cancellation path is not a reliable migration primitive; record
   any unresolved runs rather than pretending they were converted.
6. Create and verify a legacy archive using
   [`cronflow-sdk-data-archive.md`](cronflow-sdk-data-archive.md).
7. Activate WOML as the sole owner of the migrated ingress. Do not let the SDK
   and WOML consume the same webhook, schedule, or provider event concurrently.
8. Observe the acceptance window. Keep the old binary, lockfile, workflow
   source, and read-only archive until rollback is no longer required.

## What happens to historical data

Legacy Cronflow stores mutable workflow/run/step rows. WOML stores versioned
definitions and an immutable event history. There is no automatic or truthful
conversion of an arbitrary in-flight legacy run into WOML events.

Historical Cronflow data remains an archive. New WOML activations create new
WOML runs. The retirement work will never delete `.cronflow`, `cronflow.db`, a
custom legacy database path, workflow source, or secrets on a user's behalf.

## Review and changes

Changing a support date, allowed maintenance category, removal boundary, or
data-safety promise requires a new versioned retirement contract and explicit
release notes. Editing prose cannot silently weaken
`cronflow.sdk-retirement/v1`.
