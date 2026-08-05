import { findJsonViolation } from './json';
import type { JsonValue } from './model';
import { WorkflowExecutionError } from './runtime-error';
import type {
  ScriptWorkerRequest,
  ScriptWorkerResponse,
} from './script-worker';

import type { WorkflowContext } from './executor';

export interface RunScriptRequest {
  readonly nodeId: string;
  readonly source: string;
  readonly context: WorkflowContext;
}

export type ScriptRunner = (request: RunScriptRequest) => Promise<JsonValue>;

export const runScriptInWorker: ScriptRunner = async (
  request,
): Promise<JsonValue> => {
  const worker = new Worker(new URL('./script-worker.ts', import.meta.url), {
    type: 'module',
  });

  return await new Promise<JsonValue>((resolve, reject) => {
    let settled = false;
    const finish = (callback: () => void) => {
      if (settled) return;
      settled = true;
      void worker.terminate();
      callback();
    };

    worker.onmessage = (event: MessageEvent<ScriptWorkerResponse>) => {
      const response = event.data;
      if (response.ok) {
        const violation = findJsonViolation(response.result);
        if (violation !== undefined) {
          finish(() =>
            reject(
              new WorkflowExecutionError(
                'WOML_NON_JSON_RESULT',
                `Step "${request.nodeId}" returned a non-JSON value at ${violation.path}: ${violation.reason}.`,
                { nodeId: request.nodeId },
              ),
            ),
          );
          return;
        }
        finish(() => resolve(response.result));
        return;
      }

      const code =
        response.error.kind === 'non-json'
          ? 'WOML_NON_JSON_RESULT'
          : 'WOML_SCRIPT_FAILED';
      finish(() =>
        reject(
          new WorkflowExecutionError(
            code,
            `Step "${request.nodeId}" ${
              response.error.kind === 'non-json' ? 'returned a non-JSON value' : 'script failed'
            }: ${response.error.message}`,
            {
              nodeId: request.nodeId,
              remoteStack: response.error.stack,
            },
          ),
        ),
      );
    };

    worker.onerror = (event: ErrorEvent) => {
      event.preventDefault();
      finish(() =>
        reject(
          new WorkflowExecutionError(
            'WOML_SCRIPT_FAILED',
            `Step "${request.nodeId}" worker failed: ${event.message}`,
            { nodeId: request.nodeId, cause: event.error },
          ),
        ),
      );
    };

    worker.postMessage({
      nodeId: request.nodeId,
      source: request.source,
      context: request.context,
    } satisfies ScriptWorkerRequest);
  });
};
