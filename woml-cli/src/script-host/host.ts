import { findJsonViolation } from './json';
import { assertInboundMessage, MessageProtocolError } from './protocol';
import type {
  CapabilityCallMessage,
  CapabilityResultMessage,
  CancelMessage,
  CompletedMessage,
  ExecuteMessage,
  FailureMessage,
  FetchObservationAckMessage,
  FetchObservationMessage,
  HostReportedFailure,
  JsonValue,
  ModuleRegisteredMessage,
  RegisterModuleMessage,
  ScriptAttempt,
  ScriptHostLimits,
  ScriptHostProtocolVersion,
  SuccessMessage,
} from './types';
import type {
  ScriptWorkerInbound,
  ScriptWorkerOutbound,
  ScriptWorkerRequest,
  ScriptWorkerResponse,
} from './worker';

export interface ScriptHostOptions {
  readonly workerUrl: URL;
  readonly limits?: ScriptHostLimits;
  readonly send: (
    message:
      | CompletedMessage
      | CapabilityCallMessage
      | FetchObservationMessage
      | ModuleRegisteredMessage
  ) => Promise<void>;
  readonly protocolVersion?: ScriptHostProtocolVersion;
}

type WorkerOutcome =
  | { readonly kind: 'response'; readonly response: ScriptWorkerResponse }
  | { readonly kind: 'timeout' }
  | { readonly kind: 'cancelled'; readonly reason: CancelMessage['reason'] }
  | { readonly kind: 'crashed'; readonly message: string };

const MAX_MODULE_ARTIFACT_BYTES = 3 * 1024 * 1024;
const MAX_MODULE_CACHE_BYTES = 32 * 1024 * 1024;
const MAX_MODULE_CACHE_ENTRIES = 64;

interface CachedModuleArtifact {
  readonly bundle: string;
  readonly sourceMap?: string;
  readonly sourceMapDigest?: string;
  readonly bytes: number;
}

function moduleRegisteredMessage(
  request: RegisterModuleMessage,
  outcome:
    | { readonly accepted: true }
    | {
        readonly accepted: false;
        readonly code:
          | 'WOML_MODULE_DIGEST_MISMATCH'
          | 'WOML_MODULE_CACHE_LIMIT_EXCEEDED';
        readonly message: string;
      }
): ModuleRegisteredMessage {
  const base = {
    protocol: 'woml.script-host' as const,
    messageType: 'module_registered' as const,
    bundleDigest: request.bundleDigest,
    ...outcome,
  };
  return request.protocolVersion === 6 ||
    request.protocolVersion === 7 ||
    request.protocolVersion === 8
    ? {
        ...base,
        protocolVersion: request.protocolVersion,
        sourceMapDigest: request.sourceMapDigest,
      }
    : { ...base, protocolVersion: 5 };
}

function elapsedMilliseconds(startedAt: number): number {
  return Math.max(0, performance.now() - startedAt);
}

function failureMessage(
  request: ExecuteMessage,
  startedAt: number,
  error: HostReportedFailure
): FailureMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: request.protocolVersion,
    messageType: 'completed',
    invocationId: request.invocationId,
    outcome: { kind: 'failure', error },
    durationMs: elapsedMilliseconds(startedAt),
  };
}

function successMessage(
  request: ExecuteMessage,
  startedAt: number,
  value: JsonValue
): SuccessMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: request.protocolVersion,
    messageType: 'completed',
    invocationId: request.invocationId,
    outcome: { kind: 'success', value },
    durationMs: elapsedMilliseconds(startedAt),
  };
}

function byteLength(value: unknown): number {
  return Buffer.byteLength(JSON.stringify(value), 'utf8');
}

