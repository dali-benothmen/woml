#!/usr/bin/env bun

import { createHash, randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { dirname, extname, join, resolve } from 'node:path';

import packageMetadata from '../package.json' with { type: 'json' };

import {
  buildWomlDefinitionPackage,
  buildWomlExecutableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  generateWomlEditorDeclarations,
  inspectWomlModuleUsage,
  inspectWomlModuleServiceUsage,
  isWomlElement,
  parseWoml,
  WomlDiagnosticError,
  type CompiledWorkflowDefinition,
  type JsonValue,
  type SecretReferenceExpression,
  type SourcePosition,
  type ValueExpression,
  type WomlSourceDocument,
  type WomlSourceElement,
  type WomlDefinitionPackageV3,
  type WomlDefinitionPackageV5,
  type WomlDefinitionPackageV7,
} from 'woml';
import {
  compiledDefinitionHash,
  executeApprovalWorkflowWithRust,
  executeWorkflowWithRust,
  executeWorkflowWithRustDurable,
  type ExecutionProgressV1,
  NotificationProviderError,
  type NotificationJourneyDiagnostics,
  resolveApprovalWithRust,
  resumeApprovalWorkflowWithRust,
  resumeWorkflowWithRustDurable,
  RustWorkflowExecutionError,
  runNotificationProviderJourneyWithRust,
  cancelRunWithRust,
  inspectRunV2WithRust,
  listRunsWithRust,
  RunManagementError,
  RunInspectionError,
  settleApprovalTimeoutWithRust,
  inspectRunWithRust,
  inspectStoredRunRequirementsWithRust,
  resumeStoredRunWithRust,
  activateWebhookRuntimeWithRust,
  startWebhookRuntimeWithRust,
  stopWebhookRuntimeWithRust,
  submitTriggerOccurrenceWithRust,
  TriggerRuntimeError,
  type IntervalProgressV1,
  type PublicRunStatus,
  type RustRunCancellationResultV1,
  type RustRunInspectionV2,
  type RustRunListV1,
  type ScheduleProgressV1,
  type TriggerProgressV1,
  type WorkflowCallProgressV1,
  type RustApprovalRuntimeOutcome,
  type RustRuntimeModuleArtifact,
  type StoredRunRequirementsV1,
} from './rust-executor';
import {
  ApprovalServerBindError,
  DEFAULT_APPROVAL_PORT,
  serveApprovalAndWait,
} from './approval-server';
import {
  createSecretStore,
  preflightSecretReferences,
  requireValidSecretName,
  SecretStoreError,
  type SecretStore,
} from './secrets';
import { readSecretFromTerminal } from './secrets/prompt';
import {
  SharedSlackTransport,
  type SharedSlackTransportOptions,
} from './notification-provider';
import {
  SlackTriggerHost,
  slackTriggerRegistrations,
  slackTriggerStartupError,
  type SlackTriggerProtocolMessage,
} from './slack-trigger';
import {
  preflightRuntimeConfiguration,
  resolveRuntimeConfiguration,
  type ResolvedRuntimeConfigurationV1,
  type RuntimePreflightV1,
} from './runtime-config';

export interface CliIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
}

const processIo: CliIo = {
  stdout: text => process.stdout.write(text),
  stderr: text => process.stderr.write(text),
};

const WOML_CLI_VERSION = packageMetadata.version;

class CliInputError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'CliInputError';
    this.code = code;
  }
}

function runUsage(): string {
  return 'Usage: woml run <workflow.woml|directory>... [--host <address>] [--port <port>] [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] [--approval-port <port>]';
}

function testUsage(): string {
  return 'Usage: woml test <workflow.woml> [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] [--approval-port <port>]';
}

function listUsage(): string {
  return 'Usage: woml list [--workflow <workflowId>] [--status <status>] [--limit <1-200>] [--state <path>] [--json]';
}

function getUsage(): string {
  return 'Usage: woml get <runId> [--state <path>] [--json]';
}

function cancelUsage(): string {
  return 'Usage: woml cancel <runId> [--state <path>] [--json]';
}

function secretsUsage(): string {
  return [
    'Usage:',
    '  woml secrets set <NAME>',
    '  woml secrets list',
    '  woml secrets delete <NAME>',
  ].join('\n');
}

function emitUsage(): string {
  return 'Usage: woml emit <eventName> --id <publisherEventId> --data @<jsonFile> --server <url> --token-secret <NAME>';
}

function checkUsage(): string {
  return 'Usage: woml check <workflow.woml|directory>... [--config <path>] [--json]';
}

function typesUsage(): string {
  return 'Usage: woml types <workflow.woml|directory> [--output <path>]';
}

function usage(): string {
  return `${runUsage()}\n${testUsage()}\n${checkUsage()}\n${typesUsage()}\n${listUsage()}\n${getUsage()}\n${cancelUsage()}\n${emitUsage()}\n${secretsUsage()}`;
}

interface RunArguments {
  readonly command: 'run' | 'test';
  readonly filePath: string;
  readonly inputPaths: readonly string[];
  readonly statePath: string;
  readonly resumeRunId?: string;
  readonly approvalPort: number;
  readonly host: string;
  readonly port: number;
  readonly triggerId?: string;
}

function parseRunArguments(args: readonly string[]): RunArguments {
  const [command, ...operandsAndOptions] = args;
  const firstOption = operandsAndOptions.findIndex(value =>
    value.startsWith('--')
  );
  const inputPaths = operandsAndOptions.slice(
    0,
    firstOption === -1 ? operandsAndOptions.length : firstOption
  );
  const options =
    firstOption === -1 ? [] : operandsAndOptions.slice(firstOption);
  const filePath = inputPaths[0];
  if (
    (command !== 'run' && command !== 'test') ||
    filePath === undefined ||
    filePath.startsWith('--') ||
    (command === 'test' && inputPaths.length !== 1)
  ) {
    throw new CliInputError(
      'WOML_CLI_ARGUMENTS_INVALID',
      command === 'test' ? testUsage() : runUsage()
    );
  }
  let statePath = resolve('.woml/state.sqlite');
  let resumeRunId: string | undefined;
  let approvalPort = DEFAULT_APPROVAL_PORT;
  let host = '127.0.0.1';
  let port = 3_000;
  let triggerId: string | undefined;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 2) {
    const option = options[index];
    const value = options[index + 1];
    if (
      value === undefined ||
      seen.has(option) ||
      (option !== '--state' &&
        option !== '--resume' &&
        option !== '--approval-port' &&
        option !== '--trigger' &&
        (command !== 'run' || (option !== '--host' && option !== '--port')))
    ) {
      throw new CliInputError(
        'WOML_CLI_ARGUMENTS_INVALID',
        command === 'test' ? testUsage() : runUsage()
      );
    }
    seen.add(option);
    if (option === '--state') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--state requires a non-empty path.'
        );
      }
      statePath = resolve(value);
    } else if (option === '--resume') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--resume requires a run ID.'
        );
      }
      resumeRunId = value;
    } else if (option === '--approval-port') {
      const port = Number(value);
      if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--approval-port must be an integer from 1 through 65535.'
        );
      }
      approvalPort = port;
    } else if (option === '--host') {
      if (value.length === 0 || /[\s/:]/.test(value)) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--host requires an IP address or hostname without a port.'
        );
      }
      host = value;
    } else if (option === '--port') {
      const parsedPort = Number(value);
      if (
        !Number.isSafeInteger(parsedPort) ||
        parsedPort < 1 ||
        parsedPort > 65_535
      ) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--port must be an integer from 1 through 65535.'
        );
      }
      port = parsedPort;
    } else {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--trigger requires a manual trigger ID.'
        );
      }
      triggerId = value;
    }
  }
  if (resumeRunId !== undefined && inputPaths.length !== 1) {
    throw new CliInputError(
      'WOML_RESUME_REQUIRES_SINGLE_WORKFLOW',
      '--resume requires exactly one workflow file, not multiple inputs.'
    );
  }
  return {
    command,
    filePath,
    inputPaths,
    statePath,
    resumeRunId,
    approvalPort,
    host,
    port,
    triggerId,
  };
}

interface RunGetArguments {
  readonly runId: string;
  readonly statePath: string;
  readonly json: boolean;
}

interface RunListArguments {
  readonly statePath: string;
  readonly json: boolean;
  readonly workflowId?: string;
  readonly status?: PublicRunStatus;
  readonly limit: number;
}

interface EmitArguments {
  readonly eventName: string;
  readonly eventId: string;
  readonly dataPath: string;
  readonly server: string;
  readonly tokenSecretName: string;
}

const eventNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/;
const eventIdPattern = /^[A-Za-z0-9][A-Za-z0-9._:-]*$/;
function parseEmitArguments(args: readonly string[]): EmitArguments {
  const [, eventName, ...options] = args;
  if (
    eventName === undefined ||
    eventName.length > 256 ||
    !eventNamePattern.test(eventName) ||
    options.length !== 8
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', emitUsage());
  }
  const values = new Map<string, string>();
  for (let index = 0; index < options.length; index += 2) {
    const option = options[index]!;
    const value = options[index + 1];
    if (
      value === undefined ||
      values.has(option) ||
      !['--id', '--data', '--server', '--token-secret'].includes(option)
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', emitUsage());
    }
    values.set(option, value);
  }
  const eventId = values.get('--id');
  const data = values.get('--data');
  const serverValue = values.get('--server');
  const tokenSecretName = values.get('--token-secret');
  if (
    eventId === undefined ||
    eventId.length > 256 ||
    !eventIdPattern.test(eventId) ||
    data === undefined ||
    !data.startsWith('@') ||
    data.length === 1 ||
    serverValue === undefined ||
    tokenSecretName === undefined
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', emitUsage());
  }
  try {
    requireValidSecretName(tokenSecretName);
  } catch {
    throw new CliInputError(
      'WOML_CLI_ARGUMENTS_INVALID',
      '--token-secret requires a valid symbolic secret name.'
    );
  }
  let server: URL;
  try {
    server = new URL(serverValue);
  } catch {
    throw new CliInputError(
      'WOML_CLI_ARGUMENTS_INVALID',
      '--server requires an absolute http:// or https:// URL.'
    );
  }
  if (
    !['http:', 'https:'].includes(server.protocol) ||
    server.username.length > 0 ||
    server.password.length > 0 ||
    (server.pathname !== '/' && server.pathname !== '') ||
    server.search.length > 0 ||
    server.hash.length > 0
  ) {
    throw new CliInputError(
      'WOML_CLI_ARGUMENTS_INVALID',
      '--server must be an HTTP origin without credentials, a path, query, or fragment.'
    );
  }
  return {
    eventName,
    eventId,
    dataPath: resolve(data.slice(1)),
    server: server.origin,
    tokenSecretName,
  };
}

const publicRunStatuses = new Set<PublicRunStatus>([
  'not_started',
  'queued',
  'running',
  'waiting',
  'cancelling',
  'finalizing',
  'succeeded',
  'failed',
  'cancelled',
]);

function parseRunListArguments(args: readonly string[]): RunListArguments {
  if (args[0] !== 'list') {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', listUsage());
  }
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  let workflowId: string | undefined;
  let status: PublicRunStatus | undefined;
  let limit = 20;
  const seen = new Set<string>();
  for (let index = 1; index < args.length; index += 1) {
    const option = args[index];
    if (seen.has(option)) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', listUsage());
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = args[index + 1];
    if (
      value === undefined ||
      value.startsWith('--') ||
      !['--state', '--workflow', '--status', '--limit'].includes(option)
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', listUsage());
    }
    index += 1;
    if (option === '--state') statePath = resolve(value);
    if (option === '--workflow') workflowId = value;
    if (option === '--status') {
      if (!publicRunStatuses.has(value as PublicRunStatus)) {
        throw new CliInputError(
          'WOML_RUN_STATUS_INVALID',
          `--status must be one of: ${[...publicRunStatuses].join(', ')}.`
        );
      }
      status = value as PublicRunStatus;
    }
    if (option === '--limit') {
      const parsed = Number(value);
      if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 200) {
        throw new CliInputError(
          'WOML_RUN_LIMIT_INVALID',
          '--limit must be an integer from 1 through 200.'
        );
      }
      limit = parsed;
    }
  }
  return { statePath, json, workflowId, status, limit };
}

