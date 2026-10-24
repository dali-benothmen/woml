export type JsonPrimitive = null | boolean | number | string;
export type JsonValue =
  JsonPrimitive | readonly JsonValue[] | { readonly [key: string]: JsonValue };
export type JsonObject = { readonly [key: string]: JsonValue };

export interface LiteralExpression {
  readonly kind: 'literal';
  readonly value: JsonValue;
}

export interface ContextReferenceExpression {
  readonly kind: 'contextReference';
  readonly path: readonly string[];
}

export interface LifecycleReferenceExpression {
  readonly kind: 'lifecycleReference';
  readonly path: readonly string[];
}

export interface SecretReferenceExpression {
  readonly kind: 'secretReference';
  readonly name: string;
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
  readonly parts: readonly (
    TemplateTextPart | ContextReferenceExpression | LifecycleReferenceExpression
  )[];
}

export type ValueExpression =
  | LiteralExpression
  | ContextReferenceExpression
  | LifecycleReferenceExpression
  | SecretReferenceExpression
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
  NoBackoffPolicy | FixedBackoffPolicy | ExponentialBackoffPolicy;

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

export interface BooleanEdgeCondition {
  readonly kind: 'boolean';
  readonly value: ContextReferenceExpression;
}

export interface EqualsEdgeCondition {
  readonly kind: 'equals';
  readonly left: ValueExpression;
  readonly right: ValueExpression;
}

export type EdgeCondition =
  | AlwaysEdgeCondition
  | BooleanEdgeCondition
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

export interface CompiledExecutionPlacementV1 {
  readonly profileVersion: 1;
  readonly target: string;
  readonly requiredCapabilities: readonly string[];
}

export interface CompiledWorkflowNode {
  readonly id: string;
  readonly handler: string;
  readonly inputs: ValueExpression;
  readonly timeoutMs?: number;
  readonly retryPolicy?: RetryPolicy;
  readonly scriptRuntime?: ScriptRuntimeBindingsV1 | ScriptRuntimeBindingsV3;
  readonly execution?: CompiledExecutionPlacementV1;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

export interface ScriptRuntimeBindingsV1 {
  readonly bindingVersion: 1;
  readonly bindings: readonly ['context', 'attempt', 'services', 'secrets'];
  readonly requiredSecrets: readonly string[];
}

export interface ScriptRuntimeBindingsV2 {
  readonly bindingVersion: 2;
  readonly bindings: readonly [
    'context',
    'lifecycle',
    'attempt',
    'services',
    'secrets',
  ];
  readonly requiredSecrets: readonly string[];
}

export interface ScriptRuntimeBindingsV3 {
  readonly bindingVersion: 3;
  readonly bindings: readonly ['props', 'context', 'attempt', 'services'];
  readonly requiredSecrets: readonly string[];
}

export type ReusablePropExpressionV1 =
  | { readonly kind: 'literal'; readonly value: string }
  | { readonly kind: 'context'; readonly path: string }
  | { readonly kind: 'secret'; readonly name: string };

export interface CompiledReusableBoundPropV1 {
  readonly name: string;
  readonly bindingName: string;
  readonly secret: boolean;
  readonly expression: ReusablePropExpressionV1;
}

export interface CompiledReusableLifecycleV1 {
  readonly onSuccess?: readonly string[];
  readonly onError?: readonly string[];
  readonly onComplete?: readonly string[];
}

export interface CompiledReusableStepInvocationV1 {
  readonly kind: 'step';
  readonly invocationId: string;
  readonly nodeId: string;
  readonly alias: string;
  readonly definitionDigest: string;
  readonly source: string;
  readonly scriptArtifactId: string;
  readonly props: readonly CompiledReusableBoundPropV1[];
  readonly lifecycle?: CompiledReusableLifecycleV1;
}

export interface CompiledReusableNotificationProviderV1 {
  readonly kind: 'notification-provider';
  readonly providerId: string;
  readonly alias: string;
  readonly definitionDigest: string;
  readonly source: string;
  readonly scriptArtifactId: string;
  readonly props: readonly CompiledReusableBoundPropV1[];
  readonly lifecycle?: CompiledReusableLifecycleV1;
}

export type CompiledReusableInvocationV1 =
  CompiledReusableStepInvocationV1 | CompiledReusableNotificationProviderV1;

export type LifecycleEventName =
  | 'run_start'
  | 'step_start'
  | 'step_success'
  | 'step_failure'
  | 'step_complete'
  | 'run_success'
  | 'run_failure'
  | 'run_cancel'
  | 'run_complete';

export interface CompiledLifecycleScriptActionV1 {
  readonly actionId: string;
  readonly handler: 'runtime.lifecycle-script';
  readonly inputs: ValueExpression;
  readonly scriptRuntime: ScriptRuntimeBindingsV2;
}

export interface CompiledLifecycleNotificationActionV1 {
  readonly actionId: string;
  readonly handler: 'notification.informational';
  readonly inputs: ValueExpression;
}

export type CompiledLifecycleActionV1 =
  CompiledLifecycleScriptActionV1 | CompiledLifecycleNotificationActionV1;

export interface CompiledLifecycleHookV1 {
  readonly hookId: string;
  readonly event: LifecycleEventName;
  readonly stepIds?: readonly string[];
  readonly actions: readonly CompiledLifecycleActionV1[];
}

export interface CompiledLifecycleDefinitionV1 {
  readonly profileVersion: 1;
  readonly hooks: readonly CompiledLifecycleHookV1[];
}

export interface CompiledModuleBindingV1 {
  readonly name: string;
  readonly bundleDigest: string;
  readonly sourceMapDigest: string;
  readonly exports: readonly string[];
}

export interface CompiledModuleRuntimeV1 {
  readonly profileVersion: 1;
  readonly modules: readonly CompiledModuleBindingV1[];
}

export interface CompiledRuntimePolicyV1 {
  readonly profileVersion: 1;
  readonly concurrency?: number;
  readonly timeoutMs?: number;
  readonly rateLimit?: Readonly<{
    readonly count: number;
    readonly windowMs: number;
    readonly algorithm: 'rolling_window';
  }>;
  readonly queue?: Readonly<{
    readonly name: string;
    readonly discipline: 'work_conserving_fifo';
  }>;
}

export interface CompiledWorkflowEdge {
  readonly id: string;
  readonly from: string;
  readonly to: string;
  readonly condition: EdgeCondition;
  readonly branchId?: string;
  readonly parallelId?: string;
  readonly approvalId?: string;
}

export interface CompiledForkBranchV1 {
  readonly branchId: string;
  readonly entryNodeId: string;
  readonly terminalNodeId: string;
}

export interface CompiledForkV1 {
  readonly forkId: string;
  readonly openNodeId: string;
  readonly joinNodeId: string;
  readonly branches: readonly CompiledForkBranchV1[];
  readonly joinedBranchIds: readonly string[];
}

export interface CompiledControlChoiceV1 {
  readonly choiceId: string;
  readonly selectorNodeId: string;
  readonly joinNodeId: string;
  readonly armIds: readonly string[];
  readonly stringSelector?: ContextReferenceExpression;
  readonly stringCases?: readonly {
    readonly armId: string;
    readonly value: string;
  }[];
  readonly defaultArmId?: string;
  readonly resultNodeId?: string;
}

export interface CompiledContextVisibilityV1 {
  readonly nodeId: string;
  readonly stepIds: readonly string[];
}

export interface CompiledWorkflowSettlementV1 {
  readonly nodeId: string;
  readonly mainResultNodeId: string;
  readonly ownedBranchTerminalNodeIds: readonly string[];
}

export interface CompiledForEachBodyV1 {
  readonly entryNodeIds: readonly [string];
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
  readonly choices: readonly CompiledControlChoiceV1[];
  readonly contextVisibility: readonly CompiledContextVisibilityV1[];
  readonly terminalNodeId: string;
}

export interface CompiledForEachV1 {
  readonly forEachId: string;
  readonly openNodeId: string;
  readonly resultNodeId: string;
  readonly metadata?: Readonly<{
    readonly name?: string;
    readonly description?: string;
  }>;
  readonly items: ContextReferenceExpression;
  readonly concurrency: number;
  readonly outerStepIds: readonly string[];
  readonly body: CompiledForEachBodyV1;
  readonly result?: ContextReferenceExpression;
}

export interface CompiledWaitSignalV1 {
  readonly signal: string;
  readonly schema?: JsonObject;
}

export type CompiledWaitV1 =
  | {
      readonly waitId: string;
      readonly nodeId: string;
      readonly kind: 'duration';
      readonly durationMs: number;
      readonly metadata?: Readonly<{
        readonly name?: string;
        readonly description?: string;
      }>;
    }
  | {
      readonly waitId: string;
      readonly nodeId: string;
      readonly kind: 'until';
      readonly at: ValueExpression;
      readonly metadata?: Readonly<{
        readonly name?: string;
        readonly description?: string;
      }>;
    }
  | {
      readonly waitId: string;
      readonly nodeId: string;
      readonly kind: 'signal';
      readonly signals: readonly CompiledWaitSignalV1[];
      readonly correlation: ValueExpression;
      readonly timeoutMs?: number;
      readonly onTimeout: 'fail' | 'continue';
      readonly metadata?: Readonly<{
        readonly name?: string;
        readonly description?: string;
      }>;
    };

export interface CompiledWorkflowGraph {
  readonly entryNodeIds: readonly string[];
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
  readonly forks?: readonly CompiledForkV1[];
  readonly choices?: readonly CompiledControlChoiceV1[];
  readonly contextVisibility?: readonly CompiledContextVisibilityV1[];
  readonly settlement?: CompiledWorkflowSettlementV1;
}

export interface CompiledWorkflowGraphV13 extends CompiledWorkflowGraph {
  readonly forks: readonly CompiledForkV1[];
  readonly choices: readonly CompiledControlChoiceV1[];
  readonly contextVisibility: readonly CompiledContextVisibilityV1[];
  readonly settlement: CompiledWorkflowSettlementV1;
}

export interface CompiledWorkflowGraphV16 extends CompiledWorkflowGraphV13 {
  readonly forEach: readonly CompiledForEachV1[];
}

/** Model v17 adds first-class durable timer and external-signal waits. */
export interface CompiledWorkflowGraphV17 extends CompiledWorkflowGraphV16 {
  readonly waits: readonly CompiledWaitV1[];
}

export interface CompiledLockV1 {
  readonly lockId: string;
  readonly acquireNodeId: string;
  readonly releaseNodeId: string;
  readonly resultNodeId: string;
  readonly bodyEntryNodeId: string;
  readonly bodyTerminalNodeId: string;
  readonly key: ValueExpression;
  readonly acquireTimeoutMs: number;
  readonly leaseMs: number;
  readonly outerStepIds: readonly string[];
  readonly metadata?: Readonly<{
    readonly name?: string;
    readonly description?: string;
  }>;
}

/** Model v18 adds durable exclusive lock scopes and renewable leases. */
export interface CompiledWorkflowGraphV18 extends CompiledWorkflowGraphV17 {
  readonly locks: readonly CompiledLockV1[];
}

export type CompiledCorrectionIdentityV1 =
  | Readonly<{
      readonly kind: 'static';
      readonly actionId: string;
    }>
  | Readonly<{
      readonly kind: 'forEachIteration';
      readonly forEachId: string;
      readonly actionIdTemplate: string;
    }>;

export interface CompiledCorrectiveActionV1 {
  readonly actionId: string;
  readonly node: CompiledWorkflowNode;
  readonly identity: CompiledCorrectionIdentityV1;
  readonly visibleStepIds: readonly string[];
}

export interface CompiledCompensationV1 extends CompiledCorrectiveActionV1 {
  readonly forStepId: string;
}

export interface CompiledSagaV1 {
  readonly sagaId: string;
  readonly openNodeId: string;
  readonly commitNodeId: string;
  readonly resultNodeId: string;
  readonly bodyEntryNodeId: string;
  readonly bodyTerminalNodeId: string;
  readonly bodyResultNodeId: string;
  readonly outerStepIds: readonly string[];
  readonly compensations: readonly CompiledCompensationV1[];
  readonly cleanup: readonly CompiledCorrectiveActionV1[];
  readonly metadata?: Readonly<{
    readonly name?: string;
    readonly description?: string;
  }>;
}

/** Model v19 adds explicit sagas with a separate corrective execution plan. */
export interface CompiledWorkflowGraphV19 extends CompiledWorkflowGraphV18 {
  readonly sagas: readonly CompiledSagaV1[];
}

/**
 * Model v20 adds immutable rich-plan approval descriptors to approval wait
 * nodes. The graph shape otherwise retains the complete Model v19 contract.
 */
export interface CompiledWorkflowGraphV20 extends CompiledWorkflowGraphV19 {}

/** Model v21 adds explicit per-step execution placement. */
export interface CompiledWorkflowGraphV21 extends CompiledWorkflowGraphV20 {}

interface CompiledWorkflowDefinitionBase {
  readonly workflowId: string;
  readonly metadata?: CompiledWorkflowMetadata;
  readonly triggers: readonly CompiledTrigger[];
  readonly graph: CompiledWorkflowGraph;
}

export interface CompiledWorkflowDefinitionV1 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 1;
}

