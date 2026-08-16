import type {
  LifecyclePresentationV1,
  PresentationFailureV1,
  RunPresentationListV1,
  RunPresentationV1,
  RunSummaryV1,
  StepPresentationV1,
  TriggerPresentationV1,
  WorkflowPresentationV1,
} from './types';

const MAX_PRESENTATION_BYTES = 2 * 1024 * 1024;
const RUN_STATUSES = new Set([
  'queued', 'running', 'waiting', 'retrying', 'cancelling', 'finalizing',
  'succeeded', 'failed', 'cancelled', 'timed_out',
]);
const STEP_STATUSES = new Set([
  'queued', 'running', 'waiting', 'retrying', 'succeeded', 'failed',
  'cancelled', 'timed_out', 'skipped',
]);
const STEP_KINDS = new Set([
  'step', 'script', 'custom_step', 'switch', 'choose', 'parallel', 'fork',
  'branch', 'approval', 'workflow_call', 'workflow_start',
]);
const TRIGGER_TYPES = new Set([
  'manual', 'webhook', 'slack', 'telegram', 'schedule', 'interval', 'event',
]);
const LIFECYCLE_HOOKS = new Set([
  'on-start', 'on-success', 'on-error', 'on-cancel', 'on-complete',
  'on-step-start', 'on-step-success', 'on-step-failure', 'on-step-complete',
]);

export class RunPresentationDecodeError extends Error {
  readonly code: 'WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED' | 'WOML_RUN_PRESENTATION_INVALID';

  constructor(
    code: 'WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED' | 'WOML_RUN_PRESENTATION_INVALID',
    message: string
  ) {
    super(message);
    this.name = 'RunPresentationDecodeError';
    this.code = code;
  }
}

function invalid(path: string): never {
  throw new RunPresentationDecodeError(
    'WOML_RUN_PRESENTATION_INVALID',
    `The native core returned an invalid Run Presentation v1 value at ${path}.`
  );
}

function record(value: unknown, path: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) invalid(path);
  return value as Record<string, unknown>;
}

function exact(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[],
  path: string
): void {
  const allowed = new Set([...required, ...optional]);
  if (!required.every(key => Object.hasOwn(value, key))) invalid(path);
  if (!Object.keys(value).every(key => allowed.has(key))) invalid(path);
}

function text(value: unknown, path: string, maximum = 2048): string {
  if (typeof value !== 'string' || value.length === 0 || [...value].length > maximum) invalid(path);
  return value;
}

function optionalText(value: unknown, path: string, maximum = 2048): string | undefined {
  return value === undefined ? undefined : text(value, path, maximum);
}

function dateTime(value: unknown, path: string): string {
  const result = text(value, path, 64);
  if (!Number.isFinite(Date.parse(result))) invalid(path);
  return result;
}

function optionalDateTime(value: unknown, path: string): string | undefined {
  return value === undefined ? undefined : dateTime(value, path);
}

function integer(value: unknown, path: string, minimum: number, maximum: number): number {
  if (!Number.isSafeInteger(value) || Number(value) < minimum || Number(value) > maximum) invalid(path);
  return Number(value);
}

function optionalInteger(value: unknown, path: string, maximum: number): number | undefined {
  return value === undefined ? undefined : integer(value, path, 0, maximum);
}

function enumeration(value: unknown, values: ReadonlySet<string>, path: string): string {
  if (typeof value !== 'string' || !values.has(value)) invalid(path);
  return value;
}

function jsonValue(value: unknown, path: string, depth = 0, budget = { nodes: 0 }): void {
  budget.nodes += 1;
  if (budget.nodes > 2000 || depth > 5) invalid(path);
  if (value === null || typeof value === 'boolean' || typeof value === 'string') return;
  if (typeof value === 'number' && Number.isFinite(value)) return;
  if (Array.isArray(value)) {
    if (value.length > 21) invalid(path);
    value.forEach((item, index) => jsonValue(item, `${path}[${index}]`, depth + 1, budget));
    return;
  }
  const object = record(value, path);
  if (Object.keys(object).length > 21) invalid(path);
  for (const [key, item] of Object.entries(object)) jsonValue(item, `${path}.${key}`, depth + 1, budget);
}

function failure(value: unknown, path: string): PresentationFailureV1 {
  const item = record(value, path);
  exact(item, ['code', 'message'], ['kind', 'retryable'], path);
  const code = text(item.code, `${path}.code`, 124);
  if (!/^WOML_[A-Z0-9_]{1,123}$/.test(code)) invalid(`${path}.code`);
  if (item.retryable !== undefined && typeof item.retryable !== 'boolean') invalid(`${path}.retryable`);
  return {
    code,
    message: text(item.message, `${path}.message`, 8192),
    ...(item.kind === undefined ? {} : { kind: text(item.kind, `${path}.kind`) }),
    ...(item.retryable === undefined ? {} : { retryable: item.retryable as boolean }),
  };
}

