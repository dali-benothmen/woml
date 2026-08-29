#!/usr/bin/env bun

import { mkdtemp, rm, stat } from 'node:fs/promises';
import { cpus, tmpdir, totalmem } from 'node:os';
import { relative, resolve } from 'node:path';

import {
  PERFORMANCE_CONTROL_PROFILE,
  PERFORMANCE_MEASUREMENT_PROFILE,
  decodePerformanceSignal,
  formatPerformanceMeasurement,
  performanceMetric,
  type PerformanceMeasurementV1,
  type PerformanceMetricV1,
  type PerformanceMode,
  type PerformanceSignalV1,
} from './performance-measurement';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const defaultFixture = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-two-step-manual.woml'
);
const childPath = resolve(import.meta.dir, 'performance-runtime-child.ts');
const engineChildPath = resolve(import.meta.dir, 'performance-engine-child.ts');
const cliPath = resolve(cliRoot, 'dist/cli.js');
const scriptHostPath = resolve(cliRoot, 'dist/script-host.js');
const nativeCorePath = resolve(
  cliRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);

interface Arguments {
  readonly workflowPath: string;
  readonly mode: PerformanceMode;
  readonly warmups: number;
  readonly iterations: number;
  readonly json: boolean;
  readonly timeoutMs: number;
  readonly cliArtifact: 'built' | 'source';
  readonly profileOutput?: string;
}

function positiveInteger(value: string | undefined, option: string, allowZero = false): number {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < (allowZero ? 0 : 1)) {
    throw new Error(`${option} must be ${allowZero ? 'a non-negative' : 'a positive'} integer.`);
  }
  return parsed;
}

function parseArguments(values: readonly string[]): Arguments {
  let workflowPath = defaultFixture;
  let mode: PerformanceMode = 'all';
  let warmups = 3;
  let iterations = 10;
  let json = false;
  let timeoutMs = 30_000;
  let cliArtifact: 'built' | 'source' = 'built';
  let profileOutput: string | undefined;
  let positionalSeen = false;
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index]!;
    if (value === '--mode') {
      const selected = values[++index];
      if (selected !== 'manual' && selected !== 'engine' && selected !== 'all') {
        throw new Error('--mode must be manual, engine, or all.');
      }
      mode = selected;
    } else if (value === '--warmups') {
      warmups = positiveInteger(values[++index], '--warmups', true);
    } else if (value === '--iterations') {
      iterations = positiveInteger(values[++index], '--iterations');
    } else if (value === '--timeout') {
      timeoutMs = positiveInteger(values[++index], '--timeout');
    } else if (value === '--profile-output') {
      const output = values[++index];
      if (output === undefined || output.startsWith('--')) {
        throw new Error('--profile-output requires a file path.');
      }
      profileOutput = resolve(output);
    } else if (value === '--cli-artifact') {
      const artifact = values[++index];
      if (artifact !== 'built' && artifact !== 'source') {
        throw new Error('--cli-artifact must be built or source.');
      }
      cliArtifact = artifact;
    } else if (value === '--json') {
      json = true;
    } else if (value.startsWith('-')) {
      throw new Error(`Unknown performance option: ${value}`);
    } else if (positionalSeen) {
      throw new Error('Only one workflow path may be measured.');
    } else {
      workflowPath = resolve(value);
      positionalSeen = true;
    }
  }
  return {
    workflowPath,
    mode,
    warmups,
    iterations,
    json,
    timeoutMs,
    cliArtifact,
    ...(profileOutput === undefined ? {} : { profileOutput }),
  };
}

class PerformanceChild {
  readonly process: Bun.PipedSubprocess;
  readonly #signals: PerformanceSignalV1[] = [];
  readonly #waiters = new Set<() => void>();
  readonly #readTask: Promise<void>;
  #stderr = '';

  constructor(
    arguments_: readonly string[],
    environment: Readonly<Record<string, string>> = {},
    executablePath = childPath
  ) {
    this.process = Bun.spawn([process.execPath, executablePath, ...arguments_], {
      cwd: projectRoot,
      stdin: 'pipe',
      stdout: 'pipe',
      stderr: 'pipe',
      env: { ...process.env, NO_COLOR: '1', ...environment },
    });
    this.#readTask = this.#readSignals();
    void this.#readStderr();
  }

