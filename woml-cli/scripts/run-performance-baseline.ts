#!/usr/bin/env bun

import { existsSync, readFileSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { cpus, tmpdir, totalmem } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';

import {
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  parseWoml,
  type CompiledWorkflowDefinition,
} from '@woml/compiler';

import { executeWorkflowWithRustDurable } from '../src/rust-executor';
import { summarizeSamples } from './performance-measurement';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const nativeCorePath = resolve(
  cliRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const scriptHostPath = resolve(cliRoot, 'dist/script-host.js');
const measurementPath = resolve(import.meta.dir, 'measure-workflow.ts');
const presentationModesPath = resolve(import.meta.dir, 'profile-presentation-modes.ts');
const httpBenchmarkPath = resolve(import.meta.dir, 'benchmark-http.ts');
const canonicalFixture = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-two-step-manual.woml'
);

interface BaselineArguments {
  readonly batches: number;
  readonly outputPath: string;
  readonly quick: boolean;
}

interface MemorySample {
  readonly baselineRssBytes: number;
  readonly peakRssBytes: number;
  readonly deltaRssBytes: number;
  readonly scope: 'linux-process-tree' | 'current-process';
}

interface RuntimeCase {
  readonly id: string;
  readonly category: 'sequential' | 'parallel' | 'for_each' | 'context';
  readonly source: string;
  readonly workUnits: number;
  readonly iterationsPerBatch: number;
  readonly expectedContextBytes?: number;
}

interface PreparedRuntimeCase {
  readonly item: RuntimeCase;
  readonly workflow: CompiledWorkflowDefinition;
  readonly statePath: string;
  readonly execute: () => ReturnType<typeof executeWorkflowWithRustDurable>;
  readonly samples: number[];
  readonly batchThroughput: number[];
  readonly batchPositions: number[];
  readonly eventCounts: number[];
}

function argumentsFrom(values: readonly string[]): BaselineArguments {
  let batches = 3;
  let outputPath = resolve(cliRoot, '.woml/performance/baseline-v1.json');
  let quick = false;
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index]!;
    if (value === '--batches') {
      batches = Number(values[++index]);
      if (!Number.isSafeInteger(batches) || batches < 3 || batches > 10) {
        throw new Error('--batches must be an integer from 3 through 10.');
      }
    } else if (value === '--output') {
      const output = values[++index];
      if (output === undefined || output.startsWith('--')) {
        throw new Error('--output requires a file path.');
      }
      outputPath = resolve(output);
    } else if (value === '--quick') {
      quick = true;
    } else {
      throw new Error(`Unknown PERF6 baseline option: ${value}`);
    }
  }
  return { batches, outputPath, quick };
}

function sequentialSource(stepCount: number): string {
  const steps = Array.from({ length: stepCount }, (_, index) => {
    const body = index === 0
      ? 'return { value: 1 };'
      : `return { value: context.steps.step${index - 1}.value + 1 };`;
    return `      <step id="step${index}"><script>${body}</script></step>`;
  }).join('\n');
  return `<woml>
  <workflow id="performance-sequential-${stepCount}" name="Sequential ${stepCount}" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
${steps}
    </steps>
  </workflow>
</woml>\n`;
}

function parallelSource(branchCount: number, concurrency: number): string {
  const branches = Array.from({ length: branchCount }, (_, index) =>
    `        <step id="branch${index}"><script>return { value: context.steps.seed.value + ${index} };</script></step>`
  ).join('\n');
  return `<woml>
  <workflow id="performance-parallel-${branchCount}" name="Parallel ${branchCount}" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="seed"><script>return { value: 1 };</script></step>
      <parallel id="branches" concurrency="${concurrency}" on-error="wait-all">
${branches}
      </parallel>
      <step id="summary"><script>return { completed: ${branchCount} };</script></step>
    </steps>
  </workflow>
</woml>\n`;
}

