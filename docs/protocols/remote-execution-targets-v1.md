# Remote Execution Targets v1

Status: frozen by RET0 and implemented through RET8. Cross-platform release,
packaging, fault-injection, and public tutorial hardening remain in RET9.

## Purpose

Remote Execution Targets place one compiled WOML step attempt on an
authenticated WOML worker. The coordinator remains the only workflow
authority. A worker cannot admit a run, choose a branch, schedule a retry,
resolve a wait, release a lock, or settle a run.

```xml
<step id="migrateDatabase" target="production-private" timeout="20m">
  <script>
    return await services.process.run({
      executable: "bun",
      args: ["run", "database:migrate"]
    });
  </script>
</step>
```

## Frozen authoring contract

- `target` is optional on `<step>`.
- Omission retains the existing local model and byte shape.
- An authored target promotes the workflow to Model v21.
- The value is a literal lowercase kebab-case identifier of 1–128 bytes.
- Templates, secret references, whitespace, underscores, dots, paths, URLs,
  and shell fragments are rejected at the attribute value span.
- `local` is the reserved built-in target and cannot be redefined by an
  operator registry.
- A target applies to the complete `runtime.script` attempt, not only to
  `services.process` calls inside it.
- A compensation action inherits its forward step's execution placement.
- A cleanup step declares its own target or remains local.
- Control-flow nodes, triggers, waits, approvals, locks, and workflow
  settlement remain coordinator-owned.

## Compiled contracts

Model v21 adds one optional field to executable nodes:

```json
{
  "execution": {
    "profileVersion": 1,
    "target": "production-private",
    "requiredCapabilities": ["runtime.script", "services.process"]
  }
}
```

Execution Placement v1 is legal only on `runtime.script` nodes in Model v21.
Model v21 must contain at least one placement. Older models containing the
field fail structural validation. Definition Package v16 binds the exact
Model v21 bytes, module artifacts, source graph, placement, and sorted remote
capability requirements. Imported workflow modules are package artifacts, not
worker capabilities; reviewed built-in services use `services.<name>` IDs.

Local execution profiles deliberately reject Model v21. Remote placement is
admitted only by the coordinator-owned dispatch path; every ordinary,
parallel, fork, `for-each`, compensation, and cleanup script path also checks
placement before opening a local script host. This prevents a targeted step
from ever falling through to local execution while RET9 finishes packaged
coordinator activation.

## Authority

| Decision | Authority |
| --- | --- |
| Compile source and create package identity | Coordinator |
| Admit and schedule runs | Coordinator |
| Create run, node, iteration, attempt, and invocation identity | Coordinator |
| Select target and effective policy | Coordinator |
| Accept and journal one dispatch | Worker |
| Enforce host containment and worker-local policy | Worker |
| Resolve symbolic secrets | Worker-local provider |
| Commit run events and choose retries | Coordinator |
| Settle workflow outcome | Coordinator |

Effective permission is the intersection of the compiled requirement,
coordinator target policy, infrastructure policy, advertised worker
capability, and worker-local policy. No participant may widen another.

## Target Registry v1

The target registry is operator configuration outside WOML source. Its schema
is `target-registry.v1.schema.json` and its profile is
`woml.target-registry/v1`.

- At most 256 targets are accepted.
- Target IDs are unique and sorted.
- `local` is forbidden in the registry.
- Each v1 target is a `worker-pool`.
- Worker identities and required capabilities are explicit, unique, sorted,
  and bounded.
- `maximumInFlight` is 1–64.
- Registry files contain no certificate private keys, bearer tokens, or
  secret values.

The canonical JSON SHA-256 digest binds a dispatch to the reviewed target
configuration.

## Target Trust v1

Worker certificate references live in a separate operator-owned document with
profile `woml.target-trust/v1`. Each entry binds one allowed worker identity to
a relative PEM certificate path. Registry and trust files must be regular,
non-symlink files and must not be group- or world-writable on Unix. Certificate
references must stay within the trust file directory and resolve to bounded,
regular, non-symlink PEM files.

WOML parses each trusted PEM certificate and hashes its canonical DER bytes into
the trust digest and authenticated leaf-certificate identity, but never copies PEM bytes
into inspection output, runtime configuration, compiled models, or workflow
state. Private keys are not accepted by the target registry or target trust
contracts; they remain worker-local configuration.

Before runtime activation, RET2 joins every Model v21 placement with its
registry entry, trusted worker identities, declared capabilities, and the
active infrastructure policy. Remote dispatch is modeled as an
`unsafe-write` operation named `dispatch` for each required capability. An
unknown target, missing capability, untrusted allowlist, absent policy, or
policy denial stops activation before admission opens.

RET2 validates each `minimumWorkerVersion` constraint syntactically. RET3 will
apply that constraint to the version advertised by an authenticated worker;
configuration alone is never treated as proof of a compatible live worker.

