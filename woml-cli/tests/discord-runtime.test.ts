import { describe, expect, test } from 'bun:test';

import {
  DiscordNotificationAdapter,
  DiscordTriggerHost,
  SharedDiscordTransport,
  type DiscordGatewaySocket,
} from '../src/discord';
import type { SecretStore } from '../src/secrets';
import type { TriggerIngressAdmit } from '../src/rust-executor';

const BOT_ID = '123456789012345678';
const USER_ID = '234567890123456789';
const CHANNEL_ID = '345678901234567890';
const MESSAGE_ID = '456789012345678901';
const BOT_TOKEN = 'discord-test-token';
const BOT_SECRET = 'DISCORD_BOT_TOKEN';

function secrets(): SecretStore {
  const values = new Map([[BOT_SECRET, BOT_TOKEN]]);
  return {
    provider: 'environment',
    get: async name => values.get(name),
    has: async name => values.has(name),
    list: async () => [],
    set: async () => { throw new Error('read only'); },
    delete: async () => false,
  };
}

async function eventually(assertion: () => void): Promise<void> {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    try {
      assertion();
      return;
    } catch {
      await Bun.sleep(1);
    }
  }
  assertion();
}

class FakeGatewaySocket implements DiscordGatewaySocket {
  readonly sent: unknown[] = [];
  readyState = 1;
  binaryType = 'arraybuffer';
  onopen: ((event: unknown) => void) | null = null;
  onmessage: ((event: { readonly data: unknown }) => void) | null = null;
  onclose: ((event: { readonly code: number; readonly reason?: string }) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  close(code = 1000, reason = ''): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }

  message(value: unknown): void {
    this.onmessage?.({ data: JSON.stringify(value) });
  }
}

function identityResponse(): Response {
  return Response.json({ id: BOT_ID, username: 'woml-bot', bot: true });
}

function hello(socket: FakeGatewaySocket): void {
  socket.message({ op: 10, d: { heartbeat_interval: 60_000 } });
}

function ready(socket: FakeGatewaySocket, sequence = 1): void {
  socket.message({
    op: 0,
    t: 'READY',
    s: sequence,
    d: {
      session_id: 'discord-session-1',
      resume_gateway_url: 'wss://resume.discord.test',
      user: { id: BOT_ID },
    },
  });
}

