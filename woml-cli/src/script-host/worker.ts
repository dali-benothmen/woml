import { deepFreezeJson, findJsonViolation } from './json';
import type {
  CapabilityCallRequest,
  CapabilityCallResult,
  CapabilityFailure,
  FetchObservationAckMessage,
  JsonObject,
  JsonValue,
  NativeFetchObservation,
  ScriptAttempt,
  ScriptBindingsV1,
  ScriptContext,
} from './types';

export interface ScriptWorkerRequest {
  readonly invocationId: string;
  readonly runId: string;
  readonly nodeId: string;
  readonly source: string;
  readonly context: ScriptContext;
  readonly attempt?: ScriptAttempt;
  readonly bindings?: ScriptBindingsV1;
}

export type ScriptWorkerResponse =
  | { readonly ok: true; readonly result: JsonValue }
  | {
      readonly ok: false;
      readonly error: {
        readonly kind: 'script' | 'non-json' | 'service';
        readonly name: string;
        readonly message: string;
        readonly stack?: string;
        readonly capability?: string;
        readonly operation?: string;
        readonly callId?: string;
        readonly cause?: CapabilityFailure;
      };
    };

export type ScriptWorkerInbound =
  | { readonly messageType: 'execute'; readonly request: ScriptWorkerRequest }
  | {
      readonly messageType: 'capability_result';
      readonly callId: string;
      readonly result: CapabilityCallResult;
    }
  | {
      readonly messageType: 'fetch_observation_ack';
      readonly requestId: string;
      readonly ack: FetchObservationAckMessage;
    };

export type ScriptWorkerOutbound =
  | {
      readonly messageType: 'completed';
      readonly response: ScriptWorkerResponse;
    }
  | {
      readonly messageType: 'capability_call';
      readonly call: CapabilityCallRequest;
    }
  | {
      readonly messageType: 'fetch_observation';
      readonly observation: NativeFetchObservation;
    };

type AsyncFunction = (
  context: ScriptContext,
  attempt: ScriptAttempt | undefined,
  services: unknown,
  secrets: Readonly<Record<string, string>>
) => Promise<unknown>;
type AsyncFunctionConstructor = new (
  ...parametersAndBody: string[]
) => AsyncFunction;

const AsyncFunction = Object.getPrototypeOf(
  async function emptyAsyncFunction() {}
).constructor as AsyncFunctionConstructor;

function serializeError(error: unknown): ScriptWorkerResponse {
  if (error instanceof NativeFetchTrackingError) {
    return {
      ok: false,
      error: {
        kind: 'service',
        name: error.name,
        message: error.message,
        capability: 'http',
        operation: 'fetch',
        callId: error.callId,
        cause: error.cause,
      },
    };
  }
  if (
    (typeof error === 'object' && error !== null) ||
    typeof error === 'function'
  ) {
    const nativeFetch = nativeFetchFailures.get(error);
    if (nativeFetch !== undefined) {
      return {
        ok: false,
        error: {
          kind: 'service',
          name: error instanceof Error ? error.name : 'Error',
          message: nativeFetch.cause.message,
          capability: 'http',
          operation: 'fetch',
          callId: nativeFetch.callId,
          cause: nativeFetch.cause,
        },
      };
    }
  }
  if (error instanceof ServiceCallError) {
    return {
      ok: false,
      error: {
        kind: 'service',
        name: error.name,
        message: error.message,
        capability: error.capability,
        operation: error.operation,
        callId: error.callId,
        cause: error.cause,
      },
    };
  }
  if (error instanceof Error) {
    return {
      ok: false,
      error: {
        kind: 'script',
        name: error.name,
        message: error.message,
        ...(error.stack === undefined ? {} : { stack: error.stack }),
      },
    };
  }
  return {
    ok: false,
    error: {
      kind: 'script',
      name: 'Error',
      message: String(error),
    },
  };
}

class ServiceCallError extends Error {
  readonly code: string;
  readonly service: string;
  readonly capability: string;
  readonly operation: string;
  readonly callId: string;
  readonly retryable: boolean;
  readonly ambiguous: boolean;
  readonly details?: Readonly<Record<string, JsonValue>>;
  readonly cause: CapabilityFailure;

  constructor(
    capability: string,
    operation: string,
    callId: string,
    cause: CapabilityFailure
  ) {
    super(cause.message);
    this.name = 'WomlServiceError';
    this.code = cause.code;
    this.service = capability;
    this.capability = capability;
    this.operation = operation;
    this.callId = callId;
    this.retryable = cause.retryable;
    this.ambiguous = cause.ambiguous;
    this.details = cause.details;
    this.cause = cause;
  }
}

interface PendingCapabilityCall {
  readonly capability: string;
  readonly operation: string;
  readonly resolve: (value: JsonValue) => void;
  readonly reject: (error: ServiceCallError) => void;
}

const pendingCalls = new Map<string, PendingCapabilityCall>();
const pendingFetchAcks = new Map<
  string,
  {
    readonly resolve: () => void;
    readonly reject: (error: NativeFetchTrackingError) => void;
  }
