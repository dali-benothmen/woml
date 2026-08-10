# Authoring Local WOML Modules

WOML modules let you write reusable JavaScript or TypeScript once and call it
from workflow steps through `services.<name>`. Local modules require no npm
package support and run inside the existing isolated Bun Worker.

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
        return await services.openai.chat(context.trigger.message);
      </script></step>
    </steps>
  </workflow>
</woml>
```

The module uses ordinary named exports:

```ts
export async function chat(message: string) {
  const response = await services.http.request<{ reply: string }>({
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

The generated file contains the five built-in service contracts and every
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
    context.trigger.customerId,
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

Use ordinary Bun tests and install read-only service mocks for one test:

```ts
import { expect, test } from 'bun:test';
import { withWomlModuleTestRuntime } from 'woml';

test('builds a response', async () => {
  await withWomlModuleTestRuntime(
    {
      services: {
        http: {
          request: async () => ({
            status: 200,
            ok: true,
            headers: {},
            data: { reply: 'Hello Dali' },
            url: 'https://api.example.com/chat',
            redirected: false
          })
        }
      }
    },
    async () => {
      const { chat } = await import('./modules/openai.ts');
      expect(await chat('Hello')).toBe('Hello Dali');
    }
  );
});
```

Test runtimes cannot overlap in one process, and the injected service object is
read-only. This helper is for unit tests only; production always uses the
Rust-supervised WOML runtime.

## Runtime and recovery

`woml run` stores exact bundles and source maps under the immutable workflow
definition. A durable run resumes from those stored artifacts rather than the
current source tree. See [Local Module Recovery v1](protocols/module-recovery-v1.md).