function parseRunGetArguments(
  args: readonly string[],
  command: 'get' | 'cancel'
): RunGetArguments {
  const [, runId, ...options] = args;
  const commandUsage = command === 'get' ? getUsage() : cancelUsage();
  if (
    args[0] !== command ||
    runId === undefined ||
    runId.length === 0 ||
    runId.startsWith('--')
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', commandUsage);
  }
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index];
    if (seen.has(option)) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', commandUsage);
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = options[index + 1];
    if (option !== '--state' || value === undefined || value.startsWith('--')) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', commandUsage);
    }
    statePath = resolve(value);
    index += 1;
  }
  return {
    runId,
    statePath,
    json,
  };
}

function runtimeCode(code: string): string {
  if (code === 'WOML_SCRIPT_NON_JSON_RESULT') return 'WOML_NON_JSON_RESULT';
  if (code.startsWith('WOML_SCRIPT_')) return 'WOML_SCRIPT_FAILED';
  return code;
}

function formatRunList(list: RustRunListV1): string {
  if (list.runs.length === 0) return 'No WOML runs found.\n';
  const lines = ['RUN ID\tWORKFLOW\tSTATUS\tADMITTED\tSTARTED\tQUEUE\tWAITING'];
  if (list.profile === 'woml.run-list/v2') {
    for (const run of list.runs) {
      lines.push(
        `${run.runId}\t${run.workflowId}\t${run.status}\t${run.admittedAt}\t${run.startedAt ?? '-'}\t${run.queue ?? '-'}\t${run.waitingFor?.replaceAll('_', ' ') ?? '-'}`
      );
    }
  } else {
    for (const run of list.runs) {
      lines.push(
        `${run.runId}\t${run.workflowId}\t${run.status}\t${run.startedAt}\t${run.startedAt}\t-\t-`
      );
    }
  }
  return `${lines.join('\n')}\n`;
}

function formatRunInspection(inspection: RustRunInspectionV2): string {
  const lines = [
    `Run: ${inspection.runId}`,
    `Workflow: ${inspection.workflowId}`,
    `Status: ${inspection.status}`,
    `Business outcome: ${inspection.businessOutcome}`,
    `Lifecycle: ${inspection.lifecycleStatus}`,
    `Cancellation requested: ${inspection.cancellation.requested ? 'yes' : 'no'}`,
  ];
  if (inspection.policy !== undefined) {
    lines.push(`Policy queue: ${inspection.policy.queue}`);
    if (inspection.policy.waitingFor !== undefined) {
      lines.push(
        `Policy waiting for: ${inspection.policy.waitingFor.replaceAll('_', ' ')}`
      );
    }
    if (inspection.policy.eligibleAt !== undefined) {
      lines.push(`Policy eligible at: ${inspection.policy.eligibleAt}`);
    }
    if (inspection.policy.timeoutAt !== undefined) {
      lines.push(`Workflow timeout at: ${inspection.policy.timeoutAt}`);
    }
  }
  if (inspection.hooks.length > 0) {
    lines.push('Lifecycle hooks:');
    for (const hook of inspection.hooks) {
      const subject = `${hook.subjectKind} ${hook.subjectId}`;
      const failures =
        hook.failedActions === 0 ? '' : `, ${hook.failedActions} failed action(s)`;
      lines.push(`  ${hook.hookId}: ${hook.status} (${subject}${failures})`);
    }
  }
  if (inspection.warnings.length > 0) {
    lines.push(
      `Warnings: ${inspection.warnings.map(warning => warning.code).join(', ')}`
    );
  }
  return `${lines.join('\n')}\n`;
}

function formatCancellation(result: RustRunCancellationResultV1): string {
  if (result.status === 'accepted') {
    return `Cancellation requested for run ${result.runId}.\n`;
  }
  if (result.status === 'already_requested') {
    return `Cancellation was already requested for run ${result.runId}.\n`;
  }
  if (result.status === 'already_cancelled') {
    return `Run ${result.runId} is already cancelled.\n`;
  }
  return `Cancellation was rejected for run ${result.runId}${
    result.code === undefined ? '.' : `: ${result.code}.`
  }\n`;
}

export function formatExecutionProgress(progress: ExecutionProgressV1): string {
  if (
    'profile' in progress &&
    progress.profile === 'woml.runtime-policy-progress/v1'
  ) {
    if (progress.phase === 'queued') {
      const reason =
        progress.waitingFor === undefined
          ? ''
          : ` for ${progress.waitingFor.replaceAll('_', ' ')}`;
      const eligible =
        progress.eligibleAt === undefined
          ? ''
          : `; eligible at ${progress.eligibleAt}`;
      return `Run ${progress.runId} queued in "${progress.queue}"${reason}${eligible}.`;
    }
    if (progress.phase === 'eligible') {
      return `Run ${progress.runId} is eligible in queue "${progress.queue}".`;
    }
    if (progress.phase === 'started') {
      return `Run ${progress.runId} started under runtime policy.`;
    }
    return `Run ${progress.runId} timed out [${progress.code ?? 'WOML_WORKFLOW_TIMED_OUT'}].`;
  }
  if ('profile' in progress) {
    const status = progress.phase.replaceAll('_', ' ');
    const subject =
      progress.stepId === undefined ? '' : ` for step ${progress.stepId}`;
    return `Lifecycle ${progress.hookId}${subject} ${status}${progress.code === undefined ? '.' : `: ${progress.code}`}`;
  }
  if (progress.type === 'step_attempt_failed') {
    return `Step ${progress.nodeId} failed (attempt ${progress.attempt}/${progress.maxAttempts}): ${progress.failureCode}`;
  }
  if (progress.type === 'step_attempt_succeeded') {
    return `Step ${progress.nodeId} succeeded on attempt ${progress.attempt}/${progress.maxAttempts}.`;
  }
  const remainingMs = Math.max(
    0,
    Date.parse(progress.scheduledAt) - Date.now()
  );
  const delay =
    remainingMs === 0
      ? 'now'
      : remainingMs < 500
        ? `in ${Math.max(1, Math.ceil(remainingMs))}ms`
        : `in ${Math.ceil(remainingMs / 1_000)}s`;
  return `Retry ${progress.nextAttempt}/${progress.maxAttempts} scheduled ${delay}.`;
}

function durableRetryProgress(
  io: CliIo,
  args: RunArguments
): (progress: ExecutionProgressV1) => void {
  let recoveryPrinted = false;
  return progress => {
    io.stderr(`${formatExecutionProgress(progress)}\n`);
    if (
      'type' in progress &&
      progress.type === 'step_retry_scheduled' &&
      !recoveryPrinted
    ) {
      recoveryPrinted = true;
      io.stderr(
        `Recovery: woml run ${JSON.stringify(args.filePath)} --state ${JSON.stringify(
          args.statePath
        )} --resume ${JSON.stringify(progress.runId)}\n`
      );
    }
  };
}

function stepSourcePosition(
  document: WomlSourceDocument | undefined,
  nodeId: string | undefined
): SourcePosition | undefined {
  if (document === undefined || nodeId === undefined) return undefined;
  const pending = [document.root];
  while (pending.length > 0) {
    const element = pending.shift()!;
    if (element.name === 'step' && element.attributes.id?.value === nodeId) {
      return scriptSourcePosition(element);
    }
    for (const child of element.children) {
      if (isWomlElement(child)) pending.push(child);
    }
  }
  return undefined;
}

function scriptSourcePosition(element: WomlSourceElement): SourcePosition {
  const script = childElements(element).find(child => child.name === 'script');
  return (
    script?.children[0]?.span.start ??
    script?.openTagSpan.start ??
    element.openTagSpan.start
  );
}

function childElements(
  element: WomlSourceElement
): readonly WomlSourceElement[] {
  return element.children.filter(isWomlElement);
}

function findBranch(
  document: WomlSourceDocument,
  branchId: string
): WomlSourceElement | undefined {
  const pending = [document.root];
  while (pending.length > 0) {
    const element = pending.shift()!;
    if (
      element.name === 'branch' &&
      element.attributes.id?.value === branchId
    ) {
      return element;
    }
    pending.push(...childElements(element));
  }
  return undefined;
}

function findParallel(
  document: WomlSourceDocument,
  parallelId: string
): WomlSourceElement | undefined {
  const pending = [document.root];
  while (pending.length > 0) {
    const element = pending.shift()!;
    if (
      element.name === 'parallel' &&
      element.attributes.id?.value === parallelId
    ) {
      return element;
    }
    pending.push(...childElements(element));
  }
  return undefined;
}

interface RuntimeSource {
  readonly position: SourcePosition;
  readonly subject: string;
}

function parallelRuntimeSource(
  document: WomlSourceDocument | undefined,
  error: RustWorkflowExecutionError
): RuntimeSource | undefined {
  if (document === undefined || error.parallelId === undefined)
    return undefined;
  const parallel = findParallel(document, error.parallelId);
  if (parallel === undefined) return undefined;

  if (
    error.code === 'WOML_PARALLEL_CHILD_FAILED' &&
    error.primaryNodeId !== undefined
  ) {
    const primary = childElements(parallel).find(
      child =>
        child.name === 'step' &&
        child.attributes.id?.value === error.primaryNodeId
    );
    if (primary !== undefined) {
      return {
        position: scriptSourcePosition(primary),
        subject: `step "${error.primaryNodeId}" in parallel "${error.parallelId}"`,
      };
    }
  }

  return {
    position:
      parallel.attributes.id?.valueSpan.start ?? parallel.openTagSpan.start,
    subject: `parallel "${error.parallelId}"`,
  };
}

function branchRuntimeSource(
  document: WomlSourceDocument | undefined,
  error: RustWorkflowExecutionError
): RuntimeSource | undefined {
  if (document === undefined || error.branchId === undefined) return undefined;
  const branch = findBranch(document, error.branchId);
  if (branch === undefined) return undefined;

  if (error.branchSite === 'selection') {
    return {
      position:
        branch.attributes.id?.valueSpan.start ?? branch.openTagSpan.start,
      subject: `branch "${error.branchId}"`,
    };
  }

  const arm = childElements(branch).find((candidate, index) => {
    const armId =
      candidate.name === 'otherwise'
        ? `${error.branchId}:otherwise`
        : `${error.branchId}:when:${index}`;
    return armId === error.armId;
  });
  if (arm === undefined) {
    return {
      position:
        branch.attributes.id?.valueSpan.start ?? branch.openTagSpan.start,
      subject: `branch "${error.branchId}"`,
    };
  }

  if (error.branchSite === 'test') {
    return {
      position: arm.attributes.test?.valueSpan.start ?? arm.openTagSpan.start,
      subject: `<when test> in branch "${error.branchId}"`,
    };
  }

  const result = childElements(arm).find(element => element.name === 'result');
  return {
    position:
      result?.attributes.value?.valueSpan.start ??
      result?.openTagSpan.start ??
      arm.openTagSpan.start,
    subject: `<result value> in branch "${error.branchId}"`,
  };
}