>();
const operationSequences = new Map<string, number>();
const automaticEffectfulCalls = new Map<string, number>();
let fetchSequence = 0;
const nativeFetch = globalThis.fetch.bind(globalThis);
const nativeFetchFailures = new WeakMap<
  object,
  { readonly callId: string; readonly cause: CapabilityFailure }
>();

class NativeFetchTrackingError extends Error {
  readonly callId: string;
  readonly cause: CapabilityFailure;
  readonly code: string;
  readonly retryable: boolean;
  readonly ambiguous: boolean;

  constructor(callId: string, failure: CapabilityFailure) {
    super(failure.message);
    this.name = 'WomlFetchTrackingError';
    this.callId = callId;
    this.cause = failure;
    this.code = failure.code;
    this.retryable = failure.retryable;
    this.ambiguous = failure.ambiguous;
  }
}

function requestBodyBytes(
  body: BodyInit | null | undefined
): number | undefined {
  if (body === undefined || body === null) return undefined;
  if (typeof body === 'string') return Buffer.byteLength(body, 'utf8');
  if (body instanceof URLSearchParams) {
    return Buffer.byteLength(body.toString(), 'utf8');
  }
  if (body instanceof Blob) return body.size;
  if (body instanceof ArrayBuffer) return body.byteLength;
  if (ArrayBuffer.isView(body)) return body.byteLength;
  return undefined;
}

function fetchStartObservation(
  request: ScriptWorkerRequest,
  requestId: string,
  input: RequestInfo | URL,
  init?: RequestInit
): NativeFetchObservation | undefined {
  const rawUrl =
    input instanceof Request
      ? input.url
      : input instanceof URL
        ? input.href
        : String(input);
  const url = new URL(rawUrl);
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return undefined;
  }
  const method = String(
    init?.method ?? (input instanceof Request ? input.method : 'GET')
  ).toUpperCase();
  const bytes = requestBodyBytes(init?.body);
  return {
    contract: 'woml.native-fetch-observation',
    contractVersion: 1,
    observationType: 'started',
    invocationId: request.invocationId,
    requestId,
    method,
    origin: url.origin,
    path: url.pathname,
    ...(bytes === undefined ? {} : { requestBodyBytes: bytes }),
    startedAt: new Date().toISOString(),
  };
}

async function observeFetch(
  observation: NativeFetchObservation
): Promise<void> {
  if (pendingFetchAcks.has(observation.requestId)) {
    throw new Error('Native Fetch reused an active request ID.');
  }
  await new Promise<void>((resolve, reject) => {
    pendingFetchAcks.set(observation.requestId, { resolve, reject });
    self.postMessage({
      messageType: 'fetch_observation',
      observation,
    } satisfies ScriptWorkerOutbound);
  });
}

function trackedNativeFetch(request: ScriptWorkerRequest): typeof fetch {
  const tracked = async (
    input: RequestInfo | URL,
    init?: RequestInit
  ): Promise<Response> => {
    const requestId = `fetch_${++fetchSequence}_${crypto.randomUUID().replaceAll('-', '')}`;
    const startedAt = performance.now();
    const start = fetchStartObservation(request, requestId, input, init);
    if (start === undefined) return nativeFetch(input, init);
    await observeFetch(start);

    let response: Response;
    try {
      response = await nativeFetch(input, init);
    } catch (error) {
      const name = error instanceof Error ? error.name : '';
      const kind =
        name === 'AbortError'
          ? 'cancelled'
          : name === 'TimeoutError'
            ? 'timed_out'
            : 'fetch_rejected';
      const failure: CapabilityFailure = {
        kind:
          kind === 'cancelled'
            ? 'cancelled'
            : kind === 'timed_out'
              ? 'timed_out'
              : 'transport_failed',
        code:
          kind === 'cancelled'
            ? 'WOML_NATIVE_FETCH_CANCELLED'
            : kind === 'timed_out'
              ? 'WOML_NATIVE_FETCH_TIMED_OUT'
              : 'WOML_NATIVE_FETCH_REJECTED',
        message:
          kind === 'cancelled'
            ? 'Bun Fetch was cancelled.'
            : kind === 'timed_out'
              ? 'Bun Fetch exceeded its deadline.'
              : 'Bun Fetch rejected the request.',
        retryable: false,
        ambiguous: true,
      };
      await observeFetch({
        contract: 'woml.native-fetch-observation',
        contractVersion: 1,
        observationType: 'failed',
        invocationId: request.invocationId,
        requestId,
        durationMs: Math.max(0, performance.now() - startedAt),
        failedAt: new Date().toISOString(),
        error: {
          kind,
          code: failure.code,
          message: failure.message,
        },
      });
      if (
        (typeof error === 'object' && error !== null) ||
        typeof error === 'function'
      ) {
        nativeFetchFailures.set(error, { callId: requestId, cause: failure });
      }
      throw error;
    }

    await observeFetch({
      contract: 'woml.native-fetch-observation',
      contractVersion: 1,
      observationType: 'completed',
      invocationId: request.invocationId,
      requestId,
      status: response.status,
      responseBodyBytes: null,
      durationMs: Math.max(0, performance.now() - startedAt),
      completedAt: new Date().toISOString(),
    });
    return response;
  };
  return tracked as typeof fetch;
}

