# WOML

WOML is a markup-first language and durable runtime for workflow automation.
You describe triggers, steps, control flow, approvals, and lifecycle behavior
in a readable `.woml` file, while ordinary JavaScript handles the work inside
`<script>`.

```text
.woml source -> TypeScript compiler -> compiled DAG -> Rust engine -> Bun workers
```

The result is code-first automation without a JavaScript chaining API and
without giving up durable execution, recovery, retries, observability, or
human-in-the-loop workflows.

## Install

WOML requires Bun 1.3.14 or later:

```bash
bun add --global woml-cli
woml --version
```

## Your first workflow

Create `hello.woml`:

```xml
<woml>
  <workflow
    id="hello"
    name="Hello WOML"
    description="Build a greeting from two durable steps."
    version="1.0.0"
  >
    <triggers>
      <manual id="start" />
    </triggers>

    <steps>
      <step id="prepare" name="Prepare greeting">
        <script>
          return { name: context.payload.name ?? "World" };
        </script>
      </step>

      <step id="greet" name="Build message">
        <script>
          return { message: `Hello ${context.steps.prepare.name}` };
        </script>
      </step>
    </steps>
  </workflow>
</woml>
```

Check and activate it:

```bash
woml check hello.woml
woml run hello.woml
```

Press Enter to start a run. WOML prints each step, its duration, and its result,
then remains active for the next trigger. Press Ctrl+C to stop the automation.

## What WOML includes

- Manual, webhook, schedule, interval, event, Slack, Telegram, Discord, and
  WhatsApp triggers.
- Sequential steps, retries, parallel groups, choices, switches, and forked
  multi-step branches.
- Durable approvals with provider notifications and shared decisions.
- Workflow and step lifecycle hooks.
- Built-in HTTP, SQL database, storage, cache, event, durable-state, messaging,
  and workflow call/start services.
- Local JavaScript/TypeScript modules, reusable WOML steps, and custom
  notification providers.
- Runtime concurrency, rate-limit, queue, and timeout policies.
- Foreground and background operation, run inspection, log following, backup,
  recovery, and retention.
- A VS Code extension with HTML-style markup and embedded JavaScript syntax.

## How scripts receive data

Every script receives explicit runtime bindings:

```js
context.payload              // input from the trigger or parent workflow
context.steps.prepare        // successful output from an earlier visible step
services.http.request(...)   // supervised built-in capabilities
secrets.API_TOKEN            // only secrets proven necessary at compile time
attempt.number               // retry-attempt information
```

Scripts return JSON-compatible values. The Rust engine records successful
outcomes and derives later `context.steps` values by folding durable events;
there is no authoritative mutable context object.

## Common commands

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

See the [CLI reference](docs/cli-reference.md) for every command and option.

## Documentation

- [Getting started](docs/getting-started.md)
- [Language reference](docs/language-reference.md)
- [Examples](examples/README.md)
- [CLI reference](docs/cli-reference.md)
- [Triggers](docs/woml-production-triggers.md)
- [Services and data](docs/woml-services.md)
- [Modules and reusable definitions](docs/woml-modules.md)
- [Lifecycle and run control](docs/woml-lifecycle-and-run-control.md)
- [Production runtime and deployment](docs/woml-production-runtime.md)
- [Complete documentation map](docs/README.md)
- [VS Code extension](woml-vscode/README.md)

## Architecture

The Bun/TypeScript frontend owns markup parsing, source diagnostics, references,
modules, and compilation. The Rust engine receives a versioned compiled DAG and
owns scheduling, persistence, recovery, retries, control flow, services, and
runtime policy. Authored JavaScript runs in isolated Bun workers. Read the
[architecture guide](docs/architecture.md) for the full boundary.

## Support and security

Use [GitHub Discussions](https://github.com/dali-benothmen/woml/discussions) for
questions and [GitHub Issues](https://github.com/dali-benothmen/woml/issues) for
reproducible bugs. Please report vulnerabilities privately according to the
[security policy](SECURITY.md).

## License

WOML is available under the [Apache License 2.0](LICENSE).