function formatError(
  error: unknown,
  filePath?: string,
  document?: WomlSourceDocument
): string {
  if (error instanceof CliInputError) {
    return `WOML input error [${error.code}]${
      filePath === undefined ? '' : ` in "${filePath}"`
    }: ${error.message}`;
  }

  if (error instanceof WomlDiagnosticError) {
    const { diagnostic } = error;
    const location = `${diagnostic.file}:${diagnostic.location.start.line}:${diagnostic.location.start.column}`;
    const hint =
      diagnostic.hint === undefined ? '' : ` Hint: ${diagnostic.hint}`;
    return `WOML ${diagnostic.phase} error [${diagnostic.code}] at ${location}: ${diagnostic.message}${hint}`;
  }

  if (error instanceof ApprovalServerBindError) {
    return `WOML CLI error [${error.code}]: ${error.message}`;
  }

  if (error instanceof SecretStoreError) {
    return `WOML secrets error [${error.code}]: ${error.message}`;
  }

  if (error instanceof TriggerRuntimeError) {
    return `WOML trigger error [${error.code}]: ${error.message}`;
  }

  if (error instanceof RunInspectionError) {
    return `WOML run error [${error.code}]: ${error.message}`;
  }

  if (error instanceof RunManagementError) {
    return `WOML run error [${error.code}]: ${error.message}`;
  }

  if (error instanceof NotificationProviderError) {
    const details = formatNotificationDeliveryFailures(error.diagnostics);
    return `WOML notification error [${error.code}]: ${error.message}${details}`;
  }

  if (error instanceof RustWorkflowExecutionError) {
    const parallelSource = parallelRuntimeSource(document, error);
    const branchSource = branchRuntimeSource(document, error);
    const position =
      parallelSource?.position ??
      branchSource?.position ??
      stepSourcePosition(document, error.nodeId);
    const location =
      position !== undefined && filePath !== undefined
        ? ` at ${filePath}:${position.line}:${position.column}`
        : filePath === undefined
          ? ''
          : ` in "${filePath}"`;
    const subject =
      parallelSource?.subject ??
      branchSource?.subject ??
      (error.nodeId === undefined ? undefined : `step "${error.nodeId}"`);
    const identity = subject === undefined ? '' : ` (${subject})`;
    return `WOML runtime error [${runtimeCode(error.code)}]${location}${identity}: ${error.message}`;
  }

  const message = error instanceof Error ? error.message : String(error);
  return `WOML internal error${
    filePath === undefined ? '' : ` in "${filePath}"`
  }: ${message}`;
}

function formatNotificationDeliveryFailures(
  diagnostics: NotificationJourneyDiagnostics | undefined
): string {
  if (diagnostics === undefined || diagnostics.deliveryFailures.length === 0) {
    return '';
  }
  return diagnostics.deliveryFailures
    .map(({ provider, destination, failure }) => {
      const label = provider === 'slack' ? 'Slack' : provider;
      return `\n${label} notification to ${destination} failed [${failure.code}]: ${failure.message}`;
    })
    .join('');
}

function printNotificationWarnings(
  io: CliIo,
  diagnostics: NotificationJourneyDiagnostics
): void {
  if (diagnostics.deliveryFailures.length === 0) return;
  io.stderr(
    `\nWOML notification warning: some approval notifications could not be delivered.${formatNotificationDeliveryFailures(
      diagnostics
    )}\n\n`
  );
}

async function readWorkflow(filePath: string): Promise<string> {
  if (extname(filePath) !== '.woml') {
    throw new CliInputError(
      'WOML_INVALID_FILE_EXTENSION',
      'workflow files must use the .woml extension.'
    );
  }

  let file;
  try {
    file = await stat(filePath);
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
      throw new CliInputError(
        'WOML_FILE_NOT_FOUND',
        'workflow file does not exist.'
      );
    }
    throw error;
  }

  if (!file.isFile()) {
    throw new CliInputError(
      'WOML_NOT_A_FILE',
      'workflow path must point to a file.'
    );
  }

  return await Bun.file(filePath).text();
}

interface CompiledWorkflowSource {
  readonly filePath: string;
  readonly document: WomlSourceDocument;
  readonly workflow: CompiledWorkflowDefinition;
  readonly definitionHash: string;
  readonly runtimeModules: readonly RustRuntimeModuleArtifact[];
  readonly sourceSnapshot: readonly SourceSnapshotEntry[];
}

export interface SourceSnapshotEntry {
  readonly path: string;
  readonly digest: string;
}

function sourceDigest(source: string | Uint8Array): string {
  return `sha256:${createHash('sha256').update(source).digest('hex')}`;
}

function samePackageSources(
  left: readonly { readonly path: string; readonly digest: string }[],
  right: readonly { readonly path: string; readonly digest: string }[]
): boolean {
  return (
    left.length === right.length &&
    left.every(
      (source, index) =>
        source.path === right[index]?.path &&
        source.digest === right[index]?.digest
    )
  );
}

export async function assertStableSourceSnapshot(
  sources: readonly {
    readonly sourceSnapshot: readonly SourceSnapshotEntry[];
  }[]
): Promise<void> {
  for (const source of sources) {
    for (const snapshot of source.sourceSnapshot) {
      let content: Buffer;
      try {
        content = await readFile(snapshot.path);
      } catch {
        throw new CliInputError(
          'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
          `source "${snapshot.path}" changed or became unreadable during activation.`
        );
      }
      if (sourceDigest(content) !== snapshot.digest) {
        throw new CliInputError(
          'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
          `source "${snapshot.path}" changed while the deployment was being compiled; run the command again.`
        );
      }
    }
  }
}

async function assertStableWorkflowInputSet(
  inputPaths: readonly string[],
  sources: readonly Pick<CompiledWorkflowSource, 'filePath'>[]
): Promise<void> {
  const current = new Set<string>();
  for (const inputPath of inputPaths) {
    for (const path of await workflowFilePaths(inputPath))
      current.add(resolve(path));
  }
  const expected = [
    ...new Set(sources.map(source => resolve(source.filePath))),
  ].sort();
  const observed = [...current].sort();
  if (
    expected.length !== observed.length ||
    expected.some((path, index) => path !== observed[index])
  ) {
    throw new CliInputError(
      'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
      'the workflow input set changed while the deployment was activating; run the command again.'
    );
  }
}

function promoteForLifecycleAuthority(
  workflow: CompiledWorkflowDefinition
): CompiledWorkflowDefinition {
  if (workflow.schemaVersion === 11 || workflow.schemaVersion === 12) {
    return workflow;
  }
  return {
    schemaVersion: 11,
    workflowId: workflow.workflowId,
    ...(workflow.metadata === undefined ? {} : { metadata: workflow.metadata }),
    triggers: workflow.triggers,
    graph: {
      entryNodeIds: workflow.graph.entryNodeIds,
      nodes: workflow.graph.nodes.map(node =>
        node.handler !== 'runtime.script' || node.scriptRuntime !== undefined
          ? node
          : {
              ...node,
              scriptRuntime: {
                bindingVersion: 1 as const,
                bindings: [
                  'context',
                  'attempt',
                  'services',
                  'secrets',
                ] as const,
                requiredSecrets: [],
              },
            }
      ),
      edges: workflow.graph.edges,
    },
    ...('moduleRuntime' in workflow && workflow.moduleRuntime !== undefined
      ? { moduleRuntime: workflow.moduleRuntime }
      : {}),
  };
}

function workflowCallFrontendOnlySource(
  source: CompiledWorkflowSource
): boolean {
  return (
    source.workflow.triggers.length === 0 ||
    inspectWomlModuleUsage(source.document).referencedServices.includes(
      'workflows'
    )
  );
}

function runtimeModulesFromPackage(
  definitionPackage:
    | WomlDefinitionPackageV3
    | WomlDefinitionPackageV5
    | WomlDefinitionPackageV7
): readonly RustRuntimeModuleArtifact[] {
  return definitionPackage.modules.map(module => {
    const bundle = definitionPackage.artifacts.find(
      artifact => artifact.path === module.bundle.path
    );
    const sourceMap = definitionPackage.artifacts.find(
      artifact => artifact.path === module.sourceMap.path
    );
    if (bundle?.kind !== 'module-bundle' || sourceMap?.kind !== 'source-map') {
      throw new Error(
        `Runtime artifacts are incomplete for module "${module.name}".`
      );
    }
    return {
      name: module.name,
      bundleDigest: module.bundle.digest,
      sourceMapDigest: module.sourceMap.digest,
      exports: module.exports,
      bundle: bundle.content,
      sourceMap: sourceMap.content,
    };
  });
}

async function workflowFilePaths(
  inputPath: string
): Promise<readonly string[]> {
  let entry;
  try {
    entry = await stat(inputPath);
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
      throw new CliInputError(
        'WOML_FILE_NOT_FOUND',
        'workflow file or directory does not exist.'
      );
    }
    throw error;
  }
  if (entry.isFile()) return [inputPath];
  if (!entry.isDirectory()) {
    throw new CliInputError(
      'WOML_NOT_A_FILE',
      'workflow path must point to a .woml file or directory.'
    );
  }
  const paths = (await readdir(inputPath, { withFileTypes: true }))
    .filter(child => child.isFile() && extname(child.name) === '.woml')
    .map(child => join(inputPath, child.name))
    .sort((left, right) => left.localeCompare(right));
  if (paths.length === 0) {
    throw new CliInputError(
      'WOML_WORKFLOW_DIRECTORY_EMPTY',
      'workflow directory does not contain any direct .woml files.'
    );
  }
  return paths;
}

async function compileWorkflowSources(
  inputPath: string
): Promise<readonly CompiledWorkflowSource[]> {
  const compiled: CompiledWorkflowSource[] = [];
  for (const filePath of await workflowFilePaths(inputPath)) {
    const source = await readWorkflow(filePath);
    const document = parseWoml(source, { file: filePath });
    const projectRoot = moduleProjectRoot(filePath);
    const inspected = buildWomlDefinitionPackage(document, {
      sourcePath: filePath,
      projectRoot,
    });
    if (inspected.modules.length > 0) {
      inspectWomlModuleServiceUsage(document, { sourcePath: filePath, projectRoot });
    }
    const executablePackage =
      inspected.modules.length > 0
        ? await buildWomlExecutableDefinitionPackage(document, {
            sourcePath: filePath,
            projectRoot,
          })
        : undefined;
    // Definition Package v7 is the frozen Model v12 compilation identity. RP6
    // promotes its exact reviewed artifacts directly at activation time rather
    // than mutating that immutable package into a second public shape.
    const runtimePackage =
      executablePackage?.schemaVersion === 7
        ? executablePackage
        : inspected.modules.length > 0
          ? await buildWomlRuntimeDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            })
          : undefined;
    const packageSources = runtimePackage?.sources ?? inspected.sources;
    if (
      runtimePackage !== undefined &&
      !samePackageSources(inspected.sources, runtimePackage.sources)
    ) {
      throw new CliInputError(
        'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
        `workflow or module source changed while "${filePath}" was being compiled; run the command again.`
      );
    }
    const frontendWorkflow =
      runtimePackage?.workflow.model ?? compileWoml(document);
    const workflow = promoteForLifecycleAuthority(frontendWorkflow);
    const definitionHash = compiledDefinitionHash(workflow);
    compiled.push({
      filePath,
      document,
      workflow,
      definitionHash,
      runtimeModules:
        runtimePackage === undefined
          ? []
          : runtimeModulesFromPackage(runtimePackage),
      sourceSnapshot: packageSources.map(item => ({
        path: resolve(projectRoot, item.path),
        digest: item.digest,
      })),
    });
  }
  const workflowIds = new Set<string>();
  for (const item of compiled) {
    if (workflowIds.has(item.workflow.workflowId)) {
      throw new CliInputError(
        'WOML_WORKFLOW_ID_DUPLICATE',
        `workflow ID "${item.workflow.workflowId}" is declared more than once.`
      );
    }
    workflowIds.add(item.workflow.workflowId);
  }
  return compiled;
}