async function operationKey(
  stepIdempotencyKey: string,
  operationName: string
): Promise<string> {
  const bytes = new TextEncoder().encode(
    `woml.capability-operation\0v1\0${stepIdempotencyKey}\0${operationName}`
  );
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return `sha256:${[...digest]
    .map(byte => byte.toString(16).padStart(2, '0'))
    .join('')}`;
}

function callId(): string {
  return `call_${crypto.randomUUID().replaceAll('-', '')}`;
}

function plainObject(value: unknown): Record<string, JsonValue> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, JsonValue>)
    : undefined;
}

function httpTimeoutMilliseconds(input: Record<string, JsonValue>): number {
  if (input.timeout !== undefined && input.timeoutMs !== undefined) {
    throw new TypeError('Managed HTTP accepts timeout or timeoutMs, not both.');
  }
  const value = input.timeout ?? input.timeoutMs ?? 30_000;
  if (typeof value === 'number' && Number.isSafeInteger(value)) return value;
  if (typeof value !== 'string') {
    throw new TypeError(
      'Managed HTTP timeout must be milliseconds or a duration string.'
    );
  }
  const match = value.match(/^(\d+)(ms|s|m|h)$/);
  if (match === null) {
    throw new TypeError(
      'Managed HTTP timeout must look like 500ms, 10s, 2m, or 1h.'
    );
  }
  const multiplier =
    match[2] === 'ms'
      ? 1
      : match[2] === 's'
        ? 1_000
        : match[2] === 'm'
          ? 60_000
          : 3_600_000;
  return Number(match[1]) * multiplier;
}

function normalizeHttpRequest(input: JsonValue): JsonObject {
  const object = plainObject(input);
  if (object === undefined) {
    throw new TypeError('services.http.request() requires a request object.');
  }
  const allowed = new Set([
    'url',
    'method',
    'headers',
    'query',
    'json',
    'text',
    'bytesBase64',
    'responseType',
    'storage',
    'timeout',
    'timeoutMs',
    'acceptedStatus',
    'redirect',
    'maximumRedirects',
    'idempotency',
  ]);
  const unknown = Object.keys(object).find(key => !allowed.has(key));
  if (unknown !== undefined) {
    throw new TypeError(`Unknown managed HTTP option "${unknown}".`);
  }
  if (typeof object.url !== 'string' || object.url.length === 0) {
    throw new TypeError('Managed HTTP requires a URL string.');
  }
  const method = String(object.method ?? 'GET').toUpperCase();
  const headers = plainObject(object.headers ?? {});
  if (
    headers === undefined ||
    Object.values(headers).some(value => typeof value !== 'string')
  ) {
    throw new TypeError('Managed HTTP headers must contain string values.');
  }
  const timeoutMs = httpTimeoutMilliseconds(object);
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 86_400_000
  ) {
    throw new TypeError(
      'Managed HTTP timeout must be between 1 ms and 24 hours.'
    );
  }
  const responseType = object.responseType ?? 'json';
  const storage = plainObject(object.storage);
  if (responseType === 'storage') {
    if (storage === undefined) {
      throw new TypeError(
        'Managed HTTP responseType "storage" requires a storage target.'
      );
    }
    const unknownStorageOption = Object.keys(storage).find(
      key => !['key', 'contentType', 'overwrite', 'ifVersion'].includes(key)
    );
    if (unknownStorageOption !== undefined) {
      throw new TypeError(
        `Unknown managed HTTP storage option "${unknownStorageOption}".`
      );
    }
    if (typeof storage.key !== 'string' || storage.key.length === 0) {
      throw new TypeError('Managed HTTP storage requires a key string.');
    }
    if (
      storage.contentType !== undefined &&
      typeof storage.contentType !== 'string'
    ) {
      throw new TypeError('Managed HTTP storage contentType must be a string.');
    }
    if (
      storage.overwrite !== undefined &&
      typeof storage.overwrite !== 'boolean'
    ) {
      throw new TypeError('Managed HTTP storage overwrite must be a Boolean.');
    }
    if (
      storage.ifVersion !== undefined &&
      typeof storage.ifVersion !== 'string'
    ) {
      throw new TypeError('Managed HTTP storage ifVersion must be a string.');
    }
    if (storage.overwrite !== undefined && storage.ifVersion !== undefined) {
      throw new TypeError(
        'Managed HTTP storage overwrite and ifVersion are mutually exclusive.'
      );
    }
  } else if (object.storage !== undefined) {
    throw new TypeError(
      'Managed HTTP storage is valid only with responseType "storage".'
    );
  }
  const normalized: Record<string, JsonValue> = {
    contract: 'woml.managed-http',
    contractVersion: 1,
    kind: 'request',
    method,
    url: object.url,
    headers,
    responseType,
    timeoutMs,
    acceptedStatus: object.acceptedStatus ?? { minimum: 200, maximum: 299 },
    redirect: object.redirect ?? 'follow',
    maximumRedirects: object.maximumRedirects ?? 10,
  };
  for (const field of [
    'query',
    'json',
    'text',
    'bytesBase64',
    'idempotency',
    'storage',
  ] as const) {
    if (object[field] !== undefined) normalized[field] = object[field];
  }
  return normalized;
}

