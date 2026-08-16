import { CommunicationProviderAdapterError } from '../communication-provider';
import {
  COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS,
  COMMUNICATION_PROVIDER_MAX_CONNECTIONS,
  COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES,
  COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS,
  ProviderResponseLimitError,
  providerCredentialWithinBudget,
  readProviderResponseBody,
  serializeProviderRequest,
} from '../communication-provider/limits';
import type {
  TelegramBotIdentity,
  TelegramFailure,
  TelegramMessageIdentity,
  TelegramNormalizedUpdate,
  TelegramUpdateListener,
} from './types';

const TELEGRAM_API = 'https://api.telegram.org';
const POLL_TIMEOUT_SECONDS = 25;
const REQUEST_TIMEOUT_MS = 30_000;

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export class TelegramTransportError extends Error {
  constructor(readonly failure: TelegramFailure) {
    super(failure.message);
    this.name = 'TelegramTransportError';
  }
}

function failure(
  kind: TelegramFailure['kind'],
  code: string,
  message: string,
  retryable: boolean,
  retryAfterMs?: number
): TelegramTransportError {
  return new TelegramTransportError({
    kind,
    code,
    message,
    retryable,
    ...(retryAfterMs === undefined ? {} : { retryAfterMs }),
  });
}

function apiFailure(
  status: number,
  body: Readonly<Record<string, unknown>>,
  effect: 'read' | 'delivery' | 'update'
): TelegramTransportError {
  const code = typeof body.error_code === 'number' ? body.error_code : status;
  const retryAfter = record(body.parameters)
    ? body.parameters.retry_after
    : undefined;
  if (code === 429) {
    return failure(
      'rate_limited',
      'WOML_TELEGRAM_RATE_LIMITED',
      'Telegram rate-limited the bot request.',
      true,
      typeof retryAfter === 'number' && Number.isFinite(retryAfter)
        ? Math.max(0, Math.floor(retryAfter * 1_000))
        : undefined
    );
  }
  if (code === 401) {
    return failure(
      'provider_auth_failed',
      'WOML_TELEGRAM_AUTH_FAILED',
      'Telegram rejected the configured bot token.',
      false
    );
  }
  if (code === 400 || code === 403) {
    return failure(
      'destination_invalid',
      'WOML_TELEGRAM_DESTINATION_INVALID',
      'Telegram could not access the configured chat. Confirm the numeric chat ID and add the bot to that chat.',
      false
    );
  }
  if (code === 409 && effect === 'read') {
    return failure(
      'provider_unavailable',
      'WOML_TELEGRAM_POLLING_CONFLICT',
      'Telegram long polling is unavailable because another poller or webhook is active for this bot.',
      false
    );
  }
  if (effect === 'delivery') {
    return failure(
      'delivery_ambiguous',
      'WOML_TELEGRAM_DELIVERY_AMBIGUOUS',
      'Telegram did not confirm whether the message was created; WOML will not replay the uncertain delivery automatically.',
      false
    );
  }
  return failure(
    effect === 'update' ? 'update_failed' : 'provider_unavailable',
    effect === 'update'
      ? 'WOML_TELEGRAM_UPDATE_FAILED'
      : 'WOML_TELEGRAM_UNAVAILABLE',
    effect === 'update'
      ? 'Telegram could not update the message.'
      : 'Telegram could not complete the request.',
    true
  );
}

interface TelegramConnection {
  readonly token: string;
  readonly references: Set<string>;
  readonly listeners: Map<string, TelegramUpdateListener>;
  readonly identity: TelegramBotIdentity;
  offset: number;
  task?: Promise<void>;
}

export interface SharedTelegramTransportOptions {
  readonly fetch?: typeof globalThis.fetch;
  readonly pollTimeoutSeconds?: number;
  readonly retryDelayMs?: number;
  readonly log?: (message: string) => void;
  readonly onFatal?: (failure: TelegramFailure) => void;
}

export class SharedTelegramTransport {
  readonly #fetch: typeof globalThis.fetch;
  readonly #pollTimeoutSeconds: number;
  readonly #retryDelayMs: number;
  readonly #log: (message: string) => void;
  readonly #onFatal: (failure: TelegramFailure) => void;
  readonly #connectionsByToken = new Map<string, TelegramConnection>();
  readonly #connectionsByReference = new Map<string, TelegramConnection>();
  readonly #pendingListeners = new Map<string, Map<string, TelegramUpdateListener>>();
  readonly #controllers = new Set<AbortController>();
  #closed = false;

