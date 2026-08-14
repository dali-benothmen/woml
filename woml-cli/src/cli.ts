#!/usr/bin/env bun

import { createHash, randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import {
  chmod,
  mkdir,
  open,
  readFile,
  readdir,
  stat,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { dirname, extname, join, resolve } from 'node:path';

import packageMetadata from '../package.json' with { type: 'json' };

import {
  buildWomlDefinitionPackage,
  buildWomlExecutableDefinitionPackage,
  buildWomlReusableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  assertWomlDocumentRunnable,
  generateWomlEditorDeclarations,
  generateWomlReusableCustomData,
  inspectWomlMigrationDiagnostics,
  inspectWomlDocument,
  inspectWomlModuleUsage,
  inspectWomlModuleServiceUsage,
  isWomlElement,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
  validateResolvedReusableWorkflow,
  WomlDiagnosticError,
  type CompiledWorkflowDefinition,
  type JsonValue,
  type SecretReferenceExpression,
  type SourcePosition,
  type WomlAdvisoryDiagnostic,
  type ValueExpression,
  type WomlSourceDocument,
  type WomlSourceElement,
  type WomlReusableDefinitionGraph,
  type WomlDefinitionPackageV3,
  type WomlDefinitionPackageV5,
  type WomlDefinitionPackageV7,
  type WomlDefinitionPackageV9,
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
  resolveNotificationApprovalWithRust,
  resumeApprovalWorkflowWithRust,
  resumeWorkflowWithRustDurable,
  RustWorkflowExecutionError,
  runNotificationProviderJourneyWithRust,
  cancelRunWithRust,
  inspectRunV2WithRust,
  listRunsWithRust,
  observeRuntimeWithRust,
  BackupOperationError,
  RetentionOperationError,
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
import {
  readRuntimeDescriptor,
  removeRuntimeDescriptor,
  requestRuntimeOperation,
  requestRuntimeStop,
  RuntimeControlError,
  runtimeDescriptorPath,
  runtimeLogPath,
  startRuntimeControl,
  type RuntimeControlHandle,
} from './runtime-control';
import { RuntimeObservability } from './runtime-observability';
import {
  inspectUsage,
  parseInspectArguments,
  runRuntimeInspector,
  type InspectorTerminal,
} from './runtime-inspector';
import {
  backupUsage,
  createProductionBackup,
  parseBackupArguments,
  parseRestoreArguments,
  ProductionBackupError,
  restoreProductionBackup,
  restoreUsage,
} from './production-backup';
import {
  parsePruneArguments,
  ProductionRetentionError,
  pruneUsage,
  runProductionRetention,
  startAutomaticRetention,
  type AutomaticRetentionConfiguration,
  type AutomaticRetentionHandle,
} from './production-retention';

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
  return 'Usage: woml run <workflow.woml|directory>... [--config <path>] [--host <address>] [--port <port>] [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] [--approval-port <port>] [--background|-d]';
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

function stopUsage(): string {
  return 'Usage: woml stop [--state <path>] [--json]';
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
  return `${runUsage()}\n${testUsage()}\n${checkUsage()}\n${typesUsage()}\n${inspectUsage}\n${backupUsage}\n${restoreUsage}\n${pruneUsage}\n${listUsage()}\n${getUsage()}\n${cancelUsage()}\n${stopUsage()}\n${emitUsage()}\n${secretsUsage()}`;
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
  readonly configPath?: string;
  readonly stateExplicit: boolean;
  readonly hostExplicit: boolean;
  readonly portExplicit: boolean;
  readonly adminHost: string;
  readonly adminPort: number;
  readonly shutdownTimeoutMs: number;
  readonly logDirectory: string;
  readonly logFormat: 'text' | 'json';
  readonly observabilityEnabled: boolean;
  readonly observabilityHealth: boolean;
  readonly observabilityMetrics: boolean;
  readonly retention?: AutomaticRetentionConfiguration;
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
  let configPath: string | undefined;
  let stateExplicit = false;
  let hostExplicit = false;
  let portExplicit = false;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 2) {
    const option = options[index];
    const value = options[index + 1];
    if (
      value === undefined ||
      seen.has(option) ||
      (option !== '--state' &&
        option !== '--config' &&
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
      stateExplicit = true;
    } else if (option === '--config') {
      if (command !== 'run' || value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--config requires a non-empty path for woml run.'
        );
      }
      configPath = value;
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
      hostExplicit = true;
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
      portExplicit = true;
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
    ...(resumeRunId === undefined ? {} : { resumeRunId }),
    approvalPort,
    host,
    port,
    ...(triggerId === undefined ? {} : { triggerId }),
    ...(configPath === undefined ? {} : { configPath }),
    stateExplicit,
    hostExplicit,
    portExplicit,
    adminHost: '127.0.0.1',
    adminPort: 3_001,
    shutdownTimeoutMs: 30_000,
    logDirectory: resolve('.woml/logs'),
    logFormat: 'text',
    observabilityEnabled: true,
    observabilityHealth: true,
    observabilityMetrics: true,
  };
}

async function resolveRunArgumentsConfiguration(
  args: RunArguments
): Promise<RunArguments> {
  if (args.command !== 'run') return args;
  const configuration = await resolveRuntimeConfiguration(args.configPath, {
    ...(args.stateExplicit ? { statePath: args.statePath } : {}),
    ...(args.hostExplicit ? { publicHost: args.host } : {}),
    ...(args.portExplicit ? { publicPort: args.port } : {}),
  });
  if (args.configPath !== undefined) {
    await preflightRuntimeConfiguration(configuration);
  }
  return {
    ...args,
    statePath: configuration.statePath,
    host: configuration.public.host,
    port: configuration.public.port,
    adminHost: configuration.admin.host,
    adminPort: configuration.admin.port,
    shutdownTimeoutMs: configuration.shutdownTimeoutMs,
    logDirectory: configuration.logging.directory,
    logFormat: configuration.logging.format,
    observabilityEnabled:
      configuration.observability.health || configuration.observability.metrics,
    observabilityHealth: configuration.observability.health,
    observabilityMetrics: configuration.observability.metrics,
    ...(configuration.retention === undefined
      ? {}
      : { retention: configuration.retention }),
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
        hook.failedActions === 0
          ? ''
          : `, ${hook.failedActions} failed action(s)`;
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
      (element.name === 'branch' || element.name === 'choose') &&
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
  const sourceKind = branch.name === 'choose' ? 'choice' : 'legacy branch';

  if (error.branchSite === 'selection') {
    return {
      position:
        branch.attributes.id?.valueSpan.start ?? branch.openTagSpan.start,
      subject: `${sourceKind} "${error.branchId}"`,
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
      subject: `${sourceKind} "${error.branchId}"`,
    };
  }

  if (error.branchSite === 'test') {
    return {
      position: arm.attributes.test?.valueSpan.start ?? arm.openTagSpan.start,
      subject: `<when test> in ${sourceKind} "${error.branchId}"`,
    };
  }

  const result = childElements(arm).find(element => element.name === 'result');
  return {
    position:
      result?.attributes.value?.valueSpan.start ??
      result?.openTagSpan.start ??
      arm.openTagSpan.start,
    subject: `<result value> in ${sourceKind} "${error.branchId}"`,
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

  if (error instanceof RuntimeControlError) {
    return `WOML runtime error [${error.code}]: ${error.message}`;
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

  if (
    error instanceof ProductionBackupError ||
    error instanceof BackupOperationError
  ) {
    const operation = error.code.startsWith('WOML_RESTORE_')
      ? 'restore'
      : 'backup';
    return `WOML ${operation} error [${error.code}]: ${error.message}`;
  }

  if (
    error instanceof ProductionRetentionError ||
    error instanceof RetentionOperationError
  ) {
    return `WOML retention error [${error.code}]: ${error.message}`;
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

function formatAdvisoryDiagnostic(diagnostic: WomlAdvisoryDiagnostic): string {
  const location = `${diagnostic.file}:${diagnostic.location.start.line}:${diagnostic.location.start.column}`;
  const hint =
    diagnostic.hint === undefined ? '' : `\nHint: ${diagnostic.hint}`;
  return `Warning [${diagnostic.code}] at ${location}: ${diagnostic.message}${hint}`;
}

function printMigrationDiagnostics(
  io: CliIo,
  diagnostics: readonly WomlAdvisoryDiagnostic[]
): void {
  for (const diagnostic of diagnostics) {
    io.stderr(`${formatAdvisoryDiagnostic(diagnostic)}\n`);
  }
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
  readonly activationInputPaths: readonly string[];
  readonly document: WomlSourceDocument;
  readonly workflow: CompiledWorkflowDefinition;
  readonly definitionHash: string;
  readonly runtimeModules: readonly RustRuntimeModuleArtifact[];
  readonly sourceSnapshot: readonly SourceSnapshotEntry[];
  readonly migrationDiagnostics: readonly WomlAdvisoryDiagnostic[];
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
  expectedPaths: readonly string[]
): Promise<void> {
  const current = new Set<string>();
  for (const inputPath of inputPaths) {
    for (const path of await workflowFilePaths(inputPath))
      current.add(resolve(path));
  }
  const expected = [...new Set(expectedPaths.map(path => resolve(path)))].sort();
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
  if (
    workflow.schemaVersion === 11 ||
    workflow.schemaVersion === 12 ||
    workflow.schemaVersion === 13 ||
    workflow.schemaVersion === 14
  ) {
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
  if (
    source.workflow.schemaVersion === 14 &&
    (source.workflow.reusableDefinitions?.length ?? 0) > 0
  ) {
    return (
      source.workflow.triggers.length === 0 ||
      source.workflow.graph.nodes.some(node =>
        JSON.stringify(node.inputs).includes('services.workflows')
      ) ||
      source.runtimeModules.some(
        module =>
          module.name.startsWith('__woml_reusable__') &&
          /\bservices\.workflows\.(?:call|start)\s*\(/.test(module.bundle)
      )
    );
  }
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
    | WomlDefinitionPackageV9
): readonly RustRuntimeModuleArtifact[] {
  const modules = definitionPackage.modules.map(module => {
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
  if (definitionPackage.schemaVersion !== 9) return modules;
  const seen = new Set<string>();
  const reusableArtifacts = (definitionPackage.workflow.model.reusableDefinitions ?? [])
    .filter(
      definition =>
        !seen.has(definition.scriptArtifactId) &&
        seen.add(definition.scriptArtifactId)
    )
    .map(definition => {
      const artifact = definitionPackage.artifacts.find(
        item => item.path === `definitions/${definition.scriptArtifactId}.js`
      );
      if (artifact?.kind !== 'module-bundle') {
        throw new Error(
          `Reusable definition artifact "${definition.scriptArtifactId}" is unavailable.`
        );
      }
      return {
        name: `${definition.kind === 'notification-provider' ? '__woml_provider__' : '__woml_reusable__'}${definition.scriptArtifactId}`,
        bundleDigest: artifact.digest,
        sourceMapDigest: sourceDigest(''),
        exports: [] as readonly string[],
        bundle: artifact.content,
      sourceMap: '',
      };
    });
  const lifecycleArtifacts = (definitionPackage.workflow.model.reusableDefinitions ?? [])
    .flatMap(definition => {
      const ownerId = definition.kind === 'step'
        ? definition.invocationId
        : definition.providerId;
      return [
        ...((definition.lifecycle?.onSuccess ?? []).map(actionId => ({ actionId, hook: 'on-success' }))),
        ...((definition.lifecycle?.onError ?? []).map(actionId => ({ actionId, hook: 'on-error' }))),
        ...((definition.lifecycle?.onComplete ?? []).map(actionId => ({ actionId, hook: 'on-complete' }))),
      ].map(({ actionId, hook }, indexWithinHook) => {
        const actionIndex = Number(actionId.slice(actionId.lastIndexOf(':') + 1));
        const artifact = definitionPackage.artifacts.find(
          item => item.path === `definitions/${ownerId}.${hook}.${Number.isInteger(actionIndex) ? actionIndex : indexWithinHook}.js`
        );
        if (artifact?.kind !== 'module-bundle') {
          throw new Error(`Reusable lifecycle artifact "${actionId}" is unavailable.`);
        }
        return {
          name: `__woml_lifecycle__${sourceDigest(actionId).slice(7)}`,
          bundleDigest: artifact.digest,
          sourceMapDigest: sourceDigest(''),
          exports: [] as readonly string[],
          bundle: artifact.content,
          sourceMap: '',
        };
      });
    });
  return [...modules, ...reusableArtifacts, ...lifecycleArtifacts].sort((left, right) =>
    left.name.localeCompare(right.name)
  );
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
  inputPath: string,
  skipReusableDefinitions = false,
  io?: CliIo
): Promise<readonly CompiledWorkflowSource[]> {
  const compiled: CompiledWorkflowSource[] = [];
  for (const filePath of await workflowFilePaths(inputPath)) {
    const source = await readWorkflow(filePath);
    const document = parseWoml(source, { file: filePath });
    const projectRoot = moduleProjectRoot(filePath);
    const reusableGraph = resolveWomlReusableDefinitionGraph(document, {
      sourcePath: filePath,
      projectRoot,
    });
    if (reusableGraph.root.kind !== 'workflow') {
      await validateReusableModuleEntrypoints(reusableGraph, projectRoot);
      if (io !== undefined) {
        await refreshReusableEditorData(
          filePath,
          generateWomlReusableCustomData(reusableGraph),
          io
        );
      }
      if (skipReusableDefinitions) continue;
      assertWomlDocumentRunnable(document);
    }
    if (reusableGraph.definitions.length > 0) {
      if (io !== undefined) {
        await validateReusableModuleEntrypoints(reusableGraph, projectRoot);
        validateResolvedReusableWorkflow(document, reusableGraph);
        await refreshReusableEditorData(
          filePath,
          generateWomlReusableCustomData(reusableGraph),
          io
        );
      }
    }
    const inspected =
      reusableGraph.definitions.length === 0
        ? buildWomlDefinitionPackage(document, {
            sourcePath: filePath,
            projectRoot,
          })
        : undefined;
    if (inspected !== undefined && inspected.modules.length > 0) {
      inspectWomlModuleServiceUsage(document, {
        sourcePath: filePath,
        projectRoot,
      });
    }
    const reusablePackage =
      reusableGraph.definitions.length > 0
        ? await buildWomlReusableDefinitionPackage(document, reusableGraph, {
            sourcePath: filePath,
            projectRoot,
          })
        : undefined;
    const executablePackage =
      reusablePackage ?? (inspected!.modules.length > 0
        ? await buildWomlExecutableDefinitionPackage(document, {
            sourcePath: filePath,
            projectRoot,
          })
        : undefined);
    // Definition Package v7 is the frozen Model v12 compilation identity. RP6
    // promotes its exact reviewed artifacts directly at activation time rather
    // than mutating that immutable package into a second public shape.
    const runtimePackage =
      reusablePackage !== undefined
        ? reusablePackage
        : executablePackage?.schemaVersion === 7
        ? executablePackage
        : inspected!.modules.length > 0
          ? await buildWomlRuntimeDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            })
          : undefined;
    const packageSources = runtimePackage?.sources ?? inspected!.sources;
    if (
      runtimePackage !== undefined &&
      inspected !== undefined &&
      !samePackageSources(inspected.sources, runtimePackage.sources)
    ) {
      throw new CliInputError(
        'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
        `workflow or module source changed while "${filePath}" was being compiled; run the command again.`
      );
    }
    const frontendWorkflow = runtimePackage?.workflow.model ?? compileWoml(document);
    const workflow = promoteForLifecycleAuthority(frontendWorkflow);
    const definitionHash = compiledDefinitionHash(workflow);
    compiled.push({
      filePath,
      activationInputPaths: [resolve(filePath)],
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
      migrationDiagnostics:
        reusableGraph.definitions.length === 0
          ? inspectWomlMigrationDiagnostics(document)
          : [],
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
  inputPaths: readonly string[],
  io?: CliIo
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
    const explicitFile = inputSnapshots.some(
      snapshot => !snapshot.directory && snapshot.files.includes(filePath)
    );
    for (const source of await compileWorkflowSources(
      filePath,
      !explicitFile,
      io
    )) {
      compiled.push(source);
    }
  }
  if (compiled.length === 0) {
    throw new CliInputError(
      'WOML_RUNNABLE_WORKFLOW_REQUIRED',
      'input contains reusable definitions but no runnable workflow document.'
    );
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
  const activationInputPaths = inputSnapshots
    .flatMap(snapshot => snapshot.files)
    .map(path => resolve(path))
    .filter((path, index, paths) => paths.indexOf(path) === index)
    .sort((left, right) => left.localeCompare(right));
  return compiled.map(source => ({ ...source, activationInputPaths }));
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

function deploymentIdentity(statePath: string): string {
  return `deployment_${createHash('sha256')
    .update(resolve(statePath))
    .digest('hex')
    .slice(0, 24)}`;
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

async function refreshReusableEditorData(
  inputPath: string,
  content: string,
  io: CliIo
): Promise<void> {
  const outputPath = join(dirname(inputPath), 'woml-custom-data.json');
  try {
    await writeFile(outputPath, content, 'utf8');
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    io.stderr(
      `Warning [WOML_EDITOR_DATA_WRITE_FAILED]: Could not refresh ${outputPath}: ${reason}\nWorkflow validation can continue.\n`
    );
  }
}

async function validateReusableModuleEntrypoints(
  graph: WomlReusableDefinitionGraph,
  projectRoot: string
): Promise<void> {
  for (const source of graph.sources) {
    if (source.kind === 'script-module') continue;
    const sourcePath = resolve(projectRoot, source.path);
    const sourceText = await readFile(sourcePath, 'utf8');
    const inspection = inspectWomlDocument(
      parseWoml(sourceText, { file: sourcePath })
    );
    const scriptImports = inspection.imports.filter(
      declaration => declaration.kind === 'script-module'
    );
    if (scriptImports.length === 0) continue;
    const syntheticSource = `<woml><imports>${scriptImports
      .map(
        declaration =>
          `<module name="${declaration.name}" from="${declaration.from}" />`
      )
      .join(
        ''
      )}</imports><workflow id="reusable-module-check"><steps><step id="check"><script>return true;</script></step></steps></workflow></woml>`;
    buildWomlDefinitionPackage(
      parseWoml(syntheticSource, { file: sourcePath }),
      { sourcePath, projectRoot }
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
    const projectRoot = moduleProjectRoot(filePath);
    const reusableGraph = resolveWomlReusableDefinitionGraph(document, {
      sourcePath: filePath,
      projectRoot,
    });
    if (
      reusableGraph.root.kind !== 'workflow' ||
      reusableGraph.definitions.length > 0
    ) {
      await validateReusableModuleEntrypoints(reusableGraph, projectRoot);
      validateResolvedReusableWorkflow(document, reusableGraph);
      await refreshReusableEditorData(
        filePath,
        generateWomlReusableCustomData(reusableGraph),
        io
      );
      const reusablePackage =
        reusableGraph.root.kind === 'workflow' &&
        reusableGraph.definitions.length > 0
          ? await buildWomlReusableDefinitionPackage(
              document,
              reusableGraph,
              { sourcePath: filePath, projectRoot }
            )
          : undefined;
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(reusablePackage ?? reusableGraph, null, 2)}\n`
        );
        return 0;
      }
      if (reusableGraph.root.kind !== 'workflow') {
        io.stdout(
          `WOML check passed for ${reusableGraph.root.kind === 'reusable-step' ? 'reusable step' : 'notification provider'} definition "${filePath}".\n`
        );
      } else {
        io.stdout(`WOML check passed for workflow source "${filePath}".\n`);
      }
      io.stdout(`Reusable definition graph: ${reusableGraph.rootHash}\n`);
      io.stdout(
        `Definitions: ${reusableGraph.definitions.length}; pinned sources: ${reusableGraph.sources.length}.\n`
      );
      for (const definition of reusableGraph.definitions) {
        io.stdout(
          `<${definition.alias}> -> ${definition.sourcePath} (${definition.kind}).\n`
        );
      }
      io.stdout(
        reusablePackage !== undefined
          ? `Compiled Model v14 package: ${reusablePackage.rootHash}\nExecution: custom steps and notification providers are runnable.\n`
          : reusableGraph.root.kind === 'workflow'
            ? 'Execution: reusable provider source is validated; custom notification providers begin in SCP5.\n'
          : 'Execution: reusable definitions are imported by workflows and are not independently runnable.\n'
      );
      return 0;
    }
    const migrationDiagnostics = inspectWomlMigrationDiagnostics(document);
    const inspectionPackage = buildWomlDefinitionPackage(document, {
      sourcePath: filePath,
      projectRoot,
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
              projectRoot,
            })
          : await buildWomlRuntimeDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
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
            projectRoot,
          });
    await refreshEditorTypes(filePath, definitionPackage.modules, io);
    printMigrationDiagnostics(io, migrationDiagnostics);
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
        compiledWorkflow.schemaVersion === 12 ||
        compiledWorkflow.schemaVersion === 13 ||
        compiledWorkflow.schemaVersion === 14) &&
      compiledWorkflow.lifecycle !== undefined;
    const hasRuntimePolicy =
      compiledWorkflow.schemaVersion === 12 ||
      compiledWorkflow.schemaVersion === 13 ||
      compiledWorkflow.schemaVersion === 14;
    const hasFork =
      (compiledWorkflow.schemaVersion === 13 ||
        compiledWorkflow.schemaVersion === 14) &&
      compiledWorkflow.graph.forks.length > 0;
    const hasSwitch =
      compiledWorkflow.schemaVersion === 14 &&
      compiledWorkflow.graph.choices.some(
        choice => choice.stringSelector !== undefined
      );
    const workflowCallsFrontendOnly =
      compiledWorkflow.triggers.length === 0 ||
      usage.referencedServices.includes('workflows');
    io.stdout(
      hasSwitch
        ? 'Execution: Model v14 exact-string switch routing and merged results are executable through the durable Rust runtime.\n'
        : hasFork
          ? 'Execution: Model v13 all, selected, and non-blocking fork joins are executable through the durable Rust runtime.\n'
          : hasRuntimePolicy && definitionPackage.modules.length > 0
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
      if (json)
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      json = true;
      continue;
    }
    if (argument === '--config') {
      const value = args[index + 1];
      if (
        configPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
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
  const claimed = new Map<
    string,
    { readonly workflowId: string; readonly triggerId: string }
  >();
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
    const sources = await compileWorkflowInputs(parsed.inputPaths, io);
    printMigrationDiagnostics(
      io,
      sources.flatMap(source => source.migrationDiagnostics)
    );
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
        sources.flatMap(source => [
          ...workflowSecretReferences(source.workflow),
        ]),
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
      io.stdout(
        'Activation: not started; no trigger or provider was opened.\n'
      );
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

  const onlyInput = parsed.inputPaths[0];
  const onlyInputIsFile =
    onlyInput !== undefined &&
    (await stat(onlyInput).catch(() => undefined))?.isFile() === true;
  if (
    parsed.inputPaths.length === 1 &&
    parsed.configPath === undefined &&
    onlyInputIsFile
  ) {
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
  for (const definition of
    workflow.schemaVersion === 14
      ? (workflow.reusableDefinitions ?? [])
      : []) {
    for (const prop of definition.props) {
      if (prop.expression.kind === 'secret') {
        references.push({ kind: 'secretReference', name: prop.expression.name });
      }
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

function printNotificationApproval(
  io: CliIo,
  outcome: Extract<RustApprovalRuntimeOutcome, { status: 'waiting' }>,
  filePath: string,
  statePath: string
): void {
  const approval = outcome.approval;
  io.stderr('\nWOML workflow is waiting for approval.\n');
  io.stderr(`Approval: ${approval.name ?? approval.approvalId}\n`);
  if (approval.description !== undefined)
    io.stderr(`${approval.description}\n`);
  io.stderr(`Workflow: ${outcome.workflowId}\n`);
  io.stderr(`Run ID: ${outcome.runId}\n`);
  io.stderr(
    `Deadline: ${approval.expiresAt ?? 'none'} (${approval.onTimeout} on timeout)\n`
  );
  io.stderr(
    'Sending notifications; approve or reject from any configured provider.\n'
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
    printNotificationApproval(io, waiting, args.filePath, args.statePath);
    const hasCustomProvider =
      workflow.schemaVersion === 14 &&
      (workflow.reusableDefinitions ?? []).some(
        definition => definition.kind === 'notification-provider'
      );
    let journey;
    if (hasCustomProvider) {
      const controller = new AbortController();
      let announceOrigin!: (origin: string) => void;
      const originReady = new Promise<string>(resolveOrigin => {
        announceOrigin = resolveOrigin;
      });
      const server = serveApprovalAndWait({
        outcome: waiting,
        port: args.approvalPort,
        signal: controller.signal,
        onDecision: (token, decision) =>
          resolveApprovalWithRust(args.statePath, token, decision),
        onNotificationDecision: (capability, decision) =>
          resolveNotificationApprovalWithRust(
            args.statePath,
            capability,
            decision,
            { nativeCorePath: dependencies.nativeCorePath }
          ),
        onTimeout: (waitingRunId, approvalId) =>
          settleApprovalTimeoutWithRust(
            args.statePath,
            waitingRunId,
            approvalId
          ),
        onListening: url => announceOrigin(new URL(url).origin),
      });
      const approvalBaseUrl = await originReady;
      try {
        journey = await runNotificationProviderJourneyWithRust(
          args.statePath,
          waiting.runId,
          {
            notificationHostPath: dependencies.notificationHostPath,
            customNotificationHostPath:
              dependencies.customNotificationHostPath,
            nativeCorePath: dependencies.nativeCorePath,
            approvalBaseUrl,
            resolvedSecrets: secrets,
            interactionTimeoutMs: providerWaitMilliseconds(waiting),
          }
        );
      } finally {
        controller.abort();
        await server;
      }
    } else {
      journey = await runNotificationProviderJourneyWithRust(
        args.statePath,
        waiting.runId,
        {
          notificationHostPath: dependencies.notificationHostPath,
          nativeCorePath: dependencies.nativeCorePath,
          interactionTimeoutMs: providerWaitMilliseconds(waiting),
        }
      );
    }
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
      printNotificationApproval(io, waiting, args.filePath, args.statePath);
      const controller = new AbortController();
      let announceOrigin!: (origin: string) => void;
      const originReady = new Promise<string>(resolveOrigin => {
        announceOrigin = resolveOrigin;
      });
      const server = serveApprovalAndWait({
        outcome: waiting,
        port: args.approvalPort,
        signal: controller.signal,
        onDecision: (token, decision) =>
          resolveApprovalWithRust(args.statePath, token, decision),
        onNotificationDecision: (capability, decision) =>
          resolveNotificationApprovalWithRust(
            args.statePath,
            capability,
            decision,
            { nativeCorePath: dependencies.nativeCorePath }
          ),
        onTimeout: (waitingRunId, approvalId) =>
          settleApprovalTimeoutWithRust(
            args.statePath,
            waitingRunId,
            approvalId
          ),
        onListening: url => announceOrigin(new URL(url).origin),
      });
      const approvalBaseUrl = await originReady;
      let journey;
      try {
        journey = await runNotificationProviderJourneyWithRust(
          args.statePath,
          waiting.runId,
          {
            notificationHostPath: dependencies.notificationHostPath,
            customNotificationHostPath:
              dependencies.customNotificationHostPath,
            nativeCorePath: dependencies.nativeCorePath,
            approvalBaseUrl,
            resolvedSecrets: secrets,
            interactionTimeoutMs: providerWaitMilliseconds(waiting),
          }
        );
      } finally {
        controller.abort();
        await server;
      }
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
    const publicModules = runtimeModules.filter(
      module => !module.name.startsWith('__woml_')
    );
    if (publicModules.length > 0) {
      io.stderr(
        `WOML modules ready: ${publicModules.map(module => `services.${module.name}`).join(', ')}.\n`
      );
    }
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
    workflow.schemaVersion !== 12 &&
    workflow.schemaVersion !== 13 &&
    workflow.schemaVersion !== 14
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
    workflow.schemaVersion === 12 ||
    workflow.schemaVersion === 13 ||
    workflow.schemaVersion === 14
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

async function durableStoreSize(path: string): Promise<number> {
  let total = 0;
  for (const candidate of [path, `${path}-wal`, `${path}-shm`]) {
    try {
      const entry = await stat(candidate);
      if (entry.isFile()) total += entry.size;
    } catch (error) {
      if (
        !(error instanceof Error && 'code' in error && error.code === 'ENOENT')
      )
        throw error;
    }
  }
  return total;
}

function observedTriggerType(
  handler: string
):
  | 'manual'
  | 'webhook'
  | 'slack'
  | 'schedule'
  | 'interval'
  | 'event'
  | undefined {
  const type = handler.startsWith('trigger.') ? handler.slice(8) : handler;
  return [
    'manual',
    'webhook',
    'slack',
    'schedule',
    'interval',
    'event',
  ].includes(type)
    ? (type as
        | 'manual'
        | 'webhook'
        | 'slack'
        | 'schedule'
        | 'interval'
        | 'event')
    : undefined;
}

async function activateWorkflows(
  sources: readonly CompiledWorkflowSource[],
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies
): Promise<void> {
  const activationInputPaths = sources[0]?.activationInputPaths ?? [];
  if (
    sources.some(
      source =>
        source.activationInputPaths.length !== activationInputPaths.length ||
        source.activationInputPaths.some(
          (path, index) => path !== activationInputPaths[index]
        )
    )
  ) {
    throw new CliInputError(
      'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
      'compiled workflows do not share one atomic input snapshot.'
    );
  }
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
  let runtimeControl: RuntimeControlHandle | undefined;
  let observability: RuntimeObservability | undefined;
  let automaticRetention: AutomaticRetentionHandle | undefined;
  const pendingObservabilityProgress: unknown[] = [];
  const observe = (progress: unknown): void => {
    if (observability === undefined) {
      if (pendingObservabilityProgress.length < 1024)
        pendingObservabilityProgress.push(progress);
      return;
    }
    observability.recordProgress(progress);
  };
  let descriptorPath: string | undefined;
  let resolveRuntimeUnavailable!: () => void;
  const runtimeUnavailable = new Promise<void>(resolveUnavailable => {
    resolveRuntimeUnavailable = resolveUnavailable;
  });
  try {
    if (productionSources.length > 0) {
      await mkdir(dirname(args.statePath), { recursive: true });
      const store = dependencies.createSecretStore();
      for (const source of productionSources) {
        if (source.runtimeModules.length > 0) {
          const publicModules = source.runtimeModules.filter(
            module => !module.name.startsWith('__woml_')
          );
          if (publicModules.length > 0) {
            io.stderr(
              `WOML modules ready for ${source.workflow.workflowId}: ${publicModules.map(module => `services.${module.name}`).join(', ')}.\n`
            );
          }
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
      const currentActivationId = activationIdentity(productionSources);
      const currentDeploymentId = deploymentIdentity(args.statePath);
      const runtime = await startWebhookRuntimeWithRust(
        registrations,
        args.statePath,
        {
          nativeCorePath: dependencies.nativeCorePath,
          host: hasHttpEndpoint ? args.host : '127.0.0.1',
          port: hasHttpEndpoint ? args.port : 0,
          startupManualTriggers,
          deploymentId: currentDeploymentId,
          activationId: currentActivationId,
          shutdownTimeoutMs: args.shutdownTimeoutMs,
          startSuspended: true,
          onTriggerProgress: progress => {
            observe(progress);
            if (args.logFormat === 'text') {
              reportTriggerProgress(
                progress,
                args.statePath,
                io,
                dependencies.nativeCorePath
              );
            }
          },
          onScheduleProgress: progress => {
            observe(progress);
            if (args.logFormat === 'text')
              io.stderr(`${formatScheduleProgress(progress)}\n`);
          },
          onIntervalProgress: progress => {
            observe(progress);
            if (args.logFormat === 'text')
              io.stderr(`${formatIntervalProgress(progress)}\n`);
          },
          onWorkflowCallProgress: progress => {
            observe(progress);
            if (args.logFormat === 'text')
              io.stderr(`${formatWorkflowCallProgress(progress)}\n`);
          },
          onRuntimePolicyProgress: progress => {
            observe(progress);
            if (args.logFormat === 'text')
              io.stderr(`${formatExecutionProgress(progress)}\n`);
          },
          onRuntimeLifecycle: progress => {
            observe(progress);
            if (
              progress.lifecycle === 'degraded' ||
              progress.lifecycle === 'failed'
            ) {
              io.stderr(
                `WOML runtime error [WOML_RUNTIME_OWNERSHIP_LOST]: Runtime ${progress.runtimeInstanceId} lost durable deployment ownership and is shutting down.\n`
              );
              resolveRuntimeUnavailable();
            }
          },
        }
      );
      runtimeId = runtime.runtimeId;
      if (args.observabilityEnabled) {
        const componentRecords = [
          {
            name: 'sqlite',
            kind: 'store' as const,
            status: 'ready' as const,
          },
          {
            name: 'rust-trigger-host',
            kind: 'trigger' as const,
            status: 'ready' as const,
          },
          {
            name: 'script-host',
            kind: 'worker' as const,
            status: 'ready' as const,
          },
          ...(slackRegistrations.length === 0
            ? []
            : [
                {
                  name: 'slack',
                  kind: 'provider' as const,
                  status: 'unready' as const,
                },
              ]),
          ...(args.retention?.enabled === true
            ? [
                {
                  name: 'retention',
                  kind: 'retention' as const,
                  status: 'ready' as const,
                },
              ]
            : []),
        ];
        observability = new RuntimeObservability({
          runtimeInstanceId: runtime.runtimeId,
          deploymentId: currentDeploymentId,
          workflows: productionSources.map(source => ({
            workflowId: source.workflow.workflowId,
            definitionHash: source.definitionHash,
            triggerTypes: source.workflow.triggers
              .map(trigger => observedTriggerType(trigger.handler))
              .filter(
                (type): type is NonNullable<typeof type> => type !== undefined
              ),
          })),
          listRuns: () =>
            listRunsWithRust(
              args.statePath,
              { limit: 200 },
              { nativeCorePath: dependencies.nativeCorePath }
            ),
          observeDurable: () =>
            observeRuntimeWithRust(args.statePath, {
              nativeCorePath: dependencies.nativeCorePath,
            }),
          storeSize: () => durableStoreSize(args.statePath),
          logFormat: args.logFormat,
          emitLog: io.stderr,
          components: componentRecords,
        });
        observability.setLifecycle('recovering');
        for (const progress of pendingObservabilityProgress)
          observability.recordProgress(progress);
        pendingObservabilityProgress.length = 0;
      }
      runtimeControl = await startRuntimeControl({
        runtimeInstanceId: runtime.runtimeId,
        deploymentId: currentDeploymentId,
        host: args.adminHost,
        port: args.adminPort,
        observability,
        healthEnabled: args.observabilityHealth,
        metricsEnabled: args.observabilityMetrics,
        operations: {
          listRuns: () => {
            listRunsWithRust(
              args.statePath,
              { limit: 1 },
              {
                nativeCorePath: dependencies.nativeCorePath,
              }
            );
          },
          getRun: runId => {
            inspectRunV2WithRust(args.statePath, runId, {
              nativeCorePath: dependencies.nativeCorePath,
            });
          },
          cancelRun: (runId, commandId) => {
            const result = cancelRunWithRust(args.statePath, runId, commandId, {
              nativeCorePath: dependencies.nativeCorePath,
            });
            return result.code;
          },
        },
      });

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
          observability?.setComponent('slack', 'provider', 'ready');
        } catch (error) {
          observability?.setComponent(
            'slack',
            'provider',
            'unready',
            'WOML_SLACK_TRIGGER_UNAVAILABLE'
          );
          const failure = slackTriggerStartupError(error);
          throw new CliInputError(failure.code, failure.message);
        }
      }

      // Provider startup may take long enough for an editor or generator to
      // rewrite a source. Never open admission for a mixed activation.
      await assertStableSourceSnapshot(productionSources);
      await assertStableWorkflowInputSet(args.inputPaths, activationInputPaths);
      await activateWebhookRuntimeWithRust(runtime.runtimeId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      observability?.setLifecycle('ready');
      observability?.log(
        'info',
        'WOML_RUNTIME_READY',
        `WOML runtime is ready with ${productionSources.length} workflow${productionSources.length === 1 ? '' : 's'}.`
      );
      io.stderr(
        `WOML deployment activation ${currentActivationId.slice(7, 19)} ready with ${productionSources.length} workflow${productionSources.length === 1 ? '' : 's'}.\n`
      );
      automaticRetention = startAutomaticRetention({
        statePath: args.statePath,
        configuration: args.retention,
        nativeCorePath: dependencies.nativeCorePath,
        ownerId: `retention_runtime_${runtime.runtimeId}`,
        onResult: execution => {
          observability?.setComponent('retention', 'retention', 'ready');
          observability?.recordMaintenance('retention', 'completed');
          observability?.log(
            'info',
            'WOML_RETENTION_COMPLETED',
            `Retention removed ${execution.result.deletedRuns} eligible run${execution.result.deletedRuns === 1 ? '' : 's'} in ${execution.batches} batch${execution.batches === 1 ? '' : 'es'}.`
          );
        },
        onError: error => {
          const code =
            error instanceof RetentionOperationError
              ? error.code
              : 'WOML_RETENTION_FAILED';
          observability?.setComponent(
            'retention',
            'retention',
            'degraded',
            code
          );
          observability?.recordMaintenance('retention', 'failed', code);
          observability?.alert(code, formatError(error));
        },
      });
      if (automaticRetention.nextRunAt !== undefined) {
        io.stderr(
          `Automatic retention is enabled; next maintenance at ${automaticRetention.nextRunAt}.\n`
        );
      }
      descriptorPath = runtimeDescriptorPath(args.statePath);
      await runtimeControl.publishDescriptor(descriptorPath);
      io.stderr(
        `Inspect live runtime: woml inspect --state ${JSON.stringify(args.statePath)}\n`
      );
      await dependencies.onRuntimeReady?.({
        runtimeInstanceId: runtime.runtimeId,
        descriptorPath,
        workflowCount: productionSources.length,
      });
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
    await Promise.race([
      (dependencies.waitForShutdown ?? waitForShutdownSignal)(),
      runtimeUnavailable,
      ...(runtimeControl === undefined ? [] : [runtimeControl.stopRequested]),
    ]);
  } finally {
    automaticRetention?.close();
    if (runtimeId !== undefined) {
      observability?.setLifecycle('draining');
      io.stderr('WOML runtime is draining; new trigger admission is closed.\n');
    }
    await runtimeControl?.close().catch(() => {});
    const runtimeStop =
      runtimeId === undefined
        ? Promise.resolve()
        : stopWebhookRuntimeWithRust(runtimeId, {
            nativeCorePath: dependencies.nativeCorePath,
          });
    await slackHost?.close().catch(() => {});
    await slackTransport?.close().catch(() => {});
    await runtimeStop;
    observability?.setLifecycle('stopped');
    if (descriptorPath !== undefined && runtimeControl !== undefined) {
      await removeRuntimeDescriptor(
        descriptorPath,
        runtimeControl.descriptor.runtimeInstanceId
      ).catch(() => {});
    }
  }
  io.stderr('WOML automation stopped.\n');
}

export interface CliDependencies {
  readonly createSecretStore: () => SecretStore;
  readonly readSecret: (name: string) => Promise<string>;
  readonly waitForShutdown?: () => Promise<void>;
  readonly notificationHostPath?: string;
  readonly customNotificationHostPath?: string;
  readonly nativeCorePath?: string;
  readonly createSlackTransport?: (
    options: SharedSlackTransportOptions
  ) => SharedSlackTransport;
  readonly fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  readonly inspectorTerminal?: InspectorTerminal;
  readonly onRuntimeReady?: (info: {
    readonly runtimeInstanceId: string;
    readonly descriptorPath: string;
    readonly workflowCount: number;
  }) => Promise<void> | void;
}

function waitForShutdownSignal(): Promise<void> {
  return new Promise(resolveShutdown => {
    let draining = false;
    const shutdown = (signal: NodeJS.Signals): void => {
      if (draining) {
        process.stderr.write(
          `WOML runtime received a second ${signal}; forcing process exit.\n`
        );
        process.kill(process.pid, 'SIGKILL');
        return;
      }
      draining = true;
      process.stderr.write(
        `WOML runtime received ${signal}; stopping admission and draining work. Send the signal again to force exit.\n`
      );
      resolveShutdown();
    };
    process.on('SIGINT', shutdown);
    process.on('SIGTERM', shutdown);
  });
}

const defaultDependencies: CliDependencies = {
  createSecretStore: () => createSecretStore(),
  readSecret: readSecretFromTerminal,
  waitForShutdown: waitForShutdownSignal,
  fetch: globalThis.fetch,
  onRuntimeReady: async info => {
    const handoffPath = process.env.WOML_BACKGROUND_HANDOFF;
    const handoffToken = process.env.WOML_BACKGROUND_HANDOFF_TOKEN;
    if (handoffPath === undefined || handoffToken === undefined) return;
    await writeFile(
      handoffPath,
      `${JSON.stringify({
        profile: 'woml.background-runtime-control/v1',
        kind: 'started',
        token: handoffToken,
        runtimeInstanceId: info.runtimeInstanceId,
        pid: process.pid,
        status: 'ready',
        descriptorPath: info.descriptorPath,
        logPath: process.env.WOML_BACKGROUND_LOG,
        workflowCount: info.workflowCount,
      })}\n`,
      { mode: 0o600 }
    );
    await chmod(handoffPath, 0o600);
  },
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
    if (store.provider !== 'os-keychain') {
      throw new SecretStoreError(
        'WOML_SECRET_PROVIDER_READ_ONLY',
        `The ${store.provider} secret provider is read-only. Update that production secret source directly.`
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

async function runStopCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  const seen = new Set<string>();
  for (let index = 1; index < args.length; index += 1) {
    const option = args[index]!;
    if (seen.has(option) || !['--state', '--json'].includes(option)) {
      io.stderr(`${stopUsage()}\n`);
      return 2;
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = args[++index];
    if (value === undefined || value.startsWith('--')) {
      io.stderr(`${stopUsage()}\n`);
      return 2;
    }
    statePath = resolve(value);
  }
  const path = runtimeDescriptorPath(statePath);
  try {
    const descriptor = await readRuntimeDescriptor(path);
    const status = await requestRuntimeStop(
      descriptor,
      (dependencies.fetch ?? globalThis.fetch) as typeof globalThis.fetch
    );
    const deadline = Date.now() + 35_000;
    while (Date.now() < deadline && (await Bun.file(path).exists())) {
      await Bun.sleep(100);
    }
    if (await Bun.file(path).exists()) {
      throw new CliInputError(
        'WOML_RUNTIME_STOP_TIMEOUT',
        'The runtime accepted shutdown but did not stop before the deadline.'
      );
    }
    const result = {
      profile: 'woml.background-runtime-control/v1',
      kind: 'stop',
      runtimeInstanceId: descriptor.runtimeInstanceId,
      status: 'stopped',
    } as const;
    io.stdout(
      json
        ? `${JSON.stringify(result)}\n`
        : `WOML runtime ${descriptor.runtimeInstanceId} stopped (${status}).\n`
    );
    return 0;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
      io.stderr(
        'WOML runtime error [WOML_RUNTIME_NOT_RUNNING]: No active runtime descriptor was found.\n'
      );
      return 1;
    }
    try {
      const descriptor = await readRuntimeDescriptor(path);
      if (!processExists(descriptor.pid)) {
        await removeRuntimeDescriptor(path, descriptor.runtimeInstanceId);
        io.stderr(
          `WOML runtime error [WOML_RUNTIME_STALE_DESCRIPTOR]: Runtime ${descriptor.runtimeInstanceId} is no longer alive; its stale descriptor was removed.\n`
        );
        return 1;
      }
    } catch {
      // Preserve the original authenticated-control error when the descriptor
      // cannot be proved stale. PID is never used as shutdown authority.
    }
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

function processExists(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return !(
      error instanceof Error &&
      'code' in error &&
      error.code === 'ESRCH'
    );
  }
}

async function requestLiveRunOperation(
  statePath: string,
  operation: 'list_runs' | 'get_run' | 'cancel_run',
  subjectId: string | undefined,
  dependencies: CliDependencies,
  requestId?: string
): Promise<void> {
  const path = runtimeDescriptorPath(statePath);
  if (!(await Bun.file(path).exists())) return;
  const descriptor = await readRuntimeDescriptor(path);
  await requestRuntimeOperation(
    descriptor,
    operation,
    subjectId,
    (dependencies.fetch ?? globalThis.fetch) as typeof globalThis.fetch,
    requestId
  );
}

async function runInBackground(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  const foregroundArgs = args.filter(
    argument => argument !== '--background' && argument !== '-d'
  );
  let parsed: RunArguments;
  try {
    parsed = await resolveRunArgumentsConfiguration(
      parseRunArguments(foregroundArgs)
    );
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return error instanceof CliInputError ? 2 : 1;
  }
  const statePath = parsed.statePath;
  const descriptorPath = runtimeDescriptorPath(statePath);
  const logPath = runtimeLogPath(statePath, parsed.logDirectory);
  const handoffPath = join(
    dirname(statePath),
    `.background-${randomUUID()}.json`
  );
  const token =
    randomUUID().replaceAll('-', '') + randomUUID().replaceAll('-', '');
  await mkdir(dirname(logPath), { recursive: true, mode: 0o700 });
  await writeFile(handoffPath, `${JSON.stringify({ status: 'starting' })}\n`, {
    mode: 0o600,
    flag: 'wx',
  });
  await chmod(handoffPath, 0o600);
  const log = await open(logPath, 'a', 0o600);
  let child: ReturnType<typeof Bun.spawn>;
  try {
    child = Bun.spawn([process.execPath, process.argv[1]!, ...foregroundArgs], {
      cwd: process.cwd(),
      env: {
        ...process.env,
        WOML_BACKGROUND_HANDOFF: handoffPath,
        WOML_BACKGROUND_HANDOFF_TOKEN: token,
        WOML_BACKGROUND_LOG: logPath,
      },
      stdin: 'ignore',
      stdout: log.fd,
      stderr: log.fd,
      detached: true,
    });
    child.unref();
  } finally {
    await log.close();
  }

  try {
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      let handoff: Record<string, unknown> | undefined;
      try {
        handoff = JSON.parse(await readFile(handoffPath, 'utf8')) as Record<
          string,
          unknown
        >;
      } catch {
        // The child replaces this tiny file atomically enough for a bounded
        // retry to handle an in-progress write.
      }
      if (
        handoff?.kind === 'started' &&
        handoff.status === 'ready' &&
        handoff.token === token &&
        typeof handoff.runtimeInstanceId === 'string' &&
        typeof handoff.pid === 'number'
      ) {
        io.stdout(
          [
            'WOML runtime started in the background.',
            `PID: ${handoff.pid}`,
            `Runtime: ${handoff.runtimeInstanceId}`,
            `Workflows: ${handoff.workflowCount}`,
            `Descriptor: ${descriptorPath}`,
            `Logs: ${logPath}`,
            `Inspect: woml inspect --state ${JSON.stringify(statePath)}`,
            `Stop: woml stop --state ${JSON.stringify(statePath)}`,
          ].join('\n') + '\n'
        );
        return 0;
      }
      if (handoff?.status === 'startup_failed' && handoff.token === token) {
        io.stderr(
          `WOML runtime failed to start. See ${logPath} for the actionable startup error.\n`
        );
        return 1;
      }
      if (child.exitCode !== null) {
        io.stderr(`WOML runtime failed to start. See ${logPath}.\n`);
        return 1;
      }
      await Bun.sleep(50);
    }
    io.stderr(
      `WOML runtime startup timed out; readiness was not confirmed. See ${logPath}.\n`
    );
    return 1;
  } finally {
    await unlink(handoffPath).catch(() => {});
  }
}

export async function runCli(
  args: readonly string[],
  io: CliIo = processIo,
  dependencies: CliDependencies = defaultDependencies
): Promise<number> {
  if (
    args[0] === 'run' &&
    (args.includes('--background') || args.includes('-d')) &&
    process.env.WOML_BACKGROUND_HANDOFF === undefined
  ) {
    if (args.includes('--background') && args.includes('-d')) {
      io.stderr(`${runUsage()}\n`);
      return 2;
    }
    return await runInBackground(args, io);
  }
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

  if (args[0] === 'stop') {
    return await runStopCommand(args, io, dependencies);
  }

  if (args[0] === 'inspect') {
    let parsed;
    try {
      parsed = parseInspectArguments(args);
    } catch {
      io.stderr(`${inspectUsage}\n`);
      return 2;
    }
    return await runRuntimeInspector({
      args: parsed,
      ...(dependencies.inspectorTerminal === undefined
        ? {}
        : { terminal: dependencies.inspectorTerminal }),
      fetcher: (dependencies.fetch ??
        globalThis.fetch) as typeof globalThis.fetch,
    });
  }

  if (args[0] === 'backup') {
    try {
      const parsed = parseBackupArguments(args);
      const manifest = await createProductionBackup(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      io.stdout(
        parsed.json
          ? `${JSON.stringify(manifest)}\n`
          : [
              'WOML backup created and verified.',
              `Backup: ${parsed.backupDirectory}`,
              `Backup ID: ${manifest.backupId}`,
              `Store version: ${manifest.storeVersion}`,
              `Definitions: ${manifest.definitionHashes.length}`,
              `Database: ${manifest.database.sizeBytes} bytes (${manifest.database.digest})`,
            ].join('\n') + '\n'
      );
      return 0;
    } catch (error) {
      if (
        error instanceof ProductionBackupError &&
        error.code === 'WOML_CLI_ARGUMENTS_INVALID'
      ) {
        io.stderr(`${backupUsage}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return 1;
    }
  }

  if (args[0] === 'restore') {
    try {
      const parsed = parseRestoreArguments(args);
      const restored = await restoreProductionBackup(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      io.stdout(
        parsed.json
          ? `${JSON.stringify(restored)}\n`
          : [
              'WOML backup restored and verified.',
              `Backup ID: ${restored.backupId}`,
              `State: ${restored.statePath}`,
              `Store version: ${restored.storeVersion}`,
              ...(restored.rollbackPath === undefined
                ? []
                : [`Previous state retained at: ${restored.rollbackPath}`]),
            ].join('\n') + '\n'
      );
      return 0;
    } catch (error) {
      if (
        error instanceof ProductionBackupError &&
        error.code === 'WOML_CLI_ARGUMENTS_INVALID'
      ) {
        io.stderr(`${restoreUsage}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return 1;
    }
  }

  if (args[0] === 'prune') {
    try {
      const parsed = parsePruneArguments(args);
      const outcome = await runProductionRetention(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      if ('eligibleRuns' in outcome) {
        io.stdout(
          parsed.json
            ? `${JSON.stringify(outcome)}\n`
            : [
                'WOML retention dry run completed; nothing was deleted.',
                `Policy: ${outcome.policyId}`,
                `Eligible terminal runs: ${outcome.eligibleRuns}`,
                `Estimated removable history: ${outcome.estimatedBytes} bytes`,
                `Execute: woml prune --before ${parsed.before} --state ${JSON.stringify(parsed.statePath)}`,
              ].join('\n') + '\n'
        );
      } else {
        io.stdout(
          parsed.json
            ? `${JSON.stringify(outcome.result)}\n`
            : [
                'WOML retention completed.',
                `Policy: ${outcome.result.policyId}`,
                `Deleted terminal runs: ${outcome.result.deletedRuns}`,
                `Removed logical history: ${outcome.result.deletedBytes} bytes`,
                `Batches: ${outcome.batches}`,
                `Durable state entries deleted: ${outcome.result.stateEntriesDeleted}`,
                `WAL checkpoint: ${outcome.checkpointedFrames}/${outcome.checkpointLogFrames} frames${outcome.checkpointBusy === 0 ? '' : ' (readers still active)'}`,
                `Compaction: ${outcome.compacted ? 'completed' : 'not requested'}`,
              ].join('\n') + '\n'
        );
      }
      return 0;
    } catch (error) {
      if (
        error instanceof ProductionRetentionError &&
        error.code === 'WOML_CLI_ARGUMENTS_INVALID'
      ) {
        io.stderr(`${pruneUsage}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return 1;
    }
  }

  if (args[0] === 'list') {
    try {
      const list = parseRunListArguments(args);
      await requestLiveRunOperation(
        list.statePath,
        'list_runs',
        undefined,
        dependencies
      );
      const result = listRunsWithRust(
        list.statePath,
        {
          limit: list.limit,
          workflowId: list.workflowId,
          status: list.status,
        },
        { nativeCorePath: dependencies.nativeCorePath }
      );
      io.stdout(
        list.json ? `${JSON.stringify(result)}\n` : formatRunList(result)
      );
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
      await requestLiveRunOperation(
        get.statePath,
        'get_run',
        get.runId,
        dependencies
      );
      const result = inspectRunV2WithRust(get.statePath, get.runId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      io.stdout(
        get.json ? `${JSON.stringify(result)}\n` : formatRunInspection(result)
      );
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
      const commandId = `cancel_${randomUUID().replaceAll('-', '')}`;
      await requestLiveRunOperation(
        cancel.statePath,
        'cancel_run',
        cancel.runId,
        dependencies,
        commandId
      );
      const result = cancelRunWithRust(
        cancel.statePath,
        cancel.runId,
        commandId,
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
    io.stderr(
      'The "woml runs" namespace was removed. Use "woml list", "woml get", or "woml cancel".\n'
    );
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
  try {
    runArguments = await resolveRunArgumentsConfiguration(runArguments);
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
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
    sources = await compileWorkflowInputs(inputPaths, io);
    printMigrationDiagnostics(
      io,
      sources.flatMap(source => source.migrationDiagnostics)
    );
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
  const exitCode = await runCli(process.argv.slice(2));
  if (exitCode !== 0) {
    const handoffPath = process.env.WOML_BACKGROUND_HANDOFF;
    const handoffToken = process.env.WOML_BACKGROUND_HANDOFF_TOKEN;
    if (handoffPath !== undefined && handoffToken !== undefined) {
      await writeFile(
        handoffPath,
        `${JSON.stringify({
          profile: 'woml.background-runtime-control/v1',
          kind: 'started',
          token: handoffToken,
          status: 'startup_failed',
          pid: process.pid,
        })}\n`,
        { mode: 0o600 }
      ).catch(() => {});
    }
  }
  process.exitCode = exitCode;
}
