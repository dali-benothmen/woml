import { createRequire } from 'node:module';
import { resolve } from 'node:path';

import type { CompiledWorkflowDefinition, JsonObject, JsonValue } from 'woml';

export interface RustRunEvent {
  readonly eventSchemaVersion: 1 | 2 | 3 | 4 | 5 | 6;
  readonly eventId: string;
  readonly runId: string;
  readonly sequence: number;
  readonly occurredAt: string;
  readonly type: string;
  readonly data: unknown;
}

export interface RustWorkflowExecutionResult {
  readonly workflowId: string;
  readonly runId: string;
  readonly terminalNodeId: string;
  readonly result: JsonValue;
  readonly context: {
    readonly trigger: JsonObject;
    readonly steps: Readonly<Record<string, JsonValue>>;
  };
  readonly executionOrder: readonly string[];
  readonly events: readonly RustRunEvent[];
}

export interface RustExecutorOptions {
  readonly nativeCorePath?: string;
  readonly scriptHostPath?: string;
  readonly bunExecutable?: string;
  readonly scriptTimeoutMs?: number;
  readonly trigger?: JsonObject;
}

export interface RustRecoveryReport {
  readonly inspectedRuns: number;
  readonly recoveredRuns: number;
  readonly interruptedAttempts: number;
  readonly resumableRuns: number;
}

interface NativeExecutionErrorEnvelope {
  readonly kind: 'woml_execution_error';
  readonly code: string;
  readonly message: string;
  readonly nodeId?: string;
  readonly branchId?: string;
  readonly armId?: string;
  readonly referencePath?: readonly string[];
  readonly branchSite?: 'test' | 'result' | 'selection';
  readonly approvalId?: string;
  readonly requestId?: string;
  readonly details?: NativeParallelExecutionErrorDetails;
}

export interface NativeParallelExecutionErrorDetails {
  readonly parallelId: string;
  readonly policy: 'fail-fast' | 'wait-all';
  readonly primaryNodeId: string;
  readonly failedNodeIds: readonly string[];
  readonly cancelledNodeIds: readonly string[];
}

export class RustWorkflowExecutionError extends Error {
  readonly code: string;
  readonly nodeId?: string;
  readonly branchId?: string;
  readonly armId?: string;
  readonly referencePath?: readonly string[];
  readonly branchSite?: 'test' | 'result' | 'selection';
  readonly parallelId?: string;
  readonly parallelPolicy?: 'fail-fast' | 'wait-all';
  readonly primaryNodeId?: string;
  readonly failedNodeIds?: readonly string[];
  readonly cancelledNodeIds?: readonly string[];
  readonly approvalId?: string;
  readonly requestId?: string;

  constructor(
    code: string,
    message: string,
    details: {
      readonly nodeId?: string;
      readonly branchId?: string;
      readonly armId?: string;
      readonly referencePath?: readonly string[];
      readonly branchSite?: 'test' | 'result' | 'selection';
      readonly approvalId?: string;
      readonly requestId?: string;
      readonly parallel?: NativeParallelExecutionErrorDetails;
    } = {}
  ) {
    super(message);
    this.name = 'RustWorkflowExecutionError';
    this.code = code;
    if (details.nodeId !== undefined) this.nodeId = details.nodeId;
    if (details.branchId !== undefined) this.branchId = details.branchId;
    if (details.armId !== undefined) this.armId = details.armId;
    if (details.referencePath !== undefined) {
      this.referencePath = details.referencePath;
    }
    if (details.branchSite !== undefined) this.branchSite = details.branchSite;
    if (details.approvalId !== undefined) this.approvalId = details.approvalId;
    if (details.requestId !== undefined) this.requestId = details.requestId;
    if (details.parallel !== undefined) {
      this.parallelId = details.parallel.parallelId;
      this.parallelPolicy = details.parallel.policy;
      this.primaryNodeId = details.parallel.primaryNodeId;
      this.failedNodeIds = details.parallel.failedNodeIds;
      this.cancelledNodeIds = details.parallel.cancelledNodeIds;
    }
  }
}

