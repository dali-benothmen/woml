#!/usr/bin/env bun

import { Database } from 'bun:sqlite';
import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { RuntimeObservability } from '../src/runtime-observability';
import {
  renderInspectorFrame,
  type InspectorSnapshotV1,
} from '../src/runtime-inspector';
import {
  readRuntimeDescriptor,
  requestRuntimeStop,
  runtimeDescriptorPath,
} from '../src/runtime-control';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = resolve(packageRoot, 'dist/cli.js');
const nativeCore = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const budgets = await Bun.file(
  resolve(projectRoot, 'examples/production/performance-budgets.v1.json')
).json() as Record<string, number | string>;
const root = await mkdtemp(join(tmpdir(), 'woml-pro9-benchmark-'));

async function freePort(): Promise<number> {
  const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
  const port = server.port!;
  await server.stop(true);
  return port;
}

async function waitUntil(check: () => Promise<boolean>, timeout = 15_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await Bun.sleep(20);
  }
  throw new Error('PRO9 benchmark timed out.');
}

function invoke(...args: string[]) {
  return Bun.spawnSync([process.execPath, executable, ...args], {
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  });
}

function enforce(name: string, value: number): void {
  const budget = Number(budgets[name]);
  if (!Number.isFinite(value) || value > budget) {
    throw new Error(`${name} exceeded: ${value.toFixed(2)} > ${budget}.`);
  }
}

const workflow = `<woml><workflow id="pro9-benchmark" version="1.0.0">
<triggers><webhook id="ping" path="/ping" method="POST" auth="none"><schema>{"type":"object","properties":{"id":{"type":"string"}}}</schema></webhook></triggers>
<steps><step id="done"><script>return { id: context.payload.id };</script></step></steps>
</workflow></woml>`;

let runtime: ReturnType<typeof Bun.spawn> | undefined;
try {
  await mkdir(join(root, 'workflows'));
  const workflowPath = join(root, 'workflows/benchmark.woml');
  const statePath = join(root, 'data/state.sqlite');
  const configPath = join(root, 'woml.runtime.json');
  const publicPort = await freePort();
  const adminPort = await freePort();
  await writeFile(workflowPath, workflow);
  await writeFile(configPath, JSON.stringify({
    schemaVersion: 1,
    statePath,
    public: { host: '127.0.0.1', port: publicPort },
    admin: { host: '127.0.0.1', port: adminPort },
    logging: { format: 'json', directory: join(root, 'logs') },
    observability: { health: true, metrics: true },
  }));

  const cliStarted = performance.now();
  const version = invoke('--version');
  const cliVersionMs = performance.now() - cliStarted;
  if (version.exitCode !== 0) throw new Error(version.stderr.toString());

  const startRuntime = async () => {
    const started = performance.now();
    const child = Bun.spawn(
      [process.execPath, executable, 'run', 'workflows', '--config', configPath],
      { cwd: root, stdout: 'pipe', stderr: 'pipe' }
    );
    await waitUntil(() => Bun.file(runtimeDescriptorPath(statePath)).exists());
    const descriptor = await readRuntimeDescriptor(runtimeDescriptorPath(statePath));
    return { child, descriptor, elapsed: performance.now() - started };
  };

  const first = await startRuntime();
  runtime = first.child;
  const admissionStarted = performance.now();
  const admissions = await Promise.all(
    Array.from({ length: 20 }, (_, index) =>
      fetch(`http://127.0.0.1:${publicPort}/ping`, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'idempotency-key': `pro9-benchmark-${index}`,
        },
        body: JSON.stringify({ id: String(index) }),
      })
    )
  );
  if (admissions.some(response => response.status !== 202)) {
    throw new Error('A benchmark admission was rejected.');
  }
  const admissionAverageMs = (performance.now() - admissionStarted) / admissions.length;
  await waitUntil(async () => {
    const result = invoke('list', '--state', statePath, '--json');
    if (result.exitCode !== 0) return false;
    return JSON.parse(result.stdout.toString()).runs.filter(
      (run: { status: string }) => run.status === 'succeeded'
    ).length === 20;
  });

  const headers = { authorization: `Bearer ${first.descriptor.capability}` };
  const snapshotStarted = performance.now();
  const snapshot = await fetch(`${first.descriptor.adminUrl}/v1/snapshot`, { headers })
    .then(response => response.json()) as InspectorSnapshotV1;
  const snapshotMs = performance.now() - snapshotStarted;
  const metricsStarted = performance.now();
  await fetch(`${first.descriptor.adminUrl}/metrics`, { headers }).then(response => response.text());
  const metricsMs = performance.now() - metricsStarted;

  const inspectorStarted = performance.now();
  renderInspectorFrame(snapshot, {
    view: 'overview', selected: 0, filter: '', expanded: false,
    showHelp: false, stale: false, recentEvents: [],
  }, 120, 40, true);
  const inspectorRenderMs = performance.now() - inspectorStarted;

  const observed = new RuntimeObservability({
    runtimeInstanceId: 'runtime_benchmark', deploymentId: 'deployment_benchmark',
    workflows: [], listRuns: () => ({ profile: 'woml.run-list/v2', runs: [] }),
    storeSize: async () => 0, logFormat: 'json', emitLog: () => {},
  });
  const streamStarted = performance.now();
  for (let index = 0; index < 20_000; index += 1) {
    observed.record('run', `run_${index}`, 'running');
  }
  const streamEventsMs = performance.now() - streamStarted;

  const backupStarted = performance.now();
  const backup = invoke('backup', join(root, 'backup'), '--state', statePath, '--json');
  const backupMs = performance.now() - backupStarted;
  if (backup.exitCode !== 0) throw new Error(backup.stderr.toString());

  await requestRuntimeStop(first.descriptor);
  await runtime.exited;
  runtime = undefined;
  const recovered = await startRuntime();
  runtime = recovered.child;
  await requestRuntimeStop(recovered.descriptor);
  await runtime.exited;
  runtime = undefined;

  const database = new Database(statePath);
  database.run("UPDATE woml_run_summaries SET updated_at = '2025-01-01T00:00:00.000Z'");
  database.close();
  const retentionStarted = performance.now();
  const retention = invoke('prune', '--before', '30d', '--state', statePath, '--json');
  const retentionMs = performance.now() - retentionStarted;
  if (retention.exitCode !== 0) throw new Error(retention.stderr.toString());

  const installedRuntimeBytes = (await stat(nativeCore)).size +
    (await stat(executable)).size +
    (await stat(resolve(packageRoot, 'dist/script-host.js'))).size +
    (await stat(resolve(packageRoot, 'dist/script-host-worker.js'))).size +
    (await stat(resolve(packageRoot, 'dist/notification-provider-host.js'))).size;

  const results = {
    profile: 'woml.production-performance-results/v1',
    cliVersionMs,
    startupMs: first.elapsed,
    recoveryMs: recovered.elapsed,
    admissionAverageMs,
    snapshotMs,
    metricsMs,
    streamEventsMs,
    inspectorRenderMs,
    backupMs,
    retentionMs,
    installedRuntimeBytes,
  };
  for (const [name, value] of Object.entries(results)) {
    if (name !== 'profile') enforce(name, Number(value));
  }
  console.log(JSON.stringify(results));
} finally {
  if (runtime?.exitCode === null) runtime.kill('SIGKILL');
  if (runtime !== undefined) await runtime.exited;
  await rm(root, { recursive: true, force: true });
}
