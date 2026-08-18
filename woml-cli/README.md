# WOML

WOML is a markup-first language and durable runtime for workflow automation.
Define triggers, steps, control flow, approvals, lifecycle behavior, and runtime
policy in readable `.woml` files; write ordinary JavaScript inside `<script>`.

## Install

WOML requires Bun 1.3.14 or later:

```bash
bun add --global woml-cli
woml --version
```

The one `woml` package contains the CLI and compiler and selects the native Rust
engine for the current supported platform. Users do not install a compiler or
native package separately.

## Quick start

Save this as `hello.woml`:

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

Then run:

```bash
woml check hello.woml
woml run hello.woml
```

Press Enter to start a run. The automation remains active for the next trigger;
press Ctrl+C to stop it. `woml test hello.woml` is the one-shot form for tests
and CI.

## Main commands

```bash
woml --help
woml check <workflow-or-directory>...
woml run <workflow-or-directory>...
woml run workflows/ --background
woml inspect
woml list
woml get <run-id>
woml cancel <run-id>
woml <run-id-or-workflow-id> --logs
woml secrets set <NAME>
woml backup <directory>
woml prune --before 30d --dry-run
```

`woml run` activates multiple files and directories atomically. This makes
call-only workflows available to `services.workflows.call()` and
`services.workflows.start()` in the same runtime. Webhook startup prints its URL
and a generated `curl`; manual triggers print the keyboard instruction.

## Runtime bindings

Inside scripts:

- `context.payload` is input from the trigger or parent workflow;
- `context.steps.<id>` is a visible successful step/control-flow result;
- `services` exposes supervised capabilities and imported modules;
- `secrets.NAME` resolves only declared secrets; and
- `attempt` describes the current durable retry attempt.

Return JSON-compatible data for downstream steps. WOML records durable events
before advancing the compiled graph and fails closed rather than replaying an
ambiguous external effect.

## Capabilities

WOML v1 includes manual and production triggers, retries, choices, switches,
parallel groups, forked branches, approvals, lifecycle hooks, runtime policies,
HTTP, SQL databases, storage, cache, events, durable state, local modules,
reusable steps/providers, communication adapters, workflow call/start, run
inspection, backup, recovery, and retention.

## Documentation

- [Getting started](https://github.com/dali-benothmen/woml/blob/master/docs/getting-started.md)
- [Language reference](https://github.com/dali-benothmen/woml/blob/master/docs/language-reference.md)
- [CLI reference](https://github.com/dali-benothmen/woml/blob/master/docs/cli-reference.md)
- [Examples](https://github.com/dali-benothmen/woml/tree/master/examples)
- [Production deployment](https://github.com/dali-benothmen/woml/blob/master/docs/woml-production-deployment.md)
- [VS Code extension](https://github.com/dali-benothmen/woml/tree/master/woml-vscode)

For support, use
[GitHub Discussions](https://github.com/dali-benothmen/woml/discussions) or
open a reproducible [issue](https://github.com/dali-benothmen/woml/issues).
Report vulnerabilities privately according to the
[security policy](https://github.com/dali-benothmen/woml/blob/master/SECURITY.md).

WOML is licensed under the
[Apache License 2.0](https://github.com/dali-benothmen/woml/blob/master/LICENSE).
