import { describe, expect, test } from 'bun:test';

import { NotificationProviderHost } from '../src/notification-provider';
import type {
  InformationalDeliverMessage,
  NotificationProviderOutbound,
} from '../src/notification-provider/types';
import type { SecretStore } from '../src/secrets';
import {
  SharedTelegramTransport,
  TelegramNotificationAdapter,
  TelegramTriggerHost,
  type TelegramFailure,
} from '../src/telegram';
import type {
  TriggerIngressAdmit,
  TriggerIngressOutcome,
} from '../src/rust-executor';

const BOT_TOKEN = '123456789:test-telegram-token';
const BOT_SECRET = 'TELEGRAM_BOT_TOKEN';

function secrets(): SecretStore {
  const values = new Map([[BOT_SECRET, BOT_TOKEN]]);
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

function telegramMethod(input: RequestInfo | URL): string {
  return new URL(String(input)).pathname.split('/').pop()!;
}

function requestBody(init?: RequestInit): Record<string, unknown> {
  return JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>;
}

function ok(result: unknown): Response {
  return Response.json({ ok: true, result });
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

function blockedUntilAbort(init?: RequestInit): Promise<Response> {
  return new Promise((_resolve, reject) => {
    const signal = init?.signal;
    if (signal?.aborted) {
      reject(new DOMException('aborted', 'AbortError'));
      return;
    }
    signal?.addEventListener(
      'abort',
      () => reject(new DOMException('aborted', 'AbortError')),
      { once: true }
    );
  });
}

describe('Telegram production runtime', () => {
  test('reports an active webhook or competing poller as a terminal setup error', async () => {
    const failures: TelegramFailure[] = [];
    const fakeFetch = (async input => {
      const method = telegramMethod(input);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'getUpdates') {
        return Response.json(
          {
            ok: false,
            error_code: 409,
            description: 'Conflict: terminated by other getUpdates request',
          },
          { status: 409 }
        );
      }
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({
      fetch: fakeFetch,
      onFatal: failure => failures.push(failure),
    });

    await transport.ensurePolling(BOT_SECRET, BOT_TOKEN);
    await eventually(() => expect(failures).toHaveLength(1));
    expect(failures[0]).toEqual({
      kind: 'provider_unavailable',
      code: 'WOML_TELEGRAM_POLLING_CONFLICT',
      message:
        'Telegram long polling is unavailable because another poller or webhook is active for this bot.',
      retryable: false,
    });
    await transport.close();
  });

  test('stops polling after one permanent durable admission rejection', async () => {
    const failures: TelegramFailure[] = [];
    let polls = 0;
    const fakeFetch = (async input => {
      const method = telegramMethod(input);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'getUpdates') {
        polls += 1;
        return ok([
          {
            update_id: 40,
            message: {
              message_id: 6,
              date: 1_786_896_000,
              text: '/start',
              from: { id: 111222333, is_bot: false, first_name: 'Alex' },
              chat: { id: 111222333, type: 'private' },
            },
          },
        ]);
      }
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({
      fetch: fakeFetch,
      retryDelayMs: 1,
      onFatal: failure => failures.push(failure),
    });
    const host = new TelegramTriggerHost({
      registrations: [
        {
          workflowId: 'telegram-agent',
          definitionHash: `sha256:${'a'.repeat(64)}`,
          triggerId: 'agentMessage',
          events: ['message'],
          credentialNames: { botToken: BOT_SECRET },
        },
      ],
      secretStore: secrets(),
      transport,
      submit: async request => ({
        contract: 'woml.trigger-ingress',
        contractVersion: 1,
        messageType: 'rejected',
        requestId: request.requestId,
        failure: {
          code: 'WOML_TRIGGER_CONTRACT_INVALID',
          message: 'The compiled workflow and durable trigger contracts are incompatible.',
          retryable: false,
        },
      }),
    });

    await host.start();
    await eventually(() => expect(failures).toHaveLength(1));
    await Bun.sleep(5);
    expect(polls).toBe(1);
    expect(failures[0]).toMatchObject({
      code: 'WOML_TRIGGER_CONTRACT_INVALID',
      retryable: false,
    });
    await host.close();
    await transport.close();
  });

  test('advances the polling offset only after Rust durably accepts a message', async () => {
    const requests: TriggerIngressAdmit[] = [];
    const pollBodies: Record<string, unknown>[] = [];
    let release!: (outcome: TriggerIngressOutcome) => void;
    const durableAcceptance = new Promise<TriggerIngressOutcome>(resolve => {
      release = resolve;
    });
    const fakeFetch = (async (input, init) => {
      const method = telegramMethod(input);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'getUpdates') {
        pollBodies.push(requestBody(init));
        if (pollBodies.length === 1) {
          return ok([
            {
              update_id: 41,
              message: {
                message_id: 7,
                date: 1_786_896_000,
                text: 'run order 42',
                from: {
                  id: 111222333,
                  is_bot: false,
                  first_name: 'Alex',
                },
                chat: { id: -1001234567890, type: 'supergroup' },
              },
            },
          ]);
        }
        return blockedUntilAbort(init);
      }
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({
      fetch: fakeFetch,
      pollTimeoutSeconds: 1,
      retryDelayMs: 1,
    });
    const host = new TelegramTriggerHost({
      registrations: [
        {
          workflowId: 'telegram-agent',
          definitionHash: `sha256:${'a'.repeat(64)}`,
          triggerId: 'agentMessage',
          events: ['message'],
          credentialNames: { botToken: BOT_SECRET },
        },
      ],
      secretStore: secrets(),
      transport,
      submit: request => {
        requests.push(request);
        return durableAcceptance;
      },
    });

    await host.start();
    await eventually(() => expect(requests).toHaveLength(1));
    expect(pollBodies).toHaveLength(1);
    expect(requests[0]).toMatchObject({
      workflowId: 'telegram-agent',
      triggerId: 'agentMessage',
      triggerHandler: 'trigger.telegram',
      sourceIdentity: 'telegram:987654321:41:telegram-agent:agentMessage',
      payload: {
        provider: 'telegram',
        event: 'message',
        text: 'run order 42',
        senderId: '111222333',
        senderName: 'Alex',
        conversationId: '-1001234567890',
        conversationType: 'group',
        messageId: '7',
        providerData: { botId: '987654321' },
      },
    });

    release({
      contract: 'woml.trigger-ingress',
      contractVersion: 1,
      messageType: 'accepted',
      requestId: requests[0]!.requestId,
      occurrenceId: 'occ_telegram_41',
      runId: 'run_telegram_41',
      duplicate: false,
    });
    await eventually(() => expect(pollBodies).toHaveLength(2));
    expect(pollBodies[1]!.offset).toBe(42);

    await host.close();
    await transport.close();
  });

  test('resolves approval callbacks before acknowledging the Telegram update', async () => {
    const calls: string[] = [];
    const fakeFetch = (async (input, init) => {
      const method = telegramMethod(input);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'getUpdates') {
        if (!calls.includes('callback-delivered')) {
          calls.push('callback-delivered');
          return ok([
            {
              update_id: 52,
              callback_query: {
                id: 'callback-52',
                from: { id: 111222333 },
                data: `a:ncap_${'a'.repeat(16)}.${'b'.repeat(32)}`,
              },
            },
          ]);
        }
        return blockedUntilAbort(init);
      }
      if (method === 'answerCallbackQuery') {
        calls.push('callback-answered');
        return ok(true);
      }
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({ fetch: fakeFetch });
    const host = new TelegramTriggerHost({
      registrations: [],
      credentialNames: [BOT_SECRET],
      secretStore: secrets(),
      transport,
      submit: async () => {
        throw new Error('No trigger should be admitted.');
      },
      resolveApproval: async update => {
        calls.push(`approval-${update.decision}-${update.actorId}`);
      },
    });

    await host.start();
    await eventually(() =>
      expect(calls).toContain('callback-answered')
    );
    expect(calls).toEqual([
      'callback-delivered',
      'approval-approved-111222333',
      'callback-answered',
    ]);
    await host.close();
    await transport.close();
  });

  test('delivers lifecycle notifications without approval buttons', async () => {
    const sentBodies: Record<string, unknown>[] = [];
    const fakeFetch = (async (input, init) => {
      const method = telegramMethod(input);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'sendMessage') {
        sentBodies.push(requestBody(init));
        return ok({ message_id: 70, chat: { id: -1001234567890 } });
      }
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({ fetch: fakeFetch });
    const outbound: NotificationProviderOutbound[] = [];
    const host = new NotificationProviderHost({
      secretStore: secrets(),
      adapter: new TelegramNotificationAdapter(transport) as never,
      protocolVersion: 2,
      send: async message => {
        outbound.push(message);
      },
    });
    const delivery: InformationalDeliverMessage = {
      protocol: 'woml.notification-provider-host',
      protocolVersion: 2,
      messageType: 'deliver',
      mode: 'informational',
      invocationId: 'invocation_telegram_lifecycle_1',
      runId: 'run_telegram_lifecycle',
      hookInvocationId: `sha256:${'e'.repeat(64)}`,
      actionId: 'lifecycle:run_success:action:0',
      deliveryId: 'lifecycle:run_success:action:0:provider:0:chat:0',
      provider: 'telegram',
      destination: '-1001234567890',
      idempotencyKey: `sha256:${'f'.repeat(64)}`,
      credentials: {
        botToken: { kind: 'secretReference', name: BOT_SECRET },
      },
      message: 'Workflow telegram-lifecycle succeeded.',
    };

    host.accept(delivery);
    await host.drain();

    expect(sentBodies).toEqual([
      {
        chat_id: '-1001234567890',
        text: delivery.message,
      },
    ]);
    expect(outbound[0]).toMatchObject({
      protocolVersion: 2,
      messageType: 'completed',
      outcome: { kind: 'delivery_success' },
    });
    await host.close();
  });

  test('delivers approval buttons, updates the message, and reuses a confirmed idempotent delivery', async () => {
    const methods: string[] = [];
    const sentBodies: Record<string, unknown>[] = [];
    const fakeFetch = (async (input, init) => {
      const method = telegramMethod(input);
      methods.push(method);
      if (method === 'getMe') {
        return ok({ id: 987654321, is_bot: true, username: 'woml_test_bot' });
      }
      if (method === 'sendMessage') {
        sentBodies.push(requestBody(init));
        return ok({
          message_id: 88,
          chat: { id: -1001234567890 },
        });
      }
      if (method === 'editMessageReplyMarkup') return ok(true);
      throw new Error(`Unexpected Telegram method ${method}`);
    }) as typeof fetch;
    const transport = new SharedTelegramTransport({ fetch: fakeFetch });
    const outbound: NotificationProviderOutbound[] = [];
    const host = new NotificationProviderHost({
      secretStore: secrets(),
      adapter: new TelegramNotificationAdapter(transport) as never,
      send: async message => {
        outbound.push(message);
      },
    });
    const delivery = {
      protocol: 'woml.notification-provider-host',
      protocolVersion: 1,
      messageType: 'deliver',
      invocationId: 'invocation_telegram_delivery_1',
      runId: 'run_telegram_approval',
      approvalId: 'releaseApproval',
      requestId: 'aprreq_telegram_approval',
      deliveryId: 'releaseApproval:notify:0:chat:0',
      provider: 'telegram',
      destination: '-1001234567890',
      idempotencyKey: `sha256:${'c'.repeat(64)}`,
      credentials: {
        botToken: { kind: 'secretReference', name: BOT_SECRET },
      },
      decisionCapability: `ncap_${'a'.repeat(16)}.${'b'.repeat(32)}`,
      message: {
        workflowId: 'telegram-approval',
        approvalName: 'Approve release',
      },
    } as const;
    host.accept(delivery);
    await host.drain();
    host.accept({ ...delivery, invocationId: 'invocation_telegram_delivery_2' });
    await host.drain();

    expect(methods.filter(method => method === 'sendMessage')).toHaveLength(1);
    expect(sentBodies[0]).toMatchObject({
      chat_id: '-1001234567890',
      reply_markup: {
        inline_keyboard: [[
          {
            text: 'Approve',
            callback_data: `a:${delivery.decisionCapability}`,
          },
          { text: 'Reject' },
        ]],
      },
    });
    const identity = {
      provider: 'telegram',
      accountId: '987654321',
      conversationId: '-1001234567890',
      messageId: '88',
    } as const;
    expect(outbound[0]).toMatchObject({
      messageType: 'completed',
      outcome: { kind: 'delivery_success', providerMessage: identity },
    });
    expect(outbound[1]).toMatchObject({
      messageType: 'completed',
      outcome: { kind: 'delivery_success', providerMessage: identity },
    });

    host.accept({
      protocol: 'woml.notification-provider-host',
      protocolVersion: 1,
      messageType: 'update',
      invocationId: 'invocation_telegram_update_1',
      runId: 'run_telegram_approval',
      approvalId: 'releaseApproval',
      requestId: 'aprreq_telegram_approval',
      deliveryId: 'releaseApproval:notify:0:chat:0',
      updateId: 'nupdate_telegram_approval',
      idempotencyKey: `sha256:${'d'.repeat(64)}`,
      provider: 'telegram',
      credentials: {
        botToken: { kind: 'secretReference', name: BOT_SECRET },
      },
      providerMessage: identity,
      resolution: 'approved',
    });
    await host.drain();
    expect(methods).toContain('editMessageReplyMarkup');
    await host.close();
  });
});