function namedOperation(
  capability: string,
  operation: string,
  options: JsonValue | undefined
): { readonly mode: 'automatic' | 'named'; readonly name: string } {
  if (options === undefined) {
    const key = `${capability}.${operation}`;
    const sequence = (operationSequences.get(key) ?? 0) + 1;
    operationSequences.set(key, sequence);
    return {
      mode: 'automatic',
      name: sequence === 1 ? key : `${key}.${sequence}`,
    };
  }
  const object = plainObject(options);
  const name = object?.name;
  if (
    object === undefined ||
    Object.keys(object).length !== 1 ||
    typeof name !== 'string' ||
    !/^[a-z][a-z0-9._-]{0,127}$/.test(name)
  ) {
    throw new TypeError(
      'Service call options must be exactly { name: "stable-operation-name" }.'
    );
  }
  return { mode: 'named', name: `${capability}.${operation}.${name}` };
}

function publicHttpResult(result: JsonValue): JsonObject {
  const object = plainObject(result);
  if (
    object?.contract !== 'woml.managed-http' ||
    object.contractVersion !== 1 ||
    object.kind !== 'result'
  ) {
    throw new TypeError('Rust returned an invalid Managed HTTP v1 result.');
  }
  return {
    status: object.status,
    ok: object.ok,
    headers: object.headers,
    data: object.data,
    url: object.url,
    redirected: object.redirected,
  };
}

const databaseOperations = new Set([
  'query',
  'execute',
  'read',
  'insert',
  'update',
  'delete',
  'transaction',
]);

function normalizeDatabaseConfig(input: JsonValue): JsonObject {
  const object = plainObject(input);
  if (
    object === undefined ||
    Object.keys(object).some(key => key !== 'driver' && key !== 'connection') ||
    (object.driver !== 'sqlite' && object.driver !== 'postgres') ||
    typeof object.connection !== 'string' ||
    object.connection.length === 0
  ) {
    throw new TypeError(
      'services.db() requires exactly { driver: "sqlite" | "postgres", connection: "database connection" }.'
    );
  }
  return { driver: object.driver, connection: object.connection };
}

function normalizeDatabaseRequest(
  config: JsonObject,
  operation: string,
  input: JsonValue
): JsonObject {
  if (!databaseOperations.has(operation)) {
    throw new TypeError(`Unknown Database v1 operation "${operation}".`);
  }
  return {
    contract: 'woml.database',
    contractVersion: 1,
    kind: 'request',
    driver: config.driver,
    connection: config.connection,
    operation,
    input,
  };
}

function publicDatabaseResult(result: JsonValue, operation: string): JsonValue {
  const object = plainObject(result);
  if (
    object?.contract !== 'woml.database' ||
    object.contractVersion !== 1 ||
    object.kind !== 'result' ||
    object.operation !== operation ||
    object.data === undefined
  ) {
    throw new TypeError('Rust returned an invalid Database v1 result.');
  }
  return object.data;
}

const storageOperations = new Set(['put', 'get', 'head', 'list', 'delete']);
const cacheOperations = new Set([
  'get',
  'set',
  'delete',
  'has',
  'increment',
  'setIfAbsent',
]);
const cacheWireOperations = new Set([
  'get',
  'set',
  'delete',
  'has',
  'increment',
  'set_if_absent',
]);
const eventOperations = new Set(['emit']);

function normalizeStorageInput(
  operation: string,
  rawInput: JsonValue
): JsonObject {
  const object = plainObject(rawInput);
  if (object === undefined) {
    throw new TypeError(`services.storage.${operation}() requires an object.`);
  }
  const allowedByOperation: Readonly<Record<string, readonly string[]>> = {
    put: [
      'key',
      'value',
      'text',
      'bytesBase64',
      'contentType',
      'overwrite',
      'ifVersion',
    ],
    get: ['key', 'responseType', 'ifVersion'],
    head: ['key'],
    list: ['prefix', 'limit', 'cursor'],
    delete: ['key', 'ifVersion'],
  };
  const allowed = allowedByOperation[operation];
  if (allowed === undefined) {
    throw new TypeError(`Unknown Storage v1 operation "${operation}".`);
  }
  const unknown = Object.keys(object).find(key => !allowed.includes(key));
  if (unknown !== undefined) {
    throw new TypeError(`Unknown Storage v1 ${operation} option "${unknown}".`);
  }
  if (operation === 'get') {
    return { ...object, responseType: object.responseType ?? 'json' };
  }
  if (operation === 'list') {
    return { prefix: '', limit: 100, ...object };
  }
  return object;
}

