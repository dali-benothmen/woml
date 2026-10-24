import { createHash } from 'node:crypto';

export const TELEMETRY_SIGNAL_PROFILE = 'woml.telemetry-signal/v1';
export const TELEMETRY_SIGNAL_MAX_BYTES = 16 * 1024;
export const TELEMETRY_ADMISSION_LINK_PROFILE =
  'woml.telemetry-admission-link/v1';

export type TelemetrySignalKind =
  | 'runtime'
  | 'run'
  | 'attempt'
  | 'retry'
  | 'wait'
  | 'approval'
  | 'lock'
  | 'capability'
  | 'workflow_call'
  | 'compensation'
  | 'cleanup'
  | 'recovery'
  | 'remote_dispatch';

export type TelemetrySignalStatus =
  | 'admitted'
  | 'started'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'timed_out'
  | 'waiting'
  | 'resolved'
  | 'released'
  | 'degraded'
  | 'stopped';

export interface TelemetrySignalV1 {
  readonly profile: typeof TELEMETRY_SIGNAL_PROFILE;
  readonly signalId: string;
  readonly kind: TelemetrySignalKind;
  readonly occurredAt: string;
  readonly startedAt?: string;
  readonly endedAt?: string;
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly workflowId?: string;
  readonly runId?: string;
  readonly nodeId?: string;
  readonly attempt?: number;
  readonly invocationId?: string;
  readonly status: TelemetrySignalStatus;
  readonly code?: string;
  readonly durationMs?: number;
  readonly attributes: Readonly<Record<string, string | boolean>>;
}

export interface TelemetrySpanV1 {
  readonly name: string;
  readonly traceId: string;
  readonly spanId: string;
  readonly parentSpanId?: string;
  readonly startTime: string;
  readonly endTime: string;
  readonly status: 'unset' | 'ok' | 'error';
  readonly attributes: Readonly<Record<string, string | number | boolean>>;
}

export interface TelemetryAdmissionLinkV1 {
  readonly profile: typeof TELEMETRY_ADMISSION_LINK_PROFILE;
  readonly occurredAt: string;
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly runId: string;
  readonly status: 'linked' | 'invalid';
  readonly traceId?: string;
  readonly spanId?: string;
  readonly sampled?: boolean;
}

const kinds = new Set<TelemetrySignalKind>([
  'runtime',
  'run',
  'attempt',
  'retry',
  'wait',
  'approval',
  'lock',
  'capability',
  'workflow_call',
  'compensation',
  'cleanup',
  'recovery',
  'remote_dispatch',
]);
const statuses = new Set<TelemetrySignalStatus>([
  'admitted',
  'started',
  'succeeded',
  'failed',
  'cancelled',
  'timed_out',
  'waiting',
  'resolved',
  'released',
  'degraded',
  'stopped',
]);
const optionalKeys = new Set([
  'startedAt',
  'endedAt',
  'workflowId',
  'runId',
  'nodeId',
  'attempt',
  'invocationId',
  'code',
  'durationMs',
]);
const requiredKeys = new Set([
  'profile',
  'signalId',
  'kind',
  'occurredAt',
  'runtimeInstanceId',
  'deploymentId',
  'status',
  'attributes',
]);
const stringAttributeKeys = new Set([
  'trigger.type',
  'handler.category',
  'node.kind',
  'capability.category',
  'corrective.phase',
  'wait.kind',
  'lock.outcome',
  'remote.phase',
  'remote.outcome',
]);
const booleanAttributeKeys = new Set(['retry.scheduled', 'recovered']);

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function safeId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length >= 1 &&
    Buffer.byteLength(value) <= 320 &&
    !/[\u0000-\u001f\u007f]/u.test(value)
  );
}

function dateTime(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    Number.isFinite(Date.parse(value)) &&
    /^\d{4}-\d{2}-\d{2}T/u.test(value)
  );
}

function safeCategory(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length >= 1 &&
    value.length <= 64 &&
    /^[a-z0-9._/-]+$/u.test(value)
  );
}

function validAttributes(value: unknown): value is Record<string, string | boolean> {
  if (!record(value) || Object.keys(value).length > 16) return false;
  return Object.entries(value).every(([key, attribute]) => {
    if (stringAttributeKeys.has(key)) return safeCategory(attribute);
    if (booleanAttributeKeys.has(key)) return typeof attribute === 'boolean';
    return false;
  });
}

