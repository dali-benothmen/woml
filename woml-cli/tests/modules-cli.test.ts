import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
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
const verticalWorkflowPath = resolve(projectRoot, 'examples/moduleWorkflow.woml');

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
    expect(stderr).toBe('');
    expect(JSON.parse(stdout)).toEqual({ rows: [] });
    await rm(directory, { recursive: true, force: true });
  });

  nativeTest('runs two sequential module steps with one reference and fresh state', async () => {
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
  });

  nativeTest('tracks native Fetch and managed services called from a module', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-ms3-effects-'));
    const server = Bun.serve({
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
      expect(stderr).toBe('');
      expect(JSON.parse(stdout)).toEqual({ customer: 'Ada' });
    } finally {
      server.stop(true);
      await rm(directory, { recursive: true, force: true });
    }
  });
});