export class ScriptHost {
  readonly #workerUrl: URL;
  readonly #limits: ScriptHostLimits;
  readonly #send: (
    message:
      | CompletedMessage
      | CapabilityCallMessage
      | FetchObservationMessage
      | ModuleRegisteredMessage
  ) => Promise<void>;
  readonly #protocolVersion: ScriptHostProtocolVersion;
  readonly #tasks = new Map<string, Promise<void>>();
  readonly #workers = new Map<string, Worker>();
  readonly #cancellations = new Map<
    string,
    (reason: CancelMessage['reason']) => void
  >();
  readonly #pendingCalls = new Map<string, Set<string>>();
  readonly #closedCalls = new Set<string>();
  readonly #pendingFetchAcks = new Map<
    string,
    Map<string, 'started' | 'terminal'>
  >();
  readonly #activeFetches = new Map<string, Set<string>>();
  readonly #closedFetchAcks = new Set<string>();
  readonly #moduleBundles = new Map<string, CachedModuleArtifact>();
  #moduleCacheBytes = 0;
  #aborted = false;

  constructor(options: ScriptHostOptions) {
    this.#workerUrl = options.workerUrl;
    this.#limits = options.limits ?? {};
    this.#send = options.send;
    this.#protocolVersion = options.protocolVersion ?? 3;
  }

  accept(message: unknown): void {
    if (this.#aborted) {
      throw new MessageProtocolError('The script host is shutting down.');
    }
    assertInboundMessage(message, this.#protocolVersion);
    if (message.messageType === 'register_module') {
      this.#registerModule(message);
      return;
    }
    if (message.messageType === 'cancel') {
      this.#cancel(message);
      return;
    }
    if (message.messageType === 'capability_result') {
      this.#capabilityResult(message);
      return;
    }
    if (message.messageType === 'fetch_observation_ack') {
      this.#fetchObservationAck(message);
      return;
    }
    if (this.#tasks.has(message.invocationId)) {
      throw new MessageProtocolError(
        `Invocation ID "${message.invocationId}" is already active.`
      );
    }

    const task = this.#execute(message).finally(() => {
      this.#tasks.delete(message.invocationId);
      this.#cancellations.delete(message.invocationId);
    });
    this.#tasks.set(message.invocationId, task);
  }

  #registerModule(message: RegisterModuleMessage): void {
    const actualDigest = `sha256:${new Bun.CryptoHasher('sha256')
      .update(message.bundle)
      .digest('hex')}`;
    const sourceMapDigest =
      message.protocolVersion === 6 ||
      message.protocolVersion === 7 ||
      message.protocolVersion === 8
        ? `sha256:${new Bun.CryptoHasher('sha256')
            .update(message.sourceMap)
            .digest('hex')}`
        : undefined;
    const digestMismatch =
      actualDigest !== message.bundleDigest ||
      ((message.protocolVersion === 6 ||
        message.protocolVersion === 7 ||
        message.protocolVersion === 8) &&
        sourceMapDigest !== message.sourceMapDigest);
    if (digestMismatch) {
      void this.#send(
        moduleRegisteredMessage(message, {
          accepted: false,
          code: 'WOML_MODULE_DIGEST_MISMATCH',
          message:
            'The registered module bundle does not match its SHA-256 identity.',
        })
      );
      return;
    }
    const existing = this.#moduleBundles.get(message.bundleDigest);
    if (
      existing !== undefined &&
      (existing.bundle !== message.bundle ||
        ((message.protocolVersion === 6 ||
          message.protocolVersion === 7 ||
          message.protocolVersion === 8) &&
          (existing.sourceMap !== message.sourceMap ||
            existing.sourceMapDigest !== message.sourceMapDigest)))
    ) {
      throw new MessageProtocolError(
        'One module digest was registered with different immutable bytes.'
      );
    }
    const bytes =
      Buffer.byteLength(message.bundle, 'utf8') +
      (message.protocolVersion === 6 ||
      message.protocolVersion === 7 ||
      message.protocolVersion === 8
        ? Buffer.byteLength(message.sourceMap, 'utf8')
        : 0);
    const individualLimitExceeded =
      Buffer.byteLength(message.bundle, 'utf8') > MAX_MODULE_ARTIFACT_BYTES ||
      ((message.protocolVersion === 6 ||
        message.protocolVersion === 7 ||
        message.protocolVersion === 8) &&
        Buffer.byteLength(message.sourceMap, 'utf8') >
          MAX_MODULE_ARTIFACT_BYTES);
    if (
      existing === undefined &&
      (individualLimitExceeded ||
        this.#moduleBundles.size >= MAX_MODULE_CACHE_ENTRIES ||
        this.#moduleCacheBytes + bytes > MAX_MODULE_CACHE_BYTES)
    ) {
      void this.#send(
        moduleRegisteredMessage(message, {
          accepted: false,
          code: 'WOML_MODULE_CACHE_LIMIT_EXCEEDED',
          message: 'The immutable module cache limit was exceeded.',
        })
      );
      return;
    }
    if (existing === undefined) {
      this.#moduleBundles.set(message.bundleDigest, {
        bundle: message.bundle,
        ...(message.protocolVersion === 6 ||
        message.protocolVersion === 7 ||
        message.protocolVersion === 8
          ? {
              sourceMap: message.sourceMap,
              sourceMapDigest: message.sourceMapDigest,
            }
          : {}),
        bytes,
      });
      this.#moduleCacheBytes += bytes;
    }
    void this.#send(moduleRegisteredMessage(message, { accepted: true }));
  }

  async drain(): Promise<void> {
    while (this.#tasks.size > 0) {
      await Promise.all([...this.#tasks.values()]);
    }
  }

  abort(): void {
    this.#aborted = true;
    for (const worker of this.#workers.values()) worker.terminate();
    this.#workers.clear();
    this.#cancellations.clear();
    this.#pendingCalls.clear();
    this.#closedCalls.clear();
    this.#pendingFetchAcks.clear();
    this.#activeFetches.clear();
    this.#closedFetchAcks.clear();
    this.#moduleBundles.clear();
    this.#moduleCacheBytes = 0;
  }

  #cancel(message: CancelMessage): void {
    this.#cancellations.get(message.invocationId)?.(message.reason);
  }

  #capabilityResult(message: CapabilityResultMessage): void {
    if (
      message.invocationId !== message.result.invocationId ||
      message.callId !== message.result.callId
    ) {
      throw new MessageProtocolError(
        'Capability result wrapper IDs do not match its result payload.'
      );
    }
    const worker = this.#workers.get(message.invocationId);
    const calls = this.#pendingCalls.get(message.invocationId);
    if (
      worker === undefined ||
      calls === undefined ||
      !calls.delete(message.callId)
    ) {
      const key = `${message.invocationId}\0${message.callId}`;
      if (this.#closedCalls.delete(key)) return;
      throw new MessageProtocolError(
        `Capability result references unknown call ID "${message.callId}".`
      );
    }
    worker.postMessage({
      messageType: 'capability_result',
      callId: message.callId,
      result: message.result,
    } satisfies ScriptWorkerInbound);
  }

  #fetchObservationAck(message: FetchObservationAckMessage): void {
    const worker = this.#workers.get(message.invocationId);
    const pending = this.#pendingFetchAcks.get(message.invocationId);
    const phase = pending?.get(message.requestId);
    if (worker === undefined || pending === undefined || phase === undefined) {
      const key = `${message.invocationId}\0${message.requestId}`;
      if (this.#closedFetchAcks.delete(key)) return;
      throw new MessageProtocolError(
        `Native Fetch acknowledgement references unknown request ID "${message.requestId}".`
      );
    }
    pending.delete(message.requestId);
    const active = this.#activeFetches.get(message.invocationId)!;
    if (message.accepted && phase === 'started') active.add(message.requestId);
    if (!message.accepted || phase === 'terminal') {
      active.delete(message.requestId);
    }
    worker.postMessage({
      messageType: 'fetch_observation_ack',
      requestId: message.requestId,
      ack: message,
    } satisfies ScriptWorkerInbound);
  }

  async #execute(request: ExecuteMessage): Promise<void> {
    const startedAt = performance.now();
    const contextBytes = byteLength(request.context);
    if (
      this.#limits.maxContextBytes !== undefined &&
      contextBytes > this.#limits.maxContextBytes
    ) {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'context_too_large',
          code: 'WOML_SCRIPT_CONTEXT_TOO_LARGE',
          message: 'Invocation context exceeds the configured byte limit.',
          details: {
            actualBytes: contextBytes,
            limitBytes: this.#limits.maxContextBytes,
          },
        })
      );
      return;
    }

    const outcome = await this.#invokeWorker(request);
    if (outcome.kind === 'timeout') {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'script_timed_out',
          code: 'WOML_SCRIPT_TIMEOUT',
          message: 'Script exceeded its execution deadline.',
        })
      );
      return;
    }
    if (outcome.kind === 'cancelled') {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'invocation_cancelled',
          code: 'WOML_SCRIPT_CANCELLED',
          message:
            outcome.reason === 'parallel_fail_fast'
              ? 'Invocation was cancelled by parallel fail-fast.'
              : outcome.reason === 'run_cancelled'
                ? 'Invocation was cancelled because its run was cancelled.'
                : outcome.reason === 'step_timed_out'
                  ? 'Invocation was cancelled because its step timed out.'
                  : 'Invocation was cancelled because the script host is shutting down.',
        })
      );
      return;
    }
    if (outcome.kind === 'crashed') {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'worker_crashed',
          code: 'WOML_SCRIPT_WORKER_CRASHED',
          message: outcome.message,
        })
      );
      return;
    }

    const response = outcome.response;
    if (!response.ok) {
      await this.#send(
        failureMessage(request, startedAt, {
          ...(response.error.kind === 'service' &&
          response.error.capability !== undefined &&
          response.error.operation !== undefined &&
          response.error.callId !== undefined &&
          response.error.cause !== undefined
            ? {
                kind: 'service_failed' as const,
                code: response.error.cause.code as never,
                message: response.error.message,
                capability: response.error.capability,
                operation: response.error.operation,
                callId: response.error.callId,
                retryable: response.error.cause.retryable,
                ambiguous: response.error.cause.ambiguous,
                cause: response.error.cause,
              }
            : {
                kind:
                  response.error.kind === 'non-json'
                    ? ('invalid_script_result' as const)
                    : ('script_threw' as const),
                code:
                  response.error.kind === 'non-json'
                    ? ('WOML_SCRIPT_NON_JSON_RESULT' as const)
                    : ('WOML_SCRIPT_THROWN' as const),
                message:
                  response.error.moduleFrame === undefined
                    ? response.error.message
                    : `${response.error.message} (${response.error.moduleFrame})`,
              }),
        })
      );
      return;
    }

    const violation = findJsonViolation(response.result);
    if (violation !== undefined) {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'invalid_script_result',
          code: 'WOML_SCRIPT_NON_JSON_RESULT',
          message: `${violation.path}: ${violation.reason}`,
        })
      );
      return;
    }

    if (
      (request.protocolVersion === 4 ||
        request.protocolVersion === 5 ||
        request.protocolVersion === 6 ||
        request.protocolVersion === 7 ||
        request.protocolVersion === 8) &&
      containsKnownSecret(
        response.result,
        Object.values(request.bindings.secrets)
      )
    ) {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'invalid_script_result',
          code: 'WOML_SCRIPT_NON_JSON_RESULT',
          message: 'Script results must not contain a resolved secret value.',
        })
      );
      return;
    }

    const resultBytes = byteLength(response.result);
    if (
      this.#limits.maxResultBytes !== undefined &&
      resultBytes > this.#limits.maxResultBytes
    ) {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'result_too_large',
          code: 'WOML_SCRIPT_RESULT_TOO_LARGE',
          message: 'Script result exceeds the configured byte limit.',
          details: {
            actualBytes: resultBytes,
            limitBytes: this.#limits.maxResultBytes,
          },
        })
      );
      return;
    }

    await this.#send(
      successMessage(request, startedAt, response.result as JsonValue)
    );
  }

  #invokeWorker(request: ExecuteMessage): Promise<WorkerOutcome> {
    let worker: Worker;
    try {
      worker = new Worker(this.#workerUrl, {
        type: 'module',
        ref: true,
        smol: true,
        env: {},
      });
    } catch {
      return Promise.resolve({
        kind: 'crashed',
        message: 'The isolated script Worker could not be started.',
      });
    }
    this.#workers.set(request.invocationId, worker);

    return new Promise<WorkerOutcome>(resolve => {
      let settled = false;
      const finish = (outcome: WorkerOutcome) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        this.#workers.delete(request.invocationId);
        this.#cancellations.delete(request.invocationId);
        for (const callId of this.#pendingCalls.get(request.invocationId) ??
          []) {
          this.#closedCalls.add(`${request.invocationId}\0${callId}`);
        }
        for (const requestId of this.#pendingFetchAcks
          .get(request.invocationId)
          ?.keys() ?? []) {
          this.#closedFetchAcks.add(`${request.invocationId}\0${requestId}`);
        }
        while (this.#closedCalls.size > 4_096) {
          const oldest = this.#closedCalls.values().next().value;
          if (oldest === undefined) break;
          this.#closedCalls.delete(oldest);
        }
        while (this.#closedFetchAcks.size > 4_096) {
          const oldest = this.#closedFetchAcks.values().next().value;
          if (oldest === undefined) break;
          this.#closedFetchAcks.delete(oldest);
        }
        this.#pendingCalls.delete(request.invocationId);
        this.#pendingFetchAcks.delete(request.invocationId);
        this.#activeFetches.delete(request.invocationId);
        worker.terminate();
        resolve(outcome);
      };
      const timeout = setTimeout(
        () => finish({ kind: 'timeout' }),
        request.timeoutMs
      );
      this.#cancellations.set(request.invocationId, reason => {
        finish({ kind: 'cancelled', reason });
      });

      this.#pendingCalls.set(request.invocationId, new Set());
      this.#pendingFetchAcks.set(request.invocationId, new Map());
      this.#activeFetches.set(request.invocationId, new Set());
      worker.onmessage = (event: MessageEvent<ScriptWorkerOutbound>) => {
        const message = event.data;
        if (message.messageType === 'completed') {
          if (
            (this.#pendingFetchAcks.get(request.invocationId)?.size ?? 0) > 0 ||
            (this.#activeFetches.get(request.invocationId)?.size ?? 0) > 0
          ) {
            finish({
              kind: 'crashed',
              message:
                'The isolated script Worker completed with an unclosed native Fetch observation.',
            });
            return;
          }
          finish({ kind: 'response', response: message.response });
          return;
        }
        if (message.messageType === 'fetch_observation') {
          const observation = message.observation;
          if (
            (request.protocolVersion !== 4 &&
              request.protocolVersion !== 5 &&
              request.protocolVersion !== 6 &&
              request.protocolVersion !== 7 &&
              request.protocolVersion !== 8) ||
            observation.invocationId !== request.invocationId
          ) {
            finish({
              kind: 'crashed',
              message:
                'The isolated script Worker emitted an invalid native Fetch observation.',
            });
            return;
          }
          const pending = this.#pendingFetchAcks.get(request.invocationId)!;
          const active = this.#activeFetches.get(request.invocationId)!;
          const phase =
            observation.observationType === 'started' ? 'started' : 'terminal';
          const invalid =
            pending.has(observation.requestId) ||
            (phase === 'started'
              ? active.has(observation.requestId)
              : !active.has(observation.requestId));
          if (invalid) {
            finish({
              kind: 'crashed',
              message:
                'The isolated script Worker violated the native Fetch observation lifecycle.',
            });
            return;
          }
          pending.set(observation.requestId, phase);
          void this.#send({
            protocol: 'woml.script-host',
            protocolVersion: request.protocolVersion,
            messageType: 'fetch_observation',
            invocationId: request.invocationId,
            requestId: observation.requestId,
            observation,
          }).catch(() => {
            finish({
              kind: 'crashed',
              message:
                'The native Fetch observation could not be sent to Rust.',
            });
          });
          return;
        }
        const call = message.call;
        if (
          (request.protocolVersion !== 4 &&
            request.protocolVersion !== 5 &&
            request.protocolVersion !== 6 &&
            request.protocolVersion !== 7 &&
            request.protocolVersion !== 8) ||
          call.invocationId !== request.invocationId ||
          call.runId !== request.runId ||
          call.nodeId !== request.nodeId ||
          call.attemptNumber !== request.attempt.number
        ) {
          finish({
            kind: 'crashed',
            message:
              'The isolated script Worker emitted an invalid capability call.',
          });
          return;
        }
        const calls = this.#pendingCalls.get(request.invocationId);
        if (calls === undefined || calls.has(call.callId)) {
          finish({
            kind: 'crashed',
            message: 'The isolated script Worker reused a capability call ID.',
          });
          return;
        }
        calls.add(call.callId);
        void this.#send({
          protocol: 'woml.script-host',
          protocolVersion: request.protocolVersion,
          messageType: 'capability_call',
          invocationId: request.invocationId,
          callId: call.callId,
          call,
        }).catch(() => {
          finish({
            kind: 'crashed',
            message: 'The capability call could not be sent to Rust.',
          });
        });
      };
      worker.onerror = (event: ErrorEvent) => {
        event.preventDefault();
        finish({
          kind: 'crashed',
          message: event.message || 'The isolated script Worker crashed.',
        });
      };
      worker.onmessageerror = () => {
        finish({
          kind: 'crashed',
          message: 'The isolated script Worker returned an unreadable message.',
        });
      };
      worker.addEventListener('close', () => {
        finish({
          kind: 'crashed',
          message: 'The isolated script Worker exited without a result.',
        });
      });

      try {
        const attempt: ScriptAttempt | undefined =
          request.protocolVersion === 3 ||
          request.protocolVersion === 4 ||
          request.protocolVersion === 5 ||
          request.protocolVersion === 6 ||
          request.protocolVersion === 7 ||
          request.protocolVersion === 8
            ? request.attempt
            : undefined;
        worker.postMessage({
          messageType: 'execute',
          request: {
            invocationId: request.invocationId,
            runId: request.runId,
            nodeId: request.nodeId,
            timeoutMs: request.timeoutMs,
            source: request.source,
            context: request.context,
            ...(request.protocolVersion === 7 || request.protocolVersion === 8
              ? {
                  mode: request.mode,
                  ...(request.mode === 'lifecycle'
                    ? { lifecycle: request.lifecycle }
                    : {}),
                }
              : {}),
            ...(request.protocolVersion === 8 && request.reusable !== undefined
              ? {
                  reusable: request.reusable,
                  ...(request.reusableLifecycle === undefined
                    ? {}
                    : { reusableLifecycle: request.reusableLifecycle }),
                }
              : {}),
            attempt,
            ...(request.protocolVersion === 4 ||
            request.protocolVersion === 5 ||
            request.protocolVersion === 6 ||
            request.protocolVersion === 7 ||
            request.protocolVersion === 8
              ? { bindings: request.bindings }
              : {}),
            ...(request.protocolVersion === 5 ||
            request.protocolVersion === 6 ||
            request.protocolVersion === 7 ||
            request.protocolVersion === 8
              ? {
                  modules: request.modules.map(module => {
                    const artifact = this.#moduleBundles.get(
                      module.bundleDigest
                    );
                    if (artifact === undefined) {
                      throw new MessageProtocolError(
                        `Module bundle ${module.bundleDigest} was not registered.`
                      );
                    }
                    return {
                      ...module,
                      bundle: artifact.bundle,
                      ...(request.protocolVersion === 6 ||
                      request.protocolVersion === 7 ||
                      request.protocolVersion === 8
                        ? {
                            sourceMap: artifact.sourceMap,
                            sourceMapDigest: artifact.sourceMapDigest,
                          }
                        : {}),
                    };
                  }),
                }
              : {}),
          } satisfies ScriptWorkerRequest,
        } satisfies ScriptWorkerInbound);
      } catch {
        finish({
          kind: 'crashed',
          message:
            'The invocation could not be delivered to its script Worker.',
        });
      }
    });
  }
}

function containsKnownSecret(
  value: unknown,
  secrets: readonly string[]
): boolean {
  const known = secrets.filter(secret => secret.length > 0);
  if (known.length === 0) return false;
  if (typeof value === 'string') {
    return known.some(secret => value.includes(secret));
  }
  if (Array.isArray(value)) {
    return value.some(item => containsKnownSecret(item, known));
  }
  if (value !== null && typeof value === 'object') {
    return Object.values(value).some(item => containsKnownSecret(item, known));
  }
  return false;
}