function normalizeStorageRequest(
  operation: string,
  input: JsonValue
): JsonObject {
  if (!storageOperations.has(operation)) {
    throw new TypeError(`Unknown Storage v1 operation "${operation}".`);
  }
  return {
    contract: 'woml.storage',
    contractVersion: 1,
    kind: 'request',
    operation,
    input: normalizeStorageInput(operation, input),
  };
}

function publicStorageResult(result: JsonValue, operation: string): JsonValue {
  const object = plainObject(result);
  if (
    object?.contract !== 'woml.storage' ||
    object.contractVersion !== 1 ||
    object.kind !== 'result' ||
    object.operation !== operation ||
    object.data === undefined
  ) {
    throw new TypeError('Rust returned an invalid Storage v1 result.');
  }
  return object.data;
}

function cacheTtlMilliseconds(value: JsonValue | undefined): number {
  if (value === undefined) return 300_000;
  if (typeof value === 'number' && Number.isSafeInteger(value)) {
    if (value >= 1 && value <= 2_592_000_000) return value;
  } else if (typeof value === 'string') {
    const match = value.match(/^(\d+)(ms|s|m|h|d)$/);
    if (match !== null) {
      const multiplier =
        match[2] === 'ms'
          ? 1
          : match[2] === 's'
            ? 1_000
            : match[2] === 'm'
              ? 60_000
              : match[2] === 'h'
                ? 3_600_000
                : 86_400_000;
      const ttl = Number(match[1]) * multiplier;
      if (Number.isSafeInteger(ttl) && ttl >= 1 && ttl <= 2_592_000_000) {
        return ttl;
      }
    }
  }
  throw new TypeError(
    'Cache ttl must be milliseconds or a whole duration from 1ms through 30d.'
  );
}

function normalizeCacheOptions(
  value: unknown,
  acceptsTtl: boolean
): { readonly ttlMs: number; readonly callOptions?: JsonObject } {
  if (value === undefined) {
    return { ttlMs: 300_000 };
  }
  const object = plainObject(value);
  const allowed = acceptsTtl ? ['ttl', 'name'] : ['name'];
  if (
    object === undefined ||
    Object.keys(object).some(key => !allowed.includes(key)) ||
    (object.name !== undefined && typeof object.name !== 'string')
  ) {
    throw new TypeError(
      acceptsTtl
        ? 'Cache options accept only ttl and a stable name.'
        : 'Cache options accept only a stable name.'
    );
  }
  return {
    ttlMs: cacheTtlMilliseconds(object.ttl),
    ...(object.name === undefined
      ? {}
      : { callOptions: { name: object.name } }),
  };
}

function normalizeCacheRequest(
  operation: string,
  args: readonly unknown[]
): { readonly request: JsonObject; readonly callOptions?: JsonObject } {
  if (!cacheOperations.has(operation)) {
    throw new TypeError(`Unknown Cache v1 operation "${operation}".`);
  }
  const key = args[0];
  if (typeof key !== 'string') {
    throw new TypeError(`services.cache.${operation}() requires a string key.`);
  }
  let input: JsonObject;
  let callOptions: JsonObject | undefined;
  const wireOperation =
    operation === 'setIfAbsent' ? 'set_if_absent' : operation;
  if (operation === 'get' || operation === 'has') {
    if (args.length !== 1) {
      throw new TypeError(`services.cache.${operation}() accepts only a key.`);
    }
    input = { key };
  } else if (operation === 'delete') {
    if (args.length > 2) {
      throw new TypeError('services.cache.delete() accepts key and options.');
    }
    const normalized = normalizeCacheOptions(args[1], false);
    input = { key };
    callOptions = normalized.callOptions;
  } else if (operation === 'set' || operation === 'setIfAbsent') {
    if (args.length < 2 || args.length > 3) {
      throw new TypeError(
        `services.cache.${operation}() requires key, value, and optional options.`
      );
    }
    const normalized = normalizeCacheOptions(args[2], true);
    input = { key, value: args[1] as JsonValue, ttlMs: normalized.ttlMs };
    callOptions = normalized.callOptions;
  } else {
    if (args.length > 3) {
      throw new TypeError(
        'services.cache.increment() accepts key, optional amount, and options.'
      );
    }
    const secondIsOptions = plainObject(args[1]) !== undefined;
    const amount = args[1] === undefined || secondIsOptions ? 1 : args[1];
    if (typeof amount !== 'number' || !Number.isSafeInteger(amount)) {
      throw new TypeError('Cache increment amount must be a safe integer.');
    }
    const normalized = normalizeCacheOptions(
      secondIsOptions ? args[1] : args[2],
      true
    );
    input = { key, amount, ttlMs: normalized.ttlMs };
    callOptions = normalized.callOptions;
  }
  return {
    request: {
      contract: 'woml.cache',
      contractVersion: 1,
      kind: 'request',
      operation: wireOperation,
      input,
    },
    ...(callOptions === undefined ? {} : { callOptions }),
  };
}

