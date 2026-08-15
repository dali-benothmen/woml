# WOML

WOML is a markup-first workflow automation language and durable runtime. A
workflow's structure lives in a readable `.woml` file, JavaScript runs inside
`<script>`, and the Rust engine owns execution, retries, persistence, recovery,
triggers, workflow calls, lifecycle, and runtime policy.

## Install

Install Bun 1.3.14 or later, then install the CLI:

```bash
bun add --global woml-cli
```

Confirm the installation:

```bash
woml --version
woml --help
```

## A small workflow

```xml
<woml>
  <workflow id="hello" name="Hello WOML" version="1.0.0">
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="prepare" name="Prepare greeting">
        <script>
          return { name: context.payload.name ?? "World" };
        </script>
      </step>

      <step id="greet" name="Build greeting">
        <script>
          return { message: `Hello ${context.steps.prepare.name}` };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

Activate it with:

```bash
woml run hello.woml
```

For a manual trigger, WOML remains active and waits for Enter. Webhook,
schedule, interval, named-event, and Slack triggers remain active until the
runtime is stopped.

## Everyday commands

```bash
woml check workflows/
woml run workflows/
woml run workflows/ --background
woml inspect
woml list
woml get run_...
woml cancel run_...
woml backup backups/latest
woml prune --before 30d --dry-run
```

## Architecture

```text
.woml source
  -> Bun/TypeScript parser and compiler
  -> versioned compiled DAG
  -> dedicated WOML N-API boundary
  -> durable Rust execution engine
  -> isolated Bun workers for authored JavaScript
```

The frontend understands markup, references, modules, and source diagnostics.
The Rust core understands only the compiled workflow model and versioned event
contracts. It never parses XML or owns editor syntax.

## Documentation

- [CLI guide](woml-cli/README.md)
- [Language reference](docs/woml-v0.1.md)
- [Architecture](docs/architecture.md)
- [Production runtime](docs/woml-production-runtime.md)
- [Production deployment](docs/woml-production-deployment.md)
- [Triggers](docs/woml-production-triggers.md)
- [Services and data](docs/woml-data-guide.md)
- [Modules](docs/woml-modules.md)
- [Lifecycle and run controls](docs/woml-lifecycle-and-run-control.md)
- [Workflow calls](docs/woml-workflow-calls.md)
- [Editor extension](woml-vscode/README.md)

## Development

The active product packages are `woml`, `woml-cli`, `core/woml-engine`, and
`core/woml-native`. Install and run package commands from `woml-cli`:

```bash
cd woml-cli
bun install
bun run build
bun run typecheck
```

The project is licensed under the [Apache License 2.0](LICENSE).
