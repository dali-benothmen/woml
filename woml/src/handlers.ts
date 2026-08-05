import { cloneJson, findJsonViolation, isJsonObject } from './json';
import type {
  CompiledWorkflowNode,
  JsonValue,
  ValueExpression,
} from './model';
import { WorkflowExecutionError } from './runtime-error';
import {
  runScriptInWorker,
  type ScriptRunner,
} from './script-runner';

import type { WorkflowContext } from './executor';

export interface HandlerInvocation {
  readonly node: CompiledWorkflowNode;
  readonly inputs: JsonValue;
  readonly context: WorkflowContext;
}

export type WorkflowHandler = (
  invocation: HandlerInvocation,
) => Promise<JsonValue>;

export class HandlerRegistry {
  readonly #handlers = new Map<string, WorkflowHandler>();

  register(handlerId: string, handler: WorkflowHandler): void {
    if (this.#handlers.has(handlerId)) {
      throw new Error(`Handler "${handlerId}" is already registered.`);
    }
    this.#handlers.set(handlerId, handler);
  }

  get(handlerId: string): WorkflowHandler | undefined {
    return this.#handlers.get(handlerId);
  }

  get size(): number {
    return this.#handlers.size;
  }

  get handlerIds(): readonly string[] {
    return [...this.#handlers.keys()];
  }
}

export function resolveExecutableInput(
  expression: ValueExpression,
  nodeId: string,
): JsonValue {
  if (expression.kind === 'literal') {
    const violation = findJsonViolation(expression.value);
    if (violation !== undefined) {
      throw new WorkflowExecutionError(
        'WOML_INVALID_COMPILED_MODEL',
        `Node "${nodeId}" contains a non-JSON literal at ${violation.path}: ${violation.reason}.`,
        { nodeId },
      );
    }
    return cloneJson(expression.value);
  }
  if (expression.kind === 'object') {
    const fields: Record<string, JsonValue> = Object.create(null) as Record<
      string,
      JsonValue
    >;
    for (const [name, value] of Object.entries(expression.fields)) {
      fields[name] = resolveExecutableInput(value, nodeId);
    }
    return fields;
  }
  throw new WorkflowExecutionError(
    'WOML_UNSUPPORTED_INPUT_EXPRESSION',
    `Node "${nodeId}" uses input expression "${expression.kind}", which is not executable in this runtime profile.`,
    { nodeId },
  );
}

export function createRuntimeHandlerRegistry(
  scriptRunner: ScriptRunner = runScriptInWorker,
): HandlerRegistry {
  const registry = new HandlerRegistry();
  registry.register('runtime.script', async ({ node, inputs, context }) => {
    if (!isJsonObject(inputs)) {
      throw new WorkflowExecutionError(
        'WOML_INVALID_HANDLER_INPUT',
        `runtime.script input for node "${node.id}" must be an object.`,
        { nodeId: node.id },
      );
    }
    const keys = Object.keys(inputs);
    if (
      keys.length !== 1 ||
      keys[0] !== 'source' ||
      typeof inputs.source !== 'string'
    ) {
      throw new WorkflowExecutionError(
        'WOML_INVALID_HANDLER_INPUT',
        `runtime.script input for node "${node.id}" must contain exactly one string field named "source".`,
        { nodeId: node.id },
      );
    }
    return await scriptRunner({ nodeId: node.id, source: inputs.source, context });
  });
  return registry;
}
