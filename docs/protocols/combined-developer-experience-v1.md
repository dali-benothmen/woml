# Combined Developer Experience v1

Status: frozen foundation implemented through CDX7.

## Purpose

Combined Developer Experience v1 assembles WOML's existing infrastructure-agent capabilities into one project without creating a new execution authority. The project layer resolves files, validates references, computes public identity, and prepares existing configuration loaders. The Rust runtime remains authoritative for admission, effects, durability, recovery, and settlement.

## Version Decision

This foundation adds two non-durable contracts:

- WOML Project v1;
- Project Check v1.

It does not change:

- Compiled Model v21;
- Definition Package v16;
- Run Event v20;
- Durable Store v20;
- Operational Output Store v1;
- Target Registry v1;
- Worker Configuration, Protocol, Dispatch Package, or Journal v1.

Any later need to change one of those contracts requires a separate review and version decision. It must not be introduced as an incidental developer-experience change.

## WOML Project v1

The canonical filename is `woml.project.json`. Its profile is `woml.project/v1` and its version is `1`.

The schema is `docs/schemas/woml-project.v1.schema.json`.

Project v1 is reference-only. It may point to operator-owned configuration, but it cannot contain:

- secret values;
- environment values;
- private keys or certificates;
- provider credentials;
- inline policies;
- worker-local configuration;
- executable scripts.

### Field ownership

| Field | Owner | Loaded by | Effect authority |
| --- | --- | --- | --- |
| `workflows` | Author | WOML compiler | None during resolution |
| `runtime` | Operator | Runtime Configuration v2 loader | Rust runtime after admission |
| `authority.processPolicy` | Operator | Process Policy v1 loader | Process capability authority |
| `authority.artifactPolicy` | Operator | Artifact Policy v1 loader | Artifact capability authority |
| `authority.infrastructurePolicy` | Operator | Infrastructure Policy v1 loader | Infrastructure policy authority |
| `authority.policyTrust` | Operator | Infrastructure policy trust loader | Signature verification only |
| `authority.emergencyDeny` | Operator | Infrastructure Policy v1 loader | May only remove authority |
| `authority.reviewerDirectory` | Operator | Reviewer Directory v1 loader | Reviewer identity authority |
| `remoteExecution.targetRegistry` | Operator | Target Registry v1 loader | Target eligibility authority |
| `remoteExecution.targetTrust` | Operator | Target Trust v1 loader | Worker transport identity authority |

The manifest itself grants no capability.

### Reference rules

- References are relative to the canonical manifest directory.
- References use forward slashes on every operating system.
- Absolute paths, backslashes, NUL bytes, and `..` path segments are rejected.
- Canonical targets must remain beneath the project directory.
- A symlink is permitted only when its canonical target remains beneath the project directory; all loaders receive the canonical target.
- Directory workflow references include direct `.woml` files only.
- A `.woml` symlink found in a referenced directory is checked using the same containment rule.
- Duplicate authored references, canonical files, and runnable workflow IDs are rejected.
- Workflow and configuration enumeration uses deterministic lexical order.
- Project discovery is not part of CDX1. CDX2 may inspect only `./woml.project.json`; it must not search parent directories.

### Explicit trust dependencies

- `authority.infrastructurePolicy` and `authority.policyTrust` must appear together.
- `authority.emergencyDeny` requires both fields above.
- `remoteExecution.targetRegistry` and `remoteExecution.targetTrust` must appear together.
- Worker private keys and worker configuration are never project fields.
- OpenTelemetry configuration remains owned by Runtime Configuration v2.

### Bounds

| Input | Bound |
| --- | ---: |
| Manifest bytes | 256 KiB |
| JSON depth | 16 |
| JSON values | 10,000 |
| Workflow references | 256 |
| Resolved workflow files | 1,024 |
| Individual workflow bytes | 4 MiB |
| Individual referenced JSON file | 1 MiB, with stricter existing loader limits still authoritative |
| Referenced JSON identity depth and values | 64 and 100,000 |
| Project name bytes | 128 |
| Reference bytes | 4,096 |

Duplicate JSON keys, malformed Unicode, unknown fields, non-finite numbers, and future profiles fail closed.

## Deterministic Project Identity

Project identity is:

```text
sha256(canonical-json(identity-material))
```

Identity material contains:

- the project profile, version, and name;
- canonical project-relative workflow paths;
- normalized-LF workflow content digests;
- discovered runnable workflow IDs;
- canonical project-relative configuration paths;
- canonical JSON content digests for public configuration;
- prepared Infrastructure Policy digests;
- prepared target registry and certificate-bound trust digests.

Identity material never contains:

- an absolute path;
- a secret value;
- an environment override;
- a private key;
- mutable health or capacity observations.

JSON object keys and resolved files are sorted lexically. Project-relative paths use `/` and NFC normalization. Moving an identical project directory does not change its identity. Changing a workflow, policy, trust certificate, registry, reviewer directory, or runtime configuration does.

