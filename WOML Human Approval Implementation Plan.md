# WOML Human Approval Implementation Plan

Status: A0–A1 complete — approval contracts are frozen and frontend validation
is implemented; A2 lowering is next

## 1. Product Outcome

This milestone lets a WOML workflow stop at a durable human decision, show the
reviewer an approval URL, and continue through the selected route after the
reviewer approves or rejects it.

After A0–A7 are complete, a workflow author can:

- declare a first-class `<approval>` in WOML;
- give the approval an ID, name, description, timeout, and timeout policy;
- run different WOML steps after approval and rejection;
- receive one predictable decision value at
  `context.steps.<approvalId>`;
- run later steps using that decision;
- see a clear approval request and one-time capability URL in the terminal;
- approve or reject from a small browser page or the HTTP API;
- survive a process restart without losing the waiting workflow;
- resolve decision-versus-timeout races exactly once; and
- inspect a versioned durable history of the request and resolution.

There is no `woml.resume()` package API. HTTP is the only public decision
mechanism.

The acceptance command remains:

```bash
woml run approval.woml
```

The command pauses visibly, prints the approval URL, waits for the HTTP
decision, continues the selected route through Rust, and prints the final JSON
result.

## 2. Concrete Product Experience

### 2.1 Acceptance workflow

```xml
<workflow version="0.1" id="publishArticle" name="Publish Article">
  <triggers>
    <manual id="start" />
  </triggers>

  <steps>
    <step id="prepareArticle">
      <script>
        return {
          title: "WOML reaches human review",
          ready: true
        };
      </script>
    </step>

    <approval
      id="editorApproval"
      name="Editorial approval"
      description="Approve the prepared article for publication"
      timeout="24h"
      on-timeout="reject">
      <when-approved>
        <step id="publish">
          <script>
            return {
              published: true,
              title: context.steps.prepareArticle.title
            };
          </script>
        </step>
      </when-approved>

      <when-rejected>
        <step id="recordRejection">
          <script>
            return {
              published: false,
              reason: "Human review rejected publication"
            };
          </script>
        </step>
      </when-rejected>
    </approval>

    <step id="finalStatus">
      <script>
        return {
          decision: context.steps.editorApproval.decision,
          source: context.steps.editorApproval.source,
          published:
            context.steps.editorApproval.decision === "approved"
        };
      </script>
    </step>
  </steps>
</workflow>
```

### 2.2 Terminal behavior

Progress information goes to stderr so stdout remains machine-readable. Before
the decision, the user sees:

```text
⏸ Workflow paused — approval required

Workflow: Publish Article
Approval: Editorial approval
Description: Approve the prepared article for publication
Run: run_01...
Timeout: 24h

Open this URL to approve or reject:
http://127.0.0.1:7331/approvals/apr_01.secret
```

The process stays alive while the workflow is waiting. Opening the URL displays
the approval name and description with **Approve** and **Reject** buttons.
Opening the page never changes workflow state; a button submits an HTTP `POST`.

After approval, stdout receives only the final workflow result:

```json
{"decision":"approved","source":"human","published":true}
```

### 2.3 Runtime ownership

```text
WOML file
   |
   v
TypeScript parser/compiler -------- source-located diagnostics
   |
   v
Rust model-v4 DAG + event-v4 engine
   |                     |
   | waiting outcome     | SQLite events + token index
   v                     |
Bun CLI/HTTP server <----+
   |
   +-- terminal URL
   +-- safe GET approval page
   +-- POST decision ----> Rust atomic decision + DAG continuation
```

- TypeScript owns WOML parsing, validation, lowering, source locations, the
  local HTTP transport, and the small approval page.
- Rust owns compiled-model validation, token generation and verification,
  durable waiting state, decision/timeout arbitration, event folding, selected
  route execution, and recovery.
- Bun Workers continue to execute JavaScript `<script>` bodies only. They are
  not an approval-state authority.

## 3. Phase Summary

