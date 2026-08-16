import Ajv2020 from 'ajv/dist/2020';
import { readFileSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';

import {
  inspectCompiledWorkflowGraph,
  type CompiledTrigger,
  type CompiledLifecycleActionV1,
  type CompiledLifecycleDefinitionV1,
  type CompiledLifecycleHookV1,
  type CompiledModuleRuntimeV1,
  type CompiledRuntimePolicyV1,
  type CompiledReusableNotificationProviderV1,
  type CompiledReusableStepInvocationV1,
  type CompiledControlChoiceV1,
  type CompiledContextVisibilityV1,
  type CompiledForkV1,
  type CompiledWorkflowDefinition,
  type CompiledWorkflowDefinitionV9,
  type CompiledWorkflowDefinitionV10,
  type CompiledWorkflowDefinitionV11,
  type CompiledWorkflowDefinitionV12,
  type CompiledWorkflowDefinitionV13,
  type CompiledWorkflowDefinitionV14,
  type CompiledWorkflowDefinitionV15,
  type CompiledWorkflowEdge,
  type CompiledWorkflowGraph,
  type CompiledWorkflowGraphV13,
  type CompiledWorkflowMetadata,
  type CompiledWorkflowNode,
  type ContextReferenceExpression,
  type JsonValue,
  type LifecycleEventName,
  type LifecycleReferenceExpression,
  type RetryPolicy,
  type ScriptRuntimeBindingsV1,
  type ScriptRuntimeBindingsV2,
  type SecretReferenceExpression,
  type ValueExpression,
} from './model';
import {
  analyzeWomlLifecycleScript,
  analyzeWomlNotificationProviderScript,
  analyzeWomlReusableScript,
  analyzeWomlScript,
  type ScriptAnalysis,
} from './script-analysis';
import { parseWoml } from './parser';
import {
  SourceFile,
  WomlCompileError,
  WomlValidationError,
  type SourceSpan,
  type WomlDiagnostic,
  type WomlAdvisoryDiagnostic,
  type WomlSourceAttribute,
  type WomlSourceDocument,
  type WomlSourceElement,
  type WomlSourceRawText,
} from './source';
import {
  isSupportedScheduleTimeZone,
  parseScheduleCron,
  ScheduleCronSyntaxError,
} from './schedule';
import { parseSecretReference, requireSecretReference } from './secrets';
import {
  assertWomlDocumentRunnable,
  inspectWomlDocument,
  type WomlDocumentInspection,
  type WomlReusableDefinitionGraph,
} from './reusable-definitions';

interface ElementProfile {
  readonly attributes: ReadonlySet<string>;
  readonly stagedAttributes?: ReadonlySet<string>;
}

interface ValidatedStep {
  readonly kind: 'step';
  readonly id: string;
  readonly source: string;
  readonly scriptSpan: SourceSpan;
  readonly scriptAnalysis: ScriptAnalysis;
  readonly retryPolicy?: RetryPolicy;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

interface ValidatedReference {
  readonly path: readonly string[];
  readonly structuralId?: string;
  readonly span: SourceSpan;
}

interface ValidatedBranchArm {
  readonly kind: 'when' | 'otherwise';
  readonly element: WomlSourceElement;
  readonly test?: ValidatedReference;
  readonly items: readonly ValidatedFlowItem[];
  readonly result: ValidatedReference;
}

interface ValidatedControlChoiceArm {
  readonly kind: 'when' | 'otherwise';
  readonly element: WomlSourceElement;
  readonly test?: ValidatedReference;
  readonly items: readonly ValidatedFlowItem[];
}

interface ValidatedControlChoice {
  readonly kind: 'controlChoice';
  readonly element: WomlSourceElement;
  readonly arms: readonly ValidatedControlChoiceArm[];
}

interface ValidatedSwitchArm {
  readonly kind: 'case' | 'default';
  readonly element: WomlSourceElement;
  readonly value?: string;
  readonly items: readonly ValidatedFlowItem[];
  readonly result?: ValidatedReference;
}

interface ValidatedSwitch {
  readonly kind: 'switch';
  readonly id?: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly selector: ValidatedReference;
  readonly arms: readonly ValidatedSwitchArm[];
}

interface ValidatedBranch {
  readonly kind: 'branch';
  readonly sourceElementName: 'branch' | 'choose';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly arms: readonly ValidatedBranchArm[];
}

interface ValidatedParallel {
  readonly kind: 'parallel';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly concurrency: number;
  readonly onError: 'fail-fast' | 'wait-all';
  readonly children: readonly ValidatedStep[];
}

interface ValidatedApproval {
  readonly kind: 'approval';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly timeoutMs?: number;
  readonly onTimeout: 'reject' | 'fail';
  readonly notifications: readonly ValidatedNotificationDelivery[];
  readonly approvedItems: readonly ValidatedFlowItem[];
  readonly rejectedItems: readonly ValidatedFlowItem[];
}

interface ValidatedForkBranch {
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly items: readonly ValidatedFlowItem[];
}

interface ValidatedFork {
  readonly kind: 'fork';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly branches: readonly ValidatedForkBranch[];
  readonly joinedBranchIds: readonly string[];
}

interface ValidatedSlackNotificationDelivery {
  readonly deliveryId: string;
  readonly provider: 'slack';
  readonly destination: string;
  readonly botToken: SecretReferenceExpression;
  readonly appToken: SecretReferenceExpression;
  readonly message?: ValueExpression;
}

interface ValidatedTelegramNotificationDelivery {
  readonly deliveryId: string;
  readonly provider: 'telegram';
  readonly destination: string;
  readonly botToken: SecretReferenceExpression;
  readonly message?: ValueExpression;
}

type ValidatedNotificationDelivery =
  | ValidatedSlackNotificationDelivery
  | ValidatedTelegramNotificationDelivery;

type ValidatedFlowItem =
  | ValidatedStep
  | ValidatedBranch
  | ValidatedControlChoice
  | ValidatedSwitch
  | ValidatedParallel
  | ValidatedApproval
  | ValidatedFork;

interface ValidatedFlow {
  readonly items: readonly ValidatedFlowItem[];
  readonly firstBranch?: WomlSourceElement;
  readonly firstParallel?: WomlSourceElement;
  readonly firstApproval?: WomlSourceElement;
  readonly firstNotification?: WomlSourceElement;
  readonly firstFork?: WomlSourceElement;
  readonly firstControlChoice?: WomlSourceElement;
  readonly firstSwitch?: WomlSourceElement;
}

interface FlowValidationContext {
  readonly insideForkBranch: boolean;
  readonly shadowedServices: readonly string[];
}

const rootFlowValidationContext: FlowValidationContext = {
  insideForkBranch: false,
  shadowedServices: [],
};

interface ValidatedLifecycleScriptAction {
  readonly kind: 'script';
  readonly source: string;
  readonly scriptSpan: SourceSpan;
  readonly scriptAnalysis: ScriptAnalysis;
}

interface ValidatedInformationalNotification {
  readonly kind: 'notify';
  readonly deliveries: readonly ValidatedNotificationDelivery[];
}

type ValidatedLifecycleAction =
  | ValidatedLifecycleScriptAction
  | ValidatedInformationalNotification;

interface ValidatedLifecycleHook {
  readonly event: LifecycleEventName;
  readonly stepIds?: readonly string[];
  readonly actions: readonly ValidatedLifecycleAction[];
}

interface ValidatedLifecycle {
  readonly hooks: readonly ValidatedLifecycleHook[];
}

interface ValidatedRuntimePolicy {
  readonly value: CompiledRuntimePolicyV1;
  readonly element: WomlSourceElement;
}

interface ValidatedWorkflow {
  readonly element: WomlSourceElement;
  readonly modules: readonly ValidatedModuleDeclaration[];
  readonly workflowId: string;
  readonly metadata?: CompiledWorkflowMetadata;
  readonly triggers: readonly ValidatedTrigger[];
  readonly flow: ValidatedFlow;
  readonly lifecycle?: ValidatedLifecycle;
  readonly runtimePolicy?: ValidatedRuntimePolicy;
}

export interface ValidatedModuleDeclaration {
  readonly name: string;
  readonly from: string;
  readonly element: WomlSourceElement;
  readonly nameAttribute: WomlSourceAttribute;
  readonly fromAttribute: WomlSourceAttribute;
}

interface ValidatedManualTrigger {
  readonly kind: 'manual';
  readonly id: string;
}

interface ValidatedWebhookTrigger {
  readonly kind: 'webhook';
  readonly id: string;
  readonly path: string;
  readonly method: 'POST';
  readonly authentication:
    | { readonly kind: 'none' }
    | {
        readonly kind: 'bearer';
        readonly secret: SecretReferenceExpression;
      };
  readonly schema?: Readonly<Record<string, JsonValue>>;
}

type SlackTriggerEvent = 'app-mention' | 'direct-message';

interface ValidatedSlackTrigger {
  readonly kind: 'slack';
  readonly id: string;
  readonly events: readonly SlackTriggerEvent[];
  readonly channels: readonly string[];
  readonly botToken: SecretReferenceExpression;
  readonly appToken: SecretReferenceExpression;
}

interface ValidatedTelegramTrigger {
  readonly kind: 'telegram';
  readonly id: string;
  readonly events: readonly ['message'];
  readonly botToken: SecretReferenceExpression;
}

interface ValidatedScheduleTrigger {
  readonly kind: 'schedule';
  readonly id: string;
  readonly cron: string;
  readonly timezone: string;
  readonly onMissed: 'skip' | 'run-once';
}

interface ValidatedIntervalTrigger {
  readonly kind: 'interval';
  readonly id: string;
  readonly everyMs: number;
  readonly onMissed: 'skip' | 'run-once';
}

interface ValidatedEventTrigger {
  readonly kind: 'event';
  readonly id: string;
  readonly name: string;
  readonly secret?: SecretReferenceExpression;
  readonly schema?: Readonly<Record<string, JsonValue>>;
}

type ValidatedTrigger =
  | ValidatedManualTrigger
  | ValidatedWebhookTrigger
  | ValidatedSlackTrigger
  | ValidatedTelegramTrigger
  | ValidatedScheduleTrigger
  | ValidatedIntervalTrigger
  | ValidatedEventTrigger;

interface LoweredFlowFragment {
  readonly entryId: string;
  readonly exitId: string;
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
}

interface LoweredV13FlowFragment extends LoweredFlowFragment {
  readonly visibleAfter: ReadonlySet<string>;
  readonly lastResultNodeId?: string;
}

interface V13LoweringState {
  readonly forks: CompiledForkV1[];
  readonly choices: CompiledControlChoiceV1[];
  readonly contextVisibility: CompiledContextVisibilityV1[];
  readonly ownedBranchTerminalNodeIds: string[];
}

const supportedElements = new Set([
  'woml',
  'imports',
  'module',
  'workflow',
  'config',
  'lifecycle',
  'on-start',
  'on-step-start',
  'on-step-success',
  'on-step-failure',
  'on-step-complete',
  'on-success',
  'on-error',
  'on-cancel',
  'on-complete',
  'triggers',
  'manual',
  'webhook',
  'schema',
  'steps',
  'step',
  'script',
  'branch',
  'choose',
  'switch',
  'case',
  'default',
  'fork',
  'parallel',
  'when',
  'otherwise',
  'result',
  'approval',
  'notify',
  'slack',
  'telegram',
  'schedule',
  'interval',
  'event',
  'when-approved',
  'when-rejected',
]);

const stagedElements = new Set<string>();

const elementProfiles: Readonly<Record<string, ElementProfile>> = {
  woml: { attributes: new Set() },
  imports: { attributes: new Set() },
  module: { attributes: new Set(['name', 'from']) },
  workflow: {
    attributes: new Set(['id', 'name', 'description', 'version']),
    stagedAttributes: new Set(['tags']),
  },
  config: {
    attributes: new Set(['concurrency', 'timeout', 'rate-limit', 'queue']),
  },
  lifecycle: { attributes: new Set() },
  'on-start': { attributes: new Set() },
  'on-step-start': { attributes: new Set(['steps']) },
  'on-step-success': { attributes: new Set(['steps']) },
  'on-step-failure': { attributes: new Set(['steps']) },
  'on-step-complete': { attributes: new Set(['steps']) },
  'on-success': { attributes: new Set() },
  'on-error': { attributes: new Set() },
  'on-cancel': { attributes: new Set() },
  'on-complete': { attributes: new Set() },
  triggers: { attributes: new Set() },
  manual: { attributes: new Set(['id']) },
  webhook: {
    attributes: new Set(['id', 'path', 'method', 'auth', 'secret']),
  },
  schema: { attributes: new Set() },
  steps: { attributes: new Set() },
  step: {
    attributes: new Set([
      'id',
      'name',
      'description',
      'retry',
      'retry-backoff',
      'retry-delay',
      'retry-max-delay',
    ]),
    stagedAttributes: new Set(['timeout']),
  },
  script: { attributes: new Set() },
  branch: { attributes: new Set(['id', 'name', 'description']) },
  choose: { attributes: new Set(['id', 'name', 'description']) },
  switch: { attributes: new Set(['id', 'name', 'description', 'value']) },
  case: { attributes: new Set(['value']) },
  default: { attributes: new Set() },
  fork: { attributes: new Set(['id', 'join']) },
  parallel: {
    attributes: new Set([
      'id',
      'name',
      'description',
      'concurrency',
      'on-error',
    ]),
  },
  when: { attributes: new Set(['test']) },
  otherwise: { attributes: new Set() },
  result: { attributes: new Set(['value']) },
  approval: {
    attributes: new Set(['id', 'name', 'description', 'timeout', 'on-timeout']),
  },
  notify: { attributes: new Set() },
  slack: {
    attributes: new Set([
      'id',
      'events',
      'channels',
      'message',
      'bot-token',
      'app-token',
    ]),
  },
  telegram: {
    attributes: new Set(['id', 'events', 'chats', 'message', 'bot-token']),
  },
  schedule: {
    attributes: new Set(['id', 'cron', 'timezone', 'on-missed']),
  },
  interval: {
    attributes: new Set(['id', 'every', 'on-missed']),
  },
  event: { attributes: new Set(['id', 'name', 'secret']) },
  'when-approved': { attributes: new Set() },
  'when-rejected': { attributes: new Set() },
};

const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const javascriptSafeIdPattern = /^[a-z][A-Za-z0-9]*$/;
const webhookPathPattern = /^\/(?:[A-Za-z0-9._~-]+(?:\/[A-Za-z0-9._~-]+)*)?$/;
const eventNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/;
const moduleAliasPattern = /^[a-z][A-Za-z0-9]*$/;
const moduleSourcePattern =
  /^(?:\.\/|\.\.\/)(?:[A-Za-z0-9._-]+\/)*[A-Za-z0-9._-]+\.(?:js|ts)$/;
const queueNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;
const artifactDigestPattern = /^sha256:[0-9a-f]{64}$/;
const runtimeExportPattern = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const maximumWorkflowConcurrency = 1_000_000;
const maximumWorkflowRateCount = 1_000_000;
const maximumWorkflowPolicyDurationMs = 365 * 24 * 60 * 60 * 1000;
const reservedModuleAliases = new Set([
  'http',
  'db',
  'storage',
  'cache',
  'events',
  'queue',
  'workflows',
  'state',
  'context',
  'attempt',
  'services',
  'secrets',
]);

function diagnostic(
  document: WomlSourceDocument,
  phase: 'validation' | 'compile',
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): WomlDiagnostic {
  return {
    code,
    phase,
    message,
    file: document.file,
    location: span,
    ...(hint === undefined ? {} : { hint }),
  };
}

function failValidation(
  document: WomlSourceDocument,
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): never {
  throw new WomlValidationError(
    diagnostic(document, 'validation', code, message, span, hint)
  );
}

function failCompile(
  document: WomlSourceDocument,
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): never {
  throw new WomlCompileError(
    diagnostic(document, 'compile', code, message, span, hint)
  );
}

function visitProfile(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  parent?: WomlSourceElement
): void {
  if (!supportedElements.has(element.name)) {
    if (parent?.name === 'lifecycle' && element.name === 'on-failure') {
      failValidation(
        document,
        'WOML_LIFECYCLE_HOOK_INVALID',
        'WOML lifecycle uses <on-error>, not <on-failure>.',
        element.openTagSpan,
        'Replace <on-failure> with <on-error>; the same hook name is used in workflows and reusable definitions.'
      );
    }
    if (parent?.name === 'notify') {
      failValidation(
        document,
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
        `<notify> supports built-in <slack>, <telegram>, and imported custom providers; found <${element.name}>.`,
        element.openTagSpan
      );
    }
    if (stagedElements.has(element.name)) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `<${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        element.openTagSpan
      );
    }
    failValidation(
      document,
      'WOML_UNKNOWN_ELEMENT',
      `Unknown WOML element <${element.name}>.`,
      element.openTagSpan
    );
  }

  const profile = elementProfiles[element.name];
  for (const attribute of Object.values(element.attributes)) {
    if (profile.stagedAttributes?.has(attribute.name) === true) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `Attribute "${attribute.name}" on <${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        attribute.nameSpan
      );
    }
    if (!profile.attributes.has(attribute.name)) {
      const isLifecycleStepFilterMisuse =
        attribute.name === 'steps' &&
        element.name.startsWith('on-') &&
        !element.name.startsWith('on-step-');
      failValidation(
        document,
        isLifecycleStepFilterMisuse
          ? 'WOML_LIFECYCLE_STEP_FILTER_INVALID'
          : attribute.name === 'retry' || attribute.name.startsWith('retry-')
            ? 'WOML_RETRY_HANDLER_UNSUPPORTED'
            : element.name === 'slack'
              ? 'WOML_SLACK_UNKNOWN_ATTRIBUTE'
              : element.name === 'telegram'
                ? 'WOML_TELEGRAM_UNKNOWN_ATTRIBUTE'
              : element.name === 'config'
                ? 'WOML_CONFIG_ATTRIBUTE_UNKNOWN'
                : 'WOML_UNKNOWN_ATTRIBUTE',
        isLifecycleStepFilterMisuse
          ? 'Attribute "steps" is valid only on step lifecycle hooks.'
          : attribute.name === 'retry' || attribute.name.startsWith('retry-')
            ? `Retry attributes are valid only on <step>; found "${attribute.name}" on <${element.name}>.`
            : `Unknown attribute "${attribute.name}" on <${element.name}>.`,
        attribute.nameSpan,
        attribute.name === 'retry' || attribute.name.startsWith('retry-')
          ? 'Move the retry policy to a script-bearing <step>.'
          : undefined
      );
    }
  }

  for (const child of element.children) {
    if (child.kind === 'element') visitProfile(document, child, element);
  }
}

function validateSecretReferenceSinks(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  for (const attribute of Object.values(element.attributes)) {
    const isSlackCredential =
      element.name === 'slack' &&
      (attribute.name === 'bot-token' || attribute.name === 'app-token');
    const isTelegramCredential =
      element.name === 'telegram' && attribute.name === 'bot-token';
    const isWebhookCredential =
      element.name === 'webhook' && attribute.name === 'secret';
    const isEventCredential =
      element.name === 'event' && attribute.name === 'secret';
    if (
      isSlackCredential ||
      isTelegramCredential ||
      isWebhookCredential ||
      isEventCredential
    ) {
      requireSecretReference(document, attribute);
      continue;
    }

    if (
      parseSecretReference(attribute.value) !== undefined ||
      attribute.value.includes('{{secrets.')
    ) {
      failValidation(
        document,
        'WOML_SECRET_SINK_UNSUPPORTED',
        'Secret references are allowed only in reviewed secret-bearing attributes. Direct script access and arbitrary attribute interpolation are unavailable.',
        attribute.valueSpan
      );
    }
  }

  for (const child of element.children) {
    if (child.kind === 'element') {
      validateSecretReferenceSinks(document, child);
    }
  }
}

function requiredAttribute(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: string
): WomlSourceAttribute {
  const attribute = element.attributes[name];
  if (attribute === undefined) {
    failValidation(
      document,
      'WOML_MISSING_ATTRIBUTE',
      `<${element.name}> requires the "${name}" attribute.`,
      element.openTagSpan
    );
  }
  return attribute;
}

function elementChildren(
  document: WomlSourceDocument,
  parent: WomlSourceElement
): WomlSourceElement[] {
  const elements: WomlSourceElement[] = [];
  for (const child of parent.children) {
    if (child.kind !== 'element') {
      failValidation(
        document,
        'WOML_UNEXPECTED_CONTENT',
        `<${parent.name}> may contain WOML elements only.`,
        child.span
      );
    }
    elements.push(child);
  }
  return elements;
}

function ensureEmptyElement(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  if (element.children.length > 0) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<${element.name}> cannot contain child content in the first WOML CLI profile.`,
      element.children[0].span
    );
  }
}

function validateWorkflowId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): string {
  if (
    attribute.value.length > 256 ||
    !workflowIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `Workflow ID "${attribute.value}" must use lowercase kebab-case.`,
      attribute.valueSpan,
      'Example: content-moderator'
    );
  }
  return attribute.value;
}

function validateJavaScriptSafeId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  role:
    | 'trigger'
    | 'step'
    | 'branch'
    | 'choose'
    | 'switch'
    | 'fork'
    | 'parallel'
    | 'approval'
): string {
  if (
    attribute.value.length > 256 ||
    !javascriptSafeIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `${role === 'trigger' ? 'Trigger' : role === 'branch' ? 'Branch' : role === 'choose' ? 'Choice' : role === 'switch' ? 'Switch' : role === 'fork' ? 'Fork' : role === 'parallel' ? 'Parallel' : role === 'approval' ? 'Approval' : 'Step'} ID "${attribute.value}" must be a JavaScript-safe lower-camel identifier.`,
      attribute.valueSpan,
      'Use letters and numbers, start with a lowercase letter, and do not use hyphens.'
    );
  }
  return attribute.value;
}

function optionalMetadataValue(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: 'name' | 'description' | 'version'
): string | undefined {
  const attribute = element.attributes[name];
  if (attribute === undefined) return undefined;
  if (attribute.value.trim().length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_METADATA',
      `Attribute "${name}" on <${element.name}> must not be empty.`,
      attribute.valueSpan
    );
  }
  return attribute.value;
}

function workflowMetadata(
  document: WomlSourceDocument,
  workflow: WomlSourceElement
): CompiledWorkflowMetadata | undefined {
  const name = optionalMetadataValue(document, workflow, 'name');
  const description = optionalMetadataValue(document, workflow, 'description');
  const version = optionalMetadataValue(document, workflow, 'version');
  if (
    name === undefined &&
    description === undefined &&
    version === undefined
  ) {
    return undefined;
  }
  return {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
    ...(version === undefined ? {} : { version }),
  };
}

function flowItemMetadata(
  document: WomlSourceDocument,
  element: WomlSourceElement
): Readonly<Record<string, JsonValue>> | undefined {
  const name = optionalMetadataValue(document, element, 'name');
  const description = optionalMetadataValue(document, element, 'description');
  if (name === undefined && description === undefined) return undefined;
  return {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
  };
}

function validateWorkflowChildren(
  document: WomlSourceDocument,
  workflow: WomlSourceElement
): readonly [
  WomlSourceElement | undefined,
  WomlSourceElement | undefined,
  WomlSourceElement | undefined,
  WomlSourceElement,
] {
  const children = elementChildren(document, workflow);
  const configContainers = children.filter(child => child.name === 'config');
  const lifecycleContainers = children.filter(
    child => child.name === 'lifecycle'
  );
  const triggerContainers = children.filter(child => child.name === 'triggers');
  const stepsContainers = children.filter(child => child.name === 'steps');

  if (configContainers.length > 1) {
    failValidation(
      document,
      'WOML_CONFIG_DUPLICATE',
      `<workflow> accepts at most one <config> element; found ${configContainers.length}.`,
      configContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }

  if (lifecycleContainers.length > 1) {
    failValidation(
      document,
      'WOML_LIFECYCLE_DUPLICATE',
      `<workflow> accepts at most one <lifecycle> container; found ${lifecycleContainers.length}.`,
      lifecycleContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }

  if (triggerContainers.length > 1) {
    failValidation(
      document,
      'WOML_TRIGGER_CONTAINER_COUNT',
      `<workflow> accepts at most one <triggers> container; found ${triggerContainers.length}.`,
      triggerContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }
  if (stepsContainers.length !== 1) {
    failValidation(
      document,
      'WOML_STEPS_CONTAINER_COUNT',
      `<workflow> requires exactly one <steps> container; found ${stepsContainers.length}.`,
      stepsContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }
  const triggers = triggerContainers[0];
  const config = configContainers[0];
  const lifecycle = lifecycleContainers[0];
  const steps = stepsContainers[0];
  const allowed = new Set(['config', 'lifecycle', 'triggers', 'steps']);
  const offender = children.find(child => !allowed.has(child.name));
  if (offender !== undefined) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<workflow> cannot contain <${offender.name}> directly.`,
      offender.openTagSpan,
      'Use optional <config>, <lifecycle>, and <triggers> containers plus exactly one <steps> container, in any order.'
    );
  }

  return [config, lifecycle, triggers, steps];
}

