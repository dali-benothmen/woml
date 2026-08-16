import {
  CommunicationProviderAdapterError,
  type CommunicationNotificationAdapter,
} from '../communication-provider';
import { SecretStoreError, type SecretStore } from '../secrets';
import type {
  DiscordNotificationCredentials,
  NotificationInvocation,
  NotificationProviderFailure,
  ProviderMessageIdentity,
} from '../notification-provider/types';
import { DiscordTransportError, SharedDiscordTransport } from './transport';

interface ResolvedDiscordCredentials {
  readonly botToken: string;
}

function redact(message: string, values: readonly string[]): string {
  let result = message;
  for (const value of values) {
    if (value.length > 0) result = result.split(value).join('[REDACTED]');
  }
  return result.slice(0, 1024) || 'The Discord provider failed safely.';
}

function approvalText(
  invocation: Extract<NotificationInvocation, { messageType: 'deliver' }>
): string {
  if ('mode' in invocation) return invocation.message;
  return [
    invocation.message.approvalName,
    invocation.message.approvalDescription,
    `Workflow: ${invocation.message.workflowId}`,
    invocation.message.expiresAt === undefined
      ? undefined
      : `Deadline: ${invocation.message.expiresAt}`,
  ]
    .filter((line): line is string => line !== undefined && line.length > 0)
    .join('\n');
}

export class DiscordNotificationAdapter
  implements
    CommunicationNotificationAdapter<
      NotificationInvocation,
      ResolvedDiscordCredentials,
      ProviderMessageIdentity,
      NotificationProviderFailure
    >
{
  readonly provider = 'discord' as const;
  readonly #transport: SharedDiscordTransport;
  readonly #accountByToken = new Map<string, string>();
  readonly #deliveredByIdempotencyKey = new Map<
    string,
    ProviderMessageIdentity
  >();

  constructor(transport: SharedDiscordTransport) {
    this.#transport = transport;
  }

  async resolveCredentials(
    secretStore: SecretStore,
    invocation: NotificationInvocation
  ): Promise<{
    readonly credentials: ResolvedDiscordCredentials;
    readonly secretValues: readonly string[];
  }> {
    const symbolic = invocation.credentials as DiscordNotificationCredentials;
    const botToken = await secretStore.get(symbolic.botToken.name);
    if (botToken === undefined || botToken.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        'A required Discord bot token is not available.'
      );
    }
    return { credentials: { botToken }, secretValues: [botToken] };
  }

  async prepare(
    _invocation: NotificationInvocation,
    credentials: ResolvedDiscordCredentials
  ): Promise<void> {
    if (this.#accountByToken.has(credentials.botToken)) return;
    const identity = await this.#transport.botIdentity(credentials.botToken);
    this.#accountByToken.set(credentials.botToken, identity.botId);
  }

  async deliver(
    invocation: NotificationInvocation,
    credentials: ResolvedDiscordCredentials
  ): Promise<ProviderMessageIdentity> {
    if (invocation.messageType !== 'deliver') {
      throw new Error('A Discord update cannot be delivered as a new message.');
    }
    const previous = this.#deliveredByIdempotencyKey.get(
      invocation.idempotencyKey
    );
    if (previous !== undefined) return previous;
    const accountId = this.#accountByToken.get(credentials.botToken);
    if (accountId === undefined) {
      throw new Error('Discord bot identity is unavailable.');
    }
    const identity = await this.#transport.sendMessage({
      botToken: credentials.botToken,
      accountId,
      conversationId: invocation.destination,
      text: approvalText(invocation),
      ...('mode' in invocation
        ? {}
        : { decisionCapability: invocation.decisionCapability }),
    });
    this.#deliveredByIdempotencyKey.set(invocation.idempotencyKey, identity);
    return identity;
  }

  async update(
    invocation: NotificationInvocation,
    credentials: ResolvedDiscordCredentials
  ): Promise<void> {
    if (
      invocation.messageType !== 'update' ||
      !('provider' in invocation.providerMessage) ||
      invocation.providerMessage.provider !== 'discord'
    ) {
      throw new Error('Discord received an invalid message update identity.');
    }
    await this.#transport.updateMessage({
      botToken: credentials.botToken,
      conversationId: invocation.providerMessage.conversationId,
      messageId: invocation.providerMessage.messageId,
      resolution: invocation.resolution,
    });
  }

  validMessageIdentity(value: unknown): value is ProviderMessageIdentity {
    return (
      typeof value === 'object' &&
      value !== null &&
      'provider' in value &&
      value.provider === 'discord' &&
      'accountId' in value &&
      typeof value.accountId === 'string' &&
      'conversationId' in value &&
      typeof value.conversationId === 'string' &&
      'messageId' in value &&
      typeof value.messageId === 'string'
    );
  }

  invalidMessageIdentityFailure(): NotificationProviderFailure {
    return {
      kind: 'request_invalid',
      code: 'WOML_DISCORD_RESPONSE_INVALID',
      message: 'Discord returned an invalid message identity.',
      retryable: false,
    };
  }

  safeFailure(
    error: unknown,
    resolvedValues: readonly string[]
  ): NotificationProviderFailure {
    if (error instanceof CommunicationProviderAdapterError) {
      return error.failure as NotificationProviderFailure;
    }
    if (error instanceof DiscordTransportError) {
      const kind = error.failure.kind === 'permission_denied'
        ? 'destination_invalid' as const
        : error.failure.kind;
      return {
        ...error.failure,
        kind,
        message: redact(error.failure.message, resolvedValues),
      };
    }
    if (error instanceof SecretStoreError) {
      return {
        kind: 'secret_not_found',
        code: 'WOML_SECRET_NOT_FOUND',
        message: 'A required Discord credential is not available.',
        retryable: false,
      };
    }
    return {
      kind: 'provider_unavailable',
      code: 'WOML_DISCORD_UNAVAILABLE',
      message: 'The Discord adapter failed without exposing provider details.',
      retryable: true,
    };
  }

  async close(): Promise<void> {
    await this.#transport.close();
  }
}