describe('Discord production runtime', () => {
  test('identifies once and durably admits app mentions with exact source identity', async () => {
    const sockets: FakeGatewaySocket[] = [];
    const requests: TriggerIngressAdmit[] = [];
    const transport = new SharedDiscordTransport({
      fetch: (async () => identityResponse()) as unknown as typeof fetch,
      createWebSocket: () => {
        const socket = new FakeGatewaySocket();
        sockets.push(socket);
        return socket;
      },
    });
    const host = new DiscordTriggerHost({
      registrations: [{
        workflowId: 'discord-agent',
        definitionHash: `sha256:${'a'.repeat(64)}`,
        triggerId: 'agentMessage',
        events: ['app-mention'],
        channels: [CHANNEL_ID],
        credentialNames: { botToken: BOT_SECRET },
      }],
      secretStore: secrets(),
      transport,
      submit: async request => {
        requests.push(request);
        return {
          contract: 'woml.trigger-ingress',
          contractVersion: 1,
          messageType: 'accepted',
          requestId: request.requestId,
          occurrenceId: 'occ_discord_message',
          runId: 'run_discord_message',
          duplicate: false,
        };
      },
    });

    const started = host.start();
    await eventually(() => expect(sockets).toHaveLength(1));
    hello(sockets[0]!);
    await eventually(() =>
      expect(sockets[0]!.sent.some(message => (message as { op: number }).op === 2)).toBe(true)
    );
    ready(sockets[0]!);
    await started;
    sockets[0]!.message({
      op: 0,
      t: 'MESSAGE_CREATE',
      s: 2,
      d: {
        id: MESSAGE_ID,
        channel_id: CHANNEL_ID,
        guild_id: '567890123456789012',
        content: `<@${BOT_ID}> process order 42`,
        timestamp: '2026-08-16T12:00:00.000Z',
        author: { id: USER_ID, username: 'dali', bot: false },
        mentions: [{ id: BOT_ID }],
      },
    });

    await eventually(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toMatchObject({
      workflowId: 'discord-agent',
      triggerId: 'agentMessage',
      triggerHandler: 'trigger.discord',
      sourceIdentity: `discord:${BOT_ID}:${MESSAGE_ID}:discord-agent:agentMessage`,
      payload: {
        provider: 'discord',
        event: 'app-mention',
        text: `<@${BOT_ID}> process order 42`,
        senderId: USER_ID,
        senderName: 'dali',
        conversationId: CHANNEL_ID,
        conversationType: 'group',
        messageId: MESSAGE_ID,
      },
    });
    await host.close();
    await transport.close();
  });

  test('resumes the shared Gateway session after a recoverable disconnect', async () => {
    const sockets: FakeGatewaySocket[] = [];
    const transport = new SharedDiscordTransport({
      fetch: (async () => identityResponse()) as unknown as typeof fetch,
      createWebSocket: () => {
        const socket = new FakeGatewaySocket();
        sockets.push(socket);
        return socket;
      },
      reconnectDelayMs: 1,
    });
    const connecting = transport.ensureConnected(BOT_SECRET, BOT_TOKEN);
    await eventually(() => expect(sockets).toHaveLength(1));
    hello(sockets[0]!);
    ready(sockets[0]!, 9);
    await connecting;
    sockets[0]!.message({ op: 11, d: null });
    sockets[0]!.close(4000, 'test reconnect');
    await eventually(() => expect(sockets).toHaveLength(2));
    hello(sockets[1]!);
    await eventually(() =>
      expect(sockets[1]!.sent.some(message => (message as { op: number }).op === 6)).toBe(true)
    );
    sockets[1]!.message({ op: 0, t: 'RESUMED', s: 10, d: {} });
    await transport.close();
  });

  test('reports missing privileged intents as a terminal setup failure', async () => {
    const sockets: FakeGatewaySocket[] = [];
    const transport = new SharedDiscordTransport({
      fetch: (async () => identityResponse()) as unknown as typeof fetch,
      createWebSocket: () => {
        const socket = new FakeGatewaySocket();
        sockets.push(socket);
        return socket;
      },
    });
    const connecting = transport.ensureConnected(BOT_SECRET, BOT_TOKEN);
    await eventually(() => expect(sockets).toHaveLength(1));
    sockets[0]!.close(4014, 'Disallowed intent');
    await expect(connecting).rejects.toMatchObject({
      failure: {
        kind: 'permission_denied',
        code: 'WOML_DISCORD_INTENTS_MISSING',
        retryable: false,
      },
    });
    await transport.close();
  });

  test('preserves Discord retry timing without retrying an ambiguous send itself', async () => {
    const transport = new SharedDiscordTransport({
      fetch: (async () =>
        Response.json(
          { message: 'rate limited', retry_after: 1.25 },
          { status: 429 }
        )) as unknown as typeof fetch,
    });
    await expect(transport.sendMessage({
      botToken: BOT_TOKEN,
      accountId: BOT_ID,
      conversationId: CHANNEL_ID,
      text: 'Hello',
    })).rejects.toMatchObject({
      failure: {
        kind: 'rate_limited',
        code: 'WOML_DISCORD_RATE_LIMITED',
        retryable: true,
        retryAfterMs: 1_250,
      },
    });
    await transport.close();
  });

  test('durably resolves an approval before acknowledging its Discord interaction', async () => {
    const sockets: FakeGatewaySocket[] = [];
    const calls: string[] = [];
    const fakeFetch = (async (input: RequestInfo | URL) => {
      const path = new URL(String(input)).pathname;
      if (path.endsWith('/users/@me')) return identityResponse();
      if (path.includes('/interactions/')) {
        calls.push('interaction-acknowledged');
        return new Response(undefined, { status: 204 });
      }
      throw new Error(`Unexpected Discord path ${path}`);
    }) as typeof fetch;
    const transport = new SharedDiscordTransport({
      fetch: fakeFetch,
      createWebSocket: () => {
        const socket = new FakeGatewaySocket();
        sockets.push(socket);
        return socket;
      },
    });
    const host = new DiscordTriggerHost({
      registrations: [],
      credentialNames: [BOT_SECRET],
      secretStore: secrets(),
      transport,
      submit: async () => { throw new Error('No trigger should be admitted.'); },
      resolveApproval: async (update): Promise<'accepted'> => {
        calls.push(`approval-${update.decision}-${update.actorId}`);
        return 'accepted';
      },
    });
    const started = host.start();
    await eventually(() => expect(sockets).toHaveLength(1));
    hello(sockets[0]!);
    ready(sockets[0]!);
    await started;
    sockets[0]!.message({
      op: 0,
      t: 'INTERACTION_CREATE',
      s: 2,
      d: {
        type: 3,
        id: '678901234567890123',
        token: 'temporary-interaction-token',
        member: { user: { id: USER_ID } },
        data: { custom_id: `a:ncap_${'a'.repeat(16)}.${'b'.repeat(32)}` },
      },
    });
    await eventually(() => expect(calls).toHaveLength(2));
    expect(calls).toEqual([
      `approval-approved-${USER_ID}`,
      'interaction-acknowledged',
    ]);
    await host.close();
    await transport.close();
  });

  test('sends approval buttons once and removes them after convergence', async () => {
    const requests: Array<{ path: string; method: string; body: Record<string, unknown> }> = [];
    const fakeFetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input)).pathname;
      const method = init?.method ?? 'GET';
      const body = init?.body === undefined
        ? {}
        : JSON.parse(String(init.body)) as Record<string, unknown>;
      requests.push({ path, method, body });
      if (path.endsWith('/users/@me')) return identityResponse();
      if (method === 'POST') {
        return Response.json({ id: MESSAGE_ID, channel_id: CHANNEL_ID });
      }
      return Response.json({ id: MESSAGE_ID, channel_id: CHANNEL_ID });
    }) as typeof fetch;
    const transport = new SharedDiscordTransport({ fetch: fakeFetch });
    const adapter = new DiscordNotificationAdapter(transport);
    const invocation = {
      protocol: 'woml.notification-provider-host',
      protocolVersion: 1,
      messageType: 'deliver',
      invocationId: 'discord-delivery-1',
      runId: 'run-discord-1',
      approvalId: 'reviewOrder',
      requestId: 'aprreq_discord_1',
      deliveryId: 'reviewOrder:notify:0:channel:0',
      provider: 'discord',
      destination: CHANNEL_ID,
      idempotencyKey: `sha256:${'c'.repeat(64)}`,
      credentials: { botToken: { kind: 'secretReference', name: BOT_SECRET } },
      decisionCapability: `ncap_${'a'.repeat(16)}.${'b'.repeat(32)}`,
      message: { workflowId: 'discord-approval', approvalName: 'Approve order 42' },
    } as const;
    const resolved = await adapter.resolveCredentials(secrets(), invocation as never);
    await adapter.prepare(invocation as never, resolved.credentials);
    const first = await adapter.deliver(invocation as never, resolved.credentials);
    const duplicate = await adapter.deliver(invocation as never, resolved.credentials);
    expect(duplicate).toEqual(first);
    expect(requests.filter(request => request.method === 'POST')).toHaveLength(1);
    expect(requests.find(request => request.method === 'POST')?.body.components).toEqual([
      {
        type: 1,
        components: [
          { type: 2, style: 3, label: 'Approve', custom_id: `a:${invocation.decisionCapability}` },
          { type: 2, style: 4, label: 'Reject', custom_id: `r:${invocation.decisionCapability}` },
        ],
      },
    ]);
    await adapter.update({
      ...invocation,
      messageType: 'update',
      updateId: 'nupdate_discord_1',
      providerMessage: first,
      resolution: 'approved',
    } as never, resolved.credentials);
    expect(requests.at(-1)).toMatchObject({
      method: 'PATCH',
      body: { components: [] },
    });
    await adapter.close();
  });
});
