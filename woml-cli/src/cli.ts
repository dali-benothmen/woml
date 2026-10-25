#!/usr/bin/env bun

import { createHash, randomUUID } from 'node:crypto';
import { existsSync, realpathSync } from 'node:fs';
import {
  chmod,
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  stat,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { basename, dirname, extname, join, resolve } from 'node:path';

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
  validateWoml,
  validateResolvedReusableWorkflow,
  WOML_WHATSAPP_CALLBACK_PATH,
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
  type WomlDefinitionPackageV10,
  type WomlDefinitionPackageV11,
  type WomlDefinitionPackageV12,
  type WomlDefinitionPackageV13,
  type WomlDefinitionPackageV14,
  type WomlDefinitionPackageV15,
  type WomlDefinitionPackageV16,
} from '@woml/compiler';
import {
  compiledDefinitionHash,
  executeApprovalWorkflowWithRust,
  executeWorkflowWithRust,
  executeWorkflowWithRustDurable,
  type ExecutionProgressV1,
  ApprovalDecisionError,
  type NotificationDispatchReport,
  NotificationProviderError,
  type NotificationJourneyDiagnostics,
  resolveApprovalWithRust,
  resolveNotificationApprovalWithRust,
  resumeApprovalWorkflowWithRust,
  resumeWorkflowWithRustDurable,
  RustWorkflowExecutionError,
  runNotificationProviderJourneyWithRust,
  cancelRunWithRust,
  inspectRunPresentationWithRust,
  inspectRunV2WithRust,
  inspectWaitsV1WithRust,
  inspectLocksV1WithRust,
  inspectCorrectionV1WithRust,
  inspectRemoteDispatchWithRust,
  reconcileRemoteDispatchWithRust,
  recoverCorrectionWithRust,
  listRunPresentationsWithRust,
  hasWorkflowDefinitionWithRust,
  listRunsWithRust,
  observeRuntimeWithRust,
  observeLocksV1WithRust,
  readOperationalOutputPageWithRust,
  inspectOperationalProgressWithRust,
  observeOperationalOutputWithRust,
  BackupOperationError,
  ArtifactOperationError,
  RetentionOperationError,
  RunManagementError,
  RunInspectionError,
  settleApprovalTimeoutWithRust,
  settleTimerWaitWithRust,
  inspectStoredRunRequirementsWithRust,
  resumeStoredRunWithRust,
  activateWebhookRuntimeWithRust,
  startWebhookRuntimeWithRust,
  stopWebhookRuntimeWithRust,
  submitExternalSignalWithRust,
  submitTriggerOccurrenceWithRust,
  submitManualTriggerWithRust,
  TriggerRuntimeError,
  type IntervalProgressV1,
  type PublicRunStatus,
  type RustRunCancellationResultV1,
  type RustRunInspectionV2,
  type RustWaitInspectionV1,
  type RustLockInspectionV1,
  type RustCorrectionInspectionV1,
  type RustRunListV1,
  type ScheduleProgressV1,
  type TriggerProgressV1,
  type WorkflowCallProgressV1,
  type RustApprovalRuntimeOutcome,
  type RustRuntimeModuleArtifact,
  type RuntimeCapabilitiesV1,
  type StoredRunRequirementsV1,
  exportArtifactWithRust,
  inspectArtifactWithRust,
  inspectRunArtifactsWithRust,
  observeArtifactsWithRust,
  type RustArtifactInspectionV1,
} from './rust-executor';
import {
  loadReviewerDirectory,
  ReviewerDirectoryLoadError,
} from './reviewer-directory';
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
  createForegroundProcessGrant,
  loadProcessPolicy,
  processGrantSummary,
  ProcessPolicyError,
} from './process-policy';
import {
  artifactGrantSummary,
  artifactPolicySecretNames,
  ArtifactPolicyError,
  loadArtifactPolicy,
} from './artifact-policy';
import {
  InfrastructurePolicyError,
  infrastructurePolicyRuntimeAuthority,
  resolveInfrastructurePolicy,
  type ResolvedInfrastructurePolicyV1,
} from './infrastructure-policy';
import {
  analyzeInfrastructurePolicySource,
  infrastructurePolicyDiagnostics,
} from './infrastructure-policy-analysis';
import {
  CommunicationProviderRegistry,
  CommunicationTriggerHost,
} from './communication-provider';
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
  SharedTelegramTransport,
  TelegramTriggerHost,
  telegramStartupError,
  telegramTriggerRegistrations,
  type SharedTelegramTransportOptions,
} from './telegram';
import {
  DiscordTriggerHost,
  SharedDiscordTransport,
  discordStartupError,
  discordTriggerRegistrations,
  type SharedDiscordTransportOptions,
} from './discord';
import {
  preflightRuntimeConfiguration,
  resolveRuntimeConfiguration,
  type ResolvedOpenTelemetryConfigurationV1,
  type ResolvedRuntimeConfiguration,
  type RuntimePreflightV1,
} from './runtime-config';
import type {
  OpenTelemetryRuntime,
  OpenTelemetryRuntimeOptions,
  OpenTelemetrySdkLoader,
  TelemetryObservationProjector,
} from './opentelemetry-runtime';
import type {
  TelemetryAdmissionLinkV1,
  TelemetrySignalV1,
} from './opentelemetry-signal';

interface OpenTelemetryRuntimeModule {
  readonly OpenTelemetryRuntime: {
    start(
      configuration: ResolvedOpenTelemetryConfigurationV1,
      options: OpenTelemetryRuntimeOptions
    ): Promise<OpenTelemetryRuntime>;
  };
  readonly parseTelemetrySignal: (json: string) => TelemetrySignalV1;
  readonly parseTelemetryAdmissionLink: (
    json: string
  ) => TelemetryAdmissionLinkV1;
  readonly createTelemetryObservationProjector: () => TelemetryObservationProjector;
}
import {
  readRuntimeDescriptor,
  removeRuntimeDescriptor,
  requestRuntimeOperation,
  requestRuntimeSignal,
  requestRuntimeStop,
  RuntimeControlError,
  runtimeDescriptorPath,
  runtimeLogPath,
  startRuntimeControl,
  type RuntimeControlHandle,
} from './runtime-control';
import { RuntimeObservability } from './runtime-observability';
import {
  renderOperationalRecord,
  RuntimeOperationalOutput,
  type OperationalOutputWakeupV1,
} from './operational-output';
import { ForegroundPresentation } from './foreground-presentation';
import {
  followWorkflowLogs,
  LogFollowError,
  logFollowUsage,
  resolveLogFollowArguments,
} from './log-follower';
import {
  createProcessManualLineInput,
  type ManualLineInput,
} from './manual-input';
import {
  renderManualTargetSelection,
  renderRunPresentation,
  renderWorkflowStartup,
  RunPresentationDecodeError,
  type ColorMode,
  type PresentationRenderOptions,
  type TriggerPresentationV1,
  type WorkflowPresentationV1,
} from './terminal-presentation';
import {
  flushPerformanceProfile,
  profileAsync,
  profileSync,
} from './performance-profiler';
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
import {
  formatProviderDoctor,
  parseProviderDoctorArguments,
  providerDoctorUsage,
  runProviderDoctor,
} from './provider-doctor';
import {
  readCompiledWorkflowCache,
  sourceSnapshotDigest,
  writeCompiledWorkflowCache,
  type CompiledWorkflowCacheOptions,
} from './compiled-workflow-cache';
import { runtimeActivationRequirements } from './runtime-activation';
import {
  createCombinedRunPresentation,
  createProjectActivationPresentation,
  renderProjectActivationPresentation,
} from './combined-run-presentation';
import {
  checkWorkerConfiguration,
  RemoteWorkerError,
  startWorker,
} from './remote-worker';
import {
  inspectTarget,
  loadTargetRegistry,
  preflightRemoteTargets,
  remoteTargetRequirements,
  TargetRegistryError,
  type PreparedTargetRegistryV1,
  type TargetPreflightV1,
} from './target-registry';
import {
  resolveWomlProject,
  type ResolvedWomlProjectV1,
  type WomlProjectReferenceKey,
  WomlProjectError,
} from './project';
import {
  initializeWomlProject,
  WomlProjectInitializationError,
} from './project-init';
import {
  createWomlProjectCheck,
  formatWomlProjectCheck,
  type ProjectCheckDiagnosticV1,
  type ProjectCheckSubsystemV1,
} from './project-preflight';

export interface CliIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
  readonly isTTY?: boolean;
  readonly columns?: number;
}

const processIo: CliIo = {
  stdout: text => process.stdout.write(text),
  stderr: text => process.stderr.write(text),
  isTTY: process.stderr.isTTY === true,
  columns: process.stderr.columns,
};

const WOML_CLI_VERSION = packageMetadata.version;
const COMPILED_WORKFLOW_CACHE_COMPILER_IDENTITY = `woml-cli@${WOML_CLI_VERSION}:cache-v1:model-v16:definition-package-v11`;

class CliInputError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'CliInputError';
    this.code = code;
  }
}

function runUsage(): string {
  return 'Usage: woml run [<workflow.woml|directory>...|--project <path>] [--config <path>] [--target-registry <path>] [--target-trust <path>] [--policy <name|path>] [--emergency-deny <name|path>] [--policy-trust <path>] [--require-policy-signature] [--process-policy <path>|--allow-process <aliases>] [--artifact-policy <path>] [--reviewer-directory <path>] [--host <address>] [--port <port>] [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] [--approval-port <port>] [--json] [--verbose] [--color=auto|always|never] [--background|-d]';
}

function testUsage(): string {
  return 'Usage: woml test [<workflow.woml>|--project <path>] [--target-registry <path>] [--target-trust <path>] [--policy <name|path>] [--emergency-deny <name|path>] [--policy-trust <path>] [--require-policy-signature] [--process-policy <path>|--allow-process <aliases>] [--artifact-policy <path>] [--reviewer-directory <path>] [--state <path>] [--trigger <manualTriggerId>] [--resume <runId>] [--approval-port <port>]';
}

function policyUsage(): string {
  return [
    'Usage:',
    '  woml policy check <name|path> [--trust <path>] [--require-signature] [--json]',
    '  woml policy explain <workflow.woml> --policy <name|path> [--step <stepId>] [--trust <path>] [--require-signature] [--json]',
  ].join('\n');
}

function listUsage(): string {
  return 'Usage: woml list [--workflow <workflowId>] [--status <status>] [--limit <1-200>] [--state <path>] [--json]';
}

function getUsage(): string {
  return [
    'Usage: woml get <runId> [--state <path>] [--json]',
    'Shows the run together with durable waits, artifacts, and lock/lease state.',
  ].join('\n');
}

function artifactsUsage(): string {
  return [
    'Usage:',
    '  woml artifacts inspect <artifactId> [--state <path>] [--json]',
    '  woml artifacts export <artifactId> --output <path> --artifact-policy <path> [--state <path>] [--json]',
  ].join('\n');
}

function cancelUsage(): string {
  return 'Usage: woml cancel <runId> [--state <path>] [--json]';
}

function recoverUsage(): string {
  return [
    'Usage:',
    '  woml recover <runId> --retry <actionId> [--state <path>] [--json]',
    '  woml recover <runId> --resolve <actionId> --note <text> [--state <path>] [--json]',
  ].join('\n');
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

function signalUsage(): string {
  return 'Usage: woml signal <name> --run <runId> --wait <waitId> --id <idempotencyKey> --correlation <value> --payload <jsonFile> [--state <path>]';
}

function checkUsage(): string {
  return 'Usage: woml check [<workflow.woml|directory>...|--project <path>] [--config <path>] [--target-registry <path>] [--target-trust <path>] [--policy <name|path>] [--policy-trust <path>] [--require-policy-signature] [--process-policy <path>] [--artifact-policy <path>] [--reviewer-directory <path>] [--json]';
}

function initUsage(): string {
  return 'Usage: woml init [directory] [--json]';
}

function targetUsage(): string {
  return [
    'Usage:',
    '  woml target list [--registry <path>] [--trust <path>] [--json]',
    '  woml target check <target> [--registry <path>] [--trust <path>] [--json]',
    '  woml target inspect <target> [--registry <path>] [--trust <path>] --json',
    '  woml target drain <target-or-worker> [--registry <path>] [--trust <path>] [--state <path>] [--json]',
  ].join('\n');
}

function workerUsage(): string {
  return [
    'Usage:',
    '  woml worker check --config <path> [--json]',
    '  woml worker start --config <path> [--json]',
  ].join('\n');
}

function dispatchUsage(): string {
  return [
    'Usage:',
    '  woml dispatch inspect <dispatchId> [--state <path>] [--json]',
    '  woml dispatch reconcile <dispatchId> --action <record-failure|authorize-possible-duplicate-retry> --request-id <id> [--reason <text>] [--state <path>] [--json]',
  ].join('\n');
}

interface RemoteDrainEntryV1 {
  readonly kind: 'target' | 'worker';
  readonly id: string;
  readonly drainedAt: string;
}

interface RemoteDrainStateV1 {
  readonly profile: 'woml.remote-drain/v1';
  readonly version: 1;
  readonly entries: readonly RemoteDrainEntryV1[];
}

function isCanonicalDrainTimestamp(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const timestamp = new Date(value);
  return !Number.isNaN(timestamp.getTime()) && timestamp.toISOString() === value;
}

async function readRemoteDrainState(path: string): Promise<RemoteDrainStateV1> {
  if (!existsSync(path)) {
    return { profile: 'woml.remote-drain/v1', version: 1, entries: [] };
  }
  const source = await readFile(path, 'utf8');
  if (Buffer.byteLength(source) > 256 * 1024) {
    throw new TargetRegistryError('WOML_TARGET_INVALID', 'The remote drain state exceeds 256 KiB.');
  }
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    throw new TargetRegistryError('WOML_TARGET_INVALID', 'The remote drain state is malformed.');
  }
  if (
    value === null || typeof value !== 'object' || Array.isArray(value) ||
    Object.keys(value).sort().join(',') !== 'entries,profile,version' ||
    (value as { profile?: unknown }).profile !== 'woml.remote-drain/v1' ||
    (value as { version?: unknown }).version !== 1 ||
    !Array.isArray((value as { entries?: unknown }).entries)
  ) {
    throw new TargetRegistryError('WOML_TARGET_INVALID', 'The remote drain state violates v1.');
  }
  const entries = (value as { entries: unknown[] }).entries;
  if (entries.length > 1_024 || !entries.every(entry => {
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) return false;
    const record = entry as Record<string, unknown>;
    return Object.keys(record).sort().join(',') === 'drainedAt,id,kind' &&
      (record.kind === 'target' || record.kind === 'worker') &&
      typeof record.id === 'string' && /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/u.test(record.id) &&
      isCanonicalDrainTimestamp(record.drainedAt);
  })) {
    throw new TargetRegistryError('WOML_TARGET_INVALID', 'The remote drain entries violate v1.');
  }
  return value as RemoteDrainStateV1;
}

async function persistRemoteDrain(
  path: string,
  state: RemoteDrainStateV1,
  entry: RemoteDrainEntryV1
): Promise<{ readonly profile: 'woml.remote-drain-result/v1'; readonly status: 'draining'; readonly kind: 'target' | 'worker'; readonly id: string; readonly statePath: string }> {
  const entries = [
    ...state.entries.filter(existing => !(existing.kind === entry.kind && existing.id === entry.id)),
    entry,
  ].sort((left, right) => `${left.kind}:${left.id}`.localeCompare(`${right.kind}:${right.id}`));
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, `${JSON.stringify({ profile: 'woml.remote-drain/v1', version: 1, entries })}\n`, {
      encoding: 'utf8', mode: 0o600, flag: 'wx',
    });
    await rename(temporary, path);
  } finally {
    await unlink(temporary).catch(() => undefined);
  }
  return {
    profile: 'woml.remote-drain-result/v1',
    status: 'draining',
    kind: entry.kind,
    id: entry.id,
    statePath: path,
  };
}

function remoteDrainHealth(
  state: RemoteDrainStateV1,
  target: string,
  workers: readonly string[],
  fallback: { readonly status: 'not_observed'; readonly reason: 'worker-transport-not-active' }
): { readonly status: 'draining'; readonly reason: 'operator-drain'; readonly workers: readonly string[] } | typeof fallback {
  const targetDrain = state.entries.some(entry => entry.kind === 'target' && entry.id === target);
  const drainedWorkers = state.entries
    .filter(entry => entry.kind === 'worker' && workers.includes(entry.id))
    .map(entry => entry.id)
    .sort();
  return targetDrain || drainedWorkers.length > 0
    ? { status: 'draining', reason: 'operator-drain', workers: drainedWorkers }
    : fallback;
}

async function runDispatchCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  const operation = args[1];
  const dispatchId = args[2];
  if ((operation !== 'inspect' && operation !== 'reconcile') || dispatchId === undefined || !/^dispatch_[a-f0-9]{64}$/u.test(dispatchId)) {
    io.stderr(`${dispatchUsage()}\n`);
    return 2;
  }
  let statePath = resolve('.woml/state.sqlite');
  let action: 'record-failure' | 'authorize-possible-duplicate-retry' | undefined;
  let requestId: string | undefined;
  let reason: string | undefined;
  let json = false;
  for (let index = 3; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === '--json' && !json) {
      json = true;
      continue;
    }
    const value = args[index + 1];
    if (value === undefined || value.startsWith('--')) {
      io.stderr(`${dispatchUsage()}\n`);
      return 2;
    }
    if (argument === '--state') statePath = value;
    else if (argument === '--action' && ['record-failure', 'authorize-possible-duplicate-retry'].includes(value)) action = value as typeof action;
    else if (argument === '--request-id' && /^[A-Za-z0-9_-]{1,256}$/u.test(value)) requestId = value;
    else if (argument === '--reason' && value.length <= 1024) reason = value;
    else {
      io.stderr(`${dispatchUsage()}\n`);
      return 2;
    }
    index += 1;
  }
  if (operation === 'reconcile' && (action === undefined || requestId === undefined)) {
    io.stderr(`${dispatchUsage()}\n`);
    return 2;
  }
  try {
    const evidenceDigest = `sha256:${createHash('sha256').update(JSON.stringify({
      profile: 'woml.remote-dispatch-reconciliation/v1', dispatchId, action,
      requestId, ...(reason === undefined ? {} : { reason }),
    })).digest('hex')}`;
    const result = operation === 'inspect'
      ? inspectRemoteDispatchWithRust(statePath, dispatchId, { nativeCorePath: dependencies.nativeCorePath })
      : reconcileRemoteDispatchWithRust(statePath, dispatchId, action!, requestId!, evidenceDigest, { nativeCorePath: dependencies.nativeCorePath });
    io.stdout(
      json
        ? `${JSON.stringify(result)}\n`
        : operation === 'inspect'
          ? [
              'WOML remote dispatch',
              `Dispatch: ${dispatchId}`,
              `State: ${String((result as { state: string }).state)}`,
              `Target: ${String((result as { target: string }).target)}`,
              `Worker: ${String((result as { workerId?: string }).workerId ?? 'unassigned')}`,
              `Last heartbeat: ${String((result as { lastHeartbeatAt?: string }).lastHeartbeatAt ?? 'none')}`,
              `Output cursor: ${String((result as { outputSequence: number }).outputSequence)}`,
              `Ambiguous: ${String((result as { ambiguous: boolean }).ambiguous)}`,
            ].join('\n') + '\n'
          : [
              'WOML remote dispatch reconciled.',
              `Dispatch: ${dispatchId}`,
              `State: ${String((result as { state: string }).state)}`,
              `Action: ${action}`,
              `Request: ${requestId}`,
            ].join('\n') + '\n'
    );
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

async function runWorkerCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  const operation = args[1];
  let configPath: string | undefined;
  let json = false;
  for (let index = 2; index < args.length; index += 1) {
    if (args[index] === '--json' && !json) {
      json = true;
    } else if (
      args[index] === '--config' &&
      configPath === undefined &&
      args[index + 1] !== undefined
    ) {
      configPath = args[++index];
    } else {
      io.stderr(`${workerUsage()}\n`);
      return 2;
    }
  }
  if ((operation !== 'check' && operation !== 'start') || configPath === undefined) {
    io.stderr(`${workerUsage()}\n`);
    return 2;
  }
  try {
    if (operation === 'check') {
      const result = await checkWorkerConfiguration(configPath);
      io.stdout(
        json
          ? `${JSON.stringify(result)}\n`
          : [
              'WOML worker configuration is valid.',
              `Worker: ${result.workerId}`,
              `Coordinator: ${result.coordinator}`,
              `Targets: ${result.targets.join(', ')}`,
              `Capacity: ${result.capacity}`,
              'Transport: mutual TLS',
              'Dispatch activation: coordinator-owned; configuration check opens no session',
            ].join('\n') + '\n'
      );
      return 0;
    }
    await startWorker(
      configPath,
      WOML_CLI_VERSION,
      dependencies.waitForShutdown ?? waitForShutdownSignal,
      session =>
        io.stdout(
          json
            ? `${JSON.stringify(session)}\n`
            : [
                'WOML remote worker session established.',
                `Worker: ${String(session.workerId)}`,
                `Session: ${String(session.sessionId)}`,
                'Status: authenticated and polling',
              ].join('\n') + '\n'
        )
    );
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

interface TargetCommandArguments {
  readonly operation: 'list' | 'check' | 'inspect' | 'drain';
  readonly target?: string;
  readonly registryPath?: string;
  readonly trustPath?: string;
  readonly statePath: string;
  readonly json: boolean;
}

function parseTargetCommandArguments(
  args: readonly string[]
): TargetCommandArguments {
  const operation = args[1];
  if (operation !== 'list' && operation !== 'check' && operation !== 'inspect' && operation !== 'drain') {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
  }
  let index = 2;
  let target: string | undefined;
  if (operation !== 'list') {
    target = args[index];
    if (
      target === undefined ||
      !/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/u.test(target) ||
      target.length > 128
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
    }
    index += 1;
  }
  let registryPath: string | undefined;
  let trustPath: string | undefined;
  let statePath = resolve('.woml/remote-drain.json');
  let json = false;
  for (; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--json') {
      if (json) throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
      json = true;
      continue;
    }
    const value = args[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
    }
    if (argument === '--registry' && registryPath === undefined) {
      registryPath = value;
    } else if (argument === '--trust' && trustPath === undefined) {
      trustPath = value;
    } else if (argument === '--state') {
      statePath = resolve(value);
    } else {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
    }
    index += 1;
  }
  if (
    (operation === 'inspect' && !json)
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', targetUsage());
  }
  return {
    operation,
    ...(target === undefined ? {} : { target }),
    ...(registryPath === undefined ? {} : { registryPath }),
    ...(trustPath === undefined ? {} : { trustPath }),
    statePath,
    json,
  };
}

async function runTargetCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  let parsed: TargetCommandArguments;
  try {
    parsed = parseTargetCommandArguments(args);
  } catch (error) {
    io.stderr(`${error instanceof Error && error.message !== targetUsage() ? `${error.message}\n` : ''}${targetUsage()}\n`);
    return 2;
  }
  try {
    const defaultRegistry = resolve('.woml/targets.json');
    const defaultTrust = resolve('.woml/target-trust.json');
    const registryPath =
      parsed.registryPath ??
      (existsSync(defaultRegistry) ? defaultRegistry : undefined);
    const trustPath =
      parsed.trustPath ?? (existsSync(defaultTrust) ? defaultTrust : undefined);
    const localInspection = {
      profile: 'woml.target-inspection/v1' as const,
      target: 'local',
      source: 'built-in' as const,
      kind: 'local' as const,
      requiredCapabilities: [] as readonly string[],
      allowedWorkers: ['coordinator'] as readonly string[],
      eligibleWorkers: ['coordinator'] as readonly string[],
      minimumWorkerVersion: WOML_CLI_VERSION,
      maximumInFlight: null,
      health: { status: 'ready' as const, reason: 'coordinator-local' as const },
    };
    const localListEntry = {
      id: 'local',
      source: 'built-in' as const,
      kind: 'local' as const,
      requiredCapabilities: [] as readonly string[],
      eligibleWorkers: 1,
      maximumInFlight: null,
      health: localInspection.health,
    };
    if (parsed.target === 'local') {
      if (parsed.operation === 'drain') {
        throw new TargetRegistryError(
          'WOML_TARGET_INVALID',
          'The built-in local coordinator target cannot be drained through remote worker controls.'
        );
      }
      const result =
        parsed.operation === 'check'
          ? {
              ...localInspection,
              profile: 'woml.target-check/v1' as const,
              status: 'passed' as const,
            }
          : localInspection;
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : 'Target local passed built-in coordinator preflight.\n'
      );
      return 0;
    }
    if (registryPath === undefined) {
      if (parsed.operation !== 'list') {
        throw new TargetRegistryError(
          'WOML_TARGET_UNKNOWN',
          'No target registry was provided and .woml/targets.json does not exist.'
        );
      }
      const result = {
        profile: 'woml.target-list/v1' as const,
        registryPath: null,
        registryDigest: null,
        trustStatus: 'not_configured' as const,
        targets: [localListEntry],
      };
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : 'local · built-in coordinator · ready\n'
      );
      return 0;
    }
    const prepared = await loadTargetRegistry(registryPath, trustPath);
    const drainState = await readRemoteDrainState(parsed.statePath);
    if (parsed.operation === 'drain') {
      const target = prepared.registry.targets.find(entry => entry.id === parsed.target);
      const worker = prepared.registry.targets
        .flatMap(entry => entry.allowedWorkerIdentities)
        .includes(parsed.target!);
      if (target === undefined && !worker) {
        throw new TargetRegistryError(
          'WOML_TARGET_UNKNOWN',
          `Neither a target nor worker named ${JSON.stringify(parsed.target)} exists in the registry.`
        );
      }
      const kind = target === undefined ? 'worker' as const : 'target' as const;
      const result = await persistRemoteDrain(parsed.statePath, drainState, {
        kind,
        id: parsed.target!,
        drainedAt: new Date().toISOString(),
      });
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : `${kind === 'target' ? 'Target' : 'Worker'} ${parsed.target} is draining; accepted work may finish and new dispatch admission is disabled.\n`
      );
      return 0;
    }
    if (parsed.operation === 'list') {
      const result = {
        profile: 'woml.target-list/v1' as const,
        registryPath: prepared.path,
        registryDigest: prepared.digest,
        trustStatus: prepared.trust === undefined ? 'not_configured' : 'verified',
        targets: [
          localListEntry,
          ...prepared.registry.targets.map(target => {
            const inspected = inspectTarget(prepared, target.id);
            return {
              id: target.id,
              source: 'operator-registry' as const,
              kind: target.kind,
              requiredCapabilities: target.requiredCapabilities,
              eligibleWorkers: inspected.eligibleWorkers.length,
              minimumWorkerVersion: target.minimumWorkerVersion,
              maximumInFlight: target.maximumInFlight,
              health: remoteDrainHealth(drainState, target.id, target.allowedWorkerIdentities, inspected.health),
            };
          }),
        ],
      };
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : result.targets.length === 0
            ? `No remote targets are configured in ${result.registryPath}.\n`
            : [
                `Target registry ${result.registryDigest} · trust ${result.trustStatus}`,
                ...result.targets.map(target =>
                  target.id === 'local'
                    ? 'local · built-in coordinator · ready'
                    : `${target.id} · operator registry · ${target.requiredCapabilities.join(', ')} · ${target.eligibleWorkers} eligible worker${target.eligibleWorkers === 1 ? '' : 's'} · health not observed`
                ),
              ].join('\n') + '\n'
      );
      return 0;
    }
    const baseInspection = inspectTarget(prepared, parsed.target!);
    const targetEntry = prepared.registry.targets.find(target => target.id === parsed.target)!;
    const inspection = {
      ...baseInspection,
      health: remoteDrainHealth(
        drainState,
        parsed.target!,
        targetEntry.allowedWorkerIdentities,
        baseInspection.health
      ),
    };
    if (parsed.operation === 'check') {
      if (inspection.eligibleWorkers.length === 0) {
        throw new TargetRegistryError(
          'WOML_TARGET_UNAUTHORIZED',
          `Target ${JSON.stringify(inspection.target)} has no worker identity anchored by the target trust file.`
        );
      }
      const result = {
        ...inspection,
        profile: 'woml.target-check/v1' as const,
        status: 'passed' as const,
      };
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : `Target ${inspection.target} passed registry and trust preflight · ${inspection.eligibleWorkers.length} eligible worker${inspection.eligibleWorkers.length === 1 ? '' : 's'} · health not observed.\n`
      );
      return 0;
    }
    io.stdout(`${JSON.stringify(inspection)}\n`);
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
}