function forEachSource(itemCount: number, concurrency: number): string {
  const items = JSON.stringify(Array.from({ length: itemCount }, (_, index) => index));
  return `<woml>
  <workflow id="performance-for-each-${itemCount}" name="For each ${itemCount}" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="load"><script>return { items: ${items} };</script></step>
      <for-each id="process" items="{{context.steps.load.items}}" concurrency="${concurrency}">
        <step id="transform"><script>return { index: context.iteration.index, value: context.item + 1 };</script></step>
        <result value="{{context.steps.transform}}" />
      </for-each>
      <step id="summary"><script>return { processed: context.steps.process.succeeded };</script></step>
    </steps>
  </workflow>
</woml>\n`;
}

function contextSource(bytesPerStep: number, stepCount = 3): string {
  const steps = Array.from({ length: stepCount }, (_, index) =>
    `      <step id="blob${index}"><script>return { blob: "x".repeat(${bytesPerStep}) };</script></step>`
  ).join('\n');
  return `<woml>
  <workflow id="performance-context-${bytesPerStep}" name="Context ${bytesPerStep}" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
${steps}
      <step id="summary"><script>return { blobs: ${stepCount} };</script></step>
    </steps>
  </workflow>
</woml>\n`;
}

function compile(source: string, file: string): CompiledWorkflowDefinition {
  return compileWoml(parseWoml(source, { file }));
}

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)]!;
}

function processRssBytes(pid: number): number {
  try {
    const status = readFileSync(`/proc/${pid}/status`, 'utf8');
    const match = /^VmRSS:\s+(\d+)\s+kB$/mu.exec(status);
    return match === null ? 0 : Number(match[1]) * 1024;
  } catch {
    return 0;
  }
}

function childPids(pid: number): number[] {
  try {
    const task = readFileSync(`/proc/${pid}/task/${pid}/children`, 'utf8').trim();
    return task.length === 0
      ? []
      : task.split(/\s+/u).map(Number).filter(Number.isSafeInteger);
  } catch {
    return [];
  }
}

function processTreeRssBytes(rootPid: number): number {
  if (process.platform !== 'linux') return process.memoryUsage().rss;
  const pending = [rootPid];
  const visited = new Set<number>();
  let total = 0;
  while (pending.length > 0) {
    const pid = pending.pop()!;
    if (visited.has(pid)) continue;
    visited.add(pid);
    total += processRssBytes(pid);
    pending.push(...childPids(pid));
  }
  return total;
}

async function withMemorySample<T>(rootPid: number, operation: () => Promise<T>): Promise<{
  readonly result: T;
  readonly memory: MemorySample;
}> {
  const baselineRssBytes = processTreeRssBytes(rootPid);
  let peakRssBytes = baselineRssBytes;
  const timer = setInterval(() => {
    peakRssBytes = Math.max(peakRssBytes, processTreeRssBytes(rootPid));
  }, 20);
  try {
    const result = await operation();
    peakRssBytes = Math.max(peakRssBytes, processTreeRssBytes(rootPid));
    return {
      result,
      memory: {
        baselineRssBytes,
        peakRssBytes,
        deltaRssBytes: Math.max(0, peakRssBytes - baselineRssBytes),
        scope: process.platform === 'linux' ? 'linux-process-tree' : 'current-process',
      },
    };
  } finally {
    clearInterval(timer);
  }
}

async function storeBytes(path: string): Promise<number> {
  let total = 0;
  for (const candidate of [path, `${path}-wal`, `${path}-shm`]) {
    try {
      total += (await stat(candidate)).size;
    } catch {
      // Optional SQLite sidecars are absent after a clean checkpoint.
    }
  }
  return total;
}

async function prepareRuntimeCase(
  root: string,
  item: RuntimeCase
): Promise<PreparedRuntimeCase> {
  const file = join(root, `${item.id}.woml`);
  await writeFile(file, item.source);
  const workflow = compile(item.source, file);
  const statePath = join(root, `${item.id}.sqlite`);
  const execute = () => executeWorkflowWithRustDurable(workflow, statePath, {
    nativeCorePath,
    scriptHostPath,
    trigger: {},
  });

  await execute();
  return {
    item,
    workflow,
    statePath,
    execute,
    samples: [],
    batchThroughput: [],
    batchPositions: [],
    eventCounts: [],
  };
}