## Option Conflict Rule

When a project supplies a configuration category, an explicit command-line option for the same category is an error:

```text
WOML_PROJECT_OPTION_CONFLICT
```

There is no silent precedence for authority. Harmless execution presentation options are outside this rule and will be enumerated by CDX2.

## Project Check v1

Project Check v1 is frozen for CDX3 implementation. Its profile is `woml.project-check/v1`; the schema is `docs/schemas/project-check.v1.schema.json`.

The contract has:

- overall `ready`, `degraded`, or `blocked` status;
- the public project name, digest, and manifest path;
- bounded workflow readiness and capability requirements;
- bounded subsystem status;
- bounded, redacted diagnostics with a corrective action.

`woml check` must not execute workflow code, bind listeners, contact providers, resolve secret values, start workers, dispatch attempts, or probe networks implicitly.

## Frozen Capstone Journey

The capstone is a reviewed production deployment, composed only from existing syntax and service contracts:

1. A `<lock key="production/{{...}}" acquire-timeout="..." lease="...">` serializes the deployment and exposes `context.lock.fencingToken`.
2. `services.process.run()` invokes a policy-approved Terraform plan command.
3. `services.artifacts.put()` stores the plan and recovery inputs as durable artifacts.
4. A normal `<step>` computes deterministic risk evidence and may call a separately configured AI service.
5. `<approval review="{{...}}" approvals="..." required-roles="...">` binds decisions to the immutable review.
6. A `<saga>` contains the approved effect.
7. `<step target="production-private">` places the effect on the operator-owned remote target.
8. `<compensate>` and `<cleanup>` define correction behavior.
9. `<wait duration="..." />` or `<wait signal="..." timeout="..." />` suspends durably.
10. `services.progress.report()` and Process v2 output publish bounded operational progress while OpenTelemetry observes the existing runtime facts.

The reference implementation will derive its full source from reviewed examples including `examples/streamingOutput/terraformRelease.woml` and the feature-specific fixtures. CDX0 does not invent replacement syntax.

## Capability Integration Matrix

| Capability | Authoring | Project reference | Resolution/preflight | Runtime and recovery | Operator surface | Packaged gate |
| --- | --- | --- | --- | --- | --- | --- |
| Secure process | `services.process.run()` | `authority.processPolicy` | Existing Process Policy v1 loader | Rust process containment and cancellation | Process diagnostics and streamed output | Existing, composed again in CDX7 |
| Durable artifacts | `services.artifacts.*` | `authority.artifactPolicy` | Existing Artifact Policy v1 loader | Durable metadata, integrity, backup, retention | Inspect, export, maintain, observe | Existing, composed again in CDX7 |
| Waits and signals | `<wait>` and external signals | Runtime only | Compiled requirement and runtime readiness | Durable timer/signal authority | Wait inspect and signal submit | Existing, composed again in CDX7 |
| Locks and leases | `<lock>` | Runtime only | Compiled requirement and store readiness | FIFO lease, fencing, expiry, recovery | Lock inspect and observe | Existing, composed again in CDX7 |
| Compensation | `<saga>`, `<compensate>`, `<cleanup>` | Runtime only | Compiled correction requirements | Durable reverse correction and cleanup | Correction inspect and recover | Existing, composed again in CDX7 |
| Rich approvals | `<approval review="...">` | `authority.reviewerDirectory` | Existing reviewer loader and binding validation | Durable decision authority | Approval providers and inspection | Existing, composed again in CDX7 |
| Infrastructure policy | Service effects and remote placement | Policy, trust, emergency deny | Existing signed-policy resolver | Per-effect evidence evaluation | Policy check and explanation | Existing, composed again in CDX7 |
| Streaming output | Process v2 and `services.progress.report()` | Artifact/runtime policies | Output-store readiness | Ordered, redacted operational records | `woml logs`, progress inspection | Existing, composed again in CDX7 |
| OpenTelemetry | No new WOML syntax | Runtime Configuration v2 | Configuration-only validation | Failure-isolated signal export | Health and collector tooling | Existing, composed again in CDX7 |
| Remote execution | `target="..."` | Registry and trust | Existing target registry loader | Dispatch authority and worker journal | Target, worker, dispatch commands | Packaged loopback journey complete in CDX7 |

## Threat Model

| Threat | Required control |
| --- | --- |
| Manifest path traversal | Relative references, canonical containment, no `..` segments |
| Symlink escape | Canonicalize before loading and reject targets outside project root |
| Duplicate-key ambiguity | Strict JSON with duplicate-key rejection |
| Oversized or deeply nested JSON | Frozen byte, depth, and value limits |
| Authority substitution by CLI | Same-category project/CLI conflict fails closed |
| Secret smuggling | Closed schema, reference-only fields, redacted errors and identity |
| Policy weakening | Existing loaders remain authoritative; manifest cannot inline or override policy |
| Trust substitution | Policy and target trust are explicit and digest-bound |
| Target substitution | Registry and certificate-bound trust participate in project identity |
| Workflow aliasing | Canonical file deduplication and unique runnable workflow IDs |
| Cross-platform identity drift | Forward-slash relative paths, LF source normalization, canonical JSON |
| TOCTOU during activation | CDX2/CDX3 must reuse existing source snapshots and activation checks |
| Accidental parent-project adoption | No ancestor discovery |
| Optional-provider side effects during check | Resolution loads configuration only; provider SDKs and networks remain inactive |
| Remote effect duplication | RET9/CDX7 must preserve ambiguity and no-automatic-retry rules |

