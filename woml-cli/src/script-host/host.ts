import { findJsonViolation } from './json';
import { assertExecuteMessage, MessageProtocolError } from './protocol';
import type {
  CompletedMessage,
  ExecuteMessage,
  FailureMessage,
  HostReportedFailure,
  JsonValue,
  ScriptHostLimits,
  SuccessMessage,
} from './types';
import type { ScriptWorkerRequest, ScriptWorkerResponse } from './worker';

export interface ScriptHostOptions {
  readonly workerUrl: URL;
  readonly limits?: ScriptHostLimits;
  readonly send: (message: CompletedMessage) => Promise<void>;
}

type WorkerOutcome =
  | { readonly kind: 'response'; readonly response: ScriptWorkerResponse }
  | { readonly kind: 'timeout' }
  | { readonly kind: 'crashed'; readonly message: string };

function elapsedMilliseconds(startedAt: number): number {
  return Math.max(0, performance.now() - startedAt);
}

function failureMessage(
  request: ExecuteMessage,
  startedAt: number,
  error: HostReportedFailure,
): FailureMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: 1,
    messageType: 'completed',
    invocationId: request.invocationId,
    outcome: { kind: 'failure', error },
    durationMs: elapsedMilliseconds(startedAt),
  };
}

function successMessage(
  request: ExecuteMessage,
  startedAt: number,
  value: JsonValue,
): SuccessMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: 1,
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
  readonly #send: (message: CompletedMessage) => Promise<void>;
  readonly #tasks = new Map<string, Promise<void>>();
  readonly #workers = new Set<Worker>();
  #aborted = false;

  constructor(options: ScriptHostOptions) {
    this.#workerUrl = options.workerUrl;
    this.#limits = options.limits ?? {};
    this.#send = options.send;
  }

  accept(message: unknown): void {
    if (this.#aborted) {
      throw new MessageProtocolError('The script host is shutting down.');
    }
    assertExecuteMessage(message);
    if (this.#tasks.has(message.invocationId)) {
      throw new MessageProtocolError(
        `Invocation ID "${message.invocationId}" is already active.`,
      );
    }

    const task = this.#execute(message).finally(() => {
      this.#tasks.delete(message.invocationId);
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
    for (const worker of this.#workers) worker.terminate();
    this.#workers.clear();
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
        }),
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
        }),
      );
      return;
    }
    if (outcome.kind === 'crashed') {
      await this.#send(
        failureMessage(request, startedAt, {
          kind: 'worker_crashed',
          code: 'WOML_SCRIPT_WORKER_CRASHED',
          message: outcome.message,
        }),
      );
      return;
    }

    const response = outcome.response;
    if (!response.ok) {
      await this.#send(
        failureMessage(request, startedAt, {
          kind:
            response.error.kind === 'non-json'
              ? 'invalid_script_result'
              : 'script_threw',
          code:
            response.error.kind === 'non-json'
              ? 'WOML_SCRIPT_NON_JSON_RESULT'
              : 'WOML_SCRIPT_THROWN',
          message: response.error.message,
        }),
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
        }),
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
        }),
      );
      return;
    }

    await this.#send(
      successMessage(request, startedAt, response.result as JsonValue),
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
    this.#workers.add(worker);

    return new Promise<WorkerOutcome>((resolve) => {
      let settled = false;
      const finish = (outcome: WorkerOutcome) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        this.#workers.delete(worker);
        worker.terminate();
        resolve(outcome);
      };
      const timeout = setTimeout(
        () => finish({ kind: 'timeout' }),
        request.timeoutMs,
      );

      worker.onmessage = (event: MessageEvent<ScriptWorkerResponse>) => {
        finish({ kind: 'response', response: event.data });
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
        worker.postMessage({
          nodeId: request.nodeId,
          source: request.source,
          context: request.context,
        } satisfies ScriptWorkerRequest);
      } catch {
        finish({
          kind: 'crashed',
          message: 'The invocation could not be delivered to its script Worker.',
        });
      }
    });
  }
}