function trigger(value: unknown, path: string): TriggerPresentationV1 {
  const item = record(value, path);
  const optional = [
    'label', 'method', 'url', 'example', 'schedule', 'timezone', 'nextDueAt',
    'interval', 'event', 'workspace', 'scope', 'warning',
  ];
  exact(item, ['id', 'type'], optional, path);
  const type = enumeration(item.type, TRIGGER_TYPES, `${path}.type`) as TriggerPresentationV1['type'];
  if (item.method !== undefined && !['GET', 'POST', 'PUT', 'PATCH', 'DELETE'].includes(String(item.method))) invalid(`${path}.method`);
  return {
    id: text(item.id, `${path}.id`, 256), type,
    ...Object.fromEntries(optional
      .filter(key => item[key] !== undefined)
      .map(key => [key, key === 'nextDueAt'
        ? dateTime(item[key], `${path}.${key}`)
        : text(item[key], `${path}.${key}`, key === 'example' ? 16384 : 4096)])),
  } as unknown as TriggerPresentationV1;
}

function workflow(value: unknown, path: string): WorkflowPresentationV1 {
  const item = record(value, path);
  exact(item, ['id', 'definitionHash', 'triggers'], ['name', 'description', 'version'], path);
  const hash = text(item.definitionHash, `${path}.definitionHash`, 71);
  if (!/^sha256:[a-f0-9]{64}$/.test(hash)) invalid(`${path}.definitionHash`);
  if (!Array.isArray(item.triggers) || item.triggers.length < 1 || item.triggers.length > 1000) invalid(`${path}.triggers`);
  return {
    id: text(item.id, `${path}.id`, 256),
    ...(item.name === undefined ? {} : { name: optionalText(item.name, `${path}.name`) }),
    ...(item.description === undefined ? {} : { description: optionalText(item.description, `${path}.description`, 8192) }),
    ...(item.version === undefined ? {} : { version: optionalText(item.version, `${path}.version`, 128) }),
    definitionHash: hash,
    triggers: item.triggers.map((entry, index) => trigger(entry, `${path}.triggers[${index}]`)),
  };
}

function step(value: unknown, path: string): StepPresentationV1 {
  const item = record(value, path);
  exact(item, ['id', 'kind', 'status', 'depth', 'attempts'], [
    'name', 'description', 'startedAt', 'completedAt', 'durationMs', 'detail',
    'result', 'resultTruncated', 'failure',
  ], path);
  if (item.result !== undefined) jsonValue(item.result, `${path}.result`);
  if (item.resultTruncated !== undefined && typeof item.resultTruncated !== 'boolean') invalid(`${path}.resultTruncated`);
  return {
    id: text(item.id, `${path}.id`, 256),
    ...(item.name === undefined ? {} : { name: optionalText(item.name, `${path}.name`) }),
    ...(item.description === undefined ? {} : { description: optionalText(item.description, `${path}.description`, 8192) }),
    kind: enumeration(item.kind, STEP_KINDS, `${path}.kind`) as StepPresentationV1['kind'],
    status: enumeration(item.status, STEP_STATUSES, `${path}.status`) as StepPresentationV1['status'],
    depth: integer(item.depth, `${path}.depth`, 0, 64),
    ...(item.startedAt === undefined ? {} : { startedAt: optionalDateTime(item.startedAt, `${path}.startedAt`) }),
    ...(item.completedAt === undefined ? {} : { completedAt: optionalDateTime(item.completedAt, `${path}.completedAt`) }),
    ...(item.durationMs === undefined ? {} : { durationMs: optionalInteger(item.durationMs, `${path}.durationMs`, 31536000000) }),
    attempts: integer(item.attempts, `${path}.attempts`, 1, 1000),
    ...(item.detail === undefined ? {} : { detail: optionalText(item.detail, `${path}.detail`, 8192) }),
    ...(item.result === undefined ? {} : { result: item.result as StepPresentationV1['result'] }),
    ...(item.resultTruncated === undefined ? {} : { resultTruncated: item.resultTruncated as boolean }),
    ...(item.failure === undefined ? {} : { failure: failure(item.failure, `${path}.failure`) }),
  };
}

function lifecycle(value: unknown, path: string): LifecyclePresentationV1 {
  const item = record(value, path);
  exact(item, ['hook', 'status'], ['durationMs', 'provider', 'detail', 'failure'], path);
  return {
    hook: enumeration(item.hook, LIFECYCLE_HOOKS, `${path}.hook`) as LifecyclePresentationV1['hook'],
    status: enumeration(item.status, STEP_STATUSES, `${path}.status`) as LifecyclePresentationV1['status'],
    ...(item.durationMs === undefined ? {} : { durationMs: optionalInteger(item.durationMs, `${path}.durationMs`, 31536000000) }),
    ...(item.provider === undefined ? {} : { provider: optionalText(item.provider, `${path}.provider`) }),
    ...(item.detail === undefined ? {} : { detail: optionalText(item.detail, `${path}.detail`, 8192) }),
    ...(item.failure === undefined ? {} : { failure: failure(item.failure, `${path}.failure`) }),
  };
}