interface NativeCore {
  readonly executeWomlWorkflow: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number
  ) => Promise<string>;
  readonly executeWomlWorkflowDurable: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string
  ) => Promise<string>;
  readonly recoverWomlRuns: (eventStorePath: string) => string;
  readonly executeWomlWorkflowDurableOutcome: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string
  ) => Promise<string>;
  readonly resumeWomlWorkflowDurableOutcome: (
    compiledModelJson: string,
    definitionHash: string,
    runId: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string
  ) => Promise<string>;
  readonly resolveWomlApproval: (
    eventStorePath: string,
    token: string,
    decision: ApprovalDecision
  ) => string;
  readonly settleWomlApprovalTimeout: (
    eventStorePath: string,
    runId: string,
    approvalId: string
  ) => string;
  readonly runWomlNotificationProviderJourney: (
    eventStorePath: string,
    runId: string,
    bunExecutable: string,
    notificationHostPath: string,
    interactionTimeoutMs: number
  ) => Promise<string>;
}

export type ApprovalDecision = 'approved' | 'rejected';

export interface WaitingApproval {
  readonly approvalId: string;
  readonly requestId: string;
  readonly name?: string;
  readonly description?: string;
  readonly expiresAt?: string;
  readonly onTimeout: 'reject' | 'fail';
  readonly token: string;
  readonly credentialExpiresAt: string;
}

export type RustApprovalRuntimeOutcome =
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'succeeded';
      readonly execution: RustWorkflowExecutionResult;
    }
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'waiting';
      readonly workflowId: string;
      readonly runId: string;
      readonly approval: WaitingApproval;
    };

export interface ApprovalDecisionResult {
  readonly contract: 'woml.approval-http';
  readonly version: 1;
  readonly status: 'accepted' | 'already_resolved';
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly decision: ApprovalDecision;
  readonly source: 'human';
  readonly decidedAt: string;
}

export type ApprovalErrorCode =
  | 'WOML_APPROVAL_TOKEN_INVALID'
  | 'WOML_APPROVAL_TOKEN_EXPIRED'
  | 'WOML_APPROVAL_EXPIRED'
  | 'WOML_APPROVAL_DECISION_CONFLICT'
  | 'WOML_APPROVAL_INTERNAL';

export class ApprovalDecisionError extends Error {
  constructor(
    readonly code: ApprovalErrorCode,
    message: string
  ) {
    super(message);
    this.name = 'ApprovalDecisionError';
  }
}

export interface ApprovalTimeoutResult {
  readonly status: 'settled' | 'already_resolved' | 'not_due';
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly resolution: unknown | null;
  readonly settledAt: string | null;
}

export interface NotificationDispatchReport {
  readonly attempted: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly runFailed: boolean;
  readonly updatesAttempted: number;
  readonly updatesSucceeded: number;
  readonly updatesFailed: number;
}

export interface NotificationProviderJourneyResult {
  readonly runId: string;
  readonly decision: Omit<ApprovalDecisionResult, 'contract' | 'version'> | null;
  readonly resolution: 'approved' | 'rejected' | 'timeout_failed';
  readonly deliveries: NotificationDispatchReport;
  readonly updates: NotificationDispatchReport;
  readonly diagnostics: NotificationJourneyDiagnostics;
}

export interface NotificationDeliveryFailureDiagnostic {
  readonly deliveryId: string;
  readonly provider: string;
  readonly destination: string;
  readonly attempt: number;
  readonly final: boolean;
  readonly failure: {
    readonly kind: string;
    readonly code: string;
    readonly message: string;
    readonly retryable: boolean;
    readonly retryAfterMs?: number;
  };
}

export interface NotificationJourneyDiagnostics {
  readonly version: 1;
  readonly deliveryFailures: readonly NotificationDeliveryFailureDiagnostic[];
}