async function compileWorkflowInputs(
  inputPaths: readonly string[]
): Promise<readonly CompiledWorkflowSource[]> {
  const compiled: CompiledWorkflowSource[] = [];
  const seenFiles = new Set<string>();
  const filePaths: string[] = [];
  const inputSnapshots: Array<{
    readonly inputPath: string;
    readonly files: readonly string[];
    readonly directory: boolean;
  }> = [];
  for (const inputPath of inputPaths) {
    const resolvedFiles = (await workflowFilePaths(inputPath)).map(path =>
      resolve(path)
    );
    const directory = (await stat(inputPath)).isDirectory();
    inputSnapshots.push({ inputPath, files: resolvedFiles, directory });
    for (const filePath of resolvedFiles) {
      const absolutePath = resolve(filePath);
      if (seenFiles.has(absolutePath)) continue;
      seenFiles.add(absolutePath);
      filePaths.push(absolutePath);
    }
  }
  filePaths.sort((left, right) => left.localeCompare(right));
  for (const filePath of filePaths) {
    for (const source of await compileWorkflowSources(filePath)) {
      compiled.push(source);
    }
  }
  const workflowIds = new Set<string>();
  for (const source of compiled) {
    if (workflowIds.has(source.workflow.workflowId)) {
      throw new CliInputError(
        'WOML_WORKFLOW_ID_DUPLICATE',
        `workflow ID "${source.workflow.workflowId}" is declared more than once.`
      );
    }
    workflowIds.add(source.workflow.workflowId);
  }
  await assertStableSourceSnapshot(compiled);
  for (const snapshot of inputSnapshots) {
    if (!snapshot.directory) continue;
    const current = (await workflowFilePaths(snapshot.inputPath)).map(path =>
      resolve(path)
    );
    if (
      current.length !== snapshot.files.length ||
      current.some((path, index) => path !== snapshot.files[index])
    ) {
      throw new CliInputError(
        'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
        `workflow directory "${snapshot.inputPath}" changed while the deployment was being compiled; run the command again.`
      );
    }
  }
  return compiled;
}

export function activationIdentity(
  sources: readonly Pick<
    CompiledWorkflowSource,
    'workflow' | 'definitionHash' | 'runtimeModules'
  >[]
): string {
  const material = sources
    .map(source => ({
      workflowId: source.workflow.workflowId,
      definitionHash: source.definitionHash,
      modules: source.runtimeModules
        .map(module => ({
          name: module.name,
          bundleDigest: module.bundleDigest,
          sourceMapDigest: module.sourceMapDigest,
        }))
        .sort((left, right) => left.name.localeCompare(right.name)),
    }))
    .sort((left, right) => left.workflowId.localeCompare(right.workflowId));
  return `sha256:${createHash('sha256')
    .update(JSON.stringify(material))
    .digest('hex')}`;
}

function editorTypesPath(inputPath: string, inputIsDirectory: boolean): string {
  return join(
    inputIsDirectory ? inputPath : dirname(inputPath),
    'woml-env.d.ts'
  );
}

async function refreshEditorTypes(
  inputPath: string,
  modules: readonly {
    readonly name: string;
    readonly exports: readonly string[];
  }[],
  io: CliIo
): Promise<void> {
  let outputPath = join(dirname(inputPath), 'woml-env.d.ts');
  try {
    outputPath = editorTypesPath(
      inputPath,
      (await stat(inputPath)).isDirectory()
    );
    await writeFile(
      outputPath,
      generateWomlEditorDeclarations(modules),
      'utf8'
    );
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    io.stderr(
      `Warning [WOML_EDITOR_TYPES_WRITE_FAILED]: Could not refresh ${outputPath}: ${reason}\nWorkflow execution can continue; use woml types <workflow> --output <path> to choose a writable location.\n`
    );
  }
}

async function runSingleCheckCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  const [, rawFilePath, ...options] = args;
  if (
    rawFilePath === undefined ||
    rawFilePath.startsWith('--') ||
    options.length > 1 ||
    (options.length === 1 && options[0] !== '--json')
  ) {
    io.stderr(`${checkUsage()}\n`);
    return 2;
  }
  const filePath = resolve(rawFilePath);
  let document: WomlSourceDocument | undefined;
  try {
    const source = await readWorkflow(filePath);
    document = parseWoml(source, { file: filePath });
    const inspectionPackage = buildWomlDefinitionPackage(document, {
      sourcePath: filePath,
      projectRoot: moduleProjectRoot(filePath),
    });
    const workflowElement = document.root.children.find(
      (child): child is WomlSourceElement =>
        child.kind === 'element' && child.name === 'workflow'
    );
    const declaresLifecycle =
      workflowElement !== undefined &&
      workflowElement.children.some(
        child => child.kind === 'element' && child.name === 'lifecycle'
      );
    const declaresRuntimePolicy =
      workflowElement !== undefined &&
      workflowElement.children.some(
        child => child.kind === 'element' && child.name === 'config'
      );
    const definitionPackage =
      inspectionPackage.modules.length === 0
        ? inspectionPackage
        : declaresLifecycle || declaresRuntimePolicy
          ? await buildWomlExecutableDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot: moduleProjectRoot(filePath),
            })
          : await buildWomlRuntimeDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot: moduleProjectRoot(filePath),
            });
    const compiledWorkflow =
      definitionPackage.schemaVersion === 1
        ? compileWoml(document)
        : definitionPackage.workflow.model;
    const moduleServiceUsage =
      definitionPackage.modules.length === 0
        ? undefined
        : inspectWomlModuleServiceUsage(document, {
            sourcePath: filePath,
            projectRoot: moduleProjectRoot(filePath),
          });
    await refreshEditorTypes(filePath, definitionPackage.modules, io);
    if (options[0] === '--json') {
      io.stdout(`${JSON.stringify(definitionPackage, null, 2)}\n`);
      return 0;
    }
    io.stdout(
      `WOML check passed for workflow "${definitionPackage.workflow.id}".\n`
    );
    io.stdout(`Definition package: ${definitionPackage.rootHash}\n`);
    io.stdout(
      `Modules: ${definitionPackage.modules.length}; dependency sources: ${Math.max(0, definitionPackage.sources.length - 1)}.\n`
    );
    for (const module of definitionPackage.modules) {
      io.stdout(
        `services.${module.name} -> ${module.entrypoint} (${module.exports.join(', ')})\n`
      );
    }
    if ((moduleServiceUsage?.durableStateSources.length ?? 0) > 0) {
      io.stdout(
        `Durable state usage: ${moduleServiceUsage!.durableStateSources.length} local module source(s).\n`
      );
    }
    const usage = inspectWomlModuleUsage(document);
    for (const name of usage.unusedModules) {
      io.stdout(
        `Warning [WOML_MODULE_UNUSED]: services.${name} is declared but is not called by this workflow.\n`
      );
    }
    const hasLifecycle =
      (compiledWorkflow.schemaVersion === 11 ||
        compiledWorkflow.schemaVersion === 12) &&
      compiledWorkflow.lifecycle !== undefined;
    const hasRuntimePolicy = compiledWorkflow.schemaVersion === 12;
    const workflowCallsFrontendOnly =
      compiledWorkflow.triggers.length === 0 ||
      usage.referencedServices.includes('workflows');
    io.stdout(
      hasRuntimePolicy && definitionPackage.modules.length > 0
        ? 'Execution: Model v12 runtime policies and compiled local modules are executable together.\n'
        : hasRuntimePolicy
          ? 'Execution: Model v12 concurrency, durable FIFO queueing, rolling-window rate limits, and workflow timeouts are executable.\n'
        : hasLifecycle
          ? 'Execution: workflow and step lifecycle scripts plus informational Slack notifications are executable.\n'
          : workflowCallsFrontendOnly
            ? 'Execution: Workflow Calls are valid and executable through the durable Rust runtime.\n'
            : definitionPackage.modules.length === 0
              ? 'Execution: module-free workflow; woml run is available.\n'
              : 'Execution: local modules are compiled and ready for woml run.\n'
    );
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error, filePath, document)}\n`);
    return 1;
  }
}

interface CheckArguments {
  readonly inputPaths: readonly string[];
  readonly configPath?: string;
  readonly json: boolean;
}

function parseCheckArguments(args: readonly string[]): CheckArguments {
  if (args[0] !== 'check') {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
  }
  const inputPaths: string[] = [];
  let configPath: string | undefined;
  let json = false;
  for (let index = 1; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--json') {
      if (json) throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      json = true;
      continue;
    }
    if (argument === '--config') {
      const value = args[index + 1];
      if (configPath !== undefined || value === undefined || value.startsWith('--')) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      configPath = value;
      index += 1;
      continue;
    }
    if (argument.startsWith('--')) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
    }
    inputPaths.push(resolve(argument));
  }
  if (inputPaths.length === 0) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
  }
  return {
    inputPaths,
    ...(configPath === undefined ? {} : { configPath }),
    json,
  };
}

function validateDeploymentRoutes(
  sources: readonly CompiledWorkflowSource[]
): void {
  const claimed = new Map<string, { readonly workflowId: string; readonly triggerId: string }>();
  for (const source of sources) {
    for (const route of webhookRouteSummaries(source.workflow)) {
      const previous = claimed.get(route.path);
      if (previous !== undefined) {
        throw new CliInputError(
          'WOML_WEBHOOK_ROUTE_CONFLICT',
          `webhook route "${route.path}" is claimed by ${previous.workflowId}/${previous.triggerId} and ${source.workflow.workflowId}/${route.triggerId}.`
        );
      }
      claimed.set(route.path, {
        workflowId: source.workflow.workflowId,
        triggerId: route.triggerId,
      });
    }
  }
}

interface ProductionPreflightReportV1 {
  readonly profile: 'woml.production-preflight/v1';
  readonly status: 'passed';
  readonly configuration?: ResolvedRuntimeConfigurationV1;
  readonly environment?: RuntimePreflightV1;
  readonly secretProvider?: string;
  readonly workflows: readonly {
    readonly workflowId: string;
    readonly sourcePath: string;
    readonly definitionHash: string;
    readonly triggerCount: number;
    readonly moduleCount: number;
    readonly requiredSecrets: readonly string[];
  }[];
}

async function runDeploymentCheckCommand(
  parsed: CheckArguments,
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  const displayPath = parsed.inputPaths[0];
  try {
    const sources = await compileWorkflowInputs(parsed.inputPaths);
    validateDeploymentRoutes(sources);
    for (const source of sources) {
      await refreshEditorTypes(source.filePath, source.runtimeModules, io);
    }

    let configuration: ResolvedRuntimeConfigurationV1 | undefined;
    let environment: RuntimePreflightV1 | undefined;
    let secretProvider: string | undefined;
    if (parsed.configPath !== undefined) {
      configuration = await resolveRuntimeConfiguration(parsed.configPath);
      environment = await preflightRuntimeConfiguration(configuration);
      const store = dependencies.createSecretStore();
      secretProvider = store.provider;
      await preflightSecretReferences(
        sources.flatMap(source => [...workflowSecretReferences(source.workflow)]),
        store
      );
    }

    const report: ProductionPreflightReportV1 = {
      profile: 'woml.production-preflight/v1',
      status: 'passed',
      ...(configuration === undefined ? {} : { configuration }),
      ...(environment === undefined ? {} : { environment }),
      ...(secretProvider === undefined ? {} : { secretProvider }),
      workflows: sources.map(source => ({
        workflowId: source.workflow.workflowId,
        sourcePath: source.filePath,
        definitionHash: source.definitionHash,
        triggerCount: source.workflow.triggers.length,
        moduleCount: source.runtimeModules.length,
        requiredSecrets: [
          ...new Set(
            [...workflowSecretReferences(source.workflow)].map(
              reference => reference.name
            )
          ),
        ].sort(),
      })),
    };
    if (parsed.json) {
      io.stdout(`${JSON.stringify(report, null, 2)}\n`);
      return 0;
    }
    io.stdout(
      `WOML production check passed for ${sources.length} workflow${sources.length === 1 ? '' : 's'}.\n`
    );
    for (const workflow of report.workflows) {
      io.stdout(
        `Workflow ${workflow.workflowId}: ${workflow.triggerCount} trigger${workflow.triggerCount === 1 ? '' : 's'}, ${workflow.moduleCount} module${workflow.moduleCount === 1 ? '' : 's'}, ${workflow.requiredSecrets.length} required secret${workflow.requiredSecrets.length === 1 ? '' : 's'}.\n`
      );
    }
    if (configuration !== undefined && environment !== undefined) {
      io.stdout(`Deployment: ${configuration.deploymentName}.\n`);
      io.stdout(`State: ${configuration.statePath}.\n`);
      io.stdout(
        `Listeners: public ${environment.ports.public}; admin ${environment.ports.admin}.\n`
      );
      io.stdout(
        `Environment: writable storage, ${Math.floor(environment.state.availableBytes / (1024 * 1024))} MiB available, secrets ready through ${secretProvider}.\n`
      );
      io.stdout('Activation: not started; no trigger or provider was opened.\n');
    } else {
      io.stdout(
        'Environment: not checked; pass --config <path> for production storage, listener, and secret preflight.\n'
      );
    }
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error, displayPath)}\n`);
    return 1;
  }
}