async function loadDefaultTargetRegistryForRemoteWorkflows(
  workflows: readonly CompiledWorkflowDefinition[],
  prepared: PreparedTargetRegistryV1 | undefined
): Promise<PreparedTargetRegistryV1 | undefined> {
  if (prepared !== undefined || remoteTargetRequirements(workflows).length === 0) {
    return prepared;
  }
  const registryPath = resolve('.woml/targets.json');
  if (!existsSync(registryPath)) return undefined;
  const trustPath = resolve('.woml/target-trust.json');
  return await loadTargetRegistry(
    registryPath,
    existsSync(trustPath) ? trustPath : undefined
  );
}

function typesUsage(): string {
  return 'Usage: woml types [<workflow.woml|directory>|--project <path>] [--output <path>]';
}

function usage(): string {
  return `${initUsage()}\n${runUsage()}\n${testUsage()}\n${logFollowUsage}\n${checkUsage()}\n${targetUsage()}\n${workerUsage()}\n${dispatchUsage()}\n${policyUsage()}\n${typesUsage()}\n${inspectUsage}\n${artifactsUsage()}\n${backupUsage}\n${restoreUsage}\n${pruneUsage}\n${listUsage()}\n${getUsage()}\n${cancelUsage()}\n${recoverUsage()}\n${stopUsage()}\n${emitUsage()}\n${signalUsage()}\n${secretsUsage()}\n${providerDoctorUsage()}`;
}

interface RunArguments {
  readonly command: 'run' | 'test';
  readonly filePath: string;
  readonly inputPaths: readonly string[];
  readonly projectPath?: string;
  readonly projectSelection?: 'explicit' | 'current-directory';
  readonly project?: ResolvedWomlProjectV1;
  readonly statePath: string;
  readonly resumeRunId?: string;
  readonly approvalPort: number;
  readonly host: string;
  readonly port: number;
  readonly triggerId?: string;
  readonly configPath?: string;
  readonly processPolicyPath?: string;
  readonly artifactPolicyPath?: string;
  readonly reviewerDirectoryPath?: string;
  readonly targetRegistryPath?: string;
  readonly targetTrustPath?: string;
  readonly targetRegistry?: PreparedTargetRegistryV1;
  readonly targetPreflight?: TargetPreflightV1;
  readonly infrastructurePolicyReference?: string;
  readonly emergencyDenyReference?: string;
  readonly infrastructurePolicyTrustPath?: string;
  readonly requireInfrastructurePolicySignature: boolean;
  readonly allowProcess?: string;
  readonly runtimeCapabilities?: RuntimeCapabilitiesV1;
  readonly stateExplicit: boolean;
  readonly hostExplicit: boolean;
  readonly portExplicit: boolean;
  readonly adminHost: string;
  readonly adminPort: number;
  readonly shutdownTimeoutMs: number;
  readonly logDirectory: string;
  readonly logFormat: 'text' | 'json';
  readonly observabilityHealth: boolean;
  readonly observabilityMetrics: boolean;
  readonly openTelemetry?: ResolvedOpenTelemetryConfigurationV1;
  readonly retention?: AutomaticRetentionConfiguration;
  readonly verbose: boolean;
  readonly json: boolean;
  readonly color: ColorMode;
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
  let filePath = inputPaths[0];
  if (
    (command !== 'run' && command !== 'test') ||
    filePath?.startsWith('--') === true ||
    (command === 'test' && inputPaths.length > 1)
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
  let projectPath: string | undefined;
  let projectSelection: 'explicit' | 'current-directory' | undefined;
  let configPath: string | undefined;
  let processPolicyPath: string | undefined;
  let artifactPolicyPath: string | undefined;
  let reviewerDirectoryPath: string | undefined;
  let targetRegistryPath: string | undefined;
  let targetTrustPath: string | undefined;
  let infrastructurePolicyReference: string | undefined;
  let emergencyDenyReference: string | undefined;
  let infrastructurePolicyTrustPath: string | undefined;
  let requireInfrastructurePolicySignature = false;
  let allowProcess: string | undefined;
  let stateExplicit = false;
  let hostExplicit = false;
  let portExplicit = false;
  let verbose = false;
  let json = false;
  let color: ColorMode = 'auto';
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 1) {
    const rawOption = options[index]!;
    const colorAssignment = rawOption.startsWith('--color=')
      ? rawOption.slice('--color='.length)
      : undefined;
    const option = colorAssignment === undefined ? rawOption : '--color';
    if (seen.has(option)) {
      throw new CliInputError(
        'WOML_CLI_ARGUMENTS_INVALID',
        command === 'test' ? testUsage() : runUsage()
      );
    }
    seen.add(option);
    if (
      option === '--verbose' ||
      option === '--json' ||
      option === '--require-policy-signature'
    ) {
      if (option === '--require-policy-signature') {
        requireInfrastructurePolicySignature = true;
        continue;
      }
      if (command !== 'run') {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', testUsage());
      }
      if (option === '--verbose') verbose = true;
      else json = true;
      continue;
    }
    const value = colorAssignment ?? options[index + 1];
    if (
      value === undefined ||
      (option !== '--state' &&
        option !== '--project' &&
        option !== '--config' &&
        option !== '--process-policy' &&
        option !== '--artifact-policy' &&
        option !== '--reviewer-directory' &&
        option !== '--target-registry' &&
        option !== '--target-trust' &&
        option !== '--policy' &&
        option !== '--emergency-deny' &&
        option !== '--policy-trust' &&
        option !== '--allow-process' &&
        option !== '--resume' &&
        option !== '--approval-port' &&
        option !== '--trigger' &&
        option !== '--color' &&
        (command !== 'run' || (option !== '--host' && option !== '--port')))
    ) {
      throw new CliInputError(
        'WOML_CLI_ARGUMENTS_INVALID',
        command === 'test' ? testUsage() : runUsage()
      );
    }
    if (colorAssignment === undefined) index += 1;
    if (option === '--project') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--project requires a non-empty manifest path.'
        );
      }
      projectPath = resolve(value);
      projectSelection = 'explicit';
    } else if (option === '--state') {
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
    } else if (option === '--process-policy') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--process-policy requires a non-empty JSON file path.'
        );
      }
      processPolicyPath = value;
    } else if (option === '--artifact-policy') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--artifact-policy requires a non-empty JSON file path.'
        );
      }
      artifactPolicyPath = value;
    } else if (option === '--reviewer-directory') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--reviewer-directory requires a non-empty JSON file path.'
        );
      }
      reviewerDirectoryPath = value;
    } else if (option === '--target-registry') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--target-registry requires a non-empty JSON file path.'
        );
      }
      targetRegistryPath = value;
    } else if (option === '--target-trust') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--target-trust requires a non-empty JSON file path.'
        );
      }
      targetTrustPath = value;
    } else if (option === '--policy') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--policy requires a policy name or JSON file path.'
        );
      }
      infrastructurePolicyReference = value;
    } else if (option === '--emergency-deny') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--emergency-deny requires a policy name or JSON file path.'
        );
      }
      emergencyDenyReference = value;
    } else if (option === '--policy-trust') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--policy-trust requires a JSON file path.'
        );
      }
      infrastructurePolicyTrustPath = value;
    } else if (option === '--allow-process') {
      if (value.length === 0) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--allow-process requires a comma-separated executable alias list.'
        );
      }
      allowProcess = value;
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
    } else if (option === '--color') {
      if (command !== 'run' || !['auto', 'always', 'never'].includes(value)) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--color must be auto, always, or never.'
        );
      }
      color = value as ColorMode;
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
  if (inputPaths.length > 0 && projectPath !== undefined) {
    throw new CliInputError(
      'WOML_PROJECT_OPTION_CONFLICT',
      'Use positional workflow inputs or --project, not both.'
    );
  }
  if (filePath === undefined && projectPath === undefined) {
    const discovered = resolve('woml.project.json');
    if (!existsSync(discovered)) {
      throw new CliInputError(
        'WOML_CLI_ARGUMENTS_INVALID',
        command === 'test' ? testUsage() : runUsage()
      );
    }
    projectPath = discovered;
    projectSelection = 'current-directory';
  }
  filePath = filePath ?? projectPath!;
  if (
    resumeRunId !== undefined &&
    projectPath === undefined &&
    inputPaths.length !== 1
  ) {
    throw new CliInputError(
      'WOML_RESUME_REQUIRES_SINGLE_WORKFLOW',
      '--resume requires exactly one workflow file, not multiple inputs.'
    );
  }
  if (processPolicyPath !== undefined && allowProcess !== undefined) {
    throw new CliInputError(
      'WOML_PROCESS_GRANT_CONFLICT',
      'Use either --process-policy or --allow-process, not both.'
    );
  }
  if (targetRegistryPath === undefined && targetTrustPath !== undefined) {
    throw new CliInputError(
      'WOML_TARGET_REGISTRY_REQUIRED',
      '--target-trust requires --target-registry.'
    );
  }
  if (
    emergencyDenyReference !== undefined &&
    infrastructurePolicyReference === undefined &&
    projectPath === undefined
  ) {
    throw new CliInputError(
      'WOML_POLICY_REQUIRED',
      '--emergency-deny requires a base --policy so the overlay can only remove admitted authority.'
    );
  }
  if (
    infrastructurePolicyReference === undefined &&
    projectPath === undefined &&
    (requireInfrastructurePolicySignature ||
      infrastructurePolicyTrustPath !== undefined)
  ) {
    throw new CliInputError(
      'WOML_POLICY_REQUIRED',
      '--require-policy-signature and --policy-trust require a base --policy.'
    );
  }
  return {
    command,
    filePath,
    inputPaths,
    ...(projectPath === undefined ? {} : { projectPath }),
    ...(projectSelection === undefined ? {} : { projectSelection }),
    statePath,
    ...(resumeRunId === undefined ? {} : { resumeRunId }),
    approvalPort,
    host,
    port,
    ...(triggerId === undefined ? {} : { triggerId }),
    ...(configPath === undefined ? {} : { configPath }),
    ...(processPolicyPath === undefined ? {} : { processPolicyPath }),
    ...(artifactPolicyPath === undefined ? {} : { artifactPolicyPath }),
    ...(reviewerDirectoryPath === undefined ? {} : { reviewerDirectoryPath }),
    ...(targetRegistryPath === undefined ? {} : { targetRegistryPath }),
    ...(targetTrustPath === undefined ? {} : { targetTrustPath }),
    ...(infrastructurePolicyReference === undefined
      ? {}
      : { infrastructurePolicyReference }),
    ...(emergencyDenyReference === undefined ? {} : { emergencyDenyReference }),
    ...(infrastructurePolicyTrustPath === undefined
      ? {}
      : { infrastructurePolicyTrustPath }),
    requireInfrastructurePolicySignature,
    ...(allowProcess === undefined ? {} : { allowProcess }),
    stateExplicit,
    hostExplicit,
    portExplicit,
    adminHost: '127.0.0.1',
    adminPort: 3_001,
    shutdownTimeoutMs: 30_000,
    logDirectory: resolve('.woml/logs'),
    logFormat: 'text',
    observabilityHealth: true,
    observabilityMetrics: true,
    verbose,
    json,
    color,
  };
}

function explicitProjectReferences(
  args: Pick<
    RunArguments,
    | 'configPath'
    | 'processPolicyPath'
    | 'artifactPolicyPath'
    | 'reviewerDirectoryPath'
    | 'targetRegistryPath'
    | 'targetTrustPath'
    | 'infrastructurePolicyReference'
    | 'emergencyDenyReference'
    | 'infrastructurePolicyTrustPath'
    | 'requireInfrastructurePolicySignature'
    | 'allowProcess'
  >
): Readonly<Partial<Record<WomlProjectReferenceKey, string | true>>> {
  return {
    ...(args.configPath === undefined ? {} : { runtime: args.configPath }),
    ...(args.processPolicyPath === undefined && args.allowProcess === undefined
      ? {}
      : { processPolicy: args.processPolicyPath ?? true }),
    ...(args.artifactPolicyPath === undefined
      ? {}
      : { artifactPolicy: args.artifactPolicyPath }),
    ...(args.reviewerDirectoryPath === undefined
      ? {}
      : { reviewerDirectory: args.reviewerDirectoryPath }),
    ...(args.targetRegistryPath === undefined
      ? {}
      : { targetRegistry: args.targetRegistryPath }),
    ...(args.targetTrustPath === undefined
      ? {}
      : { targetTrust: args.targetTrustPath }),
    ...(args.infrastructurePolicyReference === undefined &&
    args.emergencyDenyReference === undefined &&
    args.infrastructurePolicyTrustPath === undefined &&
    !args.requireInfrastructurePolicySignature
      ? {}
      : {
          infrastructurePolicy:
            args.infrastructurePolicyReference ?? true,
        }),
    ...(args.infrastructurePolicyTrustPath === undefined
      ? {}
      : { policyTrust: args.infrastructurePolicyTrustPath }),
    ...(args.emergencyDenyReference === undefined
      ? {}
      : { emergencyDeny: args.emergencyDenyReference }),
  };
}

async function resolveRunProjectArguments(
  args: RunArguments,
  background: boolean
): Promise<RunArguments> {
  if (args.projectPath === undefined) return args;
  const project = await resolveWomlProject(args.projectPath, {
    mode: background ? 'background' : 'foreground',
    explicitReferences: explicitProjectReferences(args),
  });
  if (
    args.infrastructurePolicyReference === undefined &&
    project.references.infrastructurePolicy === undefined &&
    (args.emergencyDenyReference !== undefined ||
      args.infrastructurePolicyTrustPath !== undefined ||
      args.requireInfrastructurePolicySignature)
  ) {
    throw new CliInputError(
      'WOML_POLICY_REQUIRED',
      '--emergency-deny, --policy-trust, and --require-policy-signature require a base infrastructure policy.'
    );
  }
  if (args.command === 'test' && project.workflows.length !== 1) {
    throw new CliInputError(
      'WOML_TEST_REQUIRES_FILE',
      'woml test --project requires a project containing exactly one workflow file.'
    );
  }
  if (args.resumeRunId !== undefined && project.workflows.length !== 1) {
    throw new CliInputError(
      'WOML_RESUME_REQUIRES_SINGLE_WORKFLOW',
      '--resume with --project requires a project containing exactly one workflow file.'
    );
  }
  const inputPaths = project.workflows.map(workflow => workflow.path);
  return {
    ...args,
    project,
    inputPaths,
    filePath: inputPaths[0]!,
    ...(project.references.runtime === undefined
      ? {}
      : { configPath: project.references.runtime.path }),
    ...(project.references.processPolicy === undefined
      ? {}
      : { processPolicyPath: project.references.processPolicy.path }),
    ...(project.references.artifactPolicy === undefined
      ? {}
      : { artifactPolicyPath: project.references.artifactPolicy.path }),
    ...(project.references.reviewerDirectory === undefined
      ? {}
      : { reviewerDirectoryPath: project.references.reviewerDirectory.path }),
    ...(project.references.targetRegistry === undefined
      ? {}
      : { targetRegistryPath: project.references.targetRegistry.path }),
    ...(project.references.targetTrust === undefined
      ? {}
      : { targetTrustPath: project.references.targetTrust.path }),
    ...(project.references.infrastructurePolicy === undefined
      ? {}
      : {
          infrastructurePolicyReference:
            project.references.infrastructurePolicy.path,
        }),
    ...(project.references.emergencyDeny === undefined
      ? {}
      : { emergencyDenyReference: project.references.emergencyDeny.path }),
    ...(project.references.policyTrust === undefined
      ? {}
      : { infrastructurePolicyTrustPath: project.references.policyTrust.path }),
  };
}

async function resolveRunArgumentsConfiguration(
  args: RunArguments,
  backgroundOverride?: boolean
): Promise<RunArguments> {
  const background =
    backgroundOverride ?? process.env.WOML_BACKGROUND_HANDOFF !== undefined;
  args = await resolveRunProjectArguments(args, background);
  if (background && args.allowProcess !== undefined) {
    throw new CliInputError(
      'WOML_PROCESS_FOREGROUND_GRANT_BACKGROUND_DENIED',
      '--allow-process is a temporary foreground grant. Use --process-policy for a background runtime.'
    );
  }
  const processCapabilities =
    args.project?.authorities.process ??
    (args.processPolicyPath !== undefined
      ? await loadProcessPolicy(
          args.processPolicyPath,
          background ? 'background' : 'foreground'
        )
      : args.allowProcess !== undefined
        ? await createForegroundProcessGrant(args.allowProcess)
        : undefined);
  const artifactCapabilities =
    args.project?.authorities.artifacts ??
    (args.artifactPolicyPath === undefined
      ? undefined
      : await loadArtifactPolicy(
          args.artifactPolicyPath,
          background ? 'background' : 'foreground'
        ));
  const reviewerCapabilities =
    args.project?.authorities.reviewers ??
    (args.reviewerDirectoryPath === undefined
      ? undefined
      : await loadReviewerDirectory(args.reviewerDirectoryPath));
  const targetRegistry =
    args.project?.authorities.targets ??
    (args.targetRegistryPath === undefined
      ? undefined
      : await loadTargetRegistry(
          args.targetRegistryPath,
          args.targetTrustPath ??
            (existsSync(resolve('.woml/target-trust.json'))
              ? resolve('.woml/target-trust.json')
              : undefined)
        ));
  const projectInfrastructureCapabilities = args.project?.authorities.infrastructure;
  const infrastructurePolicy =
    projectInfrastructureCapabilities !== undefined
      ? undefined
      :
    args.infrastructurePolicyReference === undefined
      ? undefined
      : await resolveInfrastructurePolicy(args.infrastructurePolicyReference, {
          ...(args.infrastructurePolicyTrustPath === undefined
            ? {}
            : { trustPath: args.infrastructurePolicyTrustPath }),
          requireSignature: args.requireInfrastructurePolicySignature,
        });
  const emergencyDeny =
    args.emergencyDenyReference === undefined
      ? undefined
      : await resolveInfrastructurePolicy(args.emergencyDenyReference, {
          ...(args.infrastructurePolicyTrustPath === undefined
            ? {}
            : { trustPath: args.infrastructurePolicyTrustPath }),
          requireSignature: args.requireInfrastructurePolicySignature,
          emergencyDeny: true,
        });
  const infrastructurePolicyCapabilities =
    projectInfrastructureCapabilities ??
    (infrastructurePolicy === undefined
      ? undefined
      : infrastructurePolicyRuntimeAuthority(
          infrastructurePolicy,
          emergencyDeny
        ));
  const runtimeCapabilities: RuntimeCapabilitiesV1 | undefined =
    processCapabilities === undefined &&
    artifactCapabilities === undefined &&
    reviewerCapabilities === undefined &&
    infrastructurePolicyCapabilities === undefined
      ? undefined
      : {
          ...processCapabilities,
          ...artifactCapabilities,
          ...reviewerCapabilities,
          ...infrastructurePolicyCapabilities,
        };
  if (args.command !== 'run') {
    return {
      ...args,
      ...(targetRegistry === undefined ? {} : { targetRegistry }),
      ...(runtimeCapabilities === undefined ? {} : { runtimeCapabilities }),
    };
  }
  const configuration = await resolveRuntimeConfiguration(
    args.configPath,
    {
      ...(args.stateExplicit ? { statePath: args.statePath } : {}),
      ...(args.hostExplicit ? { publicHost: args.host } : {}),
      ...(args.portExplicit ? { publicPort: args.port } : {}),
    },
    {},
    args.project?.root
  );
  if (args.configPath !== undefined) {
    await preflightRuntimeConfiguration(configuration);
  }
  return {
    ...args,
    ...(targetRegistry === undefined ? {} : { targetRegistry }),
    statePath: configuration.statePath,
    host: configuration.public.host,
    port: configuration.public.port,
    adminHost: configuration.admin.host,
    adminPort: configuration.admin.port,
    shutdownTimeoutMs: configuration.shutdownTimeoutMs,
    logDirectory: configuration.logging.directory,
    logFormat: configuration.logging.format,
    observabilityHealth: configuration.observability.health,
    observabilityMetrics: configuration.observability.metrics,
    ...(configuration.schemaVersion === 2
      ? { openTelemetry: configuration.observability.openTelemetry }
      : {}),
    ...(configuration.retention === undefined
      ? {}
      : { retention: configuration.retention }),
    ...(runtimeCapabilities === undefined ? {} : { runtimeCapabilities }),
  };
}

interface RunGetArguments {
  readonly runId: string;
  readonly statePath: string;
  readonly json: boolean;
}

interface RecoverArguments {
  readonly runId: string;
  readonly action: 'retry' | 'externally_resolve';
  readonly actionId: string;
  readonly note?: string;
  readonly statePath: string;
  readonly json: boolean;
}

interface ArtifactCommandArguments {
  readonly operation: 'inspect' | 'export';
  readonly artifactId: string;
  readonly statePath: string;
  readonly json: boolean;
  readonly outputPath?: string;
  readonly artifactPolicyPath?: string;
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

interface SignalArguments {
  readonly signal: string;
  readonly runId: string;
  readonly waitId: string;
  readonly idempotencyKey: string;
  readonly correlation: string;
  readonly payloadPath: string;
  readonly statePath: string;
}

const signalNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/;

function parseSignalArguments(args: readonly string[]): SignalArguments {
  const [, signal, ...options] = args;
  if (
    signal === undefined ||
    signal.length > 128 ||
    !signalNamePattern.test(signal) ||
    options.length < 10 ||
    options.length > 12 ||
    options.length % 2 !== 0
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', signalUsage());
  }
  const values = new Map<string, string>();
  for (let index = 0; index < options.length; index += 2) {
    const option = options[index]!;
    const value = options[index + 1];
    if (
      value === undefined ||
      values.has(option) ||
      ![
        '--run',
        '--wait',
        '--id',
        '--correlation',
        '--payload',
        '--state',
      ].includes(option)
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', signalUsage());
    }
    values.set(option, value);
  }
  const runId = values.get('--run');
  const waitId = values.get('--wait');
  const idempotencyKey = values.get('--id');
  const correlation = values.get('--correlation');
  const payloadPath = values.get('--payload');
  if (
    runId === undefined ||
    runId.length > 256 ||
    waitId === undefined ||
    waitId.length > 128 ||
    idempotencyKey === undefined ||
    idempotencyKey.length > 256 ||
    correlation === undefined ||
    correlation.length > 256 ||
    payloadPath === undefined ||
    [runId, waitId, idempotencyKey, correlation, payloadPath].some(
      value => value.length === 0
    )
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', signalUsage());
  }
  return {
    signal,
    runId,
    waitId,
    idempotencyKey,
    correlation,
    payloadPath: resolve(payloadPath),
    statePath: resolve(values.get('--state') ?? '.woml/state.sqlite'),
  };
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

function parseRecoverArguments(args: readonly string[]): RecoverArguments {
  const [, runId, ...options] = args;
  if (
    runId === undefined ||
    runId.length === 0 ||
    runId.length > 320 ||
    runId.startsWith('--')
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', recoverUsage());
  }
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  let action: RecoverArguments['action'] | undefined;
  let actionId: string | undefined;
  let note: string | undefined;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index]!;
    if (seen.has(option)) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', recoverUsage());
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = options[++index];
    if (
      value === undefined ||
      value.startsWith('--') ||
      !['--state', '--retry', '--resolve', '--note'].includes(option)
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', recoverUsage());
    }
    if (option === '--state') statePath = resolve(value);
    if (option === '--retry') {
      action = 'retry';
      actionId = value;
    }
    if (option === '--resolve') {
      action = 'externally_resolve';
      actionId = value;
    }
    if (option === '--note') note = value;
  }
  if (
    action === undefined ||
    actionId === undefined ||
    actionId.length === 0 ||
    actionId.length > 768 ||
    (action === 'externally_resolve' &&
      (note === undefined || note.length === 0 || note.length > 1000)) ||
    (action === 'retry' && note !== undefined) ||
    (seen.has('--retry') && seen.has('--resolve'))
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', recoverUsage());
  }
  return {
    runId,
    action,
    actionId,
    ...(note === undefined ? {} : { note }),
    statePath,
    json,
  };
}

