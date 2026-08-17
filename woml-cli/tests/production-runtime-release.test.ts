import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';

import {
  readRuntimeDescriptor,
  runtimeDescriptorPath,
} from '../src/runtime-control';
import { nativePackageBinaryName } from '../src/native-platform';
import { installLocalReleaseCandidate } from './helpers/release-candidate';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-pro9-release-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

async function freePort(): Promise<number> {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response('reserved'),
  });
  const port = server.port!;
  await server.stop(true);
  return port;
}

async function waitUntil(predicate: () => Promise<boolean>, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await Bun.sleep(25);
  }
  throw new Error('Timed out waiting for the clean WOML server.');
}

async function filesBelow(root: string): Promise<string[]> {
  const files: string[] = [];
  const visit = async (directory: string): Promise<void> => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else files.push(path);
    }
  };
  await visit(root);
  return files.sort();
}

const webhookWorkflow = `<woml>
<workflow id="pro9-orders" name="orders" version="1.0.0">
  <config concurrency="4" timeout="30s" queue="orders" />
  <triggers>
    <webhook id="order" path="/orders" method="POST" auth="none">
      <schema>{"type":"object","required":["orderId"],"properties":{"orderId":{"type":"string"}},"additionalProperties":false}</schema>
    </webhook>
  </triggers>
  <steps><step id="remember"><script>
    const count = await services.state.increment('orders', 1, { name: 'count-order' });
    return { orderId: context.payload.orderId, count: count.value };
  </script></step></steps>
</workflow>
</woml>`;

