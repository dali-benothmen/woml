import Ajv2020 from 'ajv/dist/2020';

import {
  inspectCompiledWorkflowGraph,
  type CompiledTrigger,
  type CompiledWorkflowDefinition,
  type CompiledWorkflowEdge,
  type CompiledWorkflowGraph,
  type CompiledWorkflowMetadata,
  type CompiledWorkflowNode,
  type ContextReferenceExpression,
  type JsonValue,
  type RetryPolicy,
  type SecretReferenceExpression,
  type ValueExpression,
} from './model';
import {
  SourceFile,
  WomlCompileError,
  WomlValidationError,
  type SourceSpan,
  type WomlDiagnostic,
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

interface ElementProfile {
  readonly attributes: ReadonlySet<string>;
  readonly stagedAttributes?: ReadonlySet<string>;
}

interface ValidatedStep {
  readonly kind: 'step';
  readonly id: string;
  readonly source: string;
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

interface ValidatedBranch {
  readonly kind: 'branch';
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

interface ValidatedNotificationDelivery {
  readonly deliveryId: string;
  readonly provider: 'slack';
  readonly destination: string;
  readonly botToken: SecretReferenceExpression;
  readonly appToken: SecretReferenceExpression;
}

type ValidatedFlowItem =
  | ValidatedStep
  | ValidatedBranch
  | ValidatedParallel
  | ValidatedApproval;

interface ValidatedFlow {
  readonly items: readonly ValidatedFlowItem[];
  readonly firstBranch?: WomlSourceElement;
  readonly firstParallel?: WomlSourceElement;
  readonly firstApproval?: WomlSourceElement;
  readonly firstNotification?: WomlSourceElement;
}

interface ValidatedWorkflow {
  readonly workflowId: string;
  readonly metadata?: CompiledWorkflowMetadata;
  readonly triggers: readonly ValidatedTrigger[];
  readonly flow: ValidatedFlow;
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

interface ValidatedScheduleTrigger {
  readonly kind: 'schedule';
  readonly id: string;
  readonly cron: string;
  readonly timezone: string;
  readonly onMissed: 'skip' | 'run-once';
}

type ValidatedTrigger =
  | ValidatedManualTrigger
  | ValidatedWebhookTrigger
  | ValidatedSlackTrigger
  | ValidatedScheduleTrigger;

interface LoweredFlowFragment {
  readonly entryId: string;
  readonly exitId: string;
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
}

const supportedElements = new Set([
  'workflow',
  'triggers',
  'manual',
  'webhook',
  'schema',
  'steps',
  'step',
  'script',
  'branch',
  'parallel',
  'when',
  'otherwise',
  'result',
  'approval',
  'notify',
  'slack',
  'schedule',
  'when-approved',
  'when-rejected',
]);

const stagedElements = new Set([
  'config',
  'lifecycle',
  'on-success',
  'on-failure',
  'interval',
  'event',
]);

const elementProfiles: Readonly<Record<string, ElementProfile>> = {
  workflow: {
    attributes: new Set(['id', 'name', 'description', 'version']),
    stagedAttributes: new Set(['tags']),
  },
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
      'bot-token',
      'app-token',
    ]),
  },
  schedule: {
    attributes: new Set(['id', 'cron', 'timezone', 'on-missed']),
  },
  'when-approved': { attributes: new Set() },
  'when-rejected': { attributes: new Set() },
};

const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const javascriptSafeIdPattern = /^[a-z][A-Za-z0-9]*$/;
const webhookPathPattern = /^\/(?:[A-Za-z0-9._~-]+(?:\/[A-Za-z0-9._~-]+)*)?$/;

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
  span: SourceSpan
): never {
  throw new WomlCompileError(
    diagnostic(document, 'compile', code, message, span)
  );
}

