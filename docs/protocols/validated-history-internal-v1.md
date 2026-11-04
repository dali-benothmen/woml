# Validated history: private execution-state contract

Status: VSR2/VSR3 production implementation with the independent VSR1 reference;
**not a public Rust, N-API or persisted contract**. Contract revision 1 does not
change any WOML schema/version. See the [implementation record](../validated-state-reuse-implementation.md)
for converted callers and open release/resource gates.

## What the result means

Think of a ledger and its checked balance sheet. The result describes the exact
pages that were checked, not any later edition of that ledger.

| Result | Establishes | Does not establish |
| --- | --- | --- |
| Structural `ProjectionFoldState` | Complete event envelope/order/identity and transition checks; append bookkeeping retained | Definition-specific graph readiness, model validity, permission to perform an effect |
| Definition-checked history | Structural state plus all existing history checks against the borrowed definition/hash | A fresh database read, current ownership, policy permission, or approval review hydration |
| Hydrated public projection | Store-owned workflow-ID compatibility enrichment and immutable rich-review output | A reusable raw event-fold state or authority to settle a run |

Existing model-loading/engine constructors still validate the compiled model.
History validation does not become an additional full model validator. In
particular, an empty history can be structurally empty even when no event has
caused the checker to examine the definition. It is not a started/admitted run.

## Construction and ownership

The production result has private fields and construction at the
history-validation boundary. It owns the **complete** `ProjectionFoldState`
while borrowing the exact immutable event slice, definition and definition-hash
input for its usable lifetime.

1. Build the full structural state first. Preserve the existing structural
   error string and its priority over any earlier graph error.
2. Run every current definition/history check in its current order. Continue
   using prefix-before-event semantics and the monotone prefix cursor.
3. Return a result only after complete success. On any rejection, discard all
   tentative states. `apply` may have mutated a candidate before failing.
4. Expose a shared projection reference; never a mutable projection, mutable
   events, `validated: true` flag, serialized certificate or global proof handle.
5. Move the fold state out only by consuming the witness. Its later mutation is
   not definition-checked by this now-destroyed witness. Existing append checks
   and commit-before-cache-publication rules still apply.

The final prefix cursor position is **not** the final history state: the last
requested prefix can stop before terminal/lifecycle events. Iteration-scoped
events skipped by a validator match arm still contribute to prefix state.
Repeated requests for one prefix must not apply those events twice.

The existing `Result<(), String>` validation-only entrypoint remains available
when production reuse is introduced. No public facades, errors or hydration
ordering may change to accommodate the private result.

## Identity and lifetime

Borrow the actual inputs; do not serialize/hash/clone them again merely to bind
identity. An equal event count, same last event ID, equal serialization or
caller-supplied hash is not a certificate for a different allocation/history.
The test-only witness uses pointer identity (including slice length) as an
executable substitution guard. This is not permission to store addresses as
long-lived keys or to extend lifetimes with `unsafe`.

Existing durable cache freshness uses count/tail identity. The new witness
tests do **not** prove that existing cache hits detect arbitrary earlier-row
tampering with the same count/tail. Cache redesign is outside this proposal.

No result crosses an intervening event mutation, definition replacement,
database reload, transaction boundary or external callback as evidence that new
facts are valid. An unchanged immutable snapshot can remain in memory across
an await, but its validity alone never establishes fresh database authority;
do not add awaits or broaden locks/transactions to enable reuse.

Prepared startup ownership is about a store's runtime owner/schema. It does not
prove an event history. A validated history does not prove prepared ownership,
claim/lease freshness, cancellation state or current policy authorization.

## Independent reference and executable specification

The test-only implementation is
[`engine/history/tests/validated_state.rs`](../../core/woml-engine/src/engine/history/tests/validated_state.rs).
It first builds a complete structural state, then calls the unchanged validator
with the original **full replay at each requested prefix**. This deliberately
performs extra work; it is an oracle, not the proposed optimized algorithm.

The reference shares existing transition/definition rules but not the optimized
prefix traversal. Tests compare it with current production acceptance, exact
errors, complete projections and append behavior. Historical projection-only
fixtures that fail graph readiness remain rejected and unchanged.

Coverage includes every recorded/historical prefix, terminal suffixes, scoped
events, repeated prefix requests, structural-versus-graph error priority,
future versions, run/hash substitution, equal-content/different-input objects,
changed earlier output with the same tail, empty histories and consuming state.

Production/reference comparisons now cover the constructor and consumed fold
bookkeeping. Separate durable boundary tests check replay counts, wait rollback
and raw versus hydrated views. Native/package and cross-platform execution,
broader composition and resource measurements remain later gates.

## Allocation and peak-live-state review

The production implementation retains one complete state while the
existing definition checker builds another prefix state. Both can be live
simultaneously; peak memory can increase even if total allocations decrease.
The result itself must not clone the event vector, definition, state, or public
projection just to wrap it. Borrowing adds references, not copies of those trees.

Existing public-view cloning/hydration still costs memory and is included in
the real cache-miss boundary. Moving the accepted state into the bounded cache
does not clone it. The oracle intentionally repeats folds and must never be
used as a candidate performance measurement.

Source-derived clone counts are not allocator measurements. Baseline captures
report unavailable allocation/peak data as `null`, never zero. Before VSR2,
complete the baseline allocation/peak-live-memory capture or explicitly review
that missing evidence; before shipping, compare the actual retained-state peak.