describe('Clean package and server release', () => {
  test('the complete production example compiles as one deployment', () => {
    const result = Bun.spawnSync(
      [
        process.execPath,
        resolve(packageRoot, 'dist/cli.js'),
        'check',
        resolve(projectRoot, 'examples/production/complete/workflows'),
      ],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(result.exitCode, result.stderr.toString()).toBe(0);
    expect(result.stdout.toString()).toContain(
      'WOML production check passed for 4 workflows'
    );
  });

  test('a clean consumer installs, activates, serves, observes, backs up, stops, restores, and prunes', async () => {
    const consumerDirectory = join(temporaryDirectory, 'server');
    const cacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [consumerDirectory, cacheDirectory].map(path =>
        mkdir(path, { recursive: true })
      )
    );

    await writeFile(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-pro9-clean-server', private: true })
    );
    const candidate = await installLocalReleaseCandidate(consumerDirectory, {
      cache: cacheDirectory,
    });

    const installedPackage = join(
      consumerDirectory,
      'node_modules/woml'
    );
    const installedFiles = await filesBelow(installedPackage);
    const relativeFiles = installedFiles.map(path => relative(installedPackage, path));
    expect(relativeFiles).toContain('dist/cli.js');
    expect(relativeFiles.some(path => path.endsWith('.node'))).toBe(false);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          ...candidate.nativePackage.split('/'),
          nativePackageBinaryName(candidate.target),
        ),
      ).exists(),
    ).toBe(true);
    expect(relativeFiles.some(path => path.startsWith('src/'))).toBe(false);
    expect(relativeFiles.some(path => path.startsWith('tests/'))).toBe(false);
    expect(relativeFiles.some(path => path.endsWith('.sqlite'))).toBe(false);
    expect(relativeFiles.some(path => path.endsWith('.woml'))).toBe(false);

    const executable = join(consumerDirectory, 'node_modules/.bin/woml');
    const workflows = join(consumerDirectory, 'workflows');
    const workflowPath = join(workflows, 'orders.woml');
    const statePath = join(consumerDirectory, 'data/state.sqlite');
    const configPath = join(consumerDirectory, 'woml.runtime.json');
    const publicPort = await freePort();
    const adminPort = await freePort();
    await mkdir(workflows);
    await writeFile(workflowPath, webhookWorkflow);
    await writeFile(
      configPath,
      JSON.stringify({
        schemaVersion: 1,
        deploymentName: 'pro9-clean-server',
        statePath,
        public: { host: '127.0.0.1', port: publicPort },
        admin: { host: '127.0.0.1', port: adminPort },
        logging: { format: 'json', directory: join(consumerDirectory, 'logs') },
        observability: { health: true, metrics: true },
        retention: { enabled: false },
      })
    );

    const checked = Bun.spawnSync(
      [executable, 'check', workflows, '--config', configPath],
      { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(checked.exitCode, checked.stderr.toString()).toBe(0);

    const runtime = Bun.spawn(
      [executable, 'run', workflows, '--config', configPath],
      { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
    );
    try {
      const descriptorFile = runtimeDescriptorPath(statePath);
      await waitUntil(() => Bun.file(descriptorFile).exists());
      const descriptor = await readRuntimeDescriptor(descriptorFile);
      const response = await fetch(`http://127.0.0.1:${publicPort}/orders`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ orderId: 'order-pro9' }),
      });
      expect(response.status).toBe(202);
      await waitUntil(async () => {
        const listed = Bun.spawnSync(
          [executable, 'list', '--state', statePath, '--json'],
          { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
        );
        if (listed.exitCode !== 0) return false;
        const runs = JSON.parse(listed.stdout.toString()).runs;
        return runs.some((run: { status: string }) => run.status === 'succeeded');
      });

      const metrics = await fetch(`${descriptor.adminUrl}/metrics`, {
        headers: { authorization: `Bearer ${descriptor.capability}` },
      }).then(value => value.text());
      expect(metrics).toContain('woml_runtime_ready 1');

      const backupPath = join(consumerDirectory, 'backups/pro9');
      const backup = Bun.spawnSync(
        [executable, 'backup', backupPath, '--state', statePath, '--json'],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(backup.exitCode, backup.stderr.toString()).toBe(0);
      expect(JSON.parse(backup.stdout.toString())).toMatchObject({ verified: true });

      const stopped = Bun.spawnSync(
        [executable, 'stop', '--state', statePath],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(stopped.exitCode, stopped.stderr.toString()).toBe(0);
      expect(await runtime.exited).toBe(0);

      const restoredPath = join(consumerDirectory, 'restored/state.sqlite');
      const restore = Bun.spawnSync(
        [
          executable,
          'restore',
          backupPath,
          '--state',
          restoredPath,
          '--json',
        ],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(restore.exitCode, restore.stderr.toString()).toBe(0);
      const restoredRuns = Bun.spawnSync(
        [executable, 'list', '--state', restoredPath, '--json'],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(restoredRuns.exitCode, restoredRuns.stderr.toString()).toBe(0);
      expect(JSON.parse(restoredRuns.stdout.toString()).runs).toHaveLength(1);

      const dryRun = Bun.spawnSync(
        [
          executable,
          'prune',
          '--before',
          '30d',
          '--state',
          restoredPath,
          '--dry-run',
          '--json',
        ],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(dryRun.exitCode, dryRun.stderr.toString()).toBe(0);
      expect(JSON.parse(dryRun.stdout.toString())).toMatchObject({
        profile: 'woml.retention/v1',
        kind: 'plan',
      });

      const corruptPath = join(consumerDirectory, 'corrupt.sqlite');
      await writeFile(corruptPath, 'not a sqlite database');
      const corrupt = Bun.spawnSync(
        [executable, 'prune', '--before', '30d', '--state', corruptPath],
        { cwd: consumerDirectory, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(corrupt.exitCode).toBe(1);
      expect(corrupt.stderr.toString()).toContain('WOML_RETENTION');
    } finally {
      if (runtime.exitCode === null) runtime.kill('SIGKILL');
      await runtime.exited;
    }
  }, 60_000);

  test('installed public and durable artifacts contain no active release secrets', async () => {
    const forbidden = [
      'xoxb-live-',
      'xapp-live-',
      'sk-live-',
      'PRIVATE_CUSTOMER_PAYLOAD_PRO9',
      'ACTIVE_RELEASE_SECRET_PRO9',
    ];
    const roots = [
      join(packageRoot, 'dist'),
      join(projectRoot, 'examples/production'),
      join(projectRoot, 'docs'),
    ];
    for (const root of roots) {
      if (!existsSync(root)) continue;
      for (const path of await filesBelow(root)) {
        if (path.endsWith('.node') || path.endsWith('.png')) continue;
        const contents = await readFile(path, 'utf8').catch(() => '');
        for (const secret of forbidden) {
          expect(contents.includes(secret), path).toBe(false);
        }
      }
    }
  });
});
