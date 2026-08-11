# Operating Durable Workflow Calls

Workflow Calls v1 is publishable for one machine. A parent may call a child in
the same `woml run` process or in another local process that uses the exact same
persistent state database. Cross-machine and multi-tenant routing are not part
of this profile.

## Deployment shape

The simplest and preferred deployment activates related definitions together:

```bash
woml run workflows/ --state /var/lib/woml/state.sqlite
```

Separate processes are useful when workflows need independent lifecycle or
resource boundaries:

```bash
woml run child.woml --state /var/lib/woml/state.sqlite
woml run parent.woml --state /var/lib/woml/state.sqlite
```

Every callable workflow ID has exactly one live owner. WOML rejects duplicate
ownership instead of load-balancing ambiguously. Both processes must run on the
same machine, use the same state path, and have access to the project-local
routing key WOML creates beside that state database.

## Security boundary

- Keep the state database, its routing key, WAL files, and storage directory in
  a directory readable only by the WOML service account.
- The wake-up endpoint binds to loopback on a random port. Do not proxy or
  expose it as an application API.
- WOML generates the wake-up credential and stores only its hash in SQLite.
  Do not put a workflow-call token in `.woml` or `woml secrets`.
- Treat every local process that can write the same state database or read its
  routing key as part of one trusted runtime namespace.
- Payloads and results are bounded but may contain business data. They remain
  in authoritative run events; apply the same retention and backup policy as
  other workflow context.
- Operator progress and `woml get` expose run IDs and workflow IDs, but
  exclude payloads, results, secrets, definition hashes, call keys, and payload
  digests from the relationship summary.

Workflow Calls do not widen Fetch or managed-service network permissions. Apply
the outbound network controls described in `docs/woml-http-services.md`.

## Upgrade and recovery

Durable store v10 adds only the local runtime-route table. Opening a valid v9
store migrates it transactionally to v10 without changing definitions, call
bindings, or event histories. WOML refuses an unknown future store version and
does not rewrite it. Back up the state directory before an upgrade and test the
copy with the new binary.

SIGINT or SIGTERM stops admission, releases owned workflow routes, and leaves
committed calls and runs recoverable. A crash is handled by ownership-lease
expiry. The target also scans already admitted children, so a lost wake-up does
not create a replacement child. An ambiguous started parent attempt still
fails closed; a safe retry reattaches to the admitted child.

Local module bundles are stored under their immutable definition identity.
Recovery of a module-backed called workflow uses the durable artifact rather
than whatever source happens to be on disk at restart.

## Observability

Keep the parent and child run IDs printed by the CLI. Inspect both sides with:

```bash
woml get run_... --state /var/lib/woml/state.sqlite
```

Monitor call admission, child terminal status, rejection code, route ownership,
lease renewal, interrupted attempts, state-database errors, and graceful
shutdown. A parent timeout does not imply that its already admitted child was
cancelled.

## Performance baseline

Run the repeatable local benchmark after building the CLI:

```bash
cd woml-cli
bun run build
bun run benchmark:workflow-calls
```

The report compares sequential and concurrent calls in one runtime with calls
to another local process. Measurement begins inside the parent script, so CLI
startup and compilation are excluded. It is an observational baseline, not a
fixed latency promise or a pass/fail threshold.

## Release gate

From `woml-cli`, run:

```bash
bun run test:wc7
```

The gate covers the frontend contracts, Bun transport, Rust identity and
recovery rules, protocol/schema fixtures, adversarial corruption, v9-to-v10
migration, same-runtime and cross-process execution, clean package install,
module artifact recovery, type checking, Clippy, benchmark smoke, and secret
leak checks.

## Deployment checklist

1. Validate every selected workflow and confirm every called workflow ID is
   activated exactly once.
2. Use one explicit, persistent, access-restricted `--state` path for all local
   processes that need to call each other.
3. Keep internal routing on loopback and do not supply a user-managed call
   credential.
4. Supervise each process, send SIGTERM during deployment, and allow its route
   to be released before starting a replacement owner.
5. Back up the complete state directory consistently and test v9-to-v10
   migration and recovery on a copy.
6. Monitor both parent and child run IDs; do not infer child cancellation from
   a parent timeout.
7. Run `bun run test:wc7` and record the benchmark output for the release
   environment.
