# WOML Local Module Recovery v1

Status: frozen and implemented for Module System MS4 on 2026-08-10.

This contract makes a run depend on its immutable compiled definition package,
not on the files currently present in a developer checkout.

## Persistence authority

Durable Store v8 owns one immutable artifact row per definition hash and module
alias. Each row contains the alias, sorted named exports, bundle digest, exact
bundle bytes, source-map digest, and exact canonical source-map bytes. Updating
or deleting a stored artifact is forbidden. A conflicting repeat registration
fails before a run starts.

The following limits are part of the first runtime profile:

- bundle: 3 MiB per module;
- source map: 3 MiB per module;
- complete artifact set: 32 MiB per definition;
- Script Host cache: 64 artifacts and 32 MiB;
- protocol frame: the existing 8 MiB UTF-8 byte ceiling.

## Recovery algorithm

For both crash recovery and explicit `--resume`, Rust:

1. resolves the run to its stored definition hash;
2. loads the stored Model v9 definition and artifacts;
3. verifies names, order, exports, sizes, and both SHA-256 identities;
4. starts or reconnects the long-lived Bun host;
5. re-registers every required artifact through Script Host v6; and
6. resumes the next durable node from the folded event history.

No recovery step reads the WOML document, module entrypoint, dependency source,
or caller-provided replacement artifact. This remains true if those files were
edited, moved, or deleted after the run began.

## Diagnostics and confidentiality

Progress output exposes module aliases only. It never prints source paths,
artifact bytes, digests, source content, secret values, or full Worker stacks.
Script failures may include one sanitized project-relative source location.
Resolved secrets are rejected if found in artifacts and redacted from failure
messages before they cross the Worker boundary.

## Composition

The same stored artifact path is used by manual, webhook, schedule, interval,
event, and Slack triggers. Imported calls execute inside ordinary sequential,
branch, parallel, approval, and retry nodes and retain the same HTTP, database,
storage, cache, and event capability authority as inline scripts.

## Failure policy

Missing, corrupted, mismatched, or oversized artifacts fail closed before the
affected step runs. A host restart permits re-registration; it does not permit
substituting different code. Existing interrupted-effect and retry policies
remain authoritative.
