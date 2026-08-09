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
  const normalized: Record<string, JsonValue> = {
    contract: 'woml.managed-http',
    contractVersion: 1,
    kind: 'request',
    method,
    url: object.url,
    headers,
    responseType: object.responseType ?? 'json',
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

function deeplyReadonlyServiceFacade(request: ScriptWorkerRequest): unknown {
  if (request.attempt === undefined || request.bindings === undefined) {
    return Object.freeze({});
  }
  const capabilityCache = new Map<string, unknown>();
  return new Proxy(Object.freeze({}), {
    get(_target, capabilityProperty) {
      if (typeof capabilityProperty !== 'string') return undefined;
      const cached = capabilityCache.get(capabilityProperty);
      if (cached !== undefined) return cached;
      const operationCache = new Map<string, unknown>();
      const capability = new Proxy(Object.freeze({}), {
        get(_capabilityTarget, operationProperty) {
          if (typeof operationProperty !== 'string') return undefined;
          const known = operationCache.get(operationProperty);
          if (known !== undefined) return known;
          const invoke = async (
            rawInput: JsonValue = null,
            callOptions?: JsonValue
          ): Promise<JsonValue> => {
            const managedHttp =
              capabilityProperty === 'http' && operationProperty === 'request';
            if (!managedHttp && callOptions !== undefined) {
              throw new TypeError(
                'Named service-call options are currently supported by services.http.request() only.'
              );
            }
            const input = managedHttp
              ? normalizeHttpRequest(rawInput)
              : rawInput;
            const violation = findJsonViolation(input);
            if (violation !== undefined) {
              throw new TypeError(`${violation.path}: ${violation.reason}`);
            }
            const inputBytes = Buffer.byteLength(JSON.stringify(input), 'utf8');
            if (inputBytes > 1_048_576) {
              const id = callId();
              throw new ServiceCallError(
                capabilityProperty,
                operationProperty,
                id,
                {
                  kind: 'input_too_large',
                  code: 'WOML_CAPABILITY_INPUT_TOO_LARGE',
                  message:
                    'The capability input exceeds its configured byte limit.',
                  retryable: false,
                  ambiguous: false,
                  details: { actualBytes: inputBytes, limitBytes: 1_048_576 },
                }
              );
            }
            const identity = namedOperation(
              capabilityProperty,
              operationProperty,
              callOptions
            );
            const method = managedHttp
              ? String((input as JsonObject).method)
              : undefined;
            const effectful =
              managedHttp &&
              method !== 'GET' &&
              method !== 'HEAD' &&
              method !== 'OPTIONS';
            if (effectful && identity.mode === 'automatic') {
              const key = `${capabilityProperty}.${operationProperty}`;
              const count = (automaticEffectfulCalls.get(key) ?? 0) + 1;
              automaticEffectfulCalls.set(key, count);
              if (count > 1) {
                throw new TypeError(
                  'Multiple effectful services.http.request() calls in one step require stable names, for example { name: "create-order" }.'
                );
              }
            }
            const operationName = identity.name;
            const httpIdempotency = managedHttp
              ? plainObject((input as JsonObject).idempotency)
              : undefined;
            const providerIdempotencyKey =
              typeof httpIdempotency?.value === 'string'
                ? httpIdempotency.value
                : undefined;
            const timeoutMs = managedHttp
              ? Math.min(
                  86_400_000,
                  Number((input as JsonObject).timeoutMs) + 1_000
                )
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
              attemptNumber: request.attempt!.number,
              capability: capabilityProperty,
              operation: operationProperty,
              inputContractVersion: 1,
              resultContractVersion: 1,
              identity: {
                mode: identity.mode,
                stepIdempotencyKey: request.attempt!.idempotencyKey,
                operationName,
                operationKey: await operationKey(
                  request.attempt!.idempotencyKey,
                  operationName
                ),
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
              pendingCalls.set(id, {
                capability: capabilityProperty,
                operation: operationProperty,
                resolve,
                reject,
              });
              self.postMessage({
                messageType: 'capability_call',
                call,
              } satisfies ScriptWorkerOutbound);
            });
            return managedHttp ? publicHttpResult(result) : result;
          };
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
