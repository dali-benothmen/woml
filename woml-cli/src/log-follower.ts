import { stat } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  RunManagementError,
  hasWorkflowDefinitionWithRust,
  inspectRunPresentationWithRust,
  listRunPresentationsWithRust,
} from './rust-executor';
import {
  readRuntimeDescriptor,
  runtimeDescriptorPath,
  RuntimeControlError,
  type RuntimeDescriptorV1,
} from './runtime-control';
import { resolveRuntimeConfiguration } from './runtime-config';
import { consumeOperationsStream } from './operations-stream';
import {
  decodeRunPresentationListV1,
  decodeRunPresentationV1,
  renderPresentationWarning,
  renderRunPresentation,
  sanitizePresentationDiagnostic,
  type ColorMode,
  type PresentationRenderOptions,
  type RunPresentationListV1,
  type RunPresentationV1,
} from './terminal-presentation';

export const logFollowUsage =
  'Usage: woml <run-id|workflow-id> --logs [--state <path>] [--config <path>] [--json] [--color=auto|always|never]';

export class LogFollowError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'LogFollowError';
    this.code = code;
  }
}

export interface LogFollowArguments {
  readonly subject: string;
  readonly subjectKind: 'run' | 'workflow';
  readonly statePath: string;
  readonly json: boolean;
  readonly color: ColorMode;
}

interface ParsedLogFollowArguments extends LogFollowArguments {
  readonly stateExplicit: boolean;
  readonly configPath?: string;
}

export interface LogFollowIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
  readonly isTTY?: boolean;
  readonly columns?: number;
}

type LogFetch = (input: string, init?: RequestInit) => Promise<Response>;

interface RuntimeSnapshot {
  readonly runtimeInstanceId: string;
  readonly sequence: number;
  readonly workflows: readonly string[];
  readonly runs: readonly { readonly runId: string; readonly workflowId: string }[];
}

export interface LogFollowerDependencies {
  readonly nativeCorePath?: string;
  readonly fetch?: LogFetch;
  readonly readDescriptor?: (path: string) => Promise<RuntimeDescriptorV1>;
  readonly waitForDetach?: () => Promise<void>;
  readonly reconnectWindowMs?: number;
  readonly readRun?: (statePath: string, runId: string) => RunPresentationV1;
  readonly readWorkflow?: (
    statePath: string,
    workflowId: string,
    limit: number
  ) => RunPresentationListV1;
  readonly hasWorkflow?: (statePath: string, workflowId: string) => boolean;
}

const runIdPattern = /^run_[A-Za-z0-9_-]{1,252}$/;
const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;

function invalidArguments(): never {
  throw new LogFollowError('WOML_CLI_ARGUMENTS_INVALID', logFollowUsage);
}

export function parseLogFollowArguments(
  args: readonly string[]
): ParsedLogFollowArguments {
  const subject = args[0];
  if (subject === undefined) invalidArguments();
  const subjectKind = runIdPattern.test(subject)
    ? 'run'
    : workflowIdPattern.test(subject) && subject.length <= 256
      ? 'workflow'
      : undefined;
  if (subjectKind === undefined) {
    throw new LogFollowError(
      'WOML_LOG_SUBJECT_INVALID',
      'The log subject must be a WOML run ID or lowercase kebab-case workflow ID.'
    );
  }

  let statePath = resolve('.woml/state.sqlite');
  let stateExplicit = false;
  let configPath: string | undefined;
  let json = false;
  let color: ColorMode = 'auto';
  let logs = false;
  const seen = new Set<string>();
  for (let index = 1; index < args.length; index += 1) {
    const raw = args[index]!;
    const assignment = raw.startsWith('--color=')
      ? raw.slice('--color='.length)
      : undefined;
    const option = assignment === undefined ? raw : '--color';
    if (seen.has(option)) invalidArguments();
    seen.add(option);
    if (option === '--logs') {
      logs = true;
      continue;
    }
    if (option === '--json') {
      json = true;
      continue;
    }
    if (!['--state', '--config', '--color'].includes(option)) invalidArguments();
    const value = assignment ?? args[++index];
    if (value === undefined || value.length === 0 || value.startsWith('--')) {
      invalidArguments();
    }
    if (option === '--state') {
      statePath = resolve(value);
      stateExplicit = true;
    } else if (option === '--config') {
      configPath = value;
    } else {
      if (!['auto', 'always', 'never'].includes(value)) invalidArguments();
      color = value as ColorMode;
    }
  }
  if (!logs) invalidArguments();
  return {
    subject,
    subjectKind,
    statePath,
    stateExplicit,
    ...(configPath === undefined ? {} : { configPath }),
    json,
    color,
  };
}

