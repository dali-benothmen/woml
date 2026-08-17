# WOML Approval HTTP v1

Status: frozen and implemented for the local Human Approval profile in A6

The normative JSON schema is `docs/schemas/approval-http.v1.schema.json`.
HTTP is the only public decision mechanism. WOML exposes no package-level
resume function.

## Endpoints

`GET /approvals/{token}` returns a read-only local approval page. It never
changes workflow state. The response uses `Cache-Control: no-store`,
`Referrer-Policy: no-referrer`, a restrictive Content Security Policy, no
external resources, and frame-denial headers.

`POST /api/v1/approvals/{token}/decision` accepts JSON containing exactly one
`decision` field with `approved` or `rejected`. Tokens never appear in query
parameters, JSON responses, errors, or access logs.

A browser request carrying `Origin` must match the loopback server origin.
Clients such as curl may omit `Origin`.

## Status behavior

| HTTP status | Contract |
|---:|---|
| 200 | The decision was accepted, or the identical human decision was already durable. |
| 400 | Content type, JSON, or decision value is invalid. |
| 404 | Token is malformed or unknown; approval existence is not disclosed. |
| 409 | A different human decision already won. |
| 410 | The credential expired, or durable timeout already resolved the request. |
| 500 | The runtime could not safely confirm a decision. |

The handler reports 200 only after Rust commits `approval_resolved`. Workflow
continuation happens after that commit. A continuation crash cannot erase or
repeat the human decision.

## Capability security

The local server binds to `127.0.0.1` only. Possession of a valid high-entropy
token authorizes one human decision for one request. The browser URL is
sensitive terminal output. Remote binding, TLS, accounts, sessions, and RBAC
require a later production-runtime contract.
