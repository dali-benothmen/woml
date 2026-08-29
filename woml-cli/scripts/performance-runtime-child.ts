#!/usr/bin/env bun

import { createInterface } from 'node:readline';

import type { ManualLineInput } from '../src/manual-input';
import type { RunPresentationV1 } from '../src/terminal-presentation';
import {
  PERFORMANCE_CONTROL_PROFILE,
  PERFORMANCE_SIGNAL_PROFILE,
  type PerformanceControlV1,
  type PerformanceSignalV1,
} from './performance-measurement';

function emit(signal: PerformanceSignalV1): void {
  process.stdout.write(`${JSON.stringify(signal)}\n`);
}

if (process.argv.includes('--calibrate')) {
  emit({ profile: PERFORMANCE_SIGNAL_PROFILE, type: 'child_ready' });
  process.exit(0);
}

const workflowIndex = process.argv.indexOf('--workflow');
const stateIndex = process.argv.indexOf('--state');
const artifactIndex = process.argv.indexOf('--cli-artifact');
const workflowPath = workflowIndex === -1 ? undefined : process.argv[workflowIndex + 1];
const statePath = stateIndex === -1 ? undefined : process.argv[stateIndex + 1];
const cliArtifact = artifactIndex === -1 ? 'built' : process.argv[artifactIndex + 1];
if (workflowPath === undefined || statePath === undefined) {
  emit({
    profile: PERFORMANCE_SIGNAL_PROFILE,
    type: 'error',
    message: 'The performance child requires --workflow and --state.',
  });
  process.exit(2);
}
if (cliArtifact !== 'built' && cliArtifact !== 'source') {
  emit({
    profile: PERFORMANCE_SIGNAL_PROFILE,
    type: 'error',
    message: 'The performance child requires --cli-artifact built or source.',
  });
  process.exit(2);
}

class ControlledInput implements ManualLineInput {
  readonly isTTY = true;
  readonly pendingRequestIds: string[] = [];
  #onLine?: (line: string) => void | Promise<void>;
  #readyResolve!: () => void;
  #closeResolve!: () => void;
  readonly #ready = new Promise<void>(resolve => { this.#readyResolve = resolve; });
  readonly #closed = new Promise<void>(resolve => { this.#closeResolve = resolve; });

  async run(onLine: (line: string) => void | Promise<void>): Promise<void> {
    this.#onLine = onLine;
    this.#readyResolve();
    await this.#closed;
  }

  async trigger(requestId: string): Promise<void> {
    await this.#ready;
    this.pendingRequestIds.push(requestId);
    try {
      await this.#onLine!('');
    } catch (error) {
      this.pendingRequestIds.pop();
      throw error;
    }
  }

  close(): void {
    this.#closeResolve();
  }
}

function environmentSecretStore() {
  return {
    provider: 'environment' as const,
    get: async (name: string) => process.env[name],
    has: async (name: string) => (process.env[name]?.length ?? 0) > 0,
    list: async () => [],
    set: async () => {
      throw new Error('The performance harness cannot mutate secrets.');
    },
    delete: async () => false,
  };
}

const input = new ControlledInput();
let resolveShutdown!: () => void;
const shutdown = new Promise<void>(resolve => { resolveShutdown = resolve; });
let diagnostic = '';

function inspectPresentation(text: string): void {
  for (const line of text.split('\n')) {
    if (line.trim().length === 0) continue;
    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch {
      continue;
    }
    if (
      typeof value === 'object' &&
      value !== null &&
      !('profile' in value) &&
      'triggers' in value &&
      Array.isArray(value.triggers) &&
      value.triggers.filter(trigger =>
        typeof trigger === 'object' &&
        trigger !== null &&
        'type' in trigger &&
        trigger.type === 'manual'
      ).length !== 1
    ) {
      emit({
        profile: PERFORMANCE_SIGNAL_PROFILE,
        type: 'error',
        message: 'Manual performance mode requires exactly one manual trigger.',
      });
      continue;
    }
    if (
      typeof value !== 'object' ||
      value === null ||
      !('profile' in value) ||
      value.profile !== 'woml.run-presentation/v1'
    ) {
      continue;
    }
    const presentation = value as RunPresentationV1;
    if (!['succeeded', 'failed', 'cancelled', 'timed_out'].includes(presentation.status)) {
      continue;
    }
    const requestId = input.pendingRequestIds.shift();
    if (requestId === undefined) {
      emit({
        profile: PERFORMANCE_SIGNAL_PROFILE,
        type: 'error',
        message: `Run ${presentation.runId} settled without a pending benchmark request.`,
      });
      continue;
    }
    emit({
      profile: PERFORMANCE_SIGNAL_PROFILE,
      type: 'run_terminal',
      requestId,
      runId: presentation.runId,
      status: presentation.status,
    });
  }
}

emit({ profile: PERFORMANCE_SIGNAL_PROFILE, type: 'child_ready' });

const cliPath = new URL(
  cliArtifact === 'source' ? '../src/cli.ts' : '../dist/cli.js',
  import.meta.url
).href;
const { runCli } = await import(cliPath) as typeof import('../src/cli');
const running = runCli(
  ['run', workflowPath, '--state', statePath, '--json', '--color=never'],
  {
    stdout: inspectPresentation,
    stderr: text => { diagnostic += text; },
    isTTY: true,
    columns: 100,
  },
  {
    createSecretStore: environmentSecretStore,
    readSecret: async () => '',
    createManualInput: () => input,
    waitForShutdown: () => shutdown,
    onRuntimeReady: info => {
      emit({
        profile: PERFORMANCE_SIGNAL_PROFILE,
        type: 'runtime_ready',
        runtimeInstanceId: info.runtimeInstanceId,
        workflowCount: info.workflowCount,
      });
    },
  }
);

const controls = createInterface({ input: process.stdin, crlfDelay: Infinity });
const controlTask = (async () => {
  for await (const line of controls) {
    let control: PerformanceControlV1;
    try {
      control = JSON.parse(line) as PerformanceControlV1;
    } catch {
      emit({
        profile: PERFORMANCE_SIGNAL_PROFILE,
        type: 'error',
        message: 'The performance child received malformed JSON control input.',
      });
      continue;
    }
    if (control.profile !== PERFORMANCE_CONTROL_PROFILE) {
      emit({
        profile: PERFORMANCE_SIGNAL_PROFILE,
        type: 'error',
        message: 'The performance child received an unknown control profile.',
      });
      continue;
    }
    if (control.type === 'trigger' && typeof control.requestId === 'string') {
      await input.trigger(control.requestId);
    } else if (control.type === 'stop') {
      resolveShutdown();
      break;
    }
  }
})();

const exitCode = await running;
controls.close();
await controlTask;
if (exitCode !== 0) {
  emit({
    profile: PERFORMANCE_SIGNAL_PROFILE,
    type: 'error',
    message: diagnostic.trim() || `The WOML runtime exited with code ${exitCode}.`,
  });
}
emit({
  profile: PERFORMANCE_SIGNAL_PROFILE,
  type: 'stopped',
  exitCode,
});
process.exitCode = exitCode;
