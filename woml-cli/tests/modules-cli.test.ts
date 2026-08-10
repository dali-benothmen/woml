import { describe, expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

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
      throw new Error('MS3 fixtures have no secrets.');
    },
  };
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

describe('MS3 module runtime CLI', () => {
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
          'run',
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
      expect(stderr).toContain('WOML automation is active.');
      expect(stderr).toContain('WOML automation stopped.');
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

describe('MS4 module recovery and composition', () => {
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
        expect(JSON.parse(stdout)).toEqual({ answer: 42 });
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
