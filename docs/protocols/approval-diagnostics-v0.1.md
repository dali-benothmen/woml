# WOML Human Approval Diagnostics v0.1

Status: frozen and implemented through Rust, N-API, HTTP, and CLI in A6

Frontend diagnostics carry code, phase, message, file, and exact source span.
Runtime/N-API diagnostics carry structured approval identity but never a token.
HTTP failures use the v1 JSON envelope.

| Code | Phase/surface | Primary location or meaning |
|---|---|---|
| `WOML_APPROVAL_STRUCTURE_INVALID` | validation | Missing, duplicate, reversed, or unsupported approval children; approval or offending child tag. |
| `WOML_APPROVAL_TIMEOUT_INVALID` | validation | Invalid duration/policy combination; offending attribute, or approval tag when absent. |
| `WOML_APPROVAL_PLACEMENT_INVALID` | validation | Approval arm outside approval, or approval directly inside parallel; offending tag. |
| `WOML_APPROVAL_LOWERING_UNAVAILABLE` | compile | Retired A1 gate, reserved so old tooling can still identify the pre-A2 compiler boundary. |
| `WOML_APPROVAL_STATE_INVALID` | runtime | Compiled/event request identity is inconsistent. |
| `WOML_APPROVAL_REQUEST_INVALID` | HTTP | Invalid content type, JSON body, or decision. |
| `WOML_APPROVAL_TOKEN_INVALID` | HTTP/runtime | Token is malformed or unknown. |
| `WOML_APPROVAL_TOKEN_EXPIRED` | HTTP/runtime | Credential expired; the approval may still be waiting. |
| `WOML_APPROVAL_EXPIRED` | HTTP/runtime | Workflow approval deadline already resolved the request. |
| `WOML_APPROVAL_DECISION_CONFLICT` | HTTP/runtime | A different human decision already won. |
| `WOML_APPROVAL_TIMEOUT` | runtime | `on-timeout="fail"` failed the workflow. |
| `WOML_APPROVAL_SERVER_BIND_FAILED` | CLI | Loopback approval server could not bind to the requested port. |
| `WOML_APPROVAL_INTERNAL` | HTTP | Safe generic response for an unconfirmed internal failure. |

Existing generic codes remain authoritative for missing `id`, invalid IDs,
duplicate structural IDs, unknown attributes, empty metadata, and staged
`<notify>`.