function runtimeOrder(
  sessions: readonly PreparedRuntimeCase[],
  batch: number,
  batches: number
): PreparedRuntimeCase[] {
  const offset = Math.floor((batch * sessions.length) / batches);
  const rotated = sessions.map((_, index) =>
    sessions[(index + offset) % sessions.length]!
  );
  return batch % 2 === 0 ? rotated : rotated.reverse();
}

async function measureRuntimeBatch(
  session: PreparedRuntimeCase,
  position: number
): Promise<void> {
  Bun.gc(true);
  await Bun.sleep(75);
  const batchStartedAt = performance.now();
  for (let iteration = 0; iteration < session.item.iterationsPerBatch; iteration += 1) {
    const startedAt = performance.now();
    const execution = await session.execute();
    session.samples.push(performance.now() - startedAt);
    session.eventCounts.push(execution.events.length);
  }
  const elapsedMs = performance.now() - batchStartedAt;
  session.batchThroughput.push(
    (session.item.iterationsPerBatch * session.item.workUnits * 1_000) / elapsedMs
  );
  session.batchPositions.push(position);
}

async function finishRuntimeCase(
  session: PreparedRuntimeCase,
  batches: number
): Promise<Record<string, unknown>> {
  const monitored = await withMemorySample(process.pid, session.execute);
  session.eventCounts.push(monitored.result.events.length);
  const { item, workflow } = session;
  return {
    id: item.id,
    category: item.category,
    workUnits: item.workUnits,
    nodeCount: workflow.graph.nodes.length,
    edgeCount: workflow.graph.edges.length,
    modelBytes: Buffer.byteLength(JSON.stringify(workflow)),
    sourceBytes: Buffer.byteLength(item.source),
    batches,
    iterationsPerBatch: item.iterationsPerBatch,
    samples: session.samples,
    summary: summarizeSamples(session.samples),
    throughput: {
      unit: 'work_units_per_second',
      median: percentile(session.batchThroughput, 0.5),
      p95: percentile(session.batchThroughput, 0.95),
      samples: session.batchThroughput,
    },
    batchPositions: session.batchPositions,
    eventCount: {
      minimum: Math.min(...session.eventCounts),
      median: percentile(session.eventCounts, 0.5),
      maximum: Math.max(...session.eventCounts),
    },
    memory: monitored.memory,
    durableStoreBytes: await storeBytes(session.statePath),
    ...(item.expectedContextBytes === undefined
      ? {}
      : { expectedContextBytes: item.expectedContextBytes }),
  };
}

async function spawnJson(
  command: readonly string[],
  options: { readonly monitorMemory?: boolean; readonly environment?: Record<string, string> } = {}
): Promise<{ readonly value: Record<string, unknown>; readonly memory?: MemorySample }> {
  const child = Bun.spawn(command, {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'pipe',
    env: { ...process.env, ...options.environment },
  });
  const completion = async () => {
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    if (exitCode !== 0) {
      throw new Error(`PERF6 child failed (${command.join(' ')}):\n${stderr}`);
    }
    return JSON.parse(stdout) as Record<string, unknown>;
  };
  if (options.monitorMemory !== true) return { value: await completion() };
  const monitored = await withMemorySample(child.pid, completion);
  return { value: monitored.result, memory: monitored.memory };
}

function metricSamples(
  measurement: Record<string, unknown>,
  name: string
): number[] {
  const metrics = measurement.metrics;
  if (!Array.isArray(metrics)) throw new Error('Measurement has no metrics.');
  const metric = metrics.find(candidate =>
    typeof candidate === 'object' && candidate !== null &&
    'name' in candidate && candidate.name === name
  ) as { samples?: unknown } | undefined;
  if (metric === undefined || !Array.isArray(metric.samples) ||
      !metric.samples.every(value => typeof value === 'number')) {
    throw new Error(`Measurement has no valid ${name} samples.`);
  }
  return metric.samples;
}

