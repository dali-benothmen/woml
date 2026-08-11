import { describe, expect, test } from 'bun:test';

import type { SecretMetadata, SecretStore } from '../src/secrets';
import {
  FakeSlackTransport,
  NotificationProviderHost,
  SlackTransportError,
  type CompletedMessage,
  type DeliverMessage,
  type InteractionMessage,
  type InformationalDeliverMessage,
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type NotificationProviderOutbound,
  type ProviderMessageIdentity,
  type SlackDeliveryRequest,
  type SlackTransport,
  type SlackUpdateRequest,
  type UpdateMessage,
} from '../src/notification-provider';

const fixtureDirectory = new URL(
  './fixtures/notification-provider/',
  import.meta.url
);

async function fixture<T>(name: string): Promise<T> {
  return (await Bun.file(new URL(name, fixtureDirectory)).json()) as T;
}

class MemorySecrets implements SecretStore {
  readonly provider = 'environment' as const;
  readonly reads: string[] = [];

  constructor(readonly values: Readonly<Record<string, string>>) {}

  async get(name: string): Promise<string | undefined> {
    this.reads.push(name);
    return this.values[name];
  }

  async has(name: string): Promise<boolean> {
    return this.values[name] !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    return Object.keys(this.values).map(name => ({
      name,
      provider: this.provider,
    }));
  }

  async set(): Promise<void> {
    throw new Error('read only');
  }

  async delete(): Promise<boolean> {
    throw new Error('read only');
  }
}

function credentials(): MemorySecrets {
  return new MemorySecrets({
    SLACK_BOT_TOKEN: 'xoxb-super-secret-bot-token',
    SLACK_APP_TOKEN: 'xapp-super-secret-app-token',
  });
}

function completions(
  messages: readonly NotificationProviderOutbound[]
): CompletedMessage[] {
  return messages.filter(
    (message): message is CompletedMessage => message.messageType === 'completed'
  );
}

