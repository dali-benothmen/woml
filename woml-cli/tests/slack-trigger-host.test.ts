import { describe, expect, test } from 'bun:test';

import { SharedSlackTransport, type SlackSocket } from '../src/notification-provider';
import type { SecretStore } from '../src/secrets';
import {
  SlackTriggerHost,
  slackTriggerStartupError,
  type SlackTriggerProtocolMessage,
  type SlackTriggerRegistration,
} from '../src/slack-trigger';
import type {
  TriggerIngressAdmit,
  TriggerIngressOutcome,
} from '../src/rust-executor';

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
    ['SLACK_BOT_TOKEN', 'xoxb-trigger-test-token'],
    ['SLACK_APP_TOKEN', 'xapp-trigger-test-token'],
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
        ? { ok: true, url: 'wss://wss.slack.test/link' }
        : method === 'auth.test'
          ? {
              ok: true,
              team_id: 'T12345678',
              user_id: 'U87654321',
            }
          : method === 'conversations.list'
            ? {
                ok: true,
                channels: [
                  { id: 'C12345678', name: 'woml-testing' },
                  { id: 'C99999999', name: 'other' },
                ],
                response_metadata: { next_cursor: '' },
              }
            : { ok: false, error: 'unexpected_method' };
    return new Response(JSON.stringify(body));
  }) as typeof fetch;
}

function registration(
  overrides: Partial<SlackTriggerRegistration> = {}
): SlackTriggerRegistration {
  return {
    workflowId: 'slack-trigger-contract',
    definitionHash:
      'sha256:076bc37ede8c1a05c38944a82bf55bc11cbff2409ff96dc27c8b3d5d88c33363',
    triggerId: 'agentMessage',
    events: ['app-mention', 'direct-message'],
    channels: ['woml-testing'],
    credentialNames: {
      botToken: 'SLACK_BOT_TOKEN',
      appToken: 'SLACK_APP_TOKEN',
    },
    ...overrides,
  };
}

function eventEnvelope(
  eventId: string,
  event: Record<string, unknown>
): Record<string, unknown> {
  return {
    envelope_id: `env_${eventId}`,
    type: 'events_api',
    payload: {
      type: 'event_callback',
      event_id: eventId,
      team_id: 'T12345678',
      event,
    },
  };
}

function accepted(
  request: TriggerIngressAdmit,
  duplicate = false
): TriggerIngressOutcome {
  return {
    contract: 'woml.trigger-ingress',
    contractVersion: 1,
    messageType: 'accepted',
    requestId: request.requestId,
    occurrenceId: 'occ_slack_001',
    runId: 'run_slack_001',
    duplicate,
  };
}