function workflowPolicyDurationMs(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  code: 'WOML_CONFIG_TIMEOUT_INVALID' | 'WOML_CONFIG_RATE_LIMIT_INVALID'
): number {
  const match =
    /^(?:(?:[1-9][0-9]*)(?:\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)(ms|s|m|h|d)$/.exec(
      attribute.value
    );
  if (match === null) {
    failValidation(
      document,
      code,
      `Policy duration "${attribute.value}" must be positive and include ms, s, m, h, or d.`,
      attribute.valueSpan,
      'Examples: 500ms, 10s, 5m, 2h, 1d'
    );
  }
  const numeric = Number(attribute.value.slice(0, -match[1].length));
  const units = {
    ms: 1,
    s: 1000,
    m: 60_000,
    h: 3_600_000,
    d: 86_400_000,
  } as const;
  const milliseconds = numeric * units[match[1] as keyof typeof units];
  if (
    !Number.isSafeInteger(milliseconds) ||
    milliseconds < 1 ||
    milliseconds > maximumWorkflowPolicyDurationMs
  ) {
    failValidation(
      document,
      code,
      `Policy duration "${attribute.value}" must resolve to a whole number of milliseconds from 1ms through 365d.`,
      attribute.valueSpan
    );
  }
  return milliseconds;
}

function validateRuntimePolicy(
  document: WomlSourceDocument,
  config: WomlSourceElement
): ValidatedRuntimePolicy {
  if (config.children.length > 0) {
    failValidation(
      document,
      'WOML_CONFIG_CHILD_NOT_ALLOWED',
      '<config> is data-only and cannot contain text or child elements.',
      config.children[0].span,
      'Declare workflow runtime policy through <config> attributes only.'
    );
  }
  if (Object.keys(config.attributes).length === 0) {
    failValidation(
      document,
      'WOML_CONFIG_EMPTY',
      '<config> requires at least one runtime-policy attribute.',
      config.openTagSpan,
      'Add concurrency, timeout, rate-limit, or queue; otherwise remove <config>.'
    );
  }

  const concurrencyAttribute = config.attributes.concurrency;
  let concurrency: number | undefined;
  if (concurrencyAttribute !== undefined) {
    if (!/^[1-9][0-9]*$/.test(concurrencyAttribute.value)) {
      failValidation(
        document,
        'WOML_CONFIG_CONCURRENCY_INVALID',
        `Workflow concurrency "${concurrencyAttribute.value}" must be a positive integer.`,
        concurrencyAttribute.valueSpan
      );
    }
    concurrency = Number(concurrencyAttribute.value);
    if (
      !Number.isSafeInteger(concurrency) ||
      concurrency > maximumWorkflowConcurrency
    ) {
      failValidation(
        document,
        'WOML_CONFIG_CONCURRENCY_INVALID',
        `Workflow concurrency must be between 1 and ${maximumWorkflowConcurrency}.`,
        concurrencyAttribute.valueSpan
      );
    }
  }

  const timeoutAttribute = config.attributes.timeout;
  const timeoutMs =
    timeoutAttribute === undefined
      ? undefined
      : workflowPolicyDurationMs(
          document,
          timeoutAttribute,
          'WOML_CONFIG_TIMEOUT_INVALID'
        );

  const rateAttribute = config.attributes['rate-limit'];
  let rateLimit: CompiledRuntimePolicyV1['rateLimit'];
  if (rateAttribute !== undefined) {
    const separator = rateAttribute.value.indexOf('/');
    const countSource = rateAttribute.value.slice(0, separator);
    const windowSource = rateAttribute.value.slice(separator + 1);
    if (
      separator <= 0 ||
      rateAttribute.value.indexOf('/', separator + 1) !== -1 ||
      !/^[1-9][0-9]*$/.test(countSource) ||
      windowSource.length === 0
    ) {
      failValidation(
        document,
        'WOML_CONFIG_RATE_LIMIT_INVALID',
        `Rate limit "${rateAttribute.value}" must use positive-integer/duration syntax.`,
        rateAttribute.valueSpan,
        'Example: 100/1m'
      );
    }
    const count = Number(countSource);
    if (!Number.isSafeInteger(count) || count > maximumWorkflowRateCount) {
      failValidation(
        document,
        'WOML_CONFIG_RATE_LIMIT_INVALID',
        `Rate-limit count must be between 1 and ${maximumWorkflowRateCount}.`,
        rateAttribute.valueSpan
      );
    }
    const sourceFile = new SourceFile(document.file, document.source);
    const windowStart = rateAttribute.valueSpan.start.offset + separator + 1;
    const windowAttribute: WomlSourceAttribute = {
      name: rateAttribute.name,
      value: windowSource,
      nameSpan: rateAttribute.nameSpan,
      valueSpan: sourceFile.span(
        windowStart,
        rateAttribute.valueSpan.end.offset
      ),
      span: rateAttribute.span,
    };
    rateLimit = {
      count,
      windowMs: workflowPolicyDurationMs(
        document,
        windowAttribute,
        'WOML_CONFIG_RATE_LIMIT_INVALID'
      ),
      algorithm: 'rolling_window',
    };
  }

  const queueAttribute = config.attributes.queue;
  let queue: CompiledRuntimePolicyV1['queue'];
  if (queueAttribute !== undefined) {
    if (
      queueAttribute.value.length > 128 ||
      !queueNamePattern.test(queueAttribute.value)
    ) {
      failValidation(
        document,
        'WOML_CONFIG_QUEUE_INVALID',
        `Queue name "${queueAttribute.value}" must be a lowercase dot, underscore, or kebab identifier.`,
        queueAttribute.valueSpan,
        'Examples: orders, moderation-high, agents.inbound'
      );
    }
    queue = {
      name: queueAttribute.value,
      discipline: 'work_conserving_fifo',
    };
  }

  return {
    element: config,
    value: {
      profileVersion: 1,
      ...(concurrency === undefined ? {} : { concurrency }),
      ...(timeoutMs === undefined ? {} : { timeoutMs }),
      ...(rateLimit === undefined ? {} : { rateLimit }),
      ...(queue === undefined ? {} : { queue }),
    },
  };
}

function schemaBody(
  document: WomlSourceDocument,
  schema: WomlSourceElement,
  owner: 'webhook' | 'event'
): Readonly<Record<string, JsonValue>> {
  const ownerLabel = owner === 'webhook' ? 'webhook' : 'event';
  const codePrefix = owner === 'webhook' ? 'WOML_WEBHOOK' : 'WOML_EVENT';
  if (Object.keys(schema.attributes).length > 0) {
    const attribute = Object.values(schema.attributes)[0];
    failValidation(
      document,
      'WOML_UNKNOWN_ATTRIBUTE',
      `Unknown attribute "${attribute.name}" on <schema>.`,
      attribute.nameSpan
    );
  }
  const invalidChild = schema.children.find(child => child.kind !== 'text');
  if (invalidChild !== undefined) {
    failValidation(
      document,
      `${codePrefix}_SCHEMA_STRUCTURE_INVALID`,
      '<schema> must contain inline JSON only.',
      invalidChild.span
    );
  }
  const textNodes = schema.children.filter(child => child.kind === 'text');
  const text = textNodes.map(child => child.value).join('');
  const span = schema.children[0]?.span ?? schema.openTagSpan;
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    const position =
      error instanceof SyntaxError
        ? /position\s+(\d+)/i.exec(error.message)?.[1]
        : undefined;
    const relative = position === undefined ? 0 : Number(position);
    const source = new SourceFile(document.file, document.source);
    const start = Math.min(span.start.offset + relative, span.end.offset);
    failValidation(
      document,
      `${codePrefix}_SCHEMA_JSON_INVALID`,
      '<schema> must contain valid JSON.',
      source.span(start, Math.min(start + 1, span.end.offset))
    );
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    failValidation(
      document,
      `${codePrefix}_SCHEMA_INVALID`,
      '<schema> must contain a JSON Schema object.',
      span
    );
  }
  const validator = new Ajv2020({ allErrors: true, strict: false });
  let validSchema = false;
  let schemaError: string | undefined;
  try {
    validSchema = validator.validateSchema(parsed) === true;
    if (validSchema) validator.compile(parsed);
  } catch (error) {
    validSchema = false;
    schemaError = error instanceof Error ? error.message : undefined;
  }
  if (!validSchema) {
    const issue = validator.errors?.[0];
    const reason = issue?.message ?? schemaError;
    failValidation(
      document,
      `${codePrefix}_SCHEMA_INVALID`,
      `Inline ${ownerLabel} JSON Schema is invalid${reason === undefined ? '.' : `: ${reason}.`}`,
      span
    );
  }
  return parsed as Readonly<Record<string, JsonValue>>;
}

function validateWebhookTrigger(
  document: WomlSourceDocument,
  webhook: WomlSourceElement
): ValidatedWebhookTrigger {
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, webhook, 'id'),
    'trigger'
  );
  const pathAttribute = requiredAttribute(document, webhook, 'path');
  if (
    pathAttribute.value.length > 2048 ||
    !webhookPathPattern.test(pathAttribute.value) ||
    pathAttribute.value === '/_woml' ||
    pathAttribute.value.startsWith('/_woml/')
  ) {
    failValidation(
      document,
      'WOML_WEBHOOK_PATH_INVALID',
      `Webhook path "${pathAttribute.value}" must be an exact absolute route without parameters, wildcards, repeated slashes, or the reserved /_woml prefix.`,
      pathAttribute.valueSpan
    );
  }
  const methodAttribute = webhook.attributes.method;
  if (methodAttribute !== undefined && methodAttribute.value !== 'POST') {
    failValidation(
      document,
      'WOML_WEBHOOK_METHOD_UNSUPPORTED',
      `Webhook method "${methodAttribute.value}" is not executable in Webhook HTTP v1; use POST.`,
      methodAttribute.valueSpan
    );
  }
  const authAttribute = requiredAttribute(document, webhook, 'auth');
  let authentication: ValidatedWebhookTrigger['authentication'];
  if (authAttribute.value === 'none') {
    const secret = webhook.attributes.secret;
    if (secret !== undefined) {
      failValidation(
        document,
        'WOML_WEBHOOK_AUTH_INVALID',
        'A webhook with auth="none" must not declare a secret.',
        secret.nameSpan
      );
    }
    authentication = { kind: 'none' };
  } else if (authAttribute.value === 'bearer') {
    authentication = {
      kind: 'bearer',
      secret: requireSecretReference(
        document,
        requiredAttribute(document, webhook, 'secret')
      ),
    };
  } else {
    failValidation(
      document,
      'WOML_WEBHOOK_AUTH_INVALID',
      `Webhook auth "${authAttribute.value}" must be "bearer" or "none".`,
      authAttribute.valueSpan
    );
  }

  const children = elementChildren(document, webhook);
  if (
    children.length > 1 ||
    (children.length === 1 && children[0].name !== 'schema')
  ) {
    const offender =
      children.find(child => child.name !== 'schema') ?? children[1];
    failValidation(
      document,
      'WOML_WEBHOOK_STRUCTURE_INVALID',
      '<webhook> may contain at most one inline <schema>.',
      offender?.openTagSpan ?? webhook.openTagSpan
    );
  }
  const schema =
    children[0] === undefined
      ? undefined
      : schemaBody(document, children[0], 'webhook');
  return {
    kind: 'webhook',
    id,
    path: pathAttribute.value,
    method: 'POST',
    authentication,
    ...(schema === undefined ? {} : { schema }),
  };
}

function validateScheduleTrigger(
  document: WomlSourceDocument,
  schedule: WomlSourceElement
): ValidatedScheduleTrigger {
  ensureEmptyElement(document, schedule);
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, schedule, 'id'),
    'trigger'
  );
  const cron = requiredAttribute(document, schedule, 'cron');
  try {
    parseScheduleCron(cron.value);
  } catch (error) {
    const reason =
      error instanceof ScheduleCronSyntaxError
        ? error.reason
        : 'cron does not match WOML Cron v1';
    failValidation(
      document,
      'WOML_SCHEDULE_CRON_INVALID',
      `Schedule cron is invalid: ${reason}.`,
      cron.valueSpan,
      'Use five numeric fields: minute hour day-of-month month day-of-week. Lists, ranges, and /steps are supported.'
    );
  }
  const timezone = schedule.attributes.timezone?.value ?? 'UTC';
  if (!isSupportedScheduleTimeZone(timezone)) {
    failValidation(
      document,
      'WOML_SCHEDULE_TIMEZONE_INVALID',
      `Schedule timezone "${timezone}" is not a canonical IANA timezone identifier.`,
      schedule.attributes.timezone?.valueSpan ?? schedule.openTagSpan,
      'Examples: UTC, Europe/Berlin, America/New_York'
    );
  }
  const onMissed = schedule.attributes['on-missed']?.value ?? 'skip';
  if (onMissed !== 'skip' && onMissed !== 'run-once') {
    failValidation(
      document,
      'WOML_TRIGGER_MISFIRE_INVALID',
      `Schedule on-missed must be "skip" or "run-once", found "${onMissed}".`,
      schedule.attributes['on-missed']!.valueSpan
    );
  }
  return { kind: 'schedule', id, cron: cron.value, timezone, onMissed };
}

function validateIntervalTrigger(
  document: WomlSourceDocument,
  interval: WomlSourceElement
): ValidatedIntervalTrigger {
  ensureEmptyElement(document, interval);
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, interval, 'id'),
    'trigger'
  );
  const every = requiredAttribute(document, interval, 'every');
  const match = /^([1-9][0-9]*)(ms|s|m|h|d)$/.exec(every.value);
  if (match === null) {
    failValidation(
      document,
      'WOML_INTERVAL_INVALID',
      `Interval every="${every.value}" must be one positive whole duration using ms, s, m, h, or d.`,
      every.valueSpan,
      'Examples: 1s, 5m, 24h, 30d'
    );
  }
  const everyMs =
    Number(match[1]) *
    durationUnitsMs[match[2] as keyof typeof durationUnitsMs];
  if (
    !Number.isSafeInteger(everyMs) ||
    everyMs < durationUnitsMs.s ||
    everyMs > durationUnitsMs.d * 30
  ) {
    failValidation(
      document,
      'WOML_INTERVAL_INVALID',
      `Interval every="${every.value}" must resolve to a whole duration from 1s through 30d.`,
      every.valueSpan
    );
  }
  const onMissed = interval.attributes['on-missed']?.value ?? 'skip';
  if (onMissed !== 'skip' && onMissed !== 'run-once') {
    failValidation(
      document,
      'WOML_TRIGGER_MISFIRE_INVALID',
      `Interval on-missed must be "skip" or "run-once", found "${onMissed}".`,
      interval.attributes['on-missed']!.valueSpan
    );
  }
  return { kind: 'interval', id, everyMs, onMissed };
}

function validateEventTrigger(
  document: WomlSourceDocument,
  event: WomlSourceElement
): ValidatedEventTrigger {
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, event, 'id'),
    'trigger'
  );
  const name = requiredAttribute(document, event, 'name');
  const secretAttribute = event.attributes.secret;
  const secret =
    secretAttribute === undefined
      ? undefined
      : requireSecretReference(document, secretAttribute);
  if (name.value.length > 256 || !eventNamePattern.test(name.value)) {
    failValidation(
      document,
      'WOML_EVENT_NAME_INVALID',
      `Event name "${name.value}" must start with a lowercase letter and contain at least two lowercase alphanumeric segments separated by one dot, underscore, or hyphen.`,
      name.valueSpan,
      'Examples: order.created, payment_failed, agent-response'
    );
  }
  const children = elementChildren(document, event);
  if (
    children.length > 1 ||
    (children.length === 1 && children[0].name !== 'schema')
  ) {
    const offender =
      children.find(child => child.name !== 'schema') ?? children[1];
    failValidation(
      document,
      'WOML_EVENT_STRUCTURE_INVALID',
      '<event> may contain at most one inline <schema>.',
      offender?.openTagSpan ?? event.openTagSpan
    );
  }
  const schema =
    children[0] === undefined
      ? undefined
      : schemaBody(document, children[0], 'event');
  return {
    kind: 'event',
    id,
    name: name.value,
    ...(secret === undefined ? {} : { secret }),
    ...(schema === undefined ? {} : { schema }),
  };
}

function validateTriggers(
  document: WomlSourceDocument,
  triggers: WomlSourceElement
): readonly ValidatedTrigger[] {
  const children = elementChildren(document, triggers);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_TRIGGER_REQUIRED',
      '<triggers> requires at least one executable trigger.',
      triggers.openTagSpan
    );
  }
  if (children.length > 64) {
    failValidation(
      document,
      'WOML_MODULE_ALIAS_LIMIT_EXCEEDED',
      `<imports> declares ${children.length} modules; Module profile v1 allows at most 64.`,
      children[64].openTagSpan
    );
  }
  const validated = children.map((child): ValidatedTrigger => {
    if (child.name === 'manual') {
      ensureEmptyElement(document, child);
      return {
        kind: 'manual',
        id: validateJavaScriptSafeId(
          document,
          requiredAttribute(document, child, 'id'),
          'trigger'
        ),
      };
    }
    if (child.name === 'webhook') {
      return validateWebhookTrigger(document, child);
    }
    if (child.name === 'slack') {
      return validateSlackTrigger(document, child);
    }
    if (child.name === 'telegram') {
      return validateTelegramTrigger(document, child);
    }
    if (child.name === 'schedule') {
      return validateScheduleTrigger(document, child);
    }
    if (child.name === 'interval') {
      return validateIntervalTrigger(document, child);
    }
    if (child.name === 'event') {
      return validateEventTrigger(document, child);
    }
    failValidation(
      document,
      'WOML_TRIGGER_UNSUPPORTED',
      `<${child.name}> is not an executable trigger in this release.`,
      child.openTagSpan
    );
  });
  const ids = new Set<string>();
  for (let index = 0; index < validated.length; index += 1) {
    const trigger = validated[index];
    if (ids.has(trigger.id)) {
      failValidation(
        document,
        'WOML_TRIGGER_ID_DUPLICATE',
        `Trigger ID "${trigger.id}" is already used in this workflow.`,
        requiredAttribute(document, children[index], 'id').valueSpan
      );
    }
    ids.add(trigger.id);
  }
  return validated;
}

interface ScriptBody {
  readonly source: string;
  readonly span: SourceSpan;
}

function scriptBody(
  document: WomlSourceDocument,
  script: WomlSourceElement
): ScriptBody {
  const rawBodies = script.children.filter(
    (child): child is WomlSourceRawText => child.kind === 'raw'
  );
  if (
    script.children.some(child => child.kind !== 'raw') ||
    rawBodies.length > 1
  ) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<script> must contain one raw JavaScript body and no WOML child elements.',
      script.children[0]?.span ?? script.openTagSpan
    );
  }
  return {
    source: rawBodies[0]?.value ?? '',
    span: rawBodies[0]?.span ?? script.openTagSpan,
  };
}

function validateScriptAnalysis(
  document: WomlSourceDocument,
  body: ScriptBody,
  shadowedServices: readonly string[] = []
): ScriptAnalysis {
  const analysis = analyzeWomlScript(body.source, { shadowedServices });
  if (analysis.issue === undefined) return analysis;
  const sourceFile = new SourceFile(document.file, document.source);
  const start = body.span.start.offset + analysis.issue.start;
  const end = body.span.start.offset + analysis.issue.end;
  failValidation(
    document,
    analysis.issue.code,
    analysis.issue.message,
    sourceFile.span(start, Math.max(start + 1, end)),
    analysis.issue.hint
  );
}

function referenceSpan(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  structuralId: string
): SourceSpan {
  const relativeOffset = attribute.value.indexOf(
    structuralId,
    '{{context.steps.'.length
  );
  const start = attribute.valueSpan.start.offset + relativeOffset;
  return new SourceFile(document.file, document.source).span(
    start,
    start + structuralId.length
  );
}

function parseExactReference(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): ValidatedReference {
  const match =
    /^\{\{(context\.(?:(?:payload|trigger)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*|steps\.([a-z][A-Za-z0-9]*)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*))\}\}$/.exec(
      attribute.value
    );
  if (match === null) {
    failValidation(
      document,
      'WOML_INVALID_REFERENCE',
      `Attribute "${attribute.name}" must contain exactly one WOML context reference.`,
      attribute.valueSpan,
      'Example: {{context.steps.checkContent.needsReview}}'
    );
  }

  const structuralId = match[2];
  const span =
    structuralId === undefined
      ? attribute.valueSpan
      : referenceSpan(document, attribute, structuralId);
  return {
    path: match[1]
      .split('.')
      .slice(1)
      .map((segment, index) =>
        index === 0 && segment === 'payload' ? 'trigger' : segment
      ),
    ...(structuralId === undefined ? {} : { structuralId }),
    span,
  };
}

function registerStructuralId(
  document: WomlSourceDocument,
  registry: Set<string>,
  attribute: WomlSourceAttribute,
  role:
    | 'step'
    | 'branch'
    | 'choose'
    | 'switch'
    | 'fork'
    | 'parallel'
    | 'approval'
): string {
  const id = validateJavaScriptSafeId(document, attribute, role);
  if (registry.has(id)) {
    failValidation(
      document,
      'WOML_DUPLICATE_ID',
      `Structural ID "${id}" is duplicated across workflow steps, choices, forks, parallel groups, and approvals.`,
      attribute.valueSpan
    );
  }
  registry.add(id);
  return id;
}

const durationUnitsMs = {
  ms: 1,
  s: 1_000,
  m: 60_000,
  h: 3_600_000,
  d: 86_400_000,
} as const;

const maximumRetryDelayMs = durationUnitsMs.h * 24;

function retryDurationMs(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  code: 'WOML_RETRY_DELAY_INVALID' | 'WOML_RETRY_MAX_DELAY_INVALID'
): number {
  const match =
    /^(?:(?:[1-9][0-9]*)(?:\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)(ms|s|m|h|d)$/.exec(
      attribute.value
    );
  if (match === null) {
    failValidation(
      document,
      code,
      `Retry duration "${attribute.value}" must be a positive duration using ms, s, m, h, or d.`,
      attribute.valueSpan,
      'Examples: 500ms, 1s, 30m, 24h'
    );
  }

  const numeric = Number(attribute.value.slice(0, -match[1].length));
  const milliseconds =
    numeric * durationUnitsMs[match[1] as keyof typeof durationUnitsMs];
  if (
    !Number.isSafeInteger(milliseconds) ||
    milliseconds < 1 ||
    milliseconds > maximumRetryDelayMs
  ) {
    failValidation(
      document,
      code,
      `Retry duration "${attribute.value}" must resolve to a whole number of milliseconds from 1ms through 24h.`,
      attribute.valueSpan
    );
  }
  return milliseconds;
}

function stepRetryPolicy(
  document: WomlSourceDocument,
  step: WomlSourceElement
): RetryPolicy | undefined {
  const retry = step.attributes.retry;
  const backoff = step.attributes['retry-backoff'];
  const delay = step.attributes['retry-delay'];
  const maximumDelay = step.attributes['retry-max-delay'];
  const firstBackoffAttribute = backoff ?? delay ?? maximumDelay;

  if (retry === undefined) {
    if (firstBackoffAttribute !== undefined) {
      failValidation(
        document,
        'WOML_RETRY_BACKOFF_REQUIRES_RETRY',
        `Attribute "${firstBackoffAttribute.name}" requires retry greater than 1 on the same <step>.`,
        firstBackoffAttribute.nameSpan
      );
    }
    return undefined;
  }

  if (!/^[1-9][0-9]*$/.test(retry.value)) {
    failValidation(
      document,
      'WOML_RETRY_INVALID',
      `Retry "${retry.value}" must be an integer from 1 through 10.`,
      retry.valueSpan
    );
  }
  const maxAttempts = Number(retry.value);
  if (!Number.isSafeInteger(maxAttempts) || maxAttempts > 10) {
    failValidation(
      document,
      'WOML_RETRY_INVALID',
      `Retry "${retry.value}" must be an integer from 1 through 10.`,
      retry.valueSpan
    );
  }
  if (maxAttempts === 1) {
    if (firstBackoffAttribute !== undefined) {
      failValidation(
        document,
        'WOML_RETRY_BACKOFF_REQUIRES_RETRY',
        `Attribute "${firstBackoffAttribute.name}" requires retry greater than 1 on the same <step>.`,
        firstBackoffAttribute.nameSpan
      );
    }
    return undefined;
  }

  const strategy = backoff?.value ?? 'exponential';
  if (strategy !== 'fixed' && strategy !== 'exponential') {
    failValidation(
      document,
      'WOML_RETRY_BACKOFF_INVALID',
      `Retry backoff must be "fixed" or "exponential", found "${strategy}".`,
      backoff!.valueSpan
    );
  }

  const delayMs =
    delay === undefined
      ? 1_000
      : retryDurationMs(document, delay, 'WOML_RETRY_DELAY_INVALID');

  if (strategy === 'fixed') {
    if (maximumDelay !== undefined) {
      failValidation(
        document,
        'WOML_RETRY_MAX_DELAY_NOT_ALLOWED',
        'Attribute "retry-max-delay" is available only with exponential retry backoff.',
        maximumDelay.nameSpan
      );
    }
    return {
      maxAttempts,
      backoff: { kind: 'fixed', delayMs },
    };
  }

  const maximumDelayMs =
    maximumDelay === undefined
      ? Math.max(30_000, delayMs)
      : retryDurationMs(document, maximumDelay, 'WOML_RETRY_MAX_DELAY_INVALID');
  if (maximumDelayMs < delayMs) {
    failValidation(
      document,
      'WOML_RETRY_MAX_DELAY_INVALID',
      'Attribute "retry-max-delay" must be greater than or equal to "retry-delay".',
      maximumDelay!.valueSpan
    );
  }
  return {
    maxAttempts,
    backoff: {
      kind: 'exponential',
      initialDelayMs: delayMs,
      multiplier: 2,
      maximumDelayMs,
    },
  };
}

function approvalTimeoutMs(
  document: WomlSourceDocument,
  approval: WomlSourceElement
): number | undefined {
  const timeout = approval.attributes.timeout;
  const onTimeout = approval.attributes['on-timeout'];
  if (timeout === undefined) {
    if (onTimeout !== undefined) {
      failValidation(
        document,
        'WOML_APPROVAL_TIMEOUT_INVALID',
        '<approval> cannot declare on-timeout without a timeout.',
        onTimeout.nameSpan
      );
    }
    return undefined;
  }

  const match =
    /^(?:(?:[1-9][0-9]*)(?:\.[0-9]+)?|0\.[0-9]*[1-9][0-9]*)(ms|s|m|h|d)$/.exec(
      timeout.value
    );
  if (match === null) {
    failValidation(
      document,
      'WOML_APPROVAL_TIMEOUT_INVALID',
      `Approval timeout "${timeout.value}" must be a positive duration using ms, s, m, h, or d.`,
      timeout.valueSpan,
      'Examples: 500ms, 30m, 24h'
    );
  }

  const numeric = Number(timeout.value.slice(0, -match[1].length));
  const milliseconds =
    numeric * durationUnitsMs[match[1] as keyof typeof durationUnitsMs];
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 1) {
    failValidation(
      document,
      'WOML_APPROVAL_TIMEOUT_INVALID',
      `Approval timeout "${timeout.value}" must resolve to a positive safe integer number of milliseconds.`,
      timeout.valueSpan
    );
  }
  return milliseconds;
}

function validateApprovalArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): readonly ValidatedFlowItem[] {
  return elementChildren(document, arm).map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`, context)
  );
}

const slackChannelAliasPattern = /^#[a-z0-9][a-z0-9_-]{0,79}$/;
const slackConversationIdPattern = /^[CG][A-Z0-9]{8,31}$/;
const slackTriggerChannelAliasPattern = /^[a-z0-9][a-z0-9_-]{0,79}$/;
const slackTriggerConversationIdPattern = /^[CGD][A-Z0-9]{8,31}$/;

interface SlackChannelToken {
  readonly value: string;
  readonly span: SourceSpan;
}

function requiredSlackAttribute(
  document: WomlSourceDocument,
  slack: WomlSourceElement,
  name: 'channels' | 'message' | 'bot-token' | 'app-token'
): WomlSourceAttribute {
  const attribute = slack.attributes[name];
  if (attribute === undefined) {
    failValidation(
      document,
      'WOML_SLACK_ATTRIBUTE_REQUIRED',
      `<slack> requires the "${name}" attribute.`,
      slack.openTagSpan
    );
  }
  return attribute;
}

function slackChannelTokens(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): readonly SlackChannelToken[] {
  const sourceFile = new SourceFile(document.file, document.source);
  const tokens = [...attribute.value.matchAll(/\S+/g)].map(match => {
    const start = attribute.valueSpan.start.offset + (match.index ?? 0);
    return {
      value: match[0],
      span: sourceFile.span(start, start + match[0].length),
    };
  });
  if (tokens.length === 0) {
    failValidation(
      document,
      'WOML_SLACK_CHANNELS_EMPTY',
      '<slack> channels must contain at least one channel alias or conversation ID.',
      attribute.valueSpan
    );
  }
  for (const token of tokens) {
    if (
      !slackChannelAliasPattern.test(token.value) &&
      !slackConversationIdPattern.test(token.value)
    ) {
      failValidation(
        document,
        'WOML_SLACK_CHANNEL_INVALID',
        `Slack destination "${token.value}" must be a lowercase #channel alias or a Slack conversation ID.`,
        token.span,
        'Examples: #approvals or C0123456789'
      );
    }
  }
  return tokens;
}

function commaSeparatedSlackTokens(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  label: 'events' | 'channels'
): readonly SlackChannelToken[] {
  const sourceFile = new SourceFile(document.file, document.source);
  const tokens: SlackChannelToken[] = [];
  let offset = 0;
  for (const part of attribute.value.split(',')) {
    const leading = part.length - part.trimStart().length;
    const value = part.trim();
    const start = attribute.valueSpan.start.offset + offset + leading;
    if (value.length === 0) {
      failValidation(
        document,
        'WOML_SLACK_TRIGGER_LIST_INVALID',
        `<slack> ${label} must be a comma-separated list without empty items.`,
        sourceFile.span(start, start)
      );
    }
    tokens.push({ value, span: sourceFile.span(start, start + value.length) });
    offset += part.length + 1;
  }
  return tokens;
}

function validateSlackTrigger(
  document: WomlSourceDocument,
  slack: WomlSourceElement
): ValidatedSlackTrigger {
  ensureEmptyElement(document, slack);
  if (slack.attributes.message !== undefined) {
    failValidation(
      document,
      'WOML_SLACK_UNKNOWN_ATTRIBUTE',
      'Attribute "message" is valid on lifecycle notifications, not a Slack trigger.',
      slack.attributes.message.nameSpan
    );
  }
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, slack, 'id'),
    'trigger'
  );
  const eventTokens = commaSeparatedSlackTokens(
    document,
    requiredAttribute(document, slack, 'events'),
    'events'
  );
  const events: SlackTriggerEvent[] = [];
  const seenEvents = new Set<string>();
  for (const token of eventTokens) {
    if (token.value !== 'app-mention' && token.value !== 'direct-message') {
      failValidation(
        document,
        'WOML_SLACK_TRIGGER_EVENT_INVALID',
        `Unsupported Slack trigger event "${token.value}".`,
        token.span,
        'Use app-mention, direct-message, or both.'
      );
    }
    if (seenEvents.has(token.value)) {
      failValidation(
        document,
        'WOML_SLACK_TRIGGER_EVENT_DUPLICATE',
        `Slack trigger event "${token.value}" is listed more than once.`,
        token.span
      );
    }
    seenEvents.add(token.value);
    events.push(token.value);
  }

  const channelAttribute = slack.attributes.channels;
  const channelTokens =
    channelAttribute === undefined
      ? []
      : commaSeparatedSlackTokens(document, channelAttribute, 'channels');
  const channels: string[] = [];
  const seenChannels = new Set<string>();
  for (const token of channelTokens) {
    if (
      !slackTriggerChannelAliasPattern.test(token.value) &&
      !slackTriggerConversationIdPattern.test(token.value)
    ) {
      failValidation(
        document,
        'WOML_SLACK_TRIGGER_CHANNEL_INVALID',
        `Slack trigger channel "${token.value}" must be a lowercase channel name or Slack conversation ID.`,
        token.span,
        'Examples: woml-testing or C0123456789'
      );
    }
    if (seenChannels.has(token.value)) {
      failValidation(
        document,
        'WOML_SLACK_TRIGGER_CHANNEL_DUPLICATE',
        `Slack trigger channel "${token.value}" is listed more than once.`,
        token.span
      );
    }
    seenChannels.add(token.value);
    channels.push(token.value);
  }

  return {
    kind: 'slack',
    id,
    events,
    channels,
    botToken: requireSecretReference(
      document,
      requiredSlackAttribute(document, slack, 'bot-token')
    ),
    appToken: requireSecretReference(
      document,
      requiredSlackAttribute(document, slack, 'app-token')
    ),
  };
}

interface TelegramToken {
  readonly value: string;
  readonly span: SourceSpan;
}

const telegramChatIdPattern = /^-?[1-9][0-9]{0,19}$/;

function requiredTelegramAttribute(
  document: WomlSourceDocument,
  telegram: WomlSourceElement,
  name: 'events' | 'chats' | 'message' | 'bot-token'
): WomlSourceAttribute {
  const attribute = telegram.attributes[name];
  if (attribute === undefined) {
    failValidation(
      document,
      'WOML_TELEGRAM_ATTRIBUTE_REQUIRED',
      `<telegram> requires the "${name}" attribute.`,
      telegram.openTagSpan
    );
  }
  return attribute;
}

function commaSeparatedTelegramTokens(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  label: 'events' | 'chats'
): readonly TelegramToken[] {
  const sourceFile = new SourceFile(document.file, document.source);
  const tokens: TelegramToken[] = [];
  let offset = 0;
  for (const part of attribute.value.split(',')) {
    const leading = part.length - part.trimStart().length;
    const value = part.trim();
    const start = attribute.valueSpan.start.offset + offset + leading;
    if (value.length === 0) {
      failValidation(
        document,
        'WOML_TELEGRAM_LIST_INVALID',
        `<telegram> ${label} must be a comma-separated list without empty items.`,
        sourceFile.span(start, start)
      );
    }
    tokens.push({ value, span: sourceFile.span(start, start + value.length) });
    offset += part.length + 1;
  }
  return tokens;
}

function validateTelegramTrigger(
  document: WomlSourceDocument,
  telegram: WomlSourceElement
): ValidatedTelegramTrigger {
  ensureEmptyElement(document, telegram);
  for (const notificationOnly of ['chats', 'message'] as const) {
    const attribute = telegram.attributes[notificationOnly];
    if (attribute !== undefined) {
      failValidation(
        document,
        'WOML_TELEGRAM_UNKNOWN_ATTRIBUTE',
        `Attribute "${notificationOnly}" is valid on a Telegram notification, not a trigger.`,
        attribute.nameSpan
      );
    }
  }
  const id = validateJavaScriptSafeId(
    document,
    requiredAttribute(document, telegram, 'id'),
    'trigger'
  );
  const events = commaSeparatedTelegramTokens(
    document,
    requiredTelegramAttribute(document, telegram, 'events'),
    'events'
  );
  if (events.length !== 1 || events[0].value !== 'message') {
    const unsupported = events.find(token => token.value !== 'message');
    const invalid = unsupported ?? events[1];
    failValidation(
      document,
      unsupported === undefined
        ? 'WOML_TELEGRAM_TRIGGER_EVENT_DUPLICATE'
        : 'WOML_TELEGRAM_TRIGGER_EVENT_INVALID',
      unsupported === undefined
        ? 'Telegram trigger event "message" is listed more than once.'
        : `Unsupported Telegram trigger event "${unsupported.value}".`,
      invalid?.span ?? telegram.openTagSpan,
      'Telegram v1 supports events="message".'
    );
  }
  return {
    kind: 'telegram',
    id,
    events: ['message'],
    botToken: requireSecretReference(
      document,
      requiredTelegramAttribute(document, telegram, 'bot-token')
    ),
  };
}

function telegramChatTokens(
  document: WomlSourceDocument,
  telegram: WomlSourceElement
): readonly TelegramToken[] {
  const tokens = commaSeparatedTelegramTokens(
    document,
    requiredTelegramAttribute(document, telegram, 'chats'),
    'chats'
  );
  const seen = new Set<string>();
  for (const token of tokens) {
    if (!telegramChatIdPattern.test(token.value)) {
      failValidation(
        document,
        'WOML_TELEGRAM_CHAT_INVALID',
        `Telegram chat "${token.value}" must be a numeric Telegram chat ID.`,
        token.span,
        'Use the positive user/chat ID or negative group/channel ID returned by Telegram.'
      );
    }
    if (seen.has(token.value)) {
      failValidation(
        document,
        'WOML_TELEGRAM_CHAT_DUPLICATE',
        `Telegram chat "${token.value}" is listed more than once.`,
        token.span
      );
    }
    seen.add(token.value);
  }
  return tokens;
}

function validateNotify(
  document: WomlSourceDocument,
  notify: WomlSourceElement,
  approvalId: string
): readonly ValidatedNotificationDelivery[] {
  const providers = elementChildren(document, notify);
  if (providers.length === 0) {
    failValidation(
      document,
      'WOML_NOTIFY_EMPTY',
      '<notify> must contain at least one notification provider.',
      notify.openTagSpan
    );
  }

  const seenDestinations = new Set<string>();
  const deliveries: ValidatedNotificationDelivery[] = [];
  providers.forEach((provider, providerIndex) => {
    if (provider.name !== 'slack' && provider.name !== 'telegram') {
      failValidation(
        document,
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
        `<notify> supports built-in <slack> and <telegram> providers; found <${provider.name}>.`,
        provider.openTagSpan
      );
    }
    ensureEmptyElement(document, provider);
    for (const triggerOnlyAttribute of ['id', 'events', 'message'] as const) {
      const attribute = provider.attributes[triggerOnlyAttribute];
      if (attribute !== undefined) {
        failValidation(
          document,
          provider.name === 'telegram'
            ? 'WOML_TELEGRAM_UNKNOWN_ATTRIBUTE'
            : 'WOML_SLACK_UNKNOWN_ATTRIBUTE',
          triggerOnlyAttribute === 'message'
            ? 'Attribute "message" is valid on a lifecycle notification, not an approval notification.'
            : `Attribute "${triggerOnlyAttribute}" is valid on a ${provider.name === 'telegram' ? 'Telegram' : 'Slack'} trigger, not a notification provider.`,
          attribute.nameSpan
        );
      }
    }
    if (provider.name === 'telegram') {
      const chats = telegramChatTokens(document, provider);
      const botToken = requireSecretReference(
        document,
        requiredTelegramAttribute(document, provider, 'bot-token')
      );
      chats.forEach((chat, chatIndex) => {
        const destinationKey = `telegram\u0000${botToken.name}\u0000${chat.value}`;
        if (seenDestinations.has(destinationKey)) {
          failValidation(
            document,
            'WOML_TELEGRAM_CHAT_DUPLICATE',
            `Telegram destination "${chat.value}" is duplicated for the same credential.`,
            chat.span
          );
        }
        seenDestinations.add(destinationKey);
        deliveries.push({
          deliveryId: `${approvalId}:notify:${providerIndex}:chat:${chatIndex}`,
          provider: 'telegram',
          destination: chat.value,
          botToken,
        });
      });
      return;
    }
    const channels = slackChannelTokens(
      document,
      requiredSlackAttribute(document, provider, 'channels')
    );
    const botToken = requireSecretReference(
      document,
      requiredSlackAttribute(document, provider, 'bot-token')
    );
    const appToken = requireSecretReference(
      document,
      requiredSlackAttribute(document, provider, 'app-token')
    );
    channels.forEach((channel, channelIndex) => {
      const destinationKey = `${botToken.name}\u0000${appToken.name}\u0000${channel.value}`;
      if (seenDestinations.has(destinationKey)) {
        failValidation(
          document,
          'WOML_SLACK_CHANNEL_DUPLICATE',
          `Slack destination "${channel.value}" is duplicated for the same credential set.`,
          channel.span
        );
      }
      seenDestinations.add(destinationKey);
      deliveries.push({
        deliveryId: `${approvalId}:notify:${providerIndex}:channel:${channelIndex}`,
        provider: 'slack',
        destination: channel.value,
        botToken,
        appToken,
      });
    });
  });
  return deliveries;
}

function validateApproval(
  document: WomlSourceDocument,
  approval: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedApproval {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, approval, 'id'),
    'approval'
  );
  const children = elementChildren(document, approval);
  const notify = children.filter(child => child.name === 'notify');
  const approved = children.filter(child => child.name === 'when-approved');
  const rejected = children.filter(child => child.name === 'when-rejected');
  const invalidChild = children.find(
    child =>
      child.name !== 'notify' &&
      child.name !== 'when-approved' &&
      child.name !== 'when-rejected'
  );

  if (invalidChild !== undefined) {
    failValidation(
      document,
      'WOML_APPROVAL_STRUCTURE_INVALID',
      `<approval id="${id}"> may contain optional <notify>, then <when-approved> and <when-rejected> only.`,
      invalidChild.openTagSpan
    );
  }
  if (notify.length > 1 || (notify.length === 1 && children[0] !== notify[0])) {
    failValidation(
      document,
      'WOML_NOTIFY_INVALID_ORDER',
      '<notify> may appear once and must be the first child of <approval>.',
      notify[1]?.openTagSpan ?? notify[0]?.openTagSpan ?? approval.openTagSpan
    );
  }
  if (approved.length !== 1 || rejected.length !== 1) {
    const duplicate = approved[1] ?? rejected[1];
    failValidation(
      document,
      'WOML_APPROVAL_STRUCTURE_INVALID',
      `<approval id="${id}"> requires exactly one <when-approved> and one <when-rejected>.`,
      duplicate?.openTagSpan ?? approval.openTagSpan
    );
  }
  if (
    children.length !== 2 + notify.length ||
    children[notify.length] !== approved[0] ||
    children[notify.length + 1] !== rejected[0]
  ) {
    failValidation(
      document,
      'WOML_APPROVAL_STRUCTURE_INVALID',
      '<when-approved> must appear before <when-rejected> inside <approval>.',
      children[0]?.openTagSpan ?? approval.openTagSpan
    );
  }

  const onTimeoutAttribute = approval.attributes['on-timeout'];
  const onTimeout = onTimeoutAttribute?.value ?? 'fail';
  if (onTimeout !== 'reject' && onTimeout !== 'fail') {
    failValidation(
      document,
      'WOML_APPROVAL_TIMEOUT_INVALID',
      `Approval on-timeout must be "reject" or "fail", found "${onTimeout}".`,
      onTimeoutAttribute!.valueSpan
    );
  }

  return {
    kind: 'approval',
    id,
    element: approval,
    metadata: flowItemMetadata(document, approval),
    timeoutMs: approvalTimeoutMs(document, approval),
    onTimeout,
    notifications:
      notify.length === 0 ? [] : validateNotify(document, notify[0], id),
    approvedItems: validateApprovalArm(
      document,
      approved[0],
      registry,
      context
    ),
    rejectedItems: validateApprovalArm(
      document,
      rejected[0],
      registry,
      context
    ),
  };
}

function validateStep(
  document: WomlSourceDocument,
  step: WomlSourceElement,
  registry: Set<string>,
  shadowedServices: readonly string[] = []
): ValidatedStep {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, step, 'id'),
    'step'
  );

  const operations = elementChildren(document, step);
  if (operations.length !== 1 || operations[0].name !== 'script') {
    failValidation(
      document,
      'WOML_STEP_OPERATION_COUNT',
      `<step id="${id}"> must contain exactly one <script> operation.`,
      operations[1]?.openTagSpan ??
        operations[0]?.openTagSpan ??
        step.openTagSpan
    );
  }

  const body = scriptBody(document, operations[0]);
  return {
    kind: 'step',
    id,
    source: body.source,
    scriptSpan: body.span,
    scriptAnalysis: validateScriptAnalysis(document, body, shadowedServices),
    retryPolicy: stepRetryPolicy(document, step),
    metadata: flowItemMetadata(document, step),
  };
}

const lifecycleHookEvents = {
  'on-start': 'run_start',
  'on-step-start': 'step_start',
  'on-step-success': 'step_success',
  'on-step-failure': 'step_failure',
  'on-step-complete': 'step_complete',
  'on-success': 'run_success',
  'on-error': 'run_failure',
  'on-cancel': 'run_cancel',
  'on-complete': 'run_complete',
} as const satisfies Readonly<Record<string, LifecycleEventName>>;

const lifecycleHookOrder = Object.keys(
  lifecycleHookEvents
) as readonly (keyof typeof lifecycleHookEvents)[];

function templateReference(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  source: string,
  relativeStart: number,
  event: LifecycleEventName,
  stepIds: ReadonlySet<string>
): ContextReferenceExpression | LifecycleReferenceExpression {
  const sourceFile = new SourceFile(document.file, document.source);
  const span = sourceFile.span(
    attribute.valueSpan.start.offset + relativeStart,
    attribute.valueSpan.start.offset + relativeStart + source.length
  );
  const context =
    /^context\.(payload(?:\.[A-Za-z_$][A-Za-z0-9_$]*)+|steps\.([a-z][A-Za-z0-9]*)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)+)$/.exec(
      source
    );
  if (context !== null) {
    if (context[2] !== undefined && !stepIds.has(context[2])) {
      failValidation(
        document,
        'WOML_LIFECYCLE_STEP_UNKNOWN',
        `Lifecycle template references unknown executable step "${context[2]}".`,
        span
      );
    }
    return {
      kind: 'contextReference',
      path: context[1]
        .split('.')
        .map((segment, index) =>
          index === 0 && segment === 'payload' ? 'trigger' : segment
        ),
    };
  }

  const lifecycle =
    /^lifecycle\.(event|workflow\.(?:id|outcome)|step\.(?:id|outcome|attempts)|failure\.(?:code|message))$/.exec(
      source
    );
  if (lifecycle === null) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      `Unsupported lifecycle template reference "${source}".`,
      span,
      'Use a scalar context.payload, context.steps, or reviewed lifecycle field.'
    );
  }
  const path = lifecycle[1].split('.');
  const isStepEvent = event.startsWith('step_');
  const hasOutcome =
    event === 'step_success' ||
    event === 'step_failure' ||
    event === 'step_complete' ||
    event === 'run_success' ||
    event === 'run_failure' ||
    event === 'run_cancel' ||
    event === 'run_complete';
  if (path[0] === 'step' && !isStepEvent) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      `Reference "${source}" is unavailable for ${event}.`,
      span
    );
  }
  if (path.at(-1) === 'outcome' && !hasOutcome) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      `Reference "${source}" is unavailable before an outcome is decided.`,
      span
    );
  }
  if (
    path[0] === 'failure' &&
    event !== 'step_failure' &&
    event !== 'run_failure'
  ) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      `Reference "${source}" is available only in failure hooks.`,
      span
    );
  }
  return { kind: 'lifecycleReference', path };
}

function parseLifecycleTemplate(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  event: LifecycleEventName,
  stepIds: ReadonlySet<string>
): ValueExpression {
  if (attribute.value.length === 0 || attribute.value.length > 4096) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      'Lifecycle notification messages must contain 1 through 4096 characters.',
      attribute.valueSpan
    );
  }
  const parts: (
    | { readonly kind: 'text'; readonly text: string }
    | ContextReferenceExpression
    | LifecycleReferenceExpression
  )[] = [];
  let cursor = 0;
  let placeholders = 0;
  for (const match of attribute.value.matchAll(/\{\{([^{}]+)\}\}/g)) {
    const start = match.index ?? 0;
    const preceding = attribute.value.slice(cursor, start);
    if (preceding.includes('{{') || preceding.includes('}}')) {
      failValidation(
        document,
        'WOML_LIFECYCLE_TEMPLATE_INVALID',
        'Lifecycle message contains an unmatched WOML template delimiter.',
        attribute.valueSpan
      );
    }
    if (preceding.length > 0) parts.push({ kind: 'text', text: preceding });
    const reference = match[1].trim();
    const referenceOffset = start + match[0].indexOf(reference);
    parts.push(
      templateReference(
        document,
        attribute,
        reference,
        referenceOffset,
        event,
        stepIds
      )
    );
    placeholders += 1;
    cursor = start + match[0].length;
  }
  const tail = attribute.value.slice(cursor);
  if (tail.includes('{{') || tail.includes('}}')) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      'Lifecycle message contains an unmatched WOML template delimiter.',
      attribute.valueSpan
    );
  }
  if (tail.length > 0) parts.push({ kind: 'text', text: tail });
  if (placeholders > 32) {
    failValidation(
      document,
      'WOML_LIFECYCLE_TEMPLATE_INVALID',
      'Lifecycle messages may contain at most 32 placeholders.',
      attribute.valueSpan
    );
  }
  return { kind: 'template', parts };
}