async function headlineBaseline(root: string, batches: number): Promise<Record<string, unknown>> {
  const activation: number[] = [];
  const manual: number[] = [];
  const calibration: number[] = [];
  const memory: MemorySample[] = [];
  for (let batch = 0; batch < batches; batch += 1) {
    const measured = await spawnJson([
      process.execPath,
      measurementPath,
      canonicalFixture,
      '--mode',
      'manual',
      '--warmups',
      '1',
      '--iterations',
      '2',
      '--json',
    ], {
      monitorMemory: true,
      environment: { WOML_RUST_CORE_PATH: nativeCorePath },
    });
    activation.push(...metricSamples(measured.value, 'activation.cold'));
    manual.push(...metricSamples(measured.value, 'manual.visible'));
    calibration.push(...metricSamples(measured.value, 'harness.process'));
    if (measured.memory !== undefined) memory.push(measured.memory);
    await Bun.sleep(100);
  }
  return {
    batches,
    calibration: { samples: calibration, summary: summarizeSamples(calibration) },
    coldActivation: { samples: activation, summary: summarizeSamples(activation) },
    warmManualVisible: { samples: manual, summary: summarizeSamples(manual) },
    processTreeMemory: {
      scope: memory[0]?.scope ?? 'current-process',
      peakRssBytes: Math.max(...memory.map(item => item.peakRssBytes)),
      peakDeltaRssBytes: Math.max(...memory.map(item => item.deltaRssBytes)),
    },
  };
}

async function compileProfile(
  id: string,
  operation: () => Promise<{ readonly sourceBytes: number; readonly outputBytes: number; readonly nodes: number }>,
  iterations: number
): Promise<Record<string, unknown>> {
  for (let warmup = 0; warmup < 3; warmup += 1) await operation();
  const samples: number[] = [];
  let shape = await operation();
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const startedAt = performance.now();
    shape = await operation();
    samples.push(performance.now() - startedAt);
  }
  return { id, iterations, ...shape, samples, summary: summarizeSamples(samples) };
}

async function compileMatrix(root: string, quick: boolean): Promise<readonly Record<string, unknown>[]> {
  const iterations = quick ? 8 : 30;
  const sourceCases = [
    ['small', sequentialSource(2)],
    ['large', sequentialSource(100)],
    ['control-flow-heavy', `${parallelSource(50, 4).trim()}\n`],
  ] as const;
  const results: Record<string, unknown>[] = [];
  for (const [id, source] of sourceCases) {
    const file = join(root, `compile-${id}.woml`);
    await writeFile(file, source);
    results.push(await compileProfile(id, async () => {
      const workflow = compile(source, file);
      return {
        sourceBytes: Buffer.byteLength(source),
        outputBytes: Buffer.byteLength(JSON.stringify(workflow)),
        nodes: workflow.graph.nodes.length,
      };
    }, iterations));
  }

  for (const moduleCount of [1, 10]) {
    const moduleRoot = join(root, `modules-${moduleCount}`);
    await mkdir(moduleRoot, { recursive: true });
    const imports: string[] = [];
    for (let index = 0; index < moduleCount; index += 1) {
      await writeFile(
        join(moduleRoot, `module-${index}.ts`),
        `export function value${index}() { return ${index}; }\n`
      );
      imports.push(`<module name="module${index}" from="./module-${index}.ts" />`);
    }
    const source = `<woml><imports>${imports.join('')}</imports><workflow id="module-${moduleCount}" version="1.0.0"><triggers><manual id="start" /></triggers><steps><step id="use"><script>return services.module0.value0();</script></step></steps></workflow></woml>`;
    const file = join(moduleRoot, 'workflow.woml');
    await writeFile(file, source);
    const document = parseWoml(source, { file });
    results.push(await compileProfile(`modules-${moduleCount}`, async () => {
      const packageValue = await buildWomlRuntimeDefinitionPackage(document, {
        sourcePath: file,
        projectRoot: moduleRoot,
      });
      return {
        sourceBytes: Buffer.byteLength(source),
        outputBytes: Buffer.byteLength(JSON.stringify(packageValue)),
        nodes: packageValue.workflow.model.graph.nodes.length,
      };
    }, quick ? 3 : 10));
  }
  return results;
}

interface Span {
  readonly process: string;
  readonly name: string;
  readonly durationMs: number;
  readonly runId?: string;
}

function averageSpan(spans: readonly Span[], name: string, process?: string): number {
  const selected = spans.filter(span => span.name === name &&
    (process === undefined || span.process === process));
  if (selected.length === 0) return 0;
  return selected.reduce((sum, span) => sum + span.durationMs, 0) / selected.length;
}

