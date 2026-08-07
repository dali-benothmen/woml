import type { SecretStore } from '../secrets';
import { SecretStoreError } from '../secrets';
import { assertNotificationInvocation, validFailure, validProviderMessage } from './protocol';
import {
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type CompletedMessage,
  type NotificationInvocation,
  type NotificationProviderFailure,
  type NotificationProviderOutbound,
  type ResolvedSlackCredentials,
} from './types';
import { SlackTransportError, type SlackTransport } from './fake-slack';

export interface NotificationProviderHostOptions {
  readonly secretStore: SecretStore;
  readonly transport: SlackTransport;
  readonly send: (message: NotificationProviderOutbound) => Promise<void>;
  readonly maxFrameBytes?: number;
}

function elapsedMilliseconds(startedAt: number): number {
  return Math.max(0, performance.now() - startedAt);
}

function completed(
  invocationId: string,
  startedAt: number,
  outcome: CompletedMessage['outcome']
): CompletedMessage {
  return {
    protocol: NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    messageType: 'completed',
    invocationId,
    outcome,
    durationMs: elapsedMilliseconds(startedAt),
  };
}

function redact(message: string, values: readonly string[]): string {
  let redacted = message;
  for (const value of values) {
    if (value.length > 0) redacted = redacted.split(value).join('[REDACTED]');
  }
  return redacted.slice(0, 1024) || 'The provider invocation failed safely.';
}

function safeFailure(
  error: unknown,
  resolvedValues: readonly string[]
): NotificationProviderFailure {
  if (error instanceof SlackTransportError && validFailure(error.failure)) {
    return {
      ...error.failure,
      message: redact(error.failure.message, resolvedValues),
    };
  }
  if (error instanceof SecretStoreError && error.code === 'WOML_SECRET_NOT_FOUND') {
    return {
      kind: 'secret_not_found',
      code: 'WOML_SECRET_NOT_FOUND',
      message: 'A required Slack credential is not available.',
      retryable: false,
    };
  }
  return {
    kind: 'provider_unavailable',
    code: 'WOML_SLACK_UNAVAILABLE',
    message: 'The Slack adapter failed without exposing provider details.',
    retryable: true,
  };
}

function byteLength(value: unknown): number {
  return Buffer.byteLength(JSON.stringify(value), 'utf8');
}

export class NotificationProviderHost {
  readonly #secretStore: SecretStore;
  readonly #transport: SlackTransport;
  readonly #send: NotificationProviderHostOptions['send'];
  readonly #maxFrameBytes: number;
  readonly #tasks = new Map<string, Promise<void>>();
  #aborted = false;

  constructor(options: NotificationProviderHostOptions) {
    this.#secretStore = options.secretStore;
    this.#transport = options.transport;
    this.#send = options.send;
    this.#maxFrameBytes =
      options.maxFrameBytes ?? NOTIFICATION_PROVIDER_MAX_FRAME_BYTES;
  }

  accept(value: unknown): void {
    if (this.#aborted) throw new Error('The notification provider host is closed.');
    assertNotificationInvocation(value);
    if (this.#tasks.has(value.invocationId)) {
      throw new Error(`Invocation ID "${value.invocationId}" is already active.`);
    }
    const task = this.#invoke(value).finally(() => {
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
    await this.#transport.close();
  }

  async #resolve(name: string): Promise<string> {
    const value = await this.#secretStore.get(name);
    if (value === undefined) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        'A required provider credential is missing.'
      );
    }
    return value;
  }

  async #credentials(
    invocation: NotificationInvocation
  ): Promise<ResolvedSlackCredentials> {
    const botToken = await this.#resolve(invocation.credentials.botToken.name);
    const appToken = await this.#resolve(invocation.credentials.appToken.name);
    return { botToken, appToken };
  }

  async #invoke(invocation: NotificationInvocation): Promise<void> {
    const startedAt = performance.now();
    let resolvedValues: string[] = [];
    let response: CompletedMessage;
    try {
      const credentials = await this.#credentials(invocation);
      resolvedValues = [credentials.botToken, credentials.appToken];
      await this.#transport.ensureConnection(
        invocation.credentials.appToken.name,
        credentials.appToken
      );
      if (invocation.messageType === 'deliver') {
        const providerMessage = await this.#transport.deliver({
          invocation,
          credentials,
        });
        if (!validProviderMessage(providerMessage)) {
          throw new SlackTransportError({
            kind: 'request_invalid',
            code: 'WOML_SLACK_RESPONSE_INVALID',
            message: 'Slack returned an invalid message identity.',
            retryable: false,
          });
        }
        response = completed(invocation.invocationId, startedAt, {
          kind: 'delivery_success',
          providerMessage,
        });
      } else {
        await this.#transport.update({ invocation, credentials });
        response = completed(invocation.invocationId, startedAt, {
          kind: 'update_success',
        });
      }
    } catch (error) {
      response = completed(invocation.invocationId, startedAt, {
        kind: 'failure',
        error: safeFailure(error, resolvedValues),
      });
    }

    if (byteLength(response) > this.#maxFrameBytes) {
      response = completed(invocation.invocationId, startedAt, {
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
