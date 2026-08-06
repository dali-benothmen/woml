export type JsonPrimitive = null | boolean | number | string;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };
export type JsonObject = { readonly [key: string]: JsonValue };

export interface LiteralExpression {
  readonly kind: 'literal';
  readonly value: JsonValue;
}

export interface ContextReferenceExpression {
  readonly kind: 'contextReference';
  readonly path: readonly string[];
}

export interface ObjectExpression {
  readonly kind: 'object';
  readonly fields: Readonly<Record<string, ValueExpression>>;
}

export interface ArrayExpression {
  readonly kind: 'array';
  readonly items: readonly ValueExpression[];
}

export interface TemplateTextPart {
  readonly kind: 'text';
  readonly text: string;
}

export interface TemplateExpression {
  readonly kind: 'template';
  readonly parts: readonly (TemplateTextPart | ContextReferenceExpression)[];
}

export type ValueExpression =
  | LiteralExpression
  | ContextReferenceExpression
  | ObjectExpression
  | ArrayExpression
  | TemplateExpression;

export interface NoBackoffPolicy {
  readonly kind: 'none';
}

export interface FixedBackoffPolicy {
  readonly kind: 'fixed';
  readonly delayMs: number;
}

export interface ExponentialBackoffPolicy {
  readonly kind: 'exponential';
  readonly initialDelayMs: number;
  readonly multiplier: number;
  readonly maximumDelayMs?: number;
}

export type BackoffPolicy =
  | NoBackoffPolicy
  | FixedBackoffPolicy
  | ExponentialBackoffPolicy;

export interface RetryPolicy {
  readonly maxAttempts: number;
  readonly backoff: BackoffPolicy;
}

export interface AlwaysEdgeCondition {
  readonly kind: 'always';
}

export interface TruthyEdgeCondition {
  readonly kind: 'truthy';
  readonly value: ValueExpression;
}

export interface EqualsEdgeCondition {
  readonly kind: 'equals';
  readonly left: ValueExpression;
  readonly right: ValueExpression;
}

export type EdgeCondition =
  | AlwaysEdgeCondition
  | TruthyEdgeCondition
  | EqualsEdgeCondition;

export interface CompiledWorkflowMetadata {
  readonly name?: string;
  readonly description?: string;
  readonly version?: string;
  readonly labels?: Readonly<Record<string, string>>;
}

export interface CompiledTrigger {
  readonly id: string;
  readonly handler: string;
  readonly config: ValueExpression;
}