export class NotificationProviderError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly diagnostics?: NotificationJourneyDiagnostics
  ) {
    super(message);
    this.name = 'NotificationProviderError';
  }
}

function canonicalizeJson(value: unknown): string {
  if (
    value === null ||
    typeof value === 'boolean' ||
    typeof value === 'string'
  ) {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new Error(
        'A compiled workflow definition must contain only finite JSON numbers.'
      );
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalizeJson).join(',')}]`;
  }
  if (typeof value === 'object') {
    const object = value as Readonly<Record<string, unknown>>;
    return `{${Object.keys(object)
      .sort()
      .map(key => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`)
      .join(',')}}`;
  }
  throw new Error('A compiled workflow definition must be strict JSON.');
}

export function compiledDefinitionHash(
  workflow: CompiledWorkflowDefinition
): string {
  const hexadecimal = new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(workflow))
    .digest('hex');
  return `sha256:${hexadecimal}`;
}

function defaultNativeCorePath(): string {
  const override = process.env.WOML_RUST_CORE_PATH;
  return override === undefined
    ? resolve(
        import.meta.dir,
        `woml-core.${process.platform}-${process.arch}.node`
      )
    : resolve(override);
}

function defaultScriptHostPath(): string {
  return resolve(
    import.meta.dir,
    import.meta.url.endsWith('.ts') ? 'script-host.ts' : 'script-host.js'
  );
}

function defaultNotificationHostPath(): string {
  return resolve(
    import.meta.dir,
    import.meta.url.endsWith('.ts')
      ? 'notification-provider-host.ts'
      : 'notification-provider-host.js'
  );
}

