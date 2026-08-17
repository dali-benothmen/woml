import { afterEach, describe, expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  nextRetentionTime,
  parsePruneArguments,
  startAutomaticRetention,
} from '../src/production-retention';
import { RuntimeObservability } from '../src/runtime-observability';

const executable = resolve(import.meta.dir, '../dist/cli.js');
const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map(path =>
      rm(path, { recursive: true, force: true })
    )
  );
});

async function directory(label: string): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), `woml-pro8-${label}-`));
  temporaryDirectories.push(path);
  return path;
}

async function invoke(cwd: string, ...args: string[]) {
  const child = Bun.spawn([process.execPath, executable, ...args], {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  return { exitCode, stdout, stderr };
}

const workflow = `<woml>
<workflow id="retention" name="retention" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="write"><script>
    await services.state.set('retained-value', 42, { name: 'write' });
    return { ok: true };
  </script></step></steps>
</workflow>
</woml>`;

async function seed(root: string) {
  const workflowPath = join(root, 'retention.woml');
  const statePath = join(root, '.woml/state.sqlite');
  await writeFile(workflowPath, workflow);
  const execution = await invoke(root, 'test', workflowPath, '--state', statePath);
  expect(execution.exitCode, execution.stderr).toBe(0);
  const database = new Database(statePath);
  database.run(
    "UPDATE woml_run_summaries SET updated_at = '2025-01-01T00:00:00.000Z'"
  );
  database.close();
  return { statePath };
}

describe('Retention and storage maintenance', () => {
  test('parses the simple CLI and computes the next UTC maintenance window', () => {
    expect(parsePruneArguments(['prune', '--before', '30d', '--dry-run']))
      .toMatchObject({ before: '30d', dryRun: true, compact: false });
    expect(
      nextRetentionTime(new Date('2026-08-12T03:00:00.000Z'), 3).toISOString()
    ).toBe('2026-08-13T03:00:00.000Z');
    expect(() => parsePruneArguments(['prune', '--before', '30 days']))
      .toThrow('whole number followed by h, d, or w');
  });

  test('dry-run predicts execution exactly and pruning preserves State v1 and definitions', async () => {
    const root = await directory('journey');
    const { statePath } = await seed(root);
    const dry = await invoke(
      root,
      'prune',
      '--before',
      '30d',
      '--state',
      statePath,
      '--dry-run',
      '--json'
    );
    expect(dry.exitCode, dry.stderr).toBe(0);
    const plan = JSON.parse(dry.stdout);
    expect(plan).toMatchObject({
      profile: 'woml.retention/v1',
      kind: 'plan',
      eligibleRuns: 1,
      estimatedBytes: expect.any(Number),
    });
    let database = new Database(statePath, { readonly: true });
    expect(database.query('SELECT COUNT(*) AS count FROM woml_runs').get())
      .toEqual({ count: 1 });
    database.close();

    const prune = await invoke(
      root,
      'prune',
      '--before',
      '30d',
      '--state',
      statePath,
      '--json'
    );
    expect(prune.exitCode, prune.stderr).toBe(0);
    const result = JSON.parse(prune.stdout);
    expect(result).toMatchObject({
      profile: 'woml.retention/v1',
      kind: 'result',
      deletedRuns: plan.eligibleRuns,
      deletedBytes: plan.estimatedBytes,
      stateEntriesDeleted: 0,
    });
    database = new Database(statePath, { readonly: true });
    expect(database.query('SELECT COUNT(*) AS count FROM woml_runs').get())
      .toEqual({ count: 0 });
    expect(database.query('SELECT COUNT(*) AS count FROM woml_definitions').get())
      .toEqual({ count: 1 });
    expect(database.query('SELECT value_json FROM woml_state_entries').get())
      .toEqual({ value_json: '42' });
    database.close();
  }, 30_000);

  test('automatic retention uses the same authority without blocking its scheduler', async () => {
    const root = await directory('automatic');
    const { statePath } = await seed(root);
    let scheduledDelay = 0;
    let automaticError: unknown;
    const handle = startAutomaticRetention({
      statePath,
      configuration: {
        enabled: true,
        succeededAfterDays: 30,
        failedAfterDays: 90,
        cancelledAfterDays: 30,
        maintenanceHourUtc: 3,
      },
      ownerId: 'pro8_test_runtime',
      nativeCorePath: resolve(
        import.meta.dir,
        `../dist/woml-core.${process.platform}-${process.arch}.node`
      ),
      now: () => new Date('2026-08-12T12:00:00.000Z'),
      setTimer: (_callback, delay) => {
        scheduledDelay = delay;
        return 1;
      },
      clearTimer: () => {},
      onError: error => {
        automaticError = error;
      },
    });
    expect(handle.nextRunAt).toBe('2026-08-13T03:00:00.000Z');
    expect(scheduledDelay).toBe(15 * 60 * 60 * 1_000);
    expect((await handle.runNow())?.result.deletedRuns, String(automaticError)).toBe(1);
    handle.close();
  }, 30_000);

  test('reports retention through operations, health, and metrics without payload data', async () => {
    const observed = new RuntimeObservability({
      runtimeInstanceId: 'runtime_pro8',
      deploymentId: 'deployment_pro8',
      workflows: [],
      listRuns: () => ({ profile: 'woml.run-list/v2', runs: [] }),
      storeSize: async () => 8192,
      logFormat: 'json',
      emitLog: () => {},
      components: [
        { name: 'retention', kind: 'retention', status: 'ready' },
      ],
    });
    observed.setLifecycle('ready');
    observed.recordMaintenance('retention', 'completed');
    expect(observed.detailedHealth()).toMatchObject({
      status: 'ready',
      components: [{ name: 'retention', status: 'ready' }],
    });
    expect(await observed.metrics()).toContainEqual({
      profile: 'woml.runtime-metrics/v1',
      name: 'woml_retention_total',
      type: 'counter',
      value: 1,
      labels: { status: 'completed' },
    });
  });
});
