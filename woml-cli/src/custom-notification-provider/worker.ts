import { parentPort } from 'node:worker_threads';

import type {
  CustomNotificationRequest,
  CustomProviderReceipt,
} from './types';

interface WorkerRequest {
  readonly source: string;
  readonly props: Readonly<Record<string, unknown>>;
  readonly notification: CustomNotificationRequest;
  readonly attempt: { readonly number: number; readonly max: number };
}

type WorkerResponse =
  | { readonly ok: true; readonly receipt: CustomProviderReceipt }
  | {
      readonly ok: false;
      readonly kind: 'script_threw' | 'non_json' | 'service_failed';
      readonly message: string;
      readonly retryable: boolean;
    };

type AsyncFunction = (...args: unknown[]) => Promise<unknown>;
type AsyncFunctionConstructor = new (
  ...parametersAndBody: string[]
) => AsyncFunction;
const AsyncFunction = Object.getPrototypeOf(
  async function empty() {}
).constructor as AsyncFunctionConstructor;

function deepFreeze<T>(value: T, seen = new WeakSet<object>()): T {
  if (typeof value !== 'object' || value === null) return value;
  if (seen.has(value)) return value;
  seen.add(value);
  for (const child of Object.values(value)) deepFreeze(child, seen);
  return Object.freeze(value);
}

function sensitiveStrings(value: unknown, found: string[] = []): string[] {
  if (typeof value === 'string' && value.length > 0) found.push(value);
  else if (Array.isArray(value)) {
    for (const child of value) sensitiveStrings(child, found);
  } else if (typeof value === 'object' && value !== null) {
    for (const child of Object.values(value)) sensitiveStrings(child, found);
  }
  return found;
}

function redact(
  message: string,
  props: Readonly<Record<string, unknown>>,
  notification: CustomNotificationRequest
): string {
  let safe = message;
  for (const value of sensitiveStrings({ props, notification }).sort(
    (left, right) => right.length - left.length
  )) {
    safe = safe.split(value).join('[REDACTED]');
  }
  return safe.slice(0, 1024);
}

function printable(value: unknown): string {
  if (typeof value === 'string') return value;
  if (
    value === undefined ||
    value === null ||
    typeof value === 'number' ||
    typeof value === 'boolean' ||
    typeof value === 'bigint'
  ) {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return Object.prototype.toString.call(value);
  }
}

function managedConsole(
  props: Readonly<Record<string, unknown>>,
  notification: CustomNotificationRequest
) {
  const sensitive = sensitiveStrings({
    props,
    idempotencyKey: notification.idempotencyKey,
    actions: notification.actions,
  }).sort((left, right) => right.length - left.length);
  const write = (...values: unknown[]) => {
    let message = values.map(printable).join(' ');
    for (const value of sensitive) {
      message = message.split(value).join('[REDACTED]');
    }
    process.stderr.write(`${message.slice(0, 4096)}\n`);
  };
  return deepFreeze({
    debug: write,
    error: write,
    info: write,
    log: write,
    warn: write,
  });
}

function validReceipt(value: unknown): value is CustomProviderReceipt {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    Object.keys(record).every(key => key === 'messageId') &&
    (record.messageId === undefined ||
      (typeof record.messageId === 'string' &&
        record.messageId.length >= 1 &&
        [...record.messageId].length <= 512))
  );
}

class ManagedServiceError extends Error {
  constructor(
    message: string,
    readonly retryable: boolean
  ) {
    super(message);
    this.name = 'ManagedServiceError';
  }
}

interface ManagedHttpRequest {
  readonly url: string;
  readonly method?: string;
  readonly headers?: Readonly<Record<string, string>>;
  readonly query?: Readonly<Record<string, unknown>>;
  readonly json?: unknown;
  readonly text?: string;
  readonly responseType?: 'json' | 'text';
  readonly timeoutMs?: number;
  readonly acceptedStatus?: {
    readonly minimum: number;
    readonly maximum: number;
  };
}

const services = deepFreeze({
  http: {
    async request(input: ManagedHttpRequest) {
      if (
        typeof input !== 'object' ||
        input === null ||
        typeof input.url !== 'string'
      ) {
        throw new TypeError(
          'services.http.request() requires a request object with a URL.'
        );
      }
      const url = new URL(input.url);
      for (const [name, raw] of Object.entries(input.query ?? {})) {
        for (const value of Array.isArray(raw) ? raw : [raw]) {
          if (value !== undefined && value !== null) {
            url.searchParams.append(name, String(value));
          }
        }
      }
      const timeoutMs = input.timeoutMs ?? 30_000;
      if (
        !Number.isSafeInteger(timeoutMs) ||
        timeoutMs < 1 ||
        timeoutMs > 86_400_000
      ) {
        throw new TypeError(
          'Managed HTTP timeout must be between 1 ms and 24 hours.'
        );
      }
      if (input.json !== undefined && input.text !== undefined) {
        throw new TypeError('Managed HTTP accepts only one of json or text.');
      }
      const headers = new Headers(input.headers);
      let body: string | undefined;
      if (input.json !== undefined) {
        if (!headers.has('content-type')) {
          headers.set('content-type', 'application/json');
        }
        body = JSON.stringify(input.json);
      } else if (input.text !== undefined) {
        body = input.text;
      }
      let response: Response;
      try {
        response = await fetch(url, {
          method: input.method?.toUpperCase() ?? 'GET',
          headers,
          body,
          signal: AbortSignal.timeout(timeoutMs),
        });
      } catch {
        throw new ManagedServiceError('The managed HTTP request failed.', true);
      }
      const accepted = input.acceptedStatus ?? { minimum: 200, maximum: 299 };
      if (
        response.status < accepted.minimum ||
        response.status > accepted.maximum
      ) {
        throw new ManagedServiceError(
          `The managed HTTP response status ${response.status} is outside the accepted range.`,
          response.status === 429 || response.status >= 500
        );
      }
      const data =
        input.responseType === 'text'
          ? await response.text()
          : await response.json();
      return deepFreeze({
        ok: response.ok,
        status: response.status,
        headers: Object.fromEntries(response.headers.entries()),
        data,
        url: response.url,
        redirected: response.redirected,
      });
    },
  },
});

async function execute(request: WorkerRequest): Promise<WorkerResponse> {
  try {
    const run = new AsyncFunction(
      'props',
      'notification',
      'attempt',
      'services',
      'console',
      `"use strict";\n${request.source}`
    );
    const result = await run(
      deepFreeze(structuredClone(request.props)),
      deepFreeze(structuredClone(request.notification)),
      deepFreeze(structuredClone(request.attempt)),
      services,
      managedConsole(request.props, request.notification)
    );
    if (!validReceipt(result)) {
      return {
        ok: false,
        kind: 'non_json',
        message: 'Provider scripts must return {} or { messageId: string }.',
        retryable: false,
      };
    }
    return { ok: true, receipt: result };
  } catch (error) {
    const raw = error instanceof Error ? error.message : String(error);
    return {
      ok: false,
      kind:
        error instanceof ManagedServiceError
          ? 'service_failed'
          : 'script_threw',
      message: redact(raw, request.props, request.notification),
      retryable: error instanceof ManagedServiceError && error.retryable,
    };
  }
}

if (parentPort === null) throw new Error('Custom provider worker requires a parent port.');
parentPort.once('message', async (request: WorkerRequest) => {
  parentPort!.postMessage(await execute(request));
});