async function runCheckCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  let parsed: CheckArguments;
  try {
    parsed = parseCheckArguments(args);
  } catch (error) {
    if (error instanceof CliInputError && error.message === checkUsage()) {
      io.stderr(`${checkUsage()}\n`);
      return 2;
    }
    io.stderr(`${formatError(error)}\n`);
    return 2;
  }

  if (parsed.inputPaths.length === 1 && parsed.configPath === undefined) {
    return await runSingleCheckCommand(
      ['check', parsed.inputPaths[0]!, ...(parsed.json ? ['--json'] : [])],
      io
    );
  }
  return await runDeploymentCheckCommand(parsed, io, dependencies);
}

async function runTypesCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  const [, rawSourcePath, ...options] = args;
  if (
    rawSourcePath === undefined ||
    rawSourcePath.startsWith('--') ||
    (options.length !== 0 &&
      (options.length !== 2 || options[0] !== '--output' || !options[1]))
  ) {
    io.stderr(`${typesUsage()}\n`);
    return 2;
  }
  const sourcePath = resolve(rawSourcePath);
  try {
    const sourceStat = await stat(sourcePath);
    const workflows = await compileWorkflowSources(sourcePath);
    const modules = workflows.flatMap(workflow =>
      'moduleRuntime' in workflow.workflow &&
      workflow.workflow.moduleRuntime !== undefined
        ? workflow.workflow.moduleRuntime.modules
        : []
    );
    const outputPath = resolve(
      options[1] ?? editorTypesPath(sourcePath, sourceStat.isDirectory())
    );
    await mkdir(dirname(outputPath), { recursive: true });
    await writeFile(
      outputPath,
      generateWomlEditorDeclarations(modules),
      'utf8'
    );
    io.stdout(`WOML editor types written to ${outputPath}.\n`);
    io.stdout(
      `Modules: ${modules.length}; globals: services only. Pass context, attempt, and secrets explicitly.\n`
    );
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error, sourcePath)}\n`);
    return 1;
  }
}

function moduleProjectRoot(sourcePath: string): string {
  let directory = dirname(sourcePath);
  while (true) {
    if (
      existsSync(join(directory, 'woml.json')) ||
      existsSync(join(directory, 'package.json'))
    ) {
      return directory;
    }
    const parent = dirname(directory);
    if (parent === directory) return dirname(sourcePath);
    directory = parent;
  }
}

function printWaitingApproval(
  io: CliIo,
  outcome: Extract<RustApprovalRuntimeOutcome, { status: 'waiting' }>,
  url: string,
  filePath: string,
  statePath: string
): void {
  const approval = outcome.approval;
  io.stderr('\nWOML workflow is waiting for human approval.\n');
  io.stderr(`Approval: ${approval.name ?? approval.approvalId}\n`);
  if (approval.description !== undefined) {
    io.stderr(`${approval.description}\n`);
  }
  io.stderr(`Workflow: ${outcome.workflowId}\n`);
  io.stderr(`Run ID: ${outcome.runId}\n`);
  io.stderr(
    `Deadline: ${approval.expiresAt ?? 'none'} (${approval.onTimeout} on timeout)\n`
  );
  io.stderr(`Current URL expires: ${approval.credentialExpiresAt}\n`);
  io.stderr(`Approval URL: ${url}\n`);
  io.stderr(
    `Recovery: woml run ${JSON.stringify(filePath)} --state ${JSON.stringify(
      statePath
    )} --resume ${JSON.stringify(outcome.runId)}\n\n`
  );
}

async function runApprovalWorkflow(
  workflow: ReturnType<typeof compileWoml>,
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies,
  runtimeModules: readonly RustRuntimeModuleArtifact[] = []
): Promise<void> {
  await mkdir(dirname(args.statePath), { recursive: true });
  const secrets = await resolvedSecrets(
    workflow,
    dependencies.createSecretStore()
  );
  const runtimeOptions = {
    nativeCorePath: dependencies.nativeCorePath,
    resolvedSecrets: secrets,
    onProgress: (progress: ExecutionProgressV1) =>
      io.stderr(`${formatExecutionProgress(progress)}\n`),
    runtimeModules,
  };
  let outcome =
    args.resumeRunId === undefined
      ? await executeApprovalWorkflowWithRust(
          workflow,
          args.statePath,
          runtimeOptions
        )
      : await resumeApprovalWorkflowWithRust(
          workflow,
          args.statePath,
          args.resumeRunId,
          runtimeOptions
        );

  while (outcome.status === 'waiting') {
    const waiting = outcome;
    await serveApprovalAndWait({
      outcome: waiting,
      port: args.approvalPort,
      onDecision: (token, decision) =>
        resolveApprovalWithRust(args.statePath, token, decision),
      onTimeout: (runId, approvalId) =>
        settleApprovalTimeoutWithRust(args.statePath, runId, approvalId),
      onListening: url =>
        printWaitingApproval(io, waiting, url, args.filePath, args.statePath),
    });
    outcome = await resumeApprovalWorkflowWithRust(
      workflow,
      args.statePath,
      waiting.runId,
      runtimeOptions
    );
  }
  io.stdout(`${JSON.stringify(outcome.execution.result)}\n`);
}

function collectSecretReferences(
  value: ValueExpression,
  references: SecretReferenceExpression[]
): void {
  if (value.kind === 'secretReference') {
    references.push(value);
    return;
  }
  if (value.kind === 'object') {
    for (const child of Object.values(value.fields)) {
      collectSecretReferences(child, references);
    }
    return;
  }
  if (value.kind === 'array') {
    for (const child of value.items) collectSecretReferences(child, references);
  }
}

function workflowSecretReferences(
  workflow: CompiledWorkflowDefinition
): readonly SecretReferenceExpression[] {
  const references: SecretReferenceExpression[] = [];
  for (const trigger of workflow.triggers) {
    collectSecretReferences(trigger.config, references);
  }
  for (const node of workflow.graph.nodes) {
    collectSecretReferences(node.inputs, references);
    for (const name of node.scriptRuntime?.requiredSecrets ?? []) {
      references.push({ kind: 'secretReference', name });
    }
  }
  return references;
}

function workflowHasApproval(workflow: CompiledWorkflowDefinition): boolean {
  return workflow.graph.nodes.some(
    node => node.handler === 'engine.approval-wait'
  );
}

function workflowHasNotifications(
  workflow: CompiledWorkflowDefinition
): boolean {
  return workflow.graph.nodes.some(
    node =>
      node.handler === 'engine.approval-wait' &&
      node.inputs.kind === 'object' &&
      node.inputs.fields.notifications !== undefined
  );
}

function printSlackApproval(
  io: CliIo,
  outcome: Extract<RustApprovalRuntimeOutcome, { status: 'waiting' }>,
  filePath: string,
  statePath: string
): void {
  const approval = outcome.approval;
  io.stderr('\nWOML workflow is waiting for approval in Slack.\n');
  io.stderr(`Approval: ${approval.name ?? approval.approvalId}\n`);
  if (approval.description !== undefined)
    io.stderr(`${approval.description}\n`);
  io.stderr(`Workflow: ${outcome.workflowId}\n`);
  io.stderr(`Run ID: ${outcome.runId}\n`);
  io.stderr(
    `Deadline: ${approval.expiresAt ?? 'none'} (${approval.onTimeout} on timeout)\n`
  );
  io.stderr(
    'Sending Slack notifications; approve or reject from any configured channel.\n'
  );
  io.stderr(
    `Recovery: woml run ${JSON.stringify(filePath)} --state ${JSON.stringify(
      statePath
    )} --resume ${JSON.stringify(outcome.runId)}\n\n`
  );
}

function providerWaitMilliseconds(
  outcome: Extract<RustApprovalRuntimeOutcome, { status: 'waiting' }>
): number {
  if (outcome.approval.expiresAt === undefined) return 0xffff_ffff;
  const remaining = Date.parse(outcome.approval.expiresAt) - Date.now();
  return Math.max(1, Math.min(0xffff_ffff, remaining));
}

async function runNotificationWorkflow(
  workflow: CompiledWorkflowDefinition,
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies,
  runtimeModules: readonly RustRuntimeModuleArtifact[] = []
): Promise<void> {
  const secrets = await resolvedSecrets(
    workflow,
    dependencies.createSecretStore()
  );
  await mkdir(dirname(args.statePath), { recursive: true });
  const runtimeProgress = (progress: ExecutionProgressV1): void =>
    io.stderr(`${formatExecutionProgress(progress)}\n`);
  let outcome =
    args.resumeRunId === undefined
      ? await executeApprovalWorkflowWithRust(workflow, args.statePath, {
          nativeCorePath: dependencies.nativeCorePath,
          onProgress: runtimeProgress,
          resolvedSecrets: secrets,
          runtimeModules,
        })
      : await resumeApprovalWorkflowWithRust(
          workflow,
          args.statePath,
          args.resumeRunId,
          {
            nativeCorePath: dependencies.nativeCorePath,
            onProgress: runtimeProgress,
            resolvedSecrets: secrets,
            runtimeModules,
          }
        );
  while (outcome.status === 'waiting') {
    const waiting = outcome;
    printSlackApproval(io, waiting, args.filePath, args.statePath);
    const journey = await runNotificationProviderJourneyWithRust(
      args.statePath,
      waiting.runId,
      {
        notificationHostPath: dependencies.notificationHostPath,
        nativeCorePath: dependencies.nativeCorePath,
        interactionTimeoutMs: providerWaitMilliseconds(waiting),
      }
    );
    printNotificationWarnings(io, journey.diagnostics);
    outcome = await resumeApprovalWorkflowWithRust(
      workflow,
      args.statePath,
      waiting.runId,
      {
        nativeCorePath: dependencies.nativeCorePath,
        onProgress: runtimeProgress,
        resolvedSecrets: secrets,
        runtimeModules,
      }
    );
  }
  io.stdout(`${JSON.stringify(outcome.execution.result)}\n`);
}

async function resolvedStoredRunSecrets(
  requirements: StoredRunRequirementsV1,
  store: SecretStore
): Promise<Readonly<Record<string, string>>> {
  const values: Record<string, string> = {};
  for (const name of requirements.requiredSecrets) {
    const value = await store.get(name);
    if (value === undefined || value.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        `Missing required secret: ${name}.`
      );
    }
    values[name] = value;
  }
  return values;
}

async function resumeStoredRun(
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies
): Promise<void> {
  const runId = args.resumeRunId!;
  const requirements = inspectStoredRunRequirementsWithRust(
    args.statePath,
    runId,
    { nativeCorePath: dependencies.nativeCorePath }
  );
  const secrets = await resolvedStoredRunSecrets(
    requirements,
    dependencies.createSecretStore()
  );
  const runtimeOptions = {
    nativeCorePath: dependencies.nativeCorePath,
    resolvedSecrets: secrets,
    onProgress: (progress: ExecutionProgressV1) =>
      io.stderr(`${formatExecutionProgress(progress)}\n`),
  };

  io.stderr(
    `Recovering run ${runId} from its stored definition and ${requirements.moduleCount} immutable module artifact${requirements.moduleCount === 1 ? '' : 's'}.\n`
  );
  let outcome = await resumeStoredRunWithRust(
    args.statePath,
    runId,
    runtimeOptions
  );
  while (outcome.status === 'waiting') {
    const waiting = outcome;
    if (requirements.hasNotifications) {
      printSlackApproval(io, waiting, args.filePath, args.statePath);
      const journey = await runNotificationProviderJourneyWithRust(
        args.statePath,
        waiting.runId,
        {
          notificationHostPath: dependencies.notificationHostPath,
          nativeCorePath: dependencies.nativeCorePath,
          interactionTimeoutMs: providerWaitMilliseconds(waiting),
        }
      );
      printNotificationWarnings(io, journey.diagnostics);
    } else {
      await serveApprovalAndWait({
        outcome: waiting,
        port: args.approvalPort,
        onDecision: (token, decision) =>
          resolveApprovalWithRust(args.statePath, token, decision),
        onTimeout: (waitingRunId, approvalId) =>
          settleApprovalTimeoutWithRust(
            args.statePath,
            waitingRunId,
            approvalId
          ),
        onListening: url =>
          printWaitingApproval(io, waiting, url, args.filePath, args.statePath),
      });
    }
    outcome = await resumeStoredRunWithRust(
      args.statePath,
      waiting.runId,
      runtimeOptions
    );
  }
  io.stdout(`${JSON.stringify(outcome.execution.result)}\n`);
}

async function executeOneShot(
  workflow: CompiledWorkflowDefinition,
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies,
  runtimeModules: readonly RustRuntimeModuleArtifact[] = []
): Promise<void> {
  if (runtimeModules.length > 0) {
    io.stderr(
      `WOML modules ready: ${runtimeModules.map(module => `services.${module.name}`).join(', ')}.\n`
    );
  }
  const hasApproval = workflowHasApproval(workflow);
  const hasNotifications = workflowHasNotifications(workflow);
  if (
    args.resumeRunId !== undefined &&
    !hasApproval &&
    workflow.schemaVersion !== 6 &&
    workflow.schemaVersion !== 8 &&
    workflow.schemaVersion !== 9 &&
    workflow.schemaVersion !== 11 &&
    workflow.schemaVersion !== 12
  ) {
    throw new CliInputError(
      'WOML_RESUME_REQUIRES_DURABLE_WORKFLOW',
      '--resume requires a durable workflow with Human Approval or retry support.'
    );
  }
  if (hasNotifications) {
    await runNotificationWorkflow(
      workflow,
      args,
      io,
      dependencies,
      runtimeModules
    );
    return;
  }
  if (hasApproval) {
    await runApprovalWorkflow(workflow, args, io, dependencies, runtimeModules);
    return;
  }
  if (
    workflow.schemaVersion === 6 ||
    workflow.schemaVersion === 8 ||
    workflow.schemaVersion === 9 ||
    workflow.schemaVersion === 11 ||
    workflow.schemaVersion === 12
  ) {
    await mkdir(dirname(args.statePath), { recursive: true });
    const onProgress = durableRetryProgress(io, args);
    const secrets = await resolvedSecrets(
      workflow,
      dependencies.createSecretStore()
    );
    const execution =
      args.resumeRunId === undefined
        ? await executeWorkflowWithRustDurable(workflow, args.statePath, {
            nativeCorePath: dependencies.nativeCorePath,
            onProgress,
            resolvedSecrets: secrets,
            runtimeModules,
          })
        : await resumeWorkflowWithRustDurable(
            workflow,
            args.statePath,
            args.resumeRunId,
            {
              nativeCorePath: dependencies.nativeCorePath,
              onProgress,
              resolvedSecrets: secrets,
              runtimeModules,
            }
          );
    io.stdout(`${JSON.stringify(execution.result)}\n`);
    return;
  }
  const execution = await executeWorkflowWithRust(workflow, {
    nativeCorePath: dependencies.nativeCorePath,
    runtimeModules,
  });
  io.stdout(`${JSON.stringify(execution.result)}\n`);
}

function triggerIds(
  workflow: CompiledWorkflowDefinition,
  handler: string
): readonly string[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === handler)
    .map(trigger => trigger.id);
}

function selectedManualTrigger(
  workflow: CompiledWorkflowDefinition,
  requestedId: string | undefined
): string | undefined {
  const manualIds = triggerIds(workflow, 'trigger.manual');
  if (requestedId !== undefined) {
    if (!manualIds.includes(requestedId)) {
      throw new CliInputError(
        'WOML_MANUAL_TRIGGER_NOT_FOUND',
        `workflow "${workflow.workflowId}" has no manual trigger "${requestedId}".`
      );
    }
    return requestedId;
  }
  if (manualIds.length > 1) {
    throw new CliInputError(
      'WOML_MANUAL_TRIGGER_SELECTION_REQUIRED',
      `workflow "${workflow.workflowId}" declares multiple manual triggers; select one with --trigger.`
    );
  }
  return manualIds[0];
}

function objectFields(
  expression: ValueExpression | undefined
): Readonly<Record<string, ValueExpression>> | undefined {
  return expression?.kind === 'object' ? expression.fields : undefined;
}

function literalString(
  expression: ValueExpression | undefined
): string | undefined {
  return expression?.kind === 'literal' && typeof expression.value === 'string'
    ? expression.value
    : undefined;
}

interface WebhookRouteSummary {
  readonly workflowId: string;
  readonly triggerId: string;
  readonly path: string;
  readonly method: string;
  readonly authentication: string;
  readonly schema?: JsonValue;
}

interface EventRouteSummary {
  readonly eventName: string;
  readonly publicEndpoint: boolean;
  readonly schema?: JsonValue;
}

function webhookRouteSummaries(
  workflow: CompiledWorkflowDefinition
): readonly WebhookRouteSummary[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === 'trigger.webhook')
    .map(trigger => {
      const fields = objectFields(trigger.config);
      const authentication = objectFields(fields?.authentication);
      return {
        workflowId: workflow.workflowId,
        triggerId: trigger.id,
        path: literalString(fields?.path) ?? '',
        method: literalString(fields?.method) ?? 'POST',
        authentication: literalString(authentication?.kind) ?? '',
        ...(fields?.schema?.kind === 'literal'
          ? { schema: fields.schema.value }
          : {}),
      };
    });
}

function eventRouteSummaries(
  workflow: CompiledWorkflowDefinition
): readonly EventRouteSummary[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === 'trigger.event')
    .map(trigger => {
      const fields = objectFields(trigger.config);
      return {
        eventName: literalString(fields?.name) ?? '',
        publicEndpoint: fields?.secret?.kind === 'secretReference',
        ...(fields?.schema?.kind === 'literal'
          ? { schema: fields.schema.value }
          : {}),
      };
    });
}

function jsonObject(
  value: JsonValue | undefined
): Readonly<Record<string, JsonValue>> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Readonly<Record<string, JsonValue>>)
    : undefined;
}

function nonNegativeInteger(value: JsonValue | undefined): number | undefined {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
    ? value
    : undefined;
}

function webhookSchemaSample(
  schema: JsonValue | undefined,
  depth = 0
): JsonValue {
  if (depth > 8) return null;
  const fields = jsonObject(schema);
  if (fields === undefined) return {};

  if (fields.const !== undefined) return fields.const;
  if (Array.isArray(fields.enum) && fields.enum.length > 0) {
    return fields.enum[0]!;
  }
  if (Array.isArray(fields.examples) && fields.examples.length > 0) {
    return fields.examples[0]!;
  }
  if (fields.default !== undefined) return fields.default;

  const alternatives = Array.isArray(fields.oneOf)
    ? fields.oneOf
    : Array.isArray(fields.anyOf)
      ? fields.anyOf
      : undefined;
  if (alternatives !== undefined && alternatives.length > 0) {
    return webhookSchemaSample(alternatives[0], depth + 1);
  }

  const declaredType = Array.isArray(fields.type)
    ? fields.type.find(value => value !== 'null')
    : fields.type;
  const properties = jsonObject(fields.properties);
  if (declaredType === 'object' || properties !== undefined) {
    const required = Array.isArray(fields.required)
      ? fields.required.filter(
          (value): value is string => typeof value === 'string'
        )
      : [];
    return Object.fromEntries(
      required.map(name => [
        name,
        webhookSchemaSample(properties?.[name], depth + 1),
      ])
    );
  }
  if (declaredType === 'array') {
    const minimum = Math.min(nonNegativeInteger(fields.minItems) ?? 0, 3);
    return Array.from({ length: minimum }, () =>
      webhookSchemaSample(fields.items, depth + 1)
    );
  }
  if (declaredType === 'boolean') return true;
  if (declaredType === 'integer' || declaredType === 'number') {
    return typeof fields.minimum === 'number' ? fields.minimum : 0;
  }
  if (declaredType === 'null') return null;
  if (declaredType === 'string') {
    if (fields.format === 'email') return 'user@example.com';
    if (fields.format === 'uuid') return '00000000-0000-4000-8000-000000000000';
    if (fields.format === 'date-time') return '2026-01-01T00:00:00Z';
    const minimum = Math.min(nonNegativeInteger(fields.minLength) ?? 0, 32);
    return 'example'.padEnd(minimum, 'x');
  }
  return 'example';
}

function shellSingleQuoted(value: string): string {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}

export function webhookCurlExample(
  route: WebhookRouteSummary,
  host: string,
  port: number
): string {
  const url = `http://${host}:${port}${route.path}`;
  const lines = [
    `curl --request ${route.method} ${shellSingleQuoted(url)}`,
    `  --header 'Content-Type: application/json'`,
  ];
  if (route.authentication === 'bearer') {
    lines.push(`  --header 'Authorization: Bearer <token>'`);
  }
  lines.push(
    `  --data ${shellSingleQuoted(JSON.stringify(webhookSchemaSample(route.schema)))}`
  );
  return lines
    .map((line, index) => `${line}${index < lines.length - 1 ? ' \\' : ''}`)
    .join('\n');
}

