# WOML Run Events v2

Status: frozen for the WOML branch executable profile

This document is the normative semantic contract for durable branch selection
and failure folding. `docs/schemas/run-event.v2.schema.json` is normative for
individual event shapes. The reviewed successful history is
`woml/tests/fixtures/run-events/branch-selected.events.v2.json`.

## 1. Authority

The ordered append-only event log is the authority for a workflow run. Run
status, attempt state, branch selections, and script-facing context are derived
projections. No mutable scheduler object is authoritative.

Version 2 preserves all v1 sequential event meanings and adds one executable
event: `branch_selected`.

## 2. Version Compatibility

Every event in a v2 run contains `eventSchemaVersion: 2`. One run history never
mixes v1 and v2 events.

The Rust reader must continue to accept complete immutable v1 histories. It
selects one fold implementation from the first `run_started` event and rejects
later events with a different schema version.

Run-event v1 remains unchanged. New branch-capable runs use v2.

## 3. Definition Binding

`run_started` binds a run to one immutable compiled definition through
`workflowId` and `definitionHash`.

The hash is SHA-256 over the RFC 8785 JSON Canonicalization Scheme encoding of
the complete compiled definition. It is rendered as `sha256:` plus 64 lowercase
hexadecimal characters.

The reviewed branch definition hash is:

```text
sha256:6a9b3aa53e81ae0e95414f80df0192de5ff11489e9b65b1254b69b71a496155a
```

It binds to `woml/tests/fixtures/branch.compiled.v2.json`.

## 4. Event Vocabulary

Version 2 executes:

- `run_started`
- `step_attempt_started`
- `step_attempt_succeeded`
- `step_attempt_failed`
- `branch_selected`
- `run_succeeded`
- `run_failed`

The v1 meanings of run and attempt events remain unchanged except for the
versioned v2 `run_failed` payload described in Section 7.

## 5. `branch_selected`

The event payload is:

```json
{
  "branchId": "decision",
  "armId": "decision:when:0"
}
```

`branchId` is the public structural branch ID. `armId` is the selected outgoing
selector-edge ID from the immutable compiled definition.

Legal arm identities are:

```text
<branchId>:when:<zeroBasedIndex>
<branchId>:otherwise
```

Semantic validation requires:

1. The bound model contains one selector for `branchId`.
2. The selector is ready and has not already been selected.
3. `armId` identifies one outgoing edge on that selector.
4. All earlier `<when>` conditions resolved to false when a later arm is
   selected.
5. An otherwise arm is selected only when every `<when>` condition is false.
6. At most one `branch_selected` exists for one branch in one run.

The event contains identities only. It does not duplicate the condition value
already available through earlier output events.

Folding it records:

```json
{
  "branchSelections": {
    "decision": "decision:when:0"
  }
}
```

`branchSelections` is internal projection state. It is not exposed in the
script-facing `context`, and `branch_selected` publishes no `context.steps`
value.

## 6. Active and Inactive Routes

After `branch_selected`:

- the selected selector edge becomes active;
- every sibling selector edge becomes inactive;
- activity propagates through the selected route;
- nodes reachable only through inactive edges are skipped without attempts;
- skipped nodes do not fail and do not publish context outputs; and
- inactive incoming routes do not block the result/join node.

Recovery uses the recorded selection. It never re-evaluates a persisted branch
into a different route.

If the process crashes before `branch_selected` is appended, the engine may
re-evaluate the condition because reference resolution and boolean comparison
are pure derivation from the same folded context and immutable definition.

## 7. Failure Scopes

`step_attempt_failed` remains restricted to the canonical Attempt Failure v1
taxonomy shared with the Bun host protocol.

Version-2 `run_failed.data` is a closed union selected by `failureScope`.

### 7.1 Attempt-scoped run failure

```json
{
  "failureScope": "attempt",
  "nodeId": "reviewContent",
  "attempt": 1,
  "invocationId": "inv_branch_review_01",
  "failure": {
    "kind": "script_threw",
    "code": "WOML_SCRIPT_THROWN",
    "message": "Review failed."
  }
}
```

The identity must reference an earlier failed attempt in the same run.

### 7.2 Branch-scoped run failure

```json
{
  "failureScope": "branch",
  "branchId": "decision",
  "armId": "decision:when:0",
  "path": ["steps", "checkContent", "needsReview"],
  "failure": {
    "kind": "branch_test_not_boolean",
    "code": "WOML_BRANCH_TEST_NOT_BOOLEAN",
    "message": "Branch test must resolve to a JSON boolean.",
    "actualType": "string"
  }
}
```

The frozen branch failure pairs are:

| Kind | Code |
|---|---|
| `branch_test_not_boolean` | `WOML_BRANCH_TEST_NOT_BOOLEAN` |
| `reference_not_available` | `WOML_REFERENCE_NOT_AVAILABLE` |
| `branch_selection_invalid` | `WOML_BRANCH_SELECTION_INVALID` |

Branch evaluation errors are not fabricated script attempts. Conversely, a
script-host failure cannot claim a branch failure kind.

## 8. Branch Result Publication

The selected arm's `<result>` is a pure `engine.branch-result` handler
invocation. It uses the normal attempt identity and successful output events so
the existing fold publishes:

```text
context.steps.<branchId> = output
```

In durable mode, result evaluation plus `step_attempt_started` and
`step_attempt_succeeded` are committed atomically. The operation never crosses
the Bun boundary and cannot perform an external side effect.

The reviewed history publishes:

```json
{
  "context": {
    "trigger": {},
    "steps": {
      "checkContent": { "needsReview": true },
      "reviewContent": { "status": "reviewed", "accepted": true },
      "decision": { "status": "reviewed", "accepted": true },
      "publishDecision": { "message": "Final status: reviewed" }
    }
  }
}
```

There is no `acceptContent` output because that arm was inactive.

## 9. Legal Ordering

In addition to v1 attempt ordering, a successful branch history requires:

1. The selector's structural predecessors succeed.
2. Exactly one `branch_selected` is appended.
3. Only nodes on the selected route start attempts.
4. The selected route completes.
5. The branch-result attempt is atomically started and succeeded.
6. Downstream nodes may then become ready.
7. One terminal run event ends the history.

No execution event follows `run_succeeded` or `run_failed`.

## 10. Recovery Rules

- A completed `branch_selected` is authoritative after restart.
- No selected route may change because a referenced source value later changes;
  the immutable run history already fixed the selection.
- An interrupted selected script attempt follows the v1 fail-closed rule and is
  not replayed automatically.
- The atomic pure branch-result publication cannot reopen as an ambiguous
  external-effect attempt.
- Folding the same complete history always reconstructs the same selections,
  context, terminal result, and status.

