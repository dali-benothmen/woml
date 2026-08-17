import { describe, expect, test } from 'bun:test';

import {
  CommunicationProviderRegistry,
  CommunicationTriggerHost,
  type BuiltInCommunicationProvider,
  type CommunicationTriggerAdapter,
} from '../src/communication-provider';
import { NotificationProviderHost } from '../src/notification-provider';

class FakeTriggerAdapter implements CommunicationTriggerAdapter {
  readonly events: string[];

  constructor(
    readonly provider: BuiltInCommunicationProvider,
    private readonly sharedEvents: string[],
    private readonly startFailure?: Error
  ) {
    this.events = sharedEvents;
  }

  async start(): Promise<void> {
    this.events.push(`start:${this.provider}`);
    if (this.startFailure !== undefined) throw this.startFailure;
  }

  async close(): Promise<void> {
    this.events.push(`close:${this.provider}`);
  }
}

describe('Communication-provider foundation', () => {
  test('starts adapters in registration order and closes them in reverse order', async () => {
    const events: string[] = [];
    const host = new CommunicationTriggerHost([
      new FakeTriggerAdapter('slack', events),
      new FakeTriggerAdapter('telegram', events),
      new FakeTriggerAdapter('discord', events),
    ]);

    expect(host.providers()).toEqual(['slack', 'telegram', 'discord']);
    await host.start();
    await host.close();
    await host.close();

    expect(events).toEqual([
      'start:slack',
      'start:telegram',
      'start:discord',
      'close:discord',
      'close:telegram',
      'close:slack',
    ]);
  });

  test('rolls back a partial start, including the adapter whose startup failed', async () => {
    const events: string[] = [];
    const failure = new Error('synthetic Discord startup failure');
    const host = new CommunicationTriggerHost([
      new FakeTriggerAdapter('slack', events),
      new FakeTriggerAdapter('telegram', events),
      new FakeTriggerAdapter('discord', events, failure),
      new FakeTriggerAdapter('whatsapp', events),
    ]);

    expect(host.start()).rejects.toBe(failure);
    await host.close();
    expect(events).toEqual([
      'start:slack',
      'start:telegram',
      'start:discord',
      'close:discord',
      'close:telegram',
      'close:slack',
    ]);
  });

  test('rejects duplicate trigger adapters for one provider', () => {
    const events: string[] = [];
    expect(
      () =>
        new CommunicationTriggerHost([
          new FakeTriggerAdapter('slack', events),
          new FakeTriggerAdapter('slack', events),
        ])
    ).toThrow('registered more than once');
  });

  test('keeps trigger, notification, and messaging registrations in separate roles', () => {
    const events: string[] = [];
    const registry = new CommunicationProviderRegistry();
    const trigger = new FakeTriggerAdapter('slack', events);
    registry.register({ role: 'trigger', adapter: trigger });

    expect(registry.providers('trigger')).toEqual(['slack']);
    expect(registry.providers('notification')).toEqual([]);
    expect(registry.providers('messaging')).toEqual([]);
    expect(registry.notificationAdapter('slack')).toBeUndefined();
    expect(registry.messagingAdapter('slack')).toBeUndefined();
    expect(() =>
      registry.register({ role: 'trigger', adapter: trigger })
    ).toThrow('registered more than once');
  });

  test('does not route a Slack approval invocation through a different provider adapter', async () => {
    const deliver = await Bun.file(
      new URL('./fixtures/notification-provider/deliver.v1.json', import.meta.url)
    ).json();
    const host = new NotificationProviderHost({
      secretStore: {} as never,
      adapter: { provider: 'telegram' } as never,
      send: async () => {},
    });

    expect(() => host.accept(deliver)).toThrow(
      'Notification invocation provider "slack" has no active adapter'
    );
  });
});
