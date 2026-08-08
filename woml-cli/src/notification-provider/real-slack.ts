import { randomUUID } from 'node:crypto';

import { validProviderMessage } from './protocol';
import {
  SharedSlackTransport,
  type SharedSlackTransportOptions,
  type SlackEnvelope,
} from './shared-slack';
import { SlackTransportError, type SlackTransport } from './slack-transport';
import {
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type ApprovalDecision,
  type InteractionMessage,
  type NotificationProviderFailure,
  type ProviderMessageIdentity,
  type SlackDeliveryRequest,
  type SlackUpdateRequest,
} from './types';

const APPROVE_ACTION = 'woml_approval_approved';
const REJECT_ACTION = 'woml_approval_rejected';
const ACTION_BLOCK = 'woml_approval_actions';

interface SlackActionValue {
  readonly version: 1;
  readonly deliveryId: string;
  readonly decisionCapability: string;
  readonly decision: ApprovalDecision;
}

export interface RealSlackTransportOptions extends SharedSlackTransportOptions {
  readonly emit: (message: InteractionMessage) => void | Promise<void>;
  readonly sharedTransport?: SharedSlackTransport;
}

export type { SlackSocket } from './shared-slack';

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function safeText(value: string, maximum: number): string {
  return [...value].slice(0, maximum).join('');
}

function failure(
  kind: NotificationProviderFailure['kind'],
  code: string,
  message: string,
  retryable: boolean
): SlackTransportError {
  return new SlackTransportError({ kind, code, message, retryable });
}

function actionValue(
  request: SlackDeliveryRequest,
  decision: ApprovalDecision
): string {
  return JSON.stringify({
    version: 1,
    deliveryId: request.invocation.deliveryId,
    decisionCapability: request.invocation.decisionCapability,
    decision,
  } satisfies SlackActionValue);
}

function pendingBlocks(request: SlackDeliveryRequest): readonly unknown[] {
  const { message } = request.invocation;
  const details = [
    `Workflow: ${message.workflowId}`,
    ...(message.approvalDescription === undefined
      ? []
      : [message.approvalDescription]),
    ...(message.expiresAt === undefined
      ? []
      : [`Decision deadline: ${message.expiresAt}`]),
  ].join('\n');
  return [
    {
      type: 'header',
      text: {
        type: 'plain_text',
        text: safeText(message.approvalName, 150),
        emoji: true,
      },
    },
    {
      type: 'section',
      text: { type: 'plain_text', text: safeText(details, 3_000), emoji: true },
    },
    {
      type: 'actions',
      block_id: ACTION_BLOCK,
      elements: [
        {
          type: 'button',
          text: { type: 'plain_text', text: 'Approve', emoji: true },
          action_id: APPROVE_ACTION,
          style: 'primary',
          value: actionValue(request, 'approved'),
        },
        {
          type: 'button',
          text: { type: 'plain_text', text: 'Reject', emoji: true },
          action_id: REJECT_ACTION,
          style: 'danger',
          value: actionValue(request, 'rejected'),
        },
      ],
    },
  ];
}

function resolvedBlocks(request: SlackUpdateRequest): readonly unknown[] {
  const labels = {
    approved: ['Approved', 'This workflow approval was accepted.'],
    rejected: ['Rejected', 'This workflow approval was rejected.'],
    timeout_failed: [
      'Approval expired',
      'The approval deadline passed and the workflow failed.',
    ],
  } as const;
  const [title, detail] = labels[request.invocation.resolution];
  return [
    {
      type: 'header',
      text: { type: 'plain_text', text: title, emoji: true },
    },
    {
      type: 'section',
      text: { type: 'plain_text', text: detail, emoji: true },
    },
  ];
}

function parseActionValue(value: unknown): SlackActionValue | undefined {
  if (typeof value !== 'string' || value.length > 1_500) return undefined;
  try {
    const parsed: unknown = JSON.parse(value);
    if (
      !record(parsed) ||
      Object.keys(parsed).length !== 4 ||
      parsed.version !== 1 ||
      typeof parsed.deliveryId !== 'string' ||
      typeof parsed.decisionCapability !== 'string' ||
      (parsed.decision !== 'approved' && parsed.decision !== 'rejected')
    ) {
      return undefined;
    }
    return parsed as unknown as SlackActionValue;
  } catch {
    return undefined;
  }
}

export class RealSlackTransport implements SlackTransport {
  readonly #emit: RealSlackTransportOptions['emit'];
  readonly #shared: SharedSlackTransport;
  readonly #ownsShared: boolean;
  readonly #log: (message: string) => void;
  readonly #listenerId = `approval_${randomUUID()}`;
  readonly #unsubscribers = new Map<string, () => void>();
  readonly #deliveries = new Map<string, Promise<ProviderMessageIdentity>>();
  readonly #updates = new Map<string, Promise<void>>();
  #closed = false;

  constructor(options: RealSlackTransportOptions) {
    this.#emit = options.emit;
    this.#log = options.log ?? (() => {});
    this.#ownsShared = options.sharedTransport === undefined;
    this.#shared =
      options.sharedTransport ??
      new SharedSlackTransport({
        fetch: options.fetch,
        createWebSocket: options.createWebSocket,
        socketOpenTimeoutMs: options.socketOpenTimeoutMs,
        reconnectBaseDelayMs: options.reconnectBaseDelayMs,
        log: options.log,
      });
  }

