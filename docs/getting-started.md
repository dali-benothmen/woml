# Getting Started with WOML

This guide takes you from installation to a running automation in about five
minutes. You do not need a project scaffold or a WOML dependency inside your
application.

## 1. Install WOML

Install [Bun](https://bun.sh/) 1.3.14 or later, then install the one public WOML
package globally:

```bash
bun add --global woml-cli
woml --version
```

The second command should print `woml 1.0.0`. The package chooses the native
Rust engine for your operating system automatically.

## 2. Create a workflow

Create an empty directory, enter it, and save this as `hello.woml`:

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

The first step returns JSON. WOML records it as
`context.steps.prepare`, so the second step can use it. `context.payload` is
the input supplied by the trigger; a keyboard-triggered run starts with `{}`.

## 3. Check it before running

```bash
woml check hello.woml
```

`woml check` parses, validates, and compiles the workflow without opening a
listener, connecting a provider, or executing JavaScript. Errors include a
stable code and source line/column.

## 4. Activate the automation

```bash
woml run hello.woml
```

WOML prints the workflow name, description, version, and manual-trigger
instruction. Press Enter to create a run. A successful run ends with:

```text
Completed · 2 succeeded
→ { message: "Hello World" }
```

The process stays active because this is automation: press Enter again for a
new run, or Ctrl+C to stop. Use `woml test hello.woml` only when a test or CI
job intentionally needs one manual execution that exits.

## 5. Understand the files WOML creates

By default WOML keeps local durable state under `.woml/`, including the SQLite
event store and runtime logs. Keep this directory out of source control. Your
workflow source remains the `.woml` file you wrote.

Normal `woml check` and `woml run` commands may also refresh `woml-env.d.ts`
beside workflows that use modules. That file provides editor types and never
controls runtime behavior.

## 6. Add a real trigger

Replace the manual trigger with a webhook:

```xml
<triggers>
  <webhook id="newOrder" path="/webhooks/orders" method="POST" auth="none">
    <schema>
      {
        "type": "object",
        "required": ["orderId"],
        "properties": { "orderId": { "type": "string" } }
      }
    </schema>
  </webhook>
</triggers>
```

Run the file again. WOML prints the exact URL and a copy-pasteable `curl`
command generated from the schema. The request body becomes `context.payload`.
Use authentication before exposing a webhook outside a trusted local network.

## Where to go next

- Follow the curated [examples](../examples/README.md).
- Read the complete [language reference](language-reference.md).
- Learn every command in the [CLI reference](cli-reference.md).
- Configure [secrets](woml-data-security.md) and
  [production deployment](woml-production-deployment.md).
- Install the [VS Code extension](../woml-vscode/README.md) for WOML syntax and
  snippets.

If a command fails, keep the error code and source location when opening an
issue. The [support guide](../SUPPORT.md) explains what information is safe and
useful to include.