function loadNativeCore(path: string): NativeCore {
  const require = createRequire(import.meta.url);
  const loaded = require(path) as Partial<NativeCore>;
  if (typeof loaded.executeWomlWorkflow !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose executeWomlWorkflow; rebuild the Rust addon.`
    );
  }
  return loaded as NativeCore;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function exactKeys(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[] = []
): boolean {
  const allowed = new Set([...required, ...optional]);
  return (
    required.every(key => Object.hasOwn(value, key)) &&
    Object.keys(value).every(key => allowed.has(key))
  );
}

function dateTime(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^\d{4}-\d{2}-\d{2}T/.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

function executionResult(value: unknown): value is RustWorkflowExecutionResult {
  if (
    !record(value) ||
    !exactKeys(value, [
      'workflowId',
      'runId',
      'terminalNodeId',
      'result',
      'context',
      'executionOrder',
      'events',
    ]) ||
    typeof value.workflowId !== 'string' ||
    typeof value.runId !== 'string' ||
    typeof value.terminalNodeId !== 'string' ||
    !record(value.context) ||
    !exactKeys(value.context, ['trigger', 'steps']) ||
    !record(value.context.trigger) ||
    !record(value.context.steps) ||
    !Array.isArray(value.executionOrder) ||
    !value.executionOrder.every(nodeId => typeof nodeId === 'string') ||
    !Array.isArray(value.events)
  ) {
    return false;
  }
  return value.events.every(event => {
    if (!record(event)) return false;
    return (
      exactKeys(event, [
        'eventSchemaVersion',
        'eventId',
        'runId',
        'sequence',
        'occurredAt',
        'type',
        'data',
      ]) &&
      Number.isSafeInteger(event.eventSchemaVersion) &&
      Number(event.eventSchemaVersion) >= 1 &&
      Number(event.eventSchemaVersion) <= 6 &&
      typeof event.eventId === 'string' &&
      typeof event.runId === 'string' &&
      Number.isSafeInteger(event.sequence) &&
      dateTime(event.occurredAt) &&
      typeof event.type === 'string'
    );
  });
}

function waitingApproval(value: unknown): value is WaitingApproval {
  if (!record(value)) return false;
  return (
    exactKeys(
      value,
      ['approvalId', 'requestId', 'onTimeout', 'token', 'credentialExpiresAt'],
      ['name', 'description', 'expiresAt']
    ) &&
    /^[a-z][A-Za-z0-9]*$/.test(String(value.approvalId)) &&
    /^aprreq_[A-Za-z0-9_-]+$/.test(String(value.requestId)) &&
    (value.name === undefined ||
      (typeof value.name === 'string' && value.name.length > 0)) &&
    (value.description === undefined ||
      (typeof value.description === 'string' &&
        value.description.length > 0)) &&
    (value.expiresAt === undefined || dateTime(value.expiresAt)) &&
    (value.onTimeout === 'reject' || value.onTimeout === 'fail') &&
    /^apr_[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(String(value.token)) &&
    dateTime(value.credentialExpiresAt)
  );
}

function parseApprovalRuntimeOutcome(json: string): RustApprovalRuntimeOutcome {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.runtime-outcome' ||
    value.version !== 1
  ) {
    throw new Error('The native core returned an invalid approval outcome.');
  }
  if (
    value.status === 'succeeded' &&
    exactKeys(value, ['contract', 'version', 'status', 'execution']) &&
    executionResult(value.execution)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  if (
    value.status === 'waiting' &&
    exactKeys(value, [
      'contract',
      'version',
      'status',
      'workflowId',
      'runId',
      'approval',
    ]) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    waitingApproval(value.approval)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  throw new Error('The native core returned an invalid approval outcome.');
}

function parseApprovalDecisionResult(json: string): ApprovalDecisionResult {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'contract',
      'version',
      'status',
      'runId',
      'approvalId',
      'requestId',
      'decision',
      'source',
      'decidedAt',
    ]) ||
    value.contract !== 'woml.approval-http' ||
    value.version !== 1 ||
    (value.status !== 'accepted' && value.status !== 'already_resolved') ||
    typeof value.runId !== 'string' ||
    typeof value.approvalId !== 'string' ||
    typeof value.requestId !== 'string' ||
    (value.decision !== 'approved' && value.decision !== 'rejected') ||
    value.source !== 'human' ||
    !dateTime(value.decidedAt)
  ) {
    throw new Error('The native core returned an invalid approval decision.');
  }
  return value as unknown as ApprovalDecisionResult;
}

function requireApprovalMethods(native: NativeCore, path: string): void {
  for (const method of [
    'executeWomlWorkflowDurableOutcome',
    'resumeWomlWorkflowDurableOutcome',
    'resolveWomlApproval',
    'settleWomlApprovalTimeout',
  ] as const) {
    if (typeof native[method] !== 'function') {
      throw new Error(
        `Native core at "${path}" does not expose ${method}; rebuild the Rust addon.`
      );
    }
  }
}

function decodeNativeExecutionError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const decoded = JSON.parse(
        message.slice(jsonStart)
      ) as Partial<NativeExecutionErrorEnvelope>;
      const parallelDetails = decoded.details;
      const validParallelDetails =
        parallelDetails === undefined ||
        (typeof parallelDetails === 'object' &&
          parallelDetails !== null &&
          typeof parallelDetails.parallelId === 'string' &&
          (parallelDetails.policy === 'fail-fast' ||
            parallelDetails.policy === 'wait-all') &&
          typeof parallelDetails.primaryNodeId === 'string' &&
          Array.isArray(parallelDetails.failedNodeIds) &&
          parallelDetails.failedNodeIds.every(
            nodeId => typeof nodeId === 'string'
          ) &&
          Array.isArray(parallelDetails.cancelledNodeIds) &&
          parallelDetails.cancelledNodeIds.every(
            nodeId => typeof nodeId === 'string'
          ));
      if (
        decoded.kind === 'woml_execution_error' &&
        typeof decoded.code === 'string' &&
        typeof decoded.message === 'string' &&
        (decoded.nodeId === undefined || typeof decoded.nodeId === 'string') &&
        (decoded.branchId === undefined ||
          typeof decoded.branchId === 'string') &&
        (decoded.armId === undefined || typeof decoded.armId === 'string') &&
        (decoded.referencePath === undefined ||
          (Array.isArray(decoded.referencePath) &&
            decoded.referencePath.every(part => typeof part === 'string'))) &&
        (decoded.branchSite === undefined ||
          decoded.branchSite === 'test' ||
          decoded.branchSite === 'result' ||
          decoded.branchSite === 'selection') &&
        (decoded.approvalId === undefined ||
          typeof decoded.approvalId === 'string') &&
        (decoded.requestId === undefined ||
          typeof decoded.requestId === 'string') &&
        validParallelDetails
      ) {
        throw new RustWorkflowExecutionError(decoded.code, decoded.message, {
          nodeId: decoded.nodeId,
          branchId: decoded.branchId,
          armId: decoded.armId,
          referencePath: decoded.referencePath,
          branchSite: decoded.branchSite,
          approvalId: decoded.approvalId,
          requestId: decoded.requestId,
          parallel: parallelDetails,
        });
      }
    } catch (decodedError) {
      if (decodedError instanceof RustWorkflowExecutionError)
        throw decodedError;
    }
  }
  throw error;
}

function decodeNativeApprovalError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const decoded: unknown = JSON.parse(message.slice(jsonStart));
      if (
        record(decoded) &&
        exactKeys(decoded, ['kind', 'code', 'message']) &&
        decoded.kind === 'woml_approval_error' &&
        typeof decoded.message === 'string' &&
        (decoded.code === 'WOML_APPROVAL_TOKEN_INVALID' ||
          decoded.code === 'WOML_APPROVAL_TOKEN_EXPIRED' ||
          decoded.code === 'WOML_APPROVAL_EXPIRED' ||
          decoded.code === 'WOML_APPROVAL_DECISION_CONFLICT' ||
          decoded.code === 'WOML_APPROVAL_INTERNAL')
      ) {
        throw new ApprovalDecisionError(decoded.code, decoded.message);
      }
    } catch (decodedError) {
      if (decodedError instanceof ApprovalDecisionError) throw decodedError;
    }
  }
  throw new ApprovalDecisionError(
    'WOML_APPROVAL_INTERNAL',
    'The approval decision could not be safely confirmed.'
  );
}

export async function executeWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  options: RustExecutorOptions = {}
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const scriptHostPath = options.scriptHostPath ?? defaultScriptHostPath();
  const bunExecutable = options.bunExecutable ?? process.execPath;
  const native = loadNativeCore(nativePath);
  let resultJson: string;
  try {
    resultJson = await native.executeWomlWorkflow(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      JSON.stringify(options.trigger ?? {}),
      bunExecutable,
      scriptHostPath,
      timeoutMs
    );
  } catch (error) {
    decodeNativeExecutionError(error);
  }
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

export async function executeWorkflowWithRustDurable(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  options: RustExecutorOptions = {}
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.executeWomlWorkflowDurable !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose executeWomlWorkflowDurable; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .executeWomlWorkflowDurable(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      JSON.stringify(options.trigger ?? {}),
      options.bunExecutable ?? process.execPath,
      options.scriptHostPath ?? defaultScriptHostPath(),
      timeoutMs,
      eventStorePath
    )
    .catch(decodeNativeExecutionError);
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

function approvalNative(options: RustExecutorOptions): {
  readonly native: NativeCore;
  readonly path: string;
} {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  requireApprovalMethods(native, path);
  return { native, path };
}

function approvalRuntimeArguments(options: RustExecutorOptions): {
  readonly bunExecutable: string;
  readonly scriptHostPath: string;
  readonly timeoutMs: number;
} {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  return {
    bunExecutable: options.bunExecutable ?? process.execPath,
    scriptHostPath: options.scriptHostPath ?? defaultScriptHostPath(),
    timeoutMs,
  };
}

export async function executeApprovalWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  options: RustExecutorOptions = {}
): Promise<RustApprovalRuntimeOutcome> {
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }
  const { native } = approvalNative(options);
  const runtime = approvalRuntimeArguments(options);
  const resultJson = await native
    .executeWomlWorkflowDurableOutcome(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      JSON.stringify(options.trigger ?? {}),
      runtime.bunExecutable,
      runtime.scriptHostPath,
      runtime.timeoutMs,
      eventStorePath
    )
    .catch(decodeNativeExecutionError);
  return parseApprovalRuntimeOutcome(resultJson);
}

export async function resumeApprovalWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  runId: string,
  options: RustExecutorOptions = {}
): Promise<RustApprovalRuntimeOutcome> {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('eventStorePath and runId must not be empty.');
  }
  const { native } = approvalNative(options);
  const runtime = approvalRuntimeArguments(options);
  const resultJson = await native
    .resumeWomlWorkflowDurableOutcome(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      runId,
      runtime.bunExecutable,
      runtime.scriptHostPath,
      runtime.timeoutMs,
      eventStorePath
    )
    .catch(decodeNativeExecutionError);
  return parseApprovalRuntimeOutcome(resultJson);
}

export function resolveApprovalWithRust(
  eventStorePath: string,
  token: string,
  decision: ApprovalDecision,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): ApprovalDecisionResult {
  const { native } = approvalNative(options);
  try {
    return parseApprovalDecisionResult(
      native.resolveWomlApproval(eventStorePath, token, decision)
    );
  } catch (error) {
    if (error instanceof ApprovalDecisionError) throw error;
    decodeNativeApprovalError(error);
  }
}

export function settleApprovalTimeoutWithRust(
  eventStorePath: string,
  runId: string,
  approvalId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): ApprovalTimeoutResult {
  const { native } = approvalNative(options);
  let json: string;
  try {
    json = native.settleWomlApprovalTimeout(eventStorePath, runId, approvalId);
  } catch (error) {
    decodeNativeApprovalError(error);
  }
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'status',
      'runId',
      'approvalId',
      'requestId',
      'resolution',
      'settledAt',
    ]) ||
    (value.status !== 'settled' &&
      value.status !== 'already_resolved' &&
      value.status !== 'not_due') ||
    typeof value.runId !== 'string' ||
    typeof value.approvalId !== 'string' ||
    typeof value.requestId !== 'string' ||
    (value.settledAt !== null && !dateTime(value.settledAt))
  ) {
    throw new Error('The native core returned an invalid approval timeout.');
  }
  return value as unknown as ApprovalTimeoutResult;
}

export function recoverDurableRuns(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRecoveryReport {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.recoverWomlRuns !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose recoverWomlRuns; rebuild the Rust addon.`
    );
  }
  return JSON.parse(
    native.recoverWomlRuns(eventStorePath)
  ) as RustRecoveryReport;
}

