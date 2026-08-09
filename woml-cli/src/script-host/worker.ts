import { deepFreezeJson, findJsonViolation } from './json';
import type {
  CapabilityCallRequest,
  CapabilityCallResult,
  CapabilityFailure,
  JsonValue,
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
    };

export type ScriptWorkerOutbound =
  | {
      readonly messageType: 'completed';
      readonly response: ScriptWorkerResponse;
    }
  | {
      readonly messageType: 'capability_call';
      readonly call: CapabilityCallRequest;
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
  readonly capability: string;
  readonly operation: string;
  readonly callId: string;
  readonly cause: CapabilityFailure;

  constructor(
    capability: string,
    operation: string,
    callId: string,
    cause: CapabilityFailure
  ) {
    super(cause.message);
    this.name = 'WomlServiceError';
    this.capability = capability;
    this.operation = operation;
    this.callId = callId;
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
let callSequence = 0;

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
            input: JsonValue = null
          ): Promise<JsonValue> => {
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
            const sequence = ++callSequence;
            const operationName =
              sequence === 1
                ? `${capabilityProperty}.${operationProperty}`
                : `${capabilityProperty}.${operationProperty}.${sequence}`;
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
                mode: 'automatic',
                stepIdempotencyKey: request.attempt!.idempotencyKey,
                operationName,
                operationKey: await operationKey(
                  request.attempt!.idempotencyKey,
                  operationName
                ),
              },
              limits: {
                inputBytes: 1_048_576,
                resultBytes: 4_194_304,
                timeoutMs: 30_000,
              },
              input,
            };
            return await new Promise<JsonValue>((resolve, reject) => {
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
    const context = deepFreezeJson(request.context);
    const attempt =
      request.attempt === undefined
        ? undefined
        : deepFreezeJson(request.attempt);
    const secrets = deepFreezeJson(request.bindings?.secrets ?? {});
    const services = deeplyReadonlyServiceFacade(request);
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
