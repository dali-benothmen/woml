import { SlackTransportError } from './slack-transport';
import type { SlackTransportFailure } from './types';

const SLACK_API = 'https://slack.com/api/';
const SOCKET_OPEN_TIMEOUT_MS = 10_000;
const DIAGNOSTIC_SLACK_SCOPES = new Set([
  'connections:write',
  'chat:write',
  'chat:write.public',
  'app_mentions:read',
  'channels:read',
  'channels:history',
  'groups:read',
  'groups:history',
  'im:read',
  'im:history',
  'mpim:read',
  'mpim:history',
]);

export type SlackEffect = 'none' | 'delivery' | 'update';

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

export interface SlackEnvelope {
  readonly body: Readonly<Record<string, unknown>>;
  readonly envelopeId?: string;
  acknowledge(): void;
}

export type SlackEnvelopeListener = (
  envelope: SlackEnvelope
) => void | Promise<void>;

interface SlackConnection {
  readonly token: string;
  readonly references: Set<string>;
  readonly listeners: Map<string, SlackListenerRegistration>;
  socket?: SlackSocket;
  opening?: Promise<void>;
  reconnectTimer?: ReturnType<typeof setTimeout>;
  reconnectAttempt: number;
}

interface SlackListenerRegistration {
  readonly listener: SlackEnvelopeListener;
  readonly references: Set<string>;
}

interface SlackBotIdentity {
  readonly token: string;
  readonly teamId: string;
}