export interface CompiledWorkflowDefinitionV2 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 2;
}

export interface CompiledWorkflowDefinitionV3 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 3;
}

export interface CompiledWorkflowDefinitionV4 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 4;
}

export interface CompiledWorkflowDefinitionV5 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 5;
}

export interface CompiledWorkflowDefinitionV6 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 6;
}

export interface CompiledWorkflowDefinitionV7 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 7;
}

export interface CompiledWorkflowDefinitionV8 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 8;
}

export interface CompiledWorkflowDefinitionV9 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 9;
  readonly moduleRuntime: CompiledModuleRuntimeV1;
}

export interface CompiledWorkflowDefinitionV10 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 10;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
}

export interface CompiledWorkflowDefinitionV11 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 11;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
  readonly lifecycle?: CompiledLifecycleDefinitionV1;
}

export interface CompiledWorkflowDefinitionV12 extends CompiledWorkflowDefinitionBase {
  readonly schemaVersion: 12;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
  readonly lifecycle?: CompiledLifecycleDefinitionV1;
  readonly runtimePolicy: CompiledRuntimePolicyV1;
}

export interface CompiledWorkflowDefinitionV13 extends Omit<
  CompiledWorkflowDefinitionBase,
  'graph'
> {
  readonly schemaVersion: 13;
  readonly graph: CompiledWorkflowGraphV13;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
  readonly lifecycle?: CompiledLifecycleDefinitionV1;
  readonly runtimePolicy: CompiledRuntimePolicyV1;
}

export interface CompiledWorkflowDefinitionV14 extends Omit<
  CompiledWorkflowDefinitionBase,
  'graph'
> {
  readonly schemaVersion: 14;
  readonly graph: CompiledWorkflowGraphV13;
  readonly moduleRuntime?: CompiledModuleRuntimeV1;
  readonly lifecycle?: CompiledLifecycleDefinitionV1;
  readonly runtimePolicy: CompiledRuntimePolicyV1;
  readonly reusableDefinitions?: readonly CompiledReusableInvocationV1[];
}

export interface CompiledCommunicationProviderRequirementV1 {
  readonly provider: 'telegram' | 'discord' | 'whatsapp';
  readonly triggerIds: readonly string[];
  readonly notificationDeliveryIds: readonly string[];
  readonly messaging: boolean;
  readonly credentialNames: readonly string[];
}

export interface CompiledCommunicationRequirementsV1 {
  readonly profileVersion: 1;
  readonly providers: readonly CompiledCommunicationProviderRequirementV1[];
}

/**
 * Model v15 adds built-in communication-provider requirements while retaining
 * the complete v14 graph, switch, reusable-definition, and policy contracts.
 */
export interface CompiledWorkflowDefinitionV15 extends Omit<
  CompiledWorkflowDefinitionV14,
  'schemaVersion'
> {
  readonly schemaVersion: 15;
  readonly communication: CompiledCommunicationRequirementsV1;
}

/**
 * Model v16 adds immutable runtime-sized for-each body templates. Communication
 * remains present only when the workflow actually uses a built-in provider.
 */
export interface CompiledWorkflowDefinitionV16 extends Omit<
  CompiledWorkflowDefinitionV14,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 16;
  readonly graph: CompiledWorkflowGraphV16;
  readonly communication?: CompiledCommunicationRequirementsV1;
}

export interface CompiledWorkflowDefinitionV17 extends Omit<
  CompiledWorkflowDefinitionV16,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 17;
  readonly graph: CompiledWorkflowGraphV17;
}

export interface CompiledWorkflowDefinitionV18 extends Omit<
  CompiledWorkflowDefinitionV16,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 18;
  readonly graph: CompiledWorkflowGraphV18;
}