export async function resolveLogFollowArguments(
  args: readonly string[]
): Promise<LogFollowArguments> {
  const parsed = parseLogFollowArguments(args);
  const configuration = await resolveRuntimeConfiguration(parsed.configPath, {
    ...(parsed.stateExplicit ? { statePath: parsed.statePath } : {}),
  });
  return {
    subject: parsed.subject,
    subjectKind: parsed.subjectKind,
    statePath: configuration.statePath,
    json: parsed.json,
    color: parsed.color,
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

async function runtimeSnapshot(
  descriptor: RuntimeDescriptorV1,
  fetcher: LogFetch,
  signal?: AbortSignal
): Promise<RuntimeSnapshot> {
  const response = await fetcher(`${descriptor.adminUrl}/v1/snapshot`, {
    headers: { authorization: `Bearer ${descriptor.capability}` },
    signal,
  });
  if (!response.ok) {
    throw new RuntimeControlError(
      response.status === 401
        ? 'WOML_ADMIN_UNAUTHORIZED'
        : 'WOML_LOG_RUNTIME_UNAVAILABLE',
      `The active runtime snapshot returned HTTP ${response.status}.`
    );
  }
  const value: unknown = await response.json();
  if (
    !record(value) ||
    value.profile !== 'woml.runtime-operations-snapshot/v1' ||
    typeof value.runtimeInstanceId !== 'string' ||
    !Number.isSafeInteger(value.sequence) ||
    !Array.isArray(value.workflows) ||
    !Array.isArray(value.runs)
  ) {
    throw new LogFollowError(
      'WOML_LOG_RUNTIME_UNAVAILABLE',
      'The active runtime returned an invalid operations snapshot.'
    );
  }
  const workflows = value.workflows.map(item => {
    if (!record(item) || typeof item.workflowId !== 'string') {
      throw new LogFollowError(
        'WOML_LOG_RUNTIME_UNAVAILABLE',
        'The active runtime returned invalid workflow identity data.'
      );
    }
    return item.workflowId;
  });
  const runs = value.runs.map(item => {
    if (
      !record(item) ||
      typeof item.runId !== 'string' ||
      typeof item.workflowId !== 'string'
    ) {
      throw new LogFollowError(
        'WOML_LOG_RUNTIME_UNAVAILABLE',
        'The active runtime returned invalid run identity data.'
      );
    }
    return { runId: item.runId, workflowId: item.workflowId };
  });
  return {
    runtimeInstanceId: value.runtimeInstanceId,
    sequence: Number(value.sequence),
    workflows,
    runs,
  };
}

async function presentationResponse(
  descriptor: RuntimeDescriptorV1,
  path: string,
  fetcher: LogFetch,
  signal: AbortSignal
): Promise<unknown> {
  const response = await fetcher(`${descriptor.adminUrl}${path}`, {
    headers: { authorization: `Bearer ${descriptor.capability}` },
    signal,
  });
  if (!response.ok) {
    let code: string | undefined;
    try {
      const body: unknown = await response.json();
      if (
        record(body) &&
        record(body.error) &&
        typeof body.error.code === 'string'
      ) {
        code = body.error.code;
      }
    } catch {
      // The status remains enough to reject an invalid admin response.
    }
    throw new LogFollowError(
      code === 'WOML_RUN_NOT_FOUND'
        ? 'WOML_LOG_RUN_NOT_FOUND'
        : code ?? 'WOML_LOG_RUNTIME_UNAVAILABLE',
      `The active runtime presentation returned HTTP ${response.status}.`
    );
  }
  return await response.json();
}

function terminal(status: RunPresentationV1['status']): boolean {
  return ['succeeded', 'failed', 'cancelled', 'timed_out'].includes(status);
}

function inactiveDescriptor(error: unknown): boolean {
  return error instanceof Error && 'code' in error && error.code === 'ENOENT';
}

function stateAccessDenied(error: unknown): boolean {
  return error instanceof Error && 'code' in error &&
    (error.code === 'EACCES' || error.code === 'EPERM');
}

function stateAccessError(error: unknown): LogFollowError {
  return new LogFollowError(
    'WOML_LOG_STATE_UNAVAILABLE',
    'The durable state file cannot be read with the current user permissions.',
    { cause: error }
  );
}

async function existingState(path: string): Promise<boolean> {
  try {
    return (await stat(path)).isFile();
  } catch (error) {
    if (inactiveDescriptor(error)) return false;
    if (stateAccessDenied(error)) throw stateAccessError(error);
    throw error;
  }
}

export async function followWorkflowLogs(options: {
  readonly args: LogFollowArguments;
  readonly io: LogFollowIo;
  readonly dependencies?: LogFollowerDependencies;
}): Promise<number> {
  const { args, io } = options;
  const dependencies = options.dependencies ?? {};
  const fetcher = dependencies.fetch ?? globalThis.fetch;
  const readDescriptor = dependencies.readDescriptor ?? readRuntimeDescriptor;
  const readRun =
    dependencies.readRun ??
    ((statePath: string, runId: string) =>
      inspectRunPresentationWithRust(statePath, runId, {
        nativeCorePath: dependencies.nativeCorePath,
      }));
  const readWorkflow =
    dependencies.readWorkflow ??
    ((statePath: string, workflowId: string, limit: number) =>
      listRunPresentationsWithRust(statePath, workflowId, limit, {
        nativeCorePath: dependencies.nativeCorePath,
      }));
  const hasWorkflow =
    dependencies.hasWorkflow ??
    ((statePath: string, workflowId: string) =>
      hasWorkflowDefinitionWithRust(statePath, workflowId, {
        nativeCorePath: dependencies.nativeCorePath,
      }));
  const render: PresentationRenderOptions = {
    format: args.json ? 'json' : io.isTTY === true ? 'tty' : 'plain',
    color: args.color,
    isTTY: io.isTTY === true,
    width: io.columns,
    environment: process.env,
    fullResultCommand: runId =>
      `woml get ${runId} --state ${JSON.stringify(args.statePath)} --json`,
  };
  const descriptorFile = runtimeDescriptorPath(args.statePath);
  const emitted = new Map<string, string>();
  let renderedAny = false;
  const emit = (presentation: RunPresentationV1): void => {
    const digest = JSON.stringify(presentation);
    if (emitted.get(presentation.runId) === digest) return;
    emitted.set(presentation.runId, digest);
    const output = renderRunPresentation(presentation, render);
    io.stdout(args.json || !renderedAny ? output : `\n${output}`);
    renderedAny = true;
  };
  const warning = (code: string, message: string): void => {
    if (args.json) {
      io.stderr(
        `Warning [${sanitizePresentationDiagnostic(code)}]: ${sanitizePresentationDiagnostic(message)}\n`
      );
    }
    else io.stderr(renderPresentationWarning(code, message, render));
  };

  let retainedRun: RunPresentationV1 | undefined;
  let retainedWorkflow: RunPresentationListV1 | undefined;
  let retainedWorkflowDefinition = false;
  const stateAvailable =
    dependencies.readRun !== undefined ||
    dependencies.readWorkflow !== undefined ||
    (await existingState(args.statePath));
  if (stateAvailable) {
    try {
      if (args.subjectKind === 'run') retainedRun = readRun(args.statePath, args.subject);
      else {
        retainedWorkflow = readWorkflow(args.statePath, args.subject, 10);
        retainedWorkflowDefinition = hasWorkflow(args.statePath, args.subject);
      }
    } catch (error) {
      if (stateAccessDenied(error)) throw stateAccessError(error);
      if (
        error instanceof RunManagementError &&
        error.code === 'WOML_RUN_STATE_UNAVAILABLE'
      ) {
        throw stateAccessError(error);
      }
      if (
        !(
          args.subjectKind === 'run' &&
          error instanceof RunManagementError &&
          error.code === 'WOML_RUN_NOT_FOUND'
        )
      ) {
        throw error;
      }
    }
  }
  if (retainedRun !== undefined) emit(retainedRun);
  if (retainedWorkflow !== undefined) {
    for (const run of [...retainedWorkflow.runs].sort((left, right) =>
      left.admittedAt.localeCompare(right.admittedAt)
    )) emit(run);
  }
  if (retainedRun !== undefined && terminal(retainedRun.status)) return 0;

  let descriptor: RuntimeDescriptorV1 | undefined;
  let snapshot: RuntimeSnapshot | undefined;
  let initialRuntimeError: unknown;
  try {
    descriptor = await readDescriptor(descriptorFile);
    snapshot = await runtimeSnapshot(descriptor, fetcher);
  } catch (error) {
    if (!inactiveDescriptor(error) && error instanceof RuntimeControlError) {
      if (error.code === 'WOML_RUNTIME_DESCRIPTOR_UNSAFE') throw error;
    }
    if (!inactiveDescriptor(error)) initialRuntimeError = error;
  }
  const activeSubject =
    args.subjectKind === 'workflow'
      ? snapshot?.workflows.includes(args.subject) === true
      : snapshot?.runs.some(run => run.runId === args.subject) === true;
  const hasRetained =
    retainedRun !== undefined ||
    retainedWorkflowDefinition ||
    (retainedWorkflow?.runs.length ?? 0) > 0;
  if (!hasRetained && !activeSubject) {
    if (descriptor !== undefined && initialRuntimeError !== undefined) {
      throw new LogFollowError(
        'WOML_LOG_RUNTIME_UNAVAILABLE',
        `The active runtime could not be queried: ${initialRuntimeError instanceof Error ? initialRuntimeError.message : String(initialRuntimeError)}`
      );
    }
    throw new LogFollowError(
      args.subjectKind === 'run'
        ? 'WOML_LOG_RUN_NOT_FOUND'
        : 'WOML_LOG_WORKFLOW_NOT_FOUND',
      args.subjectKind === 'run'
        ? `Run ${args.subject} has no retained history.`
        : `Workflow ${args.subject} is neither active nor present in retained run history.`
    );
  }
  if (descriptor === undefined) {
    warning(
      'WOML_LOG_RUNTIME_UNAVAILABLE',
      'Retained history was shown, but no active runtime is available to follow.'
    );
    return 0;
  }

  if (!args.json) {
    io.stderr(
      `Following ${args.subjectKind} ${args.subject}. Press Ctrl+C to detach; workflows keep running.\n`
    );
  }
  const deploymentId = descriptor.deploymentId;
  let sequence = snapshot?.sequence ?? 0;
  let runtimeInstanceId = snapshot?.runtimeInstanceId ?? descriptor.runtimeInstanceId;
  let terminalReached = false;
  let detached = false;
  let failureStartedAt: number | undefined;
  let refreshFailure: unknown;
  let interruptStream: (() => void) | undefined;
  const reconnectWindowMs = dependencies.reconnectWindowMs ?? 5_000;
  const abort = new AbortController();
  let refreshChain = Promise.resolve();
  const refresh = async (current: RuntimeDescriptorV1): Promise<void> => {
    if (args.subjectKind === 'run') {
      try {
        const value = decodeRunPresentationV1(
          JSON.stringify(
            await presentationResponse(
              current,
              `/v1/presentations/runs/${encodeURIComponent(args.subject)}`,
              fetcher,
              abort.signal
            )
          )
        );
        emit(value);
        if (terminal(value.status)) {
          terminalReached = true;
          abort.abort();
        }
      } catch (error) {
        if (
          error instanceof LogFollowError &&
          error.code === 'WOML_LOG_RUN_NOT_FOUND' &&
          emitted.has(args.subject)
        ) {
          warning(
            'WOML_LOG_HISTORY_PRUNED',
            `Retained history for ${args.subject} became unavailable while it was being followed.`
          );
          terminalReached = true;
          abort.abort();
          return;
        }
        throw error;
      }
      return;
    }
    const value = decodeRunPresentationListV1(
      JSON.stringify(
        await presentationResponse(
          current,
          `/v1/presentations/workflows/${encodeURIComponent(args.subject)}?limit=10`,
          fetcher,
          abort.signal
        )
      )
    );
    for (const run of [...value.runs].sort((left, right) =>
      left.admittedAt.localeCompare(right.admittedAt)
    )) emit(run);
  };
  const scheduleRefresh = (current: RuntimeDescriptorV1): void => {
    refreshChain = refreshChain
      .then(() => refresh(current))
      .catch(error => {
        refreshFailure = error;
        interruptStream?.();
      });
  };

  try {
    await refresh(descriptor);
  } catch {
    // A transient presentation failure is retried through the descriptor and
    // operations-stream reconnect loop below.
  }
  if (terminalReached) return 0;

  let resolveDetach!: () => void;
  const internalDetach = new Promise<void>(resolve => {
    resolveDetach = resolve;
  });
  const onSignal = (): void => resolveDetach();
  if (dependencies.waitForDetach === undefined) {
    process.once('SIGINT', onSignal);
    process.once('SIGTERM', onSignal);
  }
  const detach = dependencies.waitForDetach?.() ?? internalDetach;
  void detach.then(() => {
    detached = true;
    abort.abort();
  });

  try {
    while (!detached && !terminalReached) {
      try {
        const current = await readDescriptor(descriptorFile);
        if (current.deploymentId !== deploymentId) {
          throw new LogFollowError(
            'WOML_LOG_RUNTIME_UNAVAILABLE',
            'The runtime descriptor now belongs to a different deployment.'
          );
        }
        if (current.runtimeInstanceId !== runtimeInstanceId) {
          const currentSnapshot = await runtimeSnapshot(
            current,
            fetcher,
            abort.signal
          );
          runtimeInstanceId = currentSnapshot.runtimeInstanceId;
          sequence = currentSnapshot.sequence;
          await refresh(current);
        }
        const response = await fetcher(
          `${current.adminUrl}/v1/stream?after=${sequence}`,
          {
            headers: { authorization: `Bearer ${current.capability}` },
            signal: abort.signal,
          }
        );
        const connectionAbort = new AbortController();
        const closeConnection = (): void => connectionAbort.abort();
        abort.signal.addEventListener('abort', closeConnection, { once: true });
        if (abort.signal.aborted) connectionAbort.abort();
        interruptStream = closeConnection;
        failureStartedAt = undefined;
        try {
          await consumeOperationsStream(
            response,
            event => {
              if (event.runtimeInstanceId !== runtimeInstanceId) {
                throw new LogFollowError(
                  'WOML_LOG_RUNTIME_UNAVAILABLE',
                  'The operations stream identity does not match the runtime descriptor.'
                );
              }
              sequence = Math.max(sequence, event.sequence);
              if (event.subject.code === 'WOML_OBSERVABILITY_STREAM_GAP') {
                scheduleRefresh(current);
                return;
              }
              if (
                ['run', 'trigger', 'approval', 'retry', 'workflow_call', 'policy'].includes(
                  event.kind
                )
              ) {
                scheduleRefresh(current);
              }
            },
            connectionAbort.signal
          );
        } finally {
          interruptStream = undefined;
          abort.signal.removeEventListener('abort', closeConnection);
        }
        await refreshChain;
        if (refreshFailure !== undefined) {
          const error = refreshFailure;
          refreshFailure = undefined;
          throw error;
        }
        if (!detached && !terminalReached) {
          throw new Error('The active runtime stream closed.');
        }
      } catch (error) {
        await refreshChain.catch(() => {});
        if (detached || terminalReached || abort.signal.aborted) break;
        if (failureStartedAt === undefined) failureStartedAt = Date.now();
        if (Date.now() - failureStartedAt >= reconnectWindowMs) {
          warning(
            'WOML_LOG_RUNTIME_UNAVAILABLE',
            `Live following ended because the runtime is unavailable${error instanceof Error ? `: ${error.message}` : '.'}`
          );
          break;
        }
        await Bun.sleep(100);
      }
    }
  } finally {
    abort.abort();
    await refreshChain.catch(() => {});
    process.off('SIGINT', onSignal);
    process.off('SIGTERM', onSignal);
  }
  return 0;
}