function summary(value: unknown, path: string, stepCount: number): RunSummaryV1 {
  const item = record(value, path);
  exact(item, ['succeeded', 'failed', 'skipped', 'cancelled', 'total'], [], path);
  const result = {
    succeeded: integer(item.succeeded, `${path}.succeeded`, 0, 10000),
    failed: integer(item.failed, `${path}.failed`, 0, 10000),
    skipped: integer(item.skipped, `${path}.skipped`, 0, 10000),
    cancelled: integer(item.cancelled, `${path}.cancelled`, 0, 10000),
    total: integer(item.total, `${path}.total`, 0, 10000),
  };
  if (result.total !== stepCount || result.succeeded + result.failed + result.skipped + result.cancelled > result.total) invalid(path);
  return result;
}

function decodeValue(value: unknown): RunPresentationV1 {
  const item = record(value, '$');
  if (item.profile !== 'woml.run-presentation/v1') {
    throw new RunPresentationDecodeError(
      'WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED',
      `Unsupported Run Presentation profile ${JSON.stringify(item.profile)}.`
    );
  }
  exact(item, [
    'profile', 'workflow', 'runId', 'trigger', 'status', 'admittedAt', 'steps',
    'summary', 'lifecycle', 'warnings',
  ], ['startedAt', 'completedAt', 'durationMs', 'result', 'resultTruncated', 'failure'], '$');
  if (!Array.isArray(item.steps) || item.steps.length > 10000) invalid('$.steps');
  if (!Array.isArray(item.lifecycle) || item.lifecycle.length > 1000) invalid('$.lifecycle');
  if (!Array.isArray(item.warnings) || item.warnings.length > 1000) invalid('$.warnings');
  const triggerItem = record(item.trigger, '$.trigger');
  exact(triggerItem, ['id', 'type'], [], '$.trigger');
  if (item.result !== undefined) jsonValue(item.result, '$.result');
  if (item.resultTruncated !== undefined && typeof item.resultTruncated !== 'boolean') invalid('$.resultTruncated');
  const steps = item.steps.map((entry, index) => step(entry, `$.steps[${index}]`));
  return {
    profile: 'woml.run-presentation/v1',
    workflow: workflow(item.workflow, '$.workflow'),
    runId: text(item.runId, '$.runId', 256),
    trigger: {
      id: text(triggerItem.id, '$.trigger.id', 256),
      type: enumeration(triggerItem.type, TRIGGER_TYPES, '$.trigger.type') as RunPresentationV1['trigger']['type'],
    },
    status: enumeration(item.status, RUN_STATUSES, '$.status') as RunPresentationV1['status'],
    admittedAt: dateTime(item.admittedAt, '$.admittedAt'),
    ...(item.startedAt === undefined ? {} : { startedAt: optionalDateTime(item.startedAt, '$.startedAt') }),
    ...(item.completedAt === undefined ? {} : { completedAt: optionalDateTime(item.completedAt, '$.completedAt') }),
    ...(item.durationMs === undefined ? {} : { durationMs: optionalInteger(item.durationMs, '$.durationMs', 31536000000) }),
    steps,
    summary: summary(item.summary, '$.summary', steps.length),
    lifecycle: item.lifecycle.map((entry, index) => lifecycle(entry, `$.lifecycle[${index}]`)),
    ...(item.result === undefined ? {} : { result: item.result as RunPresentationV1['result'] }),
    ...(item.resultTruncated === undefined ? {} : { resultTruncated: item.resultTruncated as boolean }),
    ...(item.failure === undefined ? {} : { failure: failure(item.failure, '$.failure') }),
    warnings: item.warnings.map((entry, index) => failure(entry, `$.warnings[${index}]`)),
  };
}

export function decodeRunPresentationV1(json: string): RunPresentationV1 {
  if (new TextEncoder().encode(json).byteLength > MAX_PRESENTATION_BYTES) invalid('$');
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch {
    invalid('$');
  }
  return decodeValue(value);
}

export function decodeRunPresentationListV1(json: string): RunPresentationListV1 {
  if (new TextEncoder().encode(json).byteLength > MAX_PRESENTATION_BYTES) invalid('$');
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch {
    invalid('$');
  }
  const item = record(value, '$');
  if (item.profile !== 'woml.run-presentation-list/v1') {
    throw new RunPresentationDecodeError(
      'WOML_RUN_PRESENTATION_VERSION_UNSUPPORTED',
      `Unsupported Run Presentation list profile ${JSON.stringify(item.profile)}.`
    );
  }
  exact(item, ['profile', 'workflowId', 'runs'], [], '$');
  if (!Array.isArray(item.runs) || item.runs.length > 10) invalid('$.runs');
  const workflowId = text(item.workflowId, '$.workflowId', 256);
  const runs = item.runs.map((run, index) => {
    const decoded = decodeValue(run);
    if (decoded.workflow.id !== workflowId) invalid(`$.runs[${index}].workflow.id`);
    return decoded;
  });
  return { profile: 'woml.run-presentation-list/v1', workflowId, runs };
}
