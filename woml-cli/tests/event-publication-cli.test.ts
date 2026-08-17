import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli } from '../src/cli';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const workflows = join(projectRoot, 'examples', 'events');
const payload = join(workflows, 'order-created.json');
const controlToken = 't12-cli-control-token-never-print';
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
      `Could not build the  CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-t12-cli-'));
}, 120_000);

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Event publication product journey', () => {
  test(
    'One published event starts both loaded workflows and woml emit uses the same endpoint',
    async () => {
      const port = await availablePort();
      const statePath = join(temporaryDirectory, 'events.sqlite');
      const environment = {
        ...process.env,
        WOML_SECRETS_PROVIDER: 'env',
        WOML_SECRET_EVENT_CONTROL_TOKEN: controlToken,
      };
      const child = Bun.spawn(
        [
          cliPath,
          'run',
          workflows,
          '--host',
          '127.0.0.1',
          '--port',
          String(port),
          '--state',
          statePath,
        ],
        {
          cwd: projectRoot,
          env: environment,
          stdout: 'pipe',
          stderr: 'pipe',
        }
      );
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
      const waitFor = async (needle: string): Promise<void> => {
        const deadline = Date.now() + 15_000;
        while (!stderr.includes(needle)) {
          if (Date.now() >= deadline) {
            throw new Error(`Timed out waiting for ${needle}:\n${stderr}`);
          }
          await Bun.sleep(10);
        }
      };

      await waitFor('WOML automation is active. Press Ctrl+C to stop.');
      expect(stderr).toContain('Event  order.created');
      expect(stderr).toContain(
        `POST   http://127.0.0.1:${port}/_woml/events/order.created`
      );
      expect(stderr).toContain('curl --request POST');
      expect(stderr).toContain("--header 'Event-ID: <event-id>'");
      expect(stderr).not.toContain(controlToken);

      const publish = async (eventId: string, orderId: string, token = controlToken) => {
        const response = await fetch(
          `http://127.0.0.1:${port}/_woml/events/order.created`,
          {
            method: 'POST',
            headers: {
              Authorization: `Bearer ${token}`,
              'Event-ID': eventId,
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({ orderId }),
          }
        );
        return {
          status: response.status,
          body: (await response.json()) as Record<string, any>,
        };
      };

      const first = await publish('order-cli-1', 'order-42');
      expect(first.status).toBe(200);
      expect(first.body.status).toBe('accepted');
      expect(first.body.deliveries.map((delivery: any) => delivery.workflowId)).toEqual([
        'send-confirmation',
        'update-inventory',
      ]);
      await waitFor('Confirmation sent for order-42');
      await waitFor('Inventory updated for order-42');
      for (const delivery of first.body.deliveries) {
        expect(stderr).toContain(`RUN  ${delivery.runId}`);
      }

      const duplicate = await publish('order-cli-1', 'order-42');
      expect(duplicate.body.deliveries.every((delivery: any) => delivery.duplicate)).toBe(
        true
      );
      expect(duplicate.body.deliveries.map((delivery: any) => delivery.runId)).toEqual(
        first.body.deliveries.map((delivery: any) => delivery.runId)
      );

      const conflict = await publish('order-cli-1', 'changed');
      expect(conflict.status).toBe(200);
      expect(conflict.body.status).toBe('rejected');
      expect(
        conflict.body.deliveries.every(
          (delivery: any) =>
            delivery.code === 'WOML_TRIGGER_IDEMPOTENCY_CONFLICT'
        )
      ).toBe(true);

      const unauthorized = await publish(
        'order-cli-unauthorized',
        'must-not-appear',
        'wrong-token'
      );
      expect(unauthorized.status).toBe(401);
      expect(unauthorized.body.error.code).toBe('WOML_EVENT_UNAUTHORIZED');
      expect(stderr).not.toContain('must-not-appear');

      const emitted = Bun.spawnSync(
        [
          cliPath,
          'emit',
          'order.created',
          '--id',
          'order-cli-emit-1',
          '--data',
          `@${payload}`,
          '--server',
          `http://127.0.0.1:${port}`,
          '--token-secret',
          'EVENT_CONTROL_TOKEN',
        ],
        {
          cwd: projectRoot,
          env: environment,
          stdout: 'pipe',
          stderr: 'pipe',
        }
      );
      expect(emitted.exitCode).toBe(0);
      const emittedBody = JSON.parse(emitted.stdout.toString());
      expect(emittedBody.status).toBe('accepted');
      expect(emittedBody.deliveries).toHaveLength(2);
      expect(emitted.stderr.toString()).not.toContain(controlToken);

      child.kill('SIGINT');
      expect(await child.exited).toBe(0);
      await stderrDone;
      expect(await stdout).toBe('');
      expect(stderr).toContain('WOML automation stopped.');
      expect(stderr).not.toContain(controlToken);
      const state = await readFile(statePath);
      expect(state.includes(Buffer.from(controlToken))).toBe(false);
    },
    45_000
  );

  test('resolves the publisher secret selected by --token-secret', async () => {
    let requestedSecret = '';
    let authorization = '';
    let stdout = '';
    const exitCode = await runCli(
      [
        'emit',
        'order.created',
        '--id',
        'custom-secret-1',
        '--data',
        `@${payload}`,
        '--server',
        'http://127.0.0.1:3000',
        '--token-secret',
        'CUSTOM_EVENT_TOKEN',
      ],
      {
        stdout: text => {
          stdout += text;
        },
        stderr: () => {},
      },
      {
        createSecretStore: () => ({
          provider: 'environment',
          get: async name => {
            requestedSecret = name;
            return 'custom-control-token';
          },
          has: async () => true,
          list: async () => [],
          set: async () => {},
          delete: async () => false,
        }),
        readSecret: async () => '',
        fetch: async (_input, init) => {
          authorization = new Headers(init?.headers).get('Authorization') ?? '';
          return new Response(
            JSON.stringify({
              eventId: 'custom-secret-1',
              eventName: 'order.created',
              status: 'accepted',
              deliveries: [
                {
                  workflowId: 'send-confirmation',
                  triggerId: 'orderCreated',
                  status: 'accepted',
                  runId: 'run_custom',
                  duplicate: false,
                },
              ],
            }),
            { status: 200, headers: { 'Content-Type': 'application/json' } }
          );
        },
      }
    );

    expect(exitCode).toBe(0);
    expect(requestedSecret).toBe('CUSTOM_EVENT_TOKEN');
    expect(authorization).toBe('Bearer custom-control-token');
    expect(stdout).not.toContain('custom-control-token');
  });
});
