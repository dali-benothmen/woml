import { createHash, randomUUID } from 'node:crypto';

import type {
  ApprovalDecision,
  InteractionMessage,
  SlackProviderMessageIdentity,
  SlackDeliveryRequest,
  SlackUpdateRequest,
} from './types';
import {
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
} from './types';
import { SlackTransportError, type SlackTransport } from './slack-transport';

export { SlackTransportError, type SlackTransport } from './slack-transport';

export interface FakeSlackMessage {
  readonly deliveryId: string;
  readonly destination: string;
  readonly providerMessage: SlackProviderMessageIdentity;
  readonly decisionCapability?: string;
  readonly message: string;
  readonly idempotencyKey: string;
  resolution?: SlackUpdateRequest['invocation']['resolution'];
}

export interface FakeSlackOptions {
  readonly emit: (message: InteractionMessage) => void | Promise<void>;
  readonly automaticDecision?: ApprovalDecision;
  readonly automaticActorId?: string;
  readonly automaticDelayMs?: number;
  readonly deliveryFailuresBeforeSuccess?: number;
  readonly failedDestinations?: readonly string[];
}

function stableSlackId(prefix: string, value: string): string {
  return `${prefix}${createHash('sha256').update(value).digest('hex').slice(0, 8).toUpperCase()}`;
}

export class FakeSlackTransport implements SlackTransport {
  readonly #emit: FakeSlackOptions['emit'];
  readonly #automaticDecision: ApprovalDecision | undefined;
  readonly #automaticActorId: string;
  readonly #automaticDelayMs: number;
  readonly #deliveryFailuresBeforeSuccess: number;
  readonly #failedDestinations: ReadonlySet<string>;
  readonly #connections = new Map<string, string>();
  readonly #messagesByDelivery = new Map<string, FakeSlackMessage>();
  readonly #messagesByIdempotency = new Map<string, FakeSlackMessage>();
  readonly #updates = new Set<string>();
  readonly #deliveryAttempts = new Map<string, number>();
  #messageSequence = 0;
  #closed = false;

  constructor(options: FakeSlackOptions) {
    this.#emit = options.emit;
    this.#automaticDecision = options.automaticDecision;
    this.#automaticActorId = options.automaticActorId ?? 'U12345678';
    this.#automaticDelayMs = options.automaticDelayMs ?? 0;
    this.#deliveryFailuresBeforeSuccess =
      options.deliveryFailuresBeforeSuccess ?? 0;
    this.#failedDestinations = new Set(options.failedDestinations ?? []);
  }

  get connectionCount(): number {
    return this.#connections.size;
  }

  messages(): readonly FakeSlackMessage[] {
    return [...this.#messagesByDelivery.values()];
  }

  async ensureConnection(
    appTokenReference: string,
    resolvedAppToken: string
  ): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    const existing = this.#connections.get(appTokenReference);
    if (existing !== undefined && existing !== resolvedAppToken) {
      throw new SlackTransportError({
        kind: 'provider_auth_failed',
        code: 'WOML_SLACK_AUTH_FAILED',
        message: 'The Slack app credential changed while its connection was active.',
        retryable: false,
      });
    }
    this.#connections.set(appTokenReference, resolvedAppToken);
  }