  async ensureConnection(
    appTokenReference: string,
    resolvedAppToken: string
  ): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    if (!this.#unsubscribers.has(appTokenReference)) {
      this.#unsubscribers.set(
        appTokenReference,
        this.#shared.subscribe(appTokenReference, this.#listenerId, envelope =>
          this.#onEnvelope(envelope)
        )
      );
    }
    await this.#shared.ensureConnection(appTokenReference, resolvedAppToken);
  }

  async deliver(request: SlackDeliveryRequest): Promise<ProviderMessageIdentity> {
    if (this.#closed) throw this.#unavailable();
    const key = request.invocation.idempotencyKey;
    const existing = this.#deliveries.get(key);
    if (existing !== undefined) return await existing;
    const delivery = this.#deliver(request).catch(error => {
      this.#deliveries.delete(key);
      throw error;
    });
    this.#deliveries.set(key, delivery);
    return await delivery;
  }

  async update(request: SlackUpdateRequest): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    const key = request.invocation.idempotencyKey;
    const existing = this.#updates.get(key);
    if (existing !== undefined) return await existing;
    const update = this.#update(request).catch(error => {
      this.#updates.delete(key);
      throw error;
    });
    this.#updates.set(key, update);
    await update;
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    for (const unsubscribe of this.#unsubscribers.values()) unsubscribe();
    this.#unsubscribers.clear();
    if (this.#ownsShared) await this.#shared.close();
  }

  async #deliver(
    request: SlackDeliveryRequest
  ): Promise<ProviderMessageIdentity> {
    const botReference = request.invocation.credentials.botToken.name;
    const identity = await this.#shared.botIdentity(
      botReference,
      request.credentials.botToken
    );
    const channel = await this.#shared.resolveDestination(
      request.invocation.destination,
      botReference,
      request.credentials.botToken
    );
    const response = await this.#shared.api(
      'chat.postMessage',
      request.credentials.botToken,
      {
        channel,
        text: `Approval required: ${safeText(request.invocation.message.approvalName, 300)}`,
        blocks: pendingBlocks(request),
        unfurl_links: false,
        unfurl_media: false,
      },
      'delivery'
    );
    const providerMessage = {
      workspaceId: identity.teamId,
      channelId: response.channel,
      messageId: response.ts,
    };
    if (!validProviderMessage(providerMessage)) {
      throw failure(
        'delivery_ambiguous',
        'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
        'Slack created an unrecognized response; WOML will not replay the message automatically.',
        false
      );
    }
    this.#log(
      `Slack approval sent to ${request.invocation.destination}; waiting for Approve or Reject.`
    );
    return providerMessage;
  }

  async #update(request: SlackUpdateRequest): Promise<void> {
    const botReference = request.invocation.credentials.botToken.name;
    const identity = await this.#shared.botIdentity(
      botReference,
      request.credentials.botToken
    );
    if (identity.teamId !== request.invocation.providerMessage.workspaceId) {
      throw failure(
        'update_failed',
        'WOML_SLACK_UPDATE_FAILED',
        'The Slack message belongs to a different workspace.',
        false
      );
    }
    const title =
      request.invocation.resolution === 'approved'
        ? 'Approved'
        : request.invocation.resolution === 'rejected'
          ? 'Rejected'
          : 'Approval expired';
    await this.#shared.api(
      'chat.update',
      request.credentials.botToken,
      {
        channel: request.invocation.providerMessage.channelId,
        ts: request.invocation.providerMessage.messageId,
        text: title,
        blocks: resolvedBlocks(request),
      },
      'update'
    );
  }

  async #onEnvelope(envelope: SlackEnvelope): Promise<void> {
    const body = envelope.body;
    if (body.type !== 'interactive' || !record(body.payload)) return;
    envelope.acknowledge();
    const payload = body.payload;
    if (payload.type !== 'block_actions' || !Array.isArray(payload.actions)) {
      return;
    }
    const action = payload.actions[0];
    if (!record(action) || action.block_id !== ACTION_BLOCK) return;
    const expectedDecision =
      action.action_id === APPROVE_ACTION
        ? 'approved'
        : action.action_id === REJECT_ACTION
          ? 'rejected'
          : undefined;
    const value = parseActionValue(action.value);
    if (value === undefined || value.decision !== expectedDecision) return;
    if (
      !record(payload.user) ||
      typeof payload.user.id !== 'string' ||
      !/^U[A-Z0-9]{8,31}$/.test(payload.user.id)
    ) {
      return;
    }
    await this.#emit({
      protocol: NOTIFICATION_PROVIDER_PROTOCOL,
      protocolVersion: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
      messageType: 'interaction',
      interactionId:
        envelope.envelopeId === undefined
          ? `slack_${randomUUID()}`
          : safeText(`slack_${envelope.envelopeId}`, 320),
      deliveryId: value.deliveryId,
      provider: 'slack',
      decisionCapability: value.decisionCapability,
      decision: value.decision,
      providerActorId: payload.user.id,
      occurredAt: new Date().toISOString(),
    });
  }

  #unavailable(): SlackTransportError {
    return failure(
      'provider_unavailable',
      'WOML_SLACK_UNAVAILABLE',
      'The Slack approval adapter is closed.',
      true
    );
  }
}