function validateLifecycleNotify(
  document: WomlSourceDocument,
  notify: WomlSourceElement,
  event: LifecycleEventName,
  hookIndex: number,
  actionIndex: number,
  stepIds: ReadonlySet<string>
): ValidatedInformationalNotification {
  const providers = elementChildren(document, notify);
  if (providers.length === 0) {
    failValidation(
      document,
      'WOML_LIFECYCLE_ACTION_REQUIRED',
      '<notify> must contain at least one notification provider.',
      notify.openTagSpan
    );
  }
  const seenDestinations = new Set<string>();
  const deliveries: ValidatedNotificationDelivery[] = [];
  providers.forEach((provider, providerIndex) => {
    if (provider.name !== 'slack' && provider.name !== 'telegram') {
      failValidation(
        document,
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
        `<notify> supports built-in <slack> and <telegram> providers; found <${provider.name}>.`,
        provider.openTagSpan
      );
    }
    ensureEmptyElement(document, provider);
    for (const invalidName of ['id', 'events'] as const) {
      const invalid = provider.attributes[invalidName];
      if (invalid !== undefined) {
        failValidation(
          document,
          provider.name === 'telegram'
            ? 'WOML_TELEGRAM_UNKNOWN_ATTRIBUTE'
            : 'WOML_SLACK_UNKNOWN_ATTRIBUTE',
          `Attribute "${invalidName}" is valid on a ${provider.name === 'telegram' ? 'Telegram' : 'Slack'} trigger, not a lifecycle notification.`,
          invalid.nameSpan
        );
      }
    }
    const message = parseLifecycleTemplate(
      document,
      provider.name === 'telegram'
        ? requiredTelegramAttribute(document, provider, 'message')
        : requiredSlackAttribute(document, provider, 'message'),
      event,
      stepIds
    );
    if (provider.name === 'telegram') {
      const chats = telegramChatTokens(document, provider);
      const botToken = requireSecretReference(
        document,
        requiredTelegramAttribute(document, provider, 'bot-token')
      );
      chats.forEach((chat, chatIndex) => {
        const destinationKey = `telegram\u0000${botToken.name}\u0000${chat.value}`;
        if (seenDestinations.has(destinationKey)) {
          failValidation(
            document,
            'WOML_TELEGRAM_CHAT_DUPLICATE',
            `Telegram destination "${chat.value}" is duplicated for the same credential.`,
            chat.span
          );
        }
        seenDestinations.add(destinationKey);
        deliveries.push({
          deliveryId: `lifecycle:${hookIndex}:action:${actionIndex}:provider:${providerIndex}:chat:${chatIndex}`,
          provider: 'telegram',
          destination: chat.value,
          botToken,
          message,
        });
      });
      return;
    }
    const channels = slackChannelTokens(
      document,
      requiredSlackAttribute(document, provider, 'channels')
    );
    const botToken = requireSecretReference(
      document,
      requiredSlackAttribute(document, provider, 'bot-token')
    );
    const appToken = requireSecretReference(
      document,
      requiredSlackAttribute(document, provider, 'app-token')
    );
    channels.forEach((channel, channelIndex) => {
      const destinationKey = `${botToken.name}\u0000${appToken.name}\u0000${channel.value}`;
      if (seenDestinations.has(destinationKey)) {
        failValidation(
          document,
          'WOML_SLACK_CHANNEL_DUPLICATE',
          `Slack destination "${channel.value}" is duplicated for the same credential set.`,
          channel.span
        );
      }
      seenDestinations.add(destinationKey);
      deliveries.push({
        deliveryId: `lifecycle:${hookIndex}:action:${actionIndex}:provider:${providerIndex}:channel:${channelIndex}`,
        provider: 'slack',
        destination: channel.value,
        botToken,
        appToken,
        message,
      });
    });
  });
  return { kind: 'notify', deliveries };
}

function validateLifecycle(
  document: WomlSourceDocument,
  lifecycle: WomlSourceElement,
  flow: ValidatedFlow,
  shadowedServices: readonly string[] = []
): ValidatedLifecycle {
  const children = elementChildren(document, lifecycle);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_LIFECYCLE_ACTION_REQUIRED',
      '<lifecycle> must contain at least one lifecycle hook.',
      lifecycle.openTagSpan
    );
  }
  const executableSteps = collectValidatedSteps(flow.items);
  const allStepIds = new Set(executableSteps.map(step => step.id));
  const seen = new Set<string>();
  for (const hook of children) {
    const order = lifecycleHookOrder.indexOf(
      hook.name as keyof typeof lifecycleHookEvents
    );
    if (order === -1) {
      failValidation(
        document,
        'WOML_LIFECYCLE_ACTION_INVALID',
        `<lifecycle> cannot contain <${hook.name}>.`,
        hook.openTagSpan
      );
    }
    if (seen.has(hook.name)) {
      failValidation(
        document,
        'WOML_LIFECYCLE_DUPLICATE',
        `<${hook.name}> may appear only once in <lifecycle>.`,
        hook.openTagSpan
      );
    }
    seen.add(hook.name);
  }
  const orderedChildren = [...children].sort(
    (left, right) =>
      lifecycleHookOrder.indexOf(
        left.name as keyof typeof lifecycleHookEvents
      ) -
      lifecycleHookOrder.indexOf(
        right.name as keyof typeof lifecycleHookEvents
      )
  );
  const hooks = orderedChildren.map(
    (hook, hookIndex): ValidatedLifecycleHook => {
    const event =
      lifecycleHookEvents[hook.name as keyof typeof lifecycleHookEvents];
    const stepFilter = hook.attributes.steps;
    const isStepHook = event.startsWith('step_');
    let filteredStepIds: readonly string[] | undefined;
    if (stepFilter !== undefined) {
      if (!isStepHook) {
        failValidation(
          document,
          'WOML_LIFECYCLE_STEP_FILTER_INVALID',
          `Attribute "steps" is valid only on step lifecycle hooks.`,
          stepFilter.nameSpan
        );
      }
      const values = stepFilter.value.trim().split(/\s+/).filter(Boolean);
      if (values.length === 0 || new Set(values).size !== values.length) {
        failValidation(
          document,
          'WOML_LIFECYCLE_STEP_FILTER_INVALID',
          'Lifecycle step filters must be a non-empty list of unique step IDs.',
          stepFilter.valueSpan
        );
      }
      for (const id of values) {
        if (!javascriptSafeIdPattern.test(id)) {
          failValidation(
            document,
            'WOML_LIFECYCLE_STEP_FILTER_INVALID',
            `Lifecycle step filter "${id}" is not a valid step ID.`,
            stepFilter.valueSpan
          );
        }
        if (!allStepIds.has(id)) {
          failValidation(
            document,
            'WOML_LIFECYCLE_STEP_UNKNOWN',
            `Lifecycle step filter names unknown executable step "${id}".`,
            stepFilter.valueSpan
          );
        }
      }
      filteredStepIds = values;
    }
    const actionElements = elementChildren(document, hook);
    if (actionElements.length === 0) {
      failValidation(
        document,
        'WOML_LIFECYCLE_ACTION_REQUIRED',
        `<${hook.name}> requires at least one <script> or <notify> action.`,
        hook.openTagSpan
      );
    }
    const actions = actionElements.map(
      (action, actionIndex): ValidatedLifecycleAction => {
        if (action.name === 'script') {
          const body = scriptBody(document, action);
          const analysis = analyzeWomlLifecycleScript(body.source, {
            shadowedServices,
          });
          if (analysis.issue !== undefined) {
            const sourceFile = new SourceFile(document.file, document.source);
            const start = body.span.start.offset + analysis.issue.start;
            failValidation(
              document,
              analysis.issue.code,
              analysis.issue.message,
              sourceFile.span(
                start,
                Math.max(start + 1, body.span.start.offset + analysis.issue.end)
              ),
              analysis.issue.hint
            );
          }
          return {
            kind: 'script',
            source: body.source,
            scriptSpan: body.span,
            scriptAnalysis: analysis,
          };
        }
        if (action.name === 'notify') {
          return validateLifecycleNotify(
            document,
            action,
            event,
            hookIndex,
            actionIndex,
            allStepIds
          );
        }
        failValidation(
          document,
          'WOML_LIFECYCLE_ACTION_INVALID',
          `<${hook.name}> supports <script> and <notify> actions only.`,
          action.openTagSpan
        );
      }
    );
    return {
      event,
      ...(filteredStepIds === undefined ? {} : { stepIds: filteredStepIds }),
      actions,
    };
    }
  );
  return { hooks };
}

function collectScriptAnalyses(
  items: readonly ValidatedFlowItem[],
  analyses = new Map<string, ScriptAnalysis>()
): Map<string, ScriptAnalysis> {
  for (const item of items) {
    if (item.kind === 'step') {
      analyses.set(item.id, item.scriptAnalysis);
      continue;
    }
    if (item.kind === 'parallel') {
      for (const child of item.children) {
        analyses.set(child.id, child.scriptAnalysis);
      }
      continue;
    }
    if (item.kind === 'branch') {
      for (const arm of item.arms) collectScriptAnalyses(arm.items, analyses);
      continue;
    }
    if (item.kind === 'controlChoice' || item.kind === 'switch') {
      for (const arm of item.arms) collectScriptAnalyses(arm.items, analyses);
      continue;
    }
    if (item.kind === 'fork') {
      for (const branch of item.branches) {
        collectScriptAnalyses(branch.items, analyses);
      }
      continue;
    }
    collectScriptAnalyses(item.approvedItems, analyses);
    collectScriptAnalyses(item.rejectedItems, analyses);
  }
  return analyses;
}

const builtInServiceNames = new Set([
  'http',
  'db',
  'storage',
  'cache',
  'events',
  'workflows',
  'state',
  'telegram',
]);

function collectValidatedSteps(
  items: readonly ValidatedFlowItem[],
  steps: ValidatedStep[] = []
): readonly ValidatedStep[] {
  for (const item of items) {
    if (item.kind === 'step') {
      steps.push(item);
    } else if (item.kind === 'parallel') {
      steps.push(...item.children);
    } else if (item.kind === 'branch') {
      for (const arm of item.arms) collectValidatedSteps(arm.items, steps);
    } else if (item.kind === 'controlChoice' || item.kind === 'switch') {
      for (const arm of item.arms) collectValidatedSteps(arm.items, steps);
    } else if (item.kind === 'fork') {
      for (const branch of item.branches) {
        collectValidatedSteps(branch.items, steps);
      }
    } else {
      collectValidatedSteps(item.approvedItems, steps);
      collectValidatedSteps(item.rejectedItems, steps);
    }
  }
  return steps;
}

function collectValidatedNotifications(
  items: readonly ValidatedFlowItem[],
  deliveries: ValidatedNotificationDelivery[] = []
): readonly ValidatedNotificationDelivery[] {
  for (const item of items) {
    if (item.kind === 'approval') {
      deliveries.push(...item.notifications);
      collectValidatedNotifications(item.approvedItems, deliveries);
      collectValidatedNotifications(item.rejectedItems, deliveries);
    } else if (
      item.kind === 'branch' ||
      item.kind === 'controlChoice' ||
      item.kind === 'switch'
    ) {
      for (const arm of item.arms) {
        collectValidatedNotifications(arm.items, deliveries);
      }
    } else if (item.kind === 'fork') {
      for (const branch of item.branches) {
        collectValidatedNotifications(branch.items, deliveries);
      }
    }
  }
  return deliveries;
}

export interface WomlModuleUsageInspection {
  readonly referencedServices: readonly string[];
  readonly referencedModules: readonly string[];
  readonly unusedModules: readonly string[];
}

export function inspectWomlModuleUsage(
  document: WomlSourceDocument
): WomlModuleUsageInspection {
  const validated = validateDocument(document);
  const lifecycleAnalyses =
    validated.lifecycle?.hooks.flatMap(hook =>
      hook.actions.flatMap(action =>
        action.kind === 'script' ? [action.scriptAnalysis] : []
      )
    ) ?? [];
  const referencedServices = [
    ...new Set(
      [
        ...collectValidatedSteps(validated.flow.items).map(
          step => step.scriptAnalysis
        ),
        ...lifecycleAnalyses,
      ].flatMap(analysis => analysis.requiredServices)
    ),
  ].sort();
  const moduleNames = validated.modules.map(module => module.name);
  return {
    referencedServices,
    referencedModules: referencedServices.filter(name =>
      moduleNames.includes(name)
    ),
    unusedModules: moduleNames.filter(
      name => !referencedServices.includes(name)
    ),
  };
}

const scriptRuntimeBindings = [
  'context',
  'attempt',
  'services',
  'secrets',
] as const;

const lifecycleScriptRuntimeBindings = [
  'context',
  'lifecycle',
  'attempt',
  'services',
  'secrets',
] as const;

function withScriptRuntimeBindings(
  nodes: readonly CompiledWorkflowNode[],
  analyses: ReadonlyMap<string, ScriptAnalysis>
): readonly CompiledWorkflowNode[] {
  return nodes.map(node => {
    if (node.handler !== 'runtime.script') return node;
    const scriptRuntime: ScriptRuntimeBindingsV1 = {
      bindingVersion: 1,
      bindings: scriptRuntimeBindings,
      requiredSecrets: analyses.get(node.id)?.requiredSecrets ?? [],
    };
    return { ...node, scriptRuntime };
  });
}

function lowerLifecycle(
  lifecycle: ValidatedLifecycle
): CompiledLifecycleDefinitionV1 {
  return {
    profileVersion: 1,
    hooks: lifecycle.hooks.map(hook => {
      const hookId = `lifecycle:${hook.event}`;
      const actions: CompiledLifecycleActionV1[] = hook.actions.map(
        (action, actionIndex): CompiledLifecycleActionV1 => {
          const actionId = `${hookId}:action:${actionIndex}`;
          if (action.kind === 'script') {
            const scriptRuntime: ScriptRuntimeBindingsV2 = {
              bindingVersion: 2,
              bindings: lifecycleScriptRuntimeBindings,
              requiredSecrets: action.scriptAnalysis.requiredSecrets,
            };
            return {
              actionId,
              handler: 'runtime.lifecycle-script',
              inputs: {
                kind: 'object',
                fields: {
                  source: { kind: 'literal', value: action.source },
                },
              },
              scriptRuntime,
            };
          }
          return {
            actionId,
            handler: 'notification.informational',
            inputs: {
              kind: 'object',
              fields: {
                deliveries: {
                  kind: 'array',
                  items: action.deliveries.map(delivery => ({
                    kind: 'object' as const,
                    fields: {
                      deliveryId: {
                        kind: 'literal' as const,
                        value: delivery.deliveryId,
                      },
                      provider: {
                        kind: 'literal' as const,
                        value: delivery.provider,
                      },
                      destination: {
                        kind: 'literal' as const,
                        value: delivery.destination,
                      },
                      credentials: lowerNotificationCredentials(delivery),
                      message: delivery.message!,
                    },
                  })),
                },
              },
            },
          };
        }
      );
      const compiledHook: CompiledLifecycleHookV1 = {
        hookId,
        event: hook.event,
        ...(hook.stepIds === undefined ? {} : { stepIds: hook.stepIds }),
        actions,
      };
      return compiledHook;
    }),
  };
}

function lowerNotificationCredentials(
  delivery: ValidatedNotificationDelivery
): ValueExpression {
  return delivery.provider === 'slack'
    ? {
        kind: 'object',
        fields: {
          botToken: delivery.botToken,
          appToken: delivery.appToken,
        },
      }
    : {
        kind: 'object',
        fields: { botToken: delivery.botToken },
      };
}

function validateBranchArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>,
  sourceElementName: 'branch' | 'choose',
  context: FlowValidationContext
): ValidatedBranchArm {
  const diagnosticPrefix =
    sourceElementName === 'choose' ? 'WOML_CHOOSE' : 'WOML_BRANCH';
  const testReference =
    arm.name === 'when'
      ? parseExactReference(document, requiredAttribute(document, arm, 'test'))
      : undefined;
  const children = elementChildren(document, arm);
  const results = children.filter(child => child.name === 'result');
  if (results.length !== 1) {
    failValidation(
      document,
      `${diagnosticPrefix}_RESULT_REQUIRED`,
      `<${arm.name}> must contain exactly one <result>.`,
      results[1]?.openTagSpan ?? arm.openTagSpan
    );
  }

  const result = results[0];
  if (children.at(-1) !== result) {
    failValidation(
      document,
      `${diagnosticPrefix}_RESULT_ORDER`,
      `<result> must be the final child of its <${sourceElementName}> arm.`,
      result.openTagSpan
    );
  }
  ensureEmptyElement(document, result);

  const flowChildren = children.slice(0, -1);
  if (flowChildren.length === 0) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<${arm.name}> must contain at least one step item before <result>.`,
      arm.openTagSpan
    );
  }

  const items = flowChildren.map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`, context)
  );
  const resultReference = parseExactReference(
    document,
    requiredAttribute(document, result, 'value')
  );

  if (arm.name === 'when') {
    return {
      kind: 'when',
      element: arm,
      test: testReference,
      items,
      result: resultReference,
    };
  }
  return {
    kind: 'otherwise',
    element: arm,
    items,
    result: resultReference,
  };
}

function validateBranch(
  document: WomlSourceDocument,
  branch: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedBranch {
  const sourceElementName = branch.name as 'branch' | 'choose';
  const diagnosticPrefix =
    sourceElementName === 'choose' ? 'WOML_CHOOSE' : 'WOML_BRANCH';
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, branch, 'id'),
    sourceElementName
  );
  const metadata = flowItemMetadata(document, branch);

  const children = elementChildren(document, branch);
  const whenCount = children.filter(child => child.name === 'when').length;
  if (whenCount === 0) {
    failValidation(
      document,
      `${diagnosticPrefix}_WHEN_REQUIRED`,
      `<${sourceElementName} id="${id}"> requires at least one <when>.`,
      branch.openTagSpan
    );
  }

  const otherwiseIndexes = children.flatMap((child, index) =>
    child.name === 'otherwise' ? [index] : []
  );
  if (otherwiseIndexes.length === 0) {
    failValidation(
      document,
      `${diagnosticPrefix}_OTHERWISE_REQUIRED`,
      `<${sourceElementName} id="${id}"> requires exactly one final <otherwise>.`,
      branch.openTagSpan
    );
  }
  if (otherwiseIndexes.length > 1) {
    failValidation(
      document,
      `${diagnosticPrefix}_OTHERWISE_ORDER`,
      `<otherwise> may appear exactly once and must be the final <${sourceElementName}> case.`,
      children[otherwiseIndexes[1]].openTagSpan
    );
  }
  const otherwiseIndex = otherwiseIndexes[0];
  if (otherwiseIndex !== children.length - 1) {
    failValidation(
      document,
      `${diagnosticPrefix}_OTHERWISE_ORDER`,
      `<otherwise> must be the final child of <${sourceElementName}>.`,
      children[otherwiseIndex].openTagSpan
    );
  }

  for (let index = 0; index < otherwiseIndex; index += 1) {
    if (children[index].name !== 'when') {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        `<${sourceElementName}> cannot contain <${children[index].name}> outside a <when> or <otherwise> arm.`,
        children[index].openTagSpan
      );
    }
  }

  return {
    kind: 'branch',
    sourceElementName,
    id,
    element: branch,
    metadata,
    arms: children.map(arm =>
      validateBranchArm(document, arm, registry, sourceElementName, context)
    ),
  };
}

function validateControlChoiceArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedControlChoiceArm {
  const testReference =
    arm.name === 'when'
      ? parseExactReference(document, requiredAttribute(document, arm, 'test'))
      : undefined;
  const children = elementChildren(document, arm);
  const result = children.find(child => child.name === 'result');
  if (result !== undefined) {
    failValidation(
      document,
      'WOML_CHOOSE_RESULT_REQUIRES_ID',
      'A control-only <choose> must not contain <result>; add an id to the choice to publish a merged result.',
      result.openTagSpan,
      'Either remove every <result>, or add id="..." and end every arm with exactly one <result>.'
    );
  }
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_CHOOSE_ARM_EMPTY',
      `<${arm.name}> in a control-only <choose> must contain at least one flow item.`,
      arm.openTagSpan
    );
  }
  const items = children.map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`, context)
  );
  return arm.name === 'when'
    ? {
        kind: 'when',
        element: arm,
        test: testReference,
        items,
      }
    : { kind: 'otherwise', element: arm, items };
}

function validateControlChoice(
  document: WomlSourceDocument,
  choice: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedControlChoice {
  const metadataAttribute =
    choice.attributes.name ?? choice.attributes.description;
  if (metadataAttribute !== undefined) {
    failValidation(
      document,
      'WOML_CHOOSE_METADATA_REQUIRES_ID',
      `Attribute "${metadataAttribute.name}" is available only on a result-producing <choose id="...">.`,
      metadataAttribute.nameSpan,
      'Remove the display metadata or add an id and one final <result> to every arm.'
    );
  }

  const children = elementChildren(document, choice);
  const whenCount = children.filter(child => child.name === 'when').length;
  if (whenCount === 0) {
    failValidation(
      document,
      'WOML_CHOOSE_WHEN_REQUIRED',
      '<choose> requires at least one <when>.',
      choice.openTagSpan
    );
  }
  const otherwiseIndexes = children.flatMap((child, index) =>
    child.name === 'otherwise' ? [index] : []
  );
  if (otherwiseIndexes.length === 0) {
    failValidation(
      document,
      'WOML_CHOOSE_OTHERWISE_REQUIRED',
      '<choose> requires exactly one final <otherwise>.',
      choice.openTagSpan
    );
  }
  if (otherwiseIndexes.length > 1) {
    failValidation(
      document,
      'WOML_CHOOSE_OTHERWISE_ORDER',
      '<otherwise> may appear exactly once and must be the final <choose> case.',
      children[otherwiseIndexes[1]].openTagSpan
    );
  }
  const otherwiseIndex = otherwiseIndexes[0];
  if (otherwiseIndex !== children.length - 1) {
    failValidation(
      document,
      'WOML_CHOOSE_OTHERWISE_ORDER',
      '<otherwise> must be the final child of <choose>.',
      children[otherwiseIndex].openTagSpan
    );
  }
  for (let index = 0; index < otherwiseIndex; index += 1) {
    if (children[index].name !== 'when') {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        `<choose> cannot contain <${children[index].name}> outside a <when> or <otherwise> arm.`,
        children[index].openTagSpan
      );
    }
  }

  return {
    kind: 'controlChoice',
    element: choice,
    arms: children.map(arm =>
      validateControlChoiceArm(document, arm, registry, context)
    ),
  };
}

function validateSwitchArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext,
  resultProducing: boolean
): ValidatedSwitchArm {
  const children = elementChildren(document, arm);
  const results = children.filter(child => child.name === 'result');
  let result: ValidatedReference | undefined;
  let flowChildren = children;
  if (resultProducing) {
    if (results.length !== 1) {
      failValidation(
        document,
        'WOML_SWITCH_RESULT_REQUIRED',
        `<${arm.name}> must contain exactly one final <result> because its <switch> has an id.`,
        results[1]?.openTagSpan ?? arm.openTagSpan
      );
    }
    if (children.at(-1) !== results[0]) {
      failValidation(
        document,
        'WOML_SWITCH_RESULT_ORDER',
        `<result> must be the final child of <${arm.name}>.`,
        results[0].openTagSpan
      );
    }
    ensureEmptyElement(document, results[0]);
    result = parseExactReference(
      document,
      requiredAttribute(document, results[0], 'value')
    );
    flowChildren = children.slice(0, -1);
  } else if (results.length > 0) {
    failValidation(
      document,
      'WOML_SWITCH_RESULT_REQUIRES_ID',
      'A control-only <switch> cannot contain <result>; add an id to publish one merged result.',
      results[0].openTagSpan
    );
  }
  if (flowChildren.length === 0) {
    failValidation(
      document,
      'WOML_SWITCH_ARM_EMPTY',
      `<${arm.name}> must contain at least one flow item${resultProducing ? ' before <result>' : ''}.`,
      arm.openTagSpan
    );
  }
  const items = flowChildren.map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`, context)
  );
  if (arm.name === 'case') {
    const value = requiredAttribute(document, arm, 'value');
    if (value.value.length === 0) {
      failValidation(
        document,
        'WOML_SWITCH_CASE_VALUE_EMPTY',
        '<case value="..."> requires a non-empty string.',
        value.valueSpan
      );
    }
    return {
      kind: 'case',
      element: arm,
      value: value.value,
      items,
      ...(result === undefined ? {} : { result }),
    };
  }
  return {
    kind: 'default',
    element: arm,
    items,
    ...(result === undefined ? {} : { result }),
  };
}