export interface CompiledWorkflowDefinitionV19 extends Omit<
  CompiledWorkflowDefinitionV16,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 19;
  readonly graph: CompiledWorkflowGraphV19;
}

export interface CompiledWorkflowDefinitionV20 extends Omit<
  CompiledWorkflowDefinitionV16,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 20;
  readonly graph: CompiledWorkflowGraphV20;
}

export interface CompiledWorkflowDefinitionV21 extends Omit<
  CompiledWorkflowDefinitionV16,
  'schemaVersion' | 'graph'
> {
  readonly schemaVersion: 21;
  readonly graph: CompiledWorkflowGraphV21;
}

export type CompiledWorkflowDefinition =
  | CompiledWorkflowDefinitionV1
  | CompiledWorkflowDefinitionV2
  | CompiledWorkflowDefinitionV3
  | CompiledWorkflowDefinitionV4
  | CompiledWorkflowDefinitionV5
  | CompiledWorkflowDefinitionV6
  | CompiledWorkflowDefinitionV7
  | CompiledWorkflowDefinitionV8
  | CompiledWorkflowDefinitionV9
  | CompiledWorkflowDefinitionV10
  | CompiledWorkflowDefinitionV11
  | CompiledWorkflowDefinitionV12
  | CompiledWorkflowDefinitionV13
  | CompiledWorkflowDefinitionV14
  | CompiledWorkflowDefinitionV15
  | CompiledWorkflowDefinitionV16
  | CompiledWorkflowDefinitionV17
  | CompiledWorkflowDefinitionV18
  | CompiledWorkflowDefinitionV19
  | CompiledWorkflowDefinitionV20
  | CompiledWorkflowDefinitionV21;

export interface CompiledGraphIssue {
  readonly code:
    | 'DUPLICATE_NODE_ID'
    | 'DUPLICATE_EDGE_ID'
    | 'DUPLICATE_ENTRY_NODE_ID'
    | 'UNKNOWN_EDGE_ENDPOINT'
    | 'INVALID_ENTRY_NODE'
    | 'UNREACHABLE_NODE'
    | 'CYCLIC_GRAPH'
    | 'TERMINAL_NODE_COUNT'
    | 'INVALID_BRANCH_SELECTOR'
    | 'INVALID_BRANCH_GROUP'
    | 'INVALID_BRANCH_RESULT'
    | 'INVALID_PARALLEL_GROUP'
    | 'INVALID_APPROVAL_GROUP'
    | 'INVALID_NOTIFICATION_GROUP'
    | 'INVALID_FORK_GRAPH'
    | 'INVALID_CONTROL_CHOICE'
    | 'INVALID_CONTEXT_VISIBILITY'
    | 'INVALID_WORKFLOW_SETTLEMENT'
    | 'INVALID_FOR_EACH'
    | 'INVALID_DURABLE_WAIT'
    | 'INVALID_LOCK'
    | 'INVALID_SAGA';
  readonly message: string;
}

export interface InspectCompiledGraphOptions {
  readonly requireSingleTerminal?: boolean;
}

export function inspectCompiledWorkflowGraph<TGraph extends CompiledWorkflowGraph>(
  graph: TGraph,
  options: InspectCompiledGraphOptions = {}
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
  const queue = [...entryIds].filter(id => nodeIds.has(id));
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
  const ready = [...nodeIds].filter(
    id => (remainingIncoming.get(id) ?? 0) === 0
  );
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
      id => (adjacency.get(id)?.length ?? 0) === 0
    );
    if (terminalIds.length !== 1) {
      issues.push({
        code: 'TERMINAL_NODE_COUNT',
        message: `The first CLI profile requires exactly one terminal node; found ${terminalIds.length}.`,
      });
    }
  }

  inspectBranchGroups(graph, issues);
  inspectParallelGroups(graph, issues);
  inspectApprovalGroups(graph, issues);
  inspectForkGraph(graph, issues);
  inspectDurableWaitGraph(graph, issues);
  inspectLockGraph(graph, issues);
  inspectSagaGraph(graph, issues);

  return issues;
}

function approvalJoinId(approvalId: string): string {
  return `__woml_approval__${approvalId}__join`;
}

function isApprovalDecisionCondition(
  condition: EdgeCondition,
  approvalId: string,
  decision: 'approved' | 'rejected'
): boolean {
  return (
    condition.kind === 'equals' &&
    condition.left.kind === 'contextReference' &&
    condition.left.path.length === 3 &&
    condition.left.path[0] === 'steps' &&
    condition.left.path[1] === approvalId &&
    condition.left.path[2] === 'decision' &&
    condition.right.kind === 'literal' &&
    condition.right.value === decision
  );
}

const compiledSlackAliasPattern = /^#[a-z0-9][a-z0-9_-]{0,79}$/;
const compiledSlackConversationIdPattern = /^[CG][A-Z0-9]{8,31}$/;
const compiledTelegramChatIdPattern = /^-?[1-9][0-9]{0,19}$/;
const compiledDiscordChannelIdPattern = /^[0-9]{17,20}$/;
const compiledWhatsAppRecipientPattern = /^[1-9][0-9]{7,15}$/;
const compiledWhatsAppTemplatePattern = /^[a-z0-9_]+$/;
const compiledWhatsAppLanguagePattern = /^[a-z]{2,3}(?:_[A-Z]{2})?$/;
const compiledSecretNamePattern = /^[A-Z][A-Z0-9_]*$/;

function hasExactlyKeys(
  fields: Readonly<Record<string, ValueExpression>>,
  expected: readonly string[]
): boolean {
  const keys = Object.keys(fields);
  return (
    keys.length === expected.length && expected.every(key => key in fields)
  );
}

function validApprovalNotifications(
  expression: ValueExpression | undefined,
  approvalId: string
): boolean {
  if (expression === undefined) return true;
  if (expression.kind !== 'array' || expression.items.length === 0)
    return false;

  let previousProviderIndex = -1;
  let previousChannelIndex = -1;
  const deliveryIds = new Set<string>();
  const destinationKeys = new Set<string>();
  for (const item of expression.items) {
    if (item.kind !== 'object') {
      return false;
    }
    const { deliveryId, provider, destination, credentials } = item.fields;
    const providerName =
      provider?.kind === 'literal' ? provider.value : undefined;
    const slackDelivery = providerName === 'slack';
    const telegramDelivery = providerName === 'telegram';
    const discordDelivery = providerName === 'discord';
    const whatsappDelivery = providerName === 'whatsapp';
    const expectedFields = whatsappDelivery
      ? [
          'deliveryId',
          'provider',
          'destination',
          'credentials',
          'templateName',
          'language',
        ]
      : ['deliveryId', 'provider', 'destination', 'credentials'];
    if (
      !hasExactlyKeys(item.fields, expectedFields) ||
      deliveryId?.kind !== 'literal' ||
      typeof deliveryId.value !== 'string' ||
      (!slackDelivery &&
        !telegramDelivery &&
        !discordDelivery &&
        !whatsappDelivery) ||
      destination?.kind !== 'literal' ||
      typeof destination.value !== 'string' ||
      (slackDelivery &&
        !compiledSlackAliasPattern.test(destination.value) &&
        !compiledSlackConversationIdPattern.test(destination.value)) ||
      (telegramDelivery &&
        !compiledTelegramChatIdPattern.test(destination.value)) ||
      (discordDelivery &&
        !compiledDiscordChannelIdPattern.test(destination.value)) ||
      (whatsappDelivery &&
        !compiledWhatsAppRecipientPattern.test(destination.value)) ||
      credentials?.kind !== 'object' ||
      !hasExactlyKeys(
        credentials.fields,
        slackDelivery
          ? ['botToken', 'appToken']
          : whatsappDelivery
            ? ['accessToken', 'phoneNumberId']
            : ['botToken']
      )
    ) {
      return false;
    }
    const match = new RegExp(
      `^${approvalId}:notify:([0-9]+):${telegramDelivery ? 'chat' : whatsappDelivery ? 'recipient' : 'channel'}:([0-9]+)$`
    ).exec(deliveryId.value);
    const botToken = credentials.fields.botToken;
    const appToken = credentials.fields.appToken;
    const accessToken = credentials.fields.accessToken;
    const phoneNumberId = credentials.fields.phoneNumberId;
    const templateName = item.fields.templateName;
    const language = item.fields.language;
    if (
      match === null ||
      deliveryIds.has(deliveryId.value) ||
      (!whatsappDelivery &&
        (botToken?.kind !== 'secretReference' ||
          !compiledSecretNamePattern.test(botToken.name))) ||
      (slackDelivery &&
        (appToken?.kind !== 'secretReference' ||
          !compiledSecretNamePattern.test(appToken.name))) ||
      (whatsappDelivery &&
        (accessToken?.kind !== 'secretReference' ||
          !compiledSecretNamePattern.test(accessToken.name) ||
          phoneNumberId?.kind !== 'literal' ||
          typeof phoneNumberId.value !== 'string' ||
          templateName?.kind !== 'literal' ||
          typeof templateName.value !== 'string' ||
          !compiledWhatsAppTemplatePattern.test(templateName.value) ||
          language?.kind !== 'literal' ||
          typeof language.value !== 'string' ||
          !compiledWhatsAppLanguagePattern.test(language.value)))
    ) {
      return false;
    }

    const providerIndex = Number(match[1]);
    const channelIndex = Number(match[2]);
    const followsPrevious =
      previousProviderIndex === -1
        ? providerIndex === 0 && channelIndex === 0
        : providerIndex === previousProviderIndex
          ? channelIndex === previousChannelIndex + 1
          : providerIndex === previousProviderIndex + 1 && channelIndex === 0;
    const credentialName =
      whatsappDelivery && accessToken?.kind === 'secretReference'
        ? accessToken.name
        : botToken?.kind === 'secretReference'
          ? botToken.name
          : '';
    const destinationKey = `${providerName}\u0000${credentialName}\u0000${appToken?.kind === 'secretReference' ? appToken.name : ''}\u0000${destination.value}`;
    if (!followsPrevious || destinationKeys.has(destinationKey)) return false;

    deliveryIds.add(deliveryId.value);
    destinationKeys.add(destinationKey);
    previousProviderIndex = providerIndex;
    previousChannelIndex = channelIndex;
  }
  return true;
}