async function attribution(root: string): Promise<Record<string, unknown>> {
  const tracePath = join(root, 'attribution.ndjson');
  const measured = await spawnJson([
    process.execPath,
    measurementPath,
    canonicalFixture,
    '--mode',
    'manual',
    '--warmups',
    '1',
    '--iterations',
    '3',
    '--profile-output',
    tracePath,
    '--json',
  ], { environment: { WOML_RUST_CORE_PATH: nativeCorePath } });
  const spans = (await readFile(tracePath, 'utf8')).trim().split('\n')
    .map(line => JSON.parse(line) as Span);
  const coldObserved = metricSamples(measured.value, 'activation.cold')[0]!;
  const coldStages = {
    bunBootstrap: averageSpan(spans, 'cli.process_to_profiler', 'cli'),
    compilation: averageSpan(spans, 'compiler.compile_inputs', 'cli'),
    coreServices: averageSpan(spans, 'runtime.start_core_services', 'cli'),
    observability: averageSpan(spans, 'runtime.initialize_observability', 'cli'),
    runtimeControl: averageSpan(spans, 'runtime.start_control', 'cli'),
    providerHosts: averageSpan(spans, 'runtime.start_provider_hosts', 'cli'),
    sourceRevalidation: averageSpan(spans, 'runtime.final_source_revalidation', 'cli'),
    openAdmission: averageSpan(spans, 'runtime.open_admission', 'cli'),
    retention: averageSpan(spans, 'runtime.schedule_retention', 'cli'),
    descriptor: averageSpan(spans, 'runtime.publish_descriptor', 'cli'),
    readyReceipt: averageSpan(spans, 'runtime.report_ready', 'cli'),
  };
  const coldAttributed = Object.values(coldStages).reduce((sum, value) => sum + value, 0);
  const manualSamples = metricSamples(measured.value, 'manual.visible');
  const warmObserved = manualSamples.reduce((sum, value) => sum + value, 0) / manualSamples.length;
  const warmStages = {
    admission: averageSpan(spans, 'runtime.admit_trigger', 'rust'),
    durableExecution: averageSpan(spans, 'runtime.execute_admitted_run', 'rust'),
    terminalPresentation: averageSpan(spans, 'presentation.present_terminal_result', 'cli'),
  };
  const warmAttributed = Object.values(warmStages).reduce((sum, value) => sum + value, 0);
  return {
    profilerEnabled: true,
    coldActivation: {
      observedMs: coldObserved,
      stages: coldStages,
      attributedMs: coldAttributed,
      residualMs: Math.max(0, coldObserved - coldAttributed),
      coveragePercent: Math.min(100, coldAttributed / coldObserved * 100),
    },
    warmManualVisible: {
      observedMeanMs: warmObserved,
      stages: warmStages,
      attributedMs: warmAttributed,
      residualMs: Math.max(0, warmObserved - warmAttributed),
      coveragePercent: Math.min(100, warmAttributed / warmObserved * 100),
    },
  };
}

const args = argumentsFrom(process.argv.slice(2));
for (const artifact of [nativeCorePath, scriptHostPath]) {
  if (!existsSync(artifact)) {
    throw new Error(`PERF6 requires release artifacts. Missing ${relative(projectRoot, artifact)}. Run bun run build first.`);
  }
}

