import { afterAll, describe, expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import type { ManualLineInput } from '../src/manual-input';
import { inspectRunWithRust, listRunsWithRust } from '../src/rust-executor';

const workflowPath = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/modules/customer-import.woml'
);
const projectRoot = resolve(import.meta.dir, '../..');
const nativeCorePath = resolve(
  projectRoot,
  'woml-cli/dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;
const verticalWorkflowPath = resolve(
  projectRoot,
  'examples/moduleWorkflow.woml'
);
const generatedTypePaths = [
  resolve(workflowPath, '..', 'woml-env.d.ts'),
  resolve(verticalWorkflowPath, '..', 'woml-env.d.ts'),
];

afterAll(async () => {
  await Promise.all(generatedTypePaths.map(path => rm(path, { force: true })));
});

function nativeDependencies() {
  return {
    nativeCorePath,
    createSecretStore: () => ({
      provider: 'environment' as const,
      get: async () => undefined,
      has: async () => false,
      list: async () => [],
      set: async () => {},
      delete: async () => false,
    }),
    readSecret: async () => {
      throw new Error('fixtures have no secrets.');
    },
  };
}

class OneRunManualInput implements ManualLineInput {
  readonly isTTY = true;
  #closed = false;

  constructor(private readonly statePath: string) {}

  async run(onLine: (line: string) => void | Promise<void>): Promise<void> {
    await onLine('');
    while (!this.#closed) {
      const run = listRunsWithRust(
        this.statePath,
        { limit: 1 },
        { nativeCorePath }
      ).runs[0];
      if (
        run?.status === 'succeeded' ||
        run?.status === 'failed' ||
        run?.status === 'cancelled'
      ) {
        return;
      }
      await Bun.sleep(10);
    }
  }

  close(): void {
    this.#closed = true;
  }
}

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: text => {
      stdout += text;
    },
    stderr: text => {
      stderr += text;
    },
  };
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('Module runtime CLI', () => {
  test('checks, compiles, and explains a deterministic local module graph', async () => {
    const result = await invoke(['check', workflowPath]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('services.spreadsheet');
    expect(result.stdout).toContain('(read, removeEmptyRows)');
    expect(result.stdout).toContain('ready for woml run');
  });

  test('prints the reviewed executable package as JSON without activating code', async () => {
    const result = await invoke(['check', workflowPath, '--json']);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    const manifest = JSON.parse(result.stdout);
    expect(manifest).toMatchObject({
      schemaVersion: 3,
      profile: 'woml.definition-package/v3',
      executable: true,
      runtimeReady: true,
      workflow: { id: 'customer-import' },
      modules: [
        {
          name: 'spreadsheet',
          exports: ['read', 'removeEmptyRows'],
        },
      ],
    });
  });

  nativeTest('executes the compiled module through Rust and Bun', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms3-cli-'));
    let stdout = '';
    let stderr = '';
    const exitCode = await runCli(
      ['test', workflowPath, '--state', resolve(directory, 'state.sqlite')],
      {
        stdout: text => {
          stdout += text;
        },
        stderr: text => {
          stderr += text;
        },
      },
      nativeDependencies()
    );
    expect(exitCode).toBe(0);
    expect(stderr).toBe('WOML modules ready: services.spreadsheet.\n');
    expect(JSON.parse(stdout)).toEqual({ rows: [] });
    await rm(directory, { recursive: true, force: true });
  });

  nativeTest(
    'runs two sequential module steps with one reference and fresh state',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms3-vertical-'));
      let stdout = '';
      let stderr = '';
      const exitCode = await runCli(
        [
          'test',
          verticalWorkflowPath,
          '--state',
          resolve(directory, 'state.sqlite'),
        ],
        {
          stdout: text => {
            stdout += text;
          },
          stderr: text => {
            stderr += text;
          },
        },
        { ...nativeDependencies(), waitForShutdown: async () => {} }
      );
      expect(exitCode).toBe(0);
      expect(stderr).toContain('WOML modules ready: services.spreadsheet.');
      expect(JSON.parse(stdout)).toEqual({
        rows: [
          ['Alice', 'active'],
          ['Bob', 'active'],
        ],
        invocationCalls: 1,
      });
      await rm(directory, { recursive: true, force: true });
    }
  );

  nativeTest(
    'tracks native Fetch and managed services called from a module',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms3-effects-'));
      const server = Bun.serve({
        hostname: '127.0.0.1',
        port: 0,
        fetch: () => new Response(JSON.stringify({ customer: 'Ada' })),
      });
      const modulePath = resolve(directory, 'customer.ts');
      const womlPath = resolve(directory, 'workflow.woml');
      await writeFile(
        modulePath,
        `export async function load() {
  const response = await fetch(${JSON.stringify(server.url.toString())});
  const customer = await response.json();
  await services.cache.set('module:customer', customer, { ttl: '1m' });
  return (await services.cache.get('module:customer')).value;
}
`
      );
      await writeFile(
        womlPath,
        `<woml>
  <imports><module name="customer" from="./customer.ts" /></imports>
  <workflow id="module-effects">
    <triggers><manual id="start" /></triggers>
    <steps><step id="load"><script>
      return await services.customer.load();
    </script></step></steps>
  </workflow>
</woml>
`
      );
      let stdout = '';
      let stderr = '';
      try {
        const exitCode = await runCli(
          ['test', womlPath, '--state', resolve(directory, 'state.sqlite')],
          {
            stdout: text => {
              stdout += text;
            },
            stderr: text => {
              stderr += text;
            },
          },
          nativeDependencies()
        );
        if (exitCode !== 0) throw new Error(stderr);
        expect(exitCode).toBe(0);
        expect(stderr).toBe('WOML modules ready: services.customer.\n');
        expect(JSON.parse(stdout)).toEqual({ customer: 'Ada' });
      } finally {
        server.stop(true);
        await rm(directory, { recursive: true, force: true });
      }
    }
  );
});