function inspectApprovalGroups(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const groups = new Map<string, CompiledWorkflowEdge[]>();
  for (const edge of graph.edges) {
    if (edge.approvalId === undefined) {
      if (edge.condition.kind === 'equals') {
        issues.push({
          code: 'INVALID_APPROVAL_GROUP',
          message: `Equals edge "${edge.id}" must carry an approvalId.`,
        });
      }
      continue;
    }
    const group = groups.get(edge.approvalId) ?? [];
    group.push(edge);
    groups.set(edge.approvalId, group);
    if (edge.branchId !== undefined || edge.parallelId !== undefined) {
      issues.push({
        code: 'INVALID_APPROVAL_GROUP',
        message: `Approval edge "${edge.id}" cannot also belong to a branch or parallel group.`,
      });
    }
  }

  for (const [approvalId, edges] of groups) {
    const wait = nodes.get(approvalId);
    const joinId = approvalJoinId(approvalId);
    const join = nodes.get(joinId);
    const waitFields = wait?.inputs.kind === 'object' ? wait.inputs.fields : {};
    const timeout = waitFields.timeoutMs;
    const onTimeout = waitFields.onTimeout;
    const notifications = waitFields.notifications;
    const review = waitFields.review;
    const requiredApprovals = waitFields.requiredApprovals;
    const requiredRoles = waitFields.requiredRoles;
    const richFieldCount =
      Number(review !== undefined) +
      Number(requiredApprovals !== undefined) +
      Number(requiredRoles !== undefined);
    const rolePattern = /^[a-z][a-z0-9]*(?:[-_.][a-z0-9]+)*$/;
    const roles =
      requiredRoles?.kind === 'array'
        ? requiredRoles.items.flatMap(item =>
            item.kind === 'literal' && typeof item.value === 'string'
              ? [item.value]
              : []
          )
        : [];
    const richReviewIsValid =
      richFieldCount === 0 ||
      (richFieldCount === 3 &&
        review?.kind === 'contextReference' &&
        review.path.length >= 2 &&
        review.path[0] === 'steps' &&
        publicStructuralIdPattern.test(review.path[1] ?? '') &&
        requiredApprovals?.kind === 'literal' &&
        typeof requiredApprovals.value === 'number' &&
        Number.isSafeInteger(requiredApprovals.value) &&
        requiredApprovals.value >= 1 &&
        requiredApprovals.value <= 20 &&
        requiredRoles?.kind === 'array' &&
        requiredRoles.items.length <= 16 &&
        roles.length === requiredRoles.items.length &&
        roles.every(role => role.length <= 64 && rolePattern.test(role)) &&
        new Set(roles).size === roles.length &&
        roles.every((role, index) => index === 0 || roles[index - 1]! < role));
    const notificationsAreValid = validApprovalNotifications(
      notifications,
      approvalId
    );
    const waitMetadata = wait?.metadata;
    const validWaitMetadata =
      waitMetadata === undefined ||
      (Object.keys(waitMetadata).every(key =>
        ['name', 'description'].includes(key)
      ) &&
        Object.values(waitMetadata).every(
          value => typeof value === 'string' && value.length > 0
        ));
    const validWait =
      publicStructuralIdPattern.test(approvalId) &&
      wait?.handler === 'engine.approval-wait' &&
      wait.timeoutMs === undefined &&
      wait.retryPolicy === undefined &&
      wait.inputs.kind === 'object' &&
      Object.keys(waitFields).every(key =>
        [
          'timeoutMs',
          'onTimeout',
          'notifications',
          'review',
          'requiredApprovals',
          'requiredRoles',
        ].includes(key)
      ) &&
      Object.keys(waitFields).length ===
        1 +
          Number(timeout !== undefined) +
          Number(notifications !== undefined) +
          richFieldCount &&
      onTimeout?.kind === 'literal' &&
      (onTimeout.value === 'reject' || onTimeout.value === 'fail') &&
      (timeout === undefined ||
        (timeout.kind === 'literal' &&
          typeof timeout.value === 'number' &&
          Number.isSafeInteger(timeout.value) &&
          timeout.value >= 1)) &&
      notificationsAreValid &&
      richReviewIsValid &&
      validWaitMetadata;
    const validJoin =
      join?.handler === 'engine.approval-join' &&
      join.timeoutMs === undefined &&
      join.retryPolicy === undefined &&
      join.metadata === undefined &&
      join.inputs.kind === 'object' &&
      Object.keys(join.inputs.fields).length === 0;

    const approvedRoute = edges.find(
      edge => edge.id === `${approvalId}:approved`
    );
    const rejectedRoute = edges.find(
      edge => edge.id === `${approvalId}:rejected`
    );
    const approvedJoin = edges.find(
      edge => edge.id === `${approvalId}:approved:join`
    );
    const rejectedJoin = edges.find(
      edge => edge.id === `${approvalId}:rejected:join`
    );
    const routes = [
      {
        decision: 'approved' as const,
        route: approvedRoute,
        join: approvedJoin,
      },
      {
        decision: 'rejected' as const,
        route: rejectedRoute,
        join: rejectedJoin,
      },
    ];
    const validRoutes = routes.every(({ decision, route, join: joinEdge }) => {
      if (
        route === undefined ||
        route.from !== approvalId ||
        route.approvalId !== approvalId ||
        !isApprovalDecisionCondition(route.condition, approvalId, decision)
      ) {
        return false;
      }
      const empty = route.to === joinId;
      if (empty) return joinEdge === undefined;
      return (
        joinEdge?.from !== approvalId &&
        joinEdge?.to === joinId &&
        joinEdge?.approvalId === approvalId &&
        joinEdge.condition.kind === 'always'
      );
    });
    const expectedEdgeCount =
      2 +
      Number(approvedRoute?.to !== joinId) +
      Number(rejectedRoute?.to !== joinId);
    const boundariesAreClosed =
      graph.edges.filter(edge => edge.from === approvalId).length === 2 &&
      graph.edges
        .filter(edge => edge.from === approvalId)
        .every(edge => edge.approvalId === approvalId) &&
      graph.edges
        .filter(edge => edge.to === joinId)
        .every(edge => edge.approvalId === approvalId);

    if (
      !validWait ||
      !validJoin ||
      !validRoutes ||
      edges.length !== expectedEdgeCount ||
      !boundariesAreClosed
    ) {
      issues.push({
        code: 'INVALID_APPROVAL_GROUP',
        message: `Approval group "${approvalId}" does not match the frozen wait, decision-route, empty-arm, and join contract.`,
      });
    }
    if (!notificationsAreValid) {
      issues.push({
        code: 'INVALID_NOTIFICATION_GROUP',
        message: `Approval group "${approvalId}" has invalid compiled notification deliveries.`,
      });
    }
  }

  for (const node of graph.nodes) {
    if (node.handler === 'engine.approval-wait' && !groups.has(node.id)) {
      issues.push({
        code: 'INVALID_APPROVAL_GROUP',
        message: `Approval wait "${node.id}" has no matching edge group.`,
      });
    }
    if (node.handler === 'engine.approval-join') {
      const match = /^__woml_approval__([a-z][A-Za-z0-9]*)__join$/.exec(
        node.id
      );
      if (match === null || !groups.has(match[1])) {
        issues.push({
          code: 'INVALID_APPROVAL_GROUP',
          message: `Approval join "${node.id}" has no matching edge group.`,
        });
      }
    }
  }
}

