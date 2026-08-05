#!/usr/bin/env bun

import { stat } from 'node:fs/promises';
import { extname } from 'node:path';

import {
  compileWoml,
  executeWorkflow,
  parseWoml,
  WomlDiagnosticError,
  WorkflowExecutionError,
} from 'woml';

export interface CliIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
}

const processIo: CliIo = {
  stdout: (text) => process.stdout.write(text),
  stderr: (text) => process.stderr.write(text),
};

class CliInputError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'CliInputError';
    this.code = code;
  }
}

function usage(): string {
  return 'Usage: woml run <workflow.woml>';
}

function formatError(error: unknown, filePath?: string): string {
  if (error instanceof CliInputError) {
    return `WOML input error [${error.code}]${
      filePath === undefined ? '' : ` in "${filePath}"`
    }: ${error.message}`;
  }

  if (error instanceof WomlDiagnosticError) {
    const { diagnostic } = error;
    const location = `${diagnostic.file}:${diagnostic.location.start.line}:${diagnostic.location.start.column}`;
    const hint = diagnostic.hint === undefined ? '' : ` Hint: ${diagnostic.hint}`;
    return `WOML ${diagnostic.phase} error [${diagnostic.code}] at ${location}: ${diagnostic.message}${hint}`;
  }

  if (error instanceof WorkflowExecutionError) {
    const location = filePath === undefined ? '' : ` in "${filePath}"`;
    const node = error.nodeId === undefined ? '' : ` at step "${error.nodeId}"`;
    return `WOML runtime error [${error.code}]${location}${node}: ${error.message}`;
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
      'workflow files must use the .woml extension.',
    );
  }

  let file;
  try {
    file = await stat(filePath);
  } catch (error) {
    if (
      error instanceof Error &&
      'code' in error &&
      error.code === 'ENOENT'
    ) {
      throw new CliInputError(
        'WOML_FILE_NOT_FOUND',
        'workflow file does not exist.',
      );
    }
    throw error;
  }

  if (!file.isFile()) {
    throw new CliInputError(
      'WOML_NOT_A_FILE',
      'workflow path must point to a file.',
    );
  }

  return await Bun.file(filePath).text();
}

export async function runCli(
  args: readonly string[],
  io: CliIo = processIo,
): Promise<number> {
  const [command, filePath, ...extra] = args;

  if (command !== 'run' || filePath === undefined || extra.length > 0) {
    io.stderr(`${usage()}\n`);
    return 2;
  }

  try {
    const source = await readWorkflow(filePath);
    const document = parseWoml(source, { file: filePath });
    const workflow = compileWoml(document);
    const execution = await executeWorkflow(workflow);
    io.stdout(`${JSON.stringify(execution.result)}\n`);
    return 0;
  } catch (error) {
    io.stderr(`${formatError(error, filePath)}\n`);
    return 1;
  }
}

if (import.meta.main) {
  process.exitCode = await runCli(process.argv.slice(2));
}