  constructor(options: SharedTelegramTransportOptions = {}) {
    this.#fetch = options.fetch ?? globalThis.fetch;
    this.#pollTimeoutSeconds =
      options.pollTimeoutSeconds ?? POLL_TIMEOUT_SECONDS;
    this.#retryDelayMs = options.retryDelayMs ?? 1_000;
    this.#log = options.log ?? (() => {});
    this.#onFatal = options.onFatal ?? (() => {});
  }

  subscribe(
    botTokenReference: string,
    listenerId: string,
    listener: TelegramUpdateListener
  ): () => void {
    if (this.#closed) throw this.#unavailable();
    if (this.#listenerCount() >= COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS) {
      throw failure(
        'size_limit_exceeded',
        'WOML_TELEGRAM_SUBSCRIBER_LIMIT',
        `Telegram supports at most ${COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS} active trigger subscriptions per runtime.`,
        false
      );
    }
    const connection = this.#connectionsByReference.get(botTokenReference);
    const listeners =
      connection?.listeners ??
      this.#pendingListeners.get(botTokenReference) ??
      new Map<string, TelegramUpdateListener>();
    if (listeners.has(listenerId)) {
      throw new Error(`Telegram listener "${listenerId}" is already registered.`);
    }
    listeners.set(listenerId, listener);
    if (connection === undefined) {
      this.#pendingListeners.set(botTokenReference, listeners);
    }
    return () => listeners.delete(listenerId);
  }

  async ensurePolling(
    botTokenReference: string,
    botToken: string
  ): Promise<TelegramBotIdentity> {
    if (this.#closed) throw this.#unavailable();
    const known = this.#connectionsByReference.get(botTokenReference);
    if (known !== undefined) {
      if (known.token !== botToken) throw this.#unavailable();
      return known.identity;
    }
    let connection = this.#connectionsByToken.get(botToken);
    if (connection === undefined) {
      if (
        this.#connectionsByToken.size >=
        COMMUNICATION_PROVIDER_MAX_CONNECTIONS
      ) {
        throw failure(
          'size_limit_exceeded',
          'WOML_TELEGRAM_CONNECTION_LIMIT',
          `Telegram supports at most ${COMMUNICATION_PROVIDER_MAX_CONNECTIONS} bot connections per runtime.`,
          false
        );
      }
      const identity = await this.botIdentity(botToken);
      connection = {
        token: botToken,
        references: new Set(),
        listeners: new Map(),
        identity,
        offset: 0,
      };
      this.#connectionsByToken.set(botToken, connection);
      connection.task = this.#poll(connection);
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

  async botIdentity(botToken: string): Promise<TelegramBotIdentity> {
    const result = await this.#call(botToken, 'getMe', {}, 'read');
    if (
      !record(result) ||
      (!Number.isSafeInteger(result.id) && typeof result.id !== 'string') ||
      result.is_bot !== true ||
      (result.username !== undefined && typeof result.username !== 'string')
    ) {
      throw failure(
        'request_invalid',
        'WOML_TELEGRAM_RESPONSE_INVALID',
        'Telegram returned an invalid bot identity.',
        false
      );
    }
    return {
      botId: String(result.id),
      ...(typeof result.username === 'string'
        ? { username: result.username }
        : {}),
    };
  }

  async sendMessage(options: {
    readonly botToken: string;
    readonly accountId: string;
    readonly conversationId: string;
    readonly text: string;
    readonly replyToMessageId?: string;
    readonly decisionCapability?: string;
  }): Promise<TelegramMessageIdentity> {
    const replyMarkup = options.decisionCapability === undefined
      ? undefined
      : {
          inline_keyboard: [[
            {
              text: 'Approve',
              callback_data: `a:${options.decisionCapability}`,
            },
            {
              text: 'Reject',
              callback_data: `r:${options.decisionCapability}`,
            },
          ]],
        };
    const result = await this.#call(
      options.botToken,
      'sendMessage',
      {
        chat_id: options.conversationId,
        text: options.text,
        ...(options.replyToMessageId === undefined
          ? {}
          : { reply_parameters: { message_id: Number(options.replyToMessageId) } }),
        ...(replyMarkup === undefined ? {} : { reply_markup: replyMarkup }),
      },
      'delivery'
    );
    if (
      !record(result) ||
      !Number.isSafeInteger(result.message_id) ||
      !record(result.chat) ||
      (!Number.isSafeInteger(result.chat.id) && typeof result.chat.id !== 'string')
    ) {
      throw failure(
        'request_invalid',
        'WOML_TELEGRAM_RESPONSE_INVALID',
        'Telegram returned an invalid message identity.',
        false
      );
    }
    return {
      provider: 'telegram',
      accountId: options.accountId,
      conversationId: String(result.chat.id),
      messageId: String(result.message_id),
    };
  }

  async updateMessage(options: {
    readonly botToken: string;
    readonly conversationId: string;
    readonly messageId: string;
    readonly resolution: 'approved' | 'rejected' | 'timeout_failed';
  }): Promise<void> {
    await this.#call(
      options.botToken,
      'editMessageReplyMarkup',
      {
        chat_id: options.conversationId,
        message_id: Number(options.messageId),
        reply_markup: { inline_keyboard: [] },
      },
      'update'
    );
  }

  async answerCallback(
    botTokenReference: string,
    callbackQueryId: string,
    text: string
  ): Promise<void> {
    const connection = this.#connectionsByReference.get(botTokenReference);
    if (connection === undefined) return;
    await this.#call(
      connection.token,
      'answerCallbackQuery',
      { callback_query_id: callbackQueryId, text },
      'read'
    );
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    for (const controller of this.#controllers) controller.abort();
    await Promise.allSettled(
      [...this.#connectionsByToken.values()].flatMap(connection =>
        connection.task === undefined ? [] : [connection.task]
      )
    );
    this.#connectionsByReference.clear();
    this.#connectionsByToken.clear();
    this.#pendingListeners.clear();
  }

  async #poll(connection: TelegramConnection): Promise<void> {
    while (!this.#closed) {
      try {
        const result = await this.#call(
          connection.token,
          'getUpdates',
          {
            offset: connection.offset,
            timeout: this.#pollTimeoutSeconds,
            allowed_updates: ['message', 'callback_query'],
          },
          'read',
          (this.#pollTimeoutSeconds + 5) * 1_000
        );
        if (
          !Array.isArray(result) ||
          result.length > COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS
        ) {
          throw failure(
            'request_invalid',
            'WOML_TELEGRAM_RESPONSE_INVALID',
            'Telegram returned an invalid update batch.',
            false
          );
        }
        for (const raw of result) {
          const normalized = this.#normalize(raw, connection.identity);
          if (normalized === undefined) {
            if (record(raw) && Number.isSafeInteger(raw.update_id)) {
              connection.offset = Number(raw.update_id) + 1;
            }
            continue;
          }
          for (const listener of connection.listeners.values()) {
            await listener(normalized);
          }
          // The offset moves only after every listener has durably accepted or
          // recognized the occurrence/decision.
          connection.offset = normalized.updateId + 1;
        }
      } catch (error) {
        if (this.#closed) return;
        if (
          error instanceof TelegramTransportError &&
          !error.failure.retryable
        ) {
          this.#onFatal(error.failure);
          return;
        }
        const safe =
          error instanceof TelegramTransportError
            ? error.failure.message
            : 'Telegram polling failed safely.';
        this.#log(`${safe} Retrying Telegram polling.`);
        await Bun.sleep(this.#retryDelayMs);
      }
    }
  }

  #normalize(
    raw: unknown,
    identity: TelegramBotIdentity
  ): TelegramNormalizedUpdate | undefined {
    if (!record(raw) || !Number.isSafeInteger(raw.update_id)) return undefined;
    const updateId = Number(raw.update_id);
    if (record(raw.callback_query)) {
      const query = raw.callback_query;
      if (
        typeof query.id !== 'string' ||
        !record(query.from) ||
        (!Number.isSafeInteger(query.from.id) && typeof query.from.id !== 'string') ||
        typeof query.data !== 'string'
      ) return undefined;
      const match = /^(a|r):(ncap_[a-f0-9]+\.[a-f0-9]+)$/.exec(query.data);
      if (match === null) return undefined;
      return {
        kind: 'callback',
        updateId,
        callbackQueryId: query.id,
        actorId: String(query.from.id),
        decisionCapability: match[2]!,
        decision: match[1] === 'a' ? 'approved' : 'rejected',
      };
    }
    if (!record(raw.message)) return undefined;
    const message = raw.message;
    if (
      !Number.isSafeInteger(message.message_id) ||
      !Number.isSafeInteger(message.date) ||
      typeof message.text !== 'string' ||
      Buffer.byteLength(message.text, 'utf8') >
        COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES ||
      !record(message.from) ||
      (!Number.isSafeInteger(message.from.id) && typeof message.from.id !== 'string') ||
      message.from.is_bot === true ||
      !record(message.chat) ||
      (!Number.isSafeInteger(message.chat.id) && typeof message.chat.id !== 'string')
    ) return undefined;
    const chatType = message.chat.type;
    const conversationType = chatType === 'private'
      ? 'direct'
      : chatType === 'channel'
        ? 'channel'
        : 'group';
    const senderName = [message.from.first_name, message.from.last_name]
      .filter(value => typeof value === 'string' && value.length > 0)
      .join(' ');
    const reply = record(message.reply_to_message)
      && Number.isSafeInteger(message.reply_to_message.message_id)
      ? String(message.reply_to_message.message_id)
      : undefined;
    return {
      kind: 'message',
      updateId,
      payload: {
        provider: 'telegram',
        event: 'message',
        text: message.text,
        senderId: String(message.from.id),
        ...(senderName.length === 0 ? {} : { senderName }),
        conversationId: String(message.chat.id),
        conversationType,
        messageId: String(message.message_id),
        ...(reply === undefined ? {} : { replyToMessageId: reply }),
        occurredAt: new Date(Number(message.date) * 1_000).toISOString(),
        providerData: { botId: identity.botId },
      },
    };
  }

  async #call(
    botToken: string,
    method: string,
    body: Readonly<Record<string, unknown>>,
    effect: 'read' | 'delivery' | 'update',
    timeoutMs = REQUEST_TIMEOUT_MS
  ): Promise<unknown> {
    if (!providerCredentialWithinBudget(botToken)) {
      throw failure(
        'provider_auth_failed',
        'WOML_TELEGRAM_CREDENTIAL_INVALID',
        'The configured Telegram bot token has an invalid shape or exceeds the credential limit.',
        false
      );
    }
    let requestBody: string;
    try {
      requestBody = serializeProviderRequest(body);
    } catch {
      throw failure(
        'size_limit_exceeded',
        'WOML_TELEGRAM_REQUEST_TOO_LARGE',
        'The Telegram request exceeds the 64 KiB provider limit.',
        false
      );
    }
    const controller = new AbortController();
    this.#controllers.add(controller);
    const timeout = setTimeout(() => controller.abort(), timeoutMs);
    let response: Response;
    try {
      response = await this.#fetch(`${TELEGRAM_API}/bot${botToken}/${method}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: requestBody,
        signal: controller.signal,
      });
    } catch (error) {
      if (error instanceof CommunicationProviderAdapterError) throw error;
      throw failure(
        effect === 'delivery' ? 'delivery_ambiguous' : 'provider_unavailable',
        effect === 'delivery'
          ? 'WOML_TELEGRAM_DELIVERY_AMBIGUOUS'
          : 'WOML_TELEGRAM_UNAVAILABLE',
        effect === 'delivery'
          ? 'The Telegram delivery connection ended without a confirmed result.'
          : 'Telegram is temporarily unavailable.',
        effect !== 'delivery'
      );
    } finally {
      clearTimeout(timeout);
      this.#controllers.delete(controller);
    }
    let text: string;
    try {
      text = await readProviderResponseBody(response);
    } catch (error) {
      if (error instanceof ProviderResponseLimitError) {
        throw failure(
          'size_limit_exceeded',
          'WOML_TELEGRAM_RESPONSE_TOO_LARGE',
          'Telegram returned a response larger than the 1 MiB provider limit.',
          false
        );
      }
      throw apiFailure(response.status, {}, effect);
    }
    let decoded: unknown;
    try {
      decoded = JSON.parse(text);
    } catch {
      throw apiFailure(response.status, {}, effect);
    }
    if (!record(decoded) || decoded.ok !== true) {
      throw apiFailure(
        response.status,
        record(decoded) ? decoded : {},
        effect
      );
    }
    return decoded.result;
  }

  #unavailable(): TelegramTransportError {
    return failure(
      'provider_unavailable',
      'WOML_TELEGRAM_UNAVAILABLE',
      'The Telegram transport is closed or has conflicting credentials.',
      true
    );
  }

  #listenerCount(): number {
    let total = 0;
    for (const connection of this.#connectionsByToken.values()) {
      total += connection.listeners.size;
    }
    for (const listeners of this.#pendingListeners.values()) {
      total += listeners.size;
    }
    return total;
  }
}