export function parseTelemetrySignal(json: string): TelemetrySignalV1 {
  if (Buffer.byteLength(json) > TELEMETRY_SIGNAL_MAX_BYTES) {
    throw new Error('The native core returned oversized telemetry.');
  }
  const value: unknown = JSON.parse(json);
  if (!record(value)) throw new Error('The native core returned invalid telemetry.');
  const keys = Object.keys(value);
  if (
    keys.some(key => !requiredKeys.has(key) && !optionalKeys.has(key)) ||
    [...requiredKeys].some(key => !(key in value)) ||
    value.profile !== TELEMETRY_SIGNAL_PROFILE ||
    !safeId(value.signalId) ||
    !kinds.has(value.kind as TelemetrySignalKind) ||
    !dateTime(value.occurredAt) ||
    !safeId(value.runtimeInstanceId) ||
    !safeId(value.deploymentId) ||
    !statuses.has(value.status as TelemetrySignalStatus) ||
    !validAttributes(value.attributes)
  ) {
    throw new Error('The native core returned invalid telemetry.');
  }
  for (const key of ['workflowId', 'runId', 'nodeId', 'invocationId'] as const) {
    if (value[key] !== undefined && !safeId(value[key])) {
      throw new Error('The native core returned invalid telemetry identity.');
    }
  }
  for (const key of ['startedAt', 'endedAt'] as const) {
    if (value[key] !== undefined && !dateTime(value[key])) {
      throw new Error('The native core returned invalid telemetry time.');
    }
  }
  if (
    value.attempt !== undefined &&
    (!Number.isSafeInteger(value.attempt) ||
      Number(value.attempt) < 1 ||
      Number(value.attempt) > 1_000_000)
  ) {
    throw new Error('The native core returned invalid telemetry attempt.');
  }
  if (
    value.durationMs !== undefined &&
    (typeof value.durationMs !== 'number' ||
      !Number.isFinite(value.durationMs) ||
      value.durationMs < 0 ||
      value.durationMs > 315_576_000_000)
  ) {
    throw new Error('The native core returned invalid telemetry duration.');
  }
  if (
    value.code !== undefined &&
    (typeof value.code !== 'string' ||
      value.code.length > 128 ||
      !/^WOML_[A-Z0-9_]+$/u.test(value.code))
  ) {
    throw new Error('The native core returned invalid telemetry code.');
  }
  if (
    value.startedAt !== undefined &&
    value.endedAt !== undefined &&
    Date.parse(value.endedAt as string) < Date.parse(value.startedAt as string)
  ) {
    throw new Error('The native core returned a reversed telemetry interval.');
  }
  return value as unknown as TelemetrySignalV1;
}

export function parseTelemetryAdmissionLink(
  json: string
): TelemetryAdmissionLinkV1 {
  if (Buffer.byteLength(json) > 4 * 1024) {
    throw new Error('The native core returned oversized telemetry context.');
  }
  const value: unknown = JSON.parse(json);
  if (!record(value)) throw new Error('The native core returned invalid telemetry context.');
  const allowed = new Set([
    'profile',
    'occurredAt',
    'runtimeInstanceId',
    'deploymentId',
    'runId',
    'status',
    'traceId',
    'spanId',
    'sampled',
  ]);
  const linked = value.status === 'linked';
  if (
    Object.keys(value).some(key => !allowed.has(key)) ||
    value.profile !== TELEMETRY_ADMISSION_LINK_PROFILE ||
    !dateTime(value.occurredAt) ||
    !safeId(value.runtimeInstanceId) ||
    !safeId(value.deploymentId) ||
    !safeId(value.runId) ||
    !['linked', 'invalid'].includes(String(value.status)) ||
    (linked &&
      (typeof value.traceId !== 'string' ||
        !/^[0-9a-f]{32}$/u.test(value.traceId) ||
        /^0+$/u.test(value.traceId) ||
        typeof value.spanId !== 'string' ||
        !/^[0-9a-f]{16}$/u.test(value.spanId) ||
        /^0+$/u.test(value.spanId) ||
        typeof value.sampled !== 'boolean')) ||
    (!linked &&
      (value.traceId !== undefined ||
        value.spanId !== undefined ||
        value.sampled !== undefined))
  ) {
    throw new Error('The native core returned invalid telemetry context.');
  }
  return value as unknown as TelemetryAdmissionLinkV1;
}