function validateSwitch(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedSwitch {
  const idAttribute = element.attributes.id;
  const id =
    idAttribute === undefined
      ? undefined
      : registerStructuralId(document, registry, idAttribute, 'switch');
  const metadataAttribute =
    element.attributes.name ?? element.attributes.description;
  if (id === undefined && metadataAttribute !== undefined) {
    failValidation(
      document,
      'WOML_SWITCH_METADATA_REQUIRES_ID',
      `Attribute "${metadataAttribute.name}" is available only on a result-producing <switch id="...">.`,
      metadataAttribute.nameSpan
    );
  }
  const selector = parseExactReference(
    document,
    requiredAttribute(document, element, 'value')
  );
  const children = elementChildren(document, element);
  const caseCount = children.filter(child => child.name === 'case').length;
  if (caseCount === 0) {
    failValidation(
      document,
      'WOML_SWITCH_CASE_REQUIRED',
      '<switch> requires at least one <case>.',
      element.openTagSpan
    );
  }
  const defaults = children.flatMap((child, index) =>
    child.name === 'default' ? [index] : []
  );
  if (defaults.length !== 1 || defaults[0] !== children.length - 1) {
    failValidation(
      document,
      'WOML_SWITCH_DEFAULT_ORDER',
      '<switch> requires exactly one final <default>.',
      children[defaults[1] ?? defaults[0] ?? 0]?.openTagSpan ??
        element.openTagSpan
    );
  }
  const seen = new Set<string>();
  for (let index = 0; index < children.length - 1; index += 1) {
    const child = children[index];
    if (child.name !== 'case') {
      failValidation(
        document,
        'WOML_SWITCH_STRUCTURE_INVALID',
        `<switch> accepts <case> children followed by one <default>; found <${child.name}>.`,
        child.openTagSpan
      );
    }
    const value = requiredAttribute(document, child, 'value');
    if (seen.has(value.value)) {
      failValidation(
        document,
        'WOML_SWITCH_CASE_DUPLICATE',
        `Switch case value "${value.value}" is declared more than once.`,
        value.valueSpan
      );
    }
    seen.add(value.value);
  }
  return {
    kind: 'switch',
    ...(id === undefined ? {} : { id }),
    element,
    ...(id === undefined
      ? {}
      : { metadata: flowItemMetadata(document, element) }),
    selector,
    arms: children.map(arm =>
      validateSwitchArm(document, arm, registry, context, id !== undefined)
    ),
  };
}

interface ForkJoinToken {
  readonly value: string;
  readonly span: SourceSpan;
}

function forkJoinTokens(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): readonly ForkJoinToken[] {
  const sourceFile = new SourceFile(document.file, document.source);
  const tokens = [...attribute.value.matchAll(/[^\t\n\r ]+/g)].map(match => {
    const start = attribute.valueSpan.start.offset + (match.index ?? 0);
    return {
      value: match[0],
      span: sourceFile.span(start, start + match[0].length),
    };
  });
  if (tokens.length === 0) {
    failValidation(
      document,
      'WOML_FORK_JOIN_INVALID',
      'Fork join must be "all", "none", or one or more direct branch IDs.',
      attribute.valueSpan
    );
  }
  return tokens;
}

function validateFork(
  document: WomlSourceDocument,
  fork: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedFork {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, fork, 'id'),
    'fork'
  );
  const children = elementChildren(document, fork);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_FORK_EMPTY',
      `<fork id="${id}"> must contain at least one direct <branch>.`,
      fork.openTagSpan
    );
  }
  const invalidChild = children.find(child => child.name !== 'branch');
  if (invalidChild !== undefined) {
    failValidation(
      document,
      'WOML_FORK_CHILD_INVALID',
      `<fork id="${id}"> accepts direct <branch> children only; found <${invalidChild.name}>.`,
      invalidChild.openTagSpan
    );
  }

  const branchIds = new Set<string>();
  const branchContext: FlowValidationContext = {
    ...context,
    insideForkBranch: true,
  };
  const branches: ValidatedForkBranch[] = children.map(branch => {
    for (const attribute of Object.values(branch.attributes)) {
      if (attribute.name !== 'id') {
        failValidation(
          document,
          'WOML_FORK_BRANCH_ATTRIBUTE_UNSUPPORTED',
          `Fork branch <branch> accepts only "id"; found "${attribute.name}".`,
          attribute.nameSpan
        );
      }
    }
    const idAttribute = requiredAttribute(document, branch, 'id');
    const branchId = validateJavaScriptSafeId(document, idAttribute, 'branch');
    if (branchId === 'all' || branchId === 'none') {
      failValidation(
        document,
        'WOML_FORK_BRANCH_ID_RESERVED',
        `Fork branch ID "${branchId}" is reserved by the join grammar.`,
        idAttribute.valueSpan,
        'Choose a descriptive branch ID such as instagram or analytics.'
      );
    }
    if (branchIds.has(branchId)) {
      failValidation(
        document,
        'WOML_FORK_BRANCH_ID_DUPLICATE',
        `Fork branch ID "${branchId}" is duplicated inside <fork id="${id}">.`,
        idAttribute.valueSpan
      );
    }
    branchIds.add(branchId);
    const body = elementChildren(document, branch);
    if (body.length === 0) {
      failValidation(
        document,
        'WOML_FORK_BRANCH_EMPTY',
        `<branch id="${branchId}"> must contain at least one flow item.`,
        branch.openTagSpan
      );
    }
    return {
      id: branchId,
      element: branch,
      items: body.map(item =>
        validateFlowItem(
          document,
          item,
          registry,
          `<branch id="${branchId}">`,
          branchContext
        )
      ),
    };
  });

  const joinAttribute = fork.attributes.join;
  if (joinAttribute === undefined) {
    return {
      kind: 'fork',
      id,
      element: fork,
      branches,
      joinedBranchIds: branches.map(branch => branch.id),
    };
  }

  const tokens = forkJoinTokens(document, joinAttribute);
  if (tokens.length === 1 && tokens[0].value === 'all') {
    return {
      kind: 'fork',
      id,
      element: fork,
      branches,
      joinedBranchIds: branches.map(branch => branch.id),
    };
  }
  if (tokens.length === 1 && tokens[0].value === 'none') {
    return {
      kind: 'fork',
      id,
      element: fork,
      branches,
      joinedBranchIds: [],
    };
  }

  const selected = new Set<string>();
  for (const token of tokens) {
    if (token.value === 'all' || token.value === 'none') {
      failValidation(
        document,
        'WOML_FORK_JOIN_INVALID',
        `Reserved join value "${token.value}" cannot be mixed with branch IDs.`,
        token.span
      );
    }
    if (!javascriptSafeIdPattern.test(token.value)) {
      failValidation(
        document,
        'WOML_FORK_JOIN_INVALID',
        `Fork join item "${token.value}" must be a JavaScript-safe branch ID.`,
        token.span
      );
    }
    if (!branchIds.has(token.value)) {
      failValidation(
        document,
        'WOML_FORK_JOIN_UNKNOWN_BRANCH',
        `Fork join names unknown direct branch "${token.value}".`,
        token.span
      );
    }
    if (selected.has(token.value)) {
      failValidation(
        document,
        'WOML_FORK_JOIN_DUPLICATE',
        `Fork join lists branch "${token.value}" more than once.`,
        token.span
      );
    }
    selected.add(token.value);
  }

  return {
    kind: 'fork',
    id,
    element: fork,
    branches,
    joinedBranchIds: branches
      .filter(branch => selected.has(branch.id))
      .map(branch => branch.id),
  };
}

function validateParallel(
  document: WomlSourceDocument,
  parallel: WomlSourceElement,
  registry: Set<string>,
  context: FlowValidationContext
): ValidatedParallel {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, parallel, 'id'),
    'parallel'
  );
  const metadata = flowItemMetadata(document, parallel);
  const childElements = elementChildren(document, parallel);

  if (childElements.length === 0) {
    failValidation(
      document,
      'WOML_PARALLEL_EMPTY',
      `<parallel id="${id}"> must contain at least one direct <step> child.`,
      parallel.openTagSpan
    );
  }

  const unsupportedChild = childElements.find(child => child.name !== 'step');
  if (unsupportedChild !== undefined) {
    if (unsupportedChild.name === 'approval') {
      failValidation(
        document,
        'WOML_APPROVAL_PLACEMENT_INVALID',
        `<approval> cannot be a direct child of <parallel id="${id}"> in this profile.`,
        unsupportedChild.openTagSpan,
        'Place the approval before or after the parallel group, or inside a selected branch arm.'
      );
    }
    failValidation(
      document,
      'WOML_PARALLEL_CHILD_UNSUPPORTED',
      `<parallel id="${id}"> accepts direct <step> children only in this profile; found <${unsupportedChild.name}>.`,
      unsupportedChild.openTagSpan,
      'Move branching around the parallel group, or wait for a future explicit lane/sequence construct.'
    );
  }

  const children = childElements.map(child =>
    validateStep(document, child, registry, context.shadowedServices)
  );
  const concurrencyAttribute = parallel.attributes.concurrency;
  let concurrency = children.length;
  if (concurrencyAttribute !== undefined) {
    if (!/^[1-9][0-9]*$/.test(concurrencyAttribute.value)) {
      failValidation(
        document,
        'WOML_PARALLEL_INVALID_CONCURRENCY',
        `Parallel concurrency "${concurrencyAttribute.value}" must be a positive integer.`,
        concurrencyAttribute.valueSpan
      );
    }
    concurrency = Number(concurrencyAttribute.value);
    if (!Number.isSafeInteger(concurrency) || concurrency > children.length) {
      failValidation(
        document,
        'WOML_PARALLEL_INVALID_CONCURRENCY',
        `Parallel concurrency must not exceed its ${children.length} direct child${children.length === 1 ? '' : 'ren'}.`,
        concurrencyAttribute.valueSpan
      );
    }
  }

  const onErrorAttribute = parallel.attributes['on-error'];
  const onError = onErrorAttribute?.value ?? 'fail-fast';
  if (onError !== 'fail-fast' && onError !== 'wait-all') {
    failValidation(
      document,
      'WOML_PARALLEL_INVALID_POLICY',
      `Parallel on-error must be "fail-fast" or "wait-all", found "${onError}".`,
      onErrorAttribute!.valueSpan
    );
  }

  return {
    kind: 'parallel',
    id,
    element: parallel,
    metadata,
    concurrency,
    onError,
    children,
  };
}

function validateFlowItem(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  registry: Set<string>,
  parent: string,
  context: FlowValidationContext
): ValidatedFlowItem {
  if (element.name === 'step')
    return validateStep(
      document,
      element,
      registry,
      context.shadowedServices
    );
  if (element.name === 'branch') {
    if (context.insideForkBranch) {
      failValidation(
        document,
        'WOML_FORK_BRANCH_CHILD_INVALID',
        'A fork-owned <branch> supports <step>, <choose>, <switch>, <parallel>, and <approval> flow items; nested conditional flow uses <choose> or <switch>.',
        element.openTagSpan
      );
    }
    const looksConditional = element.children.some(
      child =>
        child.kind === 'element' &&
        (child.name === 'when' || child.name === 'otherwise')
    );
    if (!looksConditional) {
      failValidation(
        document,
        'WOML_FORK_BRANCH_PLACEMENT_INVALID',
        'A route <branch> is valid only as a direct child of <fork>. Conditional flow uses <choose>.',
        element.openTagSpan,
        'Wrap route branches in <fork>, or rename an intended conditional container to <choose>.'
      );
    }
    return validateBranch(document, element, registry, context);
  }
  if (element.name === 'choose') {
    return element.attributes.id === undefined
      ? validateControlChoice(document, element, registry, context)
      : validateBranch(document, element, registry, context);
  }
  if (element.name === 'switch') {
    return validateSwitch(document, element, registry, context);
  }
  if (element.name === 'case' || element.name === 'default') {
    failValidation(
      document,
      'WOML_SWITCH_ARM_PLACEMENT_INVALID',
      `<${element.name}> is valid only as a direct child of <switch>.`,
      element.openTagSpan
    );
  }
  if (element.name === 'fork') {
    if (context.insideForkBranch) {
      failValidation(
        document,
        'WOML_FORK_NESTED_UNSUPPORTED',
        'A <fork> cannot appear anywhere inside a fork-owned branch in Fork v1.',
        element.openTagSpan,
        'Move the nested fan-out outside the branch or use <parallel> for direct concurrent steps.'
      );
    }
    return validateFork(document, element, registry, context);
  }
  if (element.name === 'parallel') {
    return validateParallel(document, element, registry, context);
  }
  if (element.name === 'approval') {
    return validateApproval(document, element, registry, context);
  }
  if (element.name === 'when-approved' || element.name === 'when-rejected') {
    failValidation(
      document,
      'WOML_APPROVAL_PLACEMENT_INVALID',
      `<${element.name}> is valid only as a direct child of <approval>.`,
      element.openTagSpan
    );
  }
  if (
    element.name === 'notify' ||
    element.name === 'slack' ||
    element.name === 'telegram'
  ) {
    failValidation(
      document,
      'WOML_NOTIFY_INVALID_ORDER',
      element.name === 'notify'
        ? '<notify> is valid only as the first direct child of <approval>.'
        : `<${element.name}> is valid only as a direct child of <notify>.`,
      element.openTagSpan
    );
  }
  failValidation(
    document,
    'WOML_INVALID_STRUCTURE',
    `${parent} cannot contain <${element.name}> as a flow item.`,
    element.openTagSpan
  );
}

function assertReferenceAvailable(
  document: WomlSourceDocument,
  reference: ValidatedReference,
  allIds: ReadonlySet<string>,
  availableIds: ReadonlySet<string>,
  invisibleForkIds: ReadonlySet<string>
): void {
  const id = reference.structuralId;
  if (id === undefined) return;
  if (!allIds.has(id)) {
    failCompile(
      document,
      'WOML_UNKNOWN_REFERENCE',
      `Reference names unknown structural ID "${id}".`,
      reference.span
    );
  }
  if (!availableIds.has(id)) {
    if (invisibleForkIds.has(id)) {
      failCompile(
        document,
        'WOML_FORK_REFERENCE_NOT_VISIBLE',
        `Output "${id}" belongs to an unjoined or sibling fork branch and is not visible on this route.`,
        reference.span,
        'Reference an output from the current branch, or include the producing branch in the fork join before reading it on the continuation route.'
      );
    }
    failCompile(
      document,
      'WOML_REFERENCE_NOT_DOMINATING',
      `Output "${id}" is not guaranteed to be available at this reference.`,
      reference.span
    );
  }
}

function collectFlowOutputIds(
  items: readonly ValidatedFlowItem[],
  output = new Set<string>()
): Set<string> {
  for (const item of items) {
    if (item.kind === 'step') {
      output.add(item.id);
    } else if (item.kind === 'parallel') {
      for (const child of item.children) output.add(child.id);
    } else if (item.kind === 'branch') {
      output.add(item.id);
      for (const arm of item.arms) collectFlowOutputIds(arm.items, output);
    } else if (item.kind === 'controlChoice' || item.kind === 'switch') {
      if (item.kind === 'switch' && item.id !== undefined) output.add(item.id);
      for (const arm of item.arms) collectFlowOutputIds(arm.items, output);
    } else if (item.kind === 'approval') {
      output.add(item.id);
      collectFlowOutputIds(item.approvedItems, output);
      collectFlowOutputIds(item.rejectedItems, output);
    } else {
      for (const branch of item.branches) {
        collectFlowOutputIds(branch.items, output);
      }
    }
  }
  return output;
}

function validateReferenceAvailability(
  document: WomlSourceDocument,
  items: readonly ValidatedFlowItem[],
  allIds: ReadonlySet<string>,
  availableBefore: ReadonlySet<string> = new Set(),
  invisibleBefore: ReadonlySet<string> = new Set()
): Set<string> {
  const available = new Set(availableBefore);
  const invisible = new Set(invisibleBefore);
  for (const item of items) {
    if (item.kind === 'step') {
      available.add(item.id);
      continue;
    }

    if (item.kind === 'parallel') {
      for (const child of item.children) available.add(child.id);
      continue;
    }

    if (item.kind === 'approval') {
      const armInput = new Set(available);
      armInput.add(item.id);
      validateReferenceAvailability(
        document,
        item.approvedItems,
        allIds,
        armInput,
        new Set(invisible)
      );
      validateReferenceAvailability(
        document,
        item.rejectedItems,
        allIds,
        armInput,
        new Set(invisible)
      );
      available.add(item.id);
      continue;
    }

    if (item.kind === 'fork') {
      const ownedIdsByBranch = new Map(
        item.branches.map(branch => [
          branch.id,
          collectFlowOutputIds(branch.items),
        ])
      );
      const allForkOutputIds = new Set(
        [...ownedIdsByBranch.values()].flatMap(ids => [...ids])
      );
      const guaranteedByBranch = new Map<string, Set<string>>();
      for (const branch of item.branches) {
        const ownIds = ownedIdsByBranch.get(branch.id)!;
        const siblingIds = new Set(invisible);
        for (const forkOutputId of allForkOutputIds) {
          if (!ownIds.has(forkOutputId)) siblingIds.add(forkOutputId);
        }
        guaranteedByBranch.set(
          branch.id,
          validateReferenceAvailability(
            document,
            branch.items,
            allIds,
            new Set(available),
            siblingIds
          )
        );
      }
      const joined = new Set(item.joinedBranchIds);
      for (const branch of item.branches) {
        const owned = ownedIdsByBranch.get(branch.id)!;
        if (joined.has(branch.id)) {
          const guaranteed = guaranteedByBranch.get(branch.id)!;
          for (const id of owned) {
            if (guaranteed.has(id)) available.add(id);
          }
        } else {
          for (const id of owned) invisible.add(id);
        }
      }
      continue;
    }

    if (item.kind === 'controlChoice') {
      for (const arm of item.arms) {
        if (arm.test !== undefined) {
          assertReferenceAvailable(
            document,
            arm.test,
            allIds,
            available,
            invisible
          );
        }
        validateReferenceAvailability(
          document,
          arm.items,
          allIds,
          new Set(available),
          new Set(invisible)
        );
      }
      continue;
    }

    if (item.kind === 'switch') {
      assertReferenceAvailable(
        document,
        item.selector,
        allIds,
        available,
        invisible
      );
      for (const arm of item.arms) {
        const armAvailable = validateReferenceAvailability(
          document,
          arm.items,
          allIds,
          new Set(available),
          new Set(invisible)
        );
        if (arm.result !== undefined) {
          assertReferenceAvailable(
            document,
            arm.result,
            allIds,
            armAvailable,
            invisible
          );
        }
      }
      if (item.id !== undefined) available.add(item.id);
      continue;
    }

    for (const arm of item.arms) {
      if (arm.test !== undefined) {
        assertReferenceAvailable(
          document,
          arm.test,
          allIds,
          available,
          invisible
        );
      }
      const armAvailable = validateReferenceAvailability(
        document,
        arm.items,
        allIds,
        available,
        new Set(invisible)
      );
      assertReferenceAvailable(
        document,
        arm.result,
        allIds,
        armAvailable,
        invisible
      );
    }
    available.add(item.id);
  }
  return available;
}

