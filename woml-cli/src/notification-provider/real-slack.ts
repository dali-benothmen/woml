import { randomUUID } from 'node:crypto';

import { validProviderMessage } from './protocol';
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

const SLACK_API = 'https://slack.com/api/';
const APPROVE_ACTION = 'woml_approval_approved';
const REJECT_ACTION = 'woml_approval_rejected';
const ACTION_BLOCK = 'woml_approval_actions';
const SOCKET_OPEN_TIMEOUT_MS = 10_000;

type SlackEffect = 'none' | 'delivery' | 'update';

interface SlackSocketEventMap {
  open: Event;
  close: CloseEvent;
  error: Event;
  message: MessageEvent;
}

export interface SlackSocket {
  readonly readyState: number;
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener<K extends keyof SlackSocketEventMap>(
    type: K,
    listener: (event: SlackSocketEventMap[K]) => void,
    options?: AddEventListenerOptions
  ): void;
}

interface SlackConnection {
  readonly reference: string;
  readonly token: string;
  socket?: SlackSocket;
  opening?: Promise<void>;
  reconnectTimer?: ReturnType<typeof setTimeout>;
  reconnectAttempt: number;
}

interface SlackBotIdentity {
  readonly token: string;
  readonly teamId: string;
}

interface SlackActionValue {
  readonly version: 1;
  readonly deliveryId: string;
  readonly decisionCapability: string;
  readonly decision: ApprovalDecision;
}