describe('T7 Slack trigger host', () => {
  test('waits for durable admission before acknowledging and preserves the normalized payload', async () => {
    const sockets: MockSocket[] = [];
    const messages: SlackTriggerProtocolMessage[] = [];
    const requests: TriggerIngressAdmit[] = [];
    let release: ((outcome: TriggerIngressOutcome) => void) | undefined;
    const pending = new Promise<TriggerIngressOutcome>(resolve => {
      release = resolve;
    });
    const transport = new SharedSlackTransport({
      fetch: slackFetch(),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const host = new SlackTriggerHost({
      registrations: [registration()],
      secretStore: secrets(),
      transport,
      submit: request => {
        requests.push(request);
        return pending;
      },
      emit: message => {
        messages.push(message);
      },
      now: () => new Date('2026-08-08T12:00:00.000Z'),
    });
    await host.start();

    sockets[0]!.receive(
      eventEnvelope('Ev001', {
        type: 'app_mention',
        user: 'U12345678',
        channel: 'C12345678',
        text: '<@U87654321> summarize order 42',
        ts: '1710000000.000100',
      })
    );
    await Bun.sleep(0);
    expect(requests).toHaveLength(1);
    expect(sockets[0]!.sent).toEqual([]);
    expect(requests[0]).toMatchObject({
      workflowId: 'slack-trigger-contract',
      triggerId: 'agentMessage',
      triggerHandler: 'trigger.slack',
      sourceIdentity:
        'slack:T12345678:Ev001:slack-trigger-contract:agentMessage',
      payload: {
        type: 'app-mention',
        text: '<@U87654321> summarize order 42',
        userId: 'U12345678',
        channelId: 'C12345678',
        messageTs: '1710000000.000100',
        threadTs: '1710000000.000100',
        teamId: 'T12345678',
      },
    });

    release!(accepted(requests[0]!));
    await Bun.sleep(0);
    expect(JSON.parse(sockets[0]!.sent[0]!)).toEqual({
      envelope_id: 'env_Ev001',
    });
    expect(messages.some(message => message.messageType === 'event')).toBe(true);
    expect(
      messages.some(message => message.messageType === 'acknowledge')
    ).toBe(true);
    await host.close();
    await transport.close();
  });

  test('deduplicates redelivery in Rust, ignores bot loops, and applies mention channel filters', async () => {
    const sockets: MockSocket[] = [];
    const requests: TriggerIngressAdmit[] = [];
    const diagnostics: string[] = [];
    const transport = new SharedSlackTransport({
      fetch: slackFetch(),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const host = new SlackTriggerHost({
      registrations: [registration()],
      secretStore: secrets(),
      transport,
      submit: async request => {
        requests.push(request);
        return accepted(request, requests.length > 1);
      },
      diagnostic: message => {
        diagnostics.push(message);
      },
    });
    await host.start();
    const mention = eventEnvelope('EvRepeat', {
      type: 'app_mention',
      user: 'U12345678',
      channel: 'C12345678',
      text: 'hello',
      ts: '1710000001.000100',
    });
    sockets[0]!.receive(mention);
    await Bun.sleep(0);
    sockets[0]!.receive(mention);
    await Bun.sleep(0);
    expect(requests).toHaveLength(2);
    expect(requests[0]!.sourceIdentity).toBe(requests[1]!.sourceIdentity);

    sockets[0]!.receive(
      eventEnvelope('EvBot', {
        type: 'app_mention',
        user: 'U87654321',
        channel: 'C12345678',
        text: 'self',
        ts: '1710000002.000100',
      })
    );
    sockets[0]!.receive(
      eventEnvelope('EvOtherChannel', {
        type: 'app_mention',
        user: 'U12345678',
        channel: 'C99999999',
        text: 'wrong channel',
        ts: '1710000003.000100',
      })
    );
    await Bun.sleep(0);
    expect(requests).toHaveLength(2);
    expect(sockets[0]!.sent).toHaveLength(4);
    expect(diagnostics).toContain(
      'Ignored Slack app-mention EvOtherChannel: no WOML trigger matched its workspace, event, or channel filters.'
    );
    await host.close();
    await transport.close();
  });

  test('accepts DMs regardless of mention channel filters and leaves failed admission unacknowledged', async () => {
    const sockets: MockSocket[] = [];
    const messages: SlackTriggerProtocolMessage[] = [];
    const transport = new SharedSlackTransport({
      fetch: slackFetch(),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const host = new SlackTriggerHost({
      registrations: [registration()],
      secretStore: secrets(),
      transport,
      submit: async request => ({
        contract: 'woml.trigger-ingress',
        contractVersion: 1,
        messageType: 'rejected',
        requestId: request.requestId,
        failure: {
          code: 'WOML_TRIGGER_UNAVAILABLE',
          message: 'temporarily unavailable',
          retryable: true,
        },
      }),
      emit: message => {
        messages.push(message);
      },
    });
    await host.start();
    sockets[0]!.receive(
      eventEnvelope('EvDm', {
        type: 'message',
        channel_type: 'im',
        user: 'U12345678',
        channel: 'D12345678',
        text: 'private request',
        ts: '1710000004.000100',
        thread_ts: '1710000000.000100',
      })
    );
    await Bun.sleep(0);
    expect(sockets[0]!.sent).toEqual([]);
    expect(messages.at(-1)).toMatchObject({
      messageType: 'failure',
      code: 'WOML_TRIGGER_UNAVAILABLE',
      retryable: true,
    });
    await host.close();
    await transport.close();
  });

  test('turns Slack scope failures into an actionable trigger startup error', async () => {
    const transport = new SharedSlackTransport({
      fetch: (async (input, init) => {
        const method = new URL(String(input)).pathname.split('/').pop();
        if (method === 'auth.test') {
          return new Response(
            JSON.stringify({
              ok: false,
              error: 'missing_scope',
              needed: 'channels:read',
              provided: 'chat:write',
            })
          );
        }
        return slackFetch()(input, init);
      }) as typeof fetch,
      createWebSocket: () => new MockSocket(),
    });
    const host = new SlackTriggerHost({
      registrations: [registration()],
      secretStore: secrets(),
      transport,
      submit: async request => accepted(request),
    });

    try {
      await host.start();
      throw new Error('Expected Slack trigger startup to fail.');
    } catch (error) {
      expect(slackTriggerStartupError(error)).toEqual({
        code: 'WOML_SLACK_TRIGGER_SCOPE_MISSING',
        message:
          'Slack operation auth.test needs additional app permissions. Missing scopes: channels:read. Granted scopes: chat:write. Add the missing Bot Token Scopes and reinstall the Slack app to the workspace.',
      });
    }
    await transport.close();
  });
});