function parallelStartId(parallelId: string): string {
  return `__woml_parallel__${parallelId}__start`;
}

function inspectParallelGroups(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const groups = new Map<string, CompiledWorkflowEdge[]>();
  for (const edge of graph.edges) {
    if (edge.parallelId === undefined) continue;
    const group = groups.get(edge.parallelId) ?? [];
    group.push(edge);
    groups.set(edge.parallelId, group);
    if (edge.branchId !== undefined || edge.condition.kind !== 'always') {
      issues.push({
        code: 'INVALID_PARALLEL_GROUP',
        message: `Parallel edge "${edge.id}" must be unconditional and cannot also belong to a branch.`,
      });
    }
  }

  for (const [parallelId, edges] of groups) {
    const startId = parallelStartId(parallelId);
    const start = nodes.get(startId);
    const join = nodes.get(parallelId);
    const fields = start?.inputs.kind === 'object' ? start.inputs.fields : {};
    const concurrency = fields.concurrency;
    const onError = fields.onError;
    const childEdges = edges.filter(edge => edge.from === startId);
    const joinEdges = edges.filter(edge => edge.to === parallelId);
    const childIds = childEdges.map(edge => edge.to);
    const inputsAreValid =
      start?.handler === 'engine.parallel-start' &&
      Object.keys(fields).length === 2 &&
      concurrency?.kind === 'literal' &&
      typeof concurrency.value === 'number' &&
      Number.isSafeInteger(concurrency.value) &&
      concurrency.value >= 1 &&
      concurrency.value <= childEdges.length &&
      onError?.kind === 'literal' &&
      (onError.value === 'fail-fast' || onError.value === 'wait-all');
    const joinIsValid =
      join?.handler === 'engine.parallel-join' &&
      join.inputs.kind === 'object' &&
      Object.keys(join.inputs.fields).length === 0;
    const edgesAreValid =
      childEdges.length >= 1 &&
      joinEdges.length === childEdges.length &&
      edges.length === childEdges.length * 2 &&
      childEdges.every(
        (edge, index) =>
          edge.id === `${parallelId}:child:${index}` &&
          edge.parallelId === parallelId &&
          edge.branchId === undefined &&
          edge.condition.kind === 'always' &&
          (nodes.get(edge.to)?.handler === 'runtime.script' ||
            nodes.get(edge.to)?.handler === 'engine.durable-wait')
      ) &&
      joinEdges.every(
        (edge, index) =>
          edge.id === `${parallelId}:join:${index}` &&
          edge.from === childIds[index] &&
          edge.parallelId === parallelId &&
          edge.branchId === undefined &&
          edge.condition.kind === 'always'
      );
    const startOutgoing = graph.edges.filter(edge => edge.from === startId);
    const joinIncoming = graph.edges.filter(edge => edge.to === parallelId);
    const boundariesAreClosed =
      startOutgoing.length === childEdges.length &&
      startOutgoing.every(edge => edge.parallelId === parallelId) &&
      joinIncoming.length === joinEdges.length &&
      joinIncoming.every(edge => edge.parallelId === parallelId) &&
      childIds.every(childId => {
        const incoming = graph.edges.filter(edge => edge.to === childId);
        const outgoing = graph.edges.filter(edge => edge.from === childId);
        return (
          incoming.length === 1 &&
          incoming[0].from === startId &&
          incoming[0].parallelId === parallelId &&
          outgoing.length === 1 &&
          outgoing[0].to === parallelId &&
          outgoing[0].parallelId === parallelId
        );
      });
    const joinHasDownstream = graph.edges.some(
      edge => edge.from === parallelId
    );

    if (
      !publicStructuralIdPattern.test(parallelId) ||
      !inputsAreValid ||
      !joinIsValid ||
      !edgesAreValid ||
      !boundariesAreClosed ||
      !joinHasDownstream
    ) {
      issues.push({
        code: 'INVALID_PARALLEL_GROUP',
        message: `Parallel group "${parallelId}" does not match the frozen start, ordered child, join, policy, and concurrency contract.`,
      });
    }
  }

  for (const node of graph.nodes) {
    if (node.handler === 'engine.parallel-start') {
      const match = /^__woml_parallel__([a-z][A-Za-z0-9]*)__start$/.exec(
        node.id
      );
      if (match === null || !groups.has(match[1])) {
        issues.push({
          code: 'INVALID_PARALLEL_GROUP',
          message: `Parallel start "${node.id}" has no matching edge group.`,
        });
      }
    }
    if (node.handler === 'engine.parallel-join' && !groups.has(node.id)) {
      issues.push({
        code: 'INVALID_PARALLEL_GROUP',
        message: `Parallel join "${node.id}" has no matching edge group.`,
      });
    }
  }
}

const publicStructuralIdPattern = /^[a-z][A-Za-z0-9]*$/;

function isBranchContextReference(
  expression: ValueExpression | undefined
): expression is ContextReferenceExpression {
  if (expression?.kind !== 'contextReference') return false;
  const [root, structuralId] = expression.path;
  return (
    (root === 'trigger' && expression.path.length >= 1) ||
    (root === 'steps' &&
      structuralId !== undefined &&
      publicStructuralIdPattern.test(structuralId))
  );
}

function inspectBranchGroups(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const groups = new Map<string, CompiledWorkflowEdge[]>();
  const controlChoiceEdges = new Set(
    (graph.choices ?? []).flatMap(choice => choice.armIds)
  );

  for (const edge of graph.edges) {
    if (edge.branchId === undefined) {
      if (
        edge.condition.kind === 'boolean' &&
        !controlChoiceEdges.has(edge.id)
      ) {
        issues.push({
          code: 'INVALID_BRANCH_GROUP',
          message: `Boolean edge "${edge.id}" must carry a branchId.`,
        });
      }
      continue;
    }
    const group = groups.get(edge.branchId) ?? [];
    group.push(edge);
    groups.set(edge.branchId, group);
  }

  for (const node of graph.nodes) {
    if (node.handler === 'engine.branch-select') {
      const match = /^__woml_branch__([a-z][A-Za-z0-9]*)__select$/.exec(
        node.id
      );
      const branchId = match?.[1];
      const inputsAreEmpty =
        node.inputs.kind === 'object' &&
        Object.keys(node.inputs.fields).length === 0;
      if (branchId === undefined || !inputsAreEmpty || !groups.has(branchId)) {
        issues.push({
          code: 'INVALID_BRANCH_SELECTOR',
          message: `Branch selector "${node.id}" does not match the frozen selector identity, inputs, and edge-group contract.`,
        });
      }
    }
  }

  for (const [branchId, edges] of groups) {
    const selectorId = `__woml_branch__${branchId}__select`;
    const selector = nodes.get(selectorId);
    const selectorOutgoing = graph.edges.filter(
      edge => edge.from === selectorId
    );
    if (
      !publicStructuralIdPattern.test(branchId) ||
      selector?.handler !== 'engine.branch-select' ||
      edges.some(edge => edge.from !== selectorId) ||
      selectorOutgoing.length !== edges.length ||
      selectorOutgoing.some(edge => edge.branchId !== branchId)
    ) {
      issues.push({
        code: 'INVALID_BRANCH_GROUP',
        message: `Branch group "${branchId}" must originate from its canonical selector.`,
      });
    }

    const whenEdges = edges.slice(0, -1);
    const otherwiseEdge = edges.at(-1);
    const validWhens =
      whenEdges.length > 0 &&
      whenEdges.every(
        (edge, index) =>
          edge.id === `${branchId}:when:${index}` &&
          edge.condition.kind === 'boolean' &&
          isBranchContextReference(edge.condition.value)
      );
    const validOtherwise =
      otherwiseEdge?.id === `${branchId}:otherwise` &&
      otherwiseEdge.condition.kind === 'always';
    if (!validWhens || !validOtherwise) {
      issues.push({
        code: 'INVALID_BRANCH_GROUP',
        message: `Branch group "${branchId}" must contain contiguous ordered boolean cases followed by one fallback.`,
      });
    }

    const result = nodes.get(branchId);
    const fields = result?.inputs.kind === 'object' ? result.inputs.fields : {};
    const expectedKeys = edges.map(edge => edge.id);
    const actualKeys = Object.keys(fields);
    if (
      result?.handler !== 'engine.branch-result' ||
      actualKeys.length !== expectedKeys.length ||
      expectedKeys.some(key => !isBranchContextReference(fields[key]))
    ) {
      issues.push({
        code: 'INVALID_BRANCH_RESULT',
        message: `Branch result "${branchId}" must expose one context reference for every ordered branch arm.`,
      });
    }

    const incomingResultEdges = graph.edges.filter(
      edge => edge.to === branchId
    );
    const validJoins =
      incomingResultEdges.length === edges.length &&
      incomingResultEdges.every(
        edge => edge.branchId === undefined && edge.condition.kind === 'always'
      );
    const adjacency = new Map<string, string[]>();
    for (const edge of graph.edges) {
      const outgoing = adjacency.get(edge.from) ?? [];
      outgoing.push(edge.to);
      adjacency.set(edge.from, outgoing);
    }
    const routeNodeSets = edges.map(edge => {
      const visited = new Set<string>();
      const queue = [edge.to];
      let reachesResult = false;
      while (queue.length > 0) {
        const current = queue.shift()!;
        if (current === branchId) {
          reachesResult = true;
          continue;
        }
        if (visited.has(current)) continue;
        visited.add(current);
        queue.push(...(adjacency.get(current) ?? []));
      }
      return { reachesResult, visited };
    });
    const routesAreDisjoint = routeNodeSets.every((route, index) =>
      routeNodeSets
        .slice(index + 1)
        .every(
          other => ![...route.visited].some(nodeId => other.visited.has(nodeId))
        )
    );
    const joinsBelongToDistinctRoutes = incomingResultEdges.every(
      join =>
        routeNodeSets.filter(route => route.visited.has(join.from)).length === 1
    );
    if (
      !validJoins ||
      routeNodeSets.some(route => !route.reachesResult) ||
      !routesAreDisjoint ||
      !joinsBelongToDistinctRoutes
    ) {
      issues.push({
        code: 'INVALID_BRANCH_GROUP',
        message: `Branch group "${branchId}" must contain disjoint routes with one ordinary join into its result per arm.`,
      });
    }
  }

  for (const node of graph.nodes) {
    if (node.handler === 'engine.branch-result' && !groups.has(node.id)) {
      issues.push({
        code: 'INVALID_BRANCH_RESULT',
        message: `Branch result "${node.id}" has no matching branch edge group.`,
      });
    }
  }
}

