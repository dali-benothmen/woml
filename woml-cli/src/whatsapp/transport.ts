import type {
  WhatsAppFailure,
  WhatsAppMessageIdentity,
} from './types';

const GRAPH_API = 'https://graph.facebook.com/v23.0';
const REQUEST_TIMEOUT_MS = 30_000;

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export class WhatsAppTransportError extends Error {
  constructor(readonly failure: WhatsAppFailure) {
    super(failure.message);
    this.name = 'WhatsAppTransportError';
  }
}

function failure(
  kind: WhatsAppFailure['kind'],
  code: string,
  message: string,
  retryable: boolean,
  retryAfterMs?: number
): WhatsAppTransportError {
  return new WhatsAppTransportError({
    kind,
    code,
    message,
    retryable,
    ...(retryAfterMs === undefined ? {} : { retryAfterMs }),
  });
}

function apiFailure(
  status: number,
  body: Readonly<Record<string, unknown>>
): WhatsAppTransportError {
  const error = record(body.error) ? body.error : {};
  const metaCode = typeof error.code === 'number' ? error.code : undefined;
  if (status === 429 || metaCode === 4) {
    return failure(
      'rate_limited',
      'WOML_WHATSAPP_RATE_LIMITED',
      'WhatsApp rate-limited the message request.',
      true
    );
  }
  if (status === 401 || status === 403 || metaCode === 190) {
    return failure(
      'provider_auth_failed',
      'WOML_WHATSAPP_AUTH_FAILED',
      'Meta rejected the configured WhatsApp access token or permission.',
      false
    );
  }
  if (status === 400) {
    return failure(
      'destination_invalid',
      'WOML_WHATSAPP_REQUEST_REJECTED',
      'WhatsApp rejected the recipient or approved-template request.',
      false
    );
  }
  return failure(
    'delivery_ambiguous',
    'WOML_WHATSAPP_DELIVERY_AMBIGUOUS',
    'Meta did not confirm whether the WhatsApp message was accepted; WOML will not replay the uncertain delivery automatically.',
    false
  );
}

export interface WhatsAppTransportOptions {
  readonly fetch?: typeof globalThis.fetch;
  readonly apiBase?: string;
}

export class SharedWhatsAppTransport {
  readonly #fetch: typeof globalThis.fetch;
  readonly #apiBase: string;
  readonly #controllers = new Set<AbortController>();
  #closed = false;

  constructor(options: WhatsAppTransportOptions = {}) {
    this.#fetch = options.fetch ?? globalThis.fetch;
    this.#apiBase = options.apiBase ?? GRAPH_API;
  }

  async sendTemplate(options: {
    readonly accessToken: string;
    readonly phoneNumberId: string;
    readonly conversationId: string;
    readonly templateName: string;
    readonly language: string;
    readonly parameters: readonly string[];
    readonly decisionCapability?: string;
  }): Promise<WhatsAppMessageIdentity> {
    if (this.#closed) {
      throw failure(
        'provider_unavailable',
        'WOML_WHATSAPP_UNAVAILABLE',
        'The WhatsApp transport is closed.',
        true
      );
    }
    const components: Record<string, unknown>[] = options.parameters.length === 0
      ? []
      : [{
          type: 'body',
          parameters: options.parameters.map(text => ({ type: 'text', text })),
        }];
    if (options.decisionCapability !== undefined) {
      components.push(
        {
          type: 'button',
          sub_type: 'quick_reply',
          index: '0',
          parameters: [{
            type: 'payload',
            payload: `woml_approve:${options.decisionCapability}`,
          }],
        },
        {
          type: 'button',
          sub_type: 'quick_reply',
          index: '1',
          parameters: [{
            type: 'payload',
            payload: `woml_reject:${options.decisionCapability}`,
          }],
        }
      );
    }
    const controller = new AbortController();
    this.#controllers.add(controller);
    const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
    let response: Response;
    try {
      response = await this.#fetch(
        `${this.#apiBase}/${options.phoneNumberId}/messages`,
        {
          method: 'POST',
          headers: {
            authorization: `Bearer ${options.accessToken}`,
            'content-type': 'application/json',
          },
          body: JSON.stringify({
            messaging_product: 'whatsapp',
            recipient_type: 'individual',
            to: options.conversationId,
            type: 'template',
            template: {
              name: options.templateName,
              language: { code: options.language },
              ...(components.length === 0 ? {} : { components }),
            },
          }),
          signal: controller.signal,
        }
      );
    } catch {
      throw failure(
        'delivery_ambiguous',
        'WOML_WHATSAPP_DELIVERY_AMBIGUOUS',
        'The WhatsApp request ended before Meta confirmed delivery.',
        false
      );
    } finally {
      clearTimeout(timeout);
      this.#controllers.delete(controller);
    }
    const raw = await response.text();
    let body: unknown;
    try {
      body = raw.length === 0 ? {} : JSON.parse(raw);
    } catch {
      throw failure(
        'delivery_ambiguous',
        'WOML_WHATSAPP_RESPONSE_INVALID',
        'Meta returned an invalid WhatsApp response.',
        false
      );
    }
    if (!response.ok) throw apiFailure(response.status, record(body) ? body : {});
    const messages = record(body) && Array.isArray(body.messages)
      ? body.messages
      : [];
    const first = record(messages[0]) ? messages[0] : undefined;
    if (typeof first?.id !== 'string' || first.id.length === 0) {
      throw failure(
        'delivery_ambiguous',
        'WOML_WHATSAPP_RESPONSE_INVALID',
        'Meta accepted the request but returned no valid message identity.',
        false
      );
    }
    return {
      provider: 'whatsapp',
      accountId: options.phoneNumberId,
      conversationId: options.conversationId,
      messageId: first.id,
    };
  }

  async close(): Promise<void> {
    this.#closed = true;
    for (const controller of this.#controllers) controller.abort();
    this.#controllers.clear();
  }
}