## Diagnostics

| Code | Meaning | Corrective action |
| --- | --- | --- |
| `WOML_PROJECT_NOT_FOUND` | Manifest or referenced file is unavailable | Restore it or select an existing relative reference |
| `WOML_PROJECT_INVALID` | Closed schema, syntax, bound, or file-kind violation | Repair the manifest using Project v1 |
| `WOML_PROJECT_UNSUPPORTED_VERSION` | Unsupported profile or version | Use `woml.project/v1`, version `1` |
| `WOML_PROJECT_PATH_ESCAPE` | Canonical reference leaves the project | Move it beneath the project and use a relative path |
| `WOML_PROJECT_DUPLICATE_WORKFLOW` | File or runnable workflow ID is duplicated | Remove the duplicate or rename the workflow |
| `WOML_PROJECT_OPTION_CONFLICT` | Project and CLI both supplied one authority category | Select exactly one source |

Errors from an existing configuration loader retain that loader's established code and corrective action.

## Performance and Packaging Budgets

The frozen budgets live in `woml/tests/fixtures/combined-developer-experience/budgets.v1.json`.

The essential zero-cost rule is exact rather than statistical: an unused optional provider, remote worker, listener, or network client starts zero times during resolution and check. CDX8 will add stable benchmark harnesses before enforcing timing and package-size budgets in release CI.

## RET9 / CDX7 Release Gate

CDX7 closes the packaged loopback portion of RET9. The coordinator now offers
integrity-bound dispatch packages over authenticated mTLS sessions, the worker
commits acceptance and start to its local journal before acknowledging either
fact, and the coordinator commits terminal facts through the durable dispatch
authority. The installed native addon executes the reviewed attempt and
replays its exact terminal fact after redelivery.

The public native coordinator entry point deliberately binds only to
`127.0.0.1` on an ephemeral port. It does not expose a caller-selected network
listener. Production network exposure remains an operator-owned transport and
trust decision rather than an implicit developer-experience feature.

The cross-platform gate covers:

- pre-acceptance loss returning to the durable request queue;
- post-acceptance or post-start loss becoming ambiguous;
- authenticated offer, acceptance, start, and terminal ordering;
- terminal replay from the worker journal;
- draining, cancellation, shutdown, reconciliation, backup, restore, and
  feature-specific recovery through the existing reviewed suites;
- local-only activation without starting remote infrastructure.

No missing or unavailable remote path falls back to local execution. CDX8 is
still required before the combined experience is declared release-ready.

## CDX4 reference project

`examples/infrastructureAgent/` is the reviewed capstone source. Its default
adapters implement deterministic local substitutes for Terraform, Prometheus,
Slack, and AI risk classification. The project contains no private key or
credential, denies infrastructure effects by default, and keeps worker-local
configuration outside `woml.project.json`. The remote apply never falls back to
local execution.

## CDX5 presentation projection

The non-durable `woml.project-activation-presentation/v1` and
`woml.combined-run-presentation/v1` views join already-authoritative project,
run, wait, lock, correction, artifact, operational-output, and dispatch facts.
They grant no authority and append no workflow event. Human output names the
current blocking state and prints bounded next commands; JSON remains a single
versioned object without color or cursor control.

## CDX6 editor and agent authoring

The VS Code extension associates `woml.project.json` with the frozen Project v1
schema and exposes snippets for approvals, locks, sagas, waits, artifacts,
progress, remote targets, and the reviewed safe-deployment pattern. Generated
environment declarations use the compiler-owned script-host surface. Newly
initialized projects include `AGENTS.md`, which directs automated authors to
validation and safe rehearsal commands and forbids weakening policies or
placing secrets in project files.

## CDX7 packaged remote execution

The worker advertises only capabilities it can execute, resolves symbolic
secrets locally, and uses the existing process-policy authority for remote
process calls. Dispatch lifecycle facts cross three durable barriers:

1. the worker journal records acceptance before the coordinator receives
   `accepted`;
2. the journal records start before the coordinator receives `started`;
3. the journal records the terminal fact before the coordinator settles the
   dispatch.

Started work recovered without a terminal fact is ambiguous and cannot enter
ordinary retry. An offer lost before acceptance may safely return to the queue.
The coordinator owns scheduling and settlement throughout; a worker never
appends workflow events or schedules downstream nodes.
