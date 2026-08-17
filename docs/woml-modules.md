# Authoring Local WOML Modules

WOML modules let you write reusable JavaScript or TypeScript once and call it
from workflow steps through `services.<name>`. Local modules require no npm
package support and run inside the existing isolated Bun Worker.

Use a JS/TS module for reusable functions. Use a reusable `.woml` definition
when the reusable unit should appear as a custom step or notification-provider
tag with declared props, durable attempts, results, and lifecycle. See
[Reusable WOML Steps and Notification Providers](woml-reusable-definitions.md).

## Declare and call a module

```xml
<woml>
  <imports>
    <module name="openai" from="./modules/openai.ts" />
  </imports>

  <workflow id="support-agent" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="answer"><script>
        return await services.openai.chat(context.payload.message);
      </script></step>
    </steps>
  </workflow>
</woml>
```

The module uses ordinary named exports:

```ts
export async function chat(
  message: string,
  http = services.http
) {
  const response = await http.request<{ reply: string }>({
    url: 'https://api.example.com/chat',
    method: 'POST',
    json: { message }
  });

  return response.data.reply;
}
```

Default exports, CommonJS, dynamic imports, npm package imports, and module
installation effects remain unsupported. Static relative `.js` and `.ts`
imports are supported.

## Editor autocomplete is automatic

The normal workflow is one command:

```bash
woml run path/to/workflow.woml
```

`woml run` refreshes `woml-env.d.ts` before activating the workflow. `woml check`
also refreshes it, so checking a workflow is enough to prepare editor
autocomplete before the first run. Type generation is editor support only and
never controls whether JavaScript executes. This applies to both `.js` and `.ts`
modules, but JavaScript authors do not need to think about or invoke it.

`woml types` remains an optional advanced command for refreshing declarations
without checking or running a workflow, or for choosing another output path:

```bash
woml types path/to/workflow.woml --output generated/woml-env.d.ts
```

The generated file contains the current built-in service contracts and every
imported alias/function found in that scope. It is self-contained: the project
does not need a runtime import merely to satisfy the editor. If WOML cannot
write the declaration file, it prints a warning but still runs the workflow.

Most TypeScript projects discover the file automatically. If a restrictive
`include` list hides it, add it explicitly:

```json
{
  "include": ["src/**/*.ts", "workflows/woml-env.d.ts"]
}
```

Local modules receive the `services` global only. They do not automatically
receive `context`, `attempt`, or `secrets`. Pass only the values a function
needs from its calling WOML step:

```xml
<script>
  return services.crm.findCustomer(
    context.payload.customerId,
    secrets.CRM_TOKEN,
    attempt.idempotencyKey
  );
</script>
```

That rule keeps module dependencies explicit and prevents secret enumeration.

## Check aliases and imports

```bash
woml check path/to/workflow.woml
```

`woml check` reports declared modules that are not used. A misspelled or
undeclared service alias in WOML, such as `services.spredsheet`, fails with a
source-located `WOML_MODULE_SERVICE_UNKNOWN` error before execution.

## Unit-test a module

The public `woml` package is a global CLI, not a JavaScript testing library.
Keep pure transformations directly testable and accept a capability as an
optional function argument when a module needs an isolated mock. The production
default can still use WOML's injected `services` binding:

```ts
import { expect, test } from 'bun:test';
import { chat } from './modules/openai.ts';

test('builds a response', async () => {
  const http = {
    request: async () => ({
      status: 200,
      ok: true,
      headers: {},
      data: { reply: 'Hello World' },
      url: 'https://api.example.com/chat',
      redirected: false
    })
  };

  expect(await chat('Hello', http)).toBe('Hello World');
});
```

Use `woml test path/to/workflow.woml` for an integration test through the real
compiler, Rust engine, module bundle, and isolated Bun Worker. Production always
uses the Rust-supervised WOML runtime.

## Runtime and recovery

`woml run` stores exact bundles and source maps under the immutable workflow
definition. A durable run resumes from those stored artifacts rather than the
current source tree. See [Local Module Recovery v1](protocols/module-recovery-v1.md).
