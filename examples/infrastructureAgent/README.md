# Infrastructure Agent Reference Project

This is a complete WOML project for one governed infrastructure deployment. It
combines supervised processes, durable artifacts, waits and signals, a fenced
deployment lock, compensation and cleanup, a rich approval, infrastructure
policy, streamed output, OpenTelemetry, and a named remote target.

The checked-in adapters are deterministic fakes. They do not call Terraform,
Prometheus, Slack, an LLM, Kubernetes, or a cloud API. The default project has
no secret values or worker private key, and the only remote step remains
fail-closed until an operator starts an authenticated worker. Running the
example cannot change real infrastructure.

## Read the project

- `workflows/deploy.woml` is the entire deployment in execution order.
- `policies/process.json` grants only the four checked-in adapter executables.
- `policies/artifacts.json` limits artifacts to the project-local `.woml/` store.
- `policies/infrastructure.json` denies by default and grants only the reviewed
  workflow operations and target.
- `reviewers/directory.json` contains rehearsal identities, not credentials.
- `targets/` binds `production-private` to one public rehearsal certificate.
- `operator/woml.worker.json` is intentionally not referenced by the project.
  Its private-key path is absent from source control.

## Safe local rehearsal

From the repository root:

```bash
woml check --project examples/infrastructureAgent/workflow.json
woml run --project examples/infrastructureAgent/workflow.json
```

`woml check` reads and compiles everything without binding ports, contacting the
collector, resolving secrets, or starting a worker. It reports OpenTelemetry as
degraded because check deliberately does not probe `127.0.0.1:4318`.

To inspect the rehearsal telemetry locally, start an OpenTelemetry Collector in
a separate terminal before the run:

```bash
otelcol-contrib --config examples/infrastructureAgent/collector/otel-collector.yaml
```

The collector listens only on loopback and writes received traces, metrics, and
logs to its debug exporter. It has no remote backend or credentials.

The run creates a deterministic local plan, stores and inspects it as a durable
artifact, collects fake health and risk evidence, and waits for two reviewers.
No remote or external effect happens before approval. Useful operator commands
are:

```bash
woml get <run-id> --state examples/infrastructureAgent/.woml/state.sqlite
woml logs <run-id> --state examples/infrastructureAgent/.woml/state.sqlite
woml signal rollout.succeeded \
  --run <run-id> \
  --wait rolloutSignal \
  --id rehearsal-rollout-1 \
  --correlation release-rehearsal-42 \
  --payload ./rollout-succeeded.json \
  --state examples/infrastructureAgent/.woml/state.sqlite
woml artifacts export <artifact-id> --output ./production.tfplan \
  --state examples/infrastructureAgent/.woml/state.sqlite \
  --artifact-policy examples/infrastructureAgent/policies/artifacts.json
```

## Remote execution

The reviewed apply step explicitly selects `target="production-private"`; WOML
must never fall back to local execution. To exercise it, create the worker
private key referenced by `operator/woml.worker.json`, replace the rehearsal
certificate and coordinator URL with operator-owned values, then run:

```bash
woml worker check --config examples/infrastructureAgent/operator/woml.worker.json
woml worker start --config examples/infrastructureAgent/operator/woml.worker.json
```

Do not add a private key, access token, provider credential, or real production
executable to this example. Copy the project and replace each fake adapter only
after reviewing all three policy files and the target trust boundary.