export interface RealSlackTransportOptions {
  readonly emit: (message: InteractionMessage) => void | Promise<void>;
  readonly fetch?: typeof globalThis.fetch;
  readonly createWebSocket?: (url: string) => SlackSocket;
  readonly socketOpenTimeoutMs?: number;
  readonly reconnectBaseDelayMs?: number;
  readonly log?: (message: string) => void;
}

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
  retryable: boolean,
  retryAfterMs?: number
): SlackTransportError {
  return new SlackTransportError({
    kind,
    code,
    message,
    retryable,
    ...(retryAfterMs === undefined ? {} : { retryAfterMs }),
  });
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
      text: {
        type: 'plain_text',
        text: safeText(details, 3_000),
        emoji: true,
      },
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

function slackError(
  error: string,
  effect: SlackEffect,
  retryAfterMs?: number
): SlackTransportError {
  if (error === 'ratelimited' || error === 'rate_limited') {
    return failure(
      'rate_limited',
      'WOML_SLACK_RATE_LIMITED',
      'Slack rate-limited the provider request.',
      true,
      retryAfterMs
    );
  }
  if (
    [
      'invalid_auth',
      'not_authed',
      'account_inactive',
      'token_expired',
      'token_revoked',
      'not_allowed_token_type',
      'missing_scope',
      'no_permission',
    ].includes(error)
  ) {
    return failure(
      'provider_auth_failed',
      error === 'missing_scope' || error === 'no_permission'
        ? 'WOML_SLACK_PERMISSION_DENIED'
        : 'WOML_SLACK_AUTH_FAILED',
      error === 'missing_scope' || error === 'no_permission'
        ? 'The Slack app does not have the required permission.'
        : 'Slack rejected a configured credential.',
      false
    );
  }
  if (
    [
      'channel_not_found',
      'not_in_channel',
      'is_archived',
      'restricted_action',
    ].includes(error)
  ) {
    return failure(
      'destination_invalid',
      'WOML_SLACK_DESTINATION_INVALID',
      'Slack could not deliver to the configured channel.',
      false
    );
  }
  if (effect === 'delivery' && ['fatal_error', 'internal_error'].includes(error)) {
    return failure(
      'delivery_ambiguous',
      'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
      'Slack did not confirm whether the message was created; WOML will not replay it automatically.',
      false
    );
  }
  return failure(
    effect === 'update' ? 'update_failed' : 'provider_unavailable',
    effect === 'update'
      ? 'WOML_SLACK_UPDATE_FAILED'
      : 'WOML_SLACK_UNAVAILABLE',
    effect === 'update'
      ? 'Slack could not update the approval message.'
      : 'Slack could not complete the provider request.',
    true
  );
}

export class RealSlackTransport implements SlackTransport {
  readonly #emit: RealSlackTransportOptions['emit'];
  readonly #fetch: typeof globalThis.fetch;
  readonly #createWebSocket: (url: string) => SlackSocket;
  readonly #socketOpenTimeoutMs: number;
  readonly #reconnectBaseDelayMs: number;
  readonly #log: (message: string) => void;
  readonly #connections = new Map<string, SlackConnection>();
  readonly #botIdentities = new Map<string, Promise<SlackBotIdentity>>();
  readonly #channelIds = new Map<string, Promise<string>>();
  readonly #deliveries = new Map<string, Promise<ProviderMessageIdentity>>();
  readonly #updates = new Map<string, Promise<void>>();
  #closed = false;

  constructor(options: RealSlackTransportOptions) {
    this.#emit = options.emit;
    this.#fetch = options.fetch ?? globalThis.fetch;
    this.#createWebSocket =
      options.createWebSocket ?? (url => new WebSocket(url));
    this.#socketOpenTimeoutMs =
      options.socketOpenTimeoutMs ?? SOCKET_OPEN_TIMEOUT_MS;
    this.#reconnectBaseDelayMs = options.reconnectBaseDelayMs ?? 500;
    this.#log = options.log ?? (() => {});
  }

  async ensureConnection(
    appTokenReference: string,
    resolvedAppToken: string
  ): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    if (!resolvedAppToken.startsWith('xapp-')) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The configured Slack app token is invalid.',
        false
      );
    }
    let connection = this.#connections.get(appTokenReference);
    if (connection === undefined) {
      connection = {
        reference: appTokenReference,
        token: resolvedAppToken,
        reconnectAttempt: 0,
      };
      this.#connections.set(appTokenReference, connection);
    } else if (connection.token !== resolvedAppToken) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The Slack app token changed while its Socket Mode connection was active.',
        false
      );
    }
    if (connection.socket?.readyState === 1) return;
    if (connection.opening === undefined) {
      connection.opening = this.#openConnection(connection).finally(() => {
        connection!.opening = undefined;
      });
    }
    await connection.opening;
  }

  async deliver(request: SlackDeliveryRequest): Promise<ProviderMessageIdentity> {
    if (this.#closed) throw this.#unavailable();
    const key = request.invocation.idempotencyKey;
    const existing = this.#deliveries.get(key);
    if (existing !== undefined) return existing;
    const delivery = this.#deliver(request).catch(error => {
      this.#deliveries.delete(key);
      throw error;
    });
    this.#deliveries.set(key, delivery);
    return delivery;
  }

  async update(request: SlackUpdateRequest): Promise<void> {
    if (this.#closed) throw this.#unavailable();
    const key = request.invocation.idempotencyKey;
    const existing = this.#updates.get(key);
    if (existing !== undefined) return existing;
    const update = this.#update(request).catch(error => {
      this.#updates.delete(key);
      throw error;
    });
    this.#updates.set(key, update);
    return update;
  }

  async close(): Promise<void> {
    this.#closed = true;
    for (const connection of this.#connections.values()) {
      if (connection.reconnectTimer !== undefined) {
        clearTimeout(connection.reconnectTimer);
      }
      connection.socket?.close(1000, 'WOML provider host shutting down');
    }
    this.#connections.clear();
    this.#botIdentities.clear();
    this.#channelIds.clear();
  }

  async #deliver(
    request: SlackDeliveryRequest
  ): Promise<ProviderMessageIdentity> {
    const botReference = request.invocation.credentials.botToken.name;
    const identity = await this.#botIdentity(
      botReference,
      request.credentials.botToken
    );
    const channel = await this.#resolveDestination(
      request.invocation.destination,
      botReference,
      request.credentials.botToken
    );
    const response = await this.#api(
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
    const identity = await this.#botIdentity(
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
    await this.#api(
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

  async #botIdentity(reference: string, token: string): Promise<SlackBotIdentity> {
    if (!token.startsWith('xoxb-')) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The configured Slack bot token is invalid.',
        false
      );
    }
    const existing = this.#botIdentities.get(reference);
    if (existing !== undefined) {
      const identity = await existing;
      if (identity.token !== token) {
        throw failure(
          'provider_auth_failed',
          'WOML_SLACK_AUTH_FAILED',
          'The Slack bot token changed while the provider host was active.',
          false
        );
      }
      return identity;
    }
    const pending = this.#api('auth.test', token, {}, 'none').then(response => {
      if (typeof response.team_id !== 'string' || !/^T[A-Z0-9]{8,31}$/.test(response.team_id)) {
        throw failure(
          'request_invalid',
          'WOML_SLACK_RESPONSE_INVALID',
          'Slack returned an invalid workspace identity.',
          false
        );
      }
      return { token, teamId: response.team_id };
    });
    this.#botIdentities.set(reference, pending);
    return pending;
  }

  async #resolveDestination(
    destination: string,
    botReference: string,
    token: string
  ): Promise<string> {
    if (!destination.startsWith('#')) return destination;
    const name = destination.slice(1);
    const cacheKey = `${botReference}\0${name}`;
    const existing = this.#channelIds.get(cacheKey);
    if (existing !== undefined) return existing;
    const pending = this.#findChannel(name, token);
    this.#channelIds.set(cacheKey, pending);
    return pending;
  }

  async #findChannel(name: string, token: string): Promise<string> {
    let cursor: string | undefined;
    do {
      const response = await this.#api(
        'conversations.list',
        token,
        {
          types: 'public_channel,private_channel',
          exclude_archived: true,
          limit: 200,
          ...(cursor === undefined ? {} : { cursor }),
        },
        'none'
      );
      if (!Array.isArray(response.channels)) {
        throw failure(
          'request_invalid',
          'WOML_SLACK_RESPONSE_INVALID',
          'Slack returned an invalid channel list.',
          false
        );
      }
      for (const value of response.channels) {
        if (
          record(value) &&
          value.name === name &&
          typeof value.id === 'string' &&
          /^[CG][A-Z0-9]{8,31}$/.test(value.id)
        ) {
          return value.id;
        }
      }
      cursor =
        record(response.response_metadata) &&
        typeof response.response_metadata.next_cursor === 'string' &&
        response.response_metadata.next_cursor.length > 0
          ? response.response_metadata.next_cursor
          : undefined;
    } while (cursor !== undefined);
    throw failure(
      'destination_invalid',
      'WOML_SLACK_DESTINATION_INVALID',
      `Slack channel #${name} was not found or is not visible to the app.`,
      false
    );
  }

  async #api(
    method: string,
    token: string,
    body: Record<string, unknown>,
    effect: SlackEffect
  ): Promise<Record<string, unknown>> {
    let response: Response;
    try {
      response = await this.#fetch(`${SLACK_API}${method}`, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json; charset=utf-8',
        },
        body: JSON.stringify(body),
      });
    } catch {
      if (effect === 'delivery') {
        throw failure(
          'delivery_ambiguous',
          'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
          'The Slack delivery connection ended without a definitive response; WOML will not replay it automatically.',
          false
        );
      }
      throw failure(
        effect === 'update' ? 'update_failed' : 'provider_unavailable',
        effect === 'update'
          ? 'WOML_SLACK_UPDATE_FAILED'
          : 'WOML_SLACK_UNAVAILABLE',
        effect === 'update'
          ? 'The Slack message update could not be confirmed.'
          : 'Slack is currently unavailable.',
        true
      );
    }
    const retryAfter = Number(response.headers.get('retry-after'));
    const retryAfterMs = Number.isFinite(retryAfter)
      ? Math.max(0, retryAfter * 1_000)
      : undefined;
    if (response.status === 429) {
      throw slackError('ratelimited', effect, retryAfterMs);
    }
    if (!response.ok) {
      if (effect === 'delivery' && response.status >= 500) {
        throw failure(
          'delivery_ambiguous',
          'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
          'Slack did not confirm whether the message was created; WOML will not replay it automatically.',
          false
        );
      }
      throw slackError('service_unavailable', effect, retryAfterMs);
    }
    let value: unknown;
    try {
      value = await response.json();
    } catch {
      throw effect === 'delivery'
        ? failure(
            'delivery_ambiguous',
            'WOML_NOTIFICATION_DELIVERY_AMBIGUOUS',
            'Slack returned an unreadable delivery response; WOML will not replay it automatically.',
            false
          )
        : failure(
            effect === 'update' ? 'update_failed' : 'request_invalid',
            effect === 'update'
              ? 'WOML_SLACK_UPDATE_FAILED'
              : 'WOML_SLACK_RESPONSE_INVALID',
            'Slack returned an unreadable response.',
            effect === 'update'
          );
    }
    if (!record(value)) {
      throw slackError('invalid_response', effect);
    }
    if (value.ok !== true) {
      throw slackError(
        typeof value.error === 'string' ? value.error : 'invalid_response',
        effect,
        retryAfterMs
      );
    }
    return value;
  }

  async #openConnection(connection: SlackConnection): Promise<void> {
    const response = await this.#api(
      'apps.connections.open',
      connection.token,
      {},
      'none'
    );
    if (typeof response.url !== 'string' || !response.url.startsWith('wss://')) {
      throw failure(
        'request_invalid',
        'WOML_SLACK_RESPONSE_INVALID',
        'Slack returned an invalid Socket Mode endpoint.',
        false
      );
    }
    const socket = this.#createWebSocket(response.url);
    connection.socket = socket;
    socket.addEventListener('message', event => {
      void this.#onSocketMessage(connection, socket, event.data);
    });
    socket.addEventListener('close', () => {
      if (connection.socket === socket) {
        connection.socket = undefined;
        this.#scheduleReconnect(connection);
      }
    });
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        socket.close(1011, 'Socket Mode open timeout');
        reject(
          failure(
            'provider_unavailable',
            'WOML_SLACK_UNAVAILABLE',
            'Slack Socket Mode did not connect before its deadline.',
            true
          )
        );
      }, this.#socketOpenTimeoutMs);
      socket.addEventListener(
        'open',
        () => {
          clearTimeout(timer);
          connection.reconnectAttempt = 0;
          resolve();
        },
        { once: true }
      );
      socket.addEventListener(
        'error',
        () => {
          clearTimeout(timer);
          reject(
            failure(
              'provider_unavailable',
              'WOML_SLACK_UNAVAILABLE',
              'Slack Socket Mode could not establish a connection.',
              true
            )
          );
        },
        { once: true }
      );
    });
  }

  #scheduleReconnect(connection: SlackConnection): void {
    if (this.#closed || connection.reconnectTimer !== undefined) return;
    const delay = Math.min(
      this.#reconnectBaseDelayMs * 2 ** connection.reconnectAttempt,
      30_000
    );
    connection.reconnectAttempt += 1;
    connection.reconnectTimer = setTimeout(() => {
      connection.reconnectTimer = undefined;
      if (this.#closed || connection.opening !== undefined) return;
      connection.opening = this.#openConnection(connection)
        .catch(() => {
          this.#scheduleReconnect(connection);
        })
        .finally(() => {
          connection.opening = undefined;
        });
    }, delay);
  }

  async #onSocketMessage(
    connection: SlackConnection,
    socket: SlackSocket,
    raw: unknown
  ): Promise<void> {
    if (typeof raw !== 'string') return;
    let envelope: unknown;
    try {
      envelope = JSON.parse(raw);
    } catch {
      return;
    }
    if (!record(envelope)) return;
    if (typeof envelope.envelope_id === 'string') {
      socket.send(JSON.stringify({ envelope_id: envelope.envelope_id }));
    }
    if (envelope.type === 'disconnect') {
      if (connection.socket === socket) {
        connection.socket = undefined;
        socket.close(1000, 'Slack requested Socket Mode refresh');
        this.#scheduleReconnect(connection);
      }
      return;
    }
    if (envelope.type !== 'interactive' || !record(envelope.payload)) return;
    const payload = envelope.payload;
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
        typeof envelope.envelope_id === 'string'
          ? safeText(`slack_${envelope.envelope_id}`, 320)
          : `slack_${randomUUID()}`,
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
      'The Slack transport is closed.',
      true
    );
  }
}
