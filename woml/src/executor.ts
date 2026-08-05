import {
  cloneJson,
  deepFreezeJson,
  findJsonViolation,
  isJsonObject,
} from './json';
import {
  inspectCompiledWorkflowGraph,
  type CompiledWorkflowDefinition,
  type JsonObject,
  type JsonValue,
} from './model';
import {
  createRuntimeHandlerRegistry,
  resolveExecutableInput,
  type HandlerRegistry,
} from './handlers';
import { WorkflowExecutionError } from './runtime-error';

export interface WorkflowContext extends JsonObject {
  readonly trigger: JsonObject;
  readonly steps: Readonly<Record<string, JsonValue>>;
}

export interface ExecuteWorkflowOptions {
  readonly trigger?: JsonObject;
  readonly registry?: HandlerRegistry;
}

export interface WorkflowExecutionResult {
  readonly workflowId: string;
  readonly terminalNodeId: string;
  readonly result: JsonValue;
  readonly context: WorkflowContext;
  readonly executionOrder: readonly string[];
}

function contextSnapshot(
  trigger: JsonObject,
  steps: Readonly<Record<string, JsonValue>>,
): WorkflowContext {
  return deepFreezeJson(
    cloneJson({ trigger, steps }) as {
      trigger: JsonObject;
      steps: Record<string, JsonValue>;
    },
  );
}

export async function executeWorkflow(
  workflow: CompiledWorkflowDefinition,
  options: ExecuteWorkflowOptions = {},
): Promise<WorkflowExecutionResult> {
  if (workflow.schemaVersion !== 1) {
    throw new WorkflowExecutionError(
      'WOML_INVALID_COMPILED_MODEL',
      `Unsupported compiled workflow schema version "${String(workflow.schemaVersion)}".`,
    );
  }

  const graphIssues = inspectCompiledWorkflowGraph(workflow.graph, {
    requireSingleTerminal: true,
  });
  if (graphIssues.length > 0) {
    throw new WorkflowExecutionError(
      'WOML_INVALID_DAG',
      graphIssues[0].message,
    );
  }
  const unsupportedEdge = workflow.graph.edges.find(
    (edge) => edge.condition.kind !== 'always',
  );
  if (unsupportedEdge !== undefined) {
    throw new WorkflowExecutionError(
      'WOML_UNSUPPORTED_EDGE_CONDITION',
      `Edge "${unsupportedEdge.id}" uses condition "${unsupportedEdge.condition.kind}", which is not executable in this runtime profile.`,
    );
  }

  const triggerInput = options.trigger ?? {};
  if (!isJsonObject(triggerInput)) {
    const violation = findJsonViolation(triggerInput);
    throw new WorkflowExecutionError(
      'WOML_INVALID_TRIGGER',
      `Workflow trigger must be a JSON object${
        violation === undefined ? '.' : `; ${violation.path}: ${violation.reason}.`
      }`,
    );
  }

  const trigger = cloneJson(triggerInput);
  const steps: Record<string, JsonValue> = Object.create(null) as Record<
    string,
    JsonValue
  >;
  const completed = new Set<string>();
  const executionOrder: string[] = [];
  const registry = options.registry ?? createRuntimeHandlerRegistry();
  const incoming = new Map<string, string[]>();
  const outgoingCount = new Map<string, number>();
  for (const node of workflow.graph.nodes) {
    incoming.set(node.id, []);
    outgoingCount.set(node.id, 0);
  }
  for (const edge of workflow.graph.edges) {
    incoming.get(edge.to)?.push(edge.from);
    outgoingCount.set(edge.from, (outgoingCount.get(edge.from) ?? 0) + 1);
  }

  while (completed.size < workflow.graph.nodes.length) {
    const node = workflow.graph.nodes.find(
      (candidate) =>
        !completed.has(candidate.id) &&
        (incoming.get(candidate.id) ?? []).every((id) => completed.has(id)),
    );
    if (node === undefined) {
      throw new WorkflowExecutionError(
        'WOML_INVALID_DAG',
        'No executable node is ready before the workflow has completed.',
      );
    }

    const handler = registry.get(node.handler);
    if (handler === undefined) {
      throw new WorkflowExecutionError(
        'WOML_UNKNOWN_HANDLER',
        `No handler is registered for "${node.handler}" on node "${node.id}".`,
        { nodeId: node.id },
      );
    }

    const inputs = resolveExecutableInput(node.inputs, node.id);
    let output: JsonValue;
    try {
      output = await handler({
        node,
        inputs,
        context: contextSnapshot(trigger, steps),
      });
    } catch (error) {
      if (error instanceof WorkflowExecutionError) throw error;
      throw new WorkflowExecutionError(
        'WOML_HANDLER_FAILED',
        `Handler "${node.handler}" failed for node "${node.id}".`,
        { nodeId: node.id, cause: error },
      );
    }

    const violation = findJsonViolation(output);
    if (violation !== undefined) {
      throw new WorkflowExecutionError(
        'WOML_NON_JSON_RESULT',
        `Node "${node.id}" returned a non-JSON value at ${violation.path}: ${violation.reason}.`,
        { nodeId: node.id },
      );
    }

    steps[node.id] = cloneJson(output);
    completed.add(node.id);
    executionOrder.push(node.id);
  }

  const terminalNode = workflow.graph.nodes.find(
    (node) => (outgoingCount.get(node.id) ?? 0) === 0,
  );
  if (
    terminalNode === undefined ||
    !Object.prototype.hasOwnProperty.call(steps, terminalNode.id)
  ) {
    throw new WorkflowExecutionError(
      'WOML_INVALID_DAG',
      'The workflow terminal node did not produce a result.',
    );
  }

  const finalContext = contextSnapshot(trigger, steps);
  return {
    workflowId: workflow.workflowId,
    terminalNodeId: terminalNode.id,
    result: finalContext.steps[terminalNode.id],
    context: finalContext,
    executionOrder,
  };
}