function publicCacheResult(result: JsonValue, operation: string): JsonValue {
  const object = plainObject(result);
  if (
    object?.contract !== 'woml.cache' ||
    object.contractVersion !== 1 ||
    object.kind !== 'result' ||
    object.operation !== operation ||
    object.data === undefined
  ) {
    throw new TypeError('Rust returned an invalid Cache v1 result.');
  }
  return object.data;
}

function normalizeEventEmit(args: readonly unknown[]): {
  readonly request: JsonObject;
  readonly callOptions?: JsonObject;
} {
  if (args.length < 1 || args.length > 3) {
    throw new TypeError(
      'services.events.emit() requires an event name, optional payload, and optional options.'
    );
  }
  const name = args[0];
  if (
    typeof name !== 'string' ||
    !/^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/.test(name) ||
    Buffer.byteLength(name, 'utf8') > 256
  ) {
    throw new TypeError(
      'services.events.emit() requires a valid lowercase dotted event name.'
    );
  }
  const payload = args[1] ?? {};
  if (plainObject(payload) === undefined) {
    throw new TypeError('Event payload must be a top-level JSON object.');
  }
  const options = args[2];
  if (options !== undefined) {
    // namedOperation performs the authoritative options validation.
    namedOperation('events', 'emit', options as JsonValue);
  }
  return {
    request: {
      contract: 'woml.events',
      contractVersion: 1,
      kind: 'request',
      operation: 'emit',
      input: { name, payload: payload as JsonValue },
    },
    ...(options === undefined ? {} : { callOptions: options as JsonObject }),
  };
}

function publicEventResult(result: JsonValue): JsonValue {
  const object = plainObject(result);
  if (
    object?.contract !== 'woml.events' ||
    object.contractVersion !== 1 ||
    object.kind !== 'result' ||
    object.operation !== 'emit' ||
    object.data === undefined
  ) {
    throw new TypeError('Rust returned an invalid Events Service v1 result.');
  }
  return object.data;
}

