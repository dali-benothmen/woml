#!/usr/bin/env bun

import { mkdir, stat } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';

import {
  compileWoml,
  isWomlElement,
  parseWoml,
  WomlDiagnosticError,
  type SourcePosition,
  type WomlSourceDocument,
  type WomlSourceElement,
} from 'woml';
import {
  executeApprovalWorkflowWithRust,
  executeWorkflowWithRust,
  resolveApprovalWithRust,
  resumeApprovalWorkflowWithRust,
  RustWorkflowExecutionError,
  settleApprovalTimeoutWithRust,
  type RustApprovalRuntimeOutcome,
} from './rust-executor';
import {
  ApprovalServerBindError,
  DEFAULT_APPROVAL_PORT,
  serveApprovalAndWait,
} from './approval-server';
import {
  createSecretStore,
  requireValidSecretName,
  SecretStoreError,
  type SecretStore,
} from './secrets';
import { readSecretFromTerminal } from './secrets/prompt';

export interface CliIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
}

const processIo: CliIo = {
  stdout: text => process.stdout.write(text),
  stderr: text => process.stderr.write(text),
};

class CliInputError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'CliInputError';
    this.code = code;
  }
}

function runUsage(): string {
  return 'Usage: woml run <workflow.woml> [--state <path>] [--resume <runId>] [--approval-port <port>]';
}

function secretsUsage(): string {
  return [
    'Usage:',
    '  woml secrets set <NAME>',
    '  woml secrets list',
    '  woml secrets delete <NAME>',
  ].join('\n');
}

function usage(): string {
  return `${runUsage()}\n${secretsUsage()}`;
}

interface RunArguments {
  readonly filePath: string;
  readonly statePath: string;
  readonly resumeRunId?: string;
  readonly approvalPort: number;
}

function parseRunArguments(args: readonly string[]): RunArguments {
  const [command, filePath, ...options] = args;
  if (
    command !== 'run' ||
    filePath === undefined ||
    filePath.startsWith('--')
  ) {
    throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', runUsage());
  }
  let statePath = resolve('.woml/state.sqlite');
  let resumeRunId: string | undefined;
  let approvalPort = DEFAULT_APPROVAL_PORT;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 2) {
    const option = options[index];
    const value = options[index + 1];
    if (
      value === undefined ||
      seen.has(option) ||
      (option !== '--state' &&
        option !== '--resume' &&
        option !== '--approval-port')
    ) {
      throw new CliInputError('WOML_CLI_ARGUMENTS_INVALID', runUsage());
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
    } else {
      const port = Number(value);
      if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) {
        throw new CliInputError(
          'WOML_CLI_ARGUMENTS_INVALID',
          '--approval-port must be an integer from 1 through 65535.'
        );
      }
      approvalPort = port;
    }
  }
  return { filePath, statePath, resumeRunId, approvalPort };
}

function runtimeCode(code: string): string {
  if (code === 'WOML_SCRIPT_NON_JSON_RESULT') return 'WOML_NON_JSON_RESULT';
  if (code.startsWith('WOML_SCRIPT_')) return 'WOML_SCRIPT_FAILED';
  return code;
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
  io: CliIo
): Promise<void> {
  await mkdir(dirname(args.statePath), { recursive: true });
  let outcome =
    args.resumeRunId === undefined
      ? await executeApprovalWorkflowWithRust(workflow, args.statePath)
      : await resumeApprovalWorkflowWithRust(
          workflow,
          args.statePath,
          args.resumeRunId
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
      waiting.runId
    );
  }
  io.stdout(`${JSON.stringify(outcome.execution.result)}\n`);
}

export interface CliDependencies {
  readonly createSecretStore: () => SecretStore;
  readonly readSecret: (name: string) => Promise<string>;
}

const defaultDependencies: CliDependencies = {
  createSecretStore: () => createSecretStore(),
  readSecret: readSecretFromTerminal,
};

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
  if (args[0] === 'secrets') {
    return await runSecretsCommand(args, io, dependencies);
  }

  let runArguments: RunArguments;
  try {
    runArguments = parseRunArguments(args);
  } catch (error) {
    if (error instanceof CliInputError && error.message !== runUsage()) {
      io.stderr(`${error.message}\n`);
    }
    io.stderr(`${args.length === 0 ? usage() : runUsage()}\n`);
    return 2;
  }
  const { filePath } = runArguments;

  let document: WomlSourceDocument | undefined;
  try {
    const source = await readWorkflow(filePath);
    document = parseWoml(source, { file: filePath });
    const workflow = compileWoml(document);
    if (workflow.schemaVersion === 5) {
      throw new CliInputError(
        'WOML_NOTIFICATION_RUNTIME_UNAVAILABLE',
        'This workflow compiled successfully and its durable notification core is available. N4 includes a deterministic conformance adapter, but real Slack execution is not enabled until N5.'
      );
    }
    if (
      runArguments.resumeRunId !== undefined &&
      workflow.schemaVersion !== 4
    ) {
      throw new CliInputError(
        'WOML_RESUME_REQUIRES_APPROVAL',
        '--resume currently supports Human Approval workflows only.'
      );
    }
    if (workflow.schemaVersion === 4) {
      await runApprovalWorkflow(workflow, runArguments, io);
      return 0;
    }
    const execution = await executeWorkflowWithRust(workflow);
    io.stdout(`${JSON.stringify(execution.result)}\n`);
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error, filePath, document)}\n`);
    return 1;
  }
}

if (import.meta.main) {
  process.exitCode = await runCli(process.argv.slice(2));
}