| Phase | What changes | Product result |
|---|---|---|
| A0 — complete | Freeze approval syntax, model v4, event v4, token, HTTP v1, timeout, errors, and fixtures. | Every layer targets one reviewed approval contract. |
| A1 — complete | Teach the WOML frontend to validate approval markup and placement. | Valid approvals pass frontend validation and invalid approvals receive useful source errors. |
| A2 | Lower approvals into a deterministic model-v4 DAG. | Approval markup becomes an engine-ready wait/route/join graph. |
| A3 | Add durable approval events, folded waiting state, token records, and SQLite migration. | Waiting approvals and access credentials survive restarts safely. |
| A4 | Make Rust stop at an approval and return a structured waiting outcome. | A workflow can durably pause without blocking a Worker or pretending to finish. |
| A5 | Add atomic approve, reject, timeout, and DAG continuation behavior. | Exactly one resolution wins and only its route executes. |
| A6 | Add the local HTTP approval page, terminal URL, and N-API/CLI lifecycle. | A person can approve or reject a real `woml run` workflow. |
| A7 | Harden recovery, races, composition, security, packaging, and documentation. | Human approval becomes a supported and publishable WOML feature. |

## 4. Source-Language Contract

### 4.1 `<approval>`

```xml
<approval
  id="editorApproval"
  name="Editorial approval"
  description="Approve the prepared article"
  timeout="24h"
  on-timeout="reject">
  <when-approved>...</when-approved>
  <when-rejected>...</when-rejected>
</approval>
```

| Attribute | Required | Meaning |
|---|---:|---|
| `id` | Yes | Stable structural identity and output key at `context.steps.<id>`. |
| `name` | No | Human-readable label shown by the terminal and approval page. |
| `description` | No | Human-readable explanation shown to the reviewer. |
| `timeout` | No | Durable maximum wait; omission means no WOML deadline. |
| `on-timeout` | No | `reject` or `fail`; defaults to `fail`. |

Rules:

- Approval IDs use the existing JavaScript-safe structural-ID grammar.
- Step, branch, parallel, and approval IDs share one workflow-wide namespace.
- `<when-approved>` and `<when-rejected>` are both required exactly once and
  must appear in that order.
- Each decision arm is a sequential container with zero or more flow items.
- Empty decision arms are valid no-op continuations.
- A root workflow containing only an approval is valid.
- Approval may appear in root `<steps>`, a selected branch arm, or another
  approval arm.
- Approval is not accepted as a direct child of the current `<parallel>`
  profile, which still accepts only direct script steps.
- A workflow may contain multiple approvals. They are reached and resolved in
  normal DAG order.
- `on-timeout` without `timeout` is rejected because it can never take effect.
- Durations use the existing frozen WOML duration rules and must fit the Rust
  millisecond range without truncation or overflow.

### 4.2 `<notify>` executable status

The fundamental syntax keeps the optional `<notify><script>...</script></notify>`
shape, but the first publishable approval profile does not execute it.

The CLI itself always provides the first notification channel by printing the
approval request and URL. A `<notify>` element receives
`WOML_FEATURE_NOT_EXECUTABLE` until services, secret delivery, notification
idempotency, and a safe non-persisted approval-URL binding are designed.

This prevents the language from accepting notification code that cannot safely
receive credentials or call configured services. Enabling `<notify>` later must
not change the approval model, events, decision output, or HTTP contract.

### 4.3 Decision output

A successful human or timeout-reject decision publishes exactly one value:

```json
{
  "decision": "approved",
  "source": "human",
  "decidedAt": "2026-08-06T12:00:00.000Z"
}
```

- `decision` is `approved` or `rejected`.
- `source` is `human` or `timeout`.
- `decidedAt` is the timestamp of the durable resolution event.
- The value is reconstructed by folding events; it is not an independently
  mutable approval record.
- It becomes available at `context.steps.<approvalId>` before the selected arm
  runs and remains available to all downstream steps.
- `on-timeout="fail"` publishes no approval output and executes neither arm.
- No reviewer identity, arbitrary payload, comment, form data, or token is part
  of the v0.1 output.

