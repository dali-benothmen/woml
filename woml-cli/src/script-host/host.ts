import { findJsonViolation } from './json';
import { assertInboundMessage, MessageProtocolError } from './protocol';
import type {
  CapabilityCallMessage,
  CapabilityResultMessage,
  CancelMessage,
  CompletedMessage,
  ExecuteMessage,
  FailureMessage,
  HostReportedFailure,
  JsonValue,
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
    message: CompletedMessage | CapabilityCallMessage
  ) => Promise<void>;
  readonly protocolVersion?: ScriptHostProtocolVersion;
}

type WorkerOutcome =
  | { readonly kind: 'response'; readonly response: ScriptWorkerResponse }
  | { readonly kind: 'timeout' }
  | { readonly kind: 'cancelled'; readonly reason: CancelMessage['reason'] }
  | { readonly kind: 'crashed'; readonly message: string };

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
    message: CompletedMessage | CapabilityCallMessage
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
    if (message.messageType === 'cancel') {
      this.#cancel(message);
      return;
    }
    if (message.messageType === 'capability_result') {
      this.#capabilityResult(message);
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
                message: response.error.message,
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
      request.protocolVersion === 4 &&
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
        while (this.#closedCalls.size > 4_096) {
          const oldest = this.#closedCalls.values().next().value;
          if (oldest === undefined) break;
          this.#closedCalls.delete(oldest);
        }
        this.#pendingCalls.delete(request.invocationId);
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
      worker.onmessage = (event: MessageEvent<ScriptWorkerOutbound>) => {
        const message = event.data;
        if (message.messageType === 'completed') {
          finish({ kind: 'response', response: message.response });
          return;
        }
        const call = message.call;
        if (
          request.protocolVersion !== 4 ||
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
          protocolVersion: 4,
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
          request.protocolVersion === 3 || request.protocolVersion === 4
            ? request.attempt
            : undefined;
        worker.postMessage({
          messageType: 'execute',
          request: {
            invocationId: request.invocationId,
            runId: request.runId,
            nodeId: request.nodeId,
            source: request.source,
            context: request.context,
            attempt,
            ...(request.protocolVersion === 4
              ? { bindings: request.bindings }
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