function deeplyReadonlyServiceFacade(request: ScriptWorkerRequest): unknown {
  if (request.attempt === undefined || request.bindings === undefined) {
    return Object.freeze({});
  }
  const attempt = request.attempt;
  const capabilityCache = new Map<string, unknown>();
  const invokeCapability = async (
    capability: string,
    operation: string,
    input: JsonValue,
    callOptions?: JsonValue
  ): Promise<JsonValue> => {
    const managedHttp = capability === 'http' && operation === 'request';
    const managedDatabase =
      capability === 'db' && databaseOperations.has(operation);
    const managedStorage =
      capability === 'storage' && storageOperations.has(operation);
    const managedCache =
      capability === 'cache' && cacheWireOperations.has(operation);
    const managedEvents =
      capability === 'events' && eventOperations.has(operation);
    if (
      !managedHttp &&
      !managedDatabase &&
      !managedStorage &&
      !managedCache &&
      !managedEvents &&
      callOptions !== undefined
    ) {
      throw new TypeError(
        'Named service-call options are supported by managed WOML services only.'
      );
    }
    const violation = findJsonViolation(input);
    if (violation !== undefined) {
      throw new TypeError(`${violation.path}: ${violation.reason}`);
    }
    const inputBytes = Buffer.byteLength(JSON.stringify(input), 'utf8');
    if (inputBytes > 1_048_576) {
      const id = callId();
      throw new ServiceCallError(capability, operation, id, {
        kind: 'input_too_large',
        code: 'WOML_CAPABILITY_INPUT_TOO_LARGE',
        message: 'The capability input exceeds its configured byte limit.',
        retryable: false,
        ambiguous: false,
        details: { actualBytes: inputBytes, limitBytes: 1_048_576 },
      });
    }
    const identity = namedOperation(capability, operation, callOptions);
    const method = managedHttp
      ? String((input as JsonObject).method)
      : undefined;
    const effectful = managedHttp
      ? (input as JsonObject).responseType === 'storage' ||
        (method !== 'GET' && method !== 'HEAD' && method !== 'OPTIONS')
      : managedDatabase
        ? operation !== 'query' && operation !== 'read'
        : managedStorage && (operation === 'put' || operation === 'delete');
    const cacheEffectful =
      managedCache && operation !== 'get' && operation !== 'has';
    if (
      (effectful || cacheEffectful || managedEvents) &&
      identity.mode === 'automatic'
    ) {
      const key = `${capability}.${operation}`;
      const count = (automaticEffectfulCalls.get(key) ?? 0) + 1;
      automaticEffectfulCalls.set(key, count);
      if (count > 1) {
        throw new TypeError(
          `Multiple effectful services.${capability}.${operation}() calls in one step require stable names, for example { name: "write-customer" }.`
        );
      }
    }
    const httpIdempotency = managedHttp
      ? plainObject((input as JsonObject).idempotency)
      : undefined;
    const providerIdempotencyKey =
      typeof httpIdempotency?.value === 'string'
        ? httpIdempotency.value
        : undefined;
    const timeoutMs = managedHttp
      ? Math.min(86_400_000, Number((input as JsonObject).timeoutMs) + 1_000)
      : 30_000;
    const id = callId();
    const call: CapabilityCallRequest = {
      contract: 'woml.capability-call',
      contractVersion: 1,
      messageType: 'request',
      invocationId: request.invocationId,
      callId: id,
      runId: request.runId,
      nodeId: request.nodeId,
      attemptNumber: attempt.number,
      capability,
      operation,
      inputContractVersion: 1,
      resultContractVersion: 1,
      identity: {
        mode: identity.mode,
        stepIdempotencyKey: attempt.idempotencyKey,
        operationName: identity.name,
        operationKey: await operationKey(attempt.idempotencyKey, identity.name),
        ...(providerIdempotencyKey === undefined
          ? {}
          : { providerIdempotencyKey }),
      },
      limits: {
        inputBytes: 1_048_576,
        resultBytes: 4_194_304,
        timeoutMs,
      },
      input,
    };
    const result = await new Promise<JsonValue>((resolve, reject) => {
      pendingCalls.set(id, { capability, operation, resolve, reject });
      self.postMessage({
        messageType: 'capability_call',
        call,
      } satisfies ScriptWorkerOutbound);
    });
    return managedHttp
      ? publicHttpResult(result)
      : managedDatabase
        ? publicDatabaseResult(result, operation)
        : managedStorage
          ? publicStorageResult(result, operation)
          : managedCache
            ? publicCacheResult(result, operation)
            : managedEvents
              ? publicEventResult(result)
              : result;
  };
  return new Proxy(Object.freeze({}), {
    get(_target, capabilityProperty) {
      if (typeof capabilityProperty !== 'string') return undefined;
      const cached = capabilityCache.get(capabilityProperty);
      if (cached !== undefined) return cached;
      if (capabilityProperty === 'db') {
        const database = (rawConfig: JsonValue): unknown => {
          const config = Object.freeze(normalizeDatabaseConfig(rawConfig));
          const operationCache = new Map<string, unknown>();
          return new Proxy(Object.freeze({}), {
            get(_databaseTarget, operationProperty) {
              if (typeof operationProperty !== 'string') return undefined;
              const known = operationCache.get(operationProperty);
              if (known !== undefined) return known;
              if (!databaseOperations.has(operationProperty)) {
                return undefined;
              }
              const invoke = async (
                rawInput: JsonValue = null,
                callOptions?: JsonValue
              ): Promise<JsonValue> =>
                invokeCapability(
                  'db',
                  operationProperty,
                  normalizeDatabaseRequest(config, operationProperty, rawInput),
                  callOptions
                );
              Object.freeze(invoke);
              operationCache.set(operationProperty, invoke);
              return invoke;
            },
            set: () => false,
            defineProperty: () => false,
            deleteProperty: () => false,
          });
        };
        Object.freeze(database);
        capabilityCache.set(capabilityProperty, database);
        return database;
      }
      if (capabilityProperty === 'storage') {
        const operationCache = new Map<string, unknown>();
        const storage = new Proxy(Object.freeze({}), {
          get(_storageTarget, operationProperty) {
            if (typeof operationProperty !== 'string') return undefined;
            const known = operationCache.get(operationProperty);
            if (known !== undefined) return known;
            if (!storageOperations.has(operationProperty)) return undefined;
            const invoke = async (
              rawInput: JsonValue = {},
              callOptions?: JsonValue
            ): Promise<JsonValue> =>
              invokeCapability(
                'storage',
                operationProperty,
                normalizeStorageRequest(operationProperty, rawInput),
                callOptions
              );
            Object.freeze(invoke);
            operationCache.set(operationProperty, invoke);
            return invoke;
          },
          set: () => false,
          defineProperty: () => false,
          deleteProperty: () => false,
        });
        capabilityCache.set(capabilityProperty, storage);
        return storage;
      }
      if (capabilityProperty === 'cache') {
        const operationCache = new Map<string, unknown>();
        const cache = new Proxy(Object.freeze({}), {
          get(_cacheTarget, operationProperty) {
            if (typeof operationProperty !== 'string') return undefined;
            const known = operationCache.get(operationProperty);
            if (known !== undefined) return known;
            if (!cacheOperations.has(operationProperty)) return undefined;
            const invoke = async (...args: unknown[]): Promise<JsonValue> => {
              const normalized = normalizeCacheRequest(operationProperty, args);
              const wireOperation = String(normalized.request.operation);
              return invokeCapability(
                'cache',
                wireOperation,
                normalized.request,
                normalized.callOptions
              );
            };
            Object.freeze(invoke);
            operationCache.set(operationProperty, invoke);
            return invoke;
          },
          set: () => false,
          defineProperty: () => false,
          deleteProperty: () => false,
        });
        capabilityCache.set(capabilityProperty, cache);
        return cache;
      }
      if (capabilityProperty === 'events') {
        const emit = async (...args: unknown[]): Promise<JsonValue> => {
          const normalized = normalizeEventEmit(args);
          return invokeCapability(
            'events',
            'emit',
            normalized.request,
            normalized.callOptions
          );
        };
        Object.freeze(emit);
        const events = Object.freeze({ emit });
        capabilityCache.set(capabilityProperty, events);
        return events;
      }
      const operationCache = new Map<string, unknown>();
      const capability = new Proxy(Object.freeze({}), {
        get(_capabilityTarget, operationProperty) {
          if (typeof operationProperty !== 'string') return undefined;
          const known = operationCache.get(operationProperty);
          if (known !== undefined) return known;
          const invoke = async (
            rawInput: JsonValue = null,
            callOptions?: JsonValue
          ): Promise<JsonValue> =>
            invokeCapability(
              capabilityProperty,
              operationProperty,
              capabilityProperty === 'http' && operationProperty === 'request'
                ? normalizeHttpRequest(rawInput)
                : rawInput,
              callOptions
            );
          Object.freeze(invoke);
          operationCache.set(operationProperty, invoke);
          return invoke;
        },
        set: () => false,
        defineProperty: () => false,
        deleteProperty: () => false,
      });
      capabilityCache.set(capabilityProperty, capability);
      return capability;
    },
    set: () => false,
    defineProperty: () => false,
    deleteProperty: () => false,
  });
}