Operators can inspect this boundary without starting a runtime:

```bash
woml target list --registry .woml/targets.json --trust .woml/target-trust.json
woml target check production-private --registry .woml/targets.json --trust .woml/target-trust.json
woml target inspect production-private --registry .woml/targets.json --trust .woml/target-trust.json --json
```

Configuration-only CLI inspection reports target health as `not_observed`;
WOML does not infer reachability from files. The authenticated coordinator
authority reports eligible, saturated, draining, offline, incompatible, and
unauthorized worker states from live session facts. A local-only workflow does
not load a target registry, resolve certificates, or construct any remote
listener or scheduler.

## Worker Configuration v1

Worker configuration uses profile `woml.worker/v1`. It names one worker, one
HTTPS coordinator, 1–32 target memberships, capacity, symbolic TLS file
references, a process policy, and a target-local secret provider.

Production sessions require mutual TLS. URI user info, query strings,
fragments, HTTP endpoints, inline PEM, and inline private keys are rejected.
The authenticated certificate identity—not the JSON `workerId` alone—is the
worker identity.

## Worker Protocol v1

Workers initiate outbound HTTPS connections and long-poll for work. The
coordinator does not connect to an inbound worker port. Every message is
canonical, bounded JSON using profile `woml.worker-protocol/v1` and one of:

- `hello` — factual compatibility and capacity declaration;
- `poll` — request a bounded offer;
- `offer` — one Dispatch Package v1;
- `accepted` — durable worker-journal acceptance;
- `started` — factual attempt start;
- `heartbeat` — bounded liveness and progress observation;
- `output` — ordered redacted output records;
- `cancel` — coordinator cancellation request;
- `terminal` — one durable success, failure, or cancellation fact.

Message IDs, session IDs, dispatch IDs, worker IDs, target IDs, sequence
numbers, timestamps, and expiries are bounded. Unknown fields fail closed.
Protocol downgrade and future versions are rejected explicitly.

## Dispatch Package v1

A dispatch contains exactly one attempt:

- deterministic dispatch ID;
- run, node, optional iteration, attempt, and invocation identity;
- target-registry and infrastructure-policy digests;
- exact Model v21 node;
- Definition Package v16 root hash;
- script and module artifact digests;
- bounded context snapshot;
- required service capabilities and symbolic secret names;
- scoped artifact grants;
- coordinator deadline and package digest.

The canonical JSON envelope is limited to 1 MiB. Larger values use durable
artifacts. Secret values, private keys, mutable workflow state, and complete
run history are forbidden.

The dispatch ID is SHA-256 over a domain-separated canonical tuple containing
run ID, node ID, iteration identity, attempt number, invocation ID, and target
registry digest.

## Delivery and ambiguity

WOML promises at-least-once offer delivery and exactly-once acceptance for a
dispatch identity. It does not promise exactly-once external effects.

```text
requested -> offered -> accepted -> started -> terminal
                                      |
                                      +----> ambiguous
```

- An offer lost before durable acceptance may be offered again.
- An accepted dispatch may be released only with durable proof it never
  started.
- A known terminal result is committed once.
- Loss after start without a terminal record is ambiguous.
- Ambiguous work never enters ordinary retry automatically.
- A trusted late terminal journal record may reconcile ambiguity.
- An operator may record failure or explicitly authorize a new attempt with
  possible-duplicate risk. That action is durable and audited.
- Operators cannot forge success.

Coordinator time owns durable ordering. Worker timestamps are diagnostic and
cannot decide expiry or settlement.

## Worker Journal v1

Every worker keeps a local append-only journal before acknowledging acceptance,
start, cancellation, or a terminal result. Records are bound to the worker
identity, session, dispatch digest, attempt identity, monotonic local sequence,
and previous-record digest. The journal contains no secret values, artifact
bodies, arbitrary environment values, or unredacted output.

Reconnecting with the same dispatch replays the recorded factual state instead
of starting a second effect. A changed package for an existing dispatch ID is a
conflict. Corrupt, incomplete, future-version, or identity-mismatched journal
records fail closed and cannot prove that an attempt is safe to retry.

## Secrets and artifacts

Dispatches carry secret names only. A worker resolves those names from its
configured provider after local policy approval. Values are excluded from the
worker journal, protocol diagnostics, durable events, output, and telemetry.

Artifact grants are bound to one dispatch, worker, operation, and expiry.
Transfers verify length and SHA-256 before visibility. A worker cannot list or
read artifacts outside its grants. Partial and corrupt uploads never publish.

## Output, progress, and heartbeat

Output batches contain at most 128 records and 1 MiB before the stricter
Operational Output Store v1 limits apply. Records use a monotonically
increasing remote sequence and are deduplicated by dispatch and sequence.
Redaction occurs before output leaves the worker.