function parseNotificationJourney(
  json: string
): NotificationProviderJourneyResult {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'runId',
      'decision',
      'resolution',
      'deliveries',
      'updates',
      'diagnostics',
    ]) ||
    typeof value.runId !== 'string' ||
    (value.decision !== null && !record(value.decision)) ||
    (value.resolution !== 'approved' &&
      value.resolution !== 'rejected' &&
      value.resolution !== 'timeout_failed') ||
    !record(value.deliveries) ||
    !record(value.updates) ||
    !notificationJourneyDiagnostics(value.diagnostics)
  ) {
    throw new Error('The native core returned an invalid notification journey.');
  }
  return value as unknown as NotificationProviderJourneyResult;
}

function notificationJourneyDiagnostics(
  value: unknown
): value is NotificationJourneyDiagnostics {
  const failureKinds = new Set([
    'secret_not_found',
    'provider_auth_failed',
    'destination_invalid',
    'rate_limited',
    'provider_unavailable',
    'delivery_ambiguous',
    'request_invalid',
    'host_crashed',
    'size_limit_exceeded',
    'update_failed',
  ]);
  if (
    !record(value) ||
    !exactKeys(value, ['version', 'deliveryFailures']) ||
    value.version !== 1 ||
    !Array.isArray(value.deliveryFailures)
  ) {
    return false;
  }
  return value.deliveryFailures.every(item => {
    if (
      !record(item) ||
      !exactKeys(item, [
        'deliveryId',
        'provider',
        'destination',
        'attempt',
        'final',
        'failure',
      ]) ||
      typeof item.deliveryId !== 'string' ||
      !/^[a-z][A-Za-z0-9]*:notify:(0|[1-9][0-9]*):channel:(0|[1-9][0-9]*)$/.test(
        item.deliveryId
      ) ||
      item.provider !== 'slack' ||
      typeof item.destination !== 'string' ||
      !/^(#[a-z0-9][a-z0-9_-]{0,79}|[CG][A-Z0-9]{8,31})$/.test(
        item.destination
      ) ||
      !Number.isSafeInteger(item.attempt) ||
      Number(item.attempt) < 1 ||
      Number(item.attempt) > 3 ||
      typeof item.final !== 'boolean' ||
      !record(item.failure) ||
      !exactKeys(
        item.failure,
        ['kind', 'code', 'message', 'retryable'],
        ['retryAfterMs']
      ) ||
      typeof item.failure.kind !== 'string' ||
      !failureKinds.has(item.failure.kind) ||
      typeof item.failure.code !== 'string' ||
      !/^WOML_[A-Z0-9_]+$/.test(item.failure.code) ||
      typeof item.failure.message !== 'string' ||
      item.failure.message.length < 1 ||
      item.failure.message.length > 1024 ||
      typeof item.failure.retryable !== 'boolean'
    ) {
      return false;
    }
    return (
      item.failure.retryAfterMs === undefined ||
      (Number.isSafeInteger(item.failure.retryAfterMs) &&
        Number(item.failure.retryAfterMs) >= 0 &&
        Number(item.failure.retryAfterMs) <= 86_400_000)
    );
  });
}