export function eventCurlExample(
  route: EventRouteSummary,
  host: string,
  port: number
): string {
  const url = `http://${host}:${port}/_woml/events/${route.eventName}`;
  const lines = [
    `curl --request POST ${shellSingleQuoted(url)}`,
    `  --header 'Authorization: Bearer <control-token>'`,
    `  --header 'Event-ID: <event-id>'`,
    `  --header 'Content-Type: application/json'`,
    `  --data ${shellSingleQuoted(JSON.stringify(webhookSchemaSample(route.schema)))}`,
  ];
  return lines
    .map((line, index) => `${line}${index < lines.length - 1 ? ' \\' : ''}`)
    .join('\n');
}

async function resolvedSecrets(
  workflow: CompiledWorkflowDefinition,
  store: SecretStore
): Promise<Readonly<Record<string, string>>> {
  const references = [...workflowSecretReferences(workflow)];
  await preflightSecretReferences(references, store);
  const values: Record<string, string> = {};
  for (const name of [
    ...new Set(references.map(reference => reference.name)),
  ]) {
    const value = await store.get(name);
    if (value === undefined || value.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        `Missing required secret: ${name}.`
      );
    }
    values[name] = value;
  }
  return values;
}

export function formatSlackTriggerMessage(
  message: SlackTriggerProtocolMessage
): string | undefined {
  if (message.messageType === 'ready') {
    return 'WOML Slack trigger host is ready.';
  }
  if (message.messageType === 'connection') {
    return message.state === 'ready'
      ? `Slack workspace ${message.workspaceId} is ready for triggers.`
      : message.state === 'reconnecting'
        ? `Slack workspace ${message.workspaceId} is reconnecting${message.retryAt === undefined ? '.' : ` at ${message.retryAt}.`}`
        : message.state === 'stopped'
          ? `Slack workspace ${message.workspaceId} stopped.`
          : `Connecting Slack workspace ${message.workspaceId}.`;
  }
  if (message.messageType === 'failure') {
    return `Slack trigger failure [${message.code}]: ${message.message}`;
  }
  if (message.messageType === 'event') {
    return `Received Slack ${message.payload.type} ${message.eventId} for trigger "${message.triggerId}".`;
  }
  return undefined;
}

