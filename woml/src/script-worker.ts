import { deepFreezeJson, findJsonViolation } from './json';
import type { JsonObject, JsonValue } from './model';

interface ScriptWorkerContext extends JsonObject {
  readonly trigger: JsonObject;
  readonly steps: Readonly<Record<string, JsonValue>>;
}

export interface ScriptWorkerRequest {
  readonly nodeId: string;
  readonly source: string;
  readonly context: ScriptWorkerContext;
}

export type ScriptWorkerResponse =
  | { readonly ok: true; readonly result: JsonValue }
  | {
      readonly ok: false;
      readonly error: {
        readonly kind: 'script' | 'non-json';
        readonly name: string;
        readonly message: string;
        readonly stack?: string;
      };
    };

type AsyncFunction = (
  context: ScriptWorkerContext,
) => Promise<unknown>;
type AsyncFunctionConstructor = new (
  ...parametersAndBody: string[]
) => AsyncFunction;

const AsyncFunction = Object.getPrototypeOf(
  async function emptyAsyncFunction() {},
).constructor as AsyncFunctionConstructor;

function serializeError(error: unknown): ScriptWorkerResponse {
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

self.onmessage = async (event: MessageEvent<ScriptWorkerRequest>) => {
  const request = event.data;
  try {
    const context = deepFreezeJson(request.context);
    const safeNodeId = request.nodeId.replace(/[^A-Za-z0-9_-]/g, '_');
    const script = new AsyncFunction(
      'context',
      `"use strict";\n${request.source}\n//# sourceURL=woml-step-${safeNodeId}.js`,
    );
    const result = await script(context);
    const violation = findJsonViolation(result);
    if (violation !== undefined) {
      self.postMessage({
        ok: false,
        error: {
          kind: 'non-json',
          name: 'NonJsonResult',
          message: `${violation.path}: ${violation.reason}`,
        },
      } satisfies ScriptWorkerResponse);
      return;
    }
    self.postMessage({ ok: true, result: result as JsonValue } satisfies ScriptWorkerResponse);
  } catch (error) {
    self.postMessage(serializeError(error));
  }
};