describe('N4 notification provider host', () => {
  test('LEC5 v2 sends informational messages without approval capabilities', async () => {
    const sent: NotificationProviderOutbound[] = [];
    const interactions: InteractionMessage[] = [];
    const transport = new FakeSlackTransport({
      emit: message => {
        interactions.push(message);
      },
      automaticDecision: 'approved',
    });
    const host = new NotificationProviderHost({
      secretStore: credentials(),
      transport,
      protocolVersion: INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
      send: async message => {
        sent.push(message);
      },
    });
    const invocation: InformationalDeliverMessage = {
      protocol: 'woml.notification-provider-host',
      protocolVersion: 2,
      messageType: 'deliver',
      mode: 'informational',
      invocationId: 'lifecycle-delivery-1',
      runId: 'run_lec5',
      hookInvocationId:
        'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      actionId: 'lifecycle:run_success:action:0',
      deliveryId: 'lifecycle:run_success:action:0:provider:0:channel:0',
      provider: 'slack',
      destination: '#woml-testing',
      idempotencyKey:
        'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      credentials: {
        botToken: { kind: 'secretReference', name: 'SLACK_BOT_TOKEN' },
        appToken: { kind: 'secretReference', name: 'SLACK_APP_TOKEN' },
      },
      message: 'Workflow lifecycle-demo succeeded.',
    };

    host.accept(invocation);
    await host.drain();

    expect(completions(sent)).toHaveLength(1);
    expect(completions(sent)[0]).toMatchObject({
      protocolVersion: 2,
      outcome: { kind: 'delivery_success' },
    });
    expect(transport.messages()[0]).toMatchObject({
      message: invocation.message,
      destination: '#woml-testing',
    });
    expect(transport.messages()[0]!.decisionCapability).toBeUndefined();
    expect(interactions).toEqual([]);
    expect(JSON.stringify(sent)).not.toContain(invocation.message);
    await host.close();
  });

  test('resolves symbolic credentials inside each invocation and reuses one app connection', async () => {
    const sent: NotificationProviderOutbound[] = [];
    const interactions: InteractionMessage[] = [];
    const secrets = credentials();
    const transport = new FakeSlackTransport({
      emit: message => {
        interactions.push(message);
      },
    });
    const host = new NotificationProviderHost({
      secretStore: secrets,
      transport,
      send: async message => {
        sent.push(message);
      },
    });
    const first = await fixture<DeliverMessage>('deliver.v1.json');
    const second: DeliverMessage = {
      ...first,
      invocationId: 'delivery-invocation-02',
      deliveryId: 'releaseApproval:notify:0:channel:1',
      destination: '#engineering',
      idempotencyKey:
        'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      decisionCapability:
        'ncap_11111111111111111111111111111111.2222222222222222222222222222222222222222222222222222222222222222',
    };

    host.accept(first);
    host.accept(second);
    await host.drain();

    expect(completions(sent).map(item => item.outcome.kind)).toEqual([
      'delivery_success',
      'delivery_success',
    ]);
    expect(secrets.reads).toEqual([
      'SLACK_BOT_TOKEN',
      'SLACK_BOT_TOKEN',
      'SLACK_APP_TOKEN',
      'SLACK_APP_TOKEN',
    ]);
    expect(transport.connectionCount).toBe(1);
    expect(transport.messages()).toHaveLength(2);
    expect(JSON.stringify(sent)).not.toContain('xoxb-super-secret');
    expect(JSON.stringify(sent)).not.toContain('xapp-super-secret');
    expect(interactions).toHaveLength(0);
    await host.close();
  });

  test('emits native actions and disables the message after a durable-style update', async () => {
    const sent: NotificationProviderOutbound[] = [];
    const transport = new FakeSlackTransport({
      emit: async message => {
        sent.push(message);
      },
    });
    const host = new NotificationProviderHost({
      secretStore: credentials(),
      transport,
      send: async message => {
        sent.push(message);
      },
    });
    const deliver = await fixture<DeliverMessage>('deliver.v1.json');
    host.accept(deliver);
    await host.drain();
    await transport.click(deliver.deliveryId, 'approved', 'U12345678');

    const update = await fixture<UpdateMessage>('update.v1.json');
    host.accept({
      ...update,
      providerMessage: transport.messages()[0]!.providerMessage,
    });
    await host.drain();
    await transport.click(deliver.deliveryId, 'rejected', 'U87654321');

    const actions = sent.filter(
      (message): message is InteractionMessage =>
        message.messageType === 'interaction'
    );
    expect(actions).toHaveLength(1);
    expect(actions[0]).toMatchObject({
      deliveryId: deliver.deliveryId,
      decision: 'approved',
      providerActorId: 'U12345678',
      decisionCapability: deliver.decisionCapability,
    });
    expect(completions(sent).map(item => item.outcome.kind)).toEqual([
      'delivery_success',
      'update_success',
    ]);
    expect(transport.messages()[0]!.resolution).toBe('approved');
    await host.close();
  });

  test('returns secret-safe failures for missing credentials and malicious provider errors', async () => {
    const deliver = await fixture<DeliverMessage>('deliver.v1.json');
    const missingSent: NotificationProviderOutbound[] = [];
    const missingHost = new NotificationProviderHost({
      secretStore: new MemorySecrets({}),
      transport: new FakeSlackTransport({ emit: () => {} }),
      send: async message => {
        missingSent.push(message);
      },
    });
    missingHost.accept(deliver);
    await missingHost.drain();
    expect(completions(missingSent)[0]!.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'secret_not_found', code: 'WOML_SECRET_NOT_FOUND' },
    });

    const maliciousToken = 'xoxb-super-secret-bot-token';
    const malicious: SlackTransport = {
      async ensureConnection() {},
      async deliver(): Promise<ProviderMessageIdentity> {
        throw new SlackTransportError({
          kind: 'provider_unavailable',
          code: 'WOML_SLACK_UNAVAILABLE',
          message: `provider rejected ${maliciousToken}`,
          retryable: true,
        });
      },
      async update() {},
      async close() {},
    };
    const maliciousSent: NotificationProviderOutbound[] = [];
    const maliciousHost = new NotificationProviderHost({
      secretStore: credentials(),
      transport: malicious,
      send: async message => {
        maliciousSent.push(message);
      },
    });
    maliciousHost.accept(deliver);
    await maliciousHost.drain();
    expect(JSON.stringify(maliciousSent)).not.toContain(maliciousToken);
    expect(JSON.stringify(maliciousSent)).toContain('[REDACTED]');

    const malformed: SlackTransport = {
      async ensureConnection() {},
      async deliver(): Promise<ProviderMessageIdentity> {
        return {
          workspaceId: 'not-a-slack-workspace',
          channelId: 'C12345678',
          messageId: '1723024800.000001',
        };
      },
      async update() {},
      async close() {},
    };
    const malformedSent: NotificationProviderOutbound[] = [];
    const malformedHost = new NotificationProviderHost({
      secretStore: credentials(),
      transport: malformed,
      send: async message => {
        malformedSent.push(message);
      },
    });
    malformedHost.accept(deliver);
    await malformedHost.drain();
    expect(completions(malformedSent)[0]!.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'request_invalid',
        code: 'WOML_SLACK_RESPONSE_INVALID',
        retryable: false,
      },
    });
  });

  test('ignores late actions after resolution and cancellation', async () => {
    const interactions: InteractionMessage[] = [];
    const transport = new FakeSlackTransport({
      emit: message => {
        interactions.push(message);
      },
    });
    const host = new NotificationProviderHost({
      secretStore: credentials(),
      transport,
      send: async () => {},
    });
    const deliver = await fixture<DeliverMessage>('deliver.v1.json');
    host.accept(deliver);
    await host.drain();
    await transport.click(deliver.deliveryId, 'approved');

    const update = await fixture<UpdateMessage>('update.v1.json');
    host.accept({
      ...update,
      providerMessage: transport.messages()[0]!.providerMessage,
    });
    await host.drain();
    await transport.click(deliver.deliveryId, 'rejected');
    await host.close();
    await transport.click(deliver.deliveryId, 'approved');

    expect(interactions).toHaveLength(1);
    expect(() => host.accept(deliver)).toThrow('closed');

    const cancelledInteractions: InteractionMessage[] = [];
    const cancelledTransport = new FakeSlackTransport({
      emit: message => {
        cancelledInteractions.push(message);
      },
      automaticDecision: 'approved',
      automaticDelayMs: 10,
    });
    const cancelledHost = new NotificationProviderHost({
      secretStore: credentials(),
      transport: cancelledTransport,
      send: async () => {},
    });
    cancelledHost.accept(deliver);
    await cancelledHost.drain();
    await cancelledHost.close();
    await Bun.sleep(20);
    expect(cancelledInteractions).toHaveLength(0);
  });

  test('multiplexes out of order and rejects duplicate active invocation IDs', async () => {
    const delivered: string[] = [];
    const transport: SlackTransport = {
      async ensureConnection() {},
      async deliver(request: SlackDeliveryRequest) {
        const slow = request.invocation.destination === '#approvals';
        await Bun.sleep(slow ? 20 : 1);
        delivered.push(request.invocation.invocationId);
        return {
          workspaceId: 'T12345678',
          channelId: slow ? 'C12345678' : 'C87654321',
          messageId: slow ? '1723024800.000001' : '1723024800.000002',
        };
      },
      async update(_request: SlackUpdateRequest) {},
      async close() {},
    };
    const sent: NotificationProviderOutbound[] = [];
    const host = new NotificationProviderHost({
      secretStore: credentials(),
      transport,
      send: async message => {
        sent.push(message);
      },
    });
    const first = await fixture<DeliverMessage>('deliver.v1.json');
    const second: DeliverMessage = {
      ...first,
      invocationId: 'delivery-invocation-fast',
      deliveryId: 'releaseApproval:notify:0:channel:1',
      destination: '#engineering',
    };
    host.accept(first);
    expect(() => host.accept(first)).toThrow('already active');
    host.accept(second);
    await host.drain();
    expect(delivered).toEqual([
      'delivery-invocation-fast',
      first.invocationId,
    ]);
    expect(completions(sent).map(item => item.invocationId)).toEqual(delivered);
  });
});