export interface CompiledWorkflowNode {
  readonly id: string;
  readonly handler: string;
  readonly inputs: ValueExpression;
  readonly timeoutMs?: number;
  readonly retryPolicy?: RetryPolicy;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

export interface CompiledWorkflowEdge {
  readonly id: string;
  readonly from: string;
  readonly to: string;
  readonly condition: EdgeCondition;
  readonly branchId?: string;
}

export interface CompiledWorkflowGraph {
  readonly entryNodeIds: readonly string[];
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
}

export interface CompiledWorkflowDefinition {
  readonly schemaVersion: 1;
  readonly workflowId: string;
  readonly metadata?: CompiledWorkflowMetadata;
  readonly triggers: readonly CompiledTrigger[];
  readonly graph: CompiledWorkflowGraph;
}

export interface CompiledGraphIssue {
  readonly code:
    | 'DUPLICATE_NODE_ID'
    | 'DUPLICATE_EDGE_ID'
    | 'DUPLICATE_ENTRY_NODE_ID'
    | 'UNKNOWN_EDGE_ENDPOINT'
    | 'INVALID_ENTRY_NODE'
    | 'UNREACHABLE_NODE'
    | 'CYCLIC_GRAPH'
    | 'TERMINAL_NODE_COUNT';
  readonly message: string;
}

export interface InspectCompiledGraphOptions {
  readonly requireSingleTerminal?: boolean;
}

export function inspectCompiledWorkflowGraph(
  graph: CompiledWorkflowGraph,
  options: InspectCompiledGraphOptions = {},
): readonly CompiledGraphIssue[] {
  const issues: CompiledGraphIssue[] = [];
  const nodeIds = new Set<string>();
  const duplicateNodeIds = new Set<string>();
  for (const node of graph.nodes) {
    if (nodeIds.has(node.id)) duplicateNodeIds.add(node.id);
    nodeIds.add(node.id);
  }
  for (const id of duplicateNodeIds) {
    issues.push({
      code: 'DUPLICATE_NODE_ID',
      message: `Compiled graph contains duplicate node ID "${id}".`,
    });
  }

  const edgeIds = new Set<string>();
  const duplicateEdgeIds = new Set<string>();
  const adjacency = new Map<string, string[]>();
  const incoming = new Map<string, number>();
  for (const id of nodeIds) {
    adjacency.set(id, []);
    incoming.set(id, 0);
  }

  for (const edge of graph.edges) {
    if (edgeIds.has(edge.id)) duplicateEdgeIds.add(edge.id);
    edgeIds.add(edge.id);

    if (!nodeIds.has(edge.from) || !nodeIds.has(edge.to)) {
      issues.push({
        code: 'UNKNOWN_EDGE_ENDPOINT',
        message: `Edge "${edge.id}" references an unknown endpoint (${edge.from} -> ${edge.to}).`,
      });
      continue;
    }
    adjacency.get(edge.from)?.push(edge.to);
    incoming.set(edge.to, (incoming.get(edge.to) ?? 0) + 1);
  }
  for (const id of duplicateEdgeIds) {
    issues.push({
      code: 'DUPLICATE_EDGE_ID',
      message: `Compiled graph contains duplicate edge ID "${id}".`,
    });
  }

  const entryIds = new Set<string>();
  for (const entryId of graph.entryNodeIds) {
    if (entryIds.has(entryId)) {
      issues.push({
        code: 'DUPLICATE_ENTRY_NODE_ID',
        message: `Compiled graph repeats entry node ID "${entryId}".`,
      });
      continue;
    }
    entryIds.add(entryId);
    if (!nodeIds.has(entryId)) {
      issues.push({
        code: 'INVALID_ENTRY_NODE',
        message: `Entry node "${entryId}" does not exist.`,
      });
    } else if ((incoming.get(entryId) ?? 0) !== 0) {
      issues.push({
        code: 'INVALID_ENTRY_NODE',
        message: `Entry node "${entryId}" has an incoming edge.`,
      });
    }
  }

  const reachable = new Set<string>();
  const queue = [...entryIds].filter((id) => nodeIds.has(id));
  while (queue.length > 0) {
    const id = queue.shift();
    if (id === undefined || reachable.has(id)) continue;
    reachable.add(id);
    queue.push(...(adjacency.get(id) ?? []));
  }
  for (const id of nodeIds) {
    if (!reachable.has(id)) {
      issues.push({
        code: 'UNREACHABLE_NODE',
        message: `Node "${id}" is not reachable from an entry node.`,
      });
    }
  }

  const remainingIncoming = new Map(incoming);
  const ready = [...nodeIds].filter((id) => (remainingIncoming.get(id) ?? 0) === 0);
  let visitedCount = 0;
  while (ready.length > 0) {
    const id = ready.shift();
    if (id === undefined) continue;
    visitedCount += 1;
    for (const next of adjacency.get(id) ?? []) {
      const nextIncoming = (remainingIncoming.get(next) ?? 0) - 1;
      remainingIncoming.set(next, nextIncoming);
      if (nextIncoming === 0) ready.push(next);
    }
  }
  if (visitedCount !== nodeIds.size) {
    issues.push({
      code: 'CYCLIC_GRAPH',
      message: 'Compiled graph contains a cycle.',
    });
  }

  if (options.requireSingleTerminal === true) {
    const terminalIds = [...nodeIds].filter(
      (id) => (adjacency.get(id)?.length ?? 0) === 0,
    );
    if (terminalIds.length !== 1) {
      issues.push({
        code: 'TERMINAL_NODE_COUNT',
        message: `The first CLI profile requires exactly one terminal node; found ${terminalIds.length}.`,
      });
    }
  }

  return issues;
}
