# WOML Parallel Diagnostics v0.1

Status: frozen for P0/P1

Frontend diagnostics keep the existing `{ code, phase, message, file,
location, hint? }` shape. Runtime/N-API failures keep identity in structured
details rather than only in prose.

| Condition | Stable code | Phase |
|---|---|---|
| Missing parallel ID | `WOML_MISSING_ATTRIBUTE` | validation |
| Empty parallel group | `WOML_PARALLEL_EMPTY` | validation |
| Duplicate step/branch/parallel ID | `WOML_DUPLICATE_ID` | validation |
| Invalid or excessive concurrency | `WOML_PARALLEL_INVALID_CONCURRENCY` | validation |
| Invalid `on-error` | `WOML_PARALLEL_INVALID_POLICY` | validation |
| Unsupported direct child | `WOML_PARALLEL_CHILD_UNSUPPORTED` | validation |
| Static sibling reference | `WOML_PARALLEL_SIBLING_REFERENCE` | compile |
| Root terminal parallel group | `WOML_PARALLEL_TERMINAL_UNSUPPORTED` | validation |
| Valid syntax before P2 lowering | `WOML_PARALLEL_LOWERING_NOT_IMPLEMENTED` | compile |
| Malformed compiled group | `WOML_PARALLEL_GROUP_INVALID` | runtime boundary |
| One or more children failed | `WOML_PARALLEL_CHILD_FAILED` | runtime |
| Engine-requested Worker cancellation | `WOML_SCRIPT_CANCELLED` | runtime |
| Inconsistent durable group history | `WOML_PARALLEL_EVENT_HISTORY_INVALID` | runtime/recovery |

The P1 operation vocabulary contains only `<script>`, so there is currently no
WOML attribute inside a child from which to express a static sibling reference.
The sibling-reference code is reserved now and becomes testable when a
declarative child operation with WOML reference attributes is introduced.
Dynamic JavaScript reads do not create dependencies and observe the frozen
pre-fork context.

The parallel N-API error `details` object is frozen as:

```json
{
  "parallelId": "fieldData",
  "policy": "fail-fast",
  "primaryNodeId": "loadWeather",
  "failedNodeIds": ["loadWeather"],
  "cancelledNodeIds": ["loadSoil"]
}
```

The lists use compiled child order. Fields are structured and may not be
encoded only in the message. A child script failure maps to that child's
`<script>` source span; a group contract/history failure maps to the
`<parallel>` opening tag. CLI success stays JSON-only on stdout; diagnostics
use stderr and a nonzero exit code.
