# WOML Approval Notification Contracts v1

Status: frozen in N0 and implemented and publication-hardened through N6.1. The
real Slack Socket Mode acceptance journey and automated N6 release gate pass.
N6.1 adds only the separately versioned Rust-to-CLI diagnostic envelope; this
provider protocol is unchanged.

This document pins the interfaces between WOML source, the TypeScript frontend,
the Rust authority, the Bun provider host, the secret resolver, and Slack.
N2 implements the source-to-Model-v5 portion; N3 implements the durable Rust
delivery, resolution, update, retry, and recovery authority; N4 implements the
versioned Bun provider host and deterministic Slack conformance adapter; N5
implements the real Slack Web API and Socket Mode transport.
N6 closes composition, multi-credential convergence, recovery, redaction,
packaging, and clean-install verification without changing this v1 contract.

## Source and identity

The executable Slack-first source profile is one optional `<notify>` immediately
before an approval's decision arms. It contains one or more `<slack>` tags.

`channels` is a whitespace-separated ordered list. Each token is either:

- a lowercase alias matching `#[a-z0-9][a-z0-9_-]{0,79}`; or
- a Slack conversation ID matching `[CG][A-Z0-9]{8,31}`.

Aliases are already canonical and are not case-folded. Conversation IDs retain
their exact spelling. Duplicate canonical destinations within one Slack tag are
invalid. Tag order and channel order are semantic for deterministic identity.

The zero-based delivery identity is:

```text
<approvalId>:notify:<slackTagIndex>:channel:<channelIndex>
```

Every delivery belongs to the same approval request. It receives a different
opaque decision capability, but the first valid decision from any delivery wins
the one Rust transaction.

## Secret contract

A secret-bearing attribute contains exactly `{{secrets.NAME}}`. Names match
`[A-Z][A-Z0-9_]*`. No interpolation, literal credential, context reference,
service reference, or script-global secret access is accepted.

The compiled model stores only `{ "kind": "secretReference", "name": "NAME" }`.
Secret values are forbidden from models, definition hashes, events, context,
SQLite, provider results, diagnostics, logs, snapshots, and terminal output.

The local provider uses Bun's OS-native credential API under service
`dev.woml.cli.secrets.v1`. The protected keychain also stores the name/updatedAt
metadata index needed by `woml secrets list`; WOML creates no plaintext secret
or metadata file. Values are non-empty and at most 2048 UTF-8 bytes.

CI selects `WOML_SECRETS_PROVIDER=env` and supplies
`WOML_SECRET_<NAME>`. The environment provider is intentionally read-only.
There is no automatic OS-to-environment fallback and no plaintext-file backend.

## Compiled model v5

Model v5 extends the approval wait node with an ordered `notifications` array.
Each expanded channel becomes one provider-neutral delivery definition with a
stable delivery ID, provider, destination, and named credential bindings. Rust
may schedule and pass these symbolic bindings but never resolves their values.

## Durable delivery policy

Rust persists `approval_requested` and every delivery intent before provider
execution. One or more successful deliveries leave the workflow durably waiting.
Individual failures remain visible and retryable without invalidating successful
messages. If every delivery exhausts its safe attempts, the run fails explicitly
with `WOML_NOTIFICATION_DELIVERY_FAILED`; it never waits invisibly.

The stable delivery idempotency key is:

```text
sha256("woml.notification.delivery.v1\0" + runId + "\0" + requestId + "\0" + deliveryId)
```

The key is unchanged across attempts. A provider adapter may derive a
provider-specific deduplication value from its digest. Explicit retryable
provider failures use at most three total attempts with delays of 0, 1, and 5
seconds. An ambiguous transport failure is `delivery_ambiguous`; WOML fails
closed and does not blindly resend until reconciliation exists.

After resolution or timeout, Rust records durable update work for every
successful message. The stable update idempotency key is:

```text
sha256("woml.notification.update.v1\0" + runId + "\0" + requestId + "\0" + deliveryId + "\0" + updateId)
```

It remains unchanged across update attempts. Update failures retry but never
reverse or reopen the decision.

## Provider host v1

`notification-provider-host.v1.schema.json` is asynchronous and multiplexed.
Several delivery/update invocations may be in flight, completed responses may
arrive out of order, and `invocationId` is the only request correlation key.

Messages use the existing LSP-style `Content-Length` framing. Length is the
number of UTF-8 bytes, not JavaScript characters; literal CRLF and multibyte
text inside JSON strings do not terminate a frame. Provider-host v1 limits one
decoded frame to 1 MiB and reports `size_limit_exceeded` without echoing the
frame. The reviewed fixture includes multibyte text and a literal CRLF value.

The Bun host resolves credential bindings only inside an adapter invocation.
It returns a provider message identity or a structured secret-safe failure.
Socket Mode interactions are acknowledged at the transport edge, then forwarded
as a decision message containing the opaque delivery capability. Rust validates
that capability and atomically records the winning decision. Only the winning
interaction adds `notification_decision_accepted`; `approval_resolved` remains
the sole workflow decision-output authority.

The stored Slack message identity is workspace ID, channel ID, and message
timestamp ID. Reviewer audit stores only the Slack user ID, provider, delivery
ID, decision, and time—never display name, email, or token.

## Socket Mode ownership

One supervised connection is shared per distinct app-token secret reference.
Socket Mode is pre-authenticated, so no HTTP signature surface is introduced.
Every envelope is acknowledged promptly, duplicate envelope/action delivery is
expected, and Rust's capability plus first-decision-wins transaction supplies
the authoritative deduplication. Reconnect never changes delivery identities.

## Compatibility

Model/event versions 1–4 and Script Host v1/v2 remain unchanged. Notification
runs use model v5 and event v5. The provider-host protocol is separate from the
script-host protocol. No Slack API concept is added to the Rust handler registry.
