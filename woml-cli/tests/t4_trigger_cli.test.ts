import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const webhookWorkflow = join(projectRoot, 'examples', 'webhookWorkflow.woml');
const helloWorkflow = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'hello.woml'
);
let temporaryDirectory: string;

async function availablePort(): Promise<number> {
  const probe = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = probe.port;
  probe.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a test port.');
  return port;
}

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the T4 CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-t4-cli-'));
}, 120_000);

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('T4 long-lived WOML runtime', () => {
  test('woml run keeps a manual-only workflow active after its startup run', async () => {
    const child = Bun.spawn([cliPath, 'run', helloWorkflow], {
      cwd: projectRoot,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const stdout = new Response(child.stdout).text();
    let stderr = '';
    const stderrDone = (async () => {
      const reader = child.stderr.getReader();
      const decoder = new TextDecoder();
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        stderr += decoder.decode(chunk.value, { stream: true });
      }
      stderr += decoder.decode();
    })();
    const deadline = Date.now() + 10_000;
    while (!stderr.includes('WOML automation is active.')) {
      if (Date.now() >= deadline) throw new Error(stderr);
      await Bun.sleep(10);
    }
    expect(child.exitCode).toBeNull();
    child.kill('SIGINT');
    expect(await child.exited).toBe(0);
    expect(await stdout).toBe('{"message":"Hello World"}\n');
    await stderrDone;
    expect(stderr).toContain('WOML automation stopped.');
  });

  test(
    'woml run remains active for multiple webhook runs and stops gracefully',
    async () => {
      const port = await availablePort();
      const statePath = join(temporaryDirectory, 'webhook.sqlite');
      const child = Bun.spawn(
        [
          cliPath,
          'run',
          webhookWorkflow,
          '--host',
          '127.0.0.1',
          '--port',
          String(port),
          '--state',
          statePath,
        ],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      let stderr = '';
      const stderrDone = (async () => {
        const reader = child.stderr.getReader();
        const decoder = new TextDecoder();
        while (true) {
          const chunk = await reader.read();
          if (chunk.done) break;
          stderr += decoder.decode(chunk.value, { stream: true });
        }
        stderr += decoder.decode();
      })();
      const waitFor = async (needle: string): Promise<void> => {
        const deadline = Date.now() + 10_000;
        while (!stderr.includes(needle)) {
          if (Date.now() >= deadline) {
            throw new Error(`Timed out waiting for ${needle}:\n${stderr}`);
          }
          await Bun.sleep(10);
        }
      };

      await waitFor('WOML automation is active. Press Ctrl+C to stop.');
      const invoke = async (key: string, orderId: string) => {
        const response = await fetch(
          `http://127.0.0.1:${port}/webhooks/orders`,
          {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              'Idempotency-Key': key,
            },
            body: JSON.stringify({ orderId }),
          }
        );
        expect(response.status).toBe(202);
        return (await response.json()) as {
          runId: string;
          status: 'accepted';
          duplicate: boolean;
        };
      };

      const rejected = await fetch(
        `http://127.0.0.1:${port}/webhooks/orders`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ privateValue: 'must-not-appear' }),
        }
      );
      expect(rejected.status).toBe(400);
      await waitFor('Rejected trigger.webhook "newOrder" [WOML_TRIGGER_SCHEMA_INVALID]');
      expect(stderr).not.toContain('must-not-appear');

      const first = await invoke('t4-first', 'order-1');
      await waitFor(`Run ${first.runId} succeeded.`);
      await waitFor(
        `Run ${first.runId} result: {"message":"Received order order-1"}`
      );
      const second = await invoke('t4-second', 'order-2');
      await waitFor(`Run ${second.runId} succeeded.`);
      await waitFor(
        `Run ${second.runId} result: {"message":"Received order order-2"}`
      );
      expect(first.runId).not.toBe(second.runId);
      const duplicate = await invoke('t4-second', 'order-2');
      expect(duplicate).toEqual({
        runId: second.runId,
        status: 'accepted',
        duplicate: true,
      });
      await waitFor(
        `Recognized duplicate trigger.webhook "newOrder" for workflow "webhook-demo": ${second.runId}.`
      );
      expect(stderr).toContain(`Try webhook newOrder:\ncurl --request POST`);
      expect(stderr).toContain(
        `--data '{"orderId":"example"}'`
      );
      expect(child.exitCode).toBeNull();

      const inspection = Bun.spawnSync(
        [cliPath, 'get', second.runId, '--state', statePath, '--json'],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(inspection.exitCode).toBe(0);
      expect(JSON.parse(inspection.stdout.toString())).toMatchObject({
        runId: second.runId,
        workflowId: 'webhook-demo',
        status: 'succeeded',
        businessOutcome: 'succeeded',
        lifecycleStatus: 'completed',
      });
      const missingInspection = Bun.spawnSync(
        [cliPath, 'get', 'run_missing', '--state', statePath],
        { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
      );
      expect(missingInspection.exitCode).toBe(1);
      expect(missingInspection.stderr.toString()).toContain(
        'WOML run error [WOML_RUN_NOT_FOUND]'
      );

      child.kill('SIGINT');
      expect(await child.exited).toBe(0);
      await stderrDone;
      expect(stderr).toContain('WOML automation stopped.');
    },
    30_000
  );

  test('woml test executes one manual occurrence and exits', () => {
    const result = Bun.spawnSync([cliPath, 'test', helloWorkflow], {
      cwd: projectRoot,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(result.exitCode).toBe(0);
    expect(result.stdout.toString()).toBe('{"message":"Hello World"}\n');
    expect(result.stderr.toString()).toBe('');
  });
});
