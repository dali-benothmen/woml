#!/usr/bin/env bun

import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

interface ProfileResult {
  readonly sequentialMs: number;
  readonly concurrentMs: number;
  readonly iterations: number;
}

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

const packageRoot = resolve(import.meta.dir, '..');
const cli = resolve(packageRoot, 'dist', 'cli.js');

function integerOption(name: string, fallback: number): number {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = Number(process.argv[index + 1]);
  if (!Number.isSafeInteger(value)) throw new Error(`${name} requires an integer.`);
  return value;
}

const iterations = integerOption('--iterations', 3);
const warmup = integerOption('--warmup', 1);
if (iterations < 1 || iterations > 4 || warmup < 0 || warmup > 2) {
  throw new Error(
    'WC7 benchmark bounds are --iterations 1..4 and --warmup 0..2.'
  );
}
if (!(await Bun.file(cli).exists())) {
  throw new Error('Build the packaged CLI first with: bun run build');
}

const directory = await mkdtemp(join(tmpdir(), 'woml-wc7-call-benchmark-'));
const parentPath = join(directory, 'parent.woml');
const childPath = join(directory, 'child.woml');

function call(name: string, index: number): string {
  return `services.workflows.call('wc7-benchmark-child', { index: ${index} }, { name: '${name}' })`;
}

function operationName(prefix: string, index: number): string {
  return `${prefix}-${String.fromCharCode('a'.charCodeAt(0) + index)}`;
}

function parentWorkflow(): string {
  const warmupCalls = Array.from(
    { length: warmup },
    (_, index) => `await ${call(operationName('warmup', index), index)};`
  ).join('\n');
  const sequentialCalls = Array.from(
    { length: iterations },
    (_, index) => `await ${call(operationName('sequential', index), index)};`
  ).join('\n');
  const concurrentCalls = Array.from(
    { length: iterations },
    (_, index) => call(operationName('concurrent', index), index)
  ).join(',\n');
  return `<woml>
<workflow id="wc7-benchmark-parent" name="WC7 Workflow Call benchmark" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="benchmark">
      <script>
        ${warmupCalls}
        const sequentialStart = performance.now();
        ${sequentialCalls}
        const sequentialMs = performance.now() - sequentialStart;
        const concurrentStart = performance.now();
        await Promise.all([
          ${concurrentCalls}
        ]);
        const concurrentMs = performance.now() - concurrentStart;
        return { sequentialMs, concurrentMs, iterations: ${iterations} };
      </script>
    </step>
  </steps>
</workflow>
</woml>`;
}

function childWorkflow(): string {
  return `<woml>
<workflow id="wc7-benchmark-child" name="WC7 benchmark worker" version="1.0.0">
  <steps>
    <step id="work"><script>return { index: context.payload.index };</script></step>
  </steps>
</workflow>
</woml>`;
}

function start(args: readonly string[]): CapturedProcess {
  const child = Bun.spawn([cli, ...args], {
    cwd: directory,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  let stderr = '';
  const stderrDone = (async () => {
    const reader = child.stderr.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      stderr += decoder.decode(chunk.value, { stream: true });
    }
    stderr += decoder.decode();
  })();
  return { child, stderr: () => stderr, stderrDone };
}

async function waitFor(
  process: CapturedProcess,
  text: string,
  timeoutMs = 30_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!process.stderr().includes(text)) {
    if (process.child.exitCode !== null) {
      await process.stderrDone;
      throw new Error(
        `WOML benchmark process exited before ${JSON.stringify(text)}:\n${process.stderr()}`
      );
    }
    if (Date.now() >= deadline) throw new Error(process.stderr());
    await Bun.sleep(10);
  }
}

function result(process: CapturedProcess): ProfileResult {
  const lines = process.stderr().split('\n').filter(line => line.includes(' result: '));
  const encoded = lines.at(-1)?.split(' result: ', 2)[1];
  if (encoded === undefined) {
    throw new Error(`The benchmark did not print a run result:\n${process.stderr()}`);
  }
  return JSON.parse(encoded) as ProfileResult;
}

async function stop(process: CapturedProcess): Promise<void> {
  if (process.child.exitCode === null) process.child.kill('SIGINT');
  const exitCode = await process.child.exited;
  await process.stderrDone;
  if (exitCode !== 0) {
    throw new Error(`WOML benchmark process exited with ${exitCode}:\n${process.stderr()}`);
  }
}

async function sameRuntime(): Promise<ProfileResult> {
  const runtime = start([
    'run',
    parentPath,
    childPath,
    '--state',
    join(directory, 'same-runtime.sqlite'),
  ]);
  try {
    await waitFor(runtime, ' result: ');
    return result(runtime);
  } finally {
    await stop(runtime);
  }
}

async function crossProcess(): Promise<ProfileResult> {
  const state = join(directory, 'cross-process.sqlite');
  const target = start(['run', childPath, '--state', state]);
  try {
    await waitFor(target, 'WOML runtime is ready with 0 registered triggers.');
    const parent = start(['run', parentPath, '--state', state]);
    try {
      await waitFor(parent, ' result: ');
      return result(parent);
    } finally {
      await stop(parent);
    }
  } finally {
    await stop(target);
  }
}

function summarize(profile: ProfileResult) {
  return {
    sequentialTotalMs: profile.sequentialMs,
    sequentialMeanMs: profile.sequentialMs / profile.iterations,
    concurrentTotalMs: profile.concurrentMs,
    concurrentCallsPerSecond:
      (profile.iterations * 1_000) / profile.concurrentMs,
  };
}

try {
  await writeFile(parentPath, parentWorkflow());
  await writeFile(childPath, childWorkflow());
  const same = await sameRuntime();
  const cross = await crossProcess();
  const report = {
    benchmark: 'woml-workflow-calls-local-v1',
    note: 'Measured inside the parent script; CLI and process startup are excluded. Results are observational, not pass/fail thresholds.',
    iterations,
    warmup,
    sameRuntime: summarize(same),
    crossProcess: summarize(cross),
    crossToSameSequentialRatio: cross.sequentialMs / same.sequentialMs,
  };
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
} finally {
  await rm(directory, { recursive: true, force: true });
}
