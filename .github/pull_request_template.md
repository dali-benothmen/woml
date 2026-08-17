## What changed

Describe the user-visible outcome and why it is needed.

## Architecture and contracts

- Which layer owns the change: frontend, CLI/host, engine, native adapter, or editor?
- Were versioned schemas, protocols, events, or stored data changed?
- If no contract changed, explain why the existing contract already covers it.

## Verification

- [ ] Relevant Bun tests pass
- [ ] `bun run typecheck` passes
- [ ] Relevant Rust tests pass with `-j 1`
- [ ] `bun run test:architecture-separation` passes
- [ ] Manual workflow journey tested when user-facing behavior changed

Commands and results:

```text

```

## Safety and compatibility

- [ ] Secrets remain out of events, logs, errors, and compiled models
- [ ] Recovery and idempotency behavior was considered
- [ ] Source diagnostics remain actionable
- [ ] No runtime database or generated artifact is included
- [ ] Documentation and runnable examples are updated