function emptyEngineNode(
  node: CompiledWorkflowNode | undefined,
  handler: string
): boolean {
  return (
    node?.handler === handler &&
    node.timeoutMs === undefined &&
    node.retryPolicy === undefined &&
    node.scriptRuntime === undefined &&
    node.metadata === undefined &&
    node.inputs.kind === 'object' &&
    Object.keys(node.inputs.fields).length === 0
  );
}

function inspectDurableWaitGraph(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const waits = (graph as Partial<CompiledWorkflowGraphV17>).waits;
  const loops = (graph as Partial<CompiledWorkflowGraphV16>).forEach ?? [];
  const waitNodes = [
    ...graph.nodes.filter(node => node.handler === 'engine.durable-wait'),
    ...loops.flatMap(loop =>
      loop.body.nodes.filter(node => node.handler === 'engine.durable-wait')
    ),
  ];
  if (waits === undefined && waitNodes.length === 0) return;
  if (waits === undefined || waits.length !== waitNodes.length) {
    issues.push({
      code: 'INVALID_DURABLE_WAIT',
      message:
        'Model v17 waits must describe every engine.durable-wait node exactly once.',
    });
    return;
  }
  const nodeById = new Map(waitNodes.map(node => [node.id, node]));
  const seen = new Set<string>();
  for (const wait of waits) {
    const node = nodeById.get(wait.nodeId);
    const commonValid =
      wait.waitId === wait.nodeId &&
      !seen.has(wait.waitId) &&
      node !== undefined &&
      node.inputs.kind === 'object' &&
      Object.keys(node.inputs.fields).length === 0 &&
      node.timeoutMs === undefined &&
      node.retryPolicy === undefined &&
      node.scriptRuntime === undefined;
    seen.add(wait.waitId);
    let kindValid = false;
    if (wait.kind === 'duration') {
      kindValid =
        Number.isSafeInteger(wait.durationMs) &&
        wait.durationMs >= 1_000 &&
        wait.durationMs <= 31_536_000_000;
    } else if (wait.kind === 'until') {
      kindValid =
        wait.at.kind === 'contextReference' ||
        (wait.at.kind === 'literal' && typeof wait.at.value === 'string');
    } else {
      kindValid =
        wait.signals.length >= 1 &&
        wait.signals.length <= 16 &&
        new Set(wait.signals.map(signal => signal.signal)).size ===
          wait.signals.length &&
        wait.signals.every(
          signal =>
            signal.signal.length <= 128 &&
            /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/.test(signal.signal) &&
            (signal.schema === undefined ||
              (typeof signal.schema === 'object' &&
                signal.schema !== null &&
                !Array.isArray(signal.schema)))
        ) &&
        (wait.correlation.kind === 'contextReference' ||
          (wait.correlation.kind === 'literal' &&
            typeof wait.correlation.value === 'string')) &&
        (wait.timeoutMs === undefined ||
          (Number.isSafeInteger(wait.timeoutMs) &&
            wait.timeoutMs >= 1_000 &&
            wait.timeoutMs <= 31_536_000_000)) &&
        (wait.onTimeout === 'fail' || wait.onTimeout === 'continue');
    }
    if (!commonValid || !kindValid) {
      issues.push({
        code: 'INVALID_DURABLE_WAIT',
        message: `Durable wait "${wait.waitId}" does not match the frozen Model v17 contract.`,
      });
    }
  }
}

function inspectLockGraph(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const locks = (graph as Partial<CompiledWorkflowGraphV18>).locks;
  const loops = (graph as Partial<CompiledWorkflowGraphV16>).forEach ?? [];
  const allNodes = [
    ...graph.nodes,
    ...loops.flatMap(loop => loop.body.nodes),
  ];
  const allEdges = [
    ...graph.edges,
    ...loops.flatMap(loop => loop.body.edges),
  ];
  const acquireNodes = allNodes.filter(
    node => node.handler === 'engine.lock-acquire'
  );
  if (locks === undefined && acquireNodes.length === 0) return;
  if (locks === undefined || locks.length !== acquireNodes.length) {
    issues.push({
      code: 'INVALID_LOCK',
      message:
        'Model v18 locks must describe every engine.lock-acquire scope exactly once.',
    });
    return;
  }
  const nodeById = new Map(allNodes.map(node => [node.id, node]));
  const edges = new Set(allEdges.map(edge => `${edge.from}\0${edge.to}`));
  const seen = new Set<string>();
  for (const lock of locks) {
    const acquire = nodeById.get(lock.acquireNodeId);
    const release = nodeById.get(lock.releaseNodeId);
    const result = nodeById.get(lock.resultNodeId);
    const literalKeyValid =
      lock.key.kind !== 'literal' ||
      (typeof lock.key.value === 'string' &&
        lock.key.value.length > 0 &&
        new TextEncoder().encode(lock.key.value).byteLength <= 256);
    const keyValid =
      literalKeyValid &&
      (lock.key.kind === 'literal' ||
        lock.key.kind === 'contextReference' ||
        (lock.key.kind === 'template' &&
          lock.key.parts.length > 0 &&
          lock.key.parts.every(
            part => part.kind === 'text' || part.kind === 'contextReference'
          )));
    const valid =
      !seen.has(lock.lockId) &&
      lock.resultNodeId === lock.lockId &&
      emptyEngineNode(acquire, 'engine.lock-acquire') &&
      emptyEngineNode(release, 'engine.lock-release') &&
      result?.handler === 'engine.lock-result' &&
      result.inputs.kind === 'object' &&
      Object.keys(result.inputs.fields).length === 0 &&
      nodeById.has(lock.bodyEntryNodeId) &&
      nodeById.has(lock.bodyTerminalNodeId) &&
      edges.has(`${lock.acquireNodeId}\0${lock.bodyEntryNodeId}`) &&
      edges.has(`${lock.bodyTerminalNodeId}\0${lock.releaseNodeId}`) &&
      edges.has(`${lock.releaseNodeId}\0${lock.resultNodeId}`) &&
      Number.isSafeInteger(lock.acquireTimeoutMs) &&
      lock.acquireTimeoutMs >= 1_000 &&
      lock.acquireTimeoutMs <= 604_800_000 &&
      Number.isSafeInteger(lock.leaseMs) &&
      lock.leaseMs >= 5_000 &&
      lock.leaseMs <= 600_000 &&
      keyValid;
    seen.add(lock.lockId);
    if (!valid) {
      issues.push({
        code: 'INVALID_LOCK',
        message: `Lock "${lock.lockId}" does not match the frozen Model v18 contract.`,
      });
    }
  }
}