Heartbeats default to five seconds during active work. Thirty seconds without
a valid heartbeat makes the worker loss-suspected. Heartbeats update bounded
operational state; they do not append one workflow event per beat.

## Retention and reconciliation

Coordinator dispatch records remain protected while their run is active,
waiting, correcting, cancelling, or ambiguous. Terminal records follow the
run-retention policy. Worker journal entries remain protected until the
coordinator has durably acknowledged the same terminal fact and the configured
minimum reconciliation window has elapsed.

Reconciliation is keyed by dispatch identity and journal digest. It may attach
a previously missing trusted terminal fact, record an operator-authorized
failure, or authorize a possible-duplicate retry. It cannot edit attempt
history, replace a known terminal fact, forge success, or bypass policy. Every
operator action records principal, timestamp, reason, prior state, and result.

## Frozen bounds

| Resource | Limit |
| --- | ---: |
| Targets per registry | 256 |
| Identifier length | 128 bytes |
| Worker target memberships | 32 |
| Worker capabilities | 128 |
| Worker capacity | 64 |
| Dispatch JSON envelope | 1 MiB |
| Inline terminal result | 1 MiB |
| Output records per batch | 256 |
| Output bytes per batch | 1 MiB |
| Long poll | 25 seconds |
| Default heartbeat | 5 seconds |
| Default loss suspicion | 30 seconds |

Implementations may choose lower configured limits but cannot exceed these
contract maxima without a new version.

## Failure catalog

| Code | Meaning | Class |
| --- | --- | --- |
| `WOML_TARGET_INVALID` | Author target is not a valid literal ID | author error |
| `WOML_TARGET_UNKNOWN` | Registry has no selected target | preflight error |
| `WOML_TARGET_UNAVAILABLE` | No eligible capacity is reachable | retryable availability |
| `WOML_TARGET_UNAUTHORIZED` | Identity is not allowed for target | security failure |
| `WOML_TARGET_INCOMPATIBLE` | Version or platform is unsupported | preflight error |
| `WOML_TARGET_CAPABILITY_UNAVAILABLE` | Required capability is absent | preflight error |
| `WOML_REMOTE_PROTOCOL_INVALID` | Message violates Worker Protocol v1 | security failure |
| `WOML_REMOTE_DISPATCH_CONFLICT` | Same identity carries changed content | security failure |
| `WOML_REMOTE_DISPATCH_AMBIGUOUS` | Started effect has no proven outcome | operator action |
| `WOML_REMOTE_RESULT_INVALID` | Terminal record is invalid or conflicting | security failure |
| `WOML_REMOTE_OUTPUT_REJECTED` | Output sequence or bounds are invalid | operation failure |
| `WOML_REMOTE_CANCEL_UNCONFIRMED` | Cancellation outcome cannot be proven | ambiguous |
| `WOML_REMOTE_ARTIFACT_INTEGRITY_FAILED` | Artifact length or digest differs | security failure |

Errors expose stable codes and corrective action, never certificates, private
keys, secret values, raw authorization headers, remote response bodies, or
arbitrary worker output.

## Threat model

V1 explicitly defends against:

- worker impersonation with mutual TLS and registry-bound identities;
- stolen, revoked, expired, and not-yet-valid certificates with transport
  validation, short sessions, and registry reauthorization on reconnect;
- message replay with sessions, expiries, deterministic dispatch identity,
  and durable deduplication;
- target-membership forgery with authenticated identity allowlists;
- script, module, package, and artifact substitution with canonical digests;
- policy widening with intersected coordinator and worker authority;
- secret exfiltration through symbolic resolution and incremental redaction;
- queue and allocation exhaustion with fixed message, capacity, page, and
  connection bounds;
- silent duplicate side effects with the ambiguity state;
- stale cancellation and terminal races with monotonic durable transitions;
- network partitions and split-brain observations by keeping the coordinator
  authoritative and routing uncertain started work to explicit ambiguity;
- compromised-worker blast radius with scoped targets, capabilities, secrets,
  and artifact grants.

A compromised worker can still misuse the local capabilities explicitly
granted to it. Remote execution reduces and audits that authority; it does not
turn an untrusted host into a trusted one.

A compromised coordinator can schedule any work permitted by its operator
configuration and reveal the symbolic inputs granted to that work. Worker-local
policy, local secret providers, scoped artifact grants, and process containment
remain independent enforcement boundaries; they must reject requests that
exceed their reviewed authority.

## Non-goals and deferred adapters

Docker, Kubernetes Job, cloud execution, SSH, and multi-coordinator scheduling
are not v1 backends. Future adapters must preserve Worker Protocol v1 attempt
identity, policy intersection, artifact integrity, output ordering, durable
acceptance, cancellation, and ambiguity semantics. SSH remains deferred until
it can meet the same containment and recovery contract.