function decodeNotificationError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const value: unknown = JSON.parse(message.slice(jsonStart));
      if (
        record(value) &&
        exactKeys(value, ['kind', 'code', 'message'], ['diagnostics']) &&
        value.kind === 'woml_notification_error' &&
        typeof value.code === 'string' &&
        typeof value.message === 'string' &&
        (value.diagnostics === undefined ||
          notificationJourneyDiagnostics(value.diagnostics))
      ) {
        throw new NotificationProviderError(
          value.code,
          value.message,
          value.diagnostics
        );
      }
    } catch (decoded) {
      if (decoded instanceof NotificationProviderError) throw decoded;
    }
  }
  throw new NotificationProviderError(
    'WOML_NOTIFICATION_INTERNAL',
    'The notification provider journey could not be completed safely.'
  );
}

export async function runNotificationProviderJourneyWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath' | 'bunExecutable'> & {
    readonly notificationHostPath?: string;
    readonly interactionTimeoutMs?: number;
  } = {}
): Promise<NotificationProviderJourneyResult> {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.runWomlNotificationProviderJourney !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose runWomlNotificationProviderJourney; rebuild the Rust addon.`
    );
  }
  const timeoutMs = options.interactionTimeoutMs ?? 30_000;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 0xffff_ffff) {
    throw new Error('interactionTimeoutMs must be a positive 32-bit integer.');
  }
  const json = await native
    .runWomlNotificationProviderJourney(
      eventStorePath,
      runId,
      options.bunExecutable ?? process.execPath,
      options.notificationHostPath ?? defaultNotificationHostPath(),
      timeoutMs
    )
    .catch(decodeNotificationError);
  return parseNotificationJourney(json);
}