## 5. Compiled Workflow Model v4

A0 will pin `compiled-workflow-model.v4.schema.json` and a reviewed
`approval.compiled.v4.json` fixture.

Model v4 must represent approval as a real DAG wait/route/join structure. It
must include:

- the public approval ID;
- deterministic generated wait, approved-route, rejected-route, and join
  identities where required;
- `timeoutMs` when authored;
- `onTimeout` as `reject` or `fail`;
- both arm identities and their entry/exit topology;
- optional display metadata; and
- deterministic downstream edges.

The exact generated IDs and handler names are frozen in A0. The following rules
are non-negotiable:

- the model is a DAG, not a suspended callback chain;
- random token data, URLs, ports, timestamps, and run IDs never enter the
  compiled model or definition hash;
- approval metadata never affects control identity;
- an approval's public output identity remains its authored ID;
- empty arms lower deterministically without inventing a script effect;
- the join waits only for the selected arm;
- unselected-arm nodes are never ready; and
- model v1–v3 definitions remain readable and behaviorally unchanged.

## 6. Event Vocabulary v4 and Folded State

A0 will pin `run-event.v4.schema.json`, `run-events-v4.md`, and reviewed event
fixtures for human approval, human rejection, timeout rejection, and timeout
failure.

### 6.1 Approval events

Event v4 adds two approval-specific events:

1. `approval_requested`
   - `approvalId`
   - `requestId`
   - optional `expiresAt`
   - `onTimeout`
2. `approval_resolved`
   - `approvalId`
   - `requestId`
   - a tagged resolution:
     - human decision: approved or rejected;
     - timeout decision: rejected; or
     - timeout failure.

For `on-timeout="fail"`, `approval_resolved` and approval-scoped `run_failed`
are appended in one SQLite transaction. Generic `run_paused` and `run_resumed`
events are not introduced; approval events already identify why the run waits
and why it continues.

### 6.2 Projection

The folded projection gains:

- `RunStatus::Waiting`;
- approval requests keyed by approval ID;
- request identity, deadline, timeout policy, and resolution state; and
- the decision output in `context.steps` after a decision resolution.

`approval_requested` changes a reachable sequential run from running to
waiting. A decision resolution returns it to running. A timeout-failure
resolution followed by `run_failed` makes it failed.

The event log remains the workflow-state authority. An in-memory timer, HTTP
request, browser page, N-API Promise, or token table cannot make a workflow
resolved without the durable event.

### 6.3 Compatibility

- One run uses exactly one event schema version.
- Model v4 starts event-v4 histories.
- Event-v1, v2, and v3 fixtures remain readable and unchanged.
- Recovery validates event identity against the immutable compiled definition.
- Impossible duplicate requests, mismatched request IDs, repeated resolutions,
  or events after a terminal run are rejected by both validation and folding.

## 7. Approval Token and Persistence Contract

### 7.1 Token form

The local approval URL carries a high-entropy capability token with a public
lookup ID and a secret component:

```text
apr_<tokenId>.<secret>
```

Rust generates both components with a cryptographically secure random source.
Possession of a valid token authorizes one decision for one approval request in
the local v0.1 profile.

### 7.2 What is persisted

SQLite store schema v2 adds an append-only approval-token index containing:

- token ID;
- hash of the secret component;
- request ID, run ID, and approval ID;
- issued timestamp; and
- credential expiry timestamp.

The plaintext token is returned once to the CLI and is never stored in:

- compiled definitions;
- run events;
- workflow context;
- step outputs;
- error envelopes; or
- the SQLite token index.

Secret comparison is constant-time. URLs and tokens are redacted from runtime
errors and access logs.

Credential expiry is separate from the WOML approval deadline. The local v0.1
token lifetime defaults to 24 hours and is capped by the approval deadline when
that deadline arrives sooner. An approval with no WOML timeout may remain
waiting after one URL expires; recovery can issue a fresh URL for the same
request without changing workflow state.