function parseArtifactCommandArguments(
  args: readonly string[]
): ArtifactCommandArguments {
  const [, operation, artifactId, ...options] = args;
  if (
    args[0] !== 'artifacts' ||
    !['inspect', 'export'].includes(String(operation)) ||
    artifactId === undefined ||
    !/^artifact_[0-9a-hjkmnp-tv-z]{26}$/.test(artifactId)
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', artifactsUsage());
  }
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  let outputPath: string | undefined;
  let artifactPolicyPath: string | undefined;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index]!;
    if (seen.has(option)) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', artifactsUsage());
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = options[++index];
    if (
      value === undefined ||
      value.startsWith('--') ||
      !['--state', '--output', '--artifact-policy'].includes(option)
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', artifactsUsage());
    }
    if (option === '--state') statePath = resolve(value);
    if (option === '--output') outputPath = resolve(value);
    if (option === '--artifact-policy') artifactPolicyPath = resolve(value);
  }
  if (
    (operation === 'inspect' &&
      (outputPath !== undefined || artifactPolicyPath !== undefined)) ||
    (operation === 'export' &&
      (outputPath === undefined || artifactPolicyPath === undefined))
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', artifactsUsage());
  }
  return {
    operation: operation as 'inspect' | 'export',
    artifactId,
    statePath,
    json,
    ...(outputPath === undefined ? {} : { outputPath }),
    ...(artifactPolicyPath === undefined ? {} : { artifactPolicyPath }),
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

function formatArtifactInspection(
  inspection: RustArtifactInspectionV1
): string {
  return [
    `Artifact: ${inspection.artifact.id}`,
    `Name: ${inspection.artifact.name}`,
    `Status: ${inspection.status}`,
    `Size: ${inspection.artifact.size} bytes`,
    `Digest: ${inspection.digestPrefix}…`,
    `Content type: ${inspection.artifact.contentType}`,
    `Retention: ${inspection.retentionMs} ms (expires ${inspection.expiresAt})`,
    `Provenance: ${inspection.provenance.workflowId} / ${inspection.provenance.runId} / ${inspection.provenance.nodeId} / attempt ${inspection.provenance.attempt}`,
  ].join('\n');
}

function formatRunInspection(
  inspection: RustRunInspectionV2,
  artifacts?: readonly RustArtifactInspectionV1[],
  artifactsTruncated = false,
  waitInspection?: RustWaitInspectionV1,
  locks: readonly RustLockInspectionV1[] = [],
  correction?: RustCorrectionInspectionV1,
  operationalOutput?: {
    readonly state: 'available' | 'unavailable';
    readonly latestSequence?: number;
    readonly activeProgress?: number;
    readonly progressLanes?: number;
  },
  statePath = '.woml/state.sqlite'
): string {
  const combined = createCombinedRunPresentation({
    inspection,
    statePath,
    ...(waitInspection === undefined ? {} : { waits: waitInspection }),
    ...(locks.length === 0 ? {} : { locks }),
    ...(correction === undefined ? {} : { correction }),
    ...(artifacts === undefined ? {} : { artifacts }),
  });
  const lines = [
    `Run: ${inspection.runId}`,
    `Workflow: ${inspection.workflowId}`,
    `Status: ${inspection.status}`,
    `Current: ${combined.state.replaceAll('-', ' ')} · ${combined.detail}`,
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
  if (inspection.infrastructurePolicy !== undefined) {
    const policy = inspection.infrastructurePolicy;
    lines.push(
      `Infrastructure policy: ${policy.binding.policyId}@${policy.binding.policyVersion} · ${policy.binding.policyDigest}`
    );
    lines.push(`Policy admitted at: ${policy.binding.admittedAt}`);
    lines.push(
      `Policy decisions: ${policy.totalDecisions}${policy.truncated ? ' (showing latest 100)' : ''}`
    );
    for (const decision of policy.decisions.slice(0, 10)) {
      const operation =
        typeof decision.operation === 'object' && decision.operation !== null
          ? (decision.operation as Record<string, unknown>)
          : undefined;
      lines.push(
        `  ${String(decision.outcome ?? 'unknown')}: ${String(operation?.capability ?? 'unknown')}.${String(operation?.operation ?? 'unknown')} · ${String(decision.decisionId ?? 'unknown')}`
      );
      const overlay =
        typeof decision.emergencyDeny === 'object' &&
        decision.emergencyDeny !== null
          ? (decision.emergencyDeny as Record<string, unknown>)
          : undefined;
      if (overlay !== undefined) {
        lines.push(
          `    Emergency deny: ${String(overlay.policyId ?? 'unknown')} · ${String(overlay.policyDigest ?? 'unknown')}`
        );
      }
    }
  }
  if (inspection.forEach !== undefined && inspection.forEach.items.length > 0) {
    lines.push('For each:');
    for (const loop of inspection.forEach.items) {
      const completed = loop.succeeded + loop.failed + loop.skipped;
      const progress = `${completed}/${loop.total} completed, ${loop.active} active, ${loop.pending} pending`;
      const failure =
        loop.failedIndex === undefined
          ? ''
          : `, failed item ${loop.failedIndex + 1} (index ${loop.failedIndex})${loop.failedNodeId === undefined ? '' : ` at ${loop.failedNodeId}`}`;
      lines.push(
        `  ${loop.forEachId}: ${loop.status} (${progress}, concurrency ${loop.concurrency}${failure})`
      );
    }
  }
  if (waitInspection !== undefined && waitInspection.waits.length > 0) {
    lines.push('Durable waits:');
    for (const wait of waitInspection.waits) {
      const signals =
        wait.signals === undefined || wait.signals.length === 0
          ? ''
          : ` · accepts ${wait.signals.join(', ')}`;
      const deadline =
        wait.deadline === undefined ? '' : ` · deadline ${wait.deadline}`;
      lines.push(
        `  ${wait.waitId}: ${wait.status} · ${wait.name}${signals}${deadline}`
      );
    }
    if (waitInspection.pendingWaits > 0) {
      lines.push(
        `Pending waits: ${waitInspection.pendingWaits}${waitInspection.oldestPendingWaitAgeMs === undefined ? '' : ` · oldest ${waitInspection.oldestPendingWaitAgeMs} ms`}`
      );
      lines.push(`Cancel safely: woml cancel ${inspection.runId}`);
    }
  }
  if (locks.length > 0) {
    lines.push('Locks:');
    for (const lock of locks) {
      const timing =
        lock.status === 'waiting'
          ? lock.waitedMs === undefined
            ? ''
            : ` · waiting ${lock.waitedMs} ms`
          : lock.heldMs === undefined
            ? ''
            : ` · held ${lock.heldMs} ms`;
      const fence =
        lock.fencingToken === undefined ? '' : ` · fence ${lock.fencingToken}`;
      lines.push(`  ${lock.lockId}: ${lock.status}${timing}${fence}`);
      lines.push(`    Resource: ${lock.key}`);
      if (lock.deadline !== undefined && lock.status === 'waiting') {
        lines.push(`    Acquisition deadline: ${lock.deadline}`);
      }
      if (lock.expiresAt !== undefined && lock.status === 'held') {
        lines.push(`    Lease expires: ${lock.expiresAt}`);
      }
      if (lock.failureCode !== undefined) {
        lines.push(`    Failure: ${lock.failureCode}`);
      }
    }
    if (
      locks.some(lock => lock.status === 'waiting' || lock.status === 'held')
    ) {
      lines.push(`Cancel safely: woml cancel ${inspection.runId}`);
      lines.push('WOML intentionally provides no force-unlock command.');
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
  if (
    inspection.approvals !== undefined &&
    inspection.approvals.items.length > 0
  ) {
    lines.push('Rich approvals:');
    for (const approval of inspection.approvals.items) {
      const review = approval.review as {
        readonly title?: unknown;
        readonly risk?: unknown;
        readonly artifacts?: unknown;
      };
      const title =
        typeof review.title === 'string' ? review.title : approval.approvalId;
      const risk = typeof review.risk === 'string' ? review.risk : 'unknown';
      lines.push(
        `  ${approval.approvalId}: ${approval.status} · ${approval.received}/${approval.required} approvals · risk ${risk}`
      );
      lines.push(`    Review: ${title}`);
      lines.push(`    Digest: ${approval.reviewDigest.slice(0, 19)}…`);
      if (approval.requiredRoles.length > 0) {
        lines.push(`    Required roles: ${approval.requiredRoles.join(', ')}`);
        lines.push(
          `    Represented roles: ${approval.representedRoles.length === 0 ? 'none' : approval.representedRoles.join(', ')}`
        );
      }
      lines.push(`    Reviewer directory: ${approval.directoryVersion}`);
      if (approval.deadline !== undefined)
        lines.push(`    Deadline: ${approval.deadline}`);
      if (Array.isArray(review.artifacts) && review.artifacts.length > 0) {
        lines.push(`    Reviewed artifacts: ${review.artifacts.length}`);
      }
      for (const decision of approval.decisions) {
        const roles =
          decision.roles.length === 0 ? '' : ` · ${decision.roles.join(', ')}`;
        lines.push(
          `    ${decision.decision}: ${decision.principalId} via ${decision.provider}${roles} · ${decision.decidedAt}`
        );
      }
      if (approval.decisionsTruncated) {
        lines.push(
          `    ${approval.totalDecisions - approval.decisions.length} additional decision(s) omitted.`
        );
      }
    }
    if (inspection.approvals.truncated) {
      lines.push(
        '  Additional rich approvals were omitted from this bounded view.'
      );
    }
  }
  if (artifacts !== undefined && artifacts.length > 0) {
    lines.push('Artifacts:');
    for (const artifact of artifacts) {
      lines.push(
        `  ${artifact.artifact.name}: ${artifact.artifact.size} bytes · ${artifact.digestPrefix}… · ${artifact.status} · expires ${artifact.expiresAt}`
      );
      lines.push(
        `    ${artifact.artifact.id} · ${artifact.provenance.nodeId} attempt ${artifact.provenance.attempt}`
      );
    }
    if (artifactsTruncated)
      lines.push('  Additional artifacts were omitted from this bounded view.');
  }
  if (correction !== undefined) {
    lines.push('Correction:');
    lines.push(
      `  ${correction.phase.replaceAll('_', ' ')} · original outcome ${correction.originalOutcome}`
    );
    for (const saga of correction.sagas) {
      lines.push(
        `  ${saga.sagaId}: ${saga.status.replaceAll('_', ' ')} · ${saga.completedCompensations}/${saga.registeredCompensations} compensated · ${saga.completedCleanup} cleanup completed`
      );
    }
    for (const action of correction.unresolvedActions) {
      lines.push(
        `  ${action.actionId}: ${action.kind} · ${action.code}${action.ambiguous ? ' · ambiguous' : ''}`
      );
      if (action.ambiguous) {
        lines.push(
          '    Verify the external effect before choosing a recovery action.'
        );
        lines.push(
          `    Retry only if it did not occur: woml recover ${inspection.runId} --retry ${JSON.stringify(action.actionId)}`
        );
        lines.push(
          `    Resolve after external repair: woml recover ${inspection.runId} --resolve ${JSON.stringify(action.actionId)} --note ${JSON.stringify('ticket or change reference')}`
        );
      }
    }
    if (correction.truncated) {
      lines.push(
        '  Additional unresolved actions were omitted from this bounded view.'
      );
    }
  }
  if (operationalOutput !== undefined) {
    lines.push('Operational output:');
    if (operationalOutput.state === 'unavailable') {
      lines.push('  Temporarily unavailable; durable run state is unaffected.');
    } else if ((operationalOutput.latestSequence ?? 0) === 0) {
      lines.push('  No retained output.');
    } else {
      lines.push(
        `  Retained through sequence ${operationalOutput.latestSequence} · ${operationalOutput.activeProgress ?? 0}/${operationalOutput.progressLanes ?? 0} progress lanes active`
      );
      lines.push(`  View: woml logs ${inspection.runId} --no-follow`);
    }
  }
  lines.push('Next safe actions:');
  for (const command of combined.nextCommands) lines.push(`  ${command}`);
  return `${lines.join('\n')}\n`;
}

function operationalOutputPath(statePath: string): string {
  const name = basename(statePath);
  if (name === 'state.sqlite') return join(dirname(statePath), 'output.sqlite');
  const extension = extname(name);
  const stem = extension.length === 0 ? name : name.slice(0, -extension.length);
  return join(dirname(statePath), `${stem}.output.sqlite`);
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
  if (progress.type === 'for_each_progress') {
    const completed = progress.succeeded + progress.failed + progress.skipped;
    return `For each ${progress.forEachId}: ${completed}/${progress.total} completed, ${progress.active} active, ${progress.pending} pending (${progress.status}).`;
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
  const presentation = new ForegroundPresentation({
    io,
    render: presentationRenderOptions(args, io),
    verbose: args.verbose,
    inspectRun: () => {
      throw new Error('Execution progress does not inspect settled runs.');
    },
  });
  return progress => {
    if (args.command === 'test') {
      io.stderr(`${formatExecutionProgress(progress)}\n`);
    } else {
      presentation.execution(progress);
    }
    if (
      'type' in progress &&
      progress.type === 'step_retry_scheduled' &&
      !recoveryPrinted
    ) {
      recoveryPrinted = true;
      presentation.verbose(
        `Recovery: woml run ${JSON.stringify(args.filePath)} --state ${JSON.stringify(
          args.statePath
        )} --resume ${JSON.stringify(progress.runId)}`
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

  if (error instanceof ProcessPolicyError) {
    return `WOML process policy error [${error.code}]: ${error.message}`;
  }

  if (error instanceof ArtifactPolicyError) {
    return `WOML artifact policy error [${error.code}]: ${error.message}`;
  }
  if (error instanceof InfrastructurePolicyError) {
    return `WOML infrastructure policy error [${error.code}]: ${error.message}`;
  }
  if (error instanceof TargetRegistryError) {
    return `WOML target error [${error.code}]: ${error.message}`;
  }
  if (error instanceof RemoteWorkerError) {
    return `WOML worker error [${error.code}]: ${error.message}`;
  }
  if (error instanceof ReviewerDirectoryLoadError) {
    return `WOML approval authority error [${error.code}]: ${error.message}`;
  }
  if (error instanceof WomlProjectError) {
    return `WOML project error [${error.code}]: ${error.message}`;
  }
  if (error instanceof WomlProjectInitializationError) {
    return `WOML project initialization error [${error.code}]: ${error.message}`;
  }
  if (error instanceof ArtifactOperationError) {
    return `WOML artifact error [${error.code}]: ${error.message}`;
  }

  if (error instanceof RuntimeControlError) {
    return `WOML runtime error [${error.code}]: ${error.message}`;
  }

  if (error instanceof LogFollowError) {
    return `WOML logs error [${error.code}]: ${error.message}`;
  }

  if (error instanceof RunPresentationDecodeError) {
    return `WOML logs error [${error.code}]: ${error.message}`;
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
      const label =
        provider === 'slack'
          ? 'Slack'
          : provider === 'telegram'
            ? 'Telegram'
            : provider === 'discord'
              ? 'Discord'
              : provider === 'whatsapp'
                ? 'WhatsApp'
                : provider === 'custom'
                  ? 'Custom provider'
                  : provider;
      return `\n${label} notification to ${destination} failed [${failure.code}]: ${failure.message}`;
    })
    .join('');
}

function printNotificationWarnings(
  io: CliIo,
  diagnostics: NotificationJourneyDiagnostics,
  updates?: NotificationDispatchReport
): void {
  if (
    diagnostics.deliveryFailures.length === 0 &&
    (updates?.updatesFailed ?? 0) === 0
  ) {
    return;
  }
  const deliveryWarning =
    diagnostics.deliveryFailures.length === 0
      ? ''
      : `some approval notifications could not be delivered.${formatNotificationDeliveryFailures(
          diagnostics
        )}`;
  const updateWarning =
    (updates?.updatesFailed ?? 0) === 0
      ? ''
      : `${deliveryWarning.length === 0 ? '' : '\n'}${updates!.updatesFailed} delivered provider message${
          updates!.updatesFailed === 1 ? '' : 's'
        } could not be updated after the decision. The durable decision is still recorded.`;
  io.stderr(
    `\nWOML notification warning: ${deliveryWarning}${updateWarning}\n\n`
  );
}

async function readWorkflowUnprofiled(filePath: string): Promise<string> {
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

async function readWorkflow(filePath: string): Promise<string> {
  return await profileAsync(
    'compiler',
    'compiler.read_source',
    async measurements => {
      const source = await readWorkflowUnprofiled(filePath);
      if (measurements !== undefined) {
        measurements.counts.files = 1;
        measurements.bytes.source = Buffer.byteLength(source);
      }
      return source;
    }
  );
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

async function assertStableSourceSnapshotUnprofiled(
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
      if (sourceSnapshotDigest(snapshot.path, content) !== snapshot.digest) {
        throw new CliInputError(
          'WOML_SOURCE_CHANGED_DURING_ACTIVATION',
          `source "${snapshot.path}" changed while the deployment was being compiled; run the command again.`
        );
      }
    }
  }
}

export async function assertStableSourceSnapshot(
  sources: readonly {
    readonly sourceSnapshot: readonly SourceSnapshotEntry[];
  }[]
): Promise<void> {
  await profileAsync(
    'compiler',
    'compiler.verify_source_snapshot',
    async measurements => {
      await assertStableSourceSnapshotUnprofiled(sources);
      if (measurements !== undefined) {
        measurements.counts.sources = sources.reduce(
          (total, source) => total + source.sourceSnapshot.length,
          0
        );
      }
    }
  );
}

async function assertStableWorkflowInputSetUnprofiled(
  inputPaths: readonly string[],
  expectedPaths: readonly string[]
): Promise<void> {
  const current = new Set<string>();
  for (const inputPath of inputPaths) {
    for (const path of await workflowFilePaths(inputPath))
      current.add(resolve(path));
  }
  const expected = [
    ...new Set(expectedPaths.map(path => resolve(path))),
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

async function assertStableWorkflowInputSet(
  inputPaths: readonly string[],
  expectedPaths: readonly string[]
): Promise<void> {
  await profileAsync(
    'compiler',
    'compiler.verify_input_set',
    async measurements => {
      await assertStableWorkflowInputSetUnprofiled(inputPaths, expectedPaths);
      if (measurements !== undefined) {
        measurements.counts.inputs = inputPaths.length;
        measurements.counts.files = expectedPaths.length;
      }
    }
  );
}

function promoteForLifecycleAuthority(
  workflow: CompiledWorkflowDefinition
): CompiledWorkflowDefinition {
  if (
    workflow.schemaVersion === 11 ||
    workflow.schemaVersion === 12 ||
    workflow.schemaVersion === 13 ||
    workflow.schemaVersion === 14 ||
    workflow.schemaVersion === 15 ||
    workflow.schemaVersion === 16 ||
    workflow.schemaVersion === 17 ||
    workflow.schemaVersion === 18 ||
    workflow.schemaVersion === 19 ||
    workflow.schemaVersion === 20 ||
    workflow.schemaVersion === 21
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

function builtInCommunicationProviders(
  workflow: CompiledWorkflowDefinition
): readonly string[] {
  return workflow.schemaVersion === 15
    ? workflow.communication.providers.map(provider => provider.provider)
    : [];
}

function reusableCommunicationAliasDiagnostics(
  document: WomlSourceDocument,
  graph: WomlReusableDefinitionGraph
): readonly WomlAdvisoryDiagnostic[] {
  const providerAliases = new Set(
    graph.definitions
      .filter(definition => definition.kind === 'notification-provider')
      .map(definition => definition.alias)
  );
  return graph.root.imports.flatMap(imported =>
    imported.kind === 'reusable-definition' &&
    (imported.name === 'telegram' || imported.name === 'discord') &&
    providerAliases.has(imported.name)
      ? [
          {
            severity: 'warning' as const,
            code: 'WOML_BUILTIN_PROVIDER_SHADOWED',
            phase: 'validation' as const,
            message: `The imported notification provider <${imported.name}> shadows WOML's built-in ${imported.name === 'telegram' ? 'Telegram' : 'Discord'} notification tag in this workflow.`,
            file: document.file,
            location: imported.element.openTagSpan,
            hint: `WOML preserves this local provider. Rename the import only when you want the built-in <${imported.name} ${imported.name === 'telegram' ? 'chats' : 'channels'}="..."> contract.`,
          },
        ]
      : []
  );
}

function runtimeModulesFromPackage(
  definitionPackage:
    | WomlDefinitionPackageV3
    | WomlDefinitionPackageV5
    | WomlDefinitionPackageV7
    | WomlDefinitionPackageV9
    | WomlDefinitionPackageV10
    | WomlDefinitionPackageV11
    | WomlDefinitionPackageV12
    | WomlDefinitionPackageV13
    | WomlDefinitionPackageV14
    | WomlDefinitionPackageV15
    | WomlDefinitionPackageV16
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
  if (
    definitionPackage.schemaVersion !== 9 &&
    definitionPackage.schemaVersion !== 10 &&
    definitionPackage.schemaVersion !== 11 &&
    definitionPackage.schemaVersion !== 12 &&
    definitionPackage.schemaVersion !== 13 &&
    definitionPackage.schemaVersion !== 14 &&
    definitionPackage.schemaVersion !== 15 &&
    definitionPackage.schemaVersion !== 16
  )
    return modules;
  const seen = new Set<string>();
  const reusableArtifacts = (
    definitionPackage.workflow.model.reusableDefinitions ?? []
  )
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
  const lifecycleArtifacts = (
    definitionPackage.workflow.model.reusableDefinitions ?? []
  ).flatMap(definition => {
    const ownerId =
      definition.kind === 'step'
        ? definition.invocationId
        : definition.providerId;
    return [
      ...(definition.lifecycle?.onSuccess ?? []).map(actionId => ({
        actionId,
        hook: 'on-success',
      })),
      ...(definition.lifecycle?.onError ?? []).map(actionId => ({
        actionId,
        hook: 'on-error',
      })),
      ...(definition.lifecycle?.onComplete ?? []).map(actionId => ({
        actionId,
        hook: 'on-complete',
      })),
    ].map(({ actionId, hook }, indexWithinHook) => {
      const actionIndex = Number(actionId.slice(actionId.lastIndexOf(':') + 1));
      const artifact = definitionPackage.artifacts.find(
        item =>
          item.path ===
          `definitions/${ownerId}.${hook}.${Number.isInteger(actionIndex) ? actionIndex : indexWithinHook}.js`
      );
      if (artifact?.kind !== 'module-bundle') {
        throw new Error(
          `Reusable lifecycle artifact "${actionId}" is unavailable.`
        );
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
  return [...modules, ...reusableArtifacts, ...lifecycleArtifacts].sort(
    (left, right) => left.name.localeCompare(right.name)
  );
}

async function workflowFilePathsUnprofiled(
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

async function workflowFilePaths(
  inputPath: string
): Promise<readonly string[]> {
  return await profileAsync(
    'compiler',
    'compiler.discover_input',
    async measurements => {
      const paths = await workflowFilePathsUnprofiled(inputPath);
      if (measurements !== undefined) measurements.counts.files = paths.length;
      return paths;
    }
  );
}

async function compileWorkflowSourcesUnprofiled(
  inputPath: string,
  skipReusableDefinitions = false,
  io?: CliIo
): Promise<readonly CompiledWorkflowSource[]> {
  const compiled: CompiledWorkflowSource[] = [];
  for (const filePath of await workflowFilePaths(inputPath)) {
    const source = await readWorkflow(filePath);
    const document = profileSync(
      'compiler',
      'compiler.parse_markup',
      measurements => {
        if (measurements !== undefined)
          measurements.bytes.source = Buffer.byteLength(source);
        return parseWoml(source, { file: filePath });
      }
    );
    const projectRoot = moduleProjectRoot(filePath);
    const cacheOptions: CompiledWorkflowCacheOptions = {
      sourcePath: resolve(filePath),
      projectRoot: resolve(projectRoot),
      compilerIdentity: COMPILED_WORKFLOW_CACHE_COMPILER_IDENTITY,
    };
    const cached =
      process.env.WOML_DISABLE_COMPILED_CACHE === '1'
        ? undefined
        : await profileAsync(
            'compiler',
            'compiler.cache_lookup',
            async measurements => {
              const candidate = await readCompiledWorkflowCache(cacheOptions);
              if (candidate === undefined) {
                if (measurements !== undefined) measurements.counts.misses = 1;
                return undefined;
              }
              try {
                if (
                  compiledDefinitionHash(candidate.workflow) !==
                  candidate.definitionHash
                ) {
                  if (measurements !== undefined)
                    measurements.counts.misses = 1;
                  return undefined;
                }
              } catch {
                if (measurements !== undefined) measurements.counts.misses = 1;
                return undefined;
              }
              if (measurements !== undefined) {
                measurements.counts.hits = 1;
                measurements.counts.sources = candidate.sourceSnapshot.length;
                measurements.counts.modules = candidate.runtimeModules.length;
              }
              return candidate;
            }
          );
    if (cached !== undefined) {
      if (io !== undefined && cached.reusableEditorData !== undefined) {
        await profileAsync(
          'compiler',
          'compiler.refresh_reusable_editor_data',
          async measurements => {
            if (measurements !== undefined) {
              measurements.bytes.output = Buffer.byteLength(
                cached.reusableEditorData!
              );
            }
            await refreshReusableEditorData(
              filePath,
              cached.reusableEditorData!,
              io
            );
          }
        );
      }
      compiled.push({
        filePath,
        activationInputPaths: [resolve(filePath)],
        document,
        workflow: cached.workflow,
        definitionHash: cached.definitionHash,
        runtimeModules: cached.runtimeModules,
        sourceSnapshot: cached.sourceSnapshot,
        migrationDiagnostics: cached.migrationDiagnostics,
      });
      continue;
    }
    const reusableGraph = profileSync(
      'compiler',
      'compiler.resolve_reusable_graph',
      measurements => {
        const graph = resolveWomlReusableDefinitionGraph(document, {
          sourcePath: filePath,
          projectRoot,
        });
        if (measurements !== undefined) {
          measurements.counts.definitions = graph.definitions.length;
          measurements.counts.sources = graph.sources.length;
        }
        return graph;
      }
    );
    if (reusableGraph.root.kind !== 'workflow') {
      await profileAsync(
        'compiler',
        'compiler.validate_module_entrypoints',
        async measurements => {
          await validateReusableModuleEntrypoints(reusableGraph, projectRoot);
          if (measurements !== undefined) {
            measurements.counts.sources = reusableGraph.sources.length;
          }
        }
      );
      if (io !== undefined) {
        await profileAsync(
          'compiler',
          'compiler.refresh_reusable_editor_data',
          async measurements => {
            const content = generateWomlReusableCustomData(reusableGraph);
            if (measurements !== undefined)
              measurements.bytes.output = Buffer.byteLength(content);
            await refreshReusableEditorData(filePath, content, io);
          }
        );
      }
      if (skipReusableDefinitions) continue;
      assertWomlDocumentRunnable(document);
    }
    const reusableEditorData =
      reusableGraph.definitions.length === 0
        ? undefined
        : generateWomlReusableCustomData(reusableGraph);
    if (reusableGraph.definitions.length > 0) {
      if (io !== undefined) {
        await profileAsync(
          'compiler',
          'compiler.validate_module_entrypoints',
          async measurements => {
            await validateReusableModuleEntrypoints(reusableGraph, projectRoot);
            if (measurements !== undefined) {
              measurements.counts.sources = reusableGraph.sources.length;
            }
          }
        );
        profileSync('compiler', 'compiler.validate_reusable_workflow', () =>
          validateResolvedReusableWorkflow(document, reusableGraph)
        );
        await profileAsync(
          'compiler',
          'compiler.refresh_reusable_editor_data',
          async measurements => {
            if (measurements !== undefined) {
              measurements.bytes.output = Buffer.byteLength(
                reusableEditorData!
              );
            }
            await refreshReusableEditorData(filePath, reusableEditorData!, io);
          }
        );
      }
    }
    const inspected =
      reusableGraph.definitions.length === 0
        ? profileSync(
            'compiler',
            'compiler.build_definition_package',
            measurements => {
              const definitionPackage = buildWomlDefinitionPackage(document, {
                sourcePath: filePath,
                projectRoot,
              });
              if (measurements !== undefined) {
                measurements.counts.modules = definitionPackage.modules.length;
                measurements.counts.sources = definitionPackage.sources.length;
              }
              return definitionPackage;
            }
          )
        : undefined;
    if (inspected !== undefined && inspected.modules.length > 0) {
      profileSync('compiler', 'compiler.inspect_module_services', () =>
        inspectWomlModuleServiceUsage(document, {
          sourcePath: filePath,
          projectRoot,
        })
      );
    }
    const reusablePackage =
      reusableGraph.definitions.length > 0
        ? await profileAsync(
            'compiler',
            'compiler.build_reusable_package',
            async measurements => {
              const definitionPackage =
                await buildWomlReusableDefinitionPackage(
                  document,
                  reusableGraph,
                  { sourcePath: filePath, projectRoot }
                );
              if (measurements !== undefined) {
                measurements.counts.sources = definitionPackage.sources.length;
                measurements.counts.artifacts =
                  definitionPackage.artifacts.length;
              }
              return definitionPackage;
            }
          )
        : undefined;
    const executablePackage =
      reusablePackage ??
      (inspected!.modules.length > 0
        ? await profileAsync(
            'compiler',
            'compiler.build_executable_package',
            async measurements => {
              const definitionPackage =
                await buildWomlExecutableDefinitionPackage(document, {
                  sourcePath: filePath,
                  projectRoot,
                });
              if (measurements !== undefined) {
                measurements.counts.modules = definitionPackage.modules.length;
                measurements.counts.sources = definitionPackage.sources.length;
                measurements.counts.artifacts =
                  definitionPackage.artifacts.length;
              }
              return definitionPackage;
            }
          )
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
            ? await profileAsync(
                'compiler',
                'compiler.build_runtime_package',
                async measurements => {
                  const definitionPackage =
                    await buildWomlRuntimeDefinitionPackage(document, {
                      sourcePath: filePath,
                      projectRoot,
                    });
                  if (measurements !== undefined) {
                    measurements.counts.modules =
                      definitionPackage.modules.length;
                    measurements.counts.sources =
                      definitionPackage.sources.length;
                    measurements.counts.artifacts =
                      definitionPackage.artifacts.length;
                  }
                  return definitionPackage;
                }
              )
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
    const frontendWorkflow =
      runtimePackage?.workflow.model ??
      profileSync('compiler', 'compiler.lower_model', measurements => {
        const lowered = compileWoml(document);
        if (measurements !== undefined) {
          measurements.counts.nodes = lowered.graph.nodes.length;
          measurements.counts.edges = lowered.graph.edges.length;
        }
        return lowered;
      });
    const workflow = profileSync('compiler', 'compiler.promote_model', () =>
      promoteForLifecycleAuthority(frontendWorkflow)
    );
    const definitionHash = compiledDefinitionHash(workflow);
    const runtimeModules =
      runtimePackage === undefined
        ? []
        : profileSync(
            'compiler',
            'compiler.materialize_runtime_modules',
            measurements => {
              const modules = runtimeModulesFromPackage(runtimePackage);
              if (measurements !== undefined) {
                measurements.counts.modules = modules.length;
                measurements.bytes.bundles = modules.reduce(
                  (total, module) => total + Buffer.byteLength(module.bundle),
                  0
                );
                measurements.bytes.source_maps = modules.reduce(
                  (total, module) =>
                    total + Buffer.byteLength(module.sourceMap),
                  0
                );
              }
              return modules;
            }
          );
    const compiledSource: CompiledWorkflowSource = {
      filePath,
      activationInputPaths: [resolve(filePath)],
      document,
      workflow,
      definitionHash,
      runtimeModules,
      sourceSnapshot: packageSources.map(item => ({
        path: resolve(projectRoot, item.path),
        digest: item.digest,
      })),
      migrationDiagnostics:
        reusableGraph.definitions.length === 0
          ? inspectWomlMigrationDiagnostics(document)
          : reusableCommunicationAliasDiagnostics(document, reusableGraph),
    };
    compiled.push(compiledSource);
    if (process.env.WOML_DISABLE_COMPILED_CACHE !== '1') {
      await profileAsync(
        'compiler',
        'compiler.cache_store',
        async measurements => {
          const stored = await writeCompiledWorkflowCache(cacheOptions, {
            workflow: compiledSource.workflow,
            definitionHash: compiledSource.definitionHash,
            runtimeModules: compiledSource.runtimeModules,
            sourceSnapshot: compiledSource.sourceSnapshot,
            migrationDiagnostics: compiledSource.migrationDiagnostics,
            ...(reusableEditorData === undefined ? {} : { reusableEditorData }),
          });
          if (measurements !== undefined) {
            measurements.counts.stored = stored ? 1 : 0;
            measurements.counts.sources = compiledSource.sourceSnapshot.length;
            measurements.counts.modules = compiledSource.runtimeModules.length;
          }
        }
      );
    }
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

async function compileWorkflowSources(
  inputPath: string,
  skipReusableDefinitions = false,
  io?: CliIo
): Promise<readonly CompiledWorkflowSource[]> {
  return await profileAsync(
    'compiler',
    'compiler.compile_source',
    async measurements => {
      const compiled = await compileWorkflowSourcesUnprofiled(
        inputPath,
        skipReusableDefinitions,
        io
      );
      if (measurements !== undefined) {
        measurements.counts.workflows = compiled.length;
        measurements.counts.nodes = compiled.reduce(
          (total, source) => total + source.workflow.graph.nodes.length,
          0
        );
        measurements.counts.modules = compiled.reduce(
          (total, source) => total + source.runtimeModules.length,
          0
        );
      }
      return compiled;
    }
  );
}

async function compileWorkflowInputsUnprofiled(
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
  await profileAsync(
    'compiler',
    'compiler.snapshot_inputs',
    async measurements => {
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
      if (measurements !== undefined) {
        measurements.counts.inputs = inputPaths.length;
        measurements.counts.files = filePaths.length;
      }
    }
  );
  await profileAsync(
    'compiler',
    'compiler.compile_files',
    async measurements => {
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
      if (measurements !== undefined)
        measurements.counts.workflows = compiled.length;
    }
  );
  profileSync('compiler', 'compiler.validate_workflow_set', measurements => {
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
    if (measurements !== undefined)
      measurements.counts.workflows = workflowIds.size;
  });
  await assertStableSourceSnapshot(compiled);
  await profileAsync(
    'compiler',
    'compiler.verify_directory_snapshots',
    async measurements => {
      let directories = 0;
      for (const snapshot of inputSnapshots) {
        if (!snapshot.directory) continue;
        directories += 1;
        const current = (await workflowFilePaths(snapshot.inputPath)).map(
          path => resolve(path)
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
      if (measurements !== undefined)
        measurements.counts.directories = directories;
    }
  );
  const activationInputPaths = profileSync(
    'compiler',
    'compiler.materialize_activation_inputs',
    measurements => {
      const paths = inputSnapshots
        .flatMap(snapshot => snapshot.files)
        .map(path => resolve(path))
        .filter((path, index, values) => values.indexOf(path) === index)
        .sort((left, right) => left.localeCompare(right));
      if (measurements !== undefined) measurements.counts.files = paths.length;
      return paths;
    }
  );
  return compiled.map(source => ({ ...source, activationInputPaths }));
}

export async function compileWorkflowInputs(
  inputPaths: readonly string[],
  io?: CliIo
): Promise<readonly CompiledWorkflowSource[]> {
  return await profileAsync(
    'compiler',
    'compiler.compile_inputs',
    async measurements => {
      const compiled = await compileWorkflowInputsUnprofiled(inputPaths, io);
      if (measurements !== undefined) {
        measurements.counts.inputs = inputPaths.length;
        measurements.counts.workflows = compiled.length;
        measurements.counts.nodes = compiled.reduce(
          (total, source) => total + source.workflow.graph.nodes.length,
          0
        );
      }
      return compiled;
    }
  );
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

function operationalOutputDeploymentIdentity(statePath: string): string {
  const identitySource = existsSync(statePath)
    ? realpathSync.native(statePath)
    : resolve(statePath);
  return `deployment_${createHash('sha256')
    .update(identitySource)
    .digest('hex')}`;
}

function editorTypesPath(inputPath: string, inputIsDirectory: boolean): string {
  return join(
    inputIsDirectory ? inputPath : dirname(inputPath),
    'woml-env.d.ts'
  );
}

async function refreshEditorTypesUnprofiled(
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

async function refreshEditorTypes(
  inputPath: string,
  modules: readonly {
    readonly name: string;
    readonly exports: readonly string[];
  }[],
  io: CliIo
): Promise<void> {
  await profileAsync(
    'compiler',
    'compiler.refresh_editor_types',
    async measurements => {
      await refreshEditorTypesUnprofiled(inputPath, modules, io);
      if (measurements !== undefined)
        measurements.counts.modules = modules.length;
    }
  );
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
    const declaresForEach = (element: WomlSourceElement): boolean =>
      element.name === 'for-each' ||
      element.children.some(
        child => child.kind === 'element' && declaresForEach(child)
      );
    const hasForEach = declaresForEach(document.root);
    const declaresDurableWait = (element: WomlSourceElement): boolean =>
      element.name === 'wait' ||
      element.name === 'wait-until' ||
      element.name === 'wait-for' ||
      element.children.some(
        child => child.kind === 'element' && declaresDurableWait(child)
      );
    const hasDurableWait = declaresDurableWait(document.root);
    const declaresLock = (element: WomlSourceElement): boolean =>
      element.name === 'lock' ||
      element.children.some(
        child => child.kind === 'element' && declaresLock(child)
      );
    const hasLock = declaresLock(document.root);
    const declaresSaga = (element: WomlSourceElement): boolean =>
      element.name === 'saga' ||
      element.children.some(
        child => child.kind === 'element' && declaresSaga(child)
      );
    const hasSaga = declaresSaga(document.root);
    const declaresRemoteTarget = (element: WomlSourceElement): boolean =>
      (element.name === 'step' && element.attributes.target !== undefined) ||
      element.children.some(
        child => child.kind === 'element' && declaresRemoteTarget(child)
      );
    const hasRemoteTarget = declaresRemoteTarget(document.root);
    if (
      !hasRemoteTarget &&
      (reusableGraph.root.kind !== 'workflow' ||
        reusableGraph.definitions.length > 0)
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
          ? await buildWomlReusableDefinitionPackage(document, reusableGraph, {
              sourcePath: filePath,
              projectRoot,
            })
          : undefined;
      if (options[0] === '--json') {
        printMigrationDiagnostics(
          io,
          reusableCommunicationAliasDiagnostics(document, reusableGraph)
        );
        io.stdout(
          `${JSON.stringify(reusablePackage ?? reusableGraph, null, 2)}\n`
        );
        return 0;
      }
      printMigrationDiagnostics(
        io,
        reusableCommunicationAliasDiagnostics(document, reusableGraph)
      );
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
        reusablePackage?.schemaVersion === 16
          ? `Compiled Model v21 package: ${reusablePackage.rootHash}\nExecution: remote placement, authenticated sessions, durable dispatch authority, and the worker attempt runtime are validated; live dispatch transport completes in RET7.\n`
          : reusablePackage?.schemaVersion === 14
          ? `Compiled Model v19 package: ${reusablePackage.rootHash}\nExecution: compensation and cleanup authoring is validated; durable correction execution begins in CC2–CC4.\n`
          : reusablePackage?.schemaVersion === 13
            ? `Compiled Model v18 package: ${reusablePackage.rootHash}\nExecution: lock authoring and lowering are validated; the durable lock runtime begins in LL2–LL4.\n`
            : reusablePackage?.schemaVersion === 11
              ? `Compiled Model v16 package: ${reusablePackage.rootHash}\nExecution: <for-each>, modules, and reusable steps are runnable together.\n`
              : reusablePackage?.schemaVersion === 10 &&
                  reusablePackage.workflow.model.communication.providers.some(
                    provider => provider.provider === 'discord'
                  )
                ? `Compiled Model v15 package: ${reusablePackage.rootHash}\nExecution: Discord triggers, notifications, approval buttons, and services.discord.send() are executable through the durable Rust runtime.\n`
                : reusablePackage?.schemaVersion === 10
                  ? `Compiled Model v15 package: ${reusablePackage.rootHash}\nExecution: custom definitions and Telegram triggers, notifications, and messaging are runnable together.\n`
                  : reusablePackage !== undefined
                    ? `Compiled Model v14 package: ${reusablePackage.rootHash}\nExecution: custom steps and notification providers are runnable.\n`
                    : reusableGraph.root.kind === 'workflow'
                      ? 'Execution: reusable provider source is validated; custom notification providers begin in SCP5.\n'
                      : 'Execution: reusable definitions are imported by workflows and are not independently runnable.\n'
      );
      return 0;
    }
    if (hasRemoteTarget) {
      const inspectionPackage = buildWomlDefinitionPackage(document, {
        sourcePath: filePath,
        projectRoot,
      });
      const executablePackage =
        inspectionPackage.modules.length === 0
          ? undefined
          : await buildWomlExecutableDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            });
      const compiled =
        executablePackage?.workflow.model ?? compileWoml(document);
      if (compiled.schemaVersion !== 21) {
        throw new CliInputError(
          'WOML_REMOTE_EXECUTION_MODEL_INVALID',
          'A targeted <step> did not lower to the frozen compiled Model v21 contract.'
        );
      }
      if (executablePackage !== undefined) {
        await refreshEditorTypes(filePath, executablePackage.modules, io);
      }
      const placedNodes = [
        ...compiled.graph.nodes,
        ...compiled.graph.forEach.flatMap(loop => loop.body.nodes),
        ...compiled.graph.sagas.flatMap(saga => [
          ...saga.compensations.map(action => action.node),
          ...saga.cleanup.map(action => action.node),
        ]),
      ]
        .filter(node => node.execution !== undefined)
        .map(node => ({
          stepId: node.id,
          target: node.execution!.target,
        }));
      const workflowElement = document.root.children.find(
        (child): child is WomlSourceElement =>
          child.kind === 'element' && child.name === 'workflow'
      );
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(
            {
              profile: 'woml.model-validation/v1',
              valid: true,
              workflowId: workflowElement?.attributes.id?.value,
              feature: 'remote-execution-targets',
              executable: false,
              modelVersion: compiled.schemaVersion,
              packageVersion: executablePackage?.schemaVersion,
              runtimePhase: 'RET1',
              placements: placedNodes,
            },
            null,
            2
          )}\n`
        );
        return 0;
      }
      io.stdout(
        `WOML check passed for workflow "${workflowElement?.attributes.id?.value ?? filePath}".\n`
      );
      io.stdout(
        'Compiled Model v21 passed Remote Execution Targets frontend validation.\n'
      );
      for (const placement of placedNodes) {
        io.stdout(`Target: ${placement.stepId} -> ${placement.target}\n`);
      }
      io.stdout(
        'Execution: authenticated sessions, durable dispatch authority, and the worker attempt runtime are available; live dispatch transport completes in RET7.\n'
      );
      return 0;
    }
    if (hasSaga) {
      const inspectionPackage = buildWomlDefinitionPackage(document, {
        sourcePath: filePath,
        projectRoot,
      });
      const executablePackage =
        inspectionPackage.modules.length === 0
          ? undefined
          : await buildWomlExecutableDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            });
      const compiled =
        executablePackage?.workflow.model ?? compileWoml(document);
      if (compiled.schemaVersion !== 19) {
        throw new CliInputError(
          'WOML_CORRECTION_MODEL_INVALID',
          '<saga> did not lower to the frozen compiled Model v19 contract.'
        );
      }
      const workflowElement = document.root.children.find(
        (child): child is WomlSourceElement =>
          child.kind === 'element' && child.name === 'workflow'
      );
      if (executablePackage !== undefined) {
        await refreshEditorTypes(filePath, executablePackage.modules, io);
      }
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(
            {
              profile: 'woml.model-validation/v1',
              valid: true,
              workflowId: workflowElement?.attributes.id?.value,
              feature: 'compensation-and-cleanup',
              executable: false,
              modelVersion: compiled.schemaVersion,
              runtimePhase: 'CC1',
            },
            null,
            2
          )}\n`
        );
        return 0;
      }
      io.stdout(
        `WOML check passed for workflow "${workflowElement?.attributes.id?.value ?? filePath}".\n`
      );
      io.stdout(
        'Compiled Model v19 passed compensation and cleanup frontend validation.\n'
      );
      io.stdout('Execution: corrective runtime execution begins in CC2–CC4.\n');
      return 0;
    }
    if (hasLock) {
      const inspectionPackage = buildWomlDefinitionPackage(document, {
        sourcePath: filePath,
        projectRoot,
      });
      const executablePackage =
        inspectionPackage.modules.length === 0
          ? undefined
          : await buildWomlExecutableDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            });
      const compiled =
        executablePackage?.workflow.model ?? compileWoml(document);
      if (compiled.schemaVersion !== 18) {
        throw new CliInputError(
          'WOML_LOCK_MODEL_INVALID',
          '<lock> did not lower to the frozen compiled Model v18 contract.'
        );
      }
      const workflowElement = document.root.children.find(
        (child): child is WomlSourceElement =>
          child.kind === 'element' && child.name === 'workflow'
      );
      if (executablePackage !== undefined) {
        await refreshEditorTypes(filePath, executablePackage.modules, io);
      }
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(
            {
              profile: 'woml.model-validation/v1',
              valid: true,
              workflowId: workflowElement?.attributes.id?.value,
              feature: 'locks-and-leases',
              executable: true,
              modelVersion: compiled.schemaVersion,
              runtimePhase: 'LL3',
            },
            null,
            2
          )}\n`
        );
        return 0;
      }
      io.stdout(
        `WOML check passed for workflow "${workflowElement?.attributes.id?.value ?? filePath}".\n`
      );
      io.stdout(
        'Compiled Model v18 passed frontend and durable Rust lock validation.\n'
      );
      return 0;
    }
    if (hasDurableWait) {
      const inspectionPackage = buildWomlDefinitionPackage(document, {
        sourcePath: filePath,
        projectRoot,
      });
      const executablePackage =
        inspectionPackage.modules.length === 0
          ? undefined
          : await buildWomlExecutableDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            });
      const compiled =
        executablePackage?.workflow.model ?? compileWoml(document);
      if (compiled.schemaVersion !== 17) {
        throw new CliInputError(
          'WOML_WAIT_MODEL_INVALID',
          'Durable waits did not lower to the frozen compiled Model v17 contract.'
        );
      }
      const workflowElement = document.root.children.find(
        (child): child is WomlSourceElement =>
          child.kind === 'element' && child.name === 'workflow'
      );
      if (executablePackage !== undefined) {
        await refreshEditorTypes(filePath, executablePackage.modules, io);
      }
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(
            {
              profile: 'woml.model-validation/v1',
              valid: true,
              workflowId: workflowElement?.attributes.id?.value,
              feature: 'durable-waits',
              executable: true,
              modelVersion: compiled.schemaVersion,
              runtimePhase: 'DW5',
            },
            null,
            2
          )}\n`
        );
        return 0;
      }
      io.stdout(
        `WOML check passed for workflow "${workflowElement?.attributes.id?.value ?? filePath}".\n`
      );
      io.stdout(
        'Compiled Model v17 passed frontend and durable Rust wait validation.\n'
      );
      return 0;
    }
    if (hasForEach) {
      const compiled = compileWoml(document);
      if (compiled.schemaVersion !== 16) {
        throw new CliInputError(
          'WOML_FOR_EACH_MODEL_INVALID',
          '<for-each> did not lower to the frozen compiled Model v16 contract.'
        );
      }
      const workflowElement = document.root.children.find(
        (child): child is WomlSourceElement =>
          child.kind === 'element' && child.name === 'workflow'
      );
      if (options[0] === '--json') {
        io.stdout(
          `${JSON.stringify(
            {
              profile: 'woml.model-validation/v1',
              valid: true,
              workflowId: workflowElement?.attributes.id?.value,
              feature: 'for-each',
              executable: true,
              modelVersion: compiled.schemaVersion,
            },
            null,
            2
          )}\n`
        );
        return 0;
      }
      io.stdout(
        `WOML check passed for workflow "${workflowElement?.attributes.id?.value ?? filePath}".\n`
      );
      io.stdout(
        'Compiled Model v16 passed frontend validation and supports bounded concurrent Rust execution.\n'
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
    const executablePackage =
      inspectionPackage.modules.length === 0
        ? undefined
        : await buildWomlExecutableDefinitionPackage(document, {
            sourcePath: filePath,
            projectRoot,
          });
    const definitionPackage =
      inspectionPackage.modules.length === 0
        ? inspectionPackage
        : executablePackage?.schemaVersion === 10
          ? await buildWomlRuntimeDefinitionPackage(document, {
              sourcePath: filePath,
              projectRoot,
            })
          : declaresLifecycle || declaresRuntimePolicy
            ? executablePackage!
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
        compiledWorkflow.schemaVersion === 14 ||
        compiledWorkflow.schemaVersion === 15) &&
      compiledWorkflow.lifecycle !== undefined;
    const hasRuntimePolicy =
      compiledWorkflow.schemaVersion === 12 ||
      compiledWorkflow.schemaVersion === 13 ||
      compiledWorkflow.schemaVersion === 14 ||
      compiledWorkflow.schemaVersion === 15;
    const hasFork =
      (compiledWorkflow.schemaVersion === 13 ||
        compiledWorkflow.schemaVersion === 14 ||
        compiledWorkflow.schemaVersion === 15) &&
      compiledWorkflow.graph.forks.length > 0;
    const hasSwitch =
      (compiledWorkflow.schemaVersion === 14 ||
        compiledWorkflow.schemaVersion === 15) &&
      compiledWorkflow.graph.choices.some(
        choice => choice.stringSelector !== undefined
      );
    const communicationProviders =
      builtInCommunicationProviders(compiledWorkflow);
    const hasDiscord = communicationProviders.includes('discord');
    const hasTelegram = communicationProviders.includes('telegram');
    const hasWhatsApp = communicationProviders.includes('whatsapp');
    const workflowCallsFrontendOnly =
      compiledWorkflow.triggers.length === 0 ||
      usage.referencedServices.includes('workflows');
    io.stdout(
      hasWhatsApp
        ? `Execution: WhatsApp triggers, approved-template notifications, lifecycle delivery, approvals, and services.whatsapp.send() are executable through the durable Rust runtime.\nCallback: ${WOML_WHATSAPP_CALLBACK_PATH} (public HTTPS is required by Meta in production).\n`
        : hasDiscord
          ? 'Execution: Discord triggers, notifications, approval buttons, and services.discord.send() are executable through the durable Rust runtime.\n'
          : hasTelegram
            ? 'Execution: Telegram triggers, notifications, and services.telegram.send() are executable through the durable Rust runtime.\n'
            : hasSwitch
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
  readonly projectPath?: string;
  readonly projectSelection?: 'explicit' | 'current-directory';
  readonly configPath?: string;
  readonly processPolicyPath?: string;
  readonly artifactPolicyPath?: string;
  readonly reviewerDirectoryPath?: string;
  readonly targetRegistryPath?: string;
  readonly targetTrustPath?: string;
  readonly infrastructurePolicyReference?: string;
  readonly infrastructurePolicyTrustPath?: string;
  readonly requireInfrastructurePolicySignature: boolean;
  readonly json: boolean;
}

function parseCheckArguments(args: readonly string[]): CheckArguments {
  if (args[0] !== 'check') {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
  }
  const inputPaths: string[] = [];
  let configPath: string | undefined;
  let projectPath: string | undefined;
  let projectSelection: 'explicit' | 'current-directory' | undefined;
  let processPolicyPath: string | undefined;
  let artifactPolicyPath: string | undefined;
  let reviewerDirectoryPath: string | undefined;
  let targetRegistryPath: string | undefined;
  let targetTrustPath: string | undefined;
  let infrastructurePolicyReference: string | undefined;
  let infrastructurePolicyTrustPath: string | undefined;
  let requireInfrastructurePolicySignature = false;
  let json = false;
  for (let index = 1; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--json') {
      if (json)
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      json = true;
      continue;
    }
    if (argument === '--require-policy-signature') {
      if (requireInfrastructurePolicySignature)
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      requireInfrastructurePolicySignature = true;
      continue;
    }
    if (argument === '--project') {
      const value = args[index + 1];
      if (
        projectPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      projectPath = resolve(value);
      projectSelection = 'explicit';
      index += 1;
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
    if (argument === '--process-policy') {
      const value = args[index + 1];
      if (
        processPolicyPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      processPolicyPath = value;
      index += 1;
      continue;
    }
    if (argument === '--artifact-policy') {
      const value = args[index + 1];
      if (
        artifactPolicyPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      artifactPolicyPath = value;
      index += 1;
      continue;
    }
    if (argument === '--reviewer-directory') {
      const value = args[index + 1];
      if (
        reviewerDirectoryPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      reviewerDirectoryPath = value;
      index += 1;
      continue;
    }
    if (argument === '--target-registry') {
      const value = args[index + 1];
      if (
        targetRegistryPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      targetRegistryPath = value;
      index += 1;
      continue;
    }
    if (argument === '--target-trust') {
      const value = args[index + 1];
      if (
        targetTrustPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      targetTrustPath = value;
      index += 1;
      continue;
    }
    if (argument === '--policy') {
      const value = args[index + 1];
      if (
        infrastructurePolicyReference !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      infrastructurePolicyReference = value;
      index += 1;
      continue;
    }
    if (argument === '--policy-trust') {
      const value = args[index + 1];
      if (
        infrastructurePolicyTrustPath !== undefined ||
        value === undefined ||
        value.startsWith('--')
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
      }
      infrastructurePolicyTrustPath = value;
      index += 1;
      continue;
    }
    if (argument.startsWith('--')) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
    }
    inputPaths.push(resolve(argument));
  }
  if (inputPaths.length > 0 && projectPath !== undefined) {
    throw new CliInputError(
      'WOML_PROJECT_OPTION_CONFLICT',
      'Use positional workflow inputs or --project, not both.'
    );
  }
  if (inputPaths.length === 0 && projectPath === undefined) {
    const discovered = resolve('woml.project.json');
    if (!existsSync(discovered)) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', checkUsage());
    }
    projectPath = discovered;
    projectSelection = 'current-directory';
  }
  if (
    infrastructurePolicyReference === undefined &&
    projectPath === undefined &&
    (requireInfrastructurePolicySignature ||
      infrastructurePolicyTrustPath !== undefined)
  ) {
    throw new CliInputError(
      'WOML_POLICY_REQUIRED',
      '--require-policy-signature and --policy-trust require a base --policy.'
    );
  }
  if (targetRegistryPath === undefined && targetTrustPath !== undefined) {
    throw new CliInputError(
      'WOML_TARGET_REGISTRY_REQUIRED',
      '--target-trust requires --target-registry.'
    );
  }
  return {
    inputPaths,
    ...(projectPath === undefined ? {} : { projectPath }),
    ...(projectSelection === undefined ? {} : { projectSelection }),
    ...(configPath === undefined ? {} : { configPath }),
    ...(processPolicyPath === undefined ? {} : { processPolicyPath }),
    ...(artifactPolicyPath === undefined ? {} : { artifactPolicyPath }),
    ...(reviewerDirectoryPath === undefined
      ? {}
      : { reviewerDirectoryPath }),
    ...(targetRegistryPath === undefined ? {} : { targetRegistryPath }),
    ...(targetTrustPath === undefined ? {} : { targetTrustPath }),
    ...(infrastructurePolicyReference === undefined
      ? {}
      : { infrastructurePolicyReference }),
    ...(infrastructurePolicyTrustPath === undefined
      ? {}
      : { infrastructurePolicyTrustPath }),
    requireInfrastructurePolicySignature,
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

interface ProductionPreflightReport {
  readonly profile:
    | 'woml.production-preflight/v1'
    | 'woml.production-preflight/v2';
  readonly status: 'passed';
  readonly configuration?: ResolvedRuntimeConfiguration;
  readonly environment?: RuntimePreflightV1;
  readonly secretProvider?: string;
  readonly infrastructurePolicy?: {
    readonly id: string;
    readonly version: string;
    readonly environment: string;
    readonly digest: string;
    readonly signatureStatus: 'unsigned' | 'verified';
    readonly staticAnalysis: readonly ReturnType<
      typeof analyzeInfrastructurePolicySource
    >[number][];
  };
  readonly remoteTargets?: TargetPreflightV1;
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
    const processUsage = sources.filter(source =>
      inspectWomlModuleUsage(source.document).referencedServices.includes(
        'process'
      )
    );
    const artifactUsage = sources.filter(source =>
      inspectWomlModuleUsage(source.document).referencedServices.includes(
        'artifacts'
      )
    );
    if (parsed.processPolicyPath !== undefined) {
      await loadProcessPolicy(parsed.processPolicyPath, 'background');
    }
    if (parsed.artifactPolicyPath !== undefined) {
      await loadArtifactPolicy(parsed.artifactPolicyPath, 'background');
    }
    const infrastructurePolicy =
      parsed.infrastructurePolicyReference === undefined
        ? undefined
        : await resolveInfrastructurePolicy(
            parsed.infrastructurePolicyReference,
            {
              ...(parsed.infrastructurePolicyTrustPath === undefined
                ? {}
                : { trustPath: parsed.infrastructurePolicyTrustPath }),
              requireSignature: parsed.requireInfrastructurePolicySignature,
            }
          );
    const explicitlyConfiguredTargetRegistry =
      parsed.targetRegistryPath === undefined
        ? undefined
        : await loadTargetRegistry(
            parsed.targetRegistryPath,
            parsed.targetTrustPath ??
              (existsSync(resolve('.woml/target-trust.json'))
                ? resolve('.woml/target-trust.json')
                : undefined)
          );
    const targetRegistry = await loadDefaultTargetRegistryForRemoteWorkflows(
      sources.map(source => source.workflow),
      explicitlyConfiguredTargetRegistry
    );
    const remoteTargets = preflightRemoteTargets(
      sources.map(source => source.workflow),
      targetRegistry,
      infrastructurePolicy?.prepared.bundle
    );
    if (processUsage.length > 0 && parsed.processPolicyPath === undefined) {
      io.stderr(
        'WOML check: services.process is valid but disabled at runtime unless --allow-process or --process-policy is provided to woml run.\n'
      );
    }
    if (artifactUsage.length > 0 && parsed.artifactPolicyPath === undefined) {
      io.stderr(
        'WOML check: services.artifacts is valid but disabled at runtime unless --artifact-policy is provided to woml run.\n'
      );
    }
    printMigrationDiagnostics(
      io,
      sources.flatMap(source => source.migrationDiagnostics)
    );
    validateDeploymentRoutes(sources);
    for (const source of sources) {
      await refreshEditorTypes(source.filePath, source.runtimeModules, io);
    }

    let configuration: ResolvedRuntimeConfiguration | undefined;
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

    const report: ProductionPreflightReport = {
      profile:
        configuration?.schemaVersion === 2
          ? 'woml.production-preflight/v2'
          : 'woml.production-preflight/v1',
      status: 'passed',
      ...(configuration === undefined ? {} : { configuration }),
      ...(environment === undefined ? {} : { environment }),
      ...(secretProvider === undefined ? {} : { secretProvider }),
      ...(infrastructurePolicy === undefined
        ? {}
        : {
            infrastructurePolicy: {
              id: infrastructurePolicy.prepared.bundle.id,
              version: infrastructurePolicy.prepared.bundle.version,
              environment: infrastructurePolicy.prepared.bundle.environment,
              digest: infrastructurePolicy.prepared.digest,
              signatureStatus: infrastructurePolicy.prepared.signatureStatus,
              staticAnalysis: (
                await Promise.all(
                  sources.map(async source =>
                    analyzeInfrastructurePolicySource(
                      infrastructurePolicy.prepared.bundle,
                      source.workflow.workflowId,
                      await readFile(source.filePath, 'utf8')
                    )
                  )
                )
              ).flat(),
            },
          }),
      ...(remoteTargets === undefined ? {} : { remoteTargets }),
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
    if (report.infrastructurePolicy !== undefined) {
      io.stdout(
        `Policy ${report.infrastructurePolicy.id}@${report.infrastructurePolicy.version}: ${report.infrastructurePolicy.environment} · ${report.infrastructurePolicy.digest}.\n`
      );
      for (const decision of report.infrastructurePolicy.staticAnalysis) {
        io.stdout(
          `  ${decision.workflowId} ${decision.capability}.${decision.operation}: ${decision.status.replaceAll('_', ' ')}.\n`
        );
      }
    }
    if (report.remoteTargets !== undefined) {
      io.stdout(
        `Remote targets: ${report.remoteTargets.targets.map(target => `${target.id} (${target.eligibleWorkers.length} trusted worker${target.eligibleWorkers.length === 1 ? '' : 's'})`).join(', ')}.\n`
      );
      io.stdout(`Target registry: ${report.remoteTargets.registryDigest}.\n`);
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

function projectCheckExplicitReferences(
  parsed: CheckArguments
): Readonly<Partial<Record<WomlProjectReferenceKey, string | true>>> {
  return {
    ...(parsed.configPath === undefined ? {} : { runtime: parsed.configPath }),
    ...(parsed.processPolicyPath === undefined
      ? {}
      : { processPolicy: parsed.processPolicyPath }),
    ...(parsed.artifactPolicyPath === undefined
      ? {}
      : { artifactPolicy: parsed.artifactPolicyPath }),
    ...(parsed.reviewerDirectoryPath === undefined
      ? {}
      : { reviewerDirectory: parsed.reviewerDirectoryPath }),
    ...(parsed.targetRegistryPath === undefined
      ? {}
      : { targetRegistry: parsed.targetRegistryPath }),
    ...(parsed.targetTrustPath === undefined
      ? {}
      : { targetTrust: parsed.targetTrustPath }),
    ...(parsed.infrastructurePolicyReference === undefined &&
    !parsed.requireInfrastructurePolicySignature
      ? {}
      : {
          infrastructurePolicy:
            parsed.infrastructurePolicyReference ?? true,
        }),
    ...(parsed.infrastructurePolicyTrustPath === undefined
      ? {}
      : { policyTrust: parsed.infrastructurePolicyTrustPath }),
  };
}

function projectDiagnostic(
  code: string,
  message: string,
  correctiveAction: string
): ProjectCheckDiagnosticV1 {
  return {
    code,
    severity: 'error',
    message: message.slice(0, 4096),
    correctiveAction: correctiveAction.slice(0, 4096),
  };
}

function projectDiagnosticFromError(error: unknown): ProjectCheckDiagnosticV1 {
  if (error instanceof WomlDiagnosticError) {
    return {
      code: error.diagnostic.code,
      severity: 'error',
      message: error.diagnostic.message.slice(0, 4096),
      correctiveAction: (
        error.diagnostic.hint ??
        'Fix the source-located workflow diagnostic and run woml check again.'
      ).slice(0, 4096),
      sourcePath: error.diagnostic.file,
      line: error.diagnostic.location.start.line,
      column: error.diagnostic.location.start.column,
    };
  }
  const candidate = error as { code?: unknown; message?: unknown };
  const message =
    typeof candidate.message === 'string'
      ? candidate.message
      : 'The project workflow set could not be compiled.';
  return projectDiagnostic(
    typeof candidate.code === 'string'
      ? candidate.code
      : 'WOML_PROJECT_WORKFLOW_INVALID',
    message.replace(/\s*Corrective action:.*$/su, ''),
    /Corrective action:\s*(.+)$/su.exec(message)?.[1] ??
      'Fix the reported workflow source and run woml check again.'
  );
}

function workflowRequiredCapabilities(
  source: CompiledWorkflowSource
): readonly string[] {
  const activation = runtimeActivationRequirements([source.workflow]);
  const usage = inspectWomlModuleUsage(source.document);
  const capabilities = new Set<string>();
  if (activation.scriptExecution) capabilities.add('runtime.script');
  if (activation.runtimeModules) capabilities.add('runtime.modules');
  for (const service of usage.referencedServices)
    capabilities.add(`services.${service}`);
  for (const handler of activation.triggerHandlers)
    capabilities.add(handler);
  for (const capability of
    activation.remoteExecution?.requiredCapabilities ?? [])
    capabilities.add(capability);
  if (
    source.workflow.graph.nodes.some(
      node =>
        node.handler === 'engine.approval-wait' &&
        node.inputs.kind === 'object' &&
        node.inputs.fields.review !== undefined
    )
  ) {
    capabilities.add('reviewer.approval');
  }
  return [...capabilities].sort((left, right) => left.localeCompare(right));
}

async function runProjectCheckCommand(
  parsed: CheckArguments,
  io: CliIo
): Promise<number> {
  let project: ResolvedWomlProjectV1;
  try {
    project = await resolveWomlProject(parsed.projectPath!, {
      mode: 'background',
      explicitReferences: projectCheckExplicitReferences(parsed),
    });
    if (
      parsed.infrastructurePolicyReference === undefined &&
      project.references.infrastructurePolicy === undefined &&
      (parsed.infrastructurePolicyTrustPath !== undefined ||
        parsed.requireInfrastructurePolicySignature)
    ) {
      throw new CliInputError(
        'WOML_POLICY_REQUIRED',
        '--policy-trust and --require-policy-signature require a base infrastructure policy.'
      );
    }
  } catch (error) {
    io.stderr(`${formatError(error, parsed.projectPath)}\n`);
    return 1;
  }

  const diagnostics: ProjectCheckDiagnosticV1[] = [];
  const subsystems: ProjectCheckSubsystemV1[] = [];
  let processAuthority = project.authorities.process;
  let artifactAuthority = project.authorities.artifacts;
  let reviewerAuthority = project.authorities.reviewers;
  let targetRegistry = project.authorities.targets;
  let infrastructureAuthority = project.authorities.infrastructure;
  if (processAuthority === undefined && parsed.processPolicyPath !== undefined) {
    try {
      processAuthority = await loadProcessPolicy(
        parsed.processPolicyPath,
        'background'
      );
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  if (
    artifactAuthority === undefined &&
    parsed.artifactPolicyPath !== undefined
  ) {
    try {
      artifactAuthority = await loadArtifactPolicy(
        parsed.artifactPolicyPath,
        'background'
      );
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  if (
    reviewerAuthority === undefined &&
    parsed.reviewerDirectoryPath !== undefined
  ) {
    try {
      reviewerAuthority = await loadReviewerDirectory(
        parsed.reviewerDirectoryPath
      );
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  if (targetRegistry === undefined && parsed.targetRegistryPath !== undefined) {
    try {
      targetRegistry = await loadTargetRegistry(
        parsed.targetRegistryPath,
        parsed.targetTrustPath
      );
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  if (
    infrastructureAuthority === undefined &&
    parsed.infrastructurePolicyReference !== undefined
  ) {
    try {
      const resolved = await resolveInfrastructurePolicy(
        parsed.infrastructurePolicyReference,
        {
          ...(parsed.infrastructurePolicyTrustPath === undefined
            ? {}
            : { trustPath: parsed.infrastructurePolicyTrustPath }),
          requireSignature: parsed.requireInfrastructurePolicySignature,
        }
      );
      infrastructureAuthority = infrastructurePolicyRuntimeAuthority(resolved);
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  const compiledSources: CompiledWorkflowSource[] = [];
  for (const workflow of project.workflows) {
    try {
      compiledSources.push(...(await compileWorkflowInputs([workflow.path], io)));
    } catch (error) {
      diagnostics.push(projectDiagnosticFromError(error));
    }
  }
  const sources: readonly CompiledWorkflowSource[] = compiledSources;
  try {
    validateDeploymentRoutes(sources);
  } catch (error) {
    diagnostics.push(projectDiagnosticFromError(error));
  }

  const workflowReports = sources.map(source => ({
    id: source.workflow.workflowId,
    sourcePath:
      project.workflows.find(workflow => workflow.path === source.filePath)
        ?.projectPath ?? source.filePath,
    status: 'ready' as const,
    requiredCapabilities: workflowRequiredCapabilities(source),
  }));
  const workflows = sources.map(source => source.workflow);
  const usages = sources.map(source => inspectWomlModuleUsage(source.document));
  if (infrastructureAuthority !== undefined) {
    for (const source of sources) {
      const decisions = analyzeInfrastructurePolicySource(
        infrastructureAuthority.infrastructurePolicy.policy,
        source.workflow.workflowId,
        await readFile(source.filePath, 'utf8')
      );
      for (const decision of decisions) {
        if (decision.status !== 'denied') continue;
        diagnostics.push(
          projectDiagnostic(
            'WOML_INFRASTRUCTURE_POLICY_DENIED',
            `Infrastructure policy denies ${decision.capability}.${decision.operation} in workflow ${decision.workflowId}.`,
            `Review ${project.references.infrastructurePolicy?.projectPath ?? 'the selected --policy file'} or change the denied workflow operation.`
          )
        );
      }
    }
  }
  const processRequired = usages.some(usage =>
    usage.referencedServices.includes('process')
  );
  const artifactsRequired = usages.some(usage =>
    usage.referencedServices.includes('artifacts')
  );
  const richApprovalRequired = workflowReports.some(workflow =>
    workflow.requiredCapabilities.includes('reviewer.approval')
  );
  const activation = runtimeActivationRequirements(workflows);
  const signalsRequired =
    activation.triggerHandlers.some(handler =>
      ['trigger.event', 'trigger.signal'].includes(handler)
    ) ||
    workflows.some(
      workflow =>
        'waits' in workflow.graph &&
        workflow.graph.waits.some(wait => wait.kind === 'signal')
    );
  const outputRequired = processRequired;

  try {
    const configuration = await resolveRuntimeConfiguration(
      parsed.configPath ?? project.references.runtime?.path,
      {},
      {},
      project.root
    );
    await preflightRuntimeConfiguration(configuration);
    subsystems.push({
      id: 'runtime',
      status: 'ready',
      required: true,
      ...(project.references.runtime === undefined
        ? {}
        : { digest: project.references.runtime.digest }),
    });
    const telemetryEnabled =
      configuration.schemaVersion === 2 &&
      configuration.observability.openTelemetry.enabled;
    subsystems.push({
      id: 'opentelemetry',
      status: telemetryEnabled ? 'degraded' : 'not-required',
      required: false,
      ...(telemetryEnabled && project.references.runtime !== undefined
        ? { digest: project.references.runtime.digest }
        : {}),
    });
    if (telemetryEnabled) {
      diagnostics.push({
        code: 'WOML_OTEL_REACHABILITY_NOT_PROBED',
        severity: 'warning',
        message: 'Collector reachability is not probed by project check.',
        correctiveAction:
          'Use the explicit runtime health command when a live collector probe is required.',
      });
    }
  } catch (error) {
    const candidate = error as { code?: unknown; message?: unknown };
    subsystems.push({ id: 'runtime', status: 'blocked', required: true });
    subsystems.push({
      id: 'opentelemetry',
      status: 'not-required',
      required: false,
    });
    diagnostics.push(
      projectDiagnostic(
        typeof candidate.code === 'string'
          ? candidate.code
          : 'WOML_DEPLOYMENT_PREFLIGHT_FAILED',
        String(candidate.message ?? 'Runtime preflight failed.'),
        'Repair the referenced runtime configuration or local storage permissions and run woml check again.'
      )
    );
  }

  const authoritySubsystem = (
    id: string,
    required: boolean,
    ready: boolean,
    reference: WomlProjectReferenceKey,
    code: string,
    correctiveAction: string
  ) => {
    subsystems.push({
      id,
      status: required ? (ready ? 'ready' : 'blocked') : 'not-required',
      required,
      ...(ready && project.references[reference] !== undefined
        ? { digest: project.references[reference]!.digest }
        : {}),
    });
    if (required && !ready) {
      diagnostics.push(
        projectDiagnostic(
          code,
          `${id} authority is required by the compiled project but is not configured.`,
          correctiveAction
        )
      );
    }
  };
  authoritySubsystem(
    'process',
    processRequired,
    processAuthority !== undefined,
    'processPolicy',
    'WOML_PROCESS_DISABLED',
    'Add authority.processPolicy to woml.project.json and reference a reviewed Process Policy v1 file.'
  );
  authoritySubsystem(
    'artifacts',
    artifactsRequired,
    artifactAuthority !== undefined,
    'artifactPolicy',
    'WOML_ARTIFACT_SERVICE_DISABLED',
    'Add authority.artifactPolicy to woml.project.json and reference a reviewed Artifact Policy v1 file.'
  );
  authoritySubsystem(
    'approvals',
    richApprovalRequired,
    reviewerAuthority !== undefined,
    'reviewerDirectory',
    'WOML_REVIEWER_DIRECTORY_REQUIRED',
    'Add authority.reviewerDirectory to woml.project.json and reference a reviewed Reviewer Directory v1 file.'
  );
  const infrastructureRequired =
    activation.remoteExecution !== undefined ||
    infrastructureAuthority !== undefined;
  authoritySubsystem(
    'infrastructure-policy',
    infrastructureRequired,
    infrastructureAuthority !== undefined,
    'infrastructurePolicy',
    'WOML_INFRASTRUCTURE_POLICY_REQUIRED',
    'Add authority.infrastructurePolicy and authority.policyTrust to woml.project.json.'
  );

  if (activation.remoteExecution === undefined) {
    subsystems.push({
      id: 'remote-execution',
      status: 'not-required',
      required: false,
    });
  } else {
    try {
      const preflight = preflightRemoteTargets(
        workflows,
        targetRegistry,
        infrastructureAuthority?.infrastructurePolicy.policy
      );
      subsystems.push({
        id: 'remote-execution',
        status: 'ready',
        required: true,
        ...(preflight === undefined
          ? {}
          : { digest: preflight.registryDigest }),
      });
    } catch (error) {
      const candidate = error as { code?: unknown; message?: unknown };
      subsystems.push({
        id: 'remote-execution',
        status: 'blocked',
        required: true,
      });
      diagnostics.push(
        projectDiagnostic(
          typeof candidate.code === 'string'
            ? candidate.code
            : 'WOML_TARGET_UNAVAILABLE',
          String(candidate.message ?? 'Remote target preflight failed.'),
          'Repair remoteExecution.targetRegistry, remoteExecution.targetTrust, and infrastructure policy references, then run woml check again.'
        )
      );
    }
  }
  subsystems.push({
    id: 'external-signals',
    status: signalsRequired ? 'ready' : 'not-required',
    required: signalsRequired,
  });
  subsystems.push({
    id: 'streaming-output',
    status: outputRequired ? 'ready' : 'not-required',
    required: outputRequired,
  });

  const report = createWomlProjectCheck(
    project,
    workflowReports,
    subsystems,
    diagnostics
  );
  if (parsed.json) io.stdout(`${JSON.stringify(report, null, 2)}\n`);
  else io.stdout(formatWomlProjectCheck(report));
  return report.status === 'blocked' ? 1 : 0;
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

  if (parsed.projectPath !== undefined) {
    return await runProjectCheckCommand(parsed, io);
  }

  const onlyInput = parsed.inputPaths[0];
  const onlyInputIsFile =
    onlyInput !== undefined &&
    (await stat(onlyInput).catch(() => undefined))?.isFile() === true;
  if (
    parsed.inputPaths.length === 1 &&
    parsed.configPath === undefined &&
    parsed.processPolicyPath === undefined &&
    parsed.artifactPolicyPath === undefined &&
    parsed.targetRegistryPath === undefined &&
    parsed.targetTrustPath === undefined &&
    parsed.infrastructurePolicyReference === undefined &&
    onlyInputIsFile
  ) {
    return await runSingleCheckCommand(
      ['check', parsed.inputPaths[0]!, ...(parsed.json ? ['--json'] : [])],
      io
    );
  }
  return await runDeploymentCheckCommand(parsed, io, dependencies);
}

interface PolicyCommandArguments {
  readonly action: 'check' | 'explain';
  readonly policyReference: string;
  readonly workflowPath?: string;
  readonly stepId?: string;
  readonly trustPath?: string;
  readonly requireSignature: boolean;
  readonly json: boolean;
}

function parsePolicyCommandArguments(
  args: readonly string[]
): PolicyCommandArguments {
  const action = args[1];
  if (args[0] !== 'policy' || (action !== 'check' && action !== 'explain')) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
  }
  let policyReference: string | undefined;
  let workflowPath: string | undefined;
  let stepId: string | undefined;
  let trustPath: string | undefined;
  let requireSignature = false;
  let json = false;
  for (let index = 2; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--json' || argument === '--require-signature') {
      if (
        (argument === '--json' && json) ||
        (argument === '--require-signature' && requireSignature)
      ) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
      }
      if (argument === '--json') json = true;
      else requireSignature = true;
      continue;
    }
    if (
      argument === '--policy' ||
      argument === '--step' ||
      argument === '--trust'
    ) {
      const value = args[index + 1];
      if (value === undefined || value.startsWith('--')) {
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
      }
      if (argument === '--policy') {
        if (policyReference !== undefined)
          throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
        policyReference = value;
      } else if (argument === '--step') {
        if (stepId !== undefined)
          throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
        stepId = value;
      } else {
        if (trustPath !== undefined)
          throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
        trustPath = value;
      }
      index += 1;
      continue;
    }
    if (argument.startsWith('--')) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
    }
    if (action === 'check') {
      if (policyReference !== undefined)
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
      policyReference = argument;
    } else {
      if (workflowPath !== undefined)
        throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
      workflowPath = resolve(argument);
    }
  }
  if (
    policyReference === undefined ||
    (action === 'explain' && workflowPath === undefined) ||
    (action === 'check' && (workflowPath !== undefined || stepId !== undefined))
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', policyUsage());
  }
  return {
    action,
    policyReference,
    ...(workflowPath === undefined ? {} : { workflowPath }),
    ...(stepId === undefined ? {} : { stepId }),
    ...(trustPath === undefined ? {} : { trustPath }),
    requireSignature,
    json,
  };
}

async function runPolicyCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  try {
    const parsed = parsePolicyCommandArguments(args);
    const resolvedPolicy: ResolvedInfrastructurePolicyV1 =
      await resolveInfrastructurePolicy(parsed.policyReference, {
        ...(parsed.trustPath === undefined
          ? {}
          : { trustPath: parsed.trustPath }),
        requireSignature: parsed.requireSignature,
      });
    const diagnostics = infrastructurePolicyDiagnostics(
      resolvedPolicy.prepared
    );
    if (parsed.action === 'check') {
      const report = {
        profile: 'woml.infrastructure-policy-check/v1' as const,
        status: 'valid' as const,
        path: resolvedPolicy.path,
        id: resolvedPolicy.prepared.bundle.id,
        version: resolvedPolicy.prepared.bundle.version,
        environment: resolvedPolicy.prepared.bundle.environment,
        digest: resolvedPolicy.prepared.digest,
        signatureStatus: resolvedPolicy.prepared.signatureStatus,
        ruleCount: resolvedPolicy.prepared.bundle.rules.length,
        diagnostics,
      };
      if (parsed.json) io.stdout(`${JSON.stringify(report, null, 2)}\n`);
      else {
        io.stdout(
          `Infrastructure policy ${report.id}@${report.version} is valid.\n`
        );
        io.stdout(
          `Environment: ${report.environment}\nDigest: ${report.digest}\nSignature: ${report.signatureStatus}\nRules: ${report.ruleCount}\n`
        );
        for (const diagnostic of diagnostics)
          io.stdout(`Warning: ${diagnostic.message}\n`);
      }
      return 0;
    }
    const sources = await compileWorkflowInputs([parsed.workflowPath!], io);
    const analysis = (
      await Promise.all(
        sources.map(async source =>
          analyzeInfrastructurePolicySource(
            resolvedPolicy.prepared.bundle,
            source.workflow.workflowId,
            await readFile(source.filePath, 'utf8'),
            parsed.stepId
          )
        )
      )
    ).flat();
    const report = {
      profile: 'woml.infrastructure-policy-explanation/v1' as const,
      policy: {
        id: resolvedPolicy.prepared.bundle.id,
        version: resolvedPolicy.prepared.bundle.version,
        environment: resolvedPolicy.prepared.bundle.environment,
        digest: resolvedPolicy.prepared.digest,
      },
      workflowPath: parsed.workflowPath!,
      ...(parsed.stepId === undefined ? {} : { stepId: parsed.stepId }),
      diagnostics,
      operations: analysis,
      guarantee:
        'Static analysis is advisory. The Rust authority evaluates the final trusted descriptor and durable evidence immediately before each effect.',
    };
    if (parsed.json) io.stdout(`${JSON.stringify(report, null, 2)}\n`);
    else {
      io.stdout(
        `Policy ${report.policy.id}@${report.policy.version} · ${report.policy.environment}\n`
      );
      if (analysis.length === 0)
        io.stdout(
          'No statically identifiable protected service calls were found.\n'
        );
      for (const operation of analysis) {
        io.stdout(
          `${operation.workflowId} · ${operation.capability}.${operation.operation} · ${operation.status.replaceAll('_', ' ')}\n  ${operation.reason}\n`
        );
      }
      io.stdout(`${report.guarantee}\n`);
    }
    return 0;
  } catch (error) {
    if (error instanceof CliInputError && error.message === policyUsage()) {
      io.stderr(`${policyUsage()}\n`);
      return 2;
    }
    io.stderr(`${formatError(error)}\n`);
    return error instanceof CliInputError ? 2 : 1;
  }
}

async function runTypesCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  let rawSourcePath: string | undefined;
  let projectPath: string | undefined;
  let output: string | undefined;
  for (let index = 1; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--project' || argument === '--output') {
      const value = args[index + 1];
      if (value === undefined || value.startsWith('--')) {
        io.stderr(`${typesUsage()}\n`);
        return 2;
      }
      if (argument === '--project') {
        if (projectPath !== undefined) {
          io.stderr(`${typesUsage()}\n`);
          return 2;
        }
        projectPath = resolve(value);
      } else {
        if (output !== undefined) {
          io.stderr(`${typesUsage()}\n`);
          return 2;
        }
        output = value;
      }
      index += 1;
      continue;
    }
    if (argument.startsWith('--') || rawSourcePath !== undefined) {
      io.stderr(`${typesUsage()}\n`);
      return 2;
    }
    rawSourcePath = argument;
  }
  if (rawSourcePath !== undefined && projectPath !== undefined) {
    io.stderr('WOML_PROJECT_OPTION_CONFLICT: Use a workflow input or --project, not both.\n');
    return 2;
  }
  if (rawSourcePath === undefined && projectPath === undefined) {
    const discovered = resolve('woml.project.json');
    if (existsSync(discovered)) projectPath = discovered;
  }
  if (rawSourcePath === undefined && projectPath === undefined) {
    io.stderr(`${typesUsage()}\n`);
    return 2;
  }
  try {
    const project =
      projectPath === undefined
        ? undefined
        : await resolveWomlProject(projectPath);
    const sourcePath =
      project === undefined ? resolve(rawSourcePath!) : project.root;
    const sourceStat = await stat(sourcePath);
    const workflows =
      project === undefined
        ? await compileWorkflowSources(sourcePath)
        : await compileWorkflowInputs(
            project.workflows.map(workflow => workflow.path)
          );
    const modules = workflows.flatMap(workflow =>
      'moduleRuntime' in workflow.workflow &&
      workflow.workflow.moduleRuntime !== undefined
        ? workflow.workflow.moduleRuntime.modules
        : []
    );
    const outputPath = resolve(
      output ?? editorTypesPath(sourcePath, sourceStat.isDirectory())
    );
    await mkdir(dirname(outputPath), { recursive: true });
    await writeFile(
      outputPath,
      generateWomlEditorDeclarations(modules),
      'utf8'
    );
    io.stdout(`WOML editor types written to ${outputPath}.\n`);
    io.stdout(
      `Modules: ${modules.length}; script globals: services, context, attempt, and symbolic secrets.\n`
    );
    return 0;
  } catch (error) {
    io.stderr(
      `${formatError(error, projectPath ?? (rawSourcePath === undefined ? undefined : resolve(rawSourcePath)))}\n`
    );
    return 1;
  }
}

async function runInitCommand(
  args: readonly string[],
  io: CliIo
): Promise<number> {
  let directory = '.';
  let json = false;
  for (let index = 1; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === '--json') {
      if (json) {
        io.stderr(`${initUsage()}\n`);
        return 2;
      }
      json = true;
      continue;
    }
    if (argument.startsWith('--') || directory !== '.') {
      io.stderr(`${initUsage()}\n`);
      return 2;
    }
    directory = argument;
  }
  try {
    const result = await initializeWomlProject(directory);
    if (json) io.stdout(`${JSON.stringify(result, null, 2)}\n`);
    else {
      io.stdout(`Initialized WOML project in ${result.directory}.\n`);
      io.stdout(`Manifest  ${result.manifestPath}\n`);
      io.stdout(`Workflow  ${result.workflowPath}\n`);
      io.stdout(`Guide     ${result.authoringGuidePath}\n`);
      io.stdout('Next      woml check --project woml.project.json\n');
    }
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
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
  if (approval.reviewDigest !== undefined && approval.authority !== undefined) {
    io.stderr(`Review digest: ${approval.reviewDigest}\n`);
    io.stderr(
      `Quorum: ${approval.authority.received}/${approval.authority.required}\n`
    );
    io.stderr(`Review endpoint: ${url}\n`);
  } else {
    io.stderr(`Current URL expires: ${approval.credentialExpiresAt}\n`);
    io.stderr(`Approval URL: ${url}\n`);
  }
  io.stderr(
    `Recovery: woml run ${JSON.stringify(filePath)} --state ${JSON.stringify(
      statePath
    )} --resume ${JSON.stringify(outcome.runId)}\n\n`
  );
}

async function waitForDurableTimer(
  io: CliIo,
  outcome: Extract<RustApprovalRuntimeOutcome, { status: 'waiting_time' }>,
  statePath: string,
  nativeCorePath: string | undefined
): Promise<void> {
  io.stderr(
    `\nWOML run ${outcome.runId} is durably waiting until ${outcome.wait.deadline}.\n`
  );
  let deadline = outcome.wait.deadline;
  for (;;) {
    const remaining = Date.parse(deadline) - Date.now();
    if (remaining > 0) {
      await new Promise<void>(resolve =>
        setTimeout(resolve, Math.min(remaining, 60_000))
      );
      continue;
    }
    const settlement = settleTimerWaitWithRust(
      statePath,
      outcome.runId,
      outcome.wait.waitId,
      nativeCorePath === undefined ? {} : { nativeCorePath }
    );
    if (settlement.status !== 'not_due') return;
    deadline = settlement.deadline;
  }
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
    dependencies.createSecretStore(),
    args.runtimeCapabilities
  );
  const runtimeOptions = {
    nativeCorePath: dependencies.nativeCorePath,
    resolvedSecrets: secrets,
    onProgress: durableRetryProgress(io, args),
    runtimeModules,
    runtimeCapabilities: args.runtimeCapabilities,
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

  while (outcome.status !== 'succeeded') {
    if (outcome.status === 'waiting_time') {
      const waiting = outcome;
      await waitForDurableTimer(
        io,
        waiting,
        args.statePath,
        dependencies.nativeCorePath
      );
      outcome = await resumeApprovalWorkflowWithRust(
        workflow,
        args.statePath,
        waiting.runId,
        runtimeOptions
      );
      continue;
    }
    if (outcome.status === 'waiting_signal') {
      throw new CliInputError(
        'WOML_SIGNAL_RUNTIME_REQUIRED',
        `run ${outcome.runId} is waiting for ${outcome.wait.signals.join(', ')}. Keep it active with "woml run" and deliver the callback with "woml signal" or the authenticated HTTP endpoint.`
      );
    }
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
  printSettledOneShot(
    args,
    io,
    dependencies,
    outcome.execution.runId,
    outcome.execution.result
  );
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
  if (
    workflow.schemaVersion === 16 ||
    workflow.schemaVersion === 17 ||
    workflow.schemaVersion === 18 ||
    workflow.schemaVersion === 19 ||
    workflow.schemaVersion === 20
  ) {
    for (const descriptor of workflow.graph.forEach) {
      for (const node of descriptor.body.nodes) {
        collectSecretReferences(node.inputs, references);
        for (const name of node.scriptRuntime?.requiredSecrets ?? []) {
          references.push({ kind: 'secretReference', name });
        }
      }
    }
  }
  if (workflow.schemaVersion === 19 || workflow.schemaVersion === 20) {
    for (const saga of workflow.graph.sagas) {
      for (const action of [...saga.compensations, ...saga.cleanup]) {
        collectSecretReferences(action.node.inputs, references);
        for (const name of action.node.scriptRuntime?.requiredSecrets ?? []) {
          references.push({ kind: 'secretReference', name });
        }
      }
    }
  }
  for (const definition of workflow.schemaVersion === 14 ||
  workflow.schemaVersion === 15 ||
  workflow.schemaVersion === 16 ||
  workflow.schemaVersion === 17 ||
  workflow.schemaVersion === 18 ||
  workflow.schemaVersion === 19 ||
  workflow.schemaVersion === 20
    ? (workflow.reusableDefinitions ?? [])
    : []) {
    for (const prop of definition.props) {
      if (prop.expression.kind === 'secret') {
        references.push({
          kind: 'secretReference',
          name: prop.expression.name,
        });
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

function workflowHasDurableWait(workflow: CompiledWorkflowDefinition): boolean {
  return workflow.graph.nodes.some(
    node => node.handler === 'engine.durable-wait'
  );
}

function workflowHasDurableLock(workflow: CompiledWorkflowDefinition): boolean {
  return workflow.graph.nodes.some(
    node => node.handler === 'engine.lock-acquire'
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
    dependencies.createSecretStore(),
    args.runtimeCapabilities
  );
  await mkdir(dirname(args.statePath), { recursive: true });
  const runtimeProgress = durableRetryProgress(io, args);
  let outcome =
    args.resumeRunId === undefined
      ? await executeApprovalWorkflowWithRust(workflow, args.statePath, {
          nativeCorePath: dependencies.nativeCorePath,
          onProgress: runtimeProgress,
          resolvedSecrets: secrets,
          runtimeModules,
          runtimeCapabilities: args.runtimeCapabilities,
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
            runtimeCapabilities: args.runtimeCapabilities,
          }
        );
  while (outcome.status !== 'succeeded') {
    if (outcome.status === 'waiting_time') {
      const waiting = outcome;
      await waitForDurableTimer(
        io,
        waiting,
        args.statePath,
        dependencies.nativeCorePath
      );
      outcome = await resumeApprovalWorkflowWithRust(
        workflow,
        args.statePath,
        waiting.runId,
        {
          nativeCorePath: dependencies.nativeCorePath,
          onProgress: runtimeProgress,
          resolvedSecrets: secrets,
          runtimeModules,
          runtimeCapabilities: args.runtimeCapabilities,
        }
      );
      continue;
    }
    if (outcome.status === 'waiting_signal') {
      throw new CliInputError(
        'WOML_SIGNAL_RUNTIME_REQUIRED',
        `run ${outcome.runId} is waiting for ${outcome.wait.signals.join(', ')}. Keep it active with "woml run" and deliver the callback with "woml signal" or the authenticated HTTP endpoint.`
      );
    }
    const waiting = outcome;
    printNotificationApproval(io, waiting, args.filePath, args.statePath);
    const hasCustomProvider =
      (workflow.schemaVersion === 14 ||
        workflow.schemaVersion === 15 ||
        workflow.schemaVersion === 16 ||
        workflow.schemaVersion === 17 ||
        workflow.schemaVersion === 18 ||
        workflow.schemaVersion === 19 ||
        workflow.schemaVersion === 20) &&
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
            customNotificationHostPath: dependencies.customNotificationHostPath,
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
    printNotificationWarnings(io, journey.diagnostics, journey.updates);
    outcome = await resumeApprovalWorkflowWithRust(
      workflow,
      args.statePath,
      waiting.runId,
      {
        nativeCorePath: dependencies.nativeCorePath,
        onProgress: runtimeProgress,
        resolvedSecrets: secrets,
        runtimeModules,
        runtimeCapabilities: args.runtimeCapabilities,
      }
    );
  }
  printSettledOneShot(
    args,
    io,
    dependencies,
    outcome.execution.runId,
    outcome.execution.result
  );
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
    onProgress: durableRetryProgress(io, args),
    runtimeCapabilities: args.runtimeCapabilities,
  };

  io.stderr(
    `Recovering run ${runId} from its stored definition and ${requirements.moduleCount} immutable module artifact${requirements.moduleCount === 1 ? '' : 's'}.\n`
  );
  let outcome = await resumeStoredRunWithRust(
    args.statePath,
    runId,
    runtimeOptions
  );
  while (outcome.status !== 'succeeded') {
    if (outcome.status === 'waiting_time') {
      const waiting = outcome;
      await waitForDurableTimer(
        io,
        waiting,
        args.statePath,
        dependencies.nativeCorePath
      );
      outcome = await resumeStoredRunWithRust(
        args.statePath,
        waiting.runId,
        runtimeOptions
      );
      continue;
    }
    if (outcome.status === 'waiting_signal') {
      throw new CliInputError(
        'WOML_SIGNAL_RUNTIME_REQUIRED',
        `run ${outcome.runId} is waiting for ${outcome.wait.signals.join(', ')}. Keep it active with "woml run" and deliver the callback with "woml signal" or the authenticated HTTP endpoint.`
      );
    }
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
            customNotificationHostPath: dependencies.customNotificationHostPath,
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
      printNotificationWarnings(io, journey.diagnostics, journey.updates);
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
  printSettledOneShot(
    args,
    io,
    dependencies,
    outcome.execution.runId,
    outcome.execution.result,
    true
  );
}

function presentationOutput(args: RunArguments, io: CliIo, text: string): void {
  if (args.json) io.stdout(text);
  else io.stderr(text);
}

function printSettledOneShot(
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies,
  runId: string,
  fallbackResult: JsonValue,
  includeWorkflowHeader = false
): void {
  if (args.command === 'test') {
    io.stdout(`${JSON.stringify(fallbackResult)}\n`);
    return;
  }
  const render = presentationRenderOptions(args, io);
  try {
    const presentation = inspectRunPresentationWithRust(args.statePath, runId, {
      nativeCorePath: dependencies.nativeCorePath,
    });
    if (includeWorkflowHeader) {
      presentationOutput(
        args,
        io,
        renderWorkflowStartup(presentation.workflow, render)
      );
    }
    const rendered = renderRunPresentation(presentation, render);
    presentationOutput(args, io, args.json ? rendered : `\n${rendered}`);
  } catch (error) {
    if (!args.json) {
      io.stderr(
        `Warning [WOML_RUN_PRESENTATION_UNAVAILABLE]: ${error instanceof Error ? error.message : String(error)}\n`
      );
    }
    io.stdout(`${JSON.stringify(fallbackResult)}\n`);
  }
}

async function executeOneShot(
  workflow: CompiledWorkflowDefinition,
  args: RunArguments,
  io: CliIo,
  dependencies: CliDependencies,
  runtimeModules: readonly RustRuntimeModuleArtifact[] = []
): Promise<void> {
  if (args.runtimeCapabilities !== undefined) {
    if (args.runtimeCapabilities.process !== undefined) {
      io.stderr(
        `${processGrantSummary({ process: args.runtimeCapabilities.process })}\n`
      );
    }
    if (args.runtimeCapabilities.artifacts !== undefined) {
      io.stderr(
        `${artifactGrantSummary({ artifacts: args.runtimeCapabilities.artifacts })}\n`
      );
    }
  }
  if (args.command === 'run') {
    presentationOutput(
      args,
      io,
      renderWorkflowStartup(
        workflowPresentation(
          workflow,
          compiledDefinitionHash(workflow),
          args.host,
          args.port,
          new Map(),
          new Map()
        ),
        presentationRenderOptions(args, io)
      )
    );
  }
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
  const hasDurableWait = workflowHasDurableWait(workflow);
  const hasDurableLock = workflowHasDurableLock(workflow);
  const hasNotifications = workflowHasNotifications(workflow);
  if (
    args.resumeRunId !== undefined &&
    !hasApproval &&
    args.runtimeCapabilities?.artifacts === undefined &&
    workflow.schemaVersion !== 6 &&
    workflow.schemaVersion !== 8 &&
    workflow.schemaVersion !== 9 &&
    workflow.schemaVersion !== 11 &&
    workflow.schemaVersion !== 12 &&
    workflow.schemaVersion !== 13 &&
    workflow.schemaVersion !== 14 &&
    workflow.schemaVersion !== 15 &&
    workflow.schemaVersion !== 16 &&
    workflow.schemaVersion !== 17 &&
    workflow.schemaVersion !== 18 &&
    workflow.schemaVersion !== 19 &&
    workflow.schemaVersion !== 20
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
  if (hasApproval || hasDurableWait || hasDurableLock) {
    await runApprovalWorkflow(workflow, args, io, dependencies, runtimeModules);
    return;
  }
  if (
    args.runtimeCapabilities?.artifacts !== undefined ||
    workflow.schemaVersion === 6 ||
    workflow.schemaVersion === 8 ||
    workflow.schemaVersion === 9 ||
    workflow.schemaVersion === 11 ||
    workflow.schemaVersion === 12 ||
    workflow.schemaVersion === 13 ||
    workflow.schemaVersion === 14 ||
    workflow.schemaVersion === 15 ||
    workflow.schemaVersion === 16 ||
    workflow.schemaVersion === 17 ||
    workflow.schemaVersion === 18 ||
    workflow.schemaVersion === 19 ||
    workflow.schemaVersion === 20
  ) {
    await mkdir(dirname(args.statePath), { recursive: true });
    const onProgress = durableRetryProgress(io, args);
    const secrets = await resolvedSecrets(
      workflow,
      dependencies.createSecretStore(),
      args.runtimeCapabilities
    );
    const execution =
      args.resumeRunId === undefined
        ? await executeWorkflowWithRustDurable(workflow, args.statePath, {
            nativeCorePath: dependencies.nativeCorePath,
            onProgress,
            resolvedSecrets: secrets,
            runtimeModules,
            runtimeCapabilities: args.runtimeCapabilities,
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
              runtimeCapabilities: args.runtimeCapabilities,
            }
          );
    printSettledOneShot(
      args,
      io,
      dependencies,
      execution.runId,
      execution.result
    );
    return;
  }
  const execution = await executeWorkflowWithRust(workflow, {
    nativeCorePath: dependencies.nativeCorePath,
    runtimeModules,
    runtimeCapabilities: args.runtimeCapabilities,
  });
  // Historical pre-durable models have no event-backed presentation. Keep
  // their established result contract until they are migrated.
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

interface ActiveManualTarget {
  readonly workflowId: string;
  readonly triggerId: string;
}

function activeManualTargets(
  sources: readonly CompiledWorkflowSource[],
  requestedTriggerId: string | undefined
): readonly ActiveManualTarget[] {
  const targets = sources.flatMap(source =>
    source.workflow.triggers
      .filter(trigger => trigger.handler === 'trigger.manual')
      .map(trigger => ({
        workflowId: source.workflow.workflowId,
        triggerId: trigger.id,
      }))
  );
  if (requestedTriggerId === undefined) return targets;
  const selected = targets.filter(
    target => target.triggerId === requestedTriggerId
  );
  if (selected.length !== 1) {
    throw new CliInputError(
      'WOML_MANUAL_TRIGGER_NOT_FOUND',
      `the loaded workflow has no unique manual trigger "${requestedTriggerId}".`
    );
  }
  return selected;
}

function hasNonManualRuntimeIngress(
  sources: readonly CompiledWorkflowSource[]
): boolean {
  return sources.some(source =>
    source.workflow.triggers.some(trigger =>
      [
        'trigger.webhook',
        'trigger.slack',
        'trigger.telegram',
        'trigger.discord',
        'trigger.whatsapp',
        'trigger.schedule',
        'trigger.interval',
        'trigger.event',
      ].includes(trigger.handler)
    )
  );
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

function literalNumber(
  expression: ValueExpression | undefined
): number | undefined {
  return expression?.kind === 'literal' && typeof expression.value === 'number'
    ? expression.value
    : undefined;
}

function literalStringArray(
  expression: ValueExpression | undefined
): readonly string[] {
  if (expression?.kind !== 'array') return [];
  return expression.items
    .map(item => literalString(item))
    .filter((item): item is string => item !== undefined);
}

function triggerPresentationType(
  handler: string
): TriggerPresentationV1['type'] | undefined {
  if (handler === 'trigger.manual') return 'manual';
  if (handler === 'trigger.webhook') return 'webhook';
  if (handler === 'trigger.slack') return 'slack';
  if (handler === 'trigger.telegram') return 'telegram';
  if (handler === 'trigger.discord') return 'discord';
  if (handler === 'trigger.whatsapp') return 'whatsapp';
  if (handler === 'trigger.schedule') return 'schedule';
  if (handler === 'trigger.interval') return 'interval';
  if (handler === 'trigger.event') return 'event';
  return undefined;
}

function readableInterval(
  milliseconds: number | undefined
): string | undefined {
  if (milliseconds === undefined) return undefined;
  const units = [
    ['d', 86_400_000],
    ['h', 3_600_000],
    ['m', 60_000],
    ['s', 1_000],
    ['ms', 1],
  ] as const;
  for (const [suffix, size] of units) {
    if (milliseconds % size === 0) return `${milliseconds / size}${suffix}`;
  }
  return `${milliseconds}ms`;
}

function presentationRenderOptions(
  args: RunArguments,
  io: CliIo,
  manualInstruction?: string
): PresentationRenderOptions {
  return {
    format: args.json ? 'json' : io.isTTY === true ? 'tty' : 'plain',
    color: args.color,
    isTTY: io.isTTY === true,
    width: io.columns,
    environment: process.env,
    fullResultCommand: runId =>
      `woml get ${runId} --state ${JSON.stringify(args.statePath)} --json`,
    ...(manualInstruction === undefined ? {} : { manualInstruction }),
  };
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

function dueIdentity(workflowId: string, triggerId: string): string {
  return `${workflowId}\u0000${triggerId}`;
}

function workflowPresentation(
  workflow: CompiledWorkflowDefinition,
  definitionHash: string,
  host: string,
  port: number,
  scheduleDue: ReadonlyMap<string, string>,
  intervalDue: ReadonlyMap<string, string>,
  slackWorkspace?: string
): WorkflowPresentationV1 {
  const webhookRoutes = new Map(
    webhookRouteSummaries(workflow).map(route => [route.triggerId, route])
  );
  const eventRoutes = new Map(
    eventRouteSummaries(workflow).map(route => [route.eventName, route])
  );
  const triggers: TriggerPresentationV1[] = [];
  for (const trigger of workflow.triggers) {
    const type = triggerPresentationType(trigger.handler);
    if (type === undefined) continue;
    const fields = objectFields(trigger.config);
    if (type === 'webhook') {
      const route = webhookRoutes.get(trigger.id);
      triggers.push({
        id: trigger.id,
        type,
        method: route?.method ?? 'POST',
        ...(route === undefined
          ? {}
          : {
              url: `http://${host}:${port}${route.path}`,
              example: webhookCurlExample(route, host, port),
              ...(route.authentication === 'none'
                ? {
                    warning:
                      'No authentication; this endpoint accepts unauthenticated requests.',
                  }
                : {}),
            }),
      });
      continue;
    }
    if (type === 'event') {
      const eventName = literalString(fields?.name) ?? trigger.id;
      const route = eventRoutes.get(eventName);
      triggers.push({
        id: trigger.id,
        type,
        event: eventName,
        ...(route?.publicEndpoint === true
          ? {
              url: `http://${host}:${port}/_woml/events/${eventName}`,
              example: eventCurlExample(route, host, port),
            }
          : {}),
      });
      continue;
    }
    if (type === 'slack') {
      const events = literalStringArray(fields?.events);
      const channels = literalStringArray(fields?.channels);
      const scope = [
        events.length === 0 ? undefined : events.join(', '),
        channels.length === 0 ? undefined : `channels: ${channels.join(', ')}`,
      ]
        .filter((value): value is string => value !== undefined)
        .join(' · ');
      triggers.push({
        id: trigger.id,
        type,
        ...(slackWorkspace === undefined ? {} : { workspace: slackWorkspace }),
        ...(scope.length === 0 ? {} : { scope }),
      });
      continue;
    }
    if (type === 'telegram') {
      const events = literalStringArray(fields?.events);
      triggers.push({
        id: trigger.id,
        type,
        ...(events.length === 0 ? {} : { scope: events.join(', ') }),
      });
      continue;
    }
    if (type === 'discord') {
      const events = literalStringArray(fields?.events);
      const channels = literalStringArray(fields?.channels);
      const scope = [
        events.length === 0 ? undefined : events.join(', '),
        channels.length === 0 ? undefined : `channels: ${channels.join(', ')}`,
      ]
        .filter((value): value is string => value !== undefined)
        .join(' · ');
      triggers.push({
        id: trigger.id,
        type,
        ...(scope.length === 0 ? {} : { scope }),
      });
      continue;
    }
    if (type === 'whatsapp') {
      const events = literalStringArray(fields?.events);
      triggers.push({
        id: trigger.id,
        type,
        url: `http://${host}:${port}${WOML_WHATSAPP_CALLBACK_PATH}`,
        ...(events.length === 0 ? {} : { scope: events.join(', ') }),
        warning:
          'Meta requires this callback to be reachable through public HTTPS in production.',
      });
      continue;
    }
    if (type === 'schedule') {
      triggers.push({
        id: trigger.id,
        type,
        ...(literalString(fields?.cron) === undefined
          ? {}
          : { schedule: literalString(fields?.cron)! }),
        ...(literalString(fields?.timezone) === undefined
          ? {}
          : { timezone: literalString(fields?.timezone)! }),
        ...(scheduleDue.get(dueIdentity(workflow.workflowId, trigger.id)) ===
        undefined
          ? {}
          : {
              nextDueAt: scheduleDue.get(
                dueIdentity(workflow.workflowId, trigger.id)
              )!,
            }),
      });
      continue;
    }
    if (type === 'interval') {
      const interval = readableInterval(literalNumber(fields?.everyMs));
      const nextDueAt = intervalDue.get(
        dueIdentity(workflow.workflowId, trigger.id)
      );
      triggers.push({
        id: trigger.id,
        type,
        ...(interval === undefined ? {} : { interval }),
        ...(nextDueAt === undefined ? {} : { nextDueAt }),
      });
      continue;
    }
    triggers.push({ id: trigger.id, type });
  }
  return {
    id: workflow.workflowId,
    ...(workflow.metadata?.name === undefined
      ? {}
      : { name: workflow.metadata.name }),
    ...(workflow.metadata?.description === undefined
      ? {}
      : { description: workflow.metadata.description }),
    ...(workflow.metadata?.version === undefined
      ? {}
      : { version: workflow.metadata.version }),
    definitionHash,
    triggers,
  };
}

async function resolvedSecrets(
  workflow: CompiledWorkflowDefinition,
  store: SecretStore,
  runtimeCapabilities?: RuntimeCapabilitiesV1
): Promise<Readonly<Record<string, string>>> {
  const references = [...workflowSecretReferences(workflow)];
  await preflightSecretReferences(references, store);
  return resolvedNamedSecrets(
    [
      ...references.map(reference => reference.name),
      ...artifactPolicySecretNames(runtimeCapabilities),
    ],
    store
  );
}

async function resolvedNamedSecrets(
  names: readonly string[],
  store: SecretStore
): Promise<Readonly<Record<string, string>>> {
  const values: Record<string, string> = {};
  for (const name of new Set(names)) {
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
      if (!(
        error instanceof Error &&
        'code' in error &&
        error.code === 'ENOENT'
      ))
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
  | 'telegram'
  | 'discord'
  | 'whatsapp'
  | 'schedule'
  | 'interval'
  | 'event'
  | undefined {
  const type = handler.startsWith('trigger.') ? handler.slice(8) : handler;
  return [
    'manual',
    'webhook',
    'slack',
    'telegram',
    'discord',
    'whatsapp',
    'schedule',
    'interval',
    'event',
  ].includes(type)
    ? (type as
        | 'manual'
        | 'webhook'
        | 'slack'
        | 'telegram'
        | 'discord'
        | 'whatsapp'
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
  if (args.runtimeCapabilities !== undefined) {
    if (args.runtimeCapabilities.process !== undefined) {
      io.stderr(
        `${processGrantSummary({ process: args.runtimeCapabilities.process })}\n`
      );
    }
    if (args.runtimeCapabilities.artifacts !== undefined) {
      io.stderr(
        `${artifactGrantSummary({ artifacts: args.runtimeCapabilities.artifacts })}\n`
      );
    }
  }
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

  const productionSources = sources;
  const manualTargets = activeManualTargets(sources, args.triggerId);
  const hasOtherIngress = hasNonManualRuntimeIngress(sources);
  const background = process.env.WOML_BACKGROUND_HANDOFF !== undefined;
  let manualInput: ManualLineInput | undefined;
  if (manualTargets.length > 0 && background && !hasOtherIngress) {
    throw new CliInputError(
      'WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE',
      'a manual-only workflow cannot run in the background because nobody can press Enter. Add another trigger or run it in the foreground.'
    );
  }
  if (manualTargets.length > 0 && !background) {
    const candidate = dependencies.createManualInput?.();
    if (candidate?.isTTY === true) {
      manualInput = candidate;
    } else {
      candidate?.close();
      if (!hasOtherIngress) {
        throw new CliInputError(
          'WOML_MANUAL_TRIGGER_TTY_REQUIRED',
          'woml run needs an interactive terminal for a manual-only workflow. Use "woml test <workflow.woml>" for one non-interactive run.'
        );
      }
    }
  }

  let runtimeId: string | undefined;
  let communicationTriggerHost: CommunicationTriggerHost | undefined;
  let slackTransport: SharedSlackTransport | undefined;
  let telegramTransport: SharedTelegramTransport | undefined;
  let discordTransport: SharedDiscordTransport | undefined;
  let runtimeControl: RuntimeControlHandle | undefined;
  let observability: RuntimeObservability | undefined;
  let openTelemetryRuntime: OpenTelemetryRuntime | undefined;
  let openTelemetryModule: OpenTelemetryRuntimeModule | undefined;
  let telemetryObservationProjector: TelemetryObservationProjector | undefined;
  const pendingTelemetrySignals: TelemetrySignalV1[] = [];
  const pendingTelemetryLinks: TelemetryAdmissionLinkV1[] = [];
  const observedTelemetrySignalIds = new Set<string>();
  let operationalOutput: RuntimeOperationalOutput | undefined;
  let foregroundOutputCursor = 0;
  let foregroundOutputDrain = Promise.resolve();
  let automaticRetention: AutomaticRetentionHandle | undefined;
  const pendingObservabilityProgress: unknown[] = [];
  const scheduleDue = new Map<string, string>();
  const intervalDue = new Map<string, string>();
  let slackWorkspaceId: string | undefined;
  const foregroundPresentation = new ForegroundPresentation({
    io,
    render: presentationRenderOptions(
      args,
      io,
      manualTargets.length > 0 && manualInput === undefined
        ? 'Unavailable without an interactive foreground terminal'
        : undefined
    ),
    verbose: args.verbose,
    inspectRun: runId =>
      inspectRunPresentationWithRust(args.statePath, runId, {
        nativeCorePath: dependencies.nativeCorePath,
      }),
  });
  const manualRunIds = new Set<string>();
  const manualPromptOptions: PresentationRenderOptions = {
    ...presentationRenderOptions(args, io),
    format: io.isTTY === true ? 'tty' : 'plain',
  };
  const printManualPrompt = (): void => {
    if (manualInput === undefined) return;
    io.stderr(
      `\n${renderManualTargetSelection(manualTargets, manualPromptOptions)}`
    );
  };
  let manualInputTask: Promise<void> | undefined;
  const observe = (progress: unknown): void => {
    if (observability === undefined) {
      if (pendingObservabilityProgress.length < 1024)
        pendingObservabilityProgress.push(progress);
      return;
    }
    observability.recordProgress(progress);
  };
  const observeOperationalOutput = (
    wakeup: OperationalOutputWakeupV1
  ): void => {
    operationalOutput?.notify(wakeup);
    if (background || wakeup.newestSequence <= foregroundOutputCursor) return;
    foregroundOutputDrain = foregroundOutputDrain
      .then(() => {
        while (foregroundOutputCursor < wakeup.newestSequence) {
          const page = readOperationalOutputPageWithRust(
            args.statePath,
            { after: foregroundOutputCursor, limit: 250 },
            { nativeCorePath: dependencies.nativeCorePath }
          );
          for (const item of page.records) {
            io.stderr(
              `${renderOperationalRecord(item, {
                color: args.color,
                includeRunLabel: productionSources.length > 1,
                unicode: process.env.TERM !== 'dumb',
                isTTY: io.isTTY === true,
                width: process.stderr.columns,
              })}\n`
            );
          }
          const previous = foregroundOutputCursor;
          foregroundOutputCursor = Math.max(
            foregroundOutputCursor,
            page.nextAfter
          );
          if (!page.hasMore || foregroundOutputCursor === previous) break;
        }
      })
      .catch(error => {
        foregroundPresentation.warning(
          'WOML_OUTPUT_STREAM_UNAVAILABLE',
          error instanceof Error ? error.message : String(error)
        );
      });
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
      const hasSignalWait = productionSources.some(
        source =>
          'waits' in source.workflow.graph &&
          source.workflow.graph.waits?.some(wait => wait.kind === 'signal')
      );
      const signalIngressToken = hasSignalWait
        ? await store.get('WOML_SIGNAL_TOKEN')
        : undefined;
      for (const source of productionSources) {
        if (source.runtimeModules.length > 0) {
          const publicModules = source.runtimeModules.filter(
            module => !module.name.startsWith('__woml_')
          );
          if (publicModules.length > 0) {
            foregroundPresentation.verbose(
              `WOML modules ready for ${source.workflow.workflowId}: ${publicModules.map(module => `services.${module.name}`).join(', ')}.\n`
            );
          }
        }
      }
      const eventRoutes = productionSources
        .flatMap(source => eventRouteSummaries(source.workflow))
        .filter(route => route.publicEndpoint);
      const registrations = await profileAsync(
        'runtime',
        'runtime.resolve_registrations',
        async measurements => {
          if (measurements !== undefined) {
            measurements.counts.workflows = productionSources.length;
          }
          return await Promise.all(
            productionSources.map(async source => ({
              workflow: source.workflow,
              definitionHash: source.definitionHash,
              resolvedSecrets: await resolvedSecrets(
                source.workflow,
                store,
                args.runtimeCapabilities
              ),
              runtimeModules: source.runtimeModules,
            }))
          );
        }
      );
      const activationRequirements = profileSync(
        'runtime',
        'runtime.resolve_activation_requirements',
        measurements => {
          const requirements = runtimeActivationRequirements(
            productionSources.map(source => source.workflow)
          );
          if (measurements !== undefined) {
            measurements.counts.provider_hosts =
              Number(requirements.triggerHandlers.includes('trigger.slack')) +
              Number(requirements.providers.telegram.inbound) +
              Number(requirements.providers.discord.inbound);
            measurements.counts.public_http = Number(requirements.publicHttp);
            measurements.counts.script_execution = Number(
              requirements.scriptExecution
            );
            measurements.counts.runtime_modules = Number(
              requirements.runtimeModules
            );
          }
          return requirements;
        }
      );
      if (
        activationRequirements.remoteExecution !== undefined &&
        args.targetPreflight === undefined
      ) {
        throw new CliInputError(
          'WOML_TARGET_UNAUTHORIZED',
          'atomic runtime activation refused to open admission because remote target preflight was not completed.'
        );
      }
      const routes = productionSources.flatMap(source =>
        webhookRouteSummaries(source.workflow)
      );
      const slackRegistrations = productionSources.flatMap(source =>
        slackTriggerRegistrations(source.workflow, source.definitionHash)
      );
      const telegramRegistrations = productionSources.flatMap(source =>
        telegramTriggerRegistrations(source.workflow, source.definitionHash)
      );
      const discordRegistrations = productionSources.flatMap(source =>
        discordTriggerRegistrations(source.workflow, source.definitionHash)
      );
      const telegramPollingCredentials = activationRequirements.providers
        .telegram.inbound
        ? activationRequirements.providers.telegram.inboundCredentialNames
        : [];
      const discordGatewayCredentials = activationRequirements.providers.discord
        .inbound
        ? activationRequirements.providers.discord.inboundCredentialNames
        : [];
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
      const hasWhatsAppEndpoint =
        activationRequirements.providers.whatsapp.inbound;
      const hasHttpEndpoint =
        activationRequirements.publicHttp ||
        routes.length > 0 ||
        uniqueEventRoutes.length > 0 ||
        hasWhatsAppEndpoint ||
        signalIngressToken !== undefined;
      const currentActivationId = activationIdentity(productionSources);
      const currentDeploymentId = deploymentIdentity(args.statePath);
      if (args.openTelemetry?.enabled === true) {
        const runtimeModulePath = new URL(
          `opentelemetry-runtime.${import.meta.url.endsWith('.ts') ? 'ts' : 'js'}`,
          import.meta.url
        ).href;
        openTelemetryModule = (await import(
          runtimeModulePath
        )) as OpenTelemetryRuntimeModule;
        telemetryObservationProjector =
          openTelemetryModule.createTelemetryObservationProjector();
      }
      const runtime = await profileAsync(
        'runtime',
        'runtime.start_core_services',
        async measurements => {
          if (measurements !== undefined) {
            measurements.counts.workflows = registrations.length;
            measurements.counts.http_endpoints = hasHttpEndpoint ? 1 : 0;
          }
          return await startWebhookRuntimeWithRust(
            registrations,
            args.statePath,
            {
              nativeCorePath: dependencies.nativeCorePath,
              host: hasHttpEndpoint ? args.host : '127.0.0.1',
              port: hasHttpEndpoint ? args.port : 0,
              deploymentId: currentDeploymentId,
              activationId: currentActivationId,
              shutdownTimeoutMs: args.shutdownTimeoutMs,
              startSuspended: true,
              runtimeCapabilities: args.runtimeCapabilities,
              signalIngressToken,
              telemetryEnabled: args.openTelemetry?.enabled === true,
              telemetryPropagation:
                args.openTelemetry?.enabled === true &&
                args.openTelemetry.propagation,
              telemetrySamplingRatio:
                args.openTelemetry === undefined
                  ? 0
                  : 'ratio' in args.openTelemetry.sampler
                    ? args.openTelemetry.sampler.ratio
                    : args.openTelemetry.sampler.name === 'always_on'
                      ? 1
                      : 0,
              decodeTelemetrySignal: openTelemetryModule?.parseTelemetrySignal,
              decodeTelemetryAdmissionLink:
                openTelemetryModule?.parseTelemetryAdmissionLink,
              onOperationalOutput: observeOperationalOutput,
              onTelemetrySignal: signal => {
                if (observability !== undefined) {
                  const observation = telemetryObservationProjector?.observe(signal);
                  if (observation !== undefined) {
                    observability.recordTelemetryObservation(observation);
                  }
                  if (openTelemetryRuntime === undefined) {
                    observedTelemetrySignalIds.add(signal.signalId);
                  }
                }
                if (openTelemetryRuntime !== undefined) {
                  openTelemetryRuntime.record(signal);
                } else if (
                  pendingTelemetrySignals.length <
                  (args.openTelemetry?.maxQueueSize ?? 0)
                ) {
                  pendingTelemetrySignals.push(signal);
                }
              },
              onTelemetryAdmissionLink: link => {
                if (openTelemetryRuntime !== undefined) {
                  openTelemetryRuntime.recordAdmissionLink(link);
                } else if (
                  pendingTelemetryLinks.length <
                  (args.openTelemetry?.maxQueueSize ?? 0)
                ) {
                  pendingTelemetryLinks.push(link);
                }
              },
              onTriggerProgress: progress => {
                observe(progress);
                if (
                  progress.type === 'occurrence_accepted' &&
                  progress.triggerHandler === 'trigger.manual'
                ) {
                  manualRunIds.add(progress.runId);
                }
                foregroundPresentation.trigger(progress);
                if (
                  progress.type === 'run_terminal' &&
                  manualRunIds.delete(progress.runId)
                ) {
                  printManualPrompt();
                }
              },
              onScheduleProgress: progress => {
                observe(progress);
                if (progress.type === 'next_due') {
                  scheduleDue.set(
                    dueIdentity(progress.workflowId, progress.triggerId),
                    progress.nextScheduledAt
                  );
                }
                foregroundPresentation.schedule(progress);
              },
              onIntervalProgress: progress => {
                observe(progress);
                if (progress.type === 'next_due') {
                  intervalDue.set(
                    dueIdentity(progress.workflowId, progress.triggerId),
                    progress.nextScheduledAt
                  );
                }
                foregroundPresentation.interval(progress);
              },
              onWorkflowCallProgress: progress => {
                observe(progress);
                foregroundPresentation.workflowCall(progress);
              },
              onRuntimePolicyProgress: progress => {
                observe(progress);
                foregroundPresentation.execution(progress);
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
        }
      );
      runtimeId = runtime.runtimeId;
      operationalOutput = new RuntimeOperationalOutput({
        deploymentId: operationalOutputDeploymentIdentity(args.statePath),
        hasRun: runId => {
          try {
            inspectRunPresentationWithRust(args.statePath, runId, {
              nativeCorePath: dependencies.nativeCorePath,
            });
            return true;
          } catch (error) {
            if (
              error instanceof RunManagementError &&
              error.code === 'WOML_RUN_NOT_FOUND'
            ) return false;
            throw error;
          }
        },
        hasWorkflow: workflowId =>
          hasWorkflowDefinitionWithRust(args.statePath, workflowId, {
            nativeCorePath: dependencies.nativeCorePath,
          }),
        readPage: (runId, after, limit) =>
          readOperationalOutputPageWithRust(
            args.statePath,
            { ...(runId === undefined ? {} : { runId }), after, limit },
            { nativeCorePath: dependencies.nativeCorePath }
          ),
        readProgress: runId =>
          inspectOperationalProgressWithRust(args.statePath, runId, {
            nativeCorePath: dependencies.nativeCorePath,
          }),
        readObservation: () =>
          observeOperationalOutputWithRust(args.statePath, {
            nativeCorePath: dependencies.nativeCorePath,
          }),
      });
      const artifactAuthority =
        args.runtimeCapabilities?.artifacts === undefined
          ? undefined
          : { artifacts: args.runtimeCapabilities.artifacts };
      const artifactObservationSecrets =
        artifactAuthority === undefined
          ? undefined
          : await resolvedNamedSecrets(
              artifactPolicySecretNames(artifactAuthority),
              store
            );
      profileSync(
        'runtime',
        'runtime.initialize_observability',
        measurements => {
          if (measurements !== undefined) {
            measurements.counts.workflows = productionSources.length;
            measurements.counts.pending_progress =
              pendingObservabilityProgress.length;
          }
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
            ...(telegramPollingCredentials.length === 0
              ? []
              : [
                  {
                    name: 'telegram',
                    kind: 'provider' as const,
                    status: 'unready' as const,
                  },
                ]),
            ...(discordGatewayCredentials.length === 0
              ? []
              : [
                  {
                    name: 'discord',
                    kind: 'provider' as const,
                    status: 'unready' as const,
                  },
                ]),
            ...(!hasWhatsAppEndpoint
              ? []
              : [
                  {
                    name: 'whatsapp',
                    kind: 'provider' as const,
                    status: 'ready' as const,
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
            ...(args.runtimeCapabilities?.artifacts === undefined
              ? []
              : [
                  {
                    name: `artifact-${args.runtimeCapabilities.artifacts.policy.backend.kind}`,
                    kind: 'store' as const,
                    status: 'ready' as const,
                  },
                ]),
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
            observeLocks: () =>
              observeLocksV1WithRust(args.statePath, {
                nativeCorePath: dependencies.nativeCorePath,
              }),
            observeOutput: () =>
              observeOperationalOutputWithRust(args.statePath, {
                nativeCorePath: dependencies.nativeCorePath,
              }),
            outputFollowers: () => operationalOutput?.followerCount ?? 0,
            ...(artifactAuthority === undefined ||
            artifactObservationSecrets === undefined
              ? {}
              : {
                  observeArtifacts: () =>
                    observeArtifactsWithRust(
                      args.statePath,
                      artifactAuthority,
                      artifactObservationSecrets,
                      { nativeCorePath: dependencies.nativeCorePath }
                    ),
                }),
            storeSize: () => durableStoreSize(args.statePath),
            logFormat: args.logFormat,
            emitLog:
              args.verbose ||
              args.logFormat === 'json' ||
              process.env.WOML_BACKGROUND_LOG !== undefined
                ? io.stderr
                : () => {},
            components: componentRecords,
          });
          observability.setLifecycle('recovering');
          for (const progress of pendingObservabilityProgress)
            observability.recordProgress(progress);
          pendingObservabilityProgress.length = 0;
        }
      );
      if (args.openTelemetry?.enabled === true) {
        if (openTelemetryModule === undefined) {
          throw new Error('OpenTelemetry runtime host was not loaded.');
        }
        const { OpenTelemetryRuntime: RuntimeHost } = openTelemetryModule;
        openTelemetryRuntime = await RuntimeHost.start(
          args.openTelemetry,
          {
            runtimeInstanceId: runtime.runtimeId,
            deploymentId: currentDeploymentId,
            runtimeVersion: WOML_CLI_VERSION,
            ...(dependencies.loadOpenTelemetrySdk === undefined
              ? {}
              : { loadSdk: dependencies.loadOpenTelemetrySdk }),
            onHealth: health => observability?.setOpenTelemetryHealth(health),
            onDiagnostic: (code, message) =>
              observability?.alert(code, message, 'warn'),
          }
        );
        for (const link of pendingTelemetryLinks.splice(0)) {
          openTelemetryRuntime.recordAdmissionLink(link);
        }
        for (const signal of pendingTelemetrySignals.splice(0)) {
          if (!observedTelemetrySignalIds.delete(signal.signalId)) {
            const observation = telemetryObservationProjector?.observe(signal);
            if (observation !== undefined) {
              observability?.recordTelemetryObservation(observation);
            }
          }
          openTelemetryRuntime.record(signal);
        }
      }
      runtimeControl = await profileAsync(
        'runtime',
        'runtime.start_control',
        async () =>
          await startRuntimeControl({
            runtimeInstanceId: runtime.runtimeId,
            deploymentId: currentDeploymentId,
            host: args.adminHost,
            port: args.adminPort,
            observability,
            output: operationalOutput,
            healthEnabled: args.observabilityHealth,
            metricsEnabled: args.observabilityMetrics,
            presentations: {
              run: runId =>
                inspectRunPresentationWithRust(args.statePath, runId, {
                  nativeCorePath: dependencies.nativeCorePath,
                }),
              workflow: (workflowId, limit) =>
                listRunPresentationsWithRust(
                  args.statePath,
                  workflowId,
                  limit,
                  { nativeCorePath: dependencies.nativeCorePath }
                ),
            },
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
                const result = cancelRunWithRust(
                  args.statePath,
                  runId,
                  commandId,
                  {
                    nativeCorePath: dependencies.nativeCorePath,
                  }
                );
                return result.code;
              },
              signal: request =>
                submitExternalSignalWithRust(
                  runtime.runtimeId,
                  {
                    profile: 'woml.external-signal-admission/v1',
                    signal: request.signal,
                    runId: request.runId,
                    waitId: request.waitId,
                    correlation: request.correlation,
                    idempotencyKey: request.idempotencyKey,
                    payload: request.payload as JsonValue,
                    receivedAt: new Date().toISOString(),
                    authenticationClass: 'local_control',
                  },
                  { nativeCorePath: dependencies.nativeCorePath }
                ),
            },
          })
      );

      const communicationProviders = new CommunicationProviderRegistry();
      if (slackRegistrations.length > 0) {
        slackTransport =
          dependencies.createSlackTransport?.({
            log: message => foregroundPresentation.verbose(message),
            onConnectionState: status => {
              if (status.state === 'reconnecting') {
                foregroundPresentation.warning(
                  'WOML_SLACK_RECONNECTING',
                  `Slack Socket Mode is reconnecting${status.retryAt === undefined ? '.' : ` at ${status.retryAt}.`}`
                );
              }
            },
          }) ??
          new SharedSlackTransport({
            log: message => foregroundPresentation.verbose(message),
            onConnectionState: status => {
              if (status.state === 'reconnecting') {
                foregroundPresentation.warning(
                  'WOML_SLACK_RECONNECTING',
                  `Slack Socket Mode is reconnecting${status.retryAt === undefined ? '.' : ` at ${status.retryAt}.`}`
                );
              }
            },
          });
        const slackHost = new SlackTriggerHost({
          registrations: slackRegistrations,
          secretStore: store,
          transport: slackTransport,
          submit: ingress =>
            submitTriggerOccurrenceWithRust(runtime.runtimeId, ingress, {
              nativeCorePath: dependencies.nativeCorePath,
            }),
          emit: message => {
            if (message.messageType === 'connection') {
              slackWorkspaceId = message.workspaceId;
            }
            const formatted = formatSlackTriggerMessage(message);
            if (message.messageType === 'failure') {
              foregroundPresentation.warning(message.code, message.message);
            } else if (formatted !== undefined) {
              foregroundPresentation.verbose(formatted);
            }
          },
          diagnostic: message =>
            foregroundPresentation.warning(
              'WOML_SLACK_TRIGGER_DIAGNOSTIC',
              message
            ),
        });
        communicationProviders.register({
          role: 'trigger',
          adapter: slackHost,
        });
      }
      if (telegramPollingCredentials.length > 0) {
        telegramTransport =
          dependencies.createTelegramTransport?.({
            log: message => foregroundPresentation.verbose(message),
            onFatal: failure => {
              foregroundPresentation.warning(failure.code, failure.message);
              resolveRuntimeUnavailable();
            },
          }) ??
          new SharedTelegramTransport({
            log: message => foregroundPresentation.verbose(message),
            onFatal: failure => {
              foregroundPresentation.warning(failure.code, failure.message);
              resolveRuntimeUnavailable();
            },
          });
        const telegramHost = new TelegramTriggerHost({
          registrations: telegramRegistrations,
          credentialNames: telegramPollingCredentials,
          secretStore: store,
          transport: telegramTransport,
          submit: ingress =>
            submitTriggerOccurrenceWithRust(runtime.runtimeId, ingress, {
              nativeCorePath: dependencies.nativeCorePath,
            }),
          resolveApproval: update => {
            try {
              const result = resolveNotificationApprovalWithRust(
                args.statePath,
                update.decisionCapability,
                update.decision,
                {
                  nativeCorePath: dependencies.nativeCorePath,
                  providerActorId: `telegram:${update.actorId}`,
                }
              );
              foregroundPresentation.verbose(
                `Telegram approval ${result.approvalId} was ${update.decision}.`
              );
              return result.status === 'accepted'
                ? 'accepted'
                : 'already-resolved';
            } catch (error) {
              if (error instanceof ApprovalDecisionError) {
                foregroundPresentation.warning(error.code, error.message);
                return error.code === 'WOML_APPROVAL_DECISION_CONFLICT'
                  ? 'already-resolved'
                  : 'expired';
              }
              throw error;
            }
          },
          diagnostic: (code, message) => {
            if (code === 'WOML_TELEGRAM_READY') {
              foregroundPresentation.verbose(message);
            } else {
              foregroundPresentation.verbose(`${code}: ${message}`);
            }
          },
        });
        communicationProviders.register({
          role: 'trigger',
          adapter: telegramHost,
        });
      }
      if (discordGatewayCredentials.length > 0) {
        discordTransport =
          dependencies.createDiscordTransport?.({
            log: message => foregroundPresentation.verbose(message),
            onFatal: failure => {
              foregroundPresentation.warning(failure.code, failure.message);
              resolveRuntimeUnavailable();
            },
          }) ??
          new SharedDiscordTransport({
            log: message => foregroundPresentation.verbose(message),
            onFatal: failure => {
              foregroundPresentation.warning(failure.code, failure.message);
              resolveRuntimeUnavailable();
            },
          });
        const discordHost = new DiscordTriggerHost({
          registrations: discordRegistrations,
          credentialNames: discordGatewayCredentials,
          secretStore: store,
          transport: discordTransport,
          submit: ingress =>
            submitTriggerOccurrenceWithRust(runtime.runtimeId, ingress, {
              nativeCorePath: dependencies.nativeCorePath,
            }),
          resolveApproval: update => {
            try {
              const result = resolveNotificationApprovalWithRust(
                args.statePath,
                update.decisionCapability,
                update.decision,
                {
                  nativeCorePath: dependencies.nativeCorePath,
                  providerActorId: `discord:${update.actorId}`,
                }
              );
              foregroundPresentation.verbose(
                `Discord approval ${result.approvalId} was ${update.decision}.`
              );
              return result.status === 'accepted'
                ? 'accepted'
                : 'already-resolved';
            } catch (error) {
              if (error instanceof ApprovalDecisionError) {
                foregroundPresentation.warning(error.code, error.message);
                return error.code === 'WOML_APPROVAL_DECISION_CONFLICT'
                  ? 'already-resolved'
                  : 'expired';
              }
              throw error;
            }
          },
          diagnostic: (code, message) => {
            if (code === 'WOML_DISCORD_READY') {
              foregroundPresentation.verbose(message);
            } else {
              foregroundPresentation.verbose(`${code}: ${message}`);
            }
          },
        });
        communicationProviders.register({
          role: 'trigger',
          adapter: discordHost,
        });
      }
      await profileAsync(
        'runtime',
        'runtime.start_provider_hosts',
        async measurements => {
          const providerCount =
            communicationProviders.providers('trigger').length;
          if (measurements !== undefined)
            measurements.counts.providers = providerCount;
          if (providerCount > 0) {
            communicationTriggerHost = new CommunicationTriggerHost(
              communicationProviders.triggerAdapters()
            );
            try {
              await communicationTriggerHost.start();
              if (slackRegistrations.length > 0) {
                observability?.setComponent('slack', 'provider', 'ready');
              }
              if (telegramPollingCredentials.length > 0) {
                observability?.setComponent('telegram', 'provider', 'ready');
              }
              if (discordGatewayCredentials.length > 0) {
                observability?.setComponent('discord', 'provider', 'ready');
              }
            } catch (error) {
              const discordFailure = discordStartupError(error);
              const telegramFailure = telegramStartupError(error);
              const slackFailure = slackTriggerStartupError(error);
              const failure =
                discordFailure.code !== 'WOML_DISCORD_TRIGGER_UNAVAILABLE'
                  ? discordFailure
                  : telegramFailure.code !== 'WOML_TELEGRAM_TRIGGER_UNAVAILABLE'
                    ? telegramFailure
                    : slackFailure;
              throw new CliInputError(failure.code, failure.message);
            }
          }
        }
      );

      // Provider startup may take long enough for an editor or generator to
      // rewrite a source. Never open admission for a mixed activation.
      await profileAsync(
        'runtime',
        'runtime.final_source_revalidation',
        async measurements => {
          if (measurements !== undefined)
            measurements.counts.inputs = activationInputPaths.length;
          await assertStableSourceSnapshot(productionSources);
          await assertStableWorkflowInputSet(
            args.inputPaths,
            activationInputPaths
          );
        }
      );
      await profileAsync('runtime', 'runtime.open_admission', async () => {
        await activateWebhookRuntimeWithRust(runtime.runtimeId, {
          nativeCorePath: dependencies.nativeCorePath,
        });
      });
      observability?.setLifecycle('ready');
      observability?.log(
        'info',
        'WOML_RUNTIME_READY',
        `WOML runtime is ready with ${productionSources.length} workflow${productionSources.length === 1 ? '' : 's'}.`
      );
      foregroundPresentation.verbose(
        `Deployment activation ${currentActivationId.slice(7, 19)} is ready with ${productionSources.length} workflow(s).`
      );
      if (signalIngressToken !== undefined) {
        const signalBaseUrl = `http://${args.host}:${args.port}`;
        io.stderr(
          [
            '',
            'Durable signal ingress is enabled.',
            `Endpoint: POST ${signalBaseUrl}/v1/signals/<signal>`,
            'Use the stored WOML_SIGNAL_TOKEN without printing it:',
            `curl -X POST "${signalBaseUrl}/v1/signals/<signal>" -H "Authorization: Bearer \$WOML_SIGNAL_TOKEN" -H "Content-Type: application/json" -d '{"runId":"<run-id>","waitId":"<wait-id>","correlation":"<correlation>","idempotencyKey":"<unique-key>","payload":{}}'`,
            '',
          ].join('\n')
        );
      }
      const automaticArtifactAuthority =
        args.runtimeCapabilities?.artifacts === undefined
          ? undefined
          : { artifacts: args.runtimeCapabilities.artifacts };
      const automaticArtifactSecrets =
        automaticArtifactAuthority === undefined
          ? undefined
          : await resolvedNamedSecrets(
              artifactPolicySecretNames(automaticArtifactAuthority),
              store
            );
      automaticRetention = profileSync(
        'runtime',
        'runtime.schedule_retention',
        measurements => {
          if (measurements !== undefined) {
            measurements.counts.enabled =
              args.retention?.enabled === true ? 1 : 0;
          }
          return startAutomaticRetention({
            statePath: args.statePath,
            configuration: args.retention,
            nativeCorePath: dependencies.nativeCorePath,
            ownerId: `retention_runtime_${runtime.runtimeId}`,
            ...(automaticArtifactAuthority === undefined
              ? {}
              : { artifactAuthority: automaticArtifactAuthority }),
            ...(automaticArtifactSecrets === undefined
              ? {}
              : { artifactSecrets: automaticArtifactSecrets }),
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
              foregroundPresentation.warning(code, formatError(error));
            },
          });
        }
      );
      if (automaticRetention.nextRunAt !== undefined) {
        foregroundPresentation.verbose(
          `Automatic retention is enabled; next maintenance at ${automaticRetention.nextRunAt}.`
        );
      }
      descriptorPath = runtimeDescriptorPath(args.statePath);
      await profileAsync('runtime', 'runtime.publish_descriptor', async () => {
        await runtimeControl!.publishDescriptor(descriptorPath!);
      });
      foregroundPresentation.verbose(
        `Inspect live runtime with woml inspect --state ${JSON.stringify(args.statePath)}.`
      );
      await profileAsync(
        'runtime',
        'runtime.report_ready',
        async measurements => {
          if (measurements !== undefined)
            measurements.counts.workflows = productionSources.length;
          await dependencies.onRuntimeReady?.({
            runtimeInstanceId: runtime.runtimeId,
            descriptorPath: descriptorPath!,
            workflowCount: productionSources.length,
          });
        }
      );
      for (const source of productionSources) {
        foregroundPresentation.startup(
          workflowPresentation(
            source.workflow,
            source.definitionHash,
            runtime.host,
            runtime.port,
            scheduleDue,
            intervalDue,
            slackWorkspaceId
          )
        );
      }

      if (manualInput !== undefined) {
        printManualPrompt();
        manualInputTask = manualInput.run(async line => {
          const value = line.trim();
          let target: ActiveManualTarget | undefined;
          if (manualTargets.length === 1) {
            if (value.length === 0) target = manualTargets[0];
          } else if (/^[1-9][0-9]*$/.test(value)) {
            target = manualTargets[Number(value) - 1];
          }
          if (target === undefined) {
            foregroundPresentation.warning(
              'WOML_MANUAL_TRIGGER_SELECTION_REQUIRED',
              manualTargets.length === 1
                ? 'Press Enter without typing a value to run the manual workflow.'
                : `Choose a manual trigger number from 1 through ${manualTargets.length}.`
            );
            printManualPrompt();
            return;
          }
          const outcome = await submitManualTriggerWithRust(
            runtime.runtimeId,
            {
              profile: 'woml.manual-trigger-admission/v1',
              type: 'request',
              requestId: `manual_request_${randomUUID().replaceAll('-', '')}`,
              workflowId: target.workflowId,
              triggerId: target.triggerId,
              payload: {},
              requestedAt: new Date().toISOString(),
            },
            { nativeCorePath: dependencies.nativeCorePath }
          );
          if (outcome.type === 'rejected') {
            foregroundPresentation.warning(outcome.code, outcome.message);
            if (outcome.code !== 'WOML_MANUAL_TRIGGER_ADMISSION_CLOSED') {
              printManualPrompt();
            }
          }
        });
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

    if (!args.json) {
      // This stable readiness receipt is also consumed by background handoff
      // and deployment supervisors.
      io.stderr('WOML automation is active. Press Ctrl+C to stop.\n');
    }
    await Promise.race([
      (dependencies.waitForShutdown ?? waitForShutdownSignal)(),
      runtimeUnavailable,
      ...(runtimeControl === undefined ? [] : [runtimeControl.stopRequested]),
      ...(manualInputTask === undefined ? [] : [manualInputTask]),
    ]);
  } finally {
    manualInput?.close();
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
    await communicationTriggerHost?.close().catch(() => {});
    await slackTransport?.close().catch(() => {});
    await telegramTransport?.close().catch(() => {});
    await discordTransport?.close().catch(() => {});
    await runtimeStop;
    await openTelemetryRuntime?.shutdown().catch(() => {});
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
  readonly createTelegramTransport?: (
    options: SharedTelegramTransportOptions
  ) => SharedTelegramTransport;
  readonly createDiscordTransport?: (
    options: SharedDiscordTransportOptions
  ) => SharedDiscordTransport;
  readonly fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  readonly inspectorTerminal?: InspectorTerminal;
  readonly createManualInput?: () => ManualLineInput;
  readonly waitForLogDetach?: () => Promise<void>;
  readonly loadOpenTelemetrySdk?: OpenTelemetrySdkLoader;
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
  createManualInput: createProcessManualLineInput,
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

async function runSignalCommand(
  args: readonly string[],
  io: CliIo,
  dependencies: CliDependencies
): Promise<number> {
  try {
    const signal = parseSignalArguments(args);
    let source: string;
    try {
      source = await readFile(signal.payloadPath, 'utf8');
    } catch {
      throw new CliInputError(
        'WOML_SIGNAL_PAYLOAD_NOT_FOUND',
        'signal payload file does not exist or cannot be read.'
      );
    }
    if (new TextEncoder().encode(source).byteLength > 1_048_576) {
      throw new CliInputError(
        'WOML_SIGNAL_PAYLOAD_TOO_LARGE',
        'signal payload exceeds the 1 MiB limit.'
      );
    }
    let payload: JsonValue;
    try {
      payload = JSON.parse(source) as JsonValue;
    } catch {
      throw new CliInputError(
        'WOML_SIGNAL_PAYLOAD_INVALID',
        'signal payload file must contain valid JSON.'
      );
    }
    const descriptor = await readRuntimeDescriptor(
      runtimeDescriptorPath(signal.statePath)
    );
    const outcome = await requestRuntimeSignal(
      descriptor,
      {
        signal: signal.signal,
        runId: signal.runId,
        waitId: signal.waitId,
        correlation: signal.correlation,
        idempotencyKey: signal.idempotencyKey,
        payload,
      },
      (dependencies.fetch ?? globalThis.fetch) as typeof globalThis.fetch
    );
    io.stdout(`${JSON.stringify(outcome)}\n`);
    const status =
      outcome !== null &&
      typeof outcome === 'object' &&
      'status' in outcome &&
      typeof outcome.status === 'string'
        ? outcome.status
        : undefined;
    return status === 'accepted' || status === 'duplicate' ? 0 : 1;
  } catch (error) {
    if (error instanceof CliInputError && error.message === signalUsage()) {
      io.stderr(`${signalUsage()}\n`);
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
  if (args.includes('--allow-process')) {
    io.stderr(
      'WOML_PROCESS_FOREGROUND_GRANT_BACKGROUND_DENIED: --allow-process is a temporary foreground grant. Use --process-policy for a background runtime.\n'
    );
    return 2;
  }
  const foregroundArgs = args.filter(
    argument => argument !== '--background' && argument !== '-d'
  );
  let parsed: RunArguments;
  try {
    parsed = await resolveRunArgumentsConfiguration(
      parseRunArguments(foregroundArgs),
      true
    );
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return error instanceof CliInputError ? 2 : 1;
  }
  const statePath = parsed.statePath;
  let workflowIds: readonly string[] = [];
  try {
    const sources = await compileWorkflowInputs(parsed.inputPaths);
    workflowIds = sources
      .map(source => source.workflow.workflowId)
      .sort((left, right) => left.localeCompare(right));
    const manualTargets = activeManualTargets(sources, parsed.triggerId);
    if (manualTargets.length > 0 && !hasNonManualRuntimeIngress(sources)) {
      throw new CliInputError(
        'WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE',
        'a manual-only workflow cannot run in the background because nobody can press Enter. Add another trigger or run it in the foreground.'
      );
    }
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return error instanceof CliInputError ? 2 : 1;
  }
  const descriptorPath = runtimeDescriptorPath(statePath);
  const logPath = runtimeLogPath(statePath, parsed.logDirectory);
  const handoffPath = join(
    dirname(statePath),
    `.background-${randomUUID()}.json`
  );
  const token =
    randomUUID().replaceAll('-', '') + randomUUID().replaceAll('-', '');
  await mkdir(dirname(handoffPath), { recursive: true, mode: 0o700 });
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
        const visibleWorkflows = workflowIds.slice(0, 5);
        const logCommands = visibleWorkflows.map(
          (workflowId, index) =>
            `${index === 0 ? 'Logs       ' : '           '}woml ${workflowId} --logs --state ${JSON.stringify(statePath)}`
        );
        if (workflowIds.length > visibleWorkflows.length) {
          logCommands.push(
            `           ${workflowIds.length - visibleWorkflows.length} more workflow(s); list them with woml list --state ${JSON.stringify(statePath)}`
          );
        }
        io.stdout(
          [
            'WOML runtime started in the background.',
            '',
            `Runtime    ${handoff.runtimeInstanceId}`,
            `PID        ${handoff.pid}`,
            `Workflows  ${handoff.workflowCount}`,
            ...logCommands,
            `Inspect    woml inspect --state ${JSON.stringify(statePath)}`,
            `Stop       woml stop --state ${JSON.stringify(statePath)}`,
            `Operations ${logPath}`,
            `Descriptor ${descriptorPath}`,
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

async function runCliUnprofiled(
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

  const commands = new Set([
    'init',
    'run',
    'test',
    'check',
    'target',
    'worker',
    'dispatch',
    'policy',
    'types',
    'secrets',
    'emit',
    'signal',
    'stop',
    'inspect',
    'backup',
    'restore',
    'prune',
    'list',
    'get',
    'cancel',
    'recover',
    'logs',
    'runs',
    'telegram',
    'discord',
    'whatsapp',
  ]);
  if (args[0] === 'init') {
    return await runInitCommand(args, io);
  }
  if (
    args[0] === 'logs' ||
    (args.includes('--logs') &&
      args[0] !== undefined &&
      !commands.has(args[0]))
  ) {
    try {
      const logArguments = await resolveLogFollowArguments(args);
      return await followWorkflowLogs({
        args: logArguments,
        io,
        dependencies: {
          nativeCorePath: dependencies.nativeCorePath,
          fetch: dependencies.fetch,
          waitForDetach: dependencies.waitForLogDetach,
        },
      });
    } catch (error) {
      io.stderr(`${formatError(error)}\n`);
      if (
        error instanceof LogFollowError &&
        error.code === 'WOML_CLI_ARGUMENTS_INVALID'
      ) {
        return 2;
      }
      return 1;
    }
  }

  if (args[0] === 'check') {
    return await runCheckCommand(args, io, dependencies);
  }

  if (args[0] === 'target') {
    return await runTargetCommand(args, io);
  }

  if (args[0] === 'worker') {
    return await runWorkerCommand(args, io, dependencies);
  }

  if (args[0] === 'dispatch') {
    return await runDispatchCommand(args, io, dependencies);
  }

  if (args[0] === 'policy') {
    return await runPolicyCommand(args, io);
  }

  if (args[0] === 'types') {
    return await runTypesCommand(args, io);
  }

  if (args[0] === 'secrets') {
    return await runSecretsCommand(args, io, dependencies);
  }

  if (args[0] === 'signal') {
    return await runSignalCommand(args, io, dependencies);
  }

  if (
    args[0] === 'telegram' ||
    args[0] === 'discord' ||
    args[0] === 'whatsapp'
  ) {
    let parsed;
    try {
      parsed = parseProviderDoctorArguments(args);
    } catch {
      io.stderr(`${providerDoctorUsage(args[0])}\n`);
      return 2;
    }
    try {
      const result = await runProviderDoctor(parsed, {
        secretStore: dependencies.createSecretStore(),
        fetch: (dependencies.fetch ??
          globalThis.fetch) as typeof globalThis.fetch,
      });
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : formatProviderDoctor(result, {
              color: parsed.color,
              isTTY: io.isTTY,
            })
      );
      return result.status === 'failed' ? 1 : 0;
    } catch (error) {
      io.stderr(`${formatError(error)}\n`);
      return 1;
    }
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

  if (args[0] === 'artifacts') {
    try {
      const parsed = parseArtifactCommandArguments(args);
      if (parsed.operation === 'inspect') {
        const inspection = inspectArtifactWithRust(
          parsed.statePath,
          parsed.artifactId,
          { nativeCorePath: dependencies.nativeCorePath }
        );
        io.stdout(
          parsed.json
            ? `${JSON.stringify(inspection)}\n`
            : `${formatArtifactInspection(inspection)}\n`
        );
        return 0;
      }
      const authority = await loadArtifactPolicy(
        parsed.artifactPolicyPath!,
        'foreground'
      );
      const secrets = await resolvedNamedSecrets(
        artifactPolicySecretNames(authority),
        dependencies.createSecretStore()
      );
      const result = exportArtifactWithRust(
        parsed.statePath,
        parsed.outputPath!,
        parsed.artifactId,
        authority,
        secrets,
        { nativeCorePath: dependencies.nativeCorePath }
      );
      io.stdout(
        parsed.json
          ? `${JSON.stringify(result)}\n`
          : [
              'WOML artifact exported and verified.',
              `Artifact: ${result.artifactId}`,
              `Name: ${result.name}`,
              `Size: ${result.size} bytes`,
              `Digest: ${result.digestPrefix}…`,
              `Audit: ${result.auditId}`,
            ].join('\n') + '\n'
      );
      return 0;
    } catch (error) {
      if (
        error instanceof CliInputError &&
        error.message === artifactsUsage()
      ) {
        io.stderr(`${artifactsUsage()}\n`);
        return 2;
      }
      io.stderr(`${formatError(error)}\n`);
      return error instanceof CliInputError ? 2 : 1;
    }
  }

  if (args[0] === 'backup') {
    try {
      const parsed = parseBackupArguments(args);
      const artifactAuthority =
        parsed.artifactPolicyPath === undefined
          ? undefined
          : await loadArtifactPolicy(parsed.artifactPolicyPath, 'foreground');
      const artifactSecrets =
        artifactAuthority === undefined
          ? undefined
          : await resolvedNamedSecrets(
              artifactPolicySecretNames(artifactAuthority),
              dependencies.createSecretStore()
            );
      const manifest = await createProductionBackup(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
        ...(artifactAuthority === undefined ? {} : { artifactAuthority }),
        ...(artifactSecrets === undefined ? {} : { artifactSecrets }),
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
              ...(manifest.artifacts === undefined
                ? []
                : [
                    `Artifacts: ${manifest.artifacts.blobCount} blobs · ${manifest.artifacts.totalBytes} bytes`,
                    `Artifact inventory: ${manifest.artifacts.inventoryDigest}`,
                  ]),
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
      const artifactAuthority =
        parsed.artifactPolicyPath === undefined
          ? undefined
          : await loadArtifactPolicy(parsed.artifactPolicyPath, 'foreground');
      const artifactSecrets =
        artifactAuthority === undefined
          ? undefined
          : await resolvedNamedSecrets(
              artifactPolicySecretNames(artifactAuthority),
              dependencies.createSecretStore()
            );
      const restored = await restoreProductionBackup(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
        ...(artifactAuthority === undefined ? {} : { artifactAuthority }),
        ...(artifactSecrets === undefined ? {} : { artifactSecrets }),
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
      const artifactAuthority =
        parsed.artifactPolicyPath === undefined
          ? undefined
          : await loadArtifactPolicy(parsed.artifactPolicyPath, 'foreground');
      const artifactSecrets =
        artifactAuthority === undefined
          ? undefined
          : await resolvedNamedSecrets(
              artifactPolicySecretNames(artifactAuthority),
              dependencies.createSecretStore()
            );
      const outcome = await runProductionRetention(parsed, {
        nativeCorePath: dependencies.nativeCorePath,
        ...(artifactAuthority === undefined ? {} : { artifactAuthority }),
        ...(artifactSecrets === undefined ? {} : { artifactSecrets }),
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
                `Eligible operational runs: ${outcome.output.eligibleRuns}`,
                `Estimated removable operational output: ${outcome.output.eligibleOutputBytes} bytes`,
                ...(outcome.artifacts === undefined
                  ? []
                  : [
                      `Eligible artifacts: ${outcome.artifacts.eligibleArtifacts}`,
                      `Estimated removable artifact bytes: ${outcome.artifacts.estimatedBytes}`,
                    ]),
                `Execute: woml prune --before ${parsed.before} --state ${JSON.stringify(parsed.statePath)}`,
              ].join('\n') + '\n'
        );
      } else {
        io.stdout(
          parsed.json
            ? `${JSON.stringify({
                ...outcome.result,
                output: outcome.output,
                ...(outcome.artifacts === undefined
                  ? {}
                  : { artifacts: outcome.artifacts }),
              })}\n`
            : [
                'WOML retention completed.',
                `Policy: ${outcome.result.policyId}`,
                `Deleted terminal runs: ${outcome.result.deletedRuns}`,
                `Removed logical history: ${outcome.result.deletedBytes} bytes`,
                `Batches: ${outcome.batches}`,
                `Durable state entries deleted: ${outcome.result.stateEntriesDeleted}`,
                `Deleted operational runs: ${outcome.output.prunedRuns}`,
                `Removed operational output: ${outcome.output.prunedOutputBytes} bytes`,
                ...(outcome.artifacts === undefined
                  ? []
                  : [
                      `Deleted artifacts: ${outcome.artifacts.deletedArtifacts}`,
                      `Deleted artifact blobs: ${outcome.artifacts.deletedBlobs}`,
                      `Removed artifact bytes: ${outcome.artifacts.deletedBytes}`,
                      `Integrity scrub: ${outcome.artifacts.scrubbedBlobs} checked · ${outcome.artifacts.corruptBlobs} corrupt`,
                      `Live artifact store: ${outcome.artifacts.liveArtifacts} artifacts · ${outcome.artifacts.liveBytes} bytes`,
                    ]),
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
      const artifactInspection = inspectRunArtifactsWithRust(
        get.statePath,
        get.runId,
        { nativeCorePath: dependencies.nativeCorePath }
      );
      const waitInspection = inspectWaitsV1WithRust(get.statePath, get.runId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      const lockInspection = inspectLocksV1WithRust(get.statePath, get.runId, {
        nativeCorePath: dependencies.nativeCorePath,
      });
      const correctionInspection = inspectCorrectionV1WithRust(
        get.statePath,
        get.runId,
        { nativeCorePath: dependencies.nativeCorePath }
      );
      let operationalSummary:
        | {
            readonly state: 'available' | 'unavailable';
            readonly latestSequence?: number;
            readonly activeProgress?: number;
            readonly progressLanes?: number;
          }
        | undefined;
      if (!get.json && existsSync(operationalOutputPath(get.statePath))) {
        try {
          const page = readOperationalOutputPageWithRust(
            get.statePath,
            { runId: get.runId, after: 0, limit: 1 },
            { nativeCorePath: dependencies.nativeCorePath }
          );
          const progress = inspectOperationalProgressWithRust(
            get.statePath,
            get.runId,
            { nativeCorePath: dependencies.nativeCorePath }
          );
          operationalSummary = {
            state: 'available',
            latestSequence: page.latestSequence,
            activeProgress: progress.filter(item => item.status === 'active')
              .length,
            progressLanes: progress.length,
          };
        } catch {
          operationalSummary = { state: 'unavailable' };
        }
      } else if (!get.json) {
        operationalSummary = {
          state: 'available',
          latestSequence: 0,
          activeProgress: 0,
          progressLanes: 0,
        };
      }
      const publicResult =
        result.profile === 'woml.run-inspection/v11'
          ? {
              ...result,
              profile: 'woml.run-inspection/v11' as const,
              artifacts: artifactInspection,
              ...(waitInspection.waits.length === 0
                ? {}
                : { durableWaits: waitInspection }),
              ...(lockInspection.length === 0 ? {} : { locks: lockInspection }),
              ...(correctionInspection === undefined
                ? {}
                : { correction: correctionInspection }),
            }
          : correctionInspection !== undefined
            ? {
                ...result,
                profile: 'woml.run-inspection/v10' as const,
                artifacts: artifactInspection,
                ...(waitInspection.waits.length === 0
                  ? {}
                  : { durableWaits: waitInspection }),
                ...(lockInspection.length === 0
                  ? {}
                  : { locks: lockInspection }),
                correction: correctionInspection,
              }
            : lockInspection.length > 0
              ? {
                  ...result,
                  profile: 'woml.run-inspection/v9' as const,
                  artifacts: artifactInspection,
                  ...(waitInspection.waits.length === 0
                    ? {}
                    : { durableWaits: waitInspection }),
                  locks: lockInspection,
                }
              : waitInspection.waits.length === 0
                ? {
                    ...result,
                    profile: 'woml.run-inspection/v7' as const,
                    artifacts: artifactInspection,
                  }
                : {
                    ...result,
                    profile: 'woml.run-inspection/v8' as const,
                    artifacts: artifactInspection,
                    durableWaits: waitInspection,
                  };
      io.stdout(
        get.json
          ? `${JSON.stringify(publicResult)}\n`
          : formatRunInspection(
              result,
              artifactInspection.artifacts,
              artifactInspection.truncated,
              waitInspection,
              lockInspection,
              correctionInspection,
              operationalSummary,
              get.statePath
            )
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

  if (args[0] === 'recover') {
    try {
      const recovery = parseRecoverArguments(args);
      const result = recoverCorrectionWithRust(
        recovery.statePath,
        {
          profile: 'woml.correction-recovery-request/v1',
          runId: recovery.runId,
          action: recovery.action,
          actionId: recovery.actionId,
          ...(recovery.note === undefined ? {} : { note: recovery.note }),
        },
        { nativeCorePath: dependencies.nativeCorePath }
      );
      io.stdout(
        recovery.json
          ? `${JSON.stringify(result)}\n`
          : result.status === 'retry_scheduled'
            ? `Correction retry scheduled for ${result.actionId} in run ${result.runId}.\n`
            : result.status === 'externally_resolved'
              ? `External correction resolution recorded for ${result.actionId} in run ${result.runId}.\n`
              : `Correction recovery was rejected for ${result.actionId} in run ${result.runId}.\n`
      );
      return result.status === 'rejected' ? 1 : 0;
    } catch (error) {
      if (error instanceof CliInputError && error.message === recoverUsage()) {
        io.stderr(`${recoverUsage()}\n`);
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
    runArguments = profileSync(
      'cli',
      'cli.parse_run_arguments',
      measurements => {
        if (measurements !== undefined)
          measurements.counts.arguments = args.length;
        return parseRunArguments(args);
      }
    );
  } catch (error) {
    const commandUsage = args[0] === 'test' ? testUsage() : runUsage();
    if (error instanceof CliInputError && error.message !== commandUsage) {
      io.stderr(`${error.message}\n`);
    }
    io.stderr(`${args.length === 0 ? usage() : commandUsage}\n`);
    return 2;
  }
  try {
    runArguments = await profileAsync(
      'cli',
      'cli.resolve_runtime_configuration',
      async () => await resolveRunArgumentsConfiguration(runArguments)
    );
  } catch (error) {
    io.stderr(`${formatError(error)}\n`);
    return 1;
  }
  const { filePath, inputPaths } = runArguments;

  if (runArguments.project !== undefined && runArguments.json) {
    const selection = {
      profile: 'woml.project-selection/v1' as const,
      project: runArguments.project.name,
      manifestPath: runArguments.project.manifestPath,
      digest: runArguments.project.digest,
      selectedBy: runArguments.projectSelection ?? 'explicit',
    };
    io.stdout(`${JSON.stringify(selection)}\n`);
  }

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
    if (
      sources.some(source =>
        inspectWomlModuleUsage(source.document).referencedServices.includes(
          'process'
        )
      ) &&
      runArguments.runtimeCapabilities?.process === undefined
    ) {
      throw new CliInputError(
        'WOML_PROCESS_DISABLED',
        'This workflow uses services.process, which is disabled by default. Use --allow-process <aliases> for a foreground run or --process-policy <path> for durable runtime authority.'
      );
    }
    if (
      sources.some(source =>
        inspectWomlModuleUsage(source.document).referencedServices.includes(
          'artifacts'
        )
      ) &&
      runArguments.runtimeCapabilities?.artifacts === undefined
    ) {
      throw new CliInputError(
        'WOML_ARTIFACT_SERVICE_DISABLED',
        'This workflow uses services.artifacts, which is disabled by default. Use --artifact-policy <path> to grant durable artifact authority.'
      );
    }
    printMigrationDiagnostics(
      io,
      sources.flatMap(source => source.migrationDiagnostics)
    );
    const activeInfrastructurePolicy =
      runArguments.runtimeCapabilities?.infrastructurePolicy;
    const compiledWorkflows = sources.map(source => source.workflow);
    const targetRegistry = await loadDefaultTargetRegistryForRemoteWorkflows(
      compiledWorkflows,
      runArguments.targetRegistry
    );
    const targetPreflight = preflightRemoteTargets(
      compiledWorkflows,
      targetRegistry,
      activeInfrastructurePolicy?.policy
    );
    if (targetPreflight !== undefined) {
      runArguments = { ...runArguments, targetRegistry, targetPreflight };
      if (!runArguments.json && runArguments.project === undefined) {
        io.stdout(
          `Remote targets ready: ${targetPreflight.targets.map(target => target.id).join(', ')} · ${targetPreflight.registryDigest}.\n`
        );
      }
    }
    if (
      activeInfrastructurePolicy !== undefined &&
      !runArguments.json &&
      runArguments.project === undefined
    ) {
      io.stdout(
        `Policy ${activeInfrastructurePolicy.policy.id}@${activeInfrastructurePolicy.policy.version} · ${activeInfrastructurePolicy.policy.environment}` +
          (runArguments.runtimeCapabilities?.emergencyDeny === undefined
            ? '\n'
            : ` · emergency deny ${runArguments.runtimeCapabilities.emergencyDeny.policy.id}@${runArguments.runtimeCapabilities.emergencyDeny.policy.version}\n`)
      );
    }
    if (runArguments.project !== undefined && !runArguments.json) {
      io.stdout(
        renderProjectActivationPresentation(
          createProjectActivationPresentation({
            project: runArguments.project.name,
            digest: runArguments.project.digest,
            workflows: sources.map(source => source.workflow.workflowId),
            statePath: runArguments.statePath,
            ...(activeInfrastructurePolicy === undefined
              ? {}
              : {
                  policy: `${activeInfrastructurePolicy.policy.id}@${activeInfrastructurePolicy.policy.version}`,
                }),
            targets:
              targetPreflight?.targets.map(target => target.id) ?? [],
            openTelemetry: runArguments.openTelemetry?.enabled === true,
          })
        )
      );
    }
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

export async function runCli(
  args: readonly string[],
  io: CliIo = processIo,
  dependencies: CliDependencies = defaultDependencies
): Promise<number> {
  try {
    return await profileAsync('cli', 'cli.run_command', async measurements => {
      if (measurements !== undefined)
        measurements.counts.arguments = args.length;
      return await runCliUnprofiled(args, io, dependencies);
    });
  } finally {
    await flushPerformanceProfile();
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
