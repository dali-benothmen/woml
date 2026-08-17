import {
  CommunicationProviderAdapterError,
  type CommunicationNotificationAdapter,
} from '../communication-provider';
import type { SecretStore } from '../secrets';
import { assertNotificationInvocation } from './protocol';
import { SlackNotificationAdapter } from './slack-adapter';
import {
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type CompletedMessage,
  type NotificationInvocation,
  type NotificationProviderFailure,
  type NotificationProviderOutbound,
} from './types';
import type { SlackTransport } from './slack-transport';

export type ActiveNotificationAdapter = CommunicationNotificationAdapter<
  NotificationInvocation,
  unknown,
  import('./types').ProviderMessageIdentity,
  NotificationProviderFailure
>;

export interface NotificationProviderHostOptions {
  readonly secretStore: SecretStore;
  readonly adapter?: ActiveNotificationAdapter;
  readonly adapters?: readonly ActiveNotificationAdapter[];
  /** Compatibility constructor for the frozen Slack v1/v2 host API. */
  readonly transport?: SlackTransport;
  readonly send: (message: NotificationProviderOutbound) => Promise<void>;
  readonly maxFrameBytes?: number;
  readonly protocolVersion?:
    | typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION
    | typeof INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
}

function elapsedMilliseconds(startedAt: number): number {
  return Math.max(0, performance.now() - startedAt);
}

function completed(
  protocolVersion: CompletedMessage['protocolVersion'],
  invocationId: string,
  startedAt: number,
  outcome: CompletedMessage['outcome']
): CompletedMessage {
  return {
    protocol: NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion,
    messageType: 'completed',
    invocationId,
    outcome,
    durationMs: elapsedMilliseconds(startedAt),
  };
}

function byteLength(value: unknown): number {
  return Buffer.byteLength(JSON.stringify(value), 'utf8');
}

export class NotificationProviderHost {
  readonly #secretStore: SecretStore;
  readonly #adapters: ReadonlyMap<string, ActiveNotificationAdapter>;
  readonly #send: NotificationProviderHostOptions['send'];
  readonly #maxFrameBytes: number;
  readonly #protocolVersion: CompletedMessage['protocolVersion'];
  readonly #tasks = new Map<string, Promise<void>>();
  #aborted = false;

  constructor(options: NotificationProviderHostOptions) {
    if (
      [options.adapter, options.adapters, options.transport].filter(
        value => value !== undefined
      ).length > 1
    ) {
      throw new Error(
        'NotificationProviderHost accepts adapters or a legacy Slack transport, not both.'
      );
    }
    if (
      options.adapter === undefined &&
      options.adapters === undefined &&
      options.transport === undefined
    ) {
      throw new Error('NotificationProviderHost requires a provider adapter.');
    }
    this.#secretStore = options.secretStore;
    const adapters = options.adapters ?? [
      options.adapter ??
        (new SlackNotificationAdapter(
          options.transport!
        ) as ActiveNotificationAdapter),
    ];
    const mapped = new Map<string, ActiveNotificationAdapter>();
    for (const adapter of adapters) {
      if (mapped.has(adapter.provider)) {
        throw new Error(
          `Notification adapter "${adapter.provider}" is registered more than once.`
        );
      }
      mapped.set(adapter.provider, adapter);
    }
    this.#adapters = mapped;
    this.#send = options.send;
    this.#maxFrameBytes =
      options.maxFrameBytes ?? NOTIFICATION_PROVIDER_MAX_FRAME_BYTES;
    this.#protocolVersion =
      options.protocolVersion ?? NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  }

  accept(value: unknown): void {
    if (this.#aborted) throw new Error('The notification provider host is closed.');
    assertNotificationInvocation(value);
    const adapter = this.#adapters.get(value.provider);
    if (adapter === undefined) {
      throw new Error(
        `Notification invocation provider "${value.provider}" has no active adapter.`
      );
    }
    if (value.protocolVersion !== this.#protocolVersion) {
      throw new Error(
        `Notification Provider Host v${this.#protocolVersion} cannot execute a v${value.protocolVersion} invocation.`
      );
    }
    if (this.#tasks.has(value.invocationId)) {
      throw new Error(`Invocation ID "${value.invocationId}" is already active.`);
    }
    const task = this.#invoke(value, adapter).finally(() => {
      this.#tasks.delete(value.invocationId);
    });
    this.#tasks.set(value.invocationId, task);
  }

  async drain(): Promise<void> {
    while (this.#tasks.size > 0) await Promise.all([...this.#tasks.values()]);
  }

  async close(): Promise<void> {
    this.#aborted = true;
    await this.drain();
    await Promise.all(
      [...this.#adapters.values()].map(adapter => adapter.close())
    );
  }

  async #invoke(
    invocation: NotificationInvocation,
    adapter: ActiveNotificationAdapter
  ): Promise<void> {
    const startedAt = performance.now();
    let resolvedValues: string[] = [];
    let response: CompletedMessage;
    try {
      const resolved = await adapter.resolveCredentials(
        this.#secretStore,
        invocation
      );
      const credentials = resolved.credentials;
      resolvedValues = [...resolved.secretValues];
      await adapter.prepare(invocation, credentials);
      if (invocation.messageType === 'deliver') {
        const providerMessage = await adapter.deliver(
          invocation,
          credentials
        );
        if (!adapter.validMessageIdentity(providerMessage)) {
          throw new CommunicationProviderAdapterError(
            adapter.invalidMessageIdentityFailure()
          );
        }
        response = completed(invocation.protocolVersion, invocation.invocationId, startedAt, {
          kind: 'delivery_success',
          providerMessage,
        });
      } else {
        await adapter.update(invocation, credentials);
        response = completed(invocation.protocolVersion, invocation.invocationId, startedAt, {
          kind: 'update_success',
        });
      }
    } catch (error) {
      response = completed(invocation.protocolVersion, invocation.invocationId, startedAt, {
        kind: 'failure',
        error: adapter.safeFailure(error, resolvedValues),
      });
    }

    if (byteLength(response) > this.#maxFrameBytes) {
      response = completed(invocation.protocolVersion, invocation.invocationId, startedAt, {
        kind: 'failure',
        error: {
          kind: 'size_limit_exceeded',
          code: 'WOML_NOTIFICATION_SIZE_LIMIT_EXCEEDED',
          message: 'The provider response exceeds the protocol byte limit.',
          retryable: false,
        },
      });
    }
    await this.#send(response);
  }
}
