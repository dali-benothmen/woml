import type {
  DiscordBotIdentity,
  DiscordFailure,
  DiscordInteractionUpdate,
  DiscordMessageIdentity,
  DiscordNormalizedUpdate,
  DiscordUpdateListener,
} from './types';

const DISCORD_API = 'https://discord.com/api/v10';
const DISCORD_GATEWAY = 'wss://gateway.discord.gg/?v=10&encoding=json';
const REQUEST_TIMEOUT_MS = 30_000;
const GATEWAY_READY_TIMEOUT_MS = 30_000;
const DISCORD_INTENTS = 1 | 512 | 4_096 | 32_768;

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function snowflake(value: unknown): value is string {
  return typeof value === 'string' && /^[0-9]{17,20}$/.test(value);
}

export class DiscordTransportError extends Error {
  constructor(readonly failure: DiscordFailure) {
    super(failure.message);
    this.name = 'DiscordTransportError';
  }
}

function failure(
  kind: DiscordFailure['kind'],
  code: string,
  message: string,
  retryable: boolean,
  retryAfterMs?: number
): DiscordTransportError {
  return new DiscordTransportError({
    kind,
    code,
    message,
    retryable,
    ...(retryAfterMs === undefined ? {} : { retryAfterMs }),
  });
}

export interface DiscordGatewaySocket {
  readonly readyState: number;
  binaryType: string;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { readonly data: unknown }) => void) | null;
  onclose:
    | ((event: { readonly code: number; readonly reason?: string }) => void)
    | null;
  onerror: ((event: unknown) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

interface DiscordConnection {
  readonly token: string;
  readonly references: Set<string>;
  readonly listeners: Map<string, DiscordUpdateListener>;
  readonly identity: DiscordBotIdentity;
  socket?: DiscordGatewaySocket;
  sequence?: number;
  sessionId?: string;
  resumeGatewayUrl?: string;
  heartbeatTimer?: ReturnType<typeof setInterval>;
  awaitingHeartbeatAck: boolean;
  reconnectTask?: Promise<void>;
  dispatchChain: Promise<void>;
  fatal: boolean;
}

export interface SharedDiscordTransportOptions {
  readonly fetch?: typeof globalThis.fetch;
  readonly createWebSocket?: (url: string) => DiscordGatewaySocket;
  readonly apiBase?: string;
  readonly gatewayUrl?: string;
  readonly reconnectDelayMs?: number;
  readonly readyTimeoutMs?: number;
  readonly log?: (message: string) => void;
  readonly onFatal?: (failure: DiscordFailure) => void;
}

export class SharedDiscordTransport {
  readonly #fetch: typeof globalThis.fetch;
  readonly #createWebSocket: (url: string) => DiscordGatewaySocket;
  readonly #apiBase: string;
  readonly #gatewayUrl: string;
  readonly #reconnectDelayMs: number;
  readonly #readyTimeoutMs: number;
  readonly #log: (message: string) => void;
  readonly #onFatal: (failure: DiscordFailure) => void;
  readonly #connectionsByToken = new Map<string, DiscordConnection>();
  readonly #connectionsByReference = new Map<string, DiscordConnection>();
  readonly #pendingListeners = new Map<
    string,
    Map<string, DiscordUpdateListener>
  >();
  readonly #controllers = new Set<AbortController>();
  #closed = false;

  constructor(options: SharedDiscordTransportOptions = {}) {
    this.#fetch = options.fetch ?? globalThis.fetch;
    this.#createWebSocket =
      options.createWebSocket ??
      (url => new WebSocket(url) as unknown as DiscordGatewaySocket);
    this.#apiBase = options.apiBase ?? DISCORD_API;
    this.#gatewayUrl = options.gatewayUrl ?? DISCORD_GATEWAY;
    this.#reconnectDelayMs = options.reconnectDelayMs ?? 1_000;
    this.#readyTimeoutMs = options.readyTimeoutMs ?? GATEWAY_READY_TIMEOUT_MS;
    this.#log = options.log ?? (() => {});
    this.#onFatal = options.onFatal ?? (() => {});
  }

  subscribe(
    botTokenReference: string,
    listenerId: string,
    listener: DiscordUpdateListener
  ): () => void {
    if (this.#closed) throw this.#unavailable();
    const connection = this.#connectionsByReference.get(botTokenReference);
    const listeners =
      connection?.listeners ??
      this.#pendingListeners.get(botTokenReference) ??
      new Map<string, DiscordUpdateListener>();
    if (listeners.has(listenerId)) {
      throw new Error(`Discord listener "${listenerId}" is already registered.`);
    }
    listeners.set(listenerId, listener);
    if (connection === undefined) {
      this.#pendingListeners.set(botTokenReference, listeners);
    }
    return () => listeners.delete(listenerId);
  }

  async ensureConnected(
    botTokenReference: string,
    botToken: string
  ): Promise<DiscordBotIdentity> {
    if (this.#closed) throw this.#unavailable();
    const known = this.#connectionsByReference.get(botTokenReference);
    if (known !== undefined) {
      if (known.token !== botToken) throw this.#unavailable();
      return known.identity;
    }
    let connection = this.#connectionsByToken.get(botToken);
    if (connection === undefined) {
      const identity = await this.botIdentity(botToken);
      connection = {
        token: botToken,
        references: new Set([botTokenReference]),
        listeners:
          this.#pendingListeners.get(botTokenReference) ??
          new Map<string, DiscordUpdateListener>(),
        identity,
        awaitingHeartbeatAck: false,
        dispatchChain: Promise.resolve(),
        fatal: false,
      };
      this.#connectionsByToken.set(botToken, connection);
      this.#connectionsByReference.set(botTokenReference, connection);
      this.#pendingListeners.delete(botTokenReference);
      try {
        await this.#open(connection);
      } catch (error) {
        this.#connectionsByToken.delete(botToken);
        this.#connectionsByReference.delete(botTokenReference);
        throw error;
      }
      return connection.identity;
    }
    connection.references.add(botTokenReference);
    this.#connectionsByReference.set(botTokenReference, connection);
    const pending = this.#pendingListeners.get(botTokenReference);
    if (pending !== undefined) {
      for (const [id, listener] of pending) connection.listeners.set(id, listener);
      this.#pendingListeners.delete(botTokenReference);
    }
    return connection.identity;
  }

  async botIdentity(botToken: string): Promise<DiscordBotIdentity> {
    const result = await this.#request(botToken, '/users/@me', 'GET', undefined, 'read');
    if (
      !record(result) ||
      !snowflake(result.id) ||
      typeof result.username !== 'string' ||
      result.username.length === 0 ||
      result.bot !== true
    ) {
      throw failure(
        'request_invalid',
        'WOML_DISCORD_RESPONSE_INVALID',
        'Discord returned an invalid bot identity.',
        false
      );
    }
    return { botId: result.id, username: result.username };
  }

  async sendMessage(options: {
    readonly botToken: string;
    readonly accountId: string;
    readonly conversationId: string;
    readonly text: string;
    readonly replyToMessageId?: string;
    readonly decisionCapability?: string;
  }): Promise<DiscordMessageIdentity> {
    const components = options.decisionCapability === undefined
      ? undefined
      : [
          {
            type: 1,
            components: [
              {
                type: 2,
                style: 3,
                label: 'Approve',
                custom_id: `a:${options.decisionCapability}`,
              },
              {
                type: 2,
                style: 4,
                label: 'Reject',
                custom_id: `r:${options.decisionCapability}`,
              },
            ],
          },
        ];
    const result = await this.#request(
      options.botToken,
      `/channels/${options.conversationId}/messages`,
      'POST',
      {
        content: options.text,
        ...(options.replyToMessageId === undefined
          ? {}
          : {
              message_reference: {
                message_id: options.replyToMessageId,
                fail_if_not_exists: false,
              },
            }),
        ...(components === undefined ? {} : { components }),
      },
      'delivery'
    );
    if (!record(result) || !snowflake(result.id) || !snowflake(result.channel_id)) {
      throw failure(
        'delivery_ambiguous',
        'WOML_DISCORD_RESPONSE_INVALID',
        'Discord returned an invalid message identity.',
        false
      );
    }
    return {
      provider: 'discord',
      accountId: options.accountId,
      conversationId: result.channel_id,
      messageId: result.id,
    };
  }

  async updateMessage(options: {
    readonly botToken: string;
    readonly conversationId: string;
    readonly messageId: string;
    readonly resolution: 'approved' | 'rejected' | 'timeout_failed';
  }): Promise<void> {
    await this.#request(
      options.botToken,
      `/channels/${options.conversationId}/messages/${options.messageId}`,
      'PATCH',
      { components: [] },
      'update'
    );
  }

  async acknowledgeInteraction(
    interaction: DiscordInteractionUpdate,
    text: string
  ): Promise<void> {
    await this.#request(
      undefined,
      `/interactions/${interaction.interactionId}/${interaction.interactionToken}/callback`,
      'POST',
      { type: 4, data: { content: text, flags: 64 } },
      'read'
    );
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    for (const controller of this.#controllers) controller.abort();
    for (const connection of this.#connectionsByToken.values()) {
      this.#clearHeartbeat(connection);
      connection.socket?.close(1000, 'WOML runtime stopped');
    }
    await Promise.allSettled(
      [...this.#connectionsByToken.values()].flatMap(connection =>
        connection.reconnectTask === undefined ? [] : [connection.reconnectTask]
      )
    );
    this.#connectionsByReference.clear();
    this.#connectionsByToken.clear();
    this.#pendingListeners.clear();
  }

  async #open(connection: DiscordConnection): Promise<void> {
    if (this.#closed || connection.fatal) throw this.#unavailable();
    const gateway = connection.sessionId === undefined
      ? this.#gatewayUrl
      : `${connection.resumeGatewayUrl ?? this.#gatewayUrl.replace(/\?.*$/, '')}?v=10&encoding=json`;
    const socket = this.#createWebSocket(gateway);
    connection.socket = socket;
    socket.binaryType = 'arraybuffer';
    let settled = false;
    let becameReady = false;
    let resolveReady!: () => void;
    let rejectReady!: (error: unknown) => void;
    const ready = new Promise<void>((resolve, reject) => {
      resolveReady = resolve;
      rejectReady = reject;
    });
    const timeout = setTimeout(() => {
      if (settled) return;
      settled = true;
      rejectReady(
        failure(
          'provider_unavailable',
          'WOML_DISCORD_GATEWAY_TIMEOUT',
          'Discord Gateway did not become ready before the startup deadline.',
          true
        )
      );
      socket.close(4000, 'Gateway startup timeout');
    }, this.#readyTimeoutMs);
    const settleReady = () => {
      if (settled) return;
      settled = true;
      becameReady = true;
      clearTimeout(timeout);
      resolveReady();
    };
    const settleFailure = (error: unknown) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      rejectReady(error);
    };
    socket.onopen = () => {};
    socket.onerror = () => {
      if (!settled) {
        settleFailure(
          failure(
            'provider_unavailable',
            'WOML_DISCORD_GATEWAY_UNAVAILABLE',
            'Discord Gateway could not be reached.',
            true
          )
        );
        socket.close(4000, 'Gateway connection error');
      }
    };
    socket.onmessage = event => {
      connection.dispatchChain = connection.dispatchChain
        .then(() => this.#gatewayMessage(connection, event.data, settleReady))
        .catch(error => this.#listenerFailure(connection, error));
    };
    socket.onclose = event => {
      this.#clearHeartbeat(connection);
      if (connection.socket === socket) connection.socket = undefined;
      const closed = this.#gatewayCloseFailure(event.code);
      if (!settled) settleFailure(closed);
      if (this.#closed || connection.fatal || event.code === 1000) return;
      if (!closed.failure.retryable) {
        connection.fatal = true;
        this.#onFatal(closed.failure);
        return;
      }
      if (!becameReady) return;
      this.#scheduleReconnect(connection, closed.failure.retryAfterMs);
    };
    await ready;
  }

  async #gatewayMessage(
    connection: DiscordConnection,
    raw: unknown,
    ready: () => void
  ): Promise<void> {
    const text = typeof raw === 'string'
      ? raw
      : raw instanceof ArrayBuffer
        ? new TextDecoder().decode(raw)
        : ArrayBuffer.isView(raw)
          ? new TextDecoder().decode(raw)
          : undefined;
    if (text === undefined || Buffer.byteLength(text, 'utf8') > 1_048_576) {
      throw failure(
        'size_limit_exceeded',
        'WOML_DISCORD_GATEWAY_FRAME_TOO_LARGE',
        'Discord sent an invalid or oversized Gateway frame.',
        false
      );
    }
    let envelope: unknown;
    try {
      envelope = JSON.parse(text);
    } catch {
      throw failure(
        'request_invalid',
        'WOML_DISCORD_GATEWAY_FRAME_INVALID',
        'Discord sent invalid Gateway JSON.',
        false
      );
    }
    if (!record(envelope) || typeof envelope.op !== 'number') return;
    if (typeof envelope.s === 'number' && Number.isSafeInteger(envelope.s)) {
      connection.sequence = envelope.s;
    }
    if (envelope.op === 10) {
      if (!record(envelope.d) || typeof envelope.d.heartbeat_interval !== 'number') {
        throw failure(
          'request_invalid',
          'WOML_DISCORD_GATEWAY_HELLO_INVALID',
          'Discord sent an invalid Gateway hello.',
          false
        );
      }
      this.#startHeartbeat(connection, envelope.d.heartbeat_interval);
      if (connection.sessionId !== undefined && connection.sequence !== undefined) {
        this.#sendGateway(connection, {
          op: 6,
          d: {
            token: connection.token,
            session_id: connection.sessionId,
            seq: connection.sequence,
          },
        });
      } else {
        this.#sendGateway(connection, {
          op: 2,
          d: {
            token: connection.token,
            intents: DISCORD_INTENTS,
            properties: {
              os: process.platform,
              browser: 'woml',
              device: 'woml',
            },
          },
        });
      }
      return;
    }
    if (envelope.op === 11) {
      connection.awaitingHeartbeatAck = false;
      return;
    }
    if (envelope.op === 1) {
      this.#heartbeat(connection);
      return;
    }
    if (envelope.op === 7) {
      connection.socket?.close(4000, 'Discord requested reconnect');
      return;
    }
    if (envelope.op === 9) {
      if (envelope.d !== true) {
        connection.sessionId = undefined;
        connection.sequence = undefined;
        connection.resumeGatewayUrl = undefined;
      }
      connection.socket?.close(4000, 'Discord invalidated the session');
      return;
    }
    if (envelope.op !== 0 || typeof envelope.t !== 'string') return;
    if (envelope.t === 'READY') {
      if (
        !record(envelope.d) ||
        typeof envelope.d.session_id !== 'string' ||
        typeof envelope.d.resume_gateway_url !== 'string' ||
        !record(envelope.d.user) ||
        envelope.d.user.id !== connection.identity.botId
      ) {
        throw failure(
          'request_invalid',
          'WOML_DISCORD_GATEWAY_READY_INVALID',
          'Discord returned an invalid Gateway ready identity.',
          false
        );
      }
      connection.sessionId = envelope.d.session_id;
      connection.resumeGatewayUrl = envelope.d.resume_gateway_url;
      ready();
      return;
    }
    if (envelope.t === 'RESUMED') {
      ready();
      return;
    }
    const update = this.#normalizeDispatch(
      envelope.t,
      envelope.d,
      connection.identity
    );
    if (update === undefined) return;
    for (const listener of connection.listeners.values()) await listener(update);
  }

  #normalizeDispatch(
    type: string,
    data: unknown,
    identity: DiscordBotIdentity
  ): DiscordNormalizedUpdate | undefined {
    if (!record(data)) return undefined;
    if (type === 'INTERACTION_CREATE') {
      if (
        data.type !== 3 ||
        !snowflake(data.id) ||
        typeof data.token !== 'string' ||
        !record(data.data) ||
        typeof data.data.custom_id !== 'string'
      ) return undefined;
      const match = /^(a|r):(ncap_[a-f0-9]+\.[a-f0-9]+)$/.exec(
        data.data.custom_id
      );
      const actor = record(data.member) && record(data.member.user)
        ? data.member.user.id
        : record(data.user)
          ? data.user.id
          : undefined;
      if (match === null || !snowflake(actor)) return undefined;
      return {
        kind: 'interaction',
        eventId: data.id,
        interactionId: data.id,
        interactionToken: data.token,
        actorId: actor,
        decisionCapability: match[2]!,
        decision: match[1] === 'a' ? 'approved' : 'rejected',
      };
    }
    if (type !== 'MESSAGE_CREATE') return undefined;
    if (
      !snowflake(data.id) ||
      !snowflake(data.channel_id) ||
      typeof data.content !== 'string' ||
      Buffer.byteLength(data.content, 'utf8') > 40_000 ||
      typeof data.timestamp !== 'string' ||
      !record(data.author) ||
      !snowflake(data.author.id) ||
      data.author.bot === true ||
      data.author.id === identity.botId
    ) return undefined;
    const direct = data.guild_id === undefined || data.guild_id === null;
    const mentioned =
      Array.isArray(data.mentions) &&
      data.mentions.some(item => record(item) && item.id === identity.botId);
    if (!direct && !mentioned) return undefined;
    const occurred = new Date(data.timestamp);
    if (Number.isNaN(occurred.getTime())) return undefined;
    const reply = record(data.message_reference) &&
      snowflake(data.message_reference.message_id)
      ? data.message_reference.message_id
      : undefined;
    const senderName =
      typeof data.author.global_name === 'string' &&
      data.author.global_name.length > 0
        ? data.author.global_name
        : typeof data.author.username === 'string' &&
            data.author.username.length > 0
          ? data.author.username
          : undefined;
    return {
      kind: 'message',
      eventId: data.id,
      payload: {
        provider: 'discord',
        event: direct ? 'direct-message' : 'app-mention',
        text: data.content,
        senderId: data.author.id,
        ...(senderName === undefined ? {} : { senderName }),
        conversationId: data.channel_id,
        conversationType: direct ? 'direct' : 'group',
        messageId: data.id,
        ...(reply === undefined ? {} : { replyToMessageId: reply }),
        occurredAt: occurred.toISOString(),
        providerData: {
          botId: identity.botId,
          ...(snowflake(data.guild_id) ? { guildId: data.guild_id } : {}),
        },
      },
    };
  }

  #startHeartbeat(connection: DiscordConnection, intervalMs: number): void {
    if (!Number.isFinite(intervalMs) || intervalMs < 1_000) {
      throw failure(
        'request_invalid',
        'WOML_DISCORD_GATEWAY_HELLO_INVALID',
        'Discord returned an invalid heartbeat interval.',
        false
      );
    }
    this.#clearHeartbeat(connection);
    connection.awaitingHeartbeatAck = false;
    this.#heartbeat(connection);
    connection.heartbeatTimer = setInterval(
      () => this.#heartbeat(connection),
      Math.floor(intervalMs)
    );
  }

  #heartbeat(connection: DiscordConnection): void {
    if (connection.awaitingHeartbeatAck) {
      connection.socket?.close(4000, 'Heartbeat acknowledgement missed');
      return;
    }
    connection.awaitingHeartbeatAck = true;
    this.#sendGateway(connection, { op: 1, d: connection.sequence ?? null });
  }

  #clearHeartbeat(connection: DiscordConnection): void {
    if (connection.heartbeatTimer !== undefined) {
      clearInterval(connection.heartbeatTimer);
      connection.heartbeatTimer = undefined;
    }
    connection.awaitingHeartbeatAck = false;
  }

  #sendGateway(connection: DiscordConnection, value: unknown): void {
    if (connection.socket?.readyState !== 1) return;
    connection.socket.send(JSON.stringify(value));
  }

  #scheduleReconnect(connection: DiscordConnection, delayMs?: number): void {
    if (connection.reconnectTask !== undefined || this.#closed) return;
    const task = (async () => {
      await Bun.sleep(delayMs ?? this.#reconnectDelayMs);
      if (this.#closed || connection.fatal) return;
      try {
        await this.#open(connection);
        this.#log(`Discord bot ${connection.identity.username} resumed Gateway events.`);
      } catch (error) {
        if (this.#closed || connection.fatal) return;
        const safe = error instanceof DiscordTransportError
          ? error.failure
          : this.#unavailable().failure;
        if (!safe.retryable) {
          connection.fatal = true;
          this.#onFatal(safe);
          return;
        }
        this.#log(`${safe.message} Retrying Discord Gateway.`);
        setTimeout(
          () => this.#scheduleReconnect(connection, safe.retryAfterMs),
          0
        );
      }
    });
    connection.reconnectTask = task();
    void connection.reconnectTask.finally(() => {
      connection.reconnectTask = undefined;
    });
  }

  #listenerFailure(connection: DiscordConnection, error: unknown): void {
    const safe = error instanceof DiscordTransportError
      ? error.failure
      : {
          kind: 'provider_unavailable' as const,
          code: 'WOML_DISCORD_TRIGGER_UNAVAILABLE',
          message: 'The durable WOML trigger authority rejected the Discord event.',
          retryable: false,
        };
    if (!safe.retryable) {
      connection.fatal = true;
      this.#onFatal(safe);
      connection.socket?.close(1000, 'Fatal trigger admission failure');
      return;
    }
    connection.socket?.close(4000, 'Retryable trigger admission failure');
  }

  #gatewayCloseFailure(code: number): DiscordTransportError {
    if (code === 4004) {
      return failure(
        'provider_auth_failed',
        'WOML_DISCORD_AUTH_FAILED',
        'Discord rejected the configured bot token.',
        false
      );
    }
    if (code === 4013 || code === 4014) {
      return failure(
        'permission_denied',
        'WOML_DISCORD_INTENTS_MISSING',
        'Discord rejected the required Gateway intents. Enable Message Content Intent for the bot and verify its server permissions.',
        false
      );
    }
    if (code === 4008) {
      return failure(
        'rate_limited',
        'WOML_DISCORD_GATEWAY_RATE_LIMITED',
        'Discord rate-limited the Gateway connection.',
        true,
        5_000
      );
    }
    return failure(
      'provider_unavailable',
      'WOML_DISCORD_GATEWAY_UNAVAILABLE',
      'The Discord Gateway connection closed and will be resumed safely.',
      true
    );
  }

  async #request(
    botToken: string | undefined,
    path: string,
    method: 'GET' | 'POST' | 'PATCH',
    body: Readonly<Record<string, unknown>> | undefined,
    effect: 'read' | 'delivery' | 'update'
  ): Promise<unknown> {
    const controller = new AbortController();
    this.#controllers.add(controller);
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
    let response: Response;
    try {
      response = await this.#fetch(`${this.#apiBase}${path}`, {
        method,
        headers: {
          ...(botToken === undefined
            ? {}
            : { authorization: `Bot ${botToken}` }),
          ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal: controller.signal,
      });
    } catch {
      throw failure(
        effect === 'delivery' ? 'delivery_ambiguous' : 'provider_unavailable',
        effect === 'delivery'
          ? 'WOML_DISCORD_DELIVERY_AMBIGUOUS'
          : 'WOML_DISCORD_UNAVAILABLE',
        effect === 'delivery'
          ? 'The Discord delivery connection ended without a confirmed result.'
          : 'Discord is temporarily unavailable.',
        effect !== 'delivery'
      );
    } finally {
      clearTimeout(timeout);
      this.#controllers.delete(controller);
    }
    let decoded: unknown;
    if (response.status === 204) decoded = undefined;
    else {
      try {
        decoded = await response.json();
      } catch {
        decoded = undefined;
      }
    }
    if (!response.ok) throw this.#apiFailure(response.status, decoded, effect);
    return decoded;
  }

  #apiFailure(
    status: number,
    body: unknown,
    effect: 'read' | 'delivery' | 'update'
  ): DiscordTransportError {
    if (status === 429) {
      const retryAfter = record(body) ? body.retry_after : undefined;
      return failure(
        'rate_limited',
        'WOML_DISCORD_RATE_LIMITED',
        'Discord rate-limited the bot request.',
        true,
        typeof retryAfter === 'number' && Number.isFinite(retryAfter)
          ? Math.max(0, Math.ceil(retryAfter * 1_000))
          : undefined
      );
    }
    if (status === 401) {
      return failure(
        'provider_auth_failed',
        'WOML_DISCORD_AUTH_FAILED',
        'Discord rejected the configured bot token.',
        false
      );
    }
    if (status === 403) {
      return failure(
        'permission_denied',
        'WOML_DISCORD_PERMISSION_DENIED',
        'Discord denied the bot permission to access the configured channel.',
        false
      );
    }
    if (status === 404) {
      return failure(
        'destination_invalid',
        'WOML_DISCORD_DESTINATION_INVALID',
        'Discord could not find the configured channel or message.',
        false
      );
    }
    if (effect === 'delivery') {
      return failure(
        'delivery_ambiguous',
        'WOML_DISCORD_DELIVERY_AMBIGUOUS',
        'Discord did not confirm whether the message was created.',
        false
      );
    }
    return failure(
      effect === 'update' ? 'update_failed' : 'provider_unavailable',
      effect === 'update'
        ? 'WOML_DISCORD_UPDATE_FAILED'
        : 'WOML_DISCORD_UNAVAILABLE',
      effect === 'update'
        ? 'Discord could not update the message.'
        : 'Discord could not complete the request.',
      true
    );
  }

  #unavailable(): DiscordTransportError {
    return failure(
      'provider_unavailable',
      'WOML_DISCORD_UNAVAILABLE',
      'The Discord transport is closed or has conflicting credentials.',
      true
    );
  }
}