function inspectSagaGraph(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const sagas = (graph as Partial<CompiledWorkflowGraphV19>).sagas;
  const loops = (graph as Partial<CompiledWorkflowGraphV16>).forEach ?? [];
  const allNodes = [
    ...graph.nodes,
    ...loops.flatMap(loop => loop.body.nodes),
  ];
  const allEdges = [
    ...graph.edges,
    ...loops.flatMap(loop => loop.body.edges),
  ];
  const openNodes = allNodes.filter(node => node.handler === 'engine.saga-open');
  if (sagas === undefined && openNodes.length === 0) return;
  if (sagas === undefined || sagas.length !== openNodes.length) {
    issues.push({
      code: 'INVALID_SAGA',
      message:
        'Model v19 sagas must describe every engine.saga-open scope exactly once.',
    });
    return;
  }

  const nodeById = new Map(allNodes.map(node => [node.id, node]));
  const edges = new Set(allEdges.map(edge => `${edge.from}\0${edge.to}`));
  const seenSagas = new Set<string>();
  const seenActions = new Set<string>();
  for (const saga of sagas) {
    const open = nodeById.get(saga.openNodeId);
    const commit = nodeById.get(saga.commitNodeId);
    const result = nodeById.get(saga.resultNodeId);
    const actions = [...saga.compensations, ...saga.cleanup];
    const actionsValid = actions.every(action => {
      const identityValid =
        action.identity.kind === 'static'
          ? action.identity.actionId === action.actionId
          : publicStructuralIdPattern.test(action.identity.forEachId) &&
            action.identity.actionIdTemplate.includes('{iterationKey}');
      const nodeValid =
        action.node.id === action.actionId &&
        action.node.handler === 'runtime.script' &&
        action.node.inputs.kind === 'object' &&
        action.node.inputs.fields.source?.kind === 'literal' &&
        typeof action.node.inputs.fields.source.value === 'string';
      const unique = !seenActions.has(action.actionId);
      seenActions.add(action.actionId);
      return unique && identityValid && nodeValid;
    });
    const compensationsValid = saga.compensations.every(
      compensation =>
        publicStructuralIdPattern.test(compensation.forStepId) &&
        nodeById.has(compensation.forStepId)
    );
    const valid =
      !seenSagas.has(saga.sagaId) &&
      saga.resultNodeId === saga.sagaId &&
      emptyEngineNode(open, 'engine.saga-open') &&
      emptyEngineNode(commit, 'engine.saga-commit') &&
      result?.handler === 'engine.saga-result' &&
      result.inputs.kind === 'object' &&
      Object.keys(result.inputs.fields).length === 0 &&
      nodeById.has(saga.bodyEntryNodeId) &&
      nodeById.has(saga.bodyTerminalNodeId) &&
      nodeById.has(saga.bodyResultNodeId) &&
      edges.has(`${saga.openNodeId}\0${saga.bodyEntryNodeId}`) &&
      edges.has(`${saga.bodyTerminalNodeId}\0${saga.commitNodeId}`) &&
      edges.has(`${saga.commitNodeId}\0${saga.resultNodeId}`) &&
      actionsValid &&
      compensationsValid;
    seenSagas.add(saga.sagaId);
    if (!valid) {
      issues.push({
        code: 'INVALID_SAGA',
        message: `Saga "${saga.sagaId}" does not match the frozen Model v19 contract.`,
      });
    }
  }
}