### 7.3 Restart and reissue behavior

If the process restarts while an approval is waiting, Rust may issue another
token row for the same unresolved request and the CLI prints a new URL. Earlier
unexpired tokens remain valid until the request resolves or expires. This makes
recovery possible without storing plaintext credentials.

All valid tokens converge on the same atomic approval resolution, so multiple
URLs cannot produce multiple decisions.

Resolved token rows are retained with the local database in v0.1. A later data
retention feature may remove expired credentials only after its audit and
idempotency behavior is specified.

### 7.4 Store migration

The existing store is schema v1. A3 adds a tested, transactional v1-to-v2
migration that creates the token index without rewriting immutable definitions
or run events. Existing sequential, branch, and parallel databases must reopen
successfully after migration.

## 8. HTTP Decision Contract v1

The initial server binds only to `127.0.0.1` and defaults to port `7331`.
Remote binding, TLS termination, account authentication, and reviewer RBAC are
production-server features, not implicit behavior of the local CLI.

The CLI may select another loopback port with `--approval-port <port>`. It must
never fall back to a public interface or an unreported random port.

### 8.1 Browser page

```http
GET /approvals/{token}
```

The response is a small HTML page showing the workflow and approval metadata,
deadline, and Approve/Reject controls. GET is read-only. It never records a
decision, even when crawled, prefetched, or refreshed.

The response uses `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, a
restrictive Content Security Policy, no external resources, and frame-denial
headers. A browser POST with an `Origin` header must match the loopback server;
command-line clients without an `Origin` header remain supported.

The page does not expose the workflow context, step outputs, compiled model,
event history, database path, or internal errors.

### 8.2 Decision endpoint

```http
POST /api/v1/approvals/{token}/decision
Content-Type: application/json