export function formatTriggerProgress(progress: TriggerProgressV1): string {
  if (progress.type === 'ready') {
    return `WOML runtime is ready with ${progress.registrationCount} registered trigger${progress.registrationCount === 1 ? '' : 's'}.`;
  }
  if (progress.type === 'occurrence_accepted') {
    return `${progress.duplicate ? 'Recognized duplicate' : 'Accepted'} ${progress.triggerHandler} "${progress.triggerId}" for workflow "${progress.workflowId}": ${progress.runId}.`;
  }
  if (progress.type === 'run_started') {
    return `Run ${progress.runId} started for workflow "${progress.workflowId}".`;
  }
  if (progress.type === 'run_terminal') {
    if (progress.failureCode === 'WOML_RUN_CANCELLED') {
      return `Run ${progress.runId} cancelled.`;
    }
    return progress.status === 'succeeded'
      ? `Run ${progress.runId} succeeded.`
      : `Run ${progress.runId} failed [${progress.failureCode ?? 'WOML_TRIGGER_EXECUTION_FAILED'}].`;
  }
  const target =
    progress.triggerId === undefined
      ? progress.triggerHandler
      : `${progress.triggerHandler} "${progress.triggerId}"`;
  return `Rejected ${target} [${progress.code}]: ${progress.message}`;
}

function reportTriggerProgress(
  progress: TriggerProgressV1,
  statePath: string,
  io: CliIo,
  nativeCorePath?: string
): void {
  io.stderr(`${formatTriggerProgress(progress)}\n`);
  if (progress.type !== 'run_terminal' || progress.status !== 'succeeded') {
    return;
  }
  try {
    const run = inspectRunWithRust(statePath, progress.runId, {
      nativeCorePath,
    });
    if (run.result !== undefined) {
      io.stderr(
        `Run ${progress.runId} result: ${JSON.stringify(run.result)}\n`
      );
    }
  } catch (error) {
    const code =
      error instanceof RunInspectionError
        ? error.code
        : 'WOML_RUN_INSPECTION_FAILED';
    io.stderr(
      `Run ${progress.runId} result is temporarily unavailable [${code}]. Inspect it with: woml get ${progress.runId} --state ${JSON.stringify(statePath)}\n`
    );
  }
}

export function formatScheduleProgress(progress: ScheduleProgressV1): string {
  if (progress.type === 'scheduler_error') {
    return `Schedule ${progress.triggerId} failed [${progress.code}]: ${progress.message}`;
  }
  const recovery =
    progress.reason === 'misfire_skipped'
      ? ' (missed occurrences skipped)'
      : progress.reason === 'misfire_run_once'
        ? ' (one missed occurrence recovered)'
        : '';
  return `Schedule ${progress.triggerId} (${progress.timezone}) next due at ${progress.nextScheduledAt}${recovery}.`;
}

export function formatIntervalProgress(progress: IntervalProgressV1): string {
  if (progress.type === 'scheduler_error') {
    return `Interval ${progress.triggerId} failed [${progress.code}]: ${progress.message}`;
  }
  const recovery =
    progress.reason === 'misfire_skipped'
      ? ' (missed occurrences skipped)'
      : progress.reason === 'misfire_run_once'
        ? ' (one missed occurrence recovered)'
        : '';
  return `Interval ${progress.triggerId} every ${progress.everyMs}ms next due at ${progress.nextScheduledAt}${recovery}.`;
}

export function formatWorkflowCallProgress(
  progress: WorkflowCallProgressV1
): string {
  if (progress.type === 'call_admitted') {
    return `Workflow call ${progress.parentRunId}/${progress.parentNodeId} ${progress.duplicate ? 'reattached to' : 'started'} child ${progress.childRunId} for "${progress.targetWorkflowId}".`;
  }
  if (progress.type === 'call_rejected') {
    return `Rejected workflow call ${progress.parentRunId}/${progress.parentNodeId} to "${progress.targetWorkflowId}" [${progress.code}]: ${progress.message}`;
  }
  return `Workflow call child ${progress.childRunId} for "${progress.targetWorkflowId}" ${progress.status}; parent ${progress.parentRunId}.`;
}

