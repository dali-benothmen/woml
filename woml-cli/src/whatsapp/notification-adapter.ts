import type { CommunicationNotificationAdapter } from '../communication-provider';
import { SecretStoreError, type SecretStore } from '../secrets';
import type {
  NotificationInvocation,
  NotificationProviderFailure,
  ProviderMessageIdentity,
  WhatsAppNotificationCredentials,
} from '../notification-provider/types';
import {
  SharedWhatsAppTransport,
  WhatsAppTransportError,
} from './transport';

interface ResolvedWhatsAppCredentials {
  readonly accessToken: string;
  readonly phoneNumberId: string;
}

function redact(message: string, values: readonly string[]): string {
  let result = message;
  for (const value of values) {
    if (value.length > 0) result = result.split(value).join('[REDACTED]');
  }
  return result.slice(0, 1024) || 'The WhatsApp provider failed safely.';
}

export class WhatsAppNotificationAdapter
  implements CommunicationNotificationAdapter<
    NotificationInvocation,
    ResolvedWhatsAppCredentials,
    ProviderMessageIdentity,
    NotificationProviderFailure
  >
{
  readonly provider = 'whatsapp' as const;
  readonly #transport: SharedWhatsAppTransport;
  readonly #deliveredByIdempotencyKey = new Map<string, ProviderMessageIdentity>();

  constructor(transport: SharedWhatsAppTransport) {
    this.#transport = transport;
  }

  async resolveCredentials(
    secretStore: SecretStore,
    invocation: NotificationInvocation
  ): Promise<{
    readonly credentials: ResolvedWhatsAppCredentials;
    readonly secretValues: readonly string[];
  }> {
    const symbolic = invocation.credentials as WhatsAppNotificationCredentials;
    const accessToken = await secretStore.get(symbolic.accessToken.name);
    if (accessToken === undefined || accessToken.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        'A required WhatsApp access token is not available.'
      );
    }
    return {
      credentials: {
        accessToken,
        phoneNumberId: symbolic.phoneNumberId,
      },
      secretValues: [accessToken],
    };
  }

  async prepare(): Promise<void> {}

  async deliver(
    invocation: NotificationInvocation,
    credentials: ResolvedWhatsAppCredentials
  ): Promise<ProviderMessageIdentity> {
    if (invocation.messageType !== 'deliver') {
      throw new Error('A WhatsApp update cannot be delivered as a new message.');
    }
    const previous = this.#deliveredByIdempotencyKey.get(invocation.idempotencyKey);
    if (previous !== undefined) return previous;
    if (invocation.templateName === undefined || invocation.language === undefined) {
      throw new Error('WhatsApp delivery is missing approved-template metadata.');
    }
    const approval = !('mode' in invocation);
    const parameters = approval
      ? [
          invocation.message.approvalName,
          invocation.message.approvalDescription ?? '',
          invocation.message.workflowId,
          invocation.message.expiresAt ?? 'No deadline',
        ]
      : [invocation.message];
    const identity = await this.#transport.sendTemplate({
      accessToken: credentials.accessToken,
      phoneNumberId: credentials.phoneNumberId,
      conversationId: invocation.destination,
      templateName: invocation.templateName,
      language: invocation.language,
      parameters,
      ...(approval
        ? { decisionCapability: invocation.decisionCapability }
        : {}),
    });
    this.#deliveredByIdempotencyKey.set(invocation.idempotencyKey, identity);
    return identity;
  }

  async update(): Promise<void> {
    // Cloud API template messages cannot be edited after delivery. The durable
    // decision is authoritative; ACP8 will provide provider-specific follow-up.
  }

  validMessageIdentity(value: unknown): value is ProviderMessageIdentity {
    return typeof value === 'object' && value !== null &&
      'provider' in value && value.provider === 'whatsapp' &&
      'accountId' in value && typeof value.accountId === 'string' &&
      'conversationId' in value && typeof value.conversationId === 'string' &&
      'messageId' in value && typeof value.messageId === 'string';
  }

  invalidMessageIdentityFailure(): NotificationProviderFailure {
    return {
      kind: 'request_invalid',
      code: 'WOML_WHATSAPP_RESPONSE_INVALID',
      message: 'WhatsApp returned an invalid message identity.',
      retryable: false,
    };
  }

  safeFailure(
    error: unknown,
    resolvedValues: readonly string[]
  ): NotificationProviderFailure {
    if (error instanceof WhatsAppTransportError) {
      return { ...error.failure, message: redact(error.failure.message, resolvedValues) };
    }
    if (error instanceof SecretStoreError) {
      return {
        kind: 'secret_not_found',
        code: 'WOML_SECRET_NOT_FOUND',
        message: 'A required WhatsApp credential is not available.',
        retryable: false,
      };
    }
    return {
      kind: 'provider_unavailable',
      code: 'WOML_WHATSAPP_UNAVAILABLE',
      message: 'The WhatsApp adapter failed without exposing provider details.',
      retryable: true,
    };
  }

  async close(): Promise<void> {
    await this.#transport.close();
  }
}