const temporaryRoot = await mkdtemp(join(tmpdir(), 'woml-perf6-baseline-'));
try {
  const batches = args.batches;
  const smallIterations = args.quick ? 1 : 2;
  const runtimeCases: RuntimeCase[] = [
    { id: 'sequential-2', category: 'sequential', source: sequentialSource(2), workUnits: 2, iterationsPerBatch: smallIterations },
    { id: 'sequential-10', category: 'sequential', source: sequentialSource(10), workUnits: 10, iterationsPerBatch: smallIterations },
    { id: 'sequential-50', category: 'sequential', source: sequentialSource(50), workUnits: 50, iterationsPerBatch: 1 },
    { id: 'sequential-100', category: 'sequential', source: sequentialSource(100), workUnits: 100, iterationsPerBatch: 1 },
    { id: 'parallel-2', category: 'parallel', source: parallelSource(2, 2), workUnits: 4, iterationsPerBatch: smallIterations },
    { id: 'parallel-10', category: 'parallel', source: parallelSource(10, 4), workUnits: 12, iterationsPerBatch: smallIterations },
    { id: 'parallel-50', category: 'parallel', source: parallelSource(50, 4), workUnits: 52, iterationsPerBatch: 1 },
    { id: 'for-each-10', category: 'for_each', source: forEachSource(10, 4), workUnits: 12, iterationsPerBatch: smallIterations },
    { id: 'for-each-100', category: 'for_each', source: forEachSource(100, 4), workUnits: 102, iterationsPerBatch: 1 },
    { id: 'context-tiny', category: 'context', source: contextSource(32), workUnits: 4, iterationsPerBatch: smallIterations, expectedContextBytes: 96 },
    { id: 'context-100kb', category: 'context', source: contextSource(100 * 1024), workUnits: 4, iterationsPerBatch: 1, expectedContextBytes: 300 * 1024 },
    { id: 'context-1mb', category: 'context', source: contextSource(1024 * 1024), workUnits: 4, iterationsPerBatch: 1, expectedContextBytes: 3 * 1024 * 1024 },
  ];

  const headline = await headlineBaseline(temporaryRoot, batches);
  const runtimeSessions: PreparedRuntimeCase[] = [];
  for (const item of runtimeCases) {
    process.stderr.write(`[PERF6] preparing ${item.id}\n`);
    runtimeSessions.push(await prepareRuntimeCase(temporaryRoot, item));
  }
  for (let batch = 0; batch < batches; batch += 1) {
    const ordered = runtimeOrder(runtimeSessions, batch, batches);
    for (let position = 0; position < ordered.length; position += 1) {
      const session = ordered[position]!;
      process.stderr.write(
        `[PERF6] batch ${batch + 1}/${batches} · ${session.item.id}\n`
      );
      await measureRuntimeBatch(session, position);
    }
  }
  const runtime: Record<string, unknown>[] = [];
  for (const session of runtimeSessions) {
    process.stderr.write(`[PERF6] memory · ${session.item.id}\n`);
    runtime.push(await finishRuntimeCase(session, batches));
  }
  const compilation = await compileMatrix(temporaryRoot, args.quick);
  const presentation = (await spawnJson([
    process.execPath,
    presentationModesPath,
  ])).value;
  const localHttp = (await spawnJson([
    process.execPath,
    httpBenchmarkPath,
    '--iterations',
    args.quick ? '4' : '12',
    '--warmup',
    '2',
  ])).value;
  const attributed = await attribution(temporaryRoot);
  const cpu = cpus();
  const commit = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], {
    cwd: projectRoot,
    stdout: 'pipe',
  }).stdout.toString().trim();
  const report = {
    profile: 'woml.performance-baseline/v1',
    createdAt: new Date().toISOString(),
    commit,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      bunVersion: Bun.version,
      cpuModel: cpu[0]?.model ?? 'unknown',
      logicalCpuCount: Math.max(1, cpu.length),
      memoryBytes: totalmem(),
      nativeBuild: 'release',
      cliArtifact: 'built',
    },
    parameters: {
      batches,
      quick: args.quick,
      concurrencySafetyCap: 4,
      network: 'loopback-only',
      runtimeOrdering: 'interleaved-rotating-reversed',
      memorySampling: 'separate-after-latency',
    },
    headline,
    runtime,
    compilation,
    presentation,
    localHttp,
    attribution: attributed,
    historicalCronflow: {
      compared: false,
      revision: 'v0.11.1',
      reason: 'The available historical benchmark measures simulated pooling, caching, serialization, and SDK setup rather than an equivalent durable workflow boundary.',
    },
  };
  await mkdir(dirname(args.outputPath), { recursive: true });
  await writeFile(args.outputPath, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`${JSON.stringify({
    profile: report.profile,
    outputPath: args.outputPath,
    headline: report.headline,
    runtimeCases: report.runtime.length,
    compileCases: report.compilation.length,
  }, null, 2)}\n`);
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
