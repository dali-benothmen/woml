import { createRequire } from 'node:module';
import { resolve } from 'node:path';

import type {
  CompiledWorkflowDefinition,
  JsonObject,
  JsonValue,
} from 'woml';

export interface RustRunEvent {
  readonly eventSchemaVersion: 1;
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

interface NativeCore {
  readonly executeWomlWorkflow: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
  ) => Promise<string>;
  readonly executeWomlWorkflowDurable: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
  ) => Promise<string>;
  readonly recoverWomlRuns: (eventStorePath: string) => string;
}

function canonicalizeJson(value: unknown): string {
  if (value === null || typeof value === 'boolean' || typeof value === 'string') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new Error('A compiled workflow definition must contain only finite JSON numbers.');
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
      .map(
        (key) => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`,
      )
      .join(',')}}`;
  }
  throw new Error('A compiled workflow definition must be strict JSON.');
}

export function compiledDefinitionHash(
  workflow: CompiledWorkflowDefinition,
): string {
  const hexadecimal = new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(workflow))
    .digest('hex');
  return `sha256:${hexadecimal}`;
}

function defaultNativeCorePath(): string {
  if (process.platform !== 'linux' || process.arch !== 'x64') {
    throw new Error(
      'R3 requires nativeCorePath outside the current Linux x64 development target.',
    );
  }
  return resolve(import.meta.dir, '../../core/core.linux-x64-gnu.node');
}

function loadNativeCore(path: string): NativeCore {
  const require = createRequire(import.meta.url);
  const loaded = require(path) as Partial<NativeCore>;
  if (typeof loaded.executeWomlWorkflow !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose executeWomlWorkflow; rebuild the Rust addon.`,
    );
  }
  return loaded as NativeCore;
}

export async function executeWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  options: RustExecutorOptions = {},
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 0xffff_ffff) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const scriptHostPath =
    options.scriptHostPath ?? resolve(import.meta.dir, 'script-host.ts');
  const bunExecutable = options.bunExecutable ?? process.execPath;
  const native = loadNativeCore(nativePath);
  const resultJson = await native.executeWomlWorkflow(
    JSON.stringify(workflow),
    compiledDefinitionHash(workflow),
    JSON.stringify(options.trigger ?? {}),
    bunExecutable,
    scriptHostPath,
    timeoutMs,
  );
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

export async function executeWorkflowWithRustDurable(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  options: RustExecutorOptions = {},
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 0xffff_ffff) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.executeWomlWorkflowDurable !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose executeWomlWorkflowDurable; rebuild the Rust addon.`,
    );
  }
  const resultJson = await native.executeWomlWorkflowDurable(
    JSON.stringify(workflow),
    compiledDefinitionHash(workflow),
    JSON.stringify(options.trigger ?? {}),
    options.bunExecutable ?? process.execPath,
    options.scriptHostPath ?? resolve(import.meta.dir, 'script-host.ts'),
    timeoutMs,
    eventStorePath,
  );
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

export function recoverDurableRuns(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {},
): RustRecoveryReport {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.recoverWomlRuns !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose recoverWomlRuns; rebuild the Rust addon.`,
    );
  }
  return JSON.parse(native.recoverWomlRuns(eventStorePath)) as RustRecoveryReport;
}