async function activateWorkflows(
  sources: readonly CompiledWorkflowSource[],
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies
): Promise<void> {
  if (sources.length > 1 && args.resumeRunId !== undefined) {
    throw new CliInputError(
      'WOML_RESUME_REQUIRES_SINGLE_WORKFLOW',
      '--resume requires exactly one workflow file, not multiple inputs or a directory.'
    );
  }
  if (sources.length > 1 && args.triggerId !== undefined) {
    throw new CliInputError(
      'WOML_TRIGGER_REQUIRES_SINGLE_WORKFLOW',
      '--trigger requires exactly one workflow file, not multiple inputs or a directory.'
    );
  }

  const hasProductionTrigger = sources.some(source =>
    source.workflow.triggers.some(
      trigger =>
        trigger.handler === 'trigger.webhook' ||
        trigger.handler === 'trigger.slack' ||
        trigger.handler === 'trigger.schedule' ||
        trigger.handler === 'trigger.interval' ||
        trigger.handler === 'trigger.event'
    )
  );
  const hasWorkflowCalls = sources.some(workflowCallFrontendOnlySource);
  // A loaded automation directory is one runtime unit. Production triggers
  // and Workflow Calls both require every definition to share one target and
  // capability registry before a startup manual trigger runs.
  const productionSources =
    hasProductionTrigger || hasWorkflowCalls ? sources : [];
  const oneShotSources = sources.filter(
    source => !productionSources.includes(source)
  );
  const startupManualTriggers: Record<string, string> = {};
  for (const source of sources) {
    const manual = selectedManualTrigger(source.workflow, args.triggerId);
    if (
      manual !== undefined &&
      args.resumeRunId === undefined &&
      productionSources.includes(source)
    ) {
      startupManualTriggers[source.workflow.workflowId] = manual;
    }
  }

  for (const source of oneShotSources) {
    try {
      await executeOneShot(
        source.workflow,
        { ...args, filePath: source.filePath },
        io,
        dependencies,
        source.runtimeModules
      );
    } catch (error) {
      if (
        args.command === 'run' &&
        error instanceof RustWorkflowExecutionError &&
        error.code === 'WOML_RUN_CANCELLED'
      ) {
        const runId = error.message.match(/^Workflow run "([^"]+)"/)?.[1];
        io.stderr(
          runId === undefined
            ? 'Workflow run cancelled.\n'
            : `Run ${runId} cancelled.\n`
        );
        continue;
      }
      throw error;
    }
  }

  let runtimeId: string | undefined;
  let slackHost: SlackTriggerHost | undefined;
  let slackTransport: SharedSlackTransport | undefined;
  try {
    if (productionSources.length > 0) {
      await mkdir(dirname(args.statePath), { recursive: true });
      const store = dependencies.createSecretStore();
      for (const source of productionSources) {
        if (source.runtimeModules.length > 0) {
          io.stderr(
            `WOML modules ready for ${source.workflow.workflowId}: ${source.runtimeModules.map(module => `services.${module.name}`).join(', ')}.\n`
          );
        }
      }
      const eventRoutes = productionSources
        .flatMap(source => eventRouteSummaries(source.workflow))
        .filter(route => route.publicEndpoint);
      const registrations = await Promise.all(
        productionSources.map(async source => ({
          workflow: source.workflow,
          definitionHash: source.definitionHash,
          resolvedSecrets: await resolvedSecrets(source.workflow, store),
          runtimeModules: source.runtimeModules,
        }))
      );
      const routes = productionSources.flatMap(source =>
        webhookRouteSummaries(source.workflow)
      );
      const slackRegistrations = productionSources.flatMap(source =>
        slackTriggerRegistrations(source.workflow, source.definitionHash)
      );
      const uniqueEventRoutes: EventRouteSummary[] = [];
      const seenEventNames = new Set<string>();
      for (const route of eventRoutes) {
        if (!seenEventNames.has(route.eventName)) {
          seenEventNames.add(route.eventName);
          uniqueEventRoutes.push(route);
        }
      }
      const seenRoutes = new Map<string, WebhookRouteSummary>();
      for (const route of routes) {
        const previous = seenRoutes.get(route.path);
        if (previous !== undefined) {
          throw new CliInputError(
            'WOML_WEBHOOK_ROUTE_CONFLICT',
            `webhook route "${route.path}" is claimed by triggers "${previous.triggerId}" and "${route.triggerId}".`
          );
        }
        seenRoutes.set(route.path, route);
      }
      const hasHttpEndpoint = routes.length > 0 || uniqueEventRoutes.length > 0;
      const runtime = await startWebhookRuntimeWithRust(
        registrations,
        args.statePath,
        {
          nativeCorePath: dependencies.nativeCorePath,
          host: hasHttpEndpoint ? args.host : '127.0.0.1',
          port: hasHttpEndpoint ? args.port : 0,
          startupManualTriggers,
          startSuspended: true,
          onTriggerProgress: progress =>
            reportTriggerProgress(
              progress,
              args.statePath,
              io,
              dependencies.nativeCorePath
            ),
          onScheduleProgress: progress =>
            io.stderr(`${formatScheduleProgress(progress)}\n`),
          onIntervalProgress: progress =>
            io.stderr(`${formatIntervalProgress(progress)}\n`),
          onWorkflowCallProgress: progress =>
            io.stderr(`${formatWorkflowCallProgress(progress)}\n`),
          onRuntimePolicyProgress: progress =>
            io.stderr(`${formatExecutionProgress(progress)}\n`),
        }
      );
      runtimeId = runtime.runtimeId;

      if (slackRegistrations.length > 0) {
        slackTransport =
          dependencies.createSlackTransport?.({
            log: message => io.stderr(`[woml] ${message}\n`),
            onConnectionState: status => {
              if (status.state === 'reconnecting') {
                io.stderr(
                  `Slack Socket Mode reconnecting${status.retryAt === undefined ? '.' : ` at ${status.retryAt}.`}\n`
                );
              }
            },
          }) ??
          new SharedSlackTransport({
            log: message => io.stderr(`[woml] ${message}\n`),
            onConnectionState: status => {
              if (status.state === 'reconnecting') {
                io.stderr(
                  `Slack Socket Mode reconnecting${status.retryAt === undefined ? '.' : ` at ${status.retryAt}.`}\n`
                );
              }
            },
          });
        slackHost = new SlackTriggerHost({
          registrations: slackRegistrations,
          secretStore: store,
          transport: slackTransport,
          submit: ingress =>
            submitTriggerOccurrenceWithRust(runtime.runtimeId, ingress, {
              nativeCorePath: dependencies.nativeCorePath,
            }),
          emit: message => {
            const formatted = formatSlackTriggerMessage(message);
            if (formatted !== undefined) io.stderr(`${formatted}\n`);
          },
          diagnostic: message => io.stderr(`${message}\n`),
        });
        try {
          await slackHost.start();
        } catch (error) {
          const failure = slackTriggerStartupError(error);
          throw new CliInputError(failure.code, failure.message);
        }
      }

      // Provider startup may take long enough for an editor or generator to
      // rewrite a source. Never open admission for a mixed activation.
      await assertStableSourceSnapshot(productionSources);
      await assertStableWorkflowInputSet(args.inputPaths, sources);
      await activateWebhookRuntimeWithRust(runtime.runtimeId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      io.stderr(
        `WOML deployment activation ${activationIdentity(productionSources).slice(7, 19)} ready with ${productionSources.length} workflow${productionSources.length === 1 ? '' : 's'}.\n`
      );
      if (hasHttpEndpoint) {
        io.stderr(
          `WOML workflow active at http://${runtime.host}:${runtime.port}.\n`
        );
      }
      for (const route of uniqueEventRoutes) {
        io.stderr(
          `Event ${route.eventName}: POST http://${runtime.host}:${runtime.port}/_woml/events/${route.eventName}\n`
        );
        io.stderr(
          `Try event ${route.eventName}:\n${eventCurlExample(route, runtime.host, runtime.port)}\n`
        );
      }
      for (const route of routes) {
        io.stderr(
          `Webhook ${route.triggerId}: ${route.method} http://${runtime.host}:${runtime.port}${route.path}\n`
        );
        if (route.authentication === 'none') {
          io.stderr(
            `Warning: webhook ${route.triggerId} has auth="none" and accepts unauthenticated requests.\n`
          );
        }
        io.stderr(
          `Try webhook ${route.triggerId}:\n${webhookCurlExample(route, runtime.host, runtime.port)}\n`
        );
      }

      if (args.resumeRunId !== undefined) {
        const source = productionSources[0]!;
        await executeOneShot(
          source.workflow,
          { ...args, filePath: source.filePath },
          io,
          dependencies
        );
      }
    }

    io.stderr('WOML automation is active. Press Ctrl+C to stop.\n');
    await (dependencies.waitForShutdown ?? waitForShutdownSignal)();
  } finally {
    await slackHost?.close().catch(() => {});
    await slackTransport?.close().catch(() => {});
    if (runtimeId !== undefined) {
      await stopWebhookRuntimeWithRust(runtimeId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
    }
  }
  io.stderr('WOML automation stopped.\n');
}

export interface CliDependencies {
  readonly createSecretStore: () => SecretStore;
  readonly readSecret: (name: string) => Promise<string>;
  readonly waitForShutdown?: () => Promise<void>;
  readonly notificationHostPath?: string;
  readonly nativeCorePath?: string;
  readonly createSlackTransport?: (
    options: SharedSlackTransportOptions
  ) => SharedSlackTransport;
  readonly fetch?: (input: string, init?: RequestInit) => Promise<Response>;
}

function waitForShutdownSignal(): Promise<void> {
  return new Promise(resolveShutdown => {
    const shutdown = (): void => {
      process.off('SIGINT', shutdown);
      process.off('SIGTERM', shutdown);
      resolveShutdown();
    };
    process.once('SIGINT', shutdown);
    process.once('SIGTERM', shutdown);
  });
}

const defaultDependencies: CliDependencies = {
  createSecretStore: () => createSecretStore(),
  readSecret: readSecretFromTerminal,
  waitForShutdown: waitForShutdownSignal,
  fetch: globalThis.fetch,
};

async function runEmitCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  try {
    const emit = parseEmitArguments(args);
    let dataEntry;
    try {
      dataEntry = await stat(emit.dataPath);
    } catch {
      throw new CliInputError(
        'WOML_EVENT_DATA_NOT_FOUND',
        'event data file does not exist.'
      );
    }
    if (!dataEntry.isFile()) {
      throw new CliInputError(
        'WOML_EVENT_DATA_INVALID',
        'event data path must point to a JSON file.'
      );
    }
    const source = await Bun.file(emit.dataPath).text();
    let decoded: unknown;
    try {
      decoded = JSON.parse(source);
    } catch {
      throw new CliInputError(
        'WOML_EVENT_PAYLOAD_INVALID',
        'event data must contain valid JSON.'
      );
    }
    if (
      decoded === null ||
      typeof decoded !== 'object' ||
      Array.isArray(decoded)
    ) {
      throw new CliInputError(
        'WOML_EVENT_PAYLOAD_INVALID',
        'event data must contain one top-level JSON object.'
      );
    }
    const payload = JSON.stringify(decoded);
    if (new TextEncoder().encode(payload).byteLength > 1_048_576) {
      throw new CliInputError(
        'WOML_EVENT_PAYLOAD_TOO_LARGE',
        'event payload exceeds the 1 MiB limit.'
      );
    }
    const store = dependencies.createSecretStore();
    const token = await store.get(emit.tokenSecretName);
    if (token === undefined || token.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        `Missing required secret: ${emit.tokenSecretName}. Configure it with: woml secrets set ${emit.tokenSecretName}`
      );
    }
    let response: Response;
    try {
      response = await (dependencies.fetch ?? globalThis.fetch)(
        `${emit.server}/_woml/events/${encodeURIComponent(emit.eventName)}`,
        {
          method: 'POST',
          headers: {
            Authorization: `Bearer ${token}`,
            'Event-ID': emit.eventId,
            'Content-Type': 'application/json',
          },
          body: payload,
        }
      );
    } catch {
      throw new CliInputError(
        'WOML_EVENT_UNAVAILABLE',
        'could not reach the WOML event publisher.'
      );
    }
    let responseBody: unknown;
    try {
      responseBody = JSON.parse(await response.text());
    } catch {
      throw new CliInputError(
        'WOML_EVENT_RESPONSE_INVALID',
        'the WOML event publisher returned an invalid response.'
      );
    }
    if (!response.ok) {
      const root = jsonObject(responseBody as JsonValue);
      const error = jsonObject(root?.error);
      const code =
        typeof error?.code === 'string' ? error.code : 'WOML_EVENT_UNAVAILABLE';
      const message =
        typeof error?.message === 'string'
          ? error.message
          : 'The WOML event publisher rejected the request.';
      io.stderr(`WOML event error [${code}]: ${message}\n`);
      return 1;
    }
    io.stdout(`${JSON.stringify(responseBody)}\n`);
    return 0;
  } catch (error) {
    if (error instanceof CliInputError && error.message === emitUsage()) {
      io.stderr(`${emitUsage()}\n`);
      return 2;
    }
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

async function runSecretsCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  const [, operation, name, ...extra] = args;
  const validShape =
    (operation === 'list' && name === undefined && extra.length === 0) ||
    ((operation === 'set' || operation === 'delete') &&
      name !== undefined &&
      extra.length === 0);
  if (!validShape) {
    io.stderr(`${secretsUsage()}\n`);
    return 2;
  }

  try {
    const store = dependencies.createSecretStore();
    if (operation === 'list') {
      const metadata = await store.list();
      if (metadata.length === 0) {
        io.stdout(`No secrets configured (${store.provider}).\n`);
      } else {
        for (const secret of metadata) {
          io.stdout(
            `${secret.name}\t${secret.provider}${
              secret.updatedAt === undefined ? '' : `\t${secret.updatedAt}`
            }\n`
          );
        }
      }
      return 0;
    }

    requireValidSecretName(name!);
    if (store.provider === 'environment') {
      throw new SecretStoreError(
        'WOML_SECRET_PROVIDER_READ_ONLY',
        'The environment secret provider is read-only. Configure WOML_SECRET_<NAME> in the CI secret manager.'
      );
    }
    if (operation === 'delete') {
      const deleted = await store.delete(name!);
      if (!deleted) {
        throw new SecretStoreError(
          'WOML_SECRET_NOT_FOUND',
          `Secret ${name} is not configured.`
        );
      }
      io.stdout(`Deleted secret ${name}.\n`);
      return 0;
    }

    let value = await dependencies.readSecret(name!);
    try {
      await store.set(name!, value);
    } finally {
      value = '';
    }
    io.stdout(`Stored secret ${name} in ${store.provider}.\n`);
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

export async function runCli(
  args: readonly string[],
  io: CliIo = processIo,
  dependencies: CliDependencies = defaultDependencies
): Promise<number> {
  if (args.length === 1 && (args[0] === '--version' || args[0] === '-v')) {
    io.stdout(`woml ${WOML_CLI_VERSION}\n`);
    return 0;
  }

  if (args.length === 1 && (args[0] === '--help' || args[0] === '-h')) {
    io.stdout(`${usage()}\n`);
    return 0;
  }

  if (args[0] === 'check') {
    return await runCheckCommand(args, io, dependencies);
  }

  if (args[0] === 'types') {
    return await runTypesCommand(args, io);
  }

  if (args[0] === 'secrets') {
    return await runSecretsCommand(args, io, dependencies);
  }

  if (args[0] === 'emit') {
    return await runEmitCommand(args, io, dependencies);
  }

  if (args[0] === 'list') {
    try {
      const list = parseRunListArguments(args);
      const result = listRunsWithRust(
        list.statePath,
        {
          limit: list.limit,
          workflowId: list.workflowId,
          status: list.status,
        },
        { nativeCorePath: dependencies.nativeCorePath }
      );
      io.stdout(list.json ? `${JSON.stringify(result)}\n` : formatRunList(result));
      return 0;
    } catch (error) {
      if (error instanceof CliInputError && error.message === listUsage()) {
        io.stderr(`${listUsage()}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return error instanceof CliInputError ? 2 : 1;
    }
  }

  if (args[0] === 'get') {
    try {
      const get = parseRunGetArguments(args, 'get');
      const result = inspectRunV2WithRust(get.statePath, get.runId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      io.stdout(get.json ? `${JSON.stringify(result)}\n` : formatRunInspection(result));
      return 0;
    } catch (error) {
      if (error instanceof CliInputError && error.message === getUsage()) {
        io.stderr(`${getUsage()}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return error instanceof CliInputError ? 2 : 1;
    }
  }

  if (args[0] === 'cancel') {
    try {
      const cancel = parseRunGetArguments(args, 'cancel');
      const result = cancelRunWithRust(
        cancel.statePath,
        cancel.runId,
        `cancel_${randomUUID().replaceAll('-', '')}`,
        { nativeCorePath: dependencies.nativeCorePath }
      );
      io.stdout(
        cancel.json ? `${JSON.stringify(result)}\n` : formatCancellation(result)
      );
      return result.status === 'rejected' ? 1 : 0;
    } catch (error) {
      if (error instanceof CliInputError && error.message === cancelUsage()) {
        io.stderr(`${cancelUsage()}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return error instanceof CliInputError ? 2 : 1;
    }
  }

  if (args[0] === 'runs') {
    io.stderr('The "woml runs" namespace was removed. Use "woml list", "woml get", or "woml cancel".\n');
    return 2;
  }

  let runArguments: RunArguments;
  try {
    runArguments = parseRunArguments(args);
  } catch (error) {
    const commandUsage = args[0] === 'test' ? testUsage() : runUsage();
    if (error instanceof CliInputError && error.message !== commandUsage) {
      io.stderr(`${error.message}\n`);
    }
    io.stderr(`${args.length === 0 ? usage() : commandUsage}\n`);
    return 2;
  }
  const { filePath, inputPaths } = runArguments;

  if (
    runArguments.command === 'run' &&
    runArguments.resumeRunId !== undefined
  ) {
    try {
      await resumeStoredRun(runArguments, io, dependencies);
      return 0;
    } catch (error) {
      io.stderr(`${formatError(error)}\n`);
      return 1;
    }
  }

  let sources: readonly CompiledWorkflowSource[] | undefined;
  try {
    sources = await compileWorkflowInputs(inputPaths);
    if (runArguments.command === 'test') {
      if ((await stat(filePath)).isDirectory()) {
        throw new CliInputError(
          'WOML_TEST_REQUIRES_FILE',
          'woml test requires one .woml file, not a directory.'
        );
      }
      const source = sources[0]!;
      if (
        selectedManualTrigger(source.workflow, runArguments.triggerId) ===
        undefined
      ) {
        throw new CliInputError(
          'WOML_TEST_REQUIRES_MANUAL_TRIGGER',
          'woml test requires the workflow to declare a <manual> trigger.'
        );
      }
      await executeOneShot(
        source.workflow,
        runArguments,
        io,
        dependencies,
        source.runtimeModules
      );
      return 0;
    }
    for (const inputPath of inputPaths) {
      const inputIsDirectory = (await stat(inputPath)).isDirectory();
      const inputRoot = inputIsDirectory
        ? `${resolve(inputPath)}/`
        : `${dirname(resolve(inputPath))}/`;
      await refreshEditorTypes(
        inputPath,
        sources
          .filter(source => resolve(source.filePath).startsWith(inputRoot))
          .flatMap(source => source.runtimeModules),
        io
      );
    }
    await activateWorkflows(sources, runArguments, io, dependencies);
    return 0;
  } catch (error) {
    const source = sources?.length === 1 ? sources[0] : undefined;
    io.stderr(
      `${formatError(error, source?.filePath ?? filePath, source?.document)}\n`
    );
    return 1;
  }
}

if (import.meta.main) {
  process.exitCode = await runCli(process.argv.slice(2));
}