export interface SharedSlackTransportOptions {
  readonly fetch?: typeof globalThis.fetch;
  readonly createWebSocket?: (url: string) => SlackSocket;
  readonly socketOpenTimeoutMs?: number;
  readonly reconnectBaseDelayMs?: number;
  readonly log?: (message: string) => void;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function safeSlackScopes(value: unknown): readonly string[] {
  if (typeof value !== 'string' || value.length > 1_024) return [];
  const scopes = value
    .split(',')
    .map(scope => scope.trim())
    .filter(scope => DIAGNOSTIC_SLACK_SCOPES.has(scope));
  return [...new Set(scopes)].slice(0, 32);
}

function failure(
  kind: SlackTransportFailure['kind'],
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

function slackError(
  error: string,
  effect: SlackEffect,
  retryAfterMs?: number,
  diagnostic: {
    readonly method?: string;
    readonly needed?: unknown;
    readonly provided?: unknown;
  } = {}
): SlackTransportError {
  if (error === 'ratelimited' || error === 'rate_limited') {
    return failure(
      'rate_limited',
      'WOML_SLACK_RATE_LIMITED',
      'Slack rate-limited the request.',
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
    const missingScopes = safeSlackScopes(diagnostic.needed);
    const grantedScopes = safeSlackScopes(diagnostic.provided);
    const operation =
      diagnostic.method !== undefined &&
      /^[a-z][a-zA-Z0-9.]{0,79}$/.test(diagnostic.method)
        ? diagnostic.method
        : undefined;
    const permissionMessage = [
      operation === undefined
        ? 'The Slack app does not have a required permission.'
        : `Slack operation ${operation} needs additional app permissions.`,
      ...(missingScopes.length === 0
        ? []
        : [`Missing scopes: ${missingScopes.join(', ')}.`]),
      ...(grantedScopes.length === 0
        ? []
        : [`Granted scopes: ${grantedScopes.join(', ')}.`]),
      'Add the missing Bot Token Scopes and reinstall the Slack app to the workspace.',
    ].join(' ');
    return failure(
      'provider_auth_failed',
      error === 'missing_scope' || error === 'no_permission'
        ? 'WOML_SLACK_PERMISSION_DENIED'
        : 'WOML_SLACK_AUTH_FAILED',
      error === 'missing_scope' || error === 'no_permission'
        ? permissionMessage
        : 'Slack rejected a configured credential.',
      false
    );
  }
  if (
    ['channel_not_found', 'not_in_channel', 'is_archived', 'restricted_action'].includes(
      error
    )
  ) {
    return failure(
      'destination_invalid',
      'WOML_SLACK_DESTINATION_INVALID',
      'Slack could not access the configured channel.',
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
      : 'Slack could not complete the request.',
    true
  );
}

export class SharedSlackTransport {
  readonly #fetch: typeof globalThis.fetch;
  readonly #createWebSocket: (url: string) => SlackSocket;
  readonly #socketOpenTimeoutMs: number;
  readonly #reconnectBaseDelayMs: number;
  readonly #log: (message: string) => void;
  readonly #connectionsByReference = new Map<string, SlackConnection>();
  readonly #connectionsByToken = new Map<string, SlackConnection>();
  readonly #pendingListeners = new Map<
    string,
    Map<string, SlackEnvelopeListener>
  >();
  readonly #botIdentities = new Map<string, Promise<SlackBotIdentity>>();
  readonly #botReferences = new Map<string, string>();
  readonly #channelIds = new Map<string, Promise<string>>();
  #closed = false;

  constructor(options: SharedSlackTransportOptions = {}) {
    this.#fetch = options.fetch ?? globalThis.fetch;
    this.#createWebSocket =
      options.createWebSocket ?? (url => new WebSocket(url));
    this.#socketOpenTimeoutMs =
      options.socketOpenTimeoutMs ?? SOCKET_OPEN_TIMEOUT_MS;
    this.#reconnectBaseDelayMs = options.reconnectBaseDelayMs ?? 500;
    this.#log = options.log ?? (() => {});
  }

  subscribe(
    appTokenReference: string,
    listenerId: string,
    listener: SlackEnvelopeListener
  ): () => void {
    if (this.#closed) throw this.#unavailable();
    const connection = this.#connectionsByReference.get(appTokenReference);
    if (connection === undefined) {
      const listeners =
        this.#pendingListeners.get(appTokenReference) ??
        new Map<string, SlackEnvelopeListener>();
      if (listeners.has(listenerId)) {
        throw new Error(`Slack listener "${listenerId}" is already registered.`);
      }
      listeners.set(listenerId, listener);
      this.#pendingListeners.set(appTokenReference, listeners);
    } else {
      const registration = connection.listeners.get(listenerId);
      if (registration === undefined) {
        connection.listeners.set(listenerId, {
          listener,
          references: new Set([appTokenReference]),
        });
      } else {
        registration.references.add(appTokenReference);
      }
    }
    return () => {
      const active = this.#connectionsByReference.get(appTokenReference);
      const registration = active?.listeners.get(listenerId);
      registration?.references.delete(appTokenReference);
      if (registration?.references.size === 0) {
        active?.listeners.delete(listenerId);
      }
      const pending = this.#pendingListeners.get(appTokenReference);
      pending?.delete(listenerId);
      if (pending?.size === 0) this.#pendingListeners.delete(appTokenReference);
    };
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
    let connection = this.#connectionsByReference.get(appTokenReference);
    if (connection !== undefined && connection.token !== resolvedAppToken) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The Slack app token changed while its Socket Mode connection was active.',
        false
      );
    }
    if (connection === undefined) {
      connection = this.#connectionsByToken.get(resolvedAppToken);
      if (connection === undefined) {
        connection = {
          token: resolvedAppToken,
          references: new Set(),
          listeners: new Map(),
          reconnectAttempt: 0,
        };
        this.#connectionsByToken.set(resolvedAppToken, connection);
      }
      connection.references.add(appTokenReference);
      const pending = this.#pendingListeners.get(appTokenReference);
      if (pending !== undefined) {
        for (const [id, listener] of pending) {
          const registration = connection.listeners.get(id);
          if (registration === undefined) {
            connection.listeners.set(id, {
              listener,
              references: new Set([appTokenReference]),
            });
          } else {
            registration.references.add(appTokenReference);
          }
        }
        this.#pendingListeners.delete(appTokenReference);
      }
      this.#connectionsByReference.set(appTokenReference, connection);
    }
    if (connection.socket?.readyState === 1) return;
    if (connection.opening === undefined) {
      connection.opening = this.#openConnection(connection).finally(() => {
        connection!.opening = undefined;
      });
    }
    await connection.opening;
  }

  async botIdentity(reference: string, token: string): Promise<SlackBotIdentity> {
    if (!token.startsWith('xoxb-')) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The configured Slack bot token is invalid.',
        false
      );
    }
    const boundToken = this.#botReferences.get(reference);
    if (boundToken !== undefined && boundToken !== token) {
      throw failure(
        'provider_auth_failed',
        'WOML_SLACK_AUTH_FAILED',
        'The Slack bot token changed while the Slack host was active.',
        false
      );
    }
    this.#botReferences.set(reference, token);
    const existing = this.#botIdentities.get(token);
    if (existing !== undefined) return await existing;
    const pending = this.api('auth.test', token, {}, 'none').then(response => {
      if (
        typeof response.team_id !== 'string' ||
        !/^T[A-Z0-9]{8,31}$/.test(response.team_id)
      ) {
        throw failure(
          'request_invalid',
          'WOML_SLACK_RESPONSE_INVALID',
          'Slack returned an invalid workspace identity.',
          false
        );
      }
      return { token, teamId: response.team_id };
    });
    this.#botIdentities.set(token, pending);
    return await pending;
  }

  async resolveDestination(
    destination: string,
    botReference: string,
    token: string
  ): Promise<string> {
    if (!destination.startsWith('#')) return destination;
    const name = destination.slice(1);
    const cacheKey = `${token}\0${name}`;
    const existing = this.#channelIds.get(cacheKey);
    if (existing !== undefined) return await existing;
    const pending = this.#findChannel(name, token);
    this.#channelIds.set(cacheKey, pending);
    return await pending;
  }

  async api(
    method: string,
    token: string,
    body: Record<string, unknown>,
    effect: SlackEffect
  ): Promise<Record<string, unknown>> {
    if (this.#closed) throw this.#unavailable();
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
    if (!record(value)) throw slackError('invalid_response', effect);
    if (value.ok !== true) {
      throw slackError(
        typeof value.error === 'string' ? value.error : 'invalid_response',
        effect,
        retryAfterMs,
        { method, needed: value.needed, provided: value.provided }
      );
    }
    return value;
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    for (const connection of this.#connectionsByToken.values()) {
      if (connection.reconnectTimer !== undefined) {
        clearTimeout(connection.reconnectTimer);
      }
      connection.socket?.close(1000, 'WOML Slack host shutting down');
    }
    this.#connectionsByReference.clear();
    this.#connectionsByToken.clear();
    this.#pendingListeners.clear();
    this.#botIdentities.clear();
    this.#botReferences.clear();
    this.#channelIds.clear();
  }

  async #findChannel(name: string, token: string): Promise<string> {
    let cursor: string | undefined;
    do {
      const response = await this.api(
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

  async #openConnection(connection: SlackConnection): Promise<void> {
    const response = await this.api(
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
      this.#onSocketMessage(connection, socket, event.data);
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
        .catch(() => this.#scheduleReconnect(connection))
        .finally(() => {
          connection.opening = undefined;
        });
    }, delay);
  }

  #onSocketMessage(
    connection: SlackConnection,
    socket: SlackSocket,
    raw: unknown
  ): void {
    if (typeof raw !== 'string') return;
    let body: unknown;
    try {
      body = JSON.parse(raw);
    } catch {
      return;
    }
    if (!record(body)) return;
    if (body.type === 'disconnect') {
      if (connection.socket === socket) {
        connection.socket = undefined;
        socket.close(1000, 'Slack requested Socket Mode refresh');
        this.#scheduleReconnect(connection);
      }
      return;
    }
    const envelopeId =
      typeof body.envelope_id === 'string' ? body.envelope_id : undefined;
    let acknowledged = false;
    const envelope: SlackEnvelope = {
      body,
      ...(envelopeId === undefined ? {} : { envelopeId }),
      acknowledge: () => {
        if (acknowledged || envelopeId === undefined) return;
        acknowledged = true;
        socket.send(JSON.stringify({ envelope_id: envelopeId }));
      },
    };
    for (const registration of connection.listeners.values()) {
      Promise.resolve(registration.listener(envelope)).catch(error => {
        const message = error instanceof Error ? error.message : String(error);
        this.#log(`Slack envelope listener failed safely: ${message}`);
      });
    }
  }

  #unavailable(): SlackTransportError {
    return failure(
      'provider_unavailable',
      'WOML_SLACK_UNAVAILABLE',
      'The shared Slack transport is closed.',
      true
    );
  }
}