describe('Module recovery and composition', () => {
  nativeTest(
    'composes modules with branch, parallel, retry, and later references',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms4-compose-'));
      const modulePath = resolve(directory, 'utility.ts');
      const womlPath = resolve(directory, 'workflow.woml');
      await writeFile(
        modulePath,
        `export function decision() { return true; }
export function label(value) { return { label: String(value) }; }
export function retry(attempt) {
  if (attempt < 2) throw new Error('temporary module failure');
  return { retried: true };
}
`
      );
      await writeFile(
        womlPath,
        `<woml>
  <imports><module name="utility" from="./utility.ts" /></imports>
  <workflow id="ms4-composition" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="decide"><script>
        return { selected: services.utility.decision() };
      </script></step>
      <branch id="route">
        <when test="{{context.steps.decide.selected}}">
          <step id="chosen"><script>
            return services.utility.label('chosen');
          </script></step>
          <result value="{{context.steps.chosen}}" />
        </when>
        <otherwise>
          <step id="fallback"><script>return { label: 'fallback' };</script></step>
          <result value="{{context.steps.fallback}}" />
        </otherwise>
      </branch>
      <parallel id="fanout">
        <step id="left"><script>return services.utility.label('left');</script></step>
        <step id="right"><script>return services.utility.label('right');</script></step>
      </parallel>
      <step id="retryModule" retry="2" retry-delay="1ms"><script>
        return services.utility.retry(attempt.number);
      </script></step>
      <step id="finish"><script>
        return {
          route: context.steps.route.label,
          parallel: [context.steps.left.label, context.steps.right.label],
          retried: context.steps.retryModule.retried
        };
      </script></step>
    </steps>
  </workflow>
</woml>
`
      );
      try {
        let stdout = '';
        let stderr = '';
        const exitCode = await runCli(
          ['test', womlPath, '--state', resolve(directory, 'state.sqlite')],
          {
            stdout: text => {
              stdout += text;
            },
            stderr: text => {
              stderr += text;
            },
          },
          nativeDependencies()
        );
        expect(exitCode).toBe(0);
        expect(stderr).toContain('Step retryModule failed (attempt 1/2)');
        expect(stderr).toContain('Step retryModule succeeded on attempt 2/2');
        expect(JSON.parse(stdout)).toEqual({
          route: 'chosen',
          parallel: ['left', 'right'],
          retried: true,
        });
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    }
  );

  nativeTest(
    'resumes a stored run after the WOML and module sources disappear',
    async () => {
      const directory = await mkdtemp(
        resolve(tmpdir(), 'woml-ms4-source-free-')
      );
      const modulePath = resolve(directory, 'answer.ts');
      const womlPath = resolve(directory, 'workflow.woml');
      const statePath = resolve(directory, 'state.sqlite');
      await writeFile(
        modulePath,
        `export function value() { return { answer: 42 }; }\n`
      );
      await writeFile(
        womlPath,
        `<woml>
  <imports><module name="answer" from="./answer.ts" /></imports>
  <workflow id="ms4-source-free" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="answer"><script>
      return services.answer.value();
    </script></step></steps>
  </workflow>
</woml>\n`
      );
      try {
        const first = await (() => {
          let stdout = '';
          let stderr = '';
          return runCli(
            ['test', womlPath, '--state', statePath],
            {
              stdout: text => {
                stdout += text;
              },
              stderr: text => {
                stderr += text;
              },
            },
            nativeDependencies()
          ).then(exitCode => ({ exitCode, stdout, stderr }));
        })();
        expect(first.exitCode).toBe(0);
        expect(JSON.parse(first.stdout)).toEqual({ answer: 42 });
        const database = new Database(statePath, { readonly: true });
        const row = database
          .query(
            'SELECT run_id AS runId FROM woml_runs ORDER BY created_at DESC LIMIT 1'
          )
          .get() as { runId: string };
        database.close();

        await rm(modulePath);
        await rm(womlPath);
        let stdout = '';
        let stderr = '';
        const exitCode = await runCli(
          ['run', womlPath, '--state', statePath, '--resume', row.runId],
          {
            stdout: text => {
              stdout += text;
            },
            stderr: text => {
              stderr += text;
            },
          },
          nativeDependencies()
        );
        expect(exitCode).toBe(0);
        expect(stdout).toBe('');
        expect(
          inspectRunWithRust(statePath, row.runId, { nativeCorePath })
        ).toMatchObject({ status: 'succeeded', result: { answer: 42 } });
        expect(stderr).toContain(
          'stored definition and 1 immutable module artifact'
        );
        expect(stderr).not.toContain(modulePath);
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    }
  );

  test('accepts modules beside approval and every published production trigger', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms4-contracts-'));
    const modulePath = resolve(directory, 'utility.ts');
    await writeFile(modulePath, `export function value() { return true; }\n`);
    const sourcePaths = [
      resolve(projectRoot, 'woml/tests/fixtures/approval.woml'),
      resolve(projectRoot, 'examples/webhookWorkflow.woml'),
      resolve(projectRoot, 'examples/scheduleWorkflow.woml'),
      resolve(projectRoot, 'examples/intervalWorkflow.woml'),
      resolve(projectRoot, 'examples/eventWorkflow.woml'),
      resolve(projectRoot, 'examples/slackTriggerWorkflow.woml'),
    ];
    try {
      for (const [index, sourcePath] of sourcePaths.entries()) {
        const source = await readFile(sourcePath, 'utf8');
        const composed = source.replace(
          /<woml>\s*/,
          '<woml>\n  <imports><module name="utility" from="./utility.ts" /></imports>\n'
        );
        const composedPath = resolve(directory, `contract-${index}.woml`);
        await writeFile(composedPath, composed);
        const result = await invoke(['check', composedPath]);
        expect(result.exitCode).toBe(0);
        expect(result.stderr).toBe('');
        expect(result.stdout).toContain('services.utility');
      }
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});

describe('Local module authoring DX', () => {
  nativeTest('normal woml run refreshes editor types automatically', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms6-run-types-'));
    const womlPath = resolve(directory, 'workflow.woml');
    await writeFile(
      resolve(directory, 'math.js'),
      'export function add(left, right) { return left + right; }\n'
    );
    await writeFile(
      womlPath,
      `<woml>
  <imports><module name="math" from="./math.js" /></imports>
  <workflow id="ms6-run-types">
    <triggers><manual id="start" /></triggers>
    <steps><step id="calculate"><script>
      return { answer: services.math.add(20, 22) };
    </script></step></steps>
  </workflow>
</woml>
`
    );
    let stdout = '';
    let stderr = '';
    try {
      const statePath = resolve(directory, 'state.sqlite');
      const exitCode = await runCli(
        ['run', womlPath, '--state', statePath],
        {
          stdout: text => {
            stdout += text;
          },
          stderr: text => {
            stderr += text;
          },
        },
        {
          ...nativeDependencies(),
          createManualInput: () => new OneRunManualInput(statePath),
          waitForShutdown: () => new Promise<void>(() => {}),
        }
      );
      if (exitCode !== 0) throw new Error(stderr);
      expect(exitCode).toBe(0);
      expect(stderr).not.toContain('WOML_EDITOR_TYPES_WRITE_FAILED');
      expect(stdout).toBe('');
      const run = listRunsWithRust(
        statePath,
        { limit: 1 },
        { nativeCorePath }
      ).runs[0]!;
      expect(
        inspectRunWithRust(statePath, run.runId, { nativeCorePath })
      ).toMatchObject({ status: 'succeeded', result: { answer: 42 } });
      const declarations = await readFile(
        resolve(directory, 'woml-env.d.ts'),
        'utf8'
      );
      expect(declarations).toContain('readonly "math"');
      expect(declarations).toContain('readonly "add"');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('woml check refreshes types without a separate command', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms6-check-types-'));
    const womlPath = resolve(directory, 'workflow.woml');
    await writeFile(
      resolve(directory, 'math.js'),
      'export function add(left, right) { return left + right; }\n'
    );
    await writeFile(
      womlPath,
      `<woml>
  <imports><module name="math" from="./math.js" /></imports>
  <workflow id="ms6-check-types">
    <triggers><manual id="start" /></triggers>
    <steps><step id="calculate"><script>
      return services.math.add(20, 22);
    </script></step></steps>
  </workflow>
</woml>
`
    );
    try {
      const result = await invoke(['check', womlPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(
        await readFile(resolve(directory, 'woml-env.d.ts'), 'utf8')
      ).toContain('readonly "math"');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('editor type write failures warn without failing validation', async () => {
    const directory = await mkdtemp(
      resolve(tmpdir(), 'woml-ms6-types-warning-')
    );
    const womlPath = resolve(directory, 'workflow.woml');
    await writeFile(
      resolve(directory, 'math.js'),
      'export function add(left, right) { return left + right; }\n'
    );
    await writeFile(
      womlPath,
      `<woml>
  <imports><module name="math" from="./math.js" /></imports>
  <workflow id="ms6-types-warning">
    <triggers><manual id="start" /></triggers>
    <steps><step id="calculate"><script>return services.math.add(1, 1);</script></step></steps>
  </workflow>
</woml>
`
    );
    await mkdir(resolve(directory, 'woml-env.d.ts'));
    try {
      const result = await invoke(['check', womlPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('WOML check passed');
      expect(result.stderr).toContain('WOML_EDITOR_TYPES_WRITE_FAILED');
      expect(result.stderr).toContain('Workflow execution can continue');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('writes a self-contained woml-env.d.ts that removes editor errors', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms6-types-'));
    const modulePath = resolve(directory, 'openai.ts');
    const womlPath = resolve(directory, 'workflow.woml');
    const typePath = resolve(directory, 'woml-env.d.ts');
    await writeFile(
      modulePath,
      `export async function chat(message: string) {
  const response = await services.http.request<{ reply: string }>({
    url: 'https://example.test/chat',
    method: 'POST',
    json: { message }
  });
  return response.data.reply;
}
`
    );
    await writeFile(
      womlPath,
      `<woml>
  <imports><module name="openai" from="./openai.ts" /></imports>
  <workflow id="ms6-types">
    <triggers><manual id="start" /></triggers>
    <steps><step id="chat"><script>
      return await services.openai.chat('hello');
    </script></step></steps>
  </workflow>
</woml>
`
    );
    try {
      const result = await invoke(['types', womlPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toContain(typePath);
      const declarations = await readFile(typePath, 'utf8');
      expect(declarations).toContain('declare const services');
      expect(declarations).toContain('readonly "openai"');
      expect(declarations).toContain('readonly "chat"');

      const compiler = Bun.spawn(
        [
          resolve(import.meta.dir, '../node_modules/.bin/tsc'),
          '--ignoreConfig',
          '--noEmit',
          '--strict',
          '--skipLibCheck',
          '--target',
          'ES2022',
          '--module',
          'ESNext',
          '--moduleResolution',
          'Bundler',
          typePath,
          modulePath,
        ],
        { stdout: 'pipe', stderr: 'pipe' }
      );
      const [exitCode, stdout, stderr] = await Promise.all([
        compiler.exited,
        new Response(compiler.stdout).text(),
        new Response(compiler.stderr).text(),
      ]);
      expect({ exitCode, stdout, stderr }).toEqual({
        exitCode: 0,
        stdout: '',
        stderr: '',
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('prints a non-blocking diagnostic for a declared but unused module', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms6-unused-'));
    const modulePath = resolve(directory, 'unused.ts');
    const womlPath = resolve(directory, 'workflow.woml');
    await writeFile(modulePath, 'export function value() { return true; }\n');
    await writeFile(
      womlPath,
      `<woml>
  <imports><module name="unused" from="./unused.ts" /></imports>
  <workflow id="ms6-unused">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>return true;</script></step></steps>
  </workflow>
</woml>
`
    );
    try {
      const result = await invoke(['check', womlPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toContain('WOML_MODULE_UNUSED');
      expect(result.stdout).toContain('services.unused');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});

describe('For-each composition', () => {
  nativeTest(
    'runs control flow, reusable steps, retries, modules, Fetch, services, and lifecycle per item',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-for-each-composition-'));
      const server = Bun.serve({
        hostname: '127.0.0.1',
        port: 0,
        fetch: request => {
          const value = Number(new URL(request.url).searchParams.get('value'));
          return Response.json({ value: value * 10 });
        },
      });
      const womlPath = resolve(directory, 'workflow.woml');
      const statePath = resolve(directory, 'state.sqlite');
      await writeFile(
        resolve(directory, 'transform.ts'),
        `export async function normalize(key, value) {
  const response = await fetch(${JSON.stringify(server.url.toString())} + '?value=' + value);
  const remote = await response.json();
  const result = { key, value: remote.value, kind: 'active' };
  await services.cache.set('fe4:' + key, result, { ttl: '1m' });
  return result;
}
`
      );
      await writeFile(
        resolve(directory, 'normalize-item.woml'),
        `<woml>
  <imports><module name="transform" from="./transform.ts" /></imports>
  <props>
    <prop name="item-key" required="true" />
    <prop name="value" required="true" />
  </props>
  <step name="Normalize item">
    <script>
      if (props.value === 2 && attempt.number === 1) throw new Error('retry once');
      return await services.transform.normalize(props.itemKey, props.value);
    </script>
  </step>
  <lifecycle>
    <on-success><script>console.log('normalized ' + props.itemKey);</script></on-success>
    <on-complete><script>console.log('completed ' + props.itemKey);</script></on-complete>
  </lifecycle>
</woml>
`
      );
      await writeFile(
        womlPath,
        `<woml>
  <imports><module name="normalize-item" from="./normalize-item.woml" /></imports>
  <workflow id="for-each-composition" version="1.0.0">
    <lifecycle>
      <on-step-success steps="normalized">
        <script>
          await services.cache.set('fe4:lifecycle:' + context.iteration.index, { ok: true });
        </script>
      </on-step-success>
    </lifecycle>
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="load"><script>
        return { items: [
          { key: 'a', value: 1, enabled: true },
          { key: 'b', value: 2, enabled: true },
          { key: 'c', value: 3, enabled: false }
        ] };
      </script></step>
      <for-each id="processItems" items="{{context.steps.load.items}}" concurrency="1">
        <choose id="routed">
          <when test="{{context.item.enabled}}">
            <normalize-item
              id="normalized"
              item-key="{{context.item.key}}"
              value="{{context.item.value}}"
              retry="2"
              retry-delay="1ms"
            />
            <result value="{{context.steps.normalized}}" />
          </when>
          <otherwise>
            <step id="disabled"><script>
              return { key: context.item.key, value: 0, kind: 'disabled' };
            </script></step>
            <result value="{{context.steps.disabled}}" />
          </otherwise>
        </choose>
        <switch id="label" value="{{context.steps.routed.kind}}">
          <case value="active">
            <step id="activeLabel"><script>return { label: 'processed' };</script></step>
            <result value="{{context.steps.activeLabel}}" />
          </case>
          <default>
            <step id="disabledLabel"><script>return { label: 'skipped' };</script></step>
            <result value="{{context.steps.disabledLabel}}" />
          </default>
        </switch>
        <parallel id="inspectItem" concurrency="2" on-error="wait-all">
          <step id="cacheRead"><script>
            const cached = await services.cache.get('fe4:' + context.item.key);
            return { key: context.item.key, cached: cached.hit, label: context.steps.label.label };
          </script></step>
          <step id="mirror"><script>
            return { key: context.item.key, index: context.iteration.index };
          </script></step>
        </parallel>
        <result value="{{context.steps.cacheRead}}" />
      </for-each>
      <step id="summary"><script>
        return { results: context.steps.processItems.results };
      </script></step>
    </steps>
  </workflow>
</woml>
`
      );
      let stdout = '';
      let stderr = '';
      try {
        const exitCode = await runCli(
          ['test', womlPath, '--state', statePath],
          {
            stdout: text => {
              stdout += text;
            },
            stderr: text => {
              stderr += text;
            },
          },
          nativeDependencies()
        );
        if (exitCode !== 0) throw new Error(stderr);
        expect(exitCode).toBe(0);
        expect(JSON.parse(stdout)).toEqual({
          results: [
            { key: 'a', cached: true, label: 'processed' },
            { key: 'b', cached: true, label: 'processed' },
            { key: 'c', cached: false, label: 'skipped' },
          ],
        });
        expect(stderr).not.toContain('WOML_FOR_EACH');
        const run = listRunsWithRust(statePath, { limit: 1 }, { nativeCorePath }).runs[0]!;
        const inspected = inspectRunWithRust(statePath, run.runId, { nativeCorePath });
        expect(inspected.status).toBe('succeeded');
        const database = new Database(statePath, { readonly: true });
        const rows = database
          .query(
            'SELECT event_json AS eventJson FROM woml_run_events WHERE run_id = ? ORDER BY sequence'
          )
          .all(run.runId) as { eventJson: string }[];
        database.close();
        const history = rows.map(row => row.eventJson).join('\n');
        expect(history).toContain('branch_selected');
        expect(history).toContain('choice_selected');
        expect(history).toContain('parallel_group_started');
        expect(history).toContain('step_retry_scheduled');
        expect(history).toContain('reusable_lifecycle_requested');
        expect(history).toContain('lifecycle_hook_requested');
        expect(history).toContain('"executionMode":"observed"');
        expect(history).toContain('"executionMode":"managed"');
        expect(history).toContain(
          '"iteration":{"forEachId":"processItems","index":1}'
        );
      } finally {
        server.stop(true);
        await rm(directory, { recursive: true, force: true });
      }
    },
    20_000
  );
});