async function execute(request: ScriptWorkerRequest): Promise<void> {
  try {
    operationSequences.clear();
    automaticEffectfulCalls.clear();
    const context = deepFreezeJson(request.context);
    const attempt =
      request.attempt === undefined
        ? undefined
        : deepFreezeJson(request.attempt);
    const secrets = deepFreezeJson(request.bindings?.secrets ?? {});
    const services = deeplyReadonlyServiceFacade(request);
    if (request.bindings !== undefined) {
      Object.defineProperty(globalThis, 'fetch', {
        configurable: false,
        enumerable: true,
        writable: false,
        value: trackedNativeFetch(request),
      });
    }
    const safeNodeId = request.nodeId.replace(/[^A-Za-z0-9_-]/g, '_');
    const body = `"use strict";\n${request.source}\n//# sourceURL=woml-step-${safeNodeId}.js`;
    const script =
      request.bindings === undefined
        ? new AsyncFunction('context', 'attempt', body)
        : new AsyncFunction('context', 'attempt', 'services', 'secrets', body);
    const result =
      request.bindings === undefined
        ? await script(context, attempt, undefined, {})
        : await script(context, attempt, services, secrets);
    const violation = findJsonViolation(result);
    if (violation !== undefined) {
      self.postMessage({
        messageType: 'completed',
        response: {
          ok: false,
          error: {
            kind: 'non-json',
            name: 'NonJsonResult',
            message: `${violation.path}: ${violation.reason}`,
          },
        },
      } satisfies ScriptWorkerOutbound);
      return;
    }
    self.postMessage({
      messageType: 'completed',
      response: { ok: true, result: result as JsonValue },
    } satisfies ScriptWorkerOutbound);
  } catch (error) {
    self.postMessage({
      messageType: 'completed',
      response: serializeError(error),
    } satisfies ScriptWorkerOutbound);
  }
}

self.onmessage = (event: MessageEvent<ScriptWorkerInbound>) => {
  const message = event.data;
  if (message.messageType === 'execute') {
    void execute(message.request);
    return;
  }
  if (message.messageType === 'fetch_observation_ack') {
    const pending = pendingFetchAcks.get(message.requestId);
    if (pending === undefined) return;
    pendingFetchAcks.delete(message.requestId);
    if (message.ack.accepted) pending.resolve();
    else
      pending.reject(
        new NativeFetchTrackingError(message.requestId, message.ack.error)
      );
    return;
  }
  const pending = pendingCalls.get(message.callId);
  if (pending === undefined) return;
  pendingCalls.delete(message.callId);
  if (message.result.outcome === 'succeeded') {
    const actualBytes = Buffer.byteLength(
      JSON.stringify(message.result.result),
      'utf8'
    );
    if (actualBytes > 4_194_304 || actualBytes !== message.result.resultBytes) {
      pending.reject(
        new ServiceCallError(
          pending.capability,
          pending.operation,
          message.callId,
          {
            kind: 'invalid_result',
            code: 'WOML_CAPABILITY_RESULT_INVALID',
            message: 'The capability result failed its byte-size contract.',
            retryable: false,
            ambiguous: false,
          }
        )
      );
      return;
    }
    pending.resolve(message.result.result);
  } else {
    pending.reject(
      new ServiceCallError(
        pending.capability,
        pending.operation,
        message.callId,
        message.result.error
      )
    );
  }
};