  async #readSignals(): Promise<void> {
    const reader = this.process.stdout.getReader();
    const decoder = new TextDecoder();
    let buffered = '';
    while (true) {
      const part = await reader.read();
      if (part.done) break;
      buffered += decoder.decode(part.value, { stream: true });
      while (buffered.includes('\n')) {
        const newline = buffered.indexOf('\n');
        const line = buffered.slice(0, newline);
        buffered = buffered.slice(newline + 1);
        const signal = decodePerformanceSignal(line);
        if (signal !== undefined) {
          this.#signals.push(signal);
          for (const wake of this.#waiters) wake();
        }
      }
    }
    buffered += decoder.decode();
    if (buffered.trim().length > 0) {
      const signal = decodePerformanceSignal(buffered);
      if (signal !== undefined) this.#signals.push(signal);
    }
    for (const wake of this.#waiters) wake();
  }

  async #readStderr(): Promise<void> {
    this.#stderr = await new Response(this.process.stderr).text();
  }

  async waitFor(
    predicate: (signal: PerformanceSignalV1) => boolean,
    timeoutMs: number
  ): Promise<PerformanceSignalV1> {
    const deadline = performance.now() + timeoutMs;
    while (true) {
      const error = this.#signals.find(signal => signal.type === 'error');
      if (error?.type === 'error') throw new Error(error.message);
      const index = this.#signals.findIndex(predicate);
      if (index !== -1) return this.#signals.splice(index, 1)[0]!;
      if (this.process.exitCode !== null) {
        await this.#readTask;
        throw new Error(
          `The performance child exited before the expected signal.${this.#stderr.length === 0 ? '' : `\n${this.#stderr}`}`
        );
      }
      const remaining = deadline - performance.now();
      if (remaining <= 0) throw new Error('Timed out waiting for the performance child.');
      await new Promise<void>((resolvePromise, reject) => {
        const timer = setTimeout(() => {
          this.#waiters.delete(wake);
          reject(new Error('Timed out waiting for the performance child.'));
        }, remaining);
        const wake = () => {
          clearTimeout(timer);
          this.#waiters.delete(wake);
          resolvePromise();
        };
        this.#waiters.add(wake);
      });
    }
  }

  async send(control: object): Promise<void> {
    this.process.stdin.write(`${JSON.stringify(control)}\n`);
    await this.process.stdin.flush();
  }

  async stop(timeoutMs: number): Promise<void> {
    if (this.process.exitCode === null) {
      await this.send({ profile: PERFORMANCE_CONTROL_PROFILE, type: 'stop' });
    }
    await this.waitFor(signal => signal.type === 'stopped', timeoutMs);
    await this.process.exited;
    await this.#readTask;
  }

  kill(): void {
    if (this.process.exitCode === null) this.process.kill('SIGKILL');
  }
}

async function calibrateHarness(iterations: number, timeoutMs: number): Promise<number[]> {
  const samples: number[] = [];
  for (let index = 0; index < iterations; index += 1) {
    const started = performance.now();
    const child = new PerformanceChild(['--calibrate']);
    try {
      await child.waitFor(signal => signal.type === 'child_ready', timeoutMs);
      samples.push(performance.now() - started);
      await child.process.exited;
    } finally {
      child.kill();
    }
  }
  return samples;
}

async function measureManual(
  args: Arguments,
  statePath: string
): Promise<{ readonly activation: number[]; readonly execution: number[] }> {
  const started = performance.now();
  const child = new PerformanceChild([
    '--workflow', args.workflowPath,
    '--state', statePath,
    '--cli-artifact', args.cliArtifact,
  ], {
    WOML_RUST_CORE_PATH: nativeCorePath,
    ...(args.profileOutput === undefined
      ? {}
      : {
        WOML_PROFILE: '1',
        WOML_PROFILE_OUTPUT: args.profileOutput,
        WOML_PROFILE_PROCESS: 'cli',
        WOML_PROFILE_TRACE_ID: `trace_frontend_${Date.now()}`,
      }),
  });
  try {
    await child.waitFor(signal => signal.type === 'child_ready', args.timeoutMs);
    const ready = await child.waitFor(signal => signal.type === 'runtime_ready', args.timeoutMs);
    if (ready.type !== 'runtime_ready' || ready.workflowCount !== 1) {
      throw new Error('The manual performance fixture must compile to exactly one workflow.');
    }
    const activation = [performance.now() - started];
    const execution: number[] = [];
    const total = args.warmups + args.iterations;
    for (let index = 0; index < total; index += 1) {
      const requestId = `performance_request_${index}`;
      const runStarted = performance.now();
      await child.send({
        profile: PERFORMANCE_CONTROL_PROFILE,
        type: 'trigger',
        requestId,
      });
      const terminal = await child.waitFor(
        signal => signal.type === 'run_terminal' && signal.requestId === requestId,
        args.timeoutMs
      );
      const elapsed = performance.now() - runStarted;
      if (terminal.type !== 'run_terminal' || terminal.status !== 'succeeded') {
        throw new Error(
          `Measured run ${terminal.type === 'run_terminal' ? terminal.runId : requestId} did not succeed.`
        );
      }
      if (index >= args.warmups) execution.push(elapsed);
    }
    await child.stop(args.timeoutMs);
    return { activation, execution };
  } finally {
    child.kill();
  }
}

async function measureEngine(
  args: Arguments,
  statePath: string
): Promise<number[]> {
  const child = new PerformanceChild([
    '--workflow', args.workflowPath,
    '--state', statePath,
    '--native-core', nativeCorePath,
    '--script-host', scriptHostPath,
    '--warmups', String(args.warmups),
    '--iterations', String(args.iterations),
  ], args.profileOutput === undefined
    ? {}
    : {
      WOML_PROFILE: '1',
      WOML_PROFILE_OUTPUT: args.profileOutput,
      WOML_PROFILE_PROCESS: 'cli',
      WOML_PROFILE_TRACE_ID: `trace_engine_${Date.now()}`,
    }, engineChildPath);
  try {
    await child.waitFor(signal => signal.type === 'child_ready', args.timeoutMs);
    const completed = await child.waitFor(
      signal => signal.type === 'engine_samples',
      args.timeoutMs * (args.warmups + args.iterations)
    );
    if (completed.type !== 'engine_samples') {
      throw new Error('The engine performance child returned an invalid result.');
    }
    await child.process.exited;
    return [...completed.samples];
  } finally {
    child.kill();
  }
}

async function requireBuiltArtifacts(cliArtifact: Arguments['cliArtifact']): Promise<void> {
  const missing: string[] = [];
  const required = [scriptHostPath, nativeCorePath];
  if (cliArtifact === 'built') required.unshift(cliPath);
  for (const path of required) {
    try {
      await stat(path);
    } catch {
      missing.push(relative(projectRoot, path));
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `The release-shaped performance harness requires a built WOML runtime. Missing: ${missing.join(', ')}. Run "bun run build" first.`
    );
  }
}

const args = parseArguments(process.argv.slice(2));
await requireBuiltArtifacts(args.cliArtifact);
const root = await mkdtemp(resolve(tmpdir(), 'woml-performance-'));
try {
  const metrics: PerformanceMetricV1[] = [];
  if (args.mode === 'manual' || args.mode === 'all') {
    const calibrationSamples = await calibrateHarness(
      Math.min(5, Math.max(1, args.iterations)),
      args.timeoutMs
    );
    metrics.push(performanceMetric(
      'harness.process',
      'calibration',
      { start: 'benchmark child process is spawned', end: 'child control protocol is ready' },
      calibrationSamples
    ));
    const manual = await measureManual(args, resolve(root, 'manual-state.sqlite'));
    metrics.push(performanceMetric(
      'activation.cold',
      'cold',
      { start: 'release-shaped runtime child is spawned', end: 'durable runtime reports ready' },
      manual.activation
    ));
    metrics.push(performanceMetric(
      'manual.visible',
      'warm',
      { start: 'manual trigger control is submitted', end: 'terminal run presentation is received' },
      manual.execution
    ));
  }

  if (args.mode === 'engine' || args.mode === 'all') {
    const samples = await measureEngine(args, resolve(root, 'engine-state.sqlite'));
    metrics.push(performanceMetric(
      'engine.durable',
      'warm',
      { start: 'compiled model enters durable N-API executor', end: 'durable terminal result returns' },
      samples
    ));
  }

  const source = await Bun.file(args.workflowPath).arrayBuffer();
  const hash = new Bun.CryptoHasher('sha256').update(source).digest('hex');
  const processors = cpus();
  const measurement: PerformanceMeasurementV1 = {
    profile: PERFORMANCE_MEASUREMENT_PROFILE,
    createdAt: new Date().toISOString(),
    fixture: {
      path: relative(projectRoot, args.workflowPath),
      sha256: hash,
    },
    environment: {
      platform: process.platform,
      architecture: process.arch,
      bunVersion: Bun.version,
      cpuModel: processors[0]?.model ?? 'unknown',
      logicalCpuCount: Math.max(1, processors.length),
      memoryBytes: totalmem(),
      nativeBuild: 'release',
      cliArtifact: args.cliArtifact,
    },
    parameters: {
      mode: args.mode,
      warmups: args.warmups,
      iterations: args.iterations,
    },
    metrics,
  };
  process.stdout.write(
    args.json
      ? `${JSON.stringify(measurement, null, 2)}\n`
      : formatPerformanceMeasurement(measurement)
  );
  if (args.profileOutput !== undefined) {
    process.stderr.write(`[profile] frontend spans appended to ${args.profileOutput}\n`);
  }
} finally {
  await rm(root, { recursive: true, force: true });
}