function digest(parts: readonly string[], bytes: number): string {
  const hash = createHash('sha256');
  parts.forEach((part, index) => {
    if (index > 0) hash.update(Buffer.from([0]));
    hash.update(part);
  });
  const value = hash.digest('hex').slice(0, bytes * 2);
  return /^0+$/u.test(value) ? `${value.slice(0, -1)}1` : value;
}

export function womlTraceIdV1(runId: string): string {
  return digest(['woml:trace:v1', runId], 16);
}

export function womlSpanIdV1(
  runId: string,
  kind: TelemetrySignalKind,
  stableOperationIdentity: string
): string {
  return digest(['woml:span:v1', runId, kind, stableOperationIdentity], 8);
}

function stableOperationIdentity(signal: TelemetrySignalV1): string {
  if (signal.kind === 'run') return signal.runId!;
  if (signal.kind === 'attempt') {
    return `${signal.nodeId}:${signal.attempt}:${signal.invocationId}`;
  }
  return [
    signal.nodeId,
    signal.attempt,
    signal.invocationId,
    signal.signalId,
  ]
    .filter(value => value !== undefined)
    .join(':');
}

function spanName(kind: TelemetrySignalKind): string {
  return {
    runtime: 'woml.runtime',
    run: 'woml.run',
    attempt: 'woml.step',
    retry: 'woml.retry.wait',
    wait: 'woml.wait',
    approval: 'woml.approval',
    lock: 'woml.lock',
    capability: 'woml.capability',
    workflow_call: 'woml.workflow.call',
    compensation: 'woml.compensation',
    cleanup: 'woml.cleanup',
    recovery: 'woml.recovery',
    remote_dispatch: 'woml.remote.dispatch',
  }[kind];
}

export function mapTelemetrySignalToSpan(
  signal: TelemetrySignalV1
): TelemetrySpanV1 | undefined {
  if (
    signal.runId === undefined ||
    signal.status === 'admitted' ||
    signal.status === 'started' ||
    (signal.status === 'waiting' && signal.endedAt === undefined)
  ) {
    return undefined;
  }
  const traceId = womlTraceIdV1(signal.runId);
  const identity = stableOperationIdentity(signal);
  const startTime = signal.startedAt ?? signal.occurredAt;
  const endTime = signal.endedAt ?? signal.occurredAt;
  const attributes: Record<string, string | number | boolean> = {
    'woml.signal.id': signal.signalId,
    'woml.signal.kind': signal.kind,
    'woml.durable.status': signal.status,
    'woml.deployment.id': signal.deploymentId,
    'woml.runtime.instance.id': signal.runtimeInstanceId,
    ...(signal.workflowId === undefined
      ? {}
      : { 'woml.workflow.id': signal.workflowId }),
    'woml.run.id': signal.runId,
    ...(signal.nodeId === undefined ? {} : { 'woml.node.id': signal.nodeId }),
    ...(signal.attempt === undefined ? {} : { 'woml.attempt': signal.attempt }),
    ...(signal.invocationId === undefined
      ? {}
      : { 'woml.invocation.id': signal.invocationId }),
    ...(signal.code === undefined ? {} : { 'error.type': signal.code }),
    ...signal.attributes,
  };
  return {
    name: spanName(signal.kind),
    traceId,
    spanId: womlSpanIdV1(signal.runId, signal.kind, identity),
    ...(signal.kind === 'run'
      ? {}
      : {
          parentSpanId: womlSpanIdV1(
            signal.runId,
            'run',
            signal.runId
          ),
        }),
    startTime,
    endTime,
    status:
      signal.status === 'failed' ||
      signal.status === 'timed_out' ||
      signal.status === 'degraded'
        ? 'error'
        : signal.status === 'succeeded' ||
            signal.status === 'resolved' ||
            signal.status === 'released'
          ? 'ok'
          : 'unset',
    attributes,
  };
}