  async deliver(request: SlackDeliveryRequest): Promise<SlackProviderMessageIdentity> {
    if (this.#closed) throw this.#unavailable();
    if (this.#failedDestinations.has(request.invocation.destination)) {
      throw new SlackTransportError({
        kind: 'destination_invalid',
        code: 'WOML_SLACK_DESTINATION_INVALID',
        message: `The fake Slack destination ${request.invocation.destination} is unavailable.`,
        retryable: false,
      });
    }
    const attempts =
      (this.#deliveryAttempts.get(request.invocation.deliveryId) ?? 0) + 1;
    this.#deliveryAttempts.set(request.invocation.deliveryId, attempts);
    if (attempts <= this.#deliveryFailuresBeforeSuccess) {
      throw new SlackTransportError({
        kind: 'rate_limited',
        code: 'WOML_SLACK_RATE_LIMITED',
        message: 'The fake Slack transport is temporarily rate-limited.',
        retryable: true,
        retryAfterMs: 1,
      });
    }
    const existing = this.#messagesByIdempotency.get(
      request.invocation.idempotencyKey
    );
    if (existing !== undefined) return existing.providerMessage;
    this.#messageSequence += 1;
    const providerMessage: SlackProviderMessageIdentity = {
      workspaceId: stableSlackId('T', request.credentials.botToken),
      channelId: request.invocation.destination.startsWith('#')
        ? stableSlackId('C', request.invocation.destination)
        : request.invocation.destination,
      messageId: `${1_723_024_800 + this.#messageSequence}.${String(
        this.#messageSequence
      ).padStart(6, '0')}`,
    };
    const message: FakeSlackMessage = {
      deliveryId: request.invocation.deliveryId,
      destination: request.invocation.destination,
      providerMessage,
      ...(request.invocation.protocolVersion === 1
        ? { decisionCapability: request.invocation.decisionCapability }
        : {}),
      message:
        request.invocation.protocolVersion === 1
          ? request.invocation.message.approvalName
          : request.invocation.message,
      idempotencyKey: request.invocation.idempotencyKey,
    };
    this.#messagesByDelivery.set(request.invocation.deliveryId, message);
    this.#messagesByIdempotency.set(request.invocation.idempotencyKey, message);
    if (
      request.invocation.protocolVersion === 1 &&
      this.#automaticDecision !== undefined
    ) {
      setTimeout(() => {
        void this.click(
          request.invocation.deliveryId,
          this.#automaticDecision!,
          this.#automaticActorId
        );
      }, this.#automaticDelayMs);
    }
    return providerMessage;
  }

  async update(request: SlackUpdateRequest): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    if (this.#updates.has(request.invocation.idempotencyKey)) return;
    if (!('workspaceId' in request.invocation.providerMessage)) {
      throw new SlackTransportError({
        kind: 'update_failed',
        code: 'WOML_SLACK_UPDATE_FAILED',
        message: 'Slack received a non-Slack message identity.',
        retryable: false,
      });
    }
    const message = this.#messagesByDelivery.get(request.invocation.deliveryId);
    if (
      message !== undefined &&
      (message.providerMessage.workspaceId !==
        request.invocation.providerMessage.workspaceId ||
        message.providerMessage.channelId !==
          request.invocation.providerMessage.channelId ||
        message.providerMessage.messageId !==
          request.invocation.providerMessage.messageId)
    ) {
      throw new SlackTransportError({
        kind: 'update_failed',
        code: 'WOML_SLACK_UPDATE_FAILED',
        message: 'The Slack message identity does not match the delivered message.',
        retryable: false,
      });
    }
    if (message !== undefined) message.resolution = request.invocation.resolution;
    this.#updates.add(request.invocation.idempotencyKey);
  }

  async click(
    deliveryId: string,
    decision: ApprovalDecision,
    providerActorId = 'U12345678'
  ): Promise<void> {
    if (this.#closed) return;
    const message = this.#messagesByDelivery.get(deliveryId);
    if (
      message === undefined ||
      message.resolution !== undefined ||
      message.decisionCapability === undefined
    )
      return;
    await this.#emit({
      protocol: NOTIFICATION_PROVIDER_PROTOCOL,
      protocolVersion: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
      messageType: 'interaction',
      interactionId: `interaction_${randomUUID()}`,
      deliveryId,
      provider: 'slack',
      decisionCapability: message.decisionCapability,
      decision,
      providerActorId,
      occurredAt: new Date().toISOString(),
    });
  }

  async close(): Promise<void> {
    this.#closed = true;
    this.#connections.clear();
  }

  #unavailable(): SlackTransportError {
    return new SlackTransportError({
      kind: 'provider_unavailable',
      code: 'WOML_SLACK_UNAVAILABLE',
      message: 'The fake Slack transport is closed.',
      retryable: true,
    });
  }
}
