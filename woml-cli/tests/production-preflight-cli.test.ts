import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import {
  preflightRuntimeConfiguration,
  resolveRuntimeConfiguration,
} from '../src/runtime-config';
import type { SecretMetadata, SecretStore } from '../src/secrets';

class MemorySecretStore implements SecretStore {
  readonly provider = 'environment' as const;
  readonly #values: Readonly<Record<string, string>>;
  constructor(values: Readonly<Record<string, string>> = {}) { this.#values = values; }
  async get(name: string) { return this.#values[name]; }
  async has(name: string) { return this.#values[name] !== undefined; }
  async list(): Promise<readonly SecretMetadata[]> { return Object.keys(this.#values).map(name => ({ name, provider: this.provider })); }
  async set(_name: string, _value: string): Promise<void> { throw new Error('read-only'); }
  async delete(_name: string): Promise<boolean> { throw new Error('read-only'); }
}

async function invoke(args: readonly string[], secrets: Readonly<Record<string, string>> = {}) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = { stdout: value => { stdout += value; }, stderr: value => { stderr += value; } };
  const exitCode = await runCli(args, io, {
    createSecretStore: () => new MemorySecretStore(secrets),
    readSecret: async () => '',
  });
  return { exitCode, stdout, stderr };
}

function workflow(id: string, trigger: string): string {
  return `<woml><workflow id="${id}"><triggers>${trigger}</triggers><steps><step id="done"><script>return { ok: true };</script></step></steps></workflow></woml>`;
}

describe('PRO1 Runtime Configuration and production preflight', () => {
  test('applies CLI, reviewed environment, config, and default precedence with relative paths', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro1-config-'));
    try {
      const path = join(directory, 'woml.runtime.json');
      await Bun.write(path, JSON.stringify({
        schemaVersion: 1,
        deploymentName: 'from-config',
        statePath: './data/state.sqlite',
        public: { host: '127.0.0.2', port: 4100 },
        logging: { level: 'warn', directory: './logs' },
      }));
      const resolved = await resolveRuntimeConfiguration(
        path,
        { publicPort: 4300 },
        { WOML_RUNTIME_DEPLOYMENT: 'from-env', WOML_RUNTIME_HOST: '127.0.0.3' },
        '/ignored'
      );
      expect(resolved).toMatchObject({
        deploymentName: 'from-env',
        statePath: join(directory, 'data/state.sqlite'),
        public: { host: '127.0.0.3', port: 4300 },
        admin: { host: '127.0.0.1', port: 3001 },
        logging: { level: 'warn', directory: join(directory, 'logs') },
        sources: {
          deploymentName: 'environment',
          statePath: 'config',
          'public.host': 'environment',
          'public.port': 'cli',
          'admin.port': 'default',
        },
      });
      const environmentPath = await resolveRuntimeConfiguration(
        path,
        {},
        { WOML_RUNTIME_STATE: './environment/state.sqlite' },
        directory
      );
      expect(environmentPath.statePath).toBe(
        join(directory, 'environment/state.sqlite')
      );
      expect(environmentPath.sources.statePath).toBe('environment');
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  test('rejects unsupported versions, unknown fields, unsafe bounds, and listener collisions', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro1-invalid-'));
    try {
      for (const [value, message] of [
        [{ schemaVersion: 2 }, 'schemaVersion must be 1'],
        [{ schemaVersion: 1, secret: 'no' }, 'unknown field'],
        [{ schemaVersion: 1, workers: 0 }, 'workers must be an integer'],
        [{ schemaVersion: 1, public: { port: 3001 } }, 'cannot use the same host and port'],
      ] as const) {
        const path = join(directory, `${Math.random()}.json`);
        await Bun.write(path, JSON.stringify(value));
        await expect(resolveRuntimeConfiguration(path)).rejects.toThrow(message);
      }
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  test('checks writable storage and disk headroom without creating runtime files', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro1-storage-'));
    try {
      const path = join(directory, 'runtime.json');
      await Bun.write(path, JSON.stringify({ schemaVersion: 1, statePath: './nested/state.sqlite', logging: { directory: './logs' } }));
      const config = await resolveRuntimeConfiguration(path);
      const report = await preflightRuntimeConfiguration(config);
      expect(report.profile).toBe('woml.runtime-preflight/v1');
      expect(report.state).toMatchObject({ path: join(directory, 'nested/state.sqlite'), existing: false, writableAncestor: directory });
      expect(report.state.availableBytes).toBeGreaterThanOrEqual(64 * 1024 * 1024);
      expect(await Bun.file(config.statePath).exists()).toBe(false);
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  test('checks multiple workflows as one deployment and aggregates environment readiness', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro1-deployment-'));
    try {
      const first = join(directory, 'orders.woml');
      const second = join(directory, 'agent.woml');
      const config = join(directory, 'runtime.json');
      await Bun.write(first, workflow('orders', '<webhook id="order" path="/orders" method="POST" auth="bearer" secret="{{secrets.ORDER_TOKEN}}" />'));
      await Bun.write(second, workflow('agent', '<manual id="start" />'));
      await Bun.write(config, JSON.stringify({ schemaVersion: 1, deploymentName: 'test-runtime', statePath: './data/state.sqlite' }));

      const checked = await invoke(['check', first, second, '--config', config, '--json'], { ORDER_TOKEN: 'configured' });
      expect(checked.exitCode).toBe(0);
      expect(checked.stderr).toBe('');
      const report = JSON.parse(checked.stdout);
      expect(report).toMatchObject({
        profile: 'woml.production-preflight/v1',
        status: 'passed',
        configuration: { deploymentName: 'test-runtime' },
        secretProvider: 'environment',
      });
      expect(report.workflows.map((entry: { workflowId: string }) => entry.workflowId)).toEqual(['agent', 'orders']);
      expect(report.workflows[1].requiredSecrets).toEqual(['ORDER_TOKEN']);
      expect(await Bun.file(join(directory, 'data/state.sqlite')).exists()).toBe(false);
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  test('reports every missing required secret and rejects cross-workflow route conflicts', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro1-errors-'));
    try {
      const one = join(directory, 'one.woml');
      const two = join(directory, 'two.woml');
      const config = join(directory, 'runtime.json');
      await Bun.write(config, JSON.stringify({ schemaVersion: 1 }));
      await Bun.write(one, workflow('one', '<webhook id="one" path="/one" method="POST" auth="bearer" secret="{{secrets.FIRST_TOKEN}}" />'));
      await Bun.write(two, workflow('two', '<webhook id="two" path="/two" method="POST" auth="bearer" secret="{{secrets.SECOND_TOKEN}}" />'));
      const missing = await invoke(['check', directory, '--config', config]);
      expect(missing.exitCode).toBe(1);
      expect(missing.stderr).toContain('FIRST_TOKEN');
      expect(missing.stderr).toContain('SECOND_TOKEN');

      await Bun.write(two, workflow('two', '<webhook id="two" path="/one" method="POST" auth="none" />'));
      const conflict = await invoke(['check', one, two, '--config', config], { FIRST_TOKEN: 'configured' });
      expect(conflict.exitCode).toBe(1);
      expect(conflict.stderr).toContain('WOML_WEBHOOK_ROUTE_CONFLICT');
    } finally { await rm(directory, { recursive: true, force: true }); }
  });

  test('keeps the historical single-workflow author check and does not activate anything', async () => {
    const fixture = resolve(import.meta.dir, '../../woml/tests/fixtures/hello.woml');
    const checked = await invoke(['check', fixture]);
    expect(checked.exitCode).toBe(0);
    expect(checked.stdout).toContain('WOML check passed for workflow');
    expect(checked.stdout).not.toContain('production check');
    expect(checked.stderr).toBe('');
  });
});