function validateSteps(
  document: WomlSourceDocument,
  stepsElement: WomlSourceElement,
  shadowedServices: readonly string[] = []
): ValidatedFlow {
  const children = elementChildren(document, stepsElement);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_STEPS',
      '<steps> must contain at least one step item.',
      stepsElement.openTagSpan
    );
  }

  const structuralIds = new Set<string>();
  const items = children.map(child =>
    validateFlowItem(
      document,
      child,
      structuralIds,
      '<steps>',
      { ...rootFlowValidationContext, shadowedServices }
    )
  );
  validateReferenceAvailability(document, items, structuralIds);

  const terminal = items.at(-1);
  if (terminal?.kind === 'parallel') {
    failValidation(
      document,
      'WOML_PARALLEL_TERMINAL_UNSUPPORTED',
      `Root <parallel id="${terminal.id}"> cannot be the final workflow item because a parallel group has no aggregate result.`,
      terminal.element.openTagSpan,
      'Add a downstream <step> that builds the workflow result.'
    );
  }
  if (
    terminal?.kind === 'fork' &&
    !items
      .slice(0, -1)
      .some(
        item =>
          item.kind === 'step' ||
          item.kind === 'branch' ||
          item.kind === 'approval'
      )
  ) {
    failValidation(
      document,
      'WOML_FORK_TERMINAL_RESULT_REQUIRED',
      `Terminal <fork id="${terminal.id}"> has no earlier value-producing main-route item to preserve as the workflow result.`,
      terminal.element.openTagSpan,
      'Add a value-producing <step> before the terminal fork or add a result-building step after it.'
    );
  }

  const findFirstBranch = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'branch') {
        return item.element;
      }
      if (item.kind === 'controlChoice' || item.kind === 'switch') {
        for (const arm of item.arms) {
          const nested = findFirstBranch(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'fork') {
        for (const branch of item.branches) {
          const nested = findFirstBranch(branch.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'approval') {
        const approved = findFirstBranch(item.approvedItems);
        if (approved !== undefined) return approved;
        const rejected = findFirstBranch(item.rejectedItems);
        if (rejected !== undefined) return rejected;
      }
    }
    return undefined;
  };
  const findFirstParallel = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'parallel') return item.element;
      if (item.kind === 'branch') {
        for (const arm of item.arms) {
          const nested = findFirstParallel(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'controlChoice' || item.kind === 'switch') {
        for (const arm of item.arms) {
          const nested = findFirstParallel(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'fork') {
        for (const branch of item.branches) {
          const nested = findFirstParallel(branch.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'approval') {
        const approved = findFirstParallel(item.approvedItems);
        if (approved !== undefined) return approved;
        const rejected = findFirstParallel(item.rejectedItems);
        if (rejected !== undefined) return rejected;
      }
    }
    return undefined;
  };
  const findFirstApproval = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'approval') return item.element;
      if (item.kind === 'branch') {
        for (const arm of item.arms) {
          const nested = findFirstApproval(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'controlChoice' || item.kind === 'switch') {
        for (const arm of item.arms) {
          const nested = findFirstApproval(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'fork') {
        for (const branch of item.branches) {
          const nested = findFirstApproval(branch.items);
          if (nested !== undefined) return nested;
        }
      }
    }
    return undefined;
  };
  const findFirstNotification = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'branch') {
        for (const arm of item.arms) {
          const nested = findFirstNotification(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'controlChoice' || item.kind === 'switch') {
        for (const arm of item.arms) {
          const nested = findFirstNotification(arm.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'fork') {
        for (const branch of item.branches) {
          const nested = findFirstNotification(branch.items);
          if (nested !== undefined) return nested;
        }
      }
      if (item.kind === 'approval') {
        if (item.notifications.length > 0) return item.element;
        const approved = findFirstNotification(item.approvedItems);
        if (approved !== undefined) return approved;
        const rejected = findFirstNotification(item.rejectedItems);
        if (rejected !== undefined) return rejected;
      }
    }
    return undefined;
  };
  const firstBranch = findFirstBranch(items);
  const firstParallel = findFirstParallel(items);
  const firstApproval = findFirstApproval(items);
  const firstNotification = findFirstNotification(items);
  const findFirstKind = (
    flowItems: readonly ValidatedFlowItem[],
    kind: 'fork' | 'controlChoice' | 'switch'
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === kind) return item.element;
      if (
        item.kind === 'branch' ||
        item.kind === 'controlChoice' ||
        item.kind === 'switch'
      ) {
        for (const arm of item.arms) {
          const nested = findFirstKind(arm.items, kind);
          if (nested !== undefined) return nested;
        }
      } else if (item.kind === 'approval') {
        const approved = findFirstKind(item.approvedItems, kind);
        if (approved !== undefined) return approved;
        const rejected = findFirstKind(item.rejectedItems, kind);
        if (rejected !== undefined) return rejected;
      } else if (item.kind === 'fork') {
        for (const branch of item.branches) {
          const nested = findFirstKind(branch.items, kind);
          if (nested !== undefined) return nested;
        }
      }
    }
    return undefined;
  };
  const firstFork = findFirstKind(items, 'fork');
  const firstControlChoice = findFirstKind(items, 'controlChoice');
  const firstSwitch = findFirstKind(items, 'switch');

  return {
    items,
    ...(firstBranch === undefined ? {} : { firstBranch }),
    ...(firstParallel === undefined ? {} : { firstParallel }),
    ...(firstApproval === undefined ? {} : { firstApproval }),
    ...(firstNotification === undefined ? {} : { firstNotification }),
    ...(firstFork === undefined ? {} : { firstFork }),
    ...(firstControlChoice === undefined ? {} : { firstControlChoice }),
    ...(firstSwitch === undefined ? {} : { firstSwitch }),
  };
}

function lowerStep(step: ValidatedStep): LoweredFlowFragment {
  const node: CompiledWorkflowNode = {
    id: step.id,
    handler: 'runtime.script',
    ...(step.metadata === undefined ? {} : { metadata: step.metadata }),
    ...(step.retryPolicy === undefined
      ? {}
      : { retryPolicy: step.retryPolicy }),
    inputs: {
      kind: 'object',
      fields: {
        source: { kind: 'literal', value: step.source },
      },
    },
  };
  return {
    entryId: step.id,
    exitId: step.id,
    nodes: [node],
    edges: [],
  };
}

function referenceExpression(
  reference: ValidatedReference
): ContextReferenceExpression {
  return { kind: 'contextReference', path: reference.path };
}

function alwaysEdge(from: string, to: string): CompiledWorkflowEdge {
  return {
    id: `${from}-to-${to}`,
    from,
    to,
    condition: { kind: 'always' },
  };
}

function lowerBranch(branch: ValidatedBranch): LoweredFlowFragment {
  const selectorId = `__woml_branch__${branch.id}__select`;
  const armFragments = branch.arms.map(arm => lowerFlowItems(arm.items));
  const armIds = branch.arms.map((arm, index) =>
    arm.kind === 'when'
      ? `${branch.id}:when:${index}`
      : `${branch.id}:otherwise`
  );

  const selector: CompiledWorkflowNode = {
    id: selectorId,
    handler: 'engine.branch-select',
    inputs: { kind: 'object', fields: {} },
    ...(branch.metadata === undefined ? {} : { metadata: branch.metadata }),
  };
  const result: CompiledWorkflowNode = {
    id: branch.id,
    handler: 'engine.branch-result',
    inputs: {
      kind: 'object',
      fields: Object.fromEntries(
        branch.arms.map((arm, index) => [
          armIds[index],
          referenceExpression(arm.result),
        ])
      ),
    },
  };

  const selectionEdges: CompiledWorkflowEdge[] = branch.arms.map(
    (arm, index) => ({
      id: armIds[index],
      from: selectorId,
      to: armFragments[index].entryId,
      condition:
        arm.kind === 'when'
          ? {
              kind: 'boolean',
              value: referenceExpression(arm.test!),
            }
          : { kind: 'always' },
      branchId: branch.id,
    })
  );

  return {
    entryId: selectorId,
    exitId: branch.id,
    nodes: [
      selector,
      ...armFragments.flatMap(fragment => fragment.nodes),
      result,
    ],
    edges: [
      ...selectionEdges,
      ...armFragments.flatMap(fragment => fragment.edges),
      ...armFragments.map(fragment => alwaysEdge(fragment.exitId, branch.id)),
    ],
  };
}

function lowerParallel(parallel: ValidatedParallel): LoweredFlowFragment {
  const startId = `__woml_parallel__${parallel.id}__start`;
  const start: CompiledWorkflowNode = {
    id: startId,
    handler: 'engine.parallel-start',
    inputs: {
      kind: 'object',
      fields: {
        concurrency: { kind: 'literal', value: parallel.concurrency },
        onError: { kind: 'literal', value: parallel.onError },
      },
    },
    ...(parallel.metadata === undefined ? {} : { metadata: parallel.metadata }),
  };
  const childFragments = parallel.children.map(lowerStep);
  const join: CompiledWorkflowNode = {
    id: parallel.id,
    handler: 'engine.parallel-join',
    inputs: { kind: 'object', fields: {} },
  };
  const fanOutEdges: CompiledWorkflowEdge[] = childFragments.map(
    (fragment, index) => ({
      id: `${parallel.id}:child:${index}`,
      from: startId,
      to: fragment.entryId,
      condition: { kind: 'always' },
      parallelId: parallel.id,
    })
  );
  const joinEdges: CompiledWorkflowEdge[] = childFragments.map(
    (fragment, index) => ({
      id: `${parallel.id}:join:${index}`,
      from: fragment.exitId,
      to: parallel.id,
      condition: { kind: 'always' },
      parallelId: parallel.id,
    })
  );

  return {
    entryId: startId,
    exitId: parallel.id,
    nodes: [start, ...childFragments.flatMap(fragment => fragment.nodes), join],
    edges: [
      ...fanOutEdges,
      ...childFragments.flatMap(fragment => fragment.edges),
      ...joinEdges,
    ],
  };
}

function approvalRouteEdge(
  approvalId: string,
  decision: 'approved' | 'rejected',
  to: string
): CompiledWorkflowEdge {
  return {
    id: `${approvalId}:${decision}`,
    from: approvalId,
    to,
    condition: {
      kind: 'equals',
      left: {
        kind: 'contextReference',
        path: ['steps', approvalId, 'decision'],
      },
      right: { kind: 'literal', value: decision },
    },
    approvalId,
  };
}

function lowerApproval(approval: ValidatedApproval): LoweredFlowFragment {
  const joinId = `__woml_approval__${approval.id}__join`;
  const approved =
    approval.approvedItems.length === 0
      ? undefined
      : lowerFlowItems(approval.approvedItems);
  const rejected =
    approval.rejectedItems.length === 0
      ? undefined
      : lowerFlowItems(approval.rejectedItems);
  const waitFields = {
    ...(approval.timeoutMs === undefined
      ? {}
      : {
          timeoutMs: {
            kind: 'literal' as const,
            value: approval.timeoutMs,
          },
        }),
    onTimeout: { kind: 'literal' as const, value: approval.onTimeout },
    ...(approval.notifications.length === 0
      ? {}
      : {
          notifications: {
            kind: 'array' as const,
            items: approval.notifications.map(notification => ({
              kind: 'object' as const,
              fields: {
                deliveryId: {
                  kind: 'literal' as const,
                  value: notification.deliveryId,
                },
                provider: {
                  kind: 'literal' as const,
                  value: notification.provider,
                },
                destination: {
                  kind: 'literal' as const,
                  value: notification.destination,
                },
                credentials: lowerNotificationCredentials(notification),
              },
            })),
          },
        }),
  };
  const wait: CompiledWorkflowNode = {
    id: approval.id,
    handler: 'engine.approval-wait',
    inputs: { kind: 'object', fields: waitFields },
    ...(approval.metadata === undefined ? {} : { metadata: approval.metadata }),
  };
  const join: CompiledWorkflowNode = {
    id: joinId,
    handler: 'engine.approval-join',
    inputs: { kind: 'object', fields: {} },
  };
  const routeEdges = [
    approvalRouteEdge(approval.id, 'approved', approved?.entryId ?? joinId),
    approvalRouteEdge(approval.id, 'rejected', rejected?.entryId ?? joinId),
  ];
  const joinEdges: CompiledWorkflowEdge[] = [
    ...(approved === undefined
      ? []
      : [
          {
            id: `${approval.id}:approved:join`,
            from: approved.exitId,
            to: joinId,
            condition: { kind: 'always' as const },
            approvalId: approval.id,
          },
        ]),
    ...(rejected === undefined
      ? []
      : [
          {
            id: `${approval.id}:rejected:join`,
            from: rejected.exitId,
            to: joinId,
            condition: { kind: 'always' as const },
            approvalId: approval.id,
          },
        ]),
  ];

  return {
    entryId: approval.id,
    exitId: joinId,
    nodes: [wait, ...(approved?.nodes ?? []), ...(rejected?.nodes ?? []), join],
    edges: [
      ...routeEdges,
      ...(approved?.edges ?? []),
      ...(rejected?.edges ?? []),
      ...joinEdges,
    ],
  };
}

function lowerFlowItem(item: ValidatedFlowItem): LoweredFlowFragment {
  if (item.kind === 'step') return lowerStep(item);
  if (item.kind === 'branch') return lowerBranch(item);
  if (item.kind === 'parallel') return lowerParallel(item);
  if (item.kind === 'approval') return lowerApproval(item);
  throw new Error(
    'Model v13 flow reached legacy lowering before the FJ3 feature gate.'
  );
}

function lowerFlowItems(
  items: readonly ValidatedFlowItem[]
): LoweredFlowFragment {
  const fragments = items.map(lowerFlowItem);
  const nodes: CompiledWorkflowNode[] = [];
  const edges: CompiledWorkflowEdge[] = [];

  for (let index = 0; index < fragments.length; index += 1) {
    const fragment = fragments[index];
    nodes.push(...fragment.nodes);
    if (index > 0) {
      edges.push(alwaysEdge(fragments[index - 1].exitId, fragment.entryId));
    }
    edges.push(...fragment.edges);
  }

  return {
    entryId: fragments[0].entryId,
    exitId: fragments.at(-1)!.exitId,
    nodes,
    edges,
  };
}

function v13EmptyEngineNode(id: string, handler: string): CompiledWorkflowNode {
  return {
    id,
    handler,
    inputs: { kind: 'object', fields: {} },
  };
}

function lowerStepV13(
  step: ValidatedStep,
  visibleBefore: ReadonlySet<string>,
  state: V13LoweringState
): LoweredV13FlowFragment {
  state.contextVisibility.push({
    nodeId: step.id,
    stepIds: [...visibleBefore],
  });
  const fragment = lowerStep(step);
  return {
    ...fragment,
    visibleAfter: new Set([...visibleBefore, step.id]),
    lastResultNodeId: step.id,
  };
}

function lowerParallelV13(
  parallel: ValidatedParallel,
  visibleBefore: ReadonlySet<string>,
  state: V13LoweringState
): LoweredV13FlowFragment {
  for (const child of parallel.children) {
    state.contextVisibility.push({
      nodeId: child.id,
      stepIds: [...visibleBefore],
    });
  }
  const fragment = lowerParallel(parallel);
  return {
    ...fragment,
    visibleAfter: new Set([
      ...visibleBefore,
      ...parallel.children.map(child => child.id),
    ]),
  };
}

function lowerResultChoiceV13(
  choice: ValidatedBranch,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const selectorId = `__woml_branch__${choice.id}__select`;
  const armFragments = choice.arms.map((arm, index) =>
    lowerFlowItemsV13(
      arm.items,
      new Set(visibleBefore),
      `${path}_arm_${index}`,
      state
    )
  );
  const armIds = choice.arms.map((arm, index) =>
    arm.kind === 'when'
      ? `${choice.id}:when:${index}`
      : `${choice.id}:otherwise`
  );
  const selector: CompiledWorkflowNode = {
    id: selectorId,
    handler: 'engine.branch-select',
    inputs: { kind: 'object', fields: {} },
    ...(choice.metadata === undefined ? {} : { metadata: choice.metadata }),
  };
  const result: CompiledWorkflowNode = {
    id: choice.id,
    handler: 'engine.branch-result',
    inputs: {
      kind: 'object',
      fields: Object.fromEntries(
        choice.arms.map((arm, index) => [
          armIds[index],
          referenceExpression(arm.result),
        ])
      ),
    },
  };
  const selectionEdges: CompiledWorkflowEdge[] = choice.arms.map(
    (arm, index) => ({
      id: armIds[index],
      from: selectorId,
      to: armFragments[index].entryId,
      condition:
        arm.kind === 'when'
          ? { kind: 'boolean', value: referenceExpression(arm.test!) }
          : { kind: 'always' },
      branchId: choice.id,
    })
  );
  return {
    entryId: selectorId,
    exitId: choice.id,
    nodes: [
      selector,
      ...armFragments.flatMap(fragment => fragment.nodes),
      result,
    ],
    edges: [
      ...selectionEdges,
      ...armFragments.flatMap(fragment => fragment.edges),
      ...armFragments.map(fragment => alwaysEdge(fragment.exitId, choice.id)),
    ],
    visibleAfter: new Set([...visibleBefore, choice.id]),
    lastResultNodeId: choice.id,
  };
}

function lowerControlChoiceV13(
  choice: ValidatedControlChoice,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const choiceId = `__woml_choice__${path}`;
  const selectorNodeId = `${choiceId}__select`;
  const joinNodeId = `${choiceId}__join`;
  const armFragments = choice.arms.map((arm, index) =>
    lowerFlowItemsV13(
      arm.items,
      new Set(visibleBefore),
      `${path}_arm_${index}`,
      state
    )
  );
  const armIds = choice.arms.map((arm, index) =>
    arm.kind === 'when' ? `${choiceId}:when:${index}` : `${choiceId}:otherwise`
  );
  state.choices.push({ choiceId, selectorNodeId, joinNodeId, armIds });
  return {
    entryId: selectorNodeId,
    exitId: joinNodeId,
    nodes: [
      v13EmptyEngineNode(selectorNodeId, 'engine.choice-select'),
      ...armFragments.flatMap(fragment => fragment.nodes),
      v13EmptyEngineNode(joinNodeId, 'engine.choice-join'),
    ],
    edges: [
      ...choice.arms.map((arm, index) => ({
        id: armIds[index],
        from: selectorNodeId,
        to: armFragments[index].entryId,
        condition:
          arm.kind === 'when'
            ? {
                kind: 'boolean' as const,
                value: referenceExpression(arm.test!),
              }
            : { kind: 'always' as const },
      })),
      ...armFragments.flatMap(fragment => fragment.edges),
      ...armFragments.map((fragment, index) => ({
        id: `${armIds[index]}:join`,
        from: fragment.exitId,
        to: joinNodeId,
        condition: { kind: 'always' as const },
      })),
    ],
    visibleAfter: new Set(visibleBefore),
  };
}

function lowerSwitchV14(
  switchItem: ValidatedSwitch,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const choiceId = `__woml_choice__${path}_switch`;
  const selectorNodeId = `${choiceId}__select`;
  const joinNodeId = `${choiceId}__join`;
  const armFragments = switchItem.arms.map((arm, index) =>
    lowerFlowItemsV13(
      arm.items,
      new Set(visibleBefore),
      `${path}_arm_${index}`,
      state
    )
  );
  const armIds = switchItem.arms.map((arm, index) =>
    arm.kind === 'case' ? `${choiceId}:case:${index}` : `${choiceId}:default`
  );
  const defaultArmIndex = switchItem.arms.findIndex(
    arm => arm.kind === 'default'
  );
  const descriptor: CompiledControlChoiceV1 = {
    choiceId,
    selectorNodeId,
    joinNodeId,
    armIds,
    stringSelector: referenceExpression(switchItem.selector),
    stringCases: switchItem.arms.flatMap((arm, index) =>
      arm.kind === 'case' ? [{ armId: armIds[index], value: arm.value! }] : []
    ),
    defaultArmId: armIds[defaultArmIndex],
    ...(switchItem.id === undefined ? {} : { resultNodeId: switchItem.id }),
  };
  state.choices.push(descriptor);

  const resultNode =
    switchItem.id === undefined
      ? undefined
      : ({
          id: switchItem.id,
          handler: 'engine.choice-result',
          inputs: {
            kind: 'object',
            fields: Object.fromEntries(
              switchItem.arms.map((arm, index) => [
                armIds[index],
                referenceExpression(arm.result!),
              ])
            ),
          },
          ...(switchItem.metadata === undefined
            ? {}
            : { metadata: switchItem.metadata }),
        } satisfies CompiledWorkflowNode);

  return {
    entryId: selectorNodeId,
    exitId: resultNode?.id ?? joinNodeId,
    nodes: [
      v13EmptyEngineNode(selectorNodeId, 'engine.choice-select'),
      ...armFragments.flatMap(fragment => fragment.nodes),
      v13EmptyEngineNode(joinNodeId, 'engine.choice-join'),
      ...(resultNode === undefined ? [] : [resultNode]),
    ],
    edges: [
      ...switchItem.arms.map((_, index) => ({
        id: armIds[index],
        from: selectorNodeId,
        to: armFragments[index].entryId,
        condition: { kind: 'always' as const },
      })),
      ...armFragments.flatMap(fragment => fragment.edges),
      ...armFragments.map((fragment, index) => ({
        id: `${armIds[index]}:join`,
        from: fragment.exitId,
        to: joinNodeId,
        condition: { kind: 'always' as const },
      })),
      ...(resultNode === undefined
        ? []
        : [alwaysEdge(joinNodeId, resultNode.id)]),
    ],
    visibleAfter:
      switchItem.id === undefined
        ? new Set(visibleBefore)
        : new Set([...visibleBefore, switchItem.id]),
    ...(switchItem.id === undefined ? {} : { lastResultNodeId: switchItem.id }),
  };
}

function lowerApprovalV13(
  approval: ValidatedApproval,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const joinId = `__woml_approval__${approval.id}__join`;
  const armVisible = new Set([...visibleBefore, approval.id]);
  const approved =
    approval.approvedItems.length === 0
      ? undefined
      : lowerFlowItemsV13(
          approval.approvedItems,
          new Set(armVisible),
          `${path}_approved`,
          state
        );
  const rejected =
    approval.rejectedItems.length === 0
      ? undefined
      : lowerFlowItemsV13(
          approval.rejectedItems,
          new Set(armVisible),
          `${path}_rejected`,
          state
        );
  const legacy = lowerApproval({
    ...approval,
    approvedItems: [],
    rejectedItems: [],
  });
  const wait = legacy.nodes[0];
  const join = legacy.nodes.at(-1)!;
  const routeEdges = [
    approvalRouteEdge(approval.id, 'approved', approved?.entryId ?? joinId),
    approvalRouteEdge(approval.id, 'rejected', rejected?.entryId ?? joinId),
  ];
  const joinEdges: CompiledWorkflowEdge[] = [
    ...(approved === undefined
      ? []
      : [
          {
            id: `${approval.id}:approved:join`,
            from: approved.exitId,
            to: joinId,
            condition: { kind: 'always' as const },
            approvalId: approval.id,
          },
        ]),
    ...(rejected === undefined
      ? []
      : [
          {
            id: `${approval.id}:rejected:join`,
            from: rejected.exitId,
            to: joinId,
            condition: { kind: 'always' as const },
            approvalId: approval.id,
          },
        ]),
  ];
  return {
    entryId: approval.id,
    exitId: joinId,
    nodes: [wait, ...(approved?.nodes ?? []), ...(rejected?.nodes ?? []), join],
    edges: [
      ...routeEdges,
      ...(approved?.edges ?? []),
      ...(rejected?.edges ?? []),
      ...joinEdges,
    ],
    visibleAfter: new Set([...visibleBefore, approval.id]),
    lastResultNodeId: approval.id,
  };
}

function lowerForkV13(
  fork: ValidatedFork,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const openNodeId = `__woml_fork__${fork.id}__open`;
  const joinNodeId = `__woml_fork__${fork.id}__join`;
  const branchFragments = fork.branches.map((branch, index) => ({
    branch,
    fragment: lowerFlowItemsV13(
      branch.items,
      new Set(visibleBefore),
      `${path}_branch_${index}`,
      state
    ),
    terminalNodeId: `__woml_fork__${fork.id}__${branch.id}__terminal`,
  }));
  const joined = new Set(fork.joinedBranchIds);
  const descriptor: CompiledForkV1 = {
    forkId: fork.id,
    openNodeId,
    joinNodeId,
    branches: branchFragments.map(({ branch, fragment, terminalNodeId }) => ({
      branchId: branch.id,
      entryNodeId: fragment.entryId,
      terminalNodeId,
    })),
    joinedBranchIds: fork.joinedBranchIds,
  };
  state.forks.push(descriptor);
  state.ownedBranchTerminalNodeIds.push(
    ...descriptor.branches.map(branch => branch.terminalNodeId)
  );

  const visibleAfter = new Set(visibleBefore);
  for (const { branch, fragment } of branchFragments) {
    if (!joined.has(branch.id)) continue;
    const owned = collectFlowOutputIds(branch.items);
    for (const id of owned) {
      if (fragment.visibleAfter.has(id)) visibleAfter.add(id);
    }
  }

  return {
    entryId: openNodeId,
    exitId: joinNodeId,
    nodes: [
      v13EmptyEngineNode(openNodeId, 'engine.fork-open'),
      ...branchFragments.flatMap(({ fragment, terminalNodeId }) => [
        ...fragment.nodes,
        v13EmptyEngineNode(terminalNodeId, 'engine.fork-branch-terminal'),
      ]),
      v13EmptyEngineNode(joinNodeId, 'engine.fork-join'),
    ],
    edges: [
      ...branchFragments.map(({ branch, fragment }) => ({
        id: `${fork.id}:branch:${branch.id}`,
        from: openNodeId,
        to: fragment.entryId,
        condition: { kind: 'always' as const },
      })),
      ...branchFragments.flatMap(({ branch, fragment, terminalNodeId }) => [
        ...fragment.edges,
        {
          id: `${fork.id}:terminal:${branch.id}`,
          from: fragment.exitId,
          to: terminalNodeId,
          condition: { kind: 'always' as const },
        },
      ]),
      ...(fork.joinedBranchIds.length === 0
        ? [
            {
              id: `${fork.id}:join:none`,
              from: openNodeId,
              to: joinNodeId,
              condition: { kind: 'always' as const },
            },
          ]
        : fork.joinedBranchIds.map(branchId => ({
            id: `${fork.id}:join:${branchId}`,
            from: descriptor.branches.find(
              branch => branch.branchId === branchId
            )!.terminalNodeId,
            to: joinNodeId,
            condition: { kind: 'always' as const },
          }))),
    ],
    visibleAfter,
  };
}

function lowerFlowItemV13(
  item: ValidatedFlowItem,
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  if (item.kind === 'step') return lowerStepV13(item, visibleBefore, state);
  if (item.kind === 'parallel')
    return lowerParallelV13(item, visibleBefore, state);
  if (item.kind === 'branch')
    return lowerResultChoiceV13(item, visibleBefore, path, state);
  if (item.kind === 'controlChoice')
    return lowerControlChoiceV13(item, visibleBefore, path, state);
  if (item.kind === 'switch')
    return lowerSwitchV14(item, visibleBefore, path, state);
  if (item.kind === 'approval')
    return lowerApprovalV13(item, visibleBefore, path, state);
  return lowerForkV13(item, visibleBefore, path, state);
}

function lowerFlowItemsV13(
  items: readonly ValidatedFlowItem[],
  visibleBefore: ReadonlySet<string>,
  path: string,
  state: V13LoweringState
): LoweredV13FlowFragment {
  const nodes: CompiledWorkflowNode[] = [];
  const edges: CompiledWorkflowEdge[] = [];
  let visible = new Set(visibleBefore);
  let lastResultNodeId: string | undefined;
  let entryId: string | undefined;
  let exitId: string | undefined;
  for (let index = 0; index < items.length; index += 1) {
    const fragment = lowerFlowItemV13(
      items[index],
      visible,
      `${path}_${index}`,
      state
    );
    entryId ??= fragment.entryId;
    if (exitId !== undefined) edges.push(alwaysEdge(exitId, fragment.entryId));
    nodes.push(...fragment.nodes);
    edges.push(...fragment.edges);
    exitId = fragment.exitId;
    visible = new Set(fragment.visibleAfter);
    lastResultNodeId = fragment.lastResultNodeId ?? lastResultNodeId;
  }
  if (entryId === undefined || exitId === undefined) {
    throw new Error('Model v13 lowering requires a non-empty flow.');
  }
  return {
    entryId,
    exitId,
    nodes,
    edges,
    visibleAfter: visible,
    ...(lastResultNodeId === undefined ? {} : { lastResultNodeId }),
  };
}

function lowerWorkflowV13(items: readonly ValidatedFlowItem[]): {
  readonly fragment: LoweredV13FlowFragment;
  readonly graph: CompiledWorkflowGraphV13;
} {
  const state: V13LoweringState = {
    forks: [],
    choices: [],
    contextVisibility: [],
    ownedBranchTerminalNodeIds: [],
  };
  const fragment = lowerFlowItemsV13(items, new Set(), 'root', state);
  if (fragment.lastResultNodeId === undefined) {
    throw new Error(
      'Model v13 workflow has no deterministic value-producing main-route result.'
    );
  }
  const settlementNodeId = '__woml_workflow__settlement';
  const settlementEdges: CompiledWorkflowEdge[] = [
    {
      id: '__woml_workflow__:main:settlement',
      from: fragment.exitId,
      to: settlementNodeId,
      condition: { kind: 'always' },
    },
    ...state.ownedBranchTerminalNodeIds.map(terminalNodeId => ({
      id: `${terminalNodeId}:settlement`,
      from: terminalNodeId,
      to: settlementNodeId,
      condition: { kind: 'always' as const },
    })),
  ];
  return {
    fragment,
    graph: {
      entryNodeIds: [fragment.entryId],
      nodes: [
        ...fragment.nodes,
        v13EmptyEngineNode(settlementNodeId, 'engine.workflow-settlement'),
      ],
      edges: [...fragment.edges, ...settlementEdges],
      forks: state.forks,
      choices: state.choices,
      contextVisibility: state.contextVisibility,
      settlement: {
        nodeId: settlementNodeId,
        mainResultNodeId: fragment.lastResultNodeId,
        ownedBranchTerminalNodeIds: state.ownedBranchTerminalNodeIds,
      },
    },
  };
}

function lowerTrigger(trigger: ValidatedTrigger): CompiledTrigger {
  if (trigger.kind === 'manual') {
    return {
      id: trigger.id,
      handler: 'trigger.manual',
      config: { kind: 'object', fields: {} },
    };
  }
  if (trigger.kind === 'slack') {
    return {
      id: trigger.id,
      handler: 'trigger.slack',
      config: {
        kind: 'object',
        fields: {
          events: {
            kind: 'array',
            items: trigger.events.map(value => ({ kind: 'literal', value })),
          },
          channels: {
            kind: 'array',
            items: trigger.channels.map(value => ({ kind: 'literal', value })),
          },
          botToken: trigger.botToken,
          appToken: trigger.appToken,
        },
      },
    };
  }
  if (trigger.kind === 'telegram') {
    return {
      id: trigger.id,
      handler: 'trigger.telegram',
      config: {
        kind: 'object',
        fields: {
          events: {
            kind: 'array',
            items: [{ kind: 'literal', value: 'message' }],
          },
          botToken: trigger.botToken,
        },
      },
    };
  }
  if (trigger.kind === 'schedule') {
    return {
      id: trigger.id,
      handler: 'trigger.schedule',
      config: {
        kind: 'object',
        fields: {
          cron: { kind: 'literal', value: trigger.cron },
          timezone: { kind: 'literal', value: trigger.timezone },
          onMissed: { kind: 'literal', value: trigger.onMissed },
        },
      },
    };
  }
  if (trigger.kind === 'interval') {
    return {
      id: trigger.id,
      handler: 'trigger.interval',
      config: {
        kind: 'object',
        fields: {
          everyMs: { kind: 'literal', value: trigger.everyMs },
          onMissed: { kind: 'literal', value: trigger.onMissed },
        },
      },
    };
  }
  if (trigger.kind === 'event') {
    return {
      id: trigger.id,
      handler: 'trigger.event',
      config: {
        kind: 'object',
        fields: {
          name: { kind: 'literal', value: trigger.name },
          ...(trigger.secret === undefined ? {} : { secret: trigger.secret }),
          ...(trigger.schema === undefined
            ? {}
            : { schema: { kind: 'literal' as const, value: trigger.schema } }),
        },
      },
    };
  }
  const authentication: ValueExpression =
    trigger.authentication.kind === 'none'
      ? {
          kind: 'object' as const,
          fields: {
            kind: { kind: 'literal' as const, value: 'none' },
          },
        }
      : {
          kind: 'object' as const,
          fields: {
            kind: { kind: 'literal' as const, value: 'bearer' },
            secret: trigger.authentication.secret,
          },
        };
  return {
    id: trigger.id,
    handler: 'trigger.webhook',
    config: {
      kind: 'object',
      fields: {
        path: { kind: 'literal', value: trigger.path },
        method: { kind: 'literal', value: trigger.method },
        authentication,
        ...(trigger.schema === undefined
          ? {}
          : { schema: { kind: 'literal' as const, value: trigger.schema } }),
      },
    },
  };
}

function validateModuleDeclarations(
  document: WomlSourceDocument,
  importsElement: WomlSourceElement
): readonly ValidatedModuleDeclaration[] {
  const children = elementChildren(document, importsElement);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_IMPORTS_EMPTY',
      '<imports> requires at least one <module> declaration.',
      importsElement.openTagSpan,
      'Remove the empty <imports> container when the workflow has no modules.'
    );
  }

  const declarations: ValidatedModuleDeclaration[] = [];
  const aliases = new Set<string>();
  const sources = new Set<string>();
  for (const child of children) {
    if (child.name !== 'module') {
      failValidation(
        document,
        'WOML_IMPORTS_INVALID_CHILD',
        `<imports> accepts <module> declarations only; found <${child.name}>.`,
        child.openTagSpan
      );
    }
    ensureEmptyElement(document, child);
    const nameAttribute = requiredAttribute(document, child, 'name');
    const fromAttribute = requiredAttribute(document, child, 'from');
    const name = nameAttribute.value;
    const from = fromAttribute.value;

    if (name.length > 128 || !moduleAliasPattern.test(name)) {
      failValidation(
        document,
        'WOML_MODULE_ALIAS_INVALID',
        `Module name "${name}" must be a JavaScript-safe lower-camel alias.`,
        nameAttribute.valueSpan,
        'Example: spreadsheet or customerTools'
      );
    }
    if (reservedModuleAliases.has(name)) {
      failValidation(
        document,
        'WOML_MODULE_ALIAS_RESERVED',
        `Module name "${name}" is reserved by WOML.`,
        nameAttribute.valueSpan,
        'Choose a workflow-specific module name.'
      );
    }
    if (aliases.has(name)) {
      failValidation(
        document,
        'WOML_MODULE_ALIAS_DUPLICATE',
        `Module name "${name}" is declared more than once.`,
        nameAttribute.valueSpan
      );
    }
    aliases.add(name);

    if (from.endsWith('.woml')) {
      failValidation(
        document,
        'WOML_MODULE_WORKFLOW_UNSUPPORTED',
        '<module> imports JavaScript or TypeScript code, not .woml workflows.',
        fromAttribute.valueSpan,
        'Use services.events.emit() for one-to-many workflow triggers. Durable services.workflows.call() is the next roadmap milestone.'
      );
    }
    if (
      from.includes('\\') ||
      from.includes('\0') ||
      from.includes('?') ||
      from.includes('#') ||
      !moduleSourcePattern.test(from)
    ) {
      failValidation(
        document,
        'WOML_MODULE_PATH_INVALID',
        `Module source "${from}" must be an explicit relative POSIX path ending in .js or .ts.`,
        fromAttribute.valueSpan,
        'Example: ./modules/spreadsheet.ts'
      );
    }
    if (sources.has(from)) {
      failValidation(
        document,
        'WOML_MODULE_SOURCE_DUPLICATE',
        `Module source "${from}" is already declared under another name.`,
        fromAttribute.valueSpan,
        'Declare one stable services alias for each module entry point.'
      );
    }
    sources.add(from);

    declarations.push({
      name,
      from,
      element: child,
      nameAttribute,
      fromAttribute,
    });
  }
  return declarations;
}

function validateDocument(document: WomlSourceDocument): ValidatedWorkflow {
  const root = document.root;
  if (root.name !== 'woml') {
    failValidation(
      document,
      'WOML_EXPECTED_DOCUMENT_ROOT',
      `Expected <woml> as the document root, found <${root.name}>.`,
      root.openTagSpan,
      root.name === 'workflow'
        ? 'Wrap the existing <workflow> with <woml>...</woml>.'
        : 'A WOML document contains optional <imports> followed by exactly one <workflow>.'
    );
  }

  validateSecretReferenceSinks(document, root);
  visitProfile(document, root);

  const documentChildren = elementChildren(document, root);
  let importsElement: WomlSourceElement | undefined;
  let workflow: WomlSourceElement | undefined;
  if (
    documentChildren.length === 1 &&
    documentChildren[0].name === 'workflow'
  ) {
    workflow = documentChildren[0];
  } else if (
    documentChildren.length === 2 &&
    documentChildren[0].name === 'imports' &&
    documentChildren[1].name === 'workflow'
  ) {
    importsElement = documentChildren[0];
    workflow = documentChildren[1];
  } else {
    const misplaced =
      documentChildren.find(
        child => child.name !== 'imports' && child.name !== 'workflow'
      ) ??
      documentChildren[0] ??
      root;
    failValidation(
      document,
      'WOML_DOCUMENT_STRUCTURE_INVALID',
      '<woml> requires optional <imports> followed by exactly one <workflow>.',
      misplaced.openTagSpan,
      'Use <woml><imports>...</imports><workflow ...>...</workflow></woml>.'
    );
  }

  const modules =
    importsElement === undefined
      ? []
      : validateModuleDeclarations(document, importsElement);

  if (workflow === undefined) {
    throw new Error('validated WOML document did not contain a workflow');
  }

  const workflowId = validateWorkflowId(
    document,
    requiredAttribute(document, workflow, 'id')
  );
  const metadata = workflowMetadata(document, workflow);
  const [configElement, lifecycleElement, triggersElement, stepsElement] =
    validateWorkflowChildren(document, workflow);
  const runtimePolicy =
    configElement === undefined
      ? undefined
      : validateRuntimePolicy(document, configElement);
  const triggers =
    triggersElement === undefined
      ? []
      : validateTriggers(document, triggersElement);
  const shadowedServices = modules
    .filter(module => module.name === 'telegram')
    .map(module => module.name);
  const flow = validateSteps(document, stepsElement, shadowedServices);
  const lifecycle =
    lifecycleElement === undefined
      ? undefined
      : validateLifecycle(document, lifecycleElement, flow, shadowedServices);

  return {
    element: workflow,
    modules,
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers,
    flow,
    ...(lifecycle === undefined ? {} : { lifecycle }),
    ...(runtimePolicy === undefined ? {} : { runtimePolicy }),
  };
}

export function inspectValidatedWomlDocument(
  document: WomlSourceDocument
): ValidatedWorkflow {
  return validateDocument(document);
}

/**
 * Returns non-fatal source migration diagnostics after the document has been
 * validated. Diagnostics never affect compiled identity or runtime stdout.
 */
export function inspectWomlMigrationDiagnostics(
  document: WomlSourceDocument
): readonly WomlAdvisoryDiagnostic[] {
  validateDocument(document);
  const diagnostics: WomlAdvisoryDiagnostic[] = [];

  const inspected = inspectWomlDocument(document);
  if (inspected.kind === 'workflow') {
    for (const imported of inspected.imports) {
      if (imported.name === 'telegram') {
        diagnostics.push({
          severity: 'warning',
          code: 'WOML_BUILTIN_SERVICE_SHADOWED',
          phase: 'validation',
          message:
            'The local module alias "telegram" shadows the built-in services.telegram capability in this workflow.',
          file: document.file,
          location: imported.element.openTagSpan,
          hint:
            'Keep this name to preserve the local module, or rename it when you want services.telegram.send().',
        });
      }
    }
  }

  const visit = (
    element: WomlSourceElement,
    parentName: string | undefined
  ): void => {
    if (element.name === 'branch' && parentName !== 'fork') {
      diagnostics.push({
        severity: 'warning',
        code: 'WOML_DEPRECATED_CONDITIONAL_BRANCH',
        phase: 'validation',
        message:
          'Conditional <branch> is deprecated; use <choose> for mutually exclusive conditions.',
        file: document.file,
        location: element.openTagSpan,
        hint: 'Rename the opening <branch> tag to <choose> and the matching closing </branch> tag to </choose>. The id, cases, result, and runtime behavior stay unchanged.',
      });
    }
    for (const child of element.children) {
      if (child.kind === 'element') visit(child, element.name);
    }
  };

  visit(document.root, undefined);
  return diagnostics;
}

export function validateWoml(document: WomlSourceDocument): void {
  const inspection = inspectWomlDocument(document);
  if (
    inspection.kind !== 'workflow' ||
    inspection.imports.some(item => item.kind === 'reusable-definition')
  ) {
    return;
  }
  validateDocument(document);
}

function placeholderAttribute(
  source: WomlSourceElement,
  name: string,
  value: string
): WomlSourceAttribute {
  return {
    name,
    value,
    span: source.openTagSpan,
    nameSpan: source.openTagSpan,
    valueSpan: source.openTagSpan,
  };
}

/**
 * Runs the existing workflow grammar/DAG/reference validator after reusable
 * tags have been resolved, without pretending those tags are executable.
 * Placeholder nodes preserve custom-step IDs and route positions. The real
 * Model v14 lowering remains gated until SCP3/SCP5.
 */
export function validateResolvedReusableWorkflow(
  document: WomlSourceDocument,
  graph: WomlReusableDefinitionGraph
): void {
  if (graph.root.kind !== 'workflow') return;
  const definitions = new Map(
    graph.definitions.map(definition => [definition.alias, definition.kind])
  );

  const transform = (
    element: WomlSourceElement,
    ancestors: readonly string[] = []
  ): WomlSourceElement => {
    const definitionKind = definitions.get(element.name);
    if (definitionKind === 'reusable-step') {
      const attributes = Object.fromEntries(
        Object.entries(element.attributes).filter(([name]) =>
          new Set([
            'id',
            'name',
            'description',
            'retry',
            'retry-delay',
            'retry-backoff',
            'retry-max-delay',
          ]).has(name)
        )
      );
      const raw: WomlSourceRawText = {
        kind: 'raw',
        value: 'return null;',
        span: element.openTagSpan,
      };
      const script: WomlSourceElement = {
        kind: 'element',
        name: 'script',
        attributes: {},
        children: [raw],
        span: element.span,
        openTagSpan: element.openTagSpan,
      };
      return { ...element, name: 'step', attributes, children: [script] };
    }
    if (definitionKind === 'notification-provider') {
      if (ancestors.at(-1) === 'triggers') return element;
      const informational = ancestors.some(name => name.startsWith('on-'));
      const attributes: Record<string, WomlSourceAttribute> = {
        channels: placeholderAttribute(element, 'channels', '#custom-provider'),
        'bot-token': placeholderAttribute(
          element,
          'bot-token',
          '{{secrets.WOML_CUSTOM_PROVIDER_PLACEHOLDER}}'
        ),
        'app-token': placeholderAttribute(
          element,
          'app-token',
          '{{secrets.WOML_CUSTOM_PROVIDER_PLACEHOLDER}}'
        ),
      };
      if (informational) {
        attributes.message =
          element.attributes.message ??
          placeholderAttribute(element, 'message', 'Custom notification');
      }
      return { ...element, name: 'slack', attributes, children: [] };
    }

    let children = element.children.map(child =>
      child.kind === 'element'
        ? transform(child, [...ancestors, element.name])
        : child
    );
    if (element.name === 'imports') {
      children = children.filter(
        child =>
          child.kind !== 'element' ||
          child.attributes.from?.value.endsWith('.woml') !== true
      );
    }
    return { ...element, children };
  };

  let root = transform(document.root);
  root = {
    ...root,
    children: root.children.filter(
      child =>
        child.kind !== 'element' ||
        child.name !== 'imports' ||
        child.children.length > 0
    ),
  };
  validateDocument({ ...document, root });
}

export interface PreparedReusableStepDefinition {
  readonly kind: 'reusable-step' | 'notification-provider';
  readonly alias: string;
  readonly source: string;
  readonly digest: string;
  readonly scriptArtifactId: string;
  readonly scriptSource: string;
  readonly lifecycleScripts: readonly {
    readonly hook: 'on-success' | 'on-error' | 'on-complete';
    readonly index: number;
    readonly source: string;
  }[];
  readonly imports: readonly {
    readonly name: string;
    readonly runtimeName: string;
    readonly from: string;
  }[];
  readonly props: WomlDocumentInspection['props'];
}

export interface PreparedReusableWorkflow {
  readonly document: WomlSourceDocument;
  readonly rootSource: string;
  readonly definitions: readonly PreparedReusableStepDefinition[];
  readonly invocations: readonly CompiledReusableStepInvocationV1[];
  readonly providerInvocations: readonly CompiledReusableNotificationProviderV1[];
  readonly providerDeliveries: readonly {
    readonly providerId: string;
    readonly domain: 'approval' | 'informational';
    readonly ownerNodeId?: string;
    readonly message?: ValueExpression;
    readonly messageAttribute?: WomlSourceAttribute;
  }[];
  readonly invocationAttributes: ReadonlyMap<string, Readonly<Record<string, WomlSourceAttribute>>>;
}

function reusableRuntimeName(alias: string, moduleName: string): string {
  const upper = (value: string): string =>
    value
      .split('-')
      .filter(Boolean)
      .map(part => `${part[0]?.toUpperCase() ?? ''}${part.slice(1)}`)
      .join('');
  return `reusable${upper(alias)}${upper(moduleName)}`;
}

function portableRelative(fromDirectory: string, target: string): string {
  const value = relative(fromDirectory, target).split(sep).join('/');
  return value.startsWith('.') ? value : `./${value}`;
}

function syntheticAttribute(
  name: string,
  value: string,
  span: SourceSpan
): WomlSourceAttribute {
  return { name, value, span, nameSpan: span, valueSpan: span };
}

function reusableLifecycleIds(
  invocationId: string,
  lifecycle: WomlSourceElement | undefined
): CompiledReusableStepInvocationV1['lifecycle'] | undefined {
  if (lifecycle === undefined) return undefined;
  const result: {
    onSuccess?: readonly string[];
    onError?: readonly string[];
    onComplete?: readonly string[];
  } = {};
  for (const child of lifecycle.children) {
    if (child.kind !== 'element') continue;
    const ids = child.children
      .filter((action): action is WomlSourceElement => action.kind === 'element')
      .map((_, index) => `${invocationId}:${child.name}:${index}`);
    if (child.name === 'on-success') result.onSuccess = ids;
    if (child.name === 'on-error') result.onError = ids;
    if (child.name === 'on-complete') result.onComplete = ids;
  }
  return Object.keys(result).length === 0 ? undefined : result;
}

function parseProviderUsageMessage(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): ValueExpression {
  if (attribute.value.length === 0 || attribute.value.length > 16_384) {
    failValidation(
      document,
      'WOML_PROVIDER_MESSAGE_INVALID',
      'Custom provider messages must contain 1 through 16384 characters.',
      attribute.valueSpan
    );
  }
  const parts: ({ readonly kind: 'text'; readonly text: string } | ContextReferenceExpression)[] = [];
  let cursor = 0;
  let placeholders = 0;
  for (const match of attribute.value.matchAll(/\{\{([^{}]+)\}\}/g)) {
    const start = match.index ?? 0;
    const preceding = attribute.value.slice(cursor, start);
    if (preceding.includes('{{') || preceding.includes('}}')) {
      failValidation(
        document,
        'WOML_PROVIDER_MESSAGE_INVALID',
        'Custom provider message contains an unmatched WOML template delimiter.',
        attribute.valueSpan
      );
    }
    if (preceding.length > 0) parts.push({ kind: 'text', text: preceding });
    const reference = match[1].trim();
    const parsed = /^context\.(payload(?:\.[A-Za-z_$][A-Za-z0-9_$]*)+|steps\.[a-z][A-Za-z0-9]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)+)$/.exec(reference);
    if (parsed === null) {
      failValidation(
        document,
        'WOML_PROVIDER_MESSAGE_INVALID',
        `Unsupported custom provider message reference "${reference}".`,
        attribute.valueSpan,
        'Use scalar context.payload or context.steps references only.'
      );
    }
    parts.push({
      kind: 'contextReference',
      path: parsed[1].split('.').map((segment, index) =>
        index === 0 && segment === 'payload' ? 'trigger' : segment
      ),
    });
    placeholders += 1;
    cursor = start + match[0].length;
  }
  const tail = attribute.value.slice(cursor);
  if (tail.includes('{{') || tail.includes('}}') || placeholders > 32) {
    failValidation(
      document,
      'WOML_PROVIDER_MESSAGE_INVALID',
      placeholders > 32
        ? 'Custom provider messages may contain at most 32 placeholders.'
        : 'Custom provider message contains an unmatched WOML template delimiter.',
      attribute.valueSpan
    );
  }
  if (tail.length > 0) parts.push({ kind: 'text', text: tail });
  return { kind: 'template', parts };
}

function boundReusableProps(
  props: WomlDocumentInspection['props'],
  element: WomlSourceElement
): CompiledReusableStepInvocationV1['props'] {
  return props.flatMap(prop => {
    const attribute = element.attributes[prop.name];
    if (attribute === undefined) return [];
    const secret = /^\{\{secrets\.([A-Z][A-Z0-9_]*)\}\}$/.exec(attribute.value);
    const context = /^\{\{context\.(.+)\}\}$/.exec(attribute.value);
    return [{
      name: prop.name,
      bindingName: prop.bindingName,
      secret: prop.secret,
      expression: secret !== null
        ? { kind: 'secret' as const, name: secret[1] }
        : context !== null
          ? { kind: 'context' as const, path: context[1] }
          : { kind: 'literal' as const, value: attribute.value },
    }];
  });
}

/**
 * Replaces each resolved reusable-step tag with an ordinary workflow step.
 * The returned document is an internal compiler view only: source provenance
 * stays attached to the Model v14 descriptor and the user's source is never
 * rewritten on disk.
 */
export function prepareResolvedReusableWorkflow(
  document: WomlSourceDocument,
  graph: WomlReusableDefinitionGraph,
  options: { readonly projectRoot: string }
): PreparedReusableWorkflow {
  if (graph.root.kind !== 'workflow') {
    failCompile(
      document,
      'WOML_DEFINITION_NOT_RUNNABLE',
      'Only a workflow document can be lowered into a reusable-step workflow.',
      graph.root.definition.openTagSpan
    );
  }
  validateResolvedReusableWorkflow(document, graph);
  const rootDirectory = dirname(resolve(document.file));
  const directDefinitions = new Map(
    graph.root.imports
      .filter(item => item.kind === 'reusable-definition')
      .map(item => [
        item.name,
        relative(
          options.projectRoot,
          resolve(dirname(document.file), item.from)
        ).split(sep).join('/'),
      ])
  );
  const definitions = new Map<string, {
    prepared: PreparedReusableStepDefinition;
    inspection: WomlDocumentInspection;
    script: WomlSourceElement;
  }>();
  for (const resolvedDefinition of graph.definitions) {
    if (
      directDefinitions.get(resolvedDefinition.alias) !== resolvedDefinition.sourcePath ||
      definitions.has(resolvedDefinition.alias)
    ) continue;
    const absolutePath = resolve(options.projectRoot, resolvedDefinition.sourcePath);
    const definitionDocument = parseWoml(readFileSync(absolutePath, 'utf8'), {
      file: absolutePath,
    });
    const inspection = inspectWomlDocument(definitionDocument);
    if (inspection.kind === 'workflow') continue;
    const script = inspection.definition.children.find(
      (child): child is WomlSourceElement =>
        child.kind === 'element' && child.name === 'script'
    )!;
    const raw = script.children[0];
    const originalSource = raw?.kind === 'raw' ? raw.value : '';
    const analysis = inspection.kind === 'reusable-step'
      ? analyzeWomlReusableScript(originalSource)
      : analyzeWomlNotificationProviderScript(originalSource);
    if (analysis.issue !== undefined) {
      const sourceFile = new SourceFile(definitionDocument.file, definitionDocument.source);
      const start = script.children[0].span.start.offset + analysis.issue.start;
      throw new WomlCompileError({
        phase: 'compile',
        code: analysis.issue.code,
        message: analysis.issue.message,
        file: definitionDocument.file,
        location: sourceFile.span(
          start,
          start + Math.max(1, analysis.issue.end - analysis.issue.start)
        ),
        ...(analysis.issue.hint === undefined ? {} : { hint: analysis.issue.hint }),
      });
    }
    const moduleImports = inspection.imports
      .filter(item => item.kind === 'script-module')
      .map(item => ({
        name: item.name,
        runtimeName: reusableRuntimeName(resolvedDefinition.alias, item.name),
        from: portableRelative(
          rootDirectory,
          resolve(dirname(absolutePath), item.from)
        ),
      }));
    const rewriteServices = (value: string): string => {
      let rewritten = value;
      for (const imported of moduleImports) {
        rewritten = rewritten.replace(
          new RegExp(`\\bservices\\.${imported.name}\\b`, 'g'),
          `services.${imported.runtimeName}`
        );
      }
      return rewritten;
    };
    const scriptSource = rewriteServices(originalSource);
    const scriptArtifactId = `reusable_${resolvedDefinition.alias.replaceAll('-', '_')}_${resolvedDefinition.digest.slice(7, 23)}`;
    const lifecycleScripts = inspection.lifecycle?.children.flatMap(hook => {
      if (hook.kind !== 'element') return [];
      return hook.children.flatMap((action, index) => {
        if (action.kind !== 'element' || action.name !== 'script') return [];
        const body = action.children[0];
        const source = body?.kind === 'raw' ? body.value : '';
        const analysis = analyzeWomlLifecycleScript(source);
        if (analysis.issue !== undefined) {
          const sourceFile = new SourceFile(definitionDocument.file, definitionDocument.source);
          const start = (body?.span.start.offset ?? action.openTagSpan.start.offset) + analysis.issue.start;
          throw new WomlCompileError({
            phase: 'compile',
            code: analysis.issue.code,
            message: analysis.issue.message,
            file: definitionDocument.file,
            location: sourceFile.span(
              start,
              start + Math.max(1, analysis.issue.end - analysis.issue.start)
            ),
            ...(analysis.issue.hint === undefined ? {} : { hint: analysis.issue.hint }),
          });
        }
        return [{
          hook: hook.name as 'on-success' | 'on-error' | 'on-complete',
          index,
          source: rewriteServices(source),
        }];
      });
    }) ?? [];
    const prepared: PreparedReusableStepDefinition = {
      kind: inspection.kind,
      alias: resolvedDefinition.alias,
      source: resolvedDefinition.sourcePath,
      digest: resolvedDefinition.digest,
      scriptArtifactId,
      scriptSource,
      lifecycleScripts,
      imports: moduleImports,
      props: inspection.props,
    };
    definitions.set(resolvedDefinition.alias, { prepared, inspection, script });
  }

  const invocations: CompiledReusableStepInvocationV1[] = [];
  const providerInvocations: CompiledReusableNotificationProviderV1[] = [];
  const providerDeliveries: PreparedReusableWorkflow['providerDeliveries'][number][] = [];
  const invocationAttributes = new Map<string, Readonly<Record<string, WomlSourceAttribute>>>();
  const transform = (
    element: WomlSourceElement,
    ancestors: readonly WomlSourceElement[] = []
  ): WomlSourceElement => {
    const found = definitions.get(element.name);
    if (found?.inspection.kind === 'notification-provider') {
      if (ancestors.at(-1)?.name === 'triggers') return element;
      const notify = ancestors.at(-1);
      const providerIndex = notify?.children
        .filter((child): child is WomlSourceElement => child.kind === 'element')
        .indexOf(element) ?? -1;
      const approval = [...ancestors].reverse().find(item => item.name === 'approval');
      const lifecycle = ancestors.find(item => item.name === 'lifecycle');
      const hook = lifecycle === undefined
        ? undefined
        : ancestors.find(item => item.name.startsWith('on-'));
      const domain = approval === undefined ? 'informational' as const : 'approval' as const;
      let providerId: string;
      let ownerNodeId: string | undefined;
      if (approval !== undefined) {
        ownerNodeId = approval.attributes.id!.value;
        providerId = `${ownerNodeId}:notify:${providerIndex}:channel:0`;
      } else {
        const hookIndex = lifecycle!.children
          .filter((child): child is WomlSourceElement => child.kind === 'element')
          .indexOf(hook!);
        const actionIndex = hook!.children
          .filter((child): child is WomlSourceElement => child.kind === 'element')
          .indexOf(notify!);
        providerId = `lifecycle:${hookIndex}:action:${actionIndex}:provider:${providerIndex}:channel:0`;
      }
      const messageAttribute = element.attributes.message;
      providerInvocations.push({
        kind: 'notification-provider',
        providerId,
        alias: found.prepared.alias,
        definitionDigest: found.prepared.digest,
        source: found.prepared.source,
        scriptArtifactId: found.prepared.scriptArtifactId,
        props: boundReusableProps(found.prepared.props, element),
        ...(found.inspection.lifecycle === undefined
          ? {}
          : { lifecycle: reusableLifecycleIds(providerId, found.inspection.lifecycle) }),
      });
      providerDeliveries.push({
        providerId,
        domain,
        ...(ownerNodeId === undefined ? {} : { ownerNodeId }),
        ...(domain === 'approval' && messageAttribute !== undefined
          ? {
              message: parseProviderUsageMessage(document, messageAttribute),
              messageAttribute,
            }
          : {}),
      });
      const attributes: Record<string, WomlSourceAttribute> = {
        channels: syntheticAttribute('channels', '#custom-provider', element.openTagSpan),
        'bot-token': syntheticAttribute(
          'bot-token',
          '{{secrets.WOML_CUSTOM_PROVIDER_PLACEHOLDER}}',
          element.openTagSpan
        ),
        'app-token': syntheticAttribute(
          'app-token',
          '{{secrets.WOML_CUSTOM_PROVIDER_PLACEHOLDER}}',
          element.openTagSpan
        ),
      };
      if (domain === 'informational') attributes.message = messageAttribute!;
      return { ...element, name: 'slack', attributes, children: [] };
    }
    if (found?.inspection.kind === 'reusable-step') {
      const invocationId = element.attributes.id!.value;
      const definitionAttributes = found.inspection.definition.attributes;
      const attributes: Record<string, WomlSourceAttribute> = {};
      for (const name of [
        'id',
        'name',
        'description',
        'retry',
        'retry-delay',
        'retry-backoff',
        'retry-max-delay',
      ]) {
        const attribute = element.attributes[name] ??
          (name === 'name' || name === 'description'
            ? definitionAttributes[name]
            : undefined);
        if (attribute !== undefined) attributes[name] = attribute;
      }
      const boundProps = boundReusableProps(found.prepared.props, element);
      invocations.push({
        kind: 'step',
        invocationId,
        nodeId: invocationId,
        alias: found.prepared.alias,
        definitionDigest: found.prepared.digest,
        source: found.prepared.source,
        scriptArtifactId: found.prepared.scriptArtifactId,
        props: boundProps,
        ...(found.inspection.lifecycle === undefined
          ? {}
          : { lifecycle: reusableLifecycleIds(invocationId, found.inspection.lifecycle) }),
      });
      invocationAttributes.set(invocationId, element.attributes);
      const raw: WomlSourceRawText = {
        kind: 'raw',
        value: found.prepared.scriptSource,
        span: found.script.children[0].span,
      };
      const script: WomlSourceElement = {
        ...found.script,
        children: [raw],
      };
      return { ...element, name: 'step', attributes, children: [script] };
    }
    return {
      ...element,
      children: element.children.map(child =>
        child.kind === 'element' ? transform(child, [...ancestors, element]) : child
      ),
    };
  };

  let root = transform(document.root);
  const imports = root.children.find(
    (child): child is WomlSourceElement =>
      child.kind === 'element' && child.name === 'imports'
  );
  const moduleChildren = [
    ...(imports?.children.filter(
      (child): child is WomlSourceElement =>
        child.kind === 'element' &&
        child.attributes.from?.value.endsWith('.woml') !== true
    ) ?? []),
    ...[...definitions.values()].flatMap(({ prepared, inspection }) =>
      prepared.imports.map(imported => {
        const declaration = inspection.imports.find(item => item.name === imported.name)!;
        return {
          ...declaration.element,
          attributes: {
            name: syntheticAttribute('name', imported.runtimeName, declaration.element.openTagSpan),
            from: syntheticAttribute('from', imported.from, declaration.element.openTagSpan),
          },
        };
      })
    ),
  ];
  const children = root.children.filter(
    child => child.kind !== 'element' || child.name !== 'imports'
  );
  if (moduleChildren.length > 0) {
    const span = imports?.openTagSpan ?? root.openTagSpan;
    children.unshift({
      kind: 'element',
      name: 'imports',
      attributes: {},
      children: moduleChildren,
      span,
      openTagSpan: span,
    });
  }
  root = { ...root, children };
  return {
    document: { ...document, root },
    rootSource: relative(options.projectRoot, resolve(document.file)).split(sep).join('/'),
    definitions: [...definitions.values()].map(value => value.prepared),
    invocations,
    providerInvocations,
    providerDeliveries,
    invocationAttributes,
  };
}

function compileValidatedWoml(
  document: WomlSourceDocument,
  moduleRuntime?: CompiledModuleRuntimeV1,
  forceModelV14 = false,
  forceModelV15 = false
): CompiledWorkflowDefinition {
  assertWomlDocumentRunnable(document);
  const {
    element: workflow,
    modules,
    workflowId,
    metadata,
    triggers,
    flow,
    lifecycle,
    runtimePolicy,
  } = validateDocument(document);
  const scriptAnalyses = collectScriptAnalyses(flow.items);
  const lifecycleScripts =
    lifecycle?.hooks.flatMap(hook =>
      hook.actions.flatMap(action =>
        action.kind === 'script'
          ? [{ analysis: action.scriptAnalysis, span: action.scriptSpan }]
          : []
      )
    ) ?? [];
  const workflowNotifications = collectValidatedNotifications(flow.items);
  const lifecycleNotifications =
    lifecycle?.hooks.flatMap(hook =>
      hook.actions.flatMap(action =>
        action.kind === 'notify' ? action.deliveries : []
      )
    ) ?? [];
  const telegramNotifications = [
    ...workflowNotifications,
    ...lifecycleNotifications,
  ].filter(delivery => delivery.provider === 'telegram');
  const localTelegramModule = modules.some(module => module.name === 'telegram');
  const usesTelegramMessaging =
    !localTelegramModule &&
    [
      ...scriptAnalyses.values(),
      ...lifecycleScripts.map(item => item.analysis),
    ].some(analysis => analysis.requiredServices.includes('telegram'));
  const usesModelV15 =
    forceModelV15 ||
    triggers.some(trigger => trigger.kind === 'telegram') ||
    telegramNotifications.length > 0 ||
    usesTelegramMessaging;
  const usesModelV14 =
    usesModelV15 || forceModelV14 || flow.firstSwitch !== undefined;
  const usesStructuredGraph =
    usesModelV14 ||
    flow.firstFork !== undefined ||
    flow.firstControlChoice !== undefined;
  if (modules.length > 0 && moduleRuntime === undefined) {
    failCompile(
      document,
      'WOML_MODULE_EXECUTION_UNAVAILABLE',
      'The MS2 frontend can compile imported modules, but runtime module loading begins in MS3.',
      modules[0].element.openTagSpan,
      'Use `woml check <file>` to inspect the immutable module graph without running it.'
    );
  }
  if (modules.length === 0 && moduleRuntime !== undefined) {
    failCompile(
      document,
      'WOML_MODULE_RUNTIME_UNEXPECTED',
      'A module runtime profile cannot be attached to a workflow with no <module> declarations.',
      workflow.openTagSpan
    );
  }
  if (triggers.length === 0 && flow.firstApproval !== undefined) {
    failCompile(
      document,
      'WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED',
      'A call-only workflow cannot contain Human Approval in Workflow Calls v1.',
      flow.firstApproval.openTagSpan,
      'Use terminal steps, branch, parallel, retry, modules, and managed services. Long approval-waiting child calls require the future engine suspension boundary.'
    );
  }
  if (moduleRuntime !== undefined) {
    const declaredNames = modules.map(module => module.name).sort();
    const runtimeNames = moduleRuntime.modules.map(module => module.name);
    if (
      moduleRuntime.profileVersion !== 1 ||
      moduleRuntime.modules.length === 0 ||
      JSON.stringify(runtimeNames) !== JSON.stringify(declaredNames)
    ) {
      failCompile(
        document,
        'WOML_MODULE_RUNTIME_MISMATCH',
        'Compiled module bindings must match every declared module alias exactly.',
        workflow.openTagSpan
      );
    }
    for (const binding of moduleRuntime.modules) {
      const declaration = modules.find(module => module.name === binding.name)!;
      if (
        !artifactDigestPattern.test(binding.bundleDigest) ||
        !artifactDigestPattern.test(binding.sourceMapDigest)
      ) {
        failCompile(
          document,
          'WOML_MODULE_ARTIFACT_DIGEST_INVALID',
          `Compiled module "${binding.name}" requires canonical bundle and source-map SHA-256 identities.`,
          declaration.element.openTagSpan
        );
      }
      if (
        binding.exports.length === 0 ||
        new Set(binding.exports).size !== binding.exports.length ||
        [...binding.exports].sort().join('\0') !== binding.exports.join('\0') ||
        binding.exports.some(name => !runtimeExportPattern.test(name))
      ) {
        failCompile(
          document,
          'WOML_MODULE_RUNTIME_EXPORTS_INVALID',
          `Compiled module "${binding.name}" requires sorted, unique JavaScript-safe function exports.`,
          declaration.element.openTagSpan
        );
      }
    }
  }
  if (
    usesStructuredGraph &&
    !flow.items.some(
      item =>
        item.kind === 'step' ||
        item.kind === 'branch' ||
        item.kind === 'approval'
    )
  ) {
    const source =
      flow.firstFork ?? flow.firstControlChoice ?? flow.firstSwitch!;
    failCompile(
      document,
      'WOML_WORKFLOW_RESULT_REQUIRED',
      `This Model v${usesModelV15 ? '15' : usesModelV14 ? '14' : '13'} workflow has no deterministic value-producing item on its main route.`,
      source.openTagSpan,
      'Add a main-route <step>, result-producing <choose id="..."> or <switch id="...">, or <approval> before the terminal control structure.'
    );
  }
  const loweredV13 = usesStructuredGraph
    ? lowerWorkflowV13(flow.items)
    : undefined;
  const lowered = loweredV13?.fragment ?? lowerFlowItems(flow.items);
  const availableServices = new Set([
    ...builtInServiceNames,
    ...modules.map(module => module.name),
  ]);
  const unknownService = [
    ...collectValidatedSteps(flow.items).flatMap(step =>
      step.scriptAnalysis.serviceReferences.map(reference => ({
        span: step.scriptSpan,
        reference,
      }))
    ),
    ...lifecycleScripts.flatMap(script =>
      script.analysis.serviceReferences.map(reference => ({
        span: script.span,
        reference,
      }))
    ),
  ].find(({ reference }) => !availableServices.has(reference.name));
  if (unknownService !== undefined) {
    const sourceFile = new SourceFile(document.file, document.source);
    const start =
      unknownService.span.start.offset + unknownService.reference.start;
    failCompile(
      document,
      'WOML_MODULE_SERVICE_UNKNOWN',
      `Unknown service alias "${unknownService.reference.name}".`,
      sourceFile.span(
        start,
        start +
          Math.max(
            1,
            unknownService.reference.end - unknownService.reference.start
          )
      ),
      'Use a built-in service or declare the local module in <imports>.'
    );
  }
  const usesScriptRuntimeV1 =
    usesStructuredGraph ||
    lifecycle !== undefined ||
    runtimePolicy !== undefined ||
    triggers.length === 0 ||
    moduleRuntime !== undefined ||
    [...scriptAnalyses.values()].some(
      analysis =>
        analysis.requiredSecrets.length > 0 ||
        analysis.usesServices ||
        analysis.usesNativeFetch
    );
  const nodes = usesScriptRuntimeV1
    ? withScriptRuntimeBindings(lowered.nodes, scriptAnalyses)
    : lowered.nodes;
  const baseGraph =
    loweredV13 === undefined
      ? ({
          entryNodeIds: [lowered.entryId],
          nodes,
          edges: lowered.edges,
        } satisfies CompiledWorkflowGraph)
      : ({
          ...loweredV13.graph,
          nodes: withScriptRuntimeBindings(
            loweredV13.graph.nodes,
            scriptAnalyses
          ),
        } satisfies CompiledWorkflowGraphV13);
  const communication = usesModelV15
    ? {
        profileVersion: 1 as const,
        providers: [
          {
            provider: 'telegram' as const,
            triggerIds: triggers
              .filter(trigger => trigger.kind === 'telegram')
              .map(trigger => trigger.id),
            notificationDeliveryIds: telegramNotifications.map(
              delivery => delivery.deliveryId
            ),
            messaging: usesTelegramMessaging || forceModelV15,
            credentialNames: [
              ...new Set([
                ...triggers.flatMap(trigger =>
                  trigger.kind === 'telegram' ? [trigger.botToken.name] : []
                ),
                ...telegramNotifications.map(delivery => delivery.botToken.name),
                ...[
                  ...scriptAnalyses.values(),
                  ...lifecycleScripts.map(item => item.analysis),
                ].flatMap(analysis =>
                  analysis.requiredServices.includes('telegram') ||
                  forceModelV15
                    ? analysis.requiredSecrets
                    : []
                ),
              ]),
            ].sort(),
          },
        ],
      }
    : undefined;
  if (
    communication !== undefined &&
    communication.providers[0].credentialNames.length === 0
  ) {
    failCompile(
      document,
      'WOML_TELEGRAM_CREDENTIAL_UNRESOLVED',
      'Telegram usage requires at least one explicit symbolic secret reference in the workflow.',
      workflow.openTagSpan,
      'Pass secrets.TELEGRAM_BOT_TOKEN from the WOML script into the imported module that calls services.telegram.send().'
    );
  }
  const definition = {
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers: triggers.map(lowerTrigger),
    graph: baseGraph,
    ...(moduleRuntime === undefined ? {} : { moduleRuntime }),
    ...(lifecycle === undefined
      ? {}
      : { lifecycle: lowerLifecycle(lifecycle) }),
    ...(runtimePolicy === undefined
      ? {}
      : { runtimePolicy: runtimePolicy.value }),
  };
  const compiled: CompiledWorkflowDefinition = usesStructuredGraph
    ? usesModelV15
      ? ({
          schemaVersion: 15,
          ...definition,
          communication: communication!,
          graph: baseGraph as CompiledWorkflowGraphV13,
          runtimePolicy: runtimePolicy?.value ?? {
            profileVersion: 1,
            concurrency: 1,
          },
        } satisfies CompiledWorkflowDefinitionV15)
      : usesModelV14
      ? ({
          schemaVersion: 14,
          ...definition,
          graph: baseGraph as CompiledWorkflowGraphV13,
          runtimePolicy: runtimePolicy?.value ?? {
            profileVersion: 1,
            concurrency: 1,
          },
        } satisfies CompiledWorkflowDefinitionV14)
      : ({
          schemaVersion: 13,
          ...definition,
          graph: baseGraph as CompiledWorkflowGraphV13,
          runtimePolicy: runtimePolicy?.value ?? {
            profileVersion: 1,
            concurrency: 1,
          },
        } satisfies CompiledWorkflowDefinitionV13)
    : runtimePolicy !== undefined
      ? {
          schemaVersion: 12,
          ...definition,
          runtimePolicy: runtimePolicy.value,
        }
      : lifecycle !== undefined
        ? { schemaVersion: 11, ...definition }
        : triggers.length === 0
          ? {
              schemaVersion: 10,
              ...definition,
            }
          : moduleRuntime !== undefined
            ? { schemaVersion: 9, ...definition, moduleRuntime }
            : usesScriptRuntimeV1
              ? { schemaVersion: 8, ...definition }
              : triggers.length > 1 ||
                  triggers.some(trigger => trigger.kind !== 'manual')
                ? { schemaVersion: 7, ...definition }
                : lowered.nodes.some(node => node.retryPolicy !== undefined)
                  ? { schemaVersion: 6, ...definition }
                  : flow.firstNotification !== undefined
                    ? { schemaVersion: 5, ...definition }
                    : flow.firstApproval !== undefined
                      ? { schemaVersion: 4, ...definition }
                      : flow.firstParallel !== undefined
                        ? { schemaVersion: 3, ...definition }
                        : flow.firstBranch === undefined
                          ? { schemaVersion: 1, ...definition }
                          : { schemaVersion: 2, ...definition };

  const graphIssues = inspectCompiledWorkflowGraph(compiled.graph, {
    requireSingleTerminal: true,
  });
  if (graphIssues.length > 0) {
    failCompile(
      document,
      'WOML_INVALID_DAG',
      graphIssues[0].message,
      workflow.openTagSpan
    );
  }

  return compiled;
}

export function compilePreparedReusableWorkflow(
  prepared: PreparedReusableWorkflow,
  moduleRuntime?: CompiledModuleRuntimeV1
): CompiledWorkflowDefinitionV14 | CompiledWorkflowDefinitionV15 {
  const compiled = compileValidatedWoml(
    prepared.document,
    moduleRuntime,
    true
  );
  if (compiled.schemaVersion !== 14 && compiled.schemaVersion !== 15) {
    throw new Error('reusable-step lowering did not produce Model v14 or v15');
  }
  const invocationsByNode = new Map(
    prepared.invocations.map(invocation => [invocation.nodeId, invocation])
  );
  const nodes = compiled.graph.nodes.map(node => {
    const invocation = invocationsByNode.get(node.id);
    if (invocation === undefined) return node;
    const requiredSecrets = invocation.props
      .flatMap(prop =>
        prop.expression.kind === 'secret' ? [prop.expression.name] : []
      )
      .sort();
    return {
      ...node,
      scriptRuntime: {
        bindingVersion: 3 as const,
        bindings: ['props', 'context', 'attempt', 'services'] as const,
        requiredSecrets,
      },
      metadata: {
        ...(node.metadata ?? {}),
        reusableDefinition: {
          alias: invocation.alias,
          invocationSource: prepared.rootSource,
          definitionSource: invocation.source,
          definitionDigest: invocation.definitionDigest,
        },
      },
    };
  });
  const visibility = new Map(
    (compiled.graph.contextVisibility ?? []).map(item => [item.nodeId, item.stepIds])
  );
  const providerById = new Map(
    prepared.providerInvocations.map(invocation => [invocation.providerId, invocation])
  );
  const deliveryById = new Map(
    prepared.providerDeliveries.map(delivery => [delivery.providerId, delivery])
  );
  const nodeById = new Map(compiled.graph.nodes.map(node => [node.id, node]));
  const visibilityMemo = new Map<string, ReadonlySet<string>>();
  const visibleBeforeNode = (nodeId: string): ReadonlySet<string> => {
    const explicit = visibility.get(nodeId);
    if (explicit !== undefined) return new Set(explicit);
    const cached = visibilityMemo.get(nodeId);
    if (cached !== undefined) return cached;
    // Break malformed cycles defensively; ordinary graph validation already
    // rejects them before reusable lowering.
    visibilityMemo.set(nodeId, new Set());
    const predecessors = compiled.graph.edges
      .filter(edge => edge.to === nodeId)
      .map(edge => edge.from);
    if (predecessors.length === 0) return new Set();
    const predecessorSets = predecessors.map(predecessor => {
      const result = new Set(visibleBeforeNode(predecessor));
      const node = nodeById.get(predecessor);
      if (
        node?.handler === 'runtime.script' ||
        node?.handler === 'engine.branch-result' ||
        node?.handler === 'engine.choice-result' ||
        node?.handler === 'engine.approval-join'
      ) result.add(predecessor);
      return result;
    });
    const guaranteed = new Set(
      [...predecessorSets[0]].filter(id =>
        predecessorSets.every(set => set.has(id))
      )
    );
    visibilityMemo.set(nodeId, guaranteed);
    return guaranteed;
  };
  const patchDeliveryExpression = (expression: ValueExpression): ValueExpression => {
    if (expression.kind === 'array') {
      return { ...expression, items: expression.items.map(patchDeliveryExpression) };
    }
    if (expression.kind !== 'object') return expression;
    const deliveryId = expression.fields.deliveryId;
    if (
      deliveryId?.kind === 'literal' &&
      typeof deliveryId.value === 'string' &&
      deliveryById.has(deliveryId.value)
    ) {
      const delivery = deliveryById.get(deliveryId.value)!;
      const provider = providerById.get(delivery.providerId)!;
      return {
        kind: 'object',
        fields: {
          deliveryId,
          provider: { kind: 'literal', value: 'custom' },
          destination: { kind: 'literal', value: provider.alias },
          credentials: { kind: 'object', fields: {} },
          providerId: { kind: 'literal', value: provider.providerId },
          domain: { kind: 'literal', value: delivery.domain },
          ...(delivery.message === undefined
            ? expression.fields.message === undefined
              ? {}
              : { message: expression.fields.message }
            : { message: delivery.message }),
        },
      };
    }
    return {
      ...expression,
      fields: Object.fromEntries(
        Object.entries(expression.fields).map(([name, value]) => [
          name,
          patchDeliveryExpression(value),
        ])
      ),
    };
  };
  for (const invocation of prepared.invocations) {
    const available = new Set(visibility.get(invocation.nodeId) ?? []);
    for (const prop of invocation.props) {
      if (prop.expression.kind !== 'context' || !prop.expression.path.startsWith('steps.')) {
        continue;
      }
      const referencedId = prop.expression.path.split('.')[1];
      if (available.has(referencedId)) continue;
      const attribute = prepared.invocationAttributes.get(invocation.invocationId)?.[prop.name];
      failCompile(
        prepared.document,
        'WOML_REUSABLE_PROP_CONTEXT_UNAVAILABLE',
        `Prop "${prop.name}" on <${invocation.alias}> cannot read step "${referencedId}" from this graph position.`,
        attribute?.valueSpan ?? prepared.document.root.openTagSpan,
        'Reference context.payload or a step that is guaranteed to be visible before this invocation.'
      );
    }
  }
  for (const provider of prepared.providerInvocations) {
    const delivery = deliveryById.get(provider.providerId)!;
    const available = new Set(
      delivery.ownerNodeId === undefined
        ? compiled.graph.nodes
            .filter(node => node.handler === 'runtime.script')
            .map(node => node.id)
        : visibleBeforeNode(delivery.ownerNodeId)
    );
    const referencedPaths = [
      ...provider.props.flatMap(prop =>
        prop.expression.kind === 'context' ? [prop.expression.path] : []
      ),
      ...(delivery.message?.kind === 'template'
        ? delivery.message.parts.flatMap(part =>
            part.kind === 'contextReference'
              ? [part.path.join('.')]
              : []
          )
        : []),
    ];
    for (const path of referencedPaths) {
      if (!path.startsWith('steps.')) continue;
      const referencedId = path.split('.')[1];
      if (available.has(referencedId)) continue;
      failCompile(
        prepared.document,
        'WOML_REUSABLE_PROP_CONTEXT_UNAVAILABLE',
        `Custom provider <${provider.alias}> cannot read step "${referencedId}" from this notification position.`,
        delivery.messageAttribute?.valueSpan ?? prepared.document.root.openTagSpan,
        'Pass context.payload or a step output available before this notification.'
      );
    }
  }
  const patchedNodes = nodes.map(node => ({
    ...node,
    inputs: patchDeliveryExpression(node.inputs),
  }));
  const patchedLifecycle = compiled.lifecycle === undefined
    ? undefined
    : {
        ...compiled.lifecycle,
        hooks: compiled.lifecycle.hooks.map(hook => ({
          ...hook,
          actions: hook.actions.map(action => ({
            ...action,
            inputs: patchDeliveryExpression(action.inputs),
          })),
        })),
      };
  return {
    ...compiled,
    graph: { ...compiled.graph, nodes: patchedNodes },
    ...(patchedLifecycle === undefined ? {} : { lifecycle: patchedLifecycle }),
    reusableDefinitions: [
      ...prepared.invocations,
      ...prepared.providerInvocations,
    ],
  };
}

export function compileWoml(
  document: WomlSourceDocument
): CompiledWorkflowDefinition {
  return compileValidatedWoml(document);
}

export function compileWomlWithModules(
  document: WomlSourceDocument,
  moduleRuntime: CompiledModuleRuntimeV1,
  options: { readonly forceModelV15?: boolean } = {}
):
  | CompiledWorkflowDefinitionV9
  | CompiledWorkflowDefinitionV10
  | CompiledWorkflowDefinitionV11
  | CompiledWorkflowDefinitionV12
  | CompiledWorkflowDefinitionV13
  | CompiledWorkflowDefinitionV14
  | CompiledWorkflowDefinitionV15 {
  const compiled = compileValidatedWoml(
    document,
    moduleRuntime,
    false,
    options.forceModelV15 === true
  );
  if (
    compiled.schemaVersion !== 9 &&
    compiled.schemaVersion !== 10 &&
    compiled.schemaVersion !== 11 &&
    compiled.schemaVersion !== 12 &&
    compiled.schemaVersion !== 13 &&
    compiled.schemaVersion !== 14 &&
    compiled.schemaVersion !== 15
  ) {
    throw new Error(
      'module compilation did not produce Model v9, v10, v11, v12, v13, v14, or v15'
    );
  }
  return compiled;
}