function inspectForkGraph(
  graph: CompiledWorkflowGraph,
  issues: CompiledGraphIssue[]
): void {
  const forks = graph.forks;
  const choices = graph.choices;
  const visibility = graph.contextVisibility;
  const settlement = graph.settlement;
  const hasV13Node = graph.nodes.some(node =>
    [
      'engine.fork-open',
      'engine.fork-branch-terminal',
      'engine.fork-join',
      'engine.choice-select',
      'engine.choice-join',
      'engine.workflow-settlement',
    ].includes(node.handler)
  );
  const hasAnyDescriptor =
    forks !== undefined ||
    choices !== undefined ||
    visibility !== undefined ||
    settlement !== undefined;
  if (!hasV13Node && !hasAnyDescriptor) return;
  if (
    forks === undefined ||
    choices === undefined ||
    visibility === undefined ||
    settlement === undefined
  ) {
    issues.push({
      code: 'INVALID_FORK_GRAPH',
      message:
        'Model v13 graph metadata requires forks, choices, contextVisibility, and settlement together.',
    });
    return;
  }

  const nodes = new Map(graph.nodes.map(node => [node.id, node]));
  const outgoing = new Map<string, CompiledWorkflowEdge[]>();
  const incoming = new Map<string, CompiledWorkflowEdge[]>();
  for (const node of graph.nodes) {
    outgoing.set(node.id, []);
    incoming.set(node.id, []);
  }
  for (const edge of graph.edges) {
    outgoing.get(edge.from)?.push(edge);
    incoming.get(edge.to)?.push(edge);
  }

  const seenForkIds = new Set<string>();
  const ownedTerminalIds: string[] = [];
  const describedForkNodes = new Set<string>();
  const routeOwners = new Map<string, string>();
  for (const fork of forks) {
    let valid =
      publicStructuralIdPattern.test(fork.forkId) &&
      !seenForkIds.has(fork.forkId) &&
      fork.openNodeId === `__woml_fork__${fork.forkId}__open` &&
      fork.joinNodeId === `__woml_fork__${fork.forkId}__join` &&
      emptyEngineNode(nodes.get(fork.openNodeId), 'engine.fork-open') &&
      emptyEngineNode(nodes.get(fork.joinNodeId), 'engine.fork-join') &&
      fork.branches.length >= 1;
    seenForkIds.add(fork.forkId);
    describedForkNodes.add(fork.openNodeId);
    describedForkNodes.add(fork.joinNodeId);

    const branchIds = new Set<string>();
    const terminalByBranch = new Map<string, string>();
    for (const branch of fork.branches) {
      const expectedTerminal = `__woml_fork__${fork.forkId}__${branch.branchId}__terminal`;
      valid &&=
        publicStructuralIdPattern.test(branch.branchId) &&
        branch.branchId !== 'all' &&
        branch.branchId !== 'none' &&
        !branchIds.has(branch.branchId) &&
        branch.terminalNodeId === expectedTerminal &&
        nodes.has(branch.entryNodeId) &&
        emptyEngineNode(
          nodes.get(branch.terminalNodeId),
          'engine.fork-branch-terminal'
        );
      branchIds.add(branch.branchId);
      terminalByBranch.set(branch.branchId, branch.terminalNodeId);
      ownedTerminalIds.push(branch.terminalNodeId);
      describedForkNodes.add(branch.terminalNodeId);

      const entryIncoming = incoming.get(branch.entryNodeId) ?? [];
      valid &&=
        entryIncoming.length === 1 &&
        entryIncoming[0].from === fork.openNodeId &&
        entryIncoming[0].condition.kind === 'always';

      const pending = [branch.entryNodeId];
      const visited = new Set<string>();
      let reachedTerminal = false;
      while (pending.length > 0) {
        const nodeId = pending.pop()!;
        if (visited.has(nodeId)) continue;
        visited.add(nodeId);
        if (nodeId === branch.terminalNodeId) {
          reachedTerminal = true;
          continue;
        }
        for (const edge of outgoing.get(nodeId) ?? []) {
          if (edge.to !== fork.joinNodeId && edge.to !== settlement.nodeId) {
            pending.push(edge.to);
          }
        }
      }
      valid &&= reachedTerminal;
      for (const nodeId of visited) {
        const owner = routeOwners.get(nodeId);
        if (
          owner !== undefined &&
          owner !== `${fork.forkId}:${branch.branchId}`
        ) {
          valid = false;
        }
        routeOwners.set(nodeId, `${fork.forkId}:${branch.branchId}`);
      }
    }

    const canonicalJoined = fork.branches
      .filter(branch => fork.joinedBranchIds.includes(branch.branchId))
      .map(branch => branch.branchId);
    valid &&=
      new Set(fork.joinedBranchIds).size === fork.joinedBranchIds.length &&
      fork.joinedBranchIds.every(id => branchIds.has(id)) &&
      JSON.stringify(canonicalJoined) === JSON.stringify(fork.joinedBranchIds);

    const openEdges = outgoing.get(fork.openNodeId) ?? [];
    const branchEntryIds = new Set(
      fork.branches.map(branch => branch.entryNodeId)
    );
    valid &&= fork.branches.every(branch =>
      openEdges.some(
        edge =>
          edge.to === branch.entryNodeId && edge.condition.kind === 'always'
      )
    );
    const joinSources = (incoming.get(fork.joinNodeId) ?? []).map(
      edge => edge.from
    );
    const expectedJoinSources =
      fork.joinedBranchIds.length === 0
        ? [fork.openNodeId]
        : fork.joinedBranchIds.map(id => terminalByBranch.get(id)!);
    valid &&=
      openEdges.every(
        edge =>
          branchEntryIds.has(edge.to) ||
          (fork.joinedBranchIds.length === 0 && edge.to === fork.joinNodeId)
      ) &&
      JSON.stringify(joinSources) === JSON.stringify(expectedJoinSources) &&
      (incoming.get(fork.joinNodeId) ?? []).every(
        edge => edge.condition.kind === 'always'
      );

    if (!valid) {
      issues.push({
        code: 'INVALID_FORK_GRAPH',
        message: `Fork "${fork.forkId}" does not match the frozen Model v13 ownership, route, and join contract.`,
      });
    }
  }

  for (const node of graph.nodes) {
    if (
      [
        'engine.fork-open',
        'engine.fork-branch-terminal',
        'engine.fork-join',
      ].includes(node.handler) &&
      !describedForkNodes.has(node.id)
    ) {
      issues.push({
        code: 'INVALID_FORK_GRAPH',
        message: `Fork control node "${node.id}" is not owned by a fork descriptor.`,
      });
    }
  }

  const seenChoices = new Set<string>();
  const describedChoiceNodes = new Set<string>();
  for (const choice of choices) {
    const isStringChoice = choice.stringSelector !== undefined;
    const stringFieldsAreComplete = isStringChoice
      ? choice.stringCases !== undefined &&
        choice.defaultArmId !== undefined &&
        choice.stringCases.length >= 1 &&
        choice.stringCases.length === choice.armIds.length - 1 &&
        isBranchContextReference(choice.stringSelector) &&
        new Set(choice.stringCases.map(item => item.value)).size ===
          choice.stringCases.length &&
        choice.stringCases.every(
          (item, index) =>
            item.armId === choice.armIds[index] && item.value.length > 0
        ) &&
        choice.defaultArmId === choice.armIds.at(-1)
      : choice.stringCases === undefined &&
        choice.defaultArmId === undefined &&
        choice.resultNodeId === undefined;
    const validChoiceId = /^__woml_choice__[a-z0-9_]+$/.test(choice.choiceId);
    let valid =
      validChoiceId &&
      !seenChoices.has(choice.choiceId) &&
      choice.selectorNodeId === `${choice.choiceId}__select` &&
      choice.joinNodeId === `${choice.choiceId}__join` &&
      emptyEngineNode(
        nodes.get(choice.selectorNodeId),
        'engine.choice-select'
      ) &&
      emptyEngineNode(nodes.get(choice.joinNodeId), 'engine.choice-join') &&
      choice.armIds.length >= 2 &&
      new Set(choice.armIds).size === choice.armIds.length &&
      stringFieldsAreComplete;
    seenChoices.add(choice.choiceId);
    describedChoiceNodes.add(choice.selectorNodeId);
    describedChoiceNodes.add(choice.joinNodeId);
    if (choice.resultNodeId !== undefined) {
      describedChoiceNodes.add(choice.resultNodeId);
      const result = nodes.get(choice.resultNodeId);
      const fields =
        result?.inputs.kind === 'object' ? result.inputs.fields : {};
      const resultIncoming = incoming.get(choice.resultNodeId) ?? [];
      valid &&=
        isStringChoice &&
        publicStructuralIdPattern.test(choice.resultNodeId) &&
        result?.handler === 'engine.choice-result' &&
        result.retryPolicy === undefined &&
        result.timeoutMs === undefined &&
        Object.keys(fields).length === choice.armIds.length &&
        choice.armIds.every(armId => isBranchContextReference(fields[armId])) &&
        resultIncoming.length === 1 &&
        resultIncoming[0].from === choice.joinNodeId &&
        resultIncoming[0].condition.kind === 'always';
    }
    const selectionEdges = outgoing.get(choice.selectorNodeId) ?? [];
    const joinEdges = incoming.get(choice.joinNodeId) ?? [];
    valid &&=
      selectionEdges.length === choice.armIds.length &&
      joinEdges.length === choice.armIds.length;
    choice.armIds.forEach((armId, index) => {
      const select = selectionEdges[index];
      const join = joinEdges[index];
      valid &&=
        armId.startsWith(`${choice.choiceId}:`) &&
        select?.id === armId &&
        select.branchId === undefined &&
        select.parallelId === undefined &&
        select.approvalId === undefined &&
        (isStringChoice
          ? select.condition.kind === 'always'
          : index === choice.armIds.length - 1
            ? select.condition.kind === 'always'
            : select.condition.kind === 'boolean') &&
        join?.id === `${armId}:join` &&
        join.to === choice.joinNodeId &&
        join.condition.kind === 'always';
    });
    if (!valid) {
      issues.push({
        code: 'INVALID_CONTROL_CHOICE',
        message: `Control choice "${choice.choiceId}" does not match the frozen selector, ordered-arm, and join contract.`,
      });
    }
  }
  for (const node of graph.nodes) {
    if (
      [
        'engine.choice-select',
        'engine.choice-join',
        'engine.choice-result',
      ].includes(node.handler) &&
      !describedChoiceNodes.has(node.id)
    ) {
      issues.push({
        code: 'INVALID_CONTROL_CHOICE',
        message: `Control-choice node "${node.id}" has no matching descriptor.`,
      });
    }
  }

  const runtimeScriptIds = graph.nodes
    .filter(node => node.handler === 'runtime.script')
    .map(node => node.id);
  const visibleNodeIds = new Set<string>();
  let visibilityValid = visibility.length === runtimeScriptIds.length;
  visibility.forEach((item, index) => {
    visibilityValid &&=
      item.nodeId === runtimeScriptIds[index] &&
      !visibleNodeIds.has(item.nodeId) &&
      new Set(item.stepIds).size === item.stepIds.length &&
      item.stepIds.every(
        id => publicStructuralIdPattern.test(id) && nodes.has(id)
      );
    visibleNodeIds.add(item.nodeId);
  });
  if (!visibilityValid) {
    issues.push({
      code: 'INVALID_CONTEXT_VISIBILITY',
      message:
        'Model v13 contextVisibility must describe every runtime.script node once, in graph order, with existing public output IDs.',
    });
  }

  const settlementIncoming = incoming.get(settlement.nodeId) ?? [];
  const ownedInDocumentOrder = forks.flatMap(fork =>
    fork.branches.map(branch => branch.terminalNodeId)
  );
  const ownedSet = new Set(ownedInDocumentOrder);
  const nonBranchSettlementSources = settlementIncoming.filter(
    edge => !ownedSet.has(edge.from)
  );
  const settlementValid =
    settlement.nodeId === '__woml_workflow__settlement' &&
    emptyEngineNode(
      nodes.get(settlement.nodeId),
      'engine.workflow-settlement'
    ) &&
    nodes.has(settlement.mainResultNodeId) &&
    JSON.stringify(settlement.ownedBranchTerminalNodeIds) ===
      JSON.stringify(ownedInDocumentOrder) &&
    nonBranchSettlementSources.length === 1 &&
    settlementIncoming.length === ownedInDocumentOrder.length + 1 &&
    settlementIncoming.every(edge => edge.condition.kind === 'always') &&
    (outgoing.get(settlement.nodeId)?.length ?? 0) === 0;
  if (!settlementValid) {
    issues.push({
      code: 'INVALID_WORKFLOW_SETTLEMENT',
      message:
        'Model v13 settlement must preserve one main result and depend on the main continuation plus every owned branch terminal.',
    });
  }
}
