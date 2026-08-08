import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import {
  SharedSlackTransport,
  type SharedSlackTransportOptions,
  type SlackSocket,
} from '../src/notification-provider';
import type { SecretStore } from '../src/secrets';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const workflowPath = resolve(
  projectRoot,
  'examples/slackTriggerWorkflow.woml'
);
const packagedCliPath = resolve(packageRoot, 'dist/cli.js');
const stagedNativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeCorePath =
  process.env.WOML_RUST_CORE_PATH ??
  (existsSync(stagedNativeCorePath) ? stagedNativeCorePath : undefined);
const nativeTest = nativeCorePath === undefined ? test.skip : test;

class MockSocket implements SlackSocket {
  readyState = 0;
  readonly sent: string[] = [];
  readonly #listeners = new Map<string, Array<(event: never) => void>>();

  constructor() {
    queueMicrotask(() => {
      this.readyState = 1;
      this.#emit('open', {});
    });
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.#emit('close', {});
  }

  addEventListener(type: string, listener: (event: never) => void): void {
    const listeners = this.#listeners.get(type) ?? [];
    listeners.push(listener);
    this.#listeners.set(type, listeners);
  }

  receive(value: unknown): void {
    this.#emit('message', { data: JSON.stringify(value) });
  }

  #emit(type: string, event: unknown): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener(event as never);
    }
  }
}

function secrets(): SecretStore {
  const values = new Map([
    ['SLACK_BOT_TOKEN', 'xoxb-cli-test-token'],
    ['SLACK_APP_TOKEN', 'xapp-cli-test-token'],
  ]);
  return {
    provider: 'environment',
    get: async name => values.get(name),
    has: async name => values.has(name),
    list: async () => [],
    set: async () => {
      throw new Error('read only');
    },
    delete: async () => false,
  };
}

function mockSlack(
  options: SharedSlackTransportOptions,
  capture: (socket: MockSocket) => void
): SharedSlackTransport {
  return new SharedSlackTransport({
    ...options,
    fetch: (async input => {
      const method = new URL(String(input)).pathname.split('/').pop();
      const body =
        method === 'apps.connections.open'
          ? { ok: true, url: 'wss://wss.slack.test/cli' }
          : method === 'auth.test'
            ? { ok: true, team_id: 'T12345678', user_id: 'U87654321' }
            : method === 'conversations.list'
              ? {
                  ok: true,
                  channels: [
                    { id: 'C12345678', name: 'woml-testing' },
                    { id: 'C22222222', name: 'agent-support' },
                  ],
                  response_metadata: { next_cursor: '' },
                }
              : { ok: false, error: 'unexpected_method' };
      return new Response(JSON.stringify(body));
    }) as typeof fetch,
    createWebSocket: () => {
      const socket = new MockSocket();
      capture(socket);
      return socket;
    },
  });
}

async function waitFor(
  value: () => string,
  needle: string
): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!value().includes(needle)) {
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${needle}:\n${value()}`);
    }
    await Bun.sleep(10);
  }
}

describe('T7 woml run Slack product journey', () => {
  nativeTest(
    'stays active, explains readiness, and reports the durable Slack result',
    async () => {
      const temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-t7-cli-'));
      const statePath = join(temporaryDirectory, 'state.sqlite');
      let socket: MockSocket | undefined;
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

      try {
        const exitCode = await runCli(
          ['run', workflowPath, '--state', statePath],
          io,
          {
            nativeCorePath,
            createSecretStore: secrets,
            readSecret: async () => '',
            createSlackTransport: options =>
              mockSlack(options, value => {
                socket = value;
              }),
            waitForShutdown: async () => {
              socket!.receive({
                envelope_id: 'env_cli_001',
                type: 'events_api',
                payload: {
                  type: 'event_callback',
                  event_id: 'EvCli001',
                  team_id: 'T12345678',
                  event: {
                    type: 'app_mention',
                    user: 'U12345678',
                    channel: 'C12345678',
                    text: '<@U87654321> hello WOML',
                    ts: '1710000010.000100',
                  },
                },
              });
              await waitFor(
                () => stderr,
                'result: {"type":"app-mention"'
              );
            },
          }
        );

        if (exitCode !== 0) throw new Error(stderr);
        expect(exitCode).toBe(0);
        expect(stdout).toBe('');
        expect(stderr).toContain('WOML Slack trigger host is ready.');
        expect(stderr).toContain(
          'Slack workspace T12345678 is ready for triggers.'
        );
        expect(stderr).toContain(
          'Received Slack app-mention EvCli001 for trigger "agentMessage".'
        );
        expect(stderr).toContain(
          'Accepted trigger.slack "agentMessage" for workflow "slack-trigger-contract"'
        );
        expect(stderr).toContain('Run run_');
        expect(stderr).toContain('succeeded.');
        expect(stderr).toContain('WOML automation stopped.');
        expect(socket!.sent).toEqual([
          JSON.stringify({ envelope_id: 'env_cli_001' }),
        ]);
      } finally {
        await rm(temporaryDirectory, { recursive: true, force: true });
      }
    },
    30_000
  );

  nativeTest(
    'ships Slack trigger activation in the clean packaged CLI without embedding credentials',
    async () => {
      expect(existsSync(packagedCliPath)).toBe(true);
      const temporaryDirectory = await mkdtemp(
        join(tmpdir(), 'woml-t7-package-')
      );
      try {
        const child = Bun.spawn(
          [
            packagedCliPath,
            'run',
            workflowPath,
            '--state',
            join(temporaryDirectory, 'state.sqlite'),
          ],
          {
            cwd: projectRoot,
            env: {
              ...process.env,
              WOML_SECRETS_PROVIDER: 'env',
              WOML_SECRET_SLACK_BOT_TOKEN: '',
              WOML_SECRET_SLACK_APP_TOKEN: '',
            },
            stdout: 'pipe',
            stderr: 'pipe',
          }
        );
        const [stdout, stderr, exitCode] = await Promise.all([
          new Response(child.stdout).text(),
          new Response(child.stderr).text(),
          child.exited,
        ]);
        expect(exitCode).toBe(1);
        expect(stdout).toBe('');
        expect(stderr).toContain('WOML_SECRET_NOT_FOUND');
        expect(stderr).toContain('SLACK_BOT_TOKEN');
        expect(stderr).not.toContain('xoxb-');
        expect(stderr).not.toContain('xapp-');
      } finally {
        await rm(temporaryDirectory, { recursive: true, force: true });
      }
    },
    30_000
  );
});
