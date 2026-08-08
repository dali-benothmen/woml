import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  SharedSlackTransport,
  type SlackSocket,
} from '../src/notification-provider';
import { runCli, type CliIo } from '../src/cli';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const nativeCorePath = join(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const eventToken = 't13-event-token-never-print';
let temporaryDirectory: string;

class MockSocket implements SlackSocket {
  readyState = 0;
  readonly #listeners = new Map<string, Array<(event: never) => void>>();

  constructor() {
    queueMicrotask(() => {
      this.readyState = 1;
      this.#emit('open', {});
    });
  }

  send(): void {}

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

  #emit(type: string, event: unknown): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener(event as never);
    }
  }
}

function slackFetch(): typeof fetch {
  return (async input => {
    const method = new URL(String(input)).pathname.split('/').pop();
    const body =
      method === 'apps.connections.open'
        ? { ok: true, url: 'wss://wss.slack.test/t13' }
        : method === 'auth.test'
          ? { ok: true, team_id: 'T12345678', user_id: 'U87654321' }
          : method === 'conversations.list'
            ? {
                ok: true,
                channels: [{ id: 'C12345678', name: 'woml-testing' }],
                response_metadata: { next_cursor: '' },
              }
            : { ok: false, error: 'unexpected_method' };
    return new Response(JSON.stringify(body));
  }) as typeof fetch;
}

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

async function waitUntil(
  predicate: () => boolean,
  description: string
): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${description}.`);
    }
    await Bun.sleep(10);
  }
}

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the T13 CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-t13-cli-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('T13 complete Production Triggers runtime', () => {
  test(
    'activates webhook, Slack, schedule, interval, and event together',
    async () => {
      const workflowPath = join(temporaryDirectory, 'all-triggers.woml');
      const statePath = join(temporaryDirectory, 'all-triggers.sqlite');
      await writeFile(
        workflowPath,
        `<workflow id="all-production-triggers" name="All production triggers">
  <triggers>
    <webhook id="incoming" path="/t13/incoming" auth="none">
      <schema>{"type":"object","required":["source"],"properties":{"source":{"type":"string"}}}</schema>
    </webhook>
    <slack id="agentMessage" events="app-mention" channels="woml-testing" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
    <schedule id="annual" cron="0 0 1 1 *" timezone="UTC" on-missed="skip" />
    <interval id="maintenance" every="30d" on-missed="skip" />
    <event id="applicationEvent" name="t13.received" secret="{{secrets.EVENT_CONTROL_TOKEN}}">
      <schema>{"type":"object","required":["source"],"properties":{"source":{"type":"string"}}}</schema>
    </event>
  </triggers>
  <steps>
    <step id="capture">
      <script>return { source: context.trigger.source };</script>
    </step>
  </steps>
</workflow>`,
        'utf8'
      );

      const secrets = new Map([
        ['SLACK_BOT_TOKEN', 'xoxb-t13-test-token'],
        ['SLACK_APP_TOKEN', 'xapp-t13-test-token'],
        ['EVENT_CONTROL_TOKEN', eventToken],
      ]);
      let stderr = '';
      let stdout = '';
      let stop!: () => void;
      const shutdown = new Promise<void>(resolveShutdown => {
        stop = resolveShutdown;
      });
      const io: CliIo = {
        stdout: text => {
          stdout += text;
        },
        stderr: text => {
          stderr += text;
        },
      };
      const port = await availablePort();
      const running = runCli(
        [
          'run',
          workflowPath,
          '--host',
          '127.0.0.1',
          '--port',
          String(port),
          '--state',
          statePath,
        ],
        io,
        {
          nativeCorePath,
          createSecretStore: () => ({
            provider: 'environment',
            get: async name => secrets.get(name),
            has: async name => secrets.has(name),
            list: async () => [],
            set: async () => {},
            delete: async () => false,
          }),
          readSecret: async () => {
            throw new Error('T13 must resolve only authored secret references.');
          },
          createSlackTransport: options =>
            new SharedSlackTransport({
              ...options,
              fetch: slackFetch(),
              createWebSocket: () => new MockSocket(),
            }),
          waitForShutdown: () => shutdown,
        }
      );

      try {
        await waitUntil(
          () => stderr.includes('WOML automation is active.'),
          'mixed-trigger runtime readiness'
        );
        expect(stderr).toContain(
          'WOML runtime is ready with 5 registered triggers.'
        );
        expect(stderr).toContain(
          `Webhook incoming: POST http://127.0.0.1:${port}/t13/incoming`
        );
        expect(stderr).toContain(
          `Event t13.received: POST http://127.0.0.1:${port}/_woml/events/t13.received`
        );
        expect(stderr).toContain('Schedule annual (UTC) next due at');
        expect(stderr).toContain(
          'Interval maintenance every 2592000000ms next due at'
        );
        expect(stderr).toContain('Slack workspace T12345678 is ready for triggers.');

        const webhook = await fetch(
          `http://127.0.0.1:${port}/t13/incoming`,
          {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ source: 'webhook' }),
          }
        );
        expect(webhook.status).toBe(202);
        const webhookRun = (await webhook.json()) as { runId: string };
        await waitUntil(
          () => stderr.includes(`Run ${webhookRun.runId} result: {"source":"webhook"}`),
          'webhook-triggered result'
        );

        const event = await fetch(
          `http://127.0.0.1:${port}/_woml/events/t13.received`,
          {
            method: 'POST',
            headers: {
              Authorization: `Bearer ${eventToken}`,
              'Event-ID': 't13-event-1',
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({ source: 'event' }),
          }
        );
        expect(event.status).toBe(200);
        const publication = (await event.json()) as {
          deliveries: Array<{ runId?: string }>;
        };
        const eventRun = publication.deliveries[0]?.runId;
        expect(eventRun).toBeString();
        await waitUntil(
          () => stderr.includes(`Run ${eventRun} result: {"source":"event"}`),
          'event-triggered result'
        );

        expect(stderr).not.toContain(eventToken);
        expect(stderr).not.toContain('xoxb-t13-test-token');
        expect(stderr).not.toContain('xapp-t13-test-token');
        expect(stdout).toBe('');
      } finally {
        stop();
      }

      expect(await running).toBe(0);
      expect(stderr).toContain('WOML automation stopped.');
    },
    45_000
  );
});
