import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

import { SharedSlackTransport, type SlackSocket } from '../src/notification-provider';
import type { SecretStore } from '../src/secrets';
import {
  SlackTriggerHost,
  slackTriggerRegistrations,
  type SlackTriggerProtocolMessage,
} from '../src/slack-trigger';
import {
  compiledDefinitionHash,
  inspectRunWithRust,
  startWebhookRuntimeWithRust,
  stopWebhookRuntimeWithRust,
  submitTriggerOccurrenceWithRust,
  type TriggerProgressV1,
} from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const stagedNativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeCorePath =
  process.env.WOML_RUST_CORE_PATH ??
  (existsSync(stagedNativeCorePath) ? stagedNativeCorePath : undefined);
const nativeTest = nativeCorePath === undefined ? test.skip : test;
const sourcePath = resolve(
  packageRoot,
  '../woml/tests/fixtures/triggers-slack.woml'
);

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

function secretStore(): SecretStore {
  const values = new Map([
    ['SLACK_BOT_TOKEN', 'xoxb-t7-test-token'],
    ['SLACK_APP_TOKEN', 'xapp-t7-test-token'],
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

function slackFetch(): typeof fetch {
  return (async input => {
    const method = new URL(String(input)).pathname.split('/').pop();
    const body =
      method === 'apps.connections.open'
        ? { ok: true, url: 'wss://wss.slack.test/t7' }
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
  }) as typeof fetch;
}

function mentionEnvelope(eventId: string): Record<string, unknown> {
  return {
    envelope_id: `env_${eventId}`,
    type: 'events_api',
    payload: {
      type: 'event_callback',
      event_id: eventId,
      team_id: 'T12345678',
      event: {
        type: 'app_mention',
        user: 'U12345678',
        channel: 'C12345678',
        text: '<@U87654321> process order 42',
        ts: '1710000000.000100',
        thread_ts: '1710000000.000050',
      },
    },
  };
}

async function waitUntil(
  predicate: () => boolean,
  description: string
): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${description}.`);
    }
    await Bun.sleep(10);
  }
}

describe('T7 Slack to durable Rust execution journey', () => {
  nativeTest(
    'admits before acknowledging, executes normalized context, and deduplicates redelivery',
    async () => {
      const temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-t7-'));
      const statePath = join(temporaryDirectory, 'state.sqlite');
      const source = await Bun.file(sourcePath).text();
      const workflow = compileWoml(parseWoml(source, { file: sourcePath }));
      const definitionHash = compiledDefinitionHash(workflow);
      const progress: TriggerProgressV1[] = [];
      const protocol: SlackTriggerProtocolMessage[] = [];
      const sockets: MockSocket[] = [];
      let runtimeId: string | undefined;
      let host: SlackTriggerHost | undefined;
      let transport: SharedSlackTransport | undefined;

      try {
        const runtime = await startWebhookRuntimeWithRust(
          [{ workflow, definitionHash, resolvedSecrets: {} }],
          statePath,
          {
            nativeCorePath,
            port: 0,
            onTriggerProgress: message => progress.push(message),
          }
        );
        runtimeId = runtime.runtimeId;
        transport = new SharedSlackTransport({
          fetch: slackFetch(),
          createWebSocket: () => {
            const socket = new MockSocket();
            sockets.push(socket);
            return socket;
          },
        });
        host = new SlackTriggerHost({
          registrations: slackTriggerRegistrations(workflow, definitionHash),
          secretStore: secretStore(),
          transport,
          submit: request =>
            submitTriggerOccurrenceWithRust(runtime.runtimeId, request, {
              nativeCorePath,
            }),
          emit: message => {
            protocol.push(message);
          },
          now: () => new Date('2026-08-08T12:00:00.000Z'),
        });
        await host.start();

        const envelope = mentionEnvelope('EvDurable001');
        sockets[0]!.receive(envelope);
        await waitUntil(
          () =>
            protocol.some(message => message.messageType === 'acknowledge'),
          'the durable Slack acknowledgement'
        );
        expect(sockets[0]!.sent).toEqual([
          JSON.stringify({ envelope_id: 'env_EvDurable001' }),
        ]);
        const firstAdmission = protocol.find(
          message => message.messageType === 'acknowledge'
        );
        expect(firstAdmission).toMatchObject({
          messageType: 'acknowledge',
          duplicate: false,
        });
        const runId =
          firstAdmission?.messageType === 'acknowledge'
            ? firstAdmission.runId
            : undefined;
        expect(runId).toBeString();
        await waitUntil(
          () =>
            progress.some(
              message =>
                message.type === 'run_terminal' && message.runId === runId
            ),
          'the Slack-triggered workflow result'
        );
        expect(inspectRunWithRust(statePath, runId!, { nativeCorePath })).toMatchObject({
          runId,
          workflowId: 'slack-trigger-contract',
          status: 'succeeded',
          result: {
            type: 'app-mention',
            text: '<@U87654321> process order 42',
            userId: 'U12345678',
            channelId: 'C12345678',
            messageTs: '1710000000.000100',
            threadTs: '1710000000.000050',
            teamId: 'T12345678',
          },
        });

        sockets[0]!.receive(envelope);
        await waitUntil(
          () =>
            protocol.filter(message => message.messageType === 'acknowledge')
              .length === 2,
          'the Slack redelivery acknowledgement'
        );
        const acknowledgements = protocol.filter(
          message => message.messageType === 'acknowledge'
        );
        expect(acknowledgements[1]).toMatchObject({
          messageType: 'acknowledge',
          runId,
          duplicate: true,
        });
        expect(
          progress.filter(message => message.type === 'run_started')
        ).toHaveLength(1);

        await host.close();
        host = undefined;
        await transport.close();
        transport = undefined;
        await stopWebhookRuntimeWithRust(runtimeId, { nativeCorePath });
        runtimeId = undefined;

        const restartProgress: TriggerProgressV1[] = [];
        const restarted = await startWebhookRuntimeWithRust(
          [{ workflow, definitionHash, resolvedSecrets: {} }],
          statePath,
          {
            nativeCorePath,
            port: 0,
            onTriggerProgress: message => restartProgress.push(message),
          }
        );
        runtimeId = restarted.runtimeId;
        transport = new SharedSlackTransport({
          fetch: slackFetch(),
          createWebSocket: () => {
            const socket = new MockSocket();
            sockets.push(socket);
            return socket;
          },
        });
        host = new SlackTriggerHost({
          registrations: slackTriggerRegistrations(workflow, definitionHash),
          secretStore: secretStore(),
          transport,
          submit: request =>
            submitTriggerOccurrenceWithRust(restarted.runtimeId, request, {
              nativeCorePath,
            }),
          emit: message => {
            protocol.push(message);
          },
        });
        await host.start();
        sockets[1]!.receive(envelope);
        await waitUntil(
          () =>
            protocol.filter(message => message.messageType === 'acknowledge')
              .length === 3,
          'the post-restart Slack redelivery acknowledgement'
        );
        expect(
          protocol.filter(message => message.messageType === 'acknowledge').at(-1)
        ).toMatchObject({
          messageType: 'acknowledge',
          runId,
          duplicate: true,
        });
        expect(
          restartProgress.filter(message => message.type === 'run_started')
        ).toHaveLength(0);
        expect(inspectRunWithRust(statePath, runId!, { nativeCorePath })).toMatchObject({
          runId,
          status: 'succeeded',
        });
      } finally {
        await host?.close();
        await transport?.close();
        if (runtimeId !== undefined) {
          await stopWebhookRuntimeWithRust(runtimeId, { nativeCorePath });
        }
        await rm(temporaryDirectory, { recursive: true, force: true });
      }
    },
    30_000
  );
});
