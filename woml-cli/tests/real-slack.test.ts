import { describe, expect, test } from 'bun:test';

import {
  RealSlackTransport,
  SlackTransportError,
  type DeliverMessage,
  type InteractionMessage,
  type ProviderMessageIdentity,
  type SlackSocket,
  type UpdateMessage,
} from '../src/notification-provider';

const fixtureDirectory = new URL(
  './fixtures/notification-provider/',
  import.meta.url
);

async function fixture<T>(name: string): Promise<T> {
  return (await Bun.file(new URL(name, fixtureDirectory)).json()) as T;
}

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

  addEventListener(
    type: string,
    listener: (event: never) => void
  ): void {
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

interface ApiCall {
  readonly method: string;
  readonly authorization: string | null;
  readonly body: Record<string, unknown>;
}

function slackFetch(
  calls: ApiCall[],
  handler: (call: ApiCall) => {
    readonly body: Record<string, unknown>;
    readonly status?: number;
    readonly headers?: Record<string, string>;
  }
): typeof fetch {
  return (async (input, init) => {
    const method = new URL(String(input)).pathname.split('/').pop()!;
    const headers = new Headers(init?.headers);
    const call: ApiCall = {
      method,
      authorization: headers.get('authorization'),
      body: JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>,
    };
    calls.push(call);
    const result = handler(call);
    return new Response(JSON.stringify(result.body), {
      status: result.status ?? 200,
      headers: result.headers,
    });
  }) as typeof fetch;
}

function successfulApi(call: ApiCall): {
  readonly body: Record<string, unknown>;
} {
  if (call.method === 'apps.connections.open') {
    return { body: { ok: true, url: 'wss://wss.slack.test/link/?ticket=secret' } };
  }
  if (call.method === 'auth.test') {
    return { body: { ok: true, team_id: 'T12345678' } };
  }
  if (call.method === 'conversations.list') {
    return {
      body: {
        ok: true,
        channels: [{ id: 'C12345678', name: 'approvals' }],
        response_metadata: { next_cursor: '' },
      },
    };
  }
  if (call.method === 'chat.postMessage') {
    return {
      body: { ok: true, channel: 'C12345678', ts: '1723024800.000001' },
    };
  }
  if (call.method === 'chat.update') {
    return { body: { ok: true } };
  }
  throw new Error(`Unexpected Slack method ${call.method}`);
}

async function delivery(): Promise<DeliverMessage> {
  return await fixture<DeliverMessage>('deliver.v1.json');
}

describe('N5 real Slack transport', () => {
  test('ships the reviewed Socket Mode app manifest with the required scopes', async () => {
    const manifest = await Bun.file(
      new URL('../slack/manifest.json', import.meta.url)
    ).json();
    expect(manifest.oauth_config.scopes.bot).toEqual([
      'chat:write',
      'chat:write.public',
      'channels:read',
      'groups:read',
    ]);
    expect(manifest.settings).toMatchObject({
      interactivity: { is_enabled: true },
      socket_mode_enabled: true,
    });
    expect(JSON.stringify(manifest)).not.toContain('xox');
  });

  test('sends Block Kit, acknowledges a native action, and disables the message', async () => {
    const calls: ApiCall[] = [];
    const sockets: MockSocket[] = [];
    const interactions: InteractionMessage[] = [];
    const transport = new RealSlackTransport({
      emit: message => {
        interactions.push(message);
      },
      fetch: slackFetch(calls, successfulApi),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const invocation = await delivery();
    const credentials = {
      botToken: 'xoxb-real-test-token',
      appToken: 'xapp-real-test-token',
    };

    await transport.ensureConnection('SLACK_APP_TOKEN', credentials.appToken);
    const providerMessage = await transport.deliver({
      invocation,
      credentials,
    });

    expect(providerMessage).toEqual({
      workspaceId: 'T12345678',
      channelId: 'C12345678',
      messageId: '1723024800.000001',
    });
    const post = calls.find(call => call.method === 'chat.postMessage')!;
    const blocks = post.body.blocks as Array<Record<string, unknown>>;
    const actions = blocks.find(block => block.type === 'actions')!;
    const approve = (actions.elements as Array<Record<string, unknown>>)[0]!;
    expect(post.body.channel).toBe('C12345678');
    expect(approve.action_id).toBe('woml_approval_approved');
    expect(JSON.parse(String(approve.value))).toMatchObject({
      deliveryId: invocation.deliveryId,
      decisionCapability: invocation.decisionCapability,
      decision: 'approved',
    });

    sockets[0]!.receive({
      envelope_id: 'env_action_01',
      type: 'interactive',
      payload: {
        type: 'block_actions',
        user: { id: 'U12345678' },
        actions: [
          {
            block_id: 'woml_approval_actions',
            action_id: 'woml_approval_approved',
            value: approve.value,
          },
        ],
      },
    });
    await Bun.sleep(0);
    expect(JSON.parse(sockets[0]!.sent[0]!)).toEqual({
      envelope_id: 'env_action_01',
    });
    expect(interactions).toHaveLength(1);
    expect(interactions[0]).toMatchObject({
      deliveryId: invocation.deliveryId,
      decision: 'approved',
      providerActorId: 'U12345678',
    });

    const update = await fixture<UpdateMessage>('update.v1.json');
    await transport.update({
      invocation: { ...update, providerMessage },
      credentials,
    });
    const updated = calls.find(call => call.method === 'chat.update')!;
    expect(updated.body).toMatchObject({
      channel: providerMessage.channelId,
      ts: providerMessage.messageId,
      text: 'Approved',
    });
    expect(JSON.stringify(updated.body)).not.toContain('button');
    expect(calls.every(call => !JSON.stringify(call.body).includes('xox'))).toBe(
      true
    );
    await transport.close();
  });

  test('reuses one Socket connection and one delivery for duplicate work', async () => {
    const calls: ApiCall[] = [];
    const sockets: MockSocket[] = [];
    const transport = new RealSlackTransport({
      emit: () => {},
      fetch: slackFetch(calls, successfulApi),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const invocation = await delivery();
    const credentials = {
      botToken: 'xoxb-real-test-token',
      appToken: 'xapp-real-test-token',
    };
    await Promise.all([
      transport.ensureConnection('SLACK_APP_TOKEN', credentials.appToken),
      transport.ensureConnection('SLACK_APP_TOKEN', credentials.appToken),
    ]);
    const first = transport.deliver({ invocation, credentials });
    const second = transport.deliver({ invocation, credentials });
    expect(await first).toEqual(await second);
    expect(sockets).toHaveLength(1);
    expect(calls.filter(call => call.method === 'apps.connections.open')).toHaveLength(1);
    expect(calls.filter(call => call.method === 'chat.postMessage')).toHaveLength(1);
    await transport.close();
  });

  test('classifies rate limits, permissions, expired tokens, and ambiguous sends', async () => {
    const invocation = await delivery();
    const credentials = {
      botToken: 'xoxb-real-test-token',
      appToken: 'xapp-real-test-token',
    };
    const make = (
      override: (call: ApiCall) =>
        | ReturnType<typeof successfulApi>
        | { body: Record<string, unknown>; status?: number; headers?: Record<string, string> }
    ) => {
      const transport = new RealSlackTransport({
        emit: () => {},
        fetch: slackFetch([], override),
        createWebSocket: () => new MockSocket(),
      });
      return transport;
    };

    const limited = make(call =>
      call.method === 'chat.postMessage'
        ? {
            body: { ok: false, error: 'ratelimited' },
            status: 429,
            headers: { 'Retry-After': '3' },
          }
        : successfulApi(call)
    );
    await limited.ensureConnection('SLACK_APP_TOKEN', credentials.appToken);
    await expect(limited.deliver({ invocation, credentials })).rejects.toMatchObject({
      failure: {
        kind: 'rate_limited',
        code: 'WOML_SLACK_RATE_LIMITED',
        retryable: true,
        retryAfterMs: 3_000,
      },
    });

    const denied = make(call =>
      call.method === 'conversations.list'
        ? { body: { ok: false, error: 'missing_scope' } }
        : successfulApi(call)
    );
    await denied.ensureConnection('SLACK_APP_TOKEN', credentials.appToken);
    await expect(denied.deliver({ invocation, credentials })).rejects.toMatchObject({
      failure: {
        kind: 'provider_auth_failed',
        code: 'WOML_SLACK_PERMISSION_DENIED',
        retryable: false,
      },
    });

    const expired = make(call =>
      call.method === 'auth.test'
        ? { body: { ok: false, error: 'token_expired' } }
        : successfulApi(call)
    );
    await expired.ensureConnection('SLACK_APP_TOKEN', credentials.appToken);
    await expect(expired.deliver({ invocation, credentials })).rejects.toMatchObject({
      failure: {
        kind: 'provider_auth_failed',
        code: 'WOML_SLACK_AUTH_FAILED',
        retryable: false,
      },
    });

    const ambiguous = new RealSlackTransport({
      emit: () => {},
      fetch: (async (input, init) => {
        const method = new URL(String(input)).pathname.split('/').pop();
        if (method === 'chat.postMessage') throw new Error('connection lost');
        return slackFetch([], successfulApi)(input, init);
      }) as typeof fetch,
      createWebSocket: () => new MockSocket(),
    });
    await ambiguous.ensureConnection('SLACK_APP_TOKEN', credentials.appToken);
    await expect(
      ambiguous.deliver({ invocation, credentials })
    ).rejects.toMatchObject({
      failure: {
        kind: 'delivery_ambiguous',
        code: 'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
        retryable: false,
      },
    });
  });

  test('refreshes Socket Mode and rejects in-process credential rotation', async () => {
    const calls: ApiCall[] = [];
    const sockets: MockSocket[] = [];
    const transport = new RealSlackTransport({
      emit: () => {},
      fetch: slackFetch(calls, successfulApi),
      createWebSocket: () => {
        const socket = new MockSocket();
        sockets.push(socket);
        return socket;
      },
      reconnectBaseDelayMs: 1,
    });
    await transport.ensureConnection('SLACK_APP_TOKEN', 'xapp-first-token');
    await expect(
      transport.ensureConnection('SLACK_APP_TOKEN', 'xapp-rotated-token')
    ).rejects.toBeInstanceOf(SlackTransportError);

    sockets[0]!.receive({ type: 'disconnect', reason: 'refresh_requested' });
    await Bun.sleep(10);
    expect(sockets.length).toBeGreaterThanOrEqual(2);
    expect(
      calls.filter(call => call.method === 'apps.connections.open').length
    ).toBeGreaterThanOrEqual(2);
    await transport.close();
  });

  test('rejects a message update bound to another Slack workspace', async () => {
    const transport = new RealSlackTransport({
      emit: () => {},
      fetch: slackFetch([], successfulApi),
      createWebSocket: () => new MockSocket(),
    });
    const update = await fixture<UpdateMessage>('update.v1.json');
    const foreign: ProviderMessageIdentity = {
      ...update.providerMessage,
      workspaceId: 'T87654321',
    };
    await transport.ensureConnection('SLACK_APP_TOKEN', 'xapp-real-test-token');
    await expect(
      transport.update({
        invocation: { ...update, providerMessage: foreign },
        credentials: {
          botToken: 'xoxb-real-test-token',
          appToken: 'xapp-real-test-token',
        },
      })
    ).rejects.toMatchObject({
      failure: {
        kind: 'update_failed',
        code: 'WOML_SLACK_UPDATE_FAILED',
        retryable: false,
      },
    });
    await transport.close();
  });
});
