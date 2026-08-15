import { afterAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  readRuntimeDescriptor,
  runtimeDescriptorPath,
} from '../src/runtime-control';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const workflowPath = join(projectRoot, 'examples', 'webhookWorkflow.woml');
const scriptPath = Bun.which('script');
const packagedTest = scriptPath === null || process.platform === 'win32' ? test.skip : test;
const directories: string[] = [];

afterAll(async () => {
  await Promise.all(directories.map(path => rm(path, { recursive: true, force: true })));
});

async function availablePort(): Promise<number> {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = server.port;
  server.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a local port.');
  return port;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

async function waitUntil(
  predicate: () => boolean | Promise<boolean>,
  description: string,
  output?: () => string,
  timeoutMs = 15_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!(await predicate())) {
    if (Date.now() >= deadline) {
      throw new Error(
        `Timed out waiting for ${description}${output === undefined ? '' : `:\n${output()}`}`
      );
    }
    await Bun.sleep(20);
  }
}

function collect(stream: ReadableStream<Uint8Array>, append: (text: string) => void) {
  return (async () => {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      append(decoder.decode(chunk.value, { stream: true }));
    }
    append(decoder.decode());
  })();
}

describe('packaged background workflow log following', () => {
  packagedTest('shows history, follows live runs, detaches safely, and preserves runtime ownership', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-log-follow-'));
    directories.push(directory);
    const statePath = join(directory, 'data', 'state.sqlite');
    const configPath = join(directory, 'runtime.json');
    const publicPort = await availablePort();
    const adminPort = await availablePort();
    await mkdir(join(directory, 'logs'), { recursive: true });
    await writeFile(
      configPath,
      JSON.stringify({
        schemaVersion: 1,
        deploymentName: 'log-follow-test',
        statePath,
        public: { host: '127.0.0.1', port: publicPort },
        admin: { host: '127.0.0.1', port: adminPort },
        logging: { format: 'text', directory: join(directory, 'logs') },
        observability: { health: true, metrics: true },
        retention: { enabled: false },
      })
    );

    const started = Bun.spawnSync(
      [cliPath, 'run', workflowPath, '--config', configPath, '--background'],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(started.exitCode, started.stderr.toString()).toBe(0);
    expect(started.stdout.toString()).toContain(
      `woml webhook-demo --logs --state ${JSON.stringify(statePath)}`
    );
    const descriptorFile = runtimeDescriptorPath(statePath);

    try {
      await waitUntil(() => Bun.file(descriptorFile).exists(), 'the runtime descriptor');
      const invoke = async (key: string, orderId: string) => {
        const response = await fetch(`http://127.0.0.1:${publicPort}/webhooks/orders`, {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            'idempotency-key': key,
          },
          body: JSON.stringify({ orderId }),
        });
        expect(response.status).toBe(202);
        return (await response.json()) as { runId: string };
      };

      const first = await invoke('logs-first', 'order-logs-first');
      await waitUntil(() => {
        const result = Bun.spawnSync(
          [cliPath, 'get', first.runId, '--state', statePath, '--json'],
          { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
        );
        return result.exitCode === 0 && JSON.parse(result.stdout.toString()).status === 'succeeded';
      }, 'the first terminal run');

      const runJson = Bun.spawnSync(
        [cliPath, first.runId, '--logs', '--config', configPath, '--json'],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(runJson.exitCode, runJson.stderr.toString()).toBe(0);
      const runRecord = JSON.parse(runJson.stdout.toString());
      expect(runRecord).toMatchObject({
        profile: 'woml.run-presentation/v1',
        runId: first.runId,
        status: 'succeeded',
      });
      expect(runJson.stdout.toString()).not.toContain('\u001b[');

      const colorCommand = [
        shellQuote(cliPath),
        shellQuote(first.runId),
        '--logs',
        '--state',
        shellQuote(statePath),
        '--color=always',
      ].join(' ');
      const colored = Bun.spawnSync(
        [scriptPath!, '-qefc', colorCommand, '/dev/null'],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(colored.exitCode, colored.stderr.toString()).toBe(0);
      expect(colored.stdout.toString()).toContain('\u001b[');
      expect(colored.stdout.toString()).toContain('RUN COMPLETED');

      const follower = Bun.spawn(
        [
          cliPath,
          'webhook-demo',
          '--logs',
          '--state',
          statePath,
          '--color=never',
        ],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      let followerStdout = '';
      let followerStderr = '';
      const stdoutDone = collect(follower.stdout, text => { followerStdout += text; });
      const stderrDone = collect(follower.stderr, text => { followerStderr += text; });
      await waitUntil(
        () => followerStderr.includes('Following workflow webhook-demo'),
        'the workflow log attachment',
        () => `${followerStdout}\n${followerStderr}`
      );
      expect(followerStdout).toContain('order-logs-first');

      const second = await invoke('logs-second', 'order-logs-second');
      await waitUntil(
        () =>
          followerStdout.includes(second.runId) &&
          followerStdout.includes('Received order order-logs-second') &&
          followerStdout.includes('RUN COMPLETED'),
        'the live second run presentation',
        () => `${followerStdout}\n${followerStderr}`
      );

      follower.kill('SIGINT');
      expect(await follower.exited).toBe(0);
      await Promise.all([stdoutDone, stderrDone]);
      expect(followerStderr).not.toContain('WOML automation stopped');

      const descriptor = await readRuntimeDescriptor(descriptorFile);
      const health = await fetch(`${descriptor.adminUrl}/readyz`);
      expect(health.status).toBe(200);
      expect((await health.json()) as { status: string }).toMatchObject({ status: 'ok' });
    } finally {
      const stopped = Bun.spawnSync(
        [cliPath, 'stop', '--state', statePath],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      if (stopped.exitCode !== 0 && (await Bun.file(descriptorFile).exists())) {
        throw new Error(`Could not stop background runtime:\n${stopped.stderr.toString()}`);
      }
    }
  }, 60_000);
});