function visitProfile(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  parent?: WomlSourceElement
): void {
  if (!supportedElements.has(element.name)) {
    if (parent?.name === 'notify') {
      failValidation(
        document,
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
        `<notify> supports <slack> only in this release; found <${element.name}>.`,
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
      failValidation(
        document,
        attribute.name === 'retry' || attribute.name.startsWith('retry-')
          ? 'WOML_RETRY_HANDLER_UNSUPPORTED'
          : element.name === 'slack'
            ? 'WOML_SLACK_UNKNOWN_ATTRIBUTE'
            : 'WOML_UNKNOWN_ATTRIBUTE',
        attribute.name === 'retry' || attribute.name.startsWith('retry-')
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
    const isWebhookCredential =
      element.name === 'webhook' && attribute.name === 'secret';
    if (isSlackCredential || isWebhookCredential) {
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
  role: 'trigger' | 'step' | 'branch' | 'parallel' | 'approval'
): string {
  if (
    attribute.value.length > 256 ||
    !javascriptSafeIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `${role === 'trigger' ? 'Trigger' : role === 'branch' ? 'Branch' : role === 'parallel' ? 'Parallel' : role === 'approval' ? 'Approval' : 'Step'} ID "${attribute.value}" must be a JavaScript-safe lower-camel identifier.`,
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
): readonly [WomlSourceElement, WomlSourceElement] {
  const children = elementChildren(document, workflow);
  const triggerContainers = children.filter(child => child.name === 'triggers');
  const stepsContainers = children.filter(child => child.name === 'steps');

  if (triggerContainers.length !== 1) {
    failValidation(
      document,
      'WOML_TRIGGER_CONTAINER_COUNT',
      `<workflow> requires exactly one <triggers> container; found ${triggerContainers.length}.`,
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
  if (
    children.length !== 2 ||
    children[0] !== triggerContainers[0] ||
    children[1] !== stepsContainers[0]
  ) {
    const offender = children.find(
      (child, index) =>
        (index === 0 && child.name !== 'triggers') ||
        (index === 1 && child.name !== 'steps') ||
        index > 1
    );
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<workflow> must contain <triggers> followed by <steps>, with no other executable-profile children.',
      offender?.openTagSpan ?? workflow.openTagSpan
    );
  }

  return [triggerContainers[0], stepsContainers[0]];
}

function schemaBody(
  document: WomlSourceDocument,
  schema: WomlSourceElement
): Readonly<Record<string, JsonValue>> {
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
      'WOML_WEBHOOK_SCHEMA_STRUCTURE_INVALID',
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
      'WOML_WEBHOOK_SCHEMA_JSON_INVALID',
      '<schema> must contain valid JSON.',
      source.span(start, Math.min(start + 1, span.end.offset))
    );
  }
  if (
    parsed === null ||
    typeof parsed !== 'object' ||
    Array.isArray(parsed)
  ) {
    failValidation(
      document,
      'WOML_WEBHOOK_SCHEMA_INVALID',
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
      'WOML_WEBHOOK_SCHEMA_INVALID',
      `Inline webhook JSON Schema is invalid${reason === undefined ? '.' : `: ${reason}.`}`,
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
  const schema = children[0] === undefined
    ? undefined
    : schemaBody(document, children[0]);
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
    if (child.name === 'schedule') {
      return validateScheduleTrigger(document, child);
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

function scriptSource(
  document: WomlSourceDocument,
  script: WomlSourceElement
): string {
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
  return rawBodies[0]?.value ?? '';
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
    /^\{\{(context\.(?:trigger(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*|steps\.([a-z][A-Za-z0-9]*)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*))\}\}$/.exec(
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
    path: match[1].split('.').slice(1),
    ...(structuralId === undefined ? {} : { structuralId }),
    span,
  };
}

function registerStructuralId(
  document: WomlSourceDocument,
  registry: Set<string>,
  attribute: WomlSourceAttribute,
  role: 'step' | 'branch' | 'parallel' | 'approval'
): string {
  const id = validateJavaScriptSafeId(document, attribute, role);
  if (registry.has(id)) {
    failValidation(
      document,
      'WOML_DUPLICATE_ID',
      `Structural ID "${id}" is duplicated across workflow steps, branches, parallel groups, and approvals.`,
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
  registry: Set<string>
): readonly ValidatedFlowItem[] {
  return elementChildren(document, arm).map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`)
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
  name: 'channels' | 'bot-token' | 'app-token'
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
      '<notify> must contain at least one <slack> provider.',
      notify.openTagSpan
    );
  }

  const seenDestinations = new Set<string>();
  const deliveries: ValidatedNotificationDelivery[] = [];
  providers.forEach((provider, providerIndex) => {
    if (provider.name !== 'slack') {
      failValidation(
        document,
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
        `<notify> supports <slack> only in this release; found <${provider.name}>.`,
        provider.openTagSpan
      );
    }
    ensureEmptyElement(document, provider);
    for (const triggerOnlyAttribute of ['id', 'events'] as const) {
      const attribute = provider.attributes[triggerOnlyAttribute];
      if (attribute !== undefined) {
        failValidation(
          document,
          'WOML_SLACK_UNKNOWN_ATTRIBUTE',
          `Attribute "${triggerOnlyAttribute}" is valid on a Slack trigger, not a Slack notification provider.`,
          attribute.nameSpan
        );
      }
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
  registry: Set<string>
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
    approvedItems: validateApprovalArm(document, approved[0], registry),
    rejectedItems: validateApprovalArm(document, rejected[0], registry),
  };
}

function validateStep(
  document: WomlSourceDocument,
  step: WomlSourceElement,
  registry: Set<string>
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

  return {
    kind: 'step',
    id,
    source: scriptSource(document, operations[0]),
    retryPolicy: stepRetryPolicy(document, step),
    metadata: flowItemMetadata(document, step),
  };
}

function validateBranchArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>
): ValidatedBranchArm {
  const testReference =
    arm.name === 'when'
      ? parseExactReference(document, requiredAttribute(document, arm, 'test'))
      : undefined;
  const children = elementChildren(document, arm);
  const results = children.filter(child => child.name === 'result');
  if (results.length !== 1) {
    failValidation(
      document,
      'WOML_BRANCH_RESULT_REQUIRED',
      `<${arm.name}> must contain exactly one <result>.`,
      results[1]?.openTagSpan ?? arm.openTagSpan
    );
  }

  const result = results[0];
  if (children.at(-1) !== result) {
    failValidation(
      document,
      'WOML_BRANCH_RESULT_ORDER',
      '<result> must be the final child of its branch arm.',
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
    validateFlowItem(document, child, registry, `<${arm.name}>`)
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
  registry: Set<string>
): ValidatedBranch {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, branch, 'id'),
    'branch'
  );
  const metadata = flowItemMetadata(document, branch);

  const children = elementChildren(document, branch);
  const whenCount = children.filter(child => child.name === 'when').length;
  if (whenCount === 0) {
    failValidation(
      document,
      'WOML_BRANCH_WHEN_REQUIRED',
      `<branch id="${id}"> requires at least one <when>.`,
      branch.openTagSpan
    );
  }

  const otherwiseIndexes = children.flatMap((child, index) =>
    child.name === 'otherwise' ? [index] : []
  );
  if (otherwiseIndexes.length === 0) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_REQUIRED',
      `<branch id="${id}"> requires exactly one final <otherwise>.`,
      branch.openTagSpan
    );
  }
  if (otherwiseIndexes.length > 1) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_ORDER',
      '<otherwise> may appear exactly once and must be the final branch case.',
      children[otherwiseIndexes[1]].openTagSpan
    );
  }
  const otherwiseIndex = otherwiseIndexes[0];
  if (otherwiseIndex !== children.length - 1) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_ORDER',
      '<otherwise> must be the final child of <branch>.',
      children[otherwiseIndex].openTagSpan
    );
  }

  for (let index = 0; index < otherwiseIndex; index += 1) {
    if (children[index].name !== 'when') {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        `<branch> cannot contain <${children[index].name}> outside a <when> or <otherwise> arm.`,
        children[index].openTagSpan
      );
    }
  }

  return {
    kind: 'branch',
    id,
    element: branch,
    metadata,
    arms: children.map(arm => validateBranchArm(document, arm, registry)),
  };
}

function validateParallel(
  document: WomlSourceDocument,
  parallel: WomlSourceElement,
  registry: Set<string>
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
    validateStep(document, child, registry)
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
  parent: string
): ValidatedFlowItem {
  if (element.name === 'step') return validateStep(document, element, registry);
  if (element.name === 'branch') {
    return validateBranch(document, element, registry);
  }
  if (element.name === 'parallel') {
    return validateParallel(document, element, registry);
  }
  if (element.name === 'approval') {
    return validateApproval(document, element, registry);
  }
  if (element.name === 'when-approved' || element.name === 'when-rejected') {
    failValidation(
      document,
      'WOML_APPROVAL_PLACEMENT_INVALID',
      `<${element.name}> is valid only as a direct child of <approval>.`,
      element.openTagSpan
    );
  }
  if (element.name === 'notify' || element.name === 'slack') {
    failValidation(
      document,
      'WOML_NOTIFY_INVALID_ORDER',
      element.name === 'notify'
        ? '<notify> is valid only as the first direct child of <approval>.'
        : '<slack> is valid only as a direct child of <notify>.',
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
  availableIds: ReadonlySet<string>
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
    failCompile(
      document,
      'WOML_REFERENCE_NOT_DOMINATING',
      `Output "${id}" is not guaranteed to be available at this reference.`,
      reference.span
    );
  }
}

function validateReferenceAvailability(
  document: WomlSourceDocument,
  items: readonly ValidatedFlowItem[],
  allIds: ReadonlySet<string>,
  availableBefore: ReadonlySet<string> = new Set()
): Set<string> {
  const available = new Set(availableBefore);
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
        armInput
      );
      validateReferenceAvailability(
        document,
        item.rejectedItems,
        allIds,
        armInput
      );
      available.add(item.id);
      continue;
    }

    for (const arm of item.arms) {
      if (arm.test !== undefined) {
        assertReferenceAvailable(document, arm.test, allIds, available);
      }
      const armAvailable = validateReferenceAvailability(
        document,
        arm.items,
        allIds,
        available
      );
      assertReferenceAvailable(document, arm.result, allIds, armAvailable);
    }
    available.add(item.id);
  }
  return available;
}

function validateSteps(
  document: WomlSourceDocument,
  stepsElement: WomlSourceElement
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
    validateFlowItem(document, child, structuralIds, '<steps>')
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

  const findFirstBranch = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'branch') {
        return item.element;
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

  return {
    items,
    ...(firstBranch === undefined ? {} : { firstBranch }),
    ...(firstParallel === undefined ? {} : { firstParallel }),
    ...(firstApproval === undefined ? {} : { firstApproval }),
    ...(firstNotification === undefined ? {} : { firstNotification }),
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
                credentials: {
                  kind: 'object' as const,
                  fields: {
                    botToken: notification.botToken,
                    appToken: notification.appToken,
                  },
                },
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
  return lowerApproval(item);
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

function validateDocument(document: WomlSourceDocument): ValidatedWorkflow {
  const workflow = document.root;
  if (workflow.name !== 'workflow') {
    failValidation(
      document,
      'WOML_EXPECTED_WORKFLOW_ROOT',
      `Expected <workflow> as the document root, found <${workflow.name}>.`,
      workflow.openTagSpan
    );
  }

  validateSecretReferenceSinks(document, workflow);
  visitProfile(document, workflow);

  const workflowId = validateWorkflowId(
    document,
    requiredAttribute(document, workflow, 'id')
  );
  const metadata = workflowMetadata(document, workflow);
  const [triggersElement, stepsElement] = validateWorkflowChildren(
    document,
    workflow
  );
  const triggers = validateTriggers(document, triggersElement);
  const flow = validateSteps(document, stepsElement);

  return {
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers,
    flow,
  };
}

export function validateWoml(document: WomlSourceDocument): void {
  validateDocument(document);
}

export function compileWoml(
  document: WomlSourceDocument
): CompiledWorkflowDefinition {
  const workflow = document.root;
  const { workflowId, metadata, triggers, flow } = validateDocument(document);
  const lowered = lowerFlowItems(flow.items);
  const definition = {
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers: triggers.map(lowerTrigger),
    graph: {
      entryNodeIds: [lowered.entryId],
      nodes: lowered.nodes,
      edges: lowered.edges,
    } satisfies CompiledWorkflowGraph,
  };
  const compiled: CompiledWorkflowDefinition =
    triggers.length > 1 || triggers.some(trigger => trigger.kind !== 'manual')
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