{
  "decision": "approved"
}
```

Only `approved` and `rejected` are accepted.

| Status | Meaning |
|---:|---|
| `200` | Decision recorded, or the identical human decision was already recorded. |
| `400` | Invalid JSON, content type, or decision value. |
| `404` | Token is unknown or malformed; the response does not reveal approval existence. |
| `409` | The approval already has a different human decision. |
| `410` | The credential expired, or the request was resolved by timeout. |
| `500` | A safe internal failure occurred before a decision could be confirmed. |

The JSON response has a versioned success/error envelope frozen in A0. It may
report the run and approval IDs after successful authorization but never echoes
the token.

### 8.3 Atomicity

Human decisions and timeout processing enter Rust through separate calls but
compete inside one SQLite transaction:

1. Look up and hash-verify the capability token.
2. Load and validate the immutable run definition and current event history.
3. Compare the durable deadline with the engine clock.
4. Detect an existing resolution and apply duplicate/conflict rules.
5. Append exactly one valid resolution, plus `run_failed` when required.
6. Commit before reporting success.

Only after the transaction succeeds may the CLI ask Rust to continue the DAG.
If continuation crashes, the resolution remains durable and recovery resumes
after it without asking the human again.

## 9. Runtime and CLI Lifecycle

The current native execution call returns only a completed result. Approval
requires a versioned discriminated outcome across N-API:

```text
succeeded -> final result and events
waiting   -> run ID + safe approval metadata + one-time token
```

Rust returns `waiting` rather than holding an in-memory Promise for hours. The
CLI keeps the process alive, serves the URL, and invokes the Rust decision
operation after an HTTP POST. Rust then resumes from the durable run history and
may return either another waiting approval or the final result.

The approval path always uses the durable engine. The CLI defaults to a
project-local `.woml/state.sqlite` and accepts `--state <path>` for tests and
automation. `.woml/` is runtime data and must be ignored by source control.

If the CLI is interrupted, it prints the run ID and this recovery shape:

```bash
woml run approval.woml --resume <runId>
```

`--resume` reopens the stored definition and events, verifies that the supplied
WOML compiles to the bound definition, reissues a token for a still-waiting
request, prints a new URL, and continues from the durable boundary. It is a
local run-recovery command, not another decision mechanism. It never replays a
completed script attempt or a resolved approval.

## 10. Error Surface

Every WOML/frontend error retains code, message, and source line/column. Every
HTTP error uses the versioned JSON envelope. Every Rust/N-API runtime error
retains structured approval identity without including the token.

A0 freezes at least these error families:

| Code | Surface | Meaning |
|---|---|---|
| `WOML_APPROVAL_STRUCTURE_INVALID` | validation | Required arms are missing, duplicated, or out of order. |
| `WOML_APPROVAL_TIMEOUT_INVALID` | validation | Timeout or timeout policy is malformed or contradictory. |
| `WOML_APPROVAL_PLACEMENT_INVALID` | validation | Approval appears in an unsupported container. |
| `WOML_APPROVAL_LOWERING_UNAVAILABLE` | compile | A valid approval reaches the compiler before A2 lowering. |
| `WOML_APPROVAL_STATE_INVALID` | runtime | Compiled/event request identity is inconsistent. |
| `WOML_APPROVAL_REQUEST_INVALID` | HTTP | The decision request body or content type is invalid. |
| `WOML_APPROVAL_TOKEN_INVALID` | HTTP/runtime | Capability token is malformed or unknown. |
| `WOML_APPROVAL_TOKEN_EXPIRED` | HTTP/runtime | The credential expired while the approval may still be waiting. |
| `WOML_APPROVAL_EXPIRED` | HTTP/runtime | The durable request deadline has passed. |
| `WOML_APPROVAL_DECISION_CONFLICT` | HTTP/runtime | A different resolution already won. |
| `WOML_APPROVAL_TIMEOUT` | runtime | `on-timeout="fail"` failed the workflow. |
| `WOML_APPROVAL_SERVER_BIND_FAILED` | CLI | The local approval server could not bind safely. |
| `WOML_APPROVAL_INTERNAL` | HTTP | A decision could not be safely confirmed. |

Exact names and diagnostic locations are reviewed in A0 rather than scattered
through implementation.

## 11. Implementation Phases

### A0 — Freeze contracts and reviewed fixtures — complete

Changes:

- Correct the language and roadmap documents so HTTP is the only public
  decision interface and no `woml.resume()` API remains.
- Freeze the executable approval subset and keep `<notify>` explicitly staged.
- Pin compiled workflow model v4 and its JSON Schema.
- Pin run-event vocabulary v4 and its JSON Schema.
- Pin SQLite token-index/store-v2 shape and migration rules.
- Pin HTTP decision contract v1, response envelopes, and status codes.
- Pin N-API waiting/succeeded outcome shapes.
- Pin token generation, hashing, reissue, duplicate, conflict, and expiry
  behavior.
- Pin timeout clock and atomic race behavior.
- Pin validation/runtime/HTTP diagnostic codes and source locations.
- Add reviewed fixtures for WOML, compiled DAG, approved/rejected/timeout event
  histories, HTTP requests/responses, context, and final results.

Result:

Every layer has one versioned approval contract before executable code changes.

Gate:

The schemas and fixtures are reviewed together, cross-reference the same IDs,
and explicitly contain no plaintext token or URL in compiled/event artifacts.

Completed proof:

- Model v4, event v4, HTTP v1, native outcome v1, durable store v2, token,
  timeout-race, and diagnostic contracts are checked in under `docs/`.
- The reviewed approval source, compiled DAG, context, result, four event
  histories, eight HTTP bodies, and two native outcome fixtures are checked in.
- AJV conformance tests pin the model hash, generated identities, exact script
  bodies, event resolution shapes, HTTP fail-closed behavior, and native
  discriminator shapes.
- Compiled and event artifacts reject tokens, URLs, ports, and credential
  fields. The native waiting contract is the only intentional plaintext-token
  delivery boundary.

### A1 — Validate approval markup in the TypeScript frontend — complete

Changes:

- Move `<approval>`, `<when-approved>`, and `<when-rejected>` into the
  executable element profile.
- Validate attributes, fixed child order, empty/non-empty arms, placement,
  duration bounds, and shared ID namespace.
- Keep `<notify>` recognized but rejected as not executable.
- Apply existing recursive validation to steps, branches, parallels, and nested
  approvals inside decision arms.
- Add source-located tests for missing arms, duplicates, order, unknown
  attributes, invalid timeout combinations, duplicate IDs, and unsupported
  parallel placement.

Result:

The WOML frontend understands the approval language and explains authoring
mistakes at the exact tag or attribute.

Gate:

Valid approval fixtures pass; every invalid fixture produces one stable code,
file, line, column, and helpful message.

Completed proof:

- `validateWoml()` is a public validation pass independent from model lowering.
- `<approval>`, `<when-approved>`, and `<when-rejected>` are accepted in the
  frontend profile; `<notify>` remains explicitly staged.
- Validation covers fixed arm order, empty and nested arms, shared IDs,
  metadata, duration units/bounds, timeout policy, branch composition, and
  unsupported parallel placement.
- `compileWoml()` reports `WOML_APPROVAL_LOWERING_UNAVAILABLE` at the approval
  tag until A2 implements the already-frozen model-v4 DAG.

### A2 — Lower approvals into model-v4 DAGs

Changes:

- Add the reviewed approval node/group representation to TypeScript model
  types and JSON serialization.
- Generate deterministic wait, route, empty-arm, join, and downstream topology.
- Preserve public approval output identity separately from generated control
  identities.
- Extend dominance, reachability, terminal-node, cycle, namespace, and
  definition-hash validation.
- Implement matching model-v4 validation in Rust.
- Add exact compiled fixture and TypeScript/Rust conformance tests.

Result:

Approval markup becomes a deterministic DAG both frontend and Rust understand
identically.

Gate:

The checked-in compiled fixture is byte-for-byte stable, accepted by Rust, and
cannot contain token, URL, clock, or random runtime data.

### A3 — Add durable approval state and credentials

Changes:

- Add event-v4 Rust types, validation, serialization, and folding.
- Add `Waiting` and approval request/resolution projections.
- Publish decision output only from a valid folded resolution event.
- Add store-v2 approval-token index and transactional migration from store v1.
- Generate high-entropy tokens in Rust, persist only hashes, and compare them
  in constant time.
- Implement token reissue for unresolved recovered requests.
- Add event/store conformance fixtures, corruption tests, migration tests, and
  plaintext-token absence tests.

Result:

Approval state and credentials survive restarts without making the token table
the workflow-state authority.

Gate:

Deleting in-memory state and reopening SQLite reconstructs the same waiting or
resolved projection; existing v1–v3 databases migrate without altered events.

### A4 — Pause execution at the durable approval boundary

Changes:

- Teach Rust scheduling to recognize a ready approval node.
- Atomically append `approval_requested` and the first hashed token record
  before exposing a token or URL.
- Return the reviewed `waiting` runtime outcome without starting downstream or
  arm nodes.
- Avoid spawning or retaining a Bun Worker while a run waits.
- Treat approval as a valid terminal item and support empty arms.
- Recover a waiting run without replaying completed scripts.

Result:

A workflow can reach an approval and durably stop at the correct DAG boundary.

Gate:

Tests prove no arm/downstream attempt exists before resolution, no Worker stays
alive solely for waiting, and restart yields the same request identity.

### A5 — Resolve, time out, and continue exactly one route

Changes:

- Implement Rust operations for human decision, due-timeout settlement, and
  continuation from a resolved history.
- Make identical human decisions idempotent and conflicting decisions fail.
- Implement `on-timeout="reject"` as rejected decision output with
  `source="timeout"` and selected rejected arm.
- Implement `on-timeout="fail"` as atomic approval resolution plus run failure
  with no arm execution.
- Continue only the selected decision arm, then join downstream once.
- Support later and nested approvals returning another `waiting` outcome.
- Add an injectable engine clock for deterministic deadline and race tests.

Result:

Approve, reject, and timeout produce one durable outcome and the correct WOML
continuation.

Gate:

Race tests run decision and timeout concurrently many times and always observe
one winner, one resolution event, and at most one selected route.

### A6 — Deliver the terminal URL and HTTP approval experience

Changes:

- Extend the N-API bridge with versioned start/wait/decide/resume outcome
  envelopes.
- Add TypeScript validation for every native approval response.
- Start a loopback-only Bun HTTP server when the CLI receives `waiting`.
- Extend CLI parsing with `--state`, `--resume`, and `--approval-port` while
  keeping `woml run approval.woml` as the default product path.
- Print approval metadata, deadline, run ID, and token URL to stderr.
- Serve the safe GET page and JSON POST decision endpoint.
- Map Rust invalid/expired/conflicting outcomes to the frozen HTTP responses.
- Resume through Rust after a successful decision and print only final JSON to
  stdout.
- Add a clear port-conflict error and guaranteed server shutdown after terminal
  completion/failure.
- Add black-box CLI/browser/API tests for approve, reject, malformed requests,
  duplicates, conflicts, and expiry.

Result:

A person can run `woml run approval.woml`, open the printed URL, decide, and see
the workflow continue.

Gate:

The packaged CLI completes the exact browser/HTTP user journey with Rust as the
only decision authority and no npm runtime package required by the caller.

### A7 — Harden, package, and close the milestone

Changes:

- Test approval at the beginning, middle, and end of root flow.
- Test approval in selected/unselected branch arms and before/after parallel
  groups where current composition rules allow it.
- Test empty, one-item, multi-item, and nested decision arms.
- Test multiple sequential approvals and repeated waiting outcomes.
- Test restart before URL display, while waiting, after decision commit, during
  selected-arm execution, before join, and after completion.
- Test multiple issued tokens, simultaneous decisions, timeout boundary clocks,
  browser refresh/prefetch, malformed tokens, and database contention.
- Verify no generated live plaintext token appears in events, context, errors,
  checked-in snapshots, or access logs. HTTP fixtures use obvious inert test
  credentials only.
- Verify model/event v1–v3 compatibility and all sequential, branch, and
  parallel regressions.
- Run frontend, Rust, N-API, CLI, clean-package, typecheck, Clippy, timing, and
  whitespace verification.
- Update the WOML specification, Rust integration plan, previous roadmaps, CLI
  help, and language maturity table.

Result:

Human approval is a supported WOML feature rather than an in-memory pause or
JavaScript helper.

Gate:

All verification rows below pass in one release build with no skipped native
approval tests.

## 12. Verification Matrix

| Area | Required proof |
|---|---|
| Syntax | Valid approvals parse; invalid structure and attributes report original line/column. |
| Lowering | Deterministic model-v4 wait/route/join identities and canonical hash. |
| Waiting | Request is durable before the URL is exposed; no downstream work starts. |
| Token | High entropy, bounded credential lifetime, hashed at rest, constant-time verification, no plaintext persistence. |
| HTTP | GET is safe; POST accepts only reviewed decisions and returns frozen envelopes/statuses. |
| Decision | Approve/reject publishes exactly one predictable output and selects exactly one arm. |
| Timeout | Reject/fail policies work and race atomically with human decisions. |
| Context | Decision appears only at `context.steps.<approvalId>`; token never appears. |
| Events | Request/resolution validate, fold, persist, migrate, and reopen deterministically. |
| Recovery | Waiting and resolved runs resume without repeating completed effects or decisions. |
| Composition | Root, branch-arm, nested approval, and allowed parallel-adjacent placements behave correctly. |
| Errors | Frontend, Rust, N-API, HTTP, and CLI retain stable codes and safe details. |
| Compatibility | Model/event v1–v3 fixtures and existing workflows remain unchanged. |
| CLI | Terminal asks clearly, prints the URL to stderr, waits, and prints final JSON to stdout. |
| Package | A clean installation includes the native engine, HTTP page, and all required Bun components. |

## 13. Explicit Non-Goals

This milestone does not add:

- `woml.resume()` or any required npm client package;
- state-changing GET links;
- executable `<notify>` scripts;
- Slack, email, SMS, or provider integrations;
- `services`, resolved secrets, or `context.env`;
- remote/public binding, TLS, users, sessions, RBAC, reviewer identity, or
  organization policy;
- arbitrary approval payloads, comments, attachments, custom forms, multiple
  decision values, quorum, delegation, reassignment, or escalation;
- an approval inbox/dashboard;
- approval as a direct child of the current parallel profile;
- retries greater than one;
- webhook, schedule, interval, or event triggers;
- generic durable event waiting;
- workflow cancellation or lifecycle hooks;
- distributed queues or multi-process approval ownership; or
- removal of the legacy chaining SDK.

These require separate contracts and milestones.

## 14. Open Architecture Decisions Carried Forward

Approval must not silently resolve unrelated WOML contracts:

- `context.run` remains unavailable.
- `context.env` and resolved secrets remain unavailable and unpersisted.
- Retry idempotency-key derivation remains unresolved.
- Workflow-level cancellation remains distinct from approval timeout.
- `<notify>` URL delivery and notification idempotency remain deferred to the
  services/capabilities milestone.
- Production reviewer authentication and authorization remain separate from
  the local capability-token profile.
- Production database retention and credential cleanup remain configuration
  and operations decisions.
- Distributed scheduling and queue ownership remain out of scope.

If implementation forces any of these decisions, work pauses for contract
review rather than selecting an invisible default.

## 15. Roadmap After Human Approval

After A0–A7, product expansion continues in this order:

1. **Retries and idempotency** — freeze idempotency-key derivation, duplicate
   effect behavior, retryable failures, durable scheduling, and backoff before
   enabling `retry` values greater than one.
2. **Production triggers** — implement webhook first, then schedule, interval,
   and event triggers with complete payload, validation, delivery, and failure
   contracts. The approval HTTP server may provide reusable transport helpers,
   but trigger registration remains a separate runtime subsystem.
3. **Services and capabilities** — add registered HTTP, database, messaging,
   and provider operations without persisting clients or resolved secrets; then
   make `<notify>` executable with a safe non-persisted approval URL binding and
   notification idempotency.
4. **Lifecycle and engine controls** — add lifecycle hooks, workflow
   cancellation, durable user state, and other engine-owned operations with
   explicit events and race behavior.
5. **Production runtime and operations** — add long-lived hosting,
   multi-workflow registration, deployment configuration, observability,
   retention, and later distributed queue/worker ownership.
6. **SDK migration and retirement** — publish migration tooling and remove the
   old JavaScript chaining SDK only after WOML reaches the agreed parity and
   existing users have a supported path.

## 16. Definition of Done

The Human Approval milestone is complete only when:

- the executable approval syntax and staged `<notify>` status are documented;
- the compiler emits the reviewed deterministic model-v4 DAG;
- Rust durably requests, resolves, times out, folds, and recovers approvals;
- SQLite migrates old stores and retains only hashed token secrets;
- the public decision mechanism is HTTP only;
- GET never changes state and POST follows the frozen v1 contract;
- terminal progress stays on stderr and final workflow JSON stays on stdout;
- approve, reject, timeout-reject, and timeout-fail each have exact event and
  context behavior;
- decision-versus-timeout races have one durable winner;
- only the selected decision arm executes;
- interrupted continuation never repeats the human decision or a completed
  script effect;
- approval composes with existing sequential, branch, parallel-adjacent, and
  nested flows under the published placement rules;
- errors retain stable codes, safe details, and original WOML locations;
- generated live tokens never appear in compiled models, events, context,
  outputs, errors, checked-in snapshots, or stored plaintext;
- old model/event fixtures and all existing workflows remain compatible;
- the clean packaged CLI passes the complete browser approval journey; and
- the WOML maturity table marks Human Approval executable and publishable.
