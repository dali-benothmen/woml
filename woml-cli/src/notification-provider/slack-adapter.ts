import {
  CommunicationProviderAdapterError,
  type CommunicationNotificationAdapter,
} from '../communication-provider';
import type { SecretStore } from '../secrets';
import { SecretStoreError } from '../secrets';
import { validFailure, validProviderMessage } from './protocol';
import {
  resolveSlackCredentials,
  SlackTransportError,
  type SlackTransport,
} from './slack-transport';
import type {
  NotificationInvocation,
  NotificationProviderFailure,
  ProviderMessageIdentity,
  ResolvedSlackCredentials,
} from './types';

function redact(message: string, values: readonly string[]): string {
  let redacted = message;
  for (const value of values) {
    if (value.length > 0) redacted = redacted.split(value).join('[REDACTED]');
  }
  return redacted.slice(0, 1024) || 'The provider invocation failed safely.';
}

/** Slack implementation of the provider-neutral notification adapter role. */
export class SlackNotificationAdapter
  implements
    CommunicationNotificationAdapter<
      NotificationInvocation,
      ResolvedSlackCredentials,
      ProviderMessageIdentity,
      NotificationProviderFailure
    >
{
  readonly provider = 'slack' as const;
  readonly #transport: SlackTransport;

  constructor(transport: SlackTransport) {
    this.#transport = transport;
  }

  async resolveCredentials(
    secretStore: SecretStore,
    invocation: NotificationInvocation
  ): Promise<{
    readonly credentials: ResolvedSlackCredentials;
    readonly secretValues: readonly string[];
  }> {
    const credentials = await resolveSlackCredentials(
      secretStore,
      invocation.credentials
    );
    return {
      credentials,
      secretValues: [credentials.botToken, credentials.appToken],
    };
  }

  async prepare(
    invocation: NotificationInvocation,
    credentials: ResolvedSlackCredentials
  ): Promise<void> {
    await this.#transport.ensureConnection(
      invocation.credentials.appToken.name,
      credentials.appToken
    );
  }

  async deliver(
    invocation: NotificationInvocation,
    credentials: ResolvedSlackCredentials
  ): Promise<ProviderMessageIdentity> {
    if (invocation.messageType !== 'deliver') {
      throw new Error('A Slack update cannot be delivered as a new message.');
    }
    return await this.#transport.deliver({ invocation, credentials });
  }

  async update(
    invocation: NotificationInvocation,
    credentials: ResolvedSlackCredentials
  ): Promise<void> {
    if (invocation.messageType !== 'update') {
      throw new Error('A Slack delivery cannot be executed as an update.');
    }
    await this.#transport.update({ invocation, credentials });
  }

  validMessageIdentity(value: unknown): value is ProviderMessageIdentity {
    return validProviderMessage(value);
  }

  invalidMessageIdentityFailure(): NotificationProviderFailure {
    return {
      kind: 'request_invalid',
      code: 'WOML_SLACK_RESPONSE_INVALID',
      message: 'Slack returned an invalid message identity.',
      retryable: false,
    };
  }

  safeFailure(
    error: unknown,
    resolvedValues: readonly string[]
  ): NotificationProviderFailure {
    if (
      error instanceof CommunicationProviderAdapterError &&
      validFailure(error.failure)
    ) {
      return {
        ...error.failure,
        message: redact(error.failure.message, resolvedValues),
      };
    }
    if (error instanceof SlackTransportError && validFailure(error.failure)) {
      return {
        ...error.failure,
        message: redact(error.failure.message, resolvedValues),
      };
    }
    if (
      error instanceof SecretStoreError &&
      error.code === 'WOML_SECRET_NOT_FOUND'
    ) {
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

  async close(): Promise<void> {
    await this.#transport.close();
  }
}
