#!/usr/bin/env bun

import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  compileWoml,
  parseWoml,
  type CompiledWorkflowDefinition,
} from '@woml/compiler';

import {
  cancelRunWithRust,
  executeWorkflowWithRustDurable,
  inspectRunV2WithRust,
  listRunsWithRust,
  RustWorkflowExecutionError,
} from '../src/rust-executor';

interface SampleSummary {
  readonly meanMs: number;
  readonly p95Ms: number;
}

const packageRoot = resolve(import.meta.dir, '..');
const nativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);

function integerOption(name: string, fallback: number): number {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = Number(process.argv[index + 1]);
  if (!Number.isSafeInteger(value)) {
    throw new Error(`${name} requires an integer.`);
  }
  return value;
}

const iterations = integerOption('--iterations', 5);
const warmup = integerOption('--warmup', 1);
if (iterations < 1 || iterations > 50 || warmup < 0 || warmup > 10) {
  throw new Error(
    'LEC8 benchmark bounds are --iterations 1..50 and --warmup 0..10.'
  );
}
if (!(await Bun.file(nativeCorePath).exists())) {
  throw new Error('Build the packaged CLI first with: bun run build');
}

function compile(source: string, file: string): CompiledWorkflowDefinition {
  return compileWoml(parseWoml(source, { file }));
}

function modelWithLifecycle(): CompiledWorkflowDefinition {
  return compile(
    `<woml>
<workflow id="lec8-benchmark" name="LEC8 benchmark" version="1.0.0">
  <lifecycle>
    <on-complete><script>return;</script></on-complete>
  </lifecycle>
  <triggers><manual id="start" /></triggers>
  <steps><step id="work"><script>return { ok: true };</script></step></steps>
</workflow>
</woml>`,
    'lec8-benchmark.woml'
  );
}

function withoutLifecycle(
  model: CompiledWorkflowDefinition,
  schemaVersion: 8 | 11
): CompiledWorkflowDefinition {
  const value = structuredClone(model) as CompiledWorkflowDefinition & {
    schemaVersion: number;
    lifecycle?: unknown;
  };
  value.schemaVersion = schemaVersion;
  delete value.lifecycle;
  return value;
}

function cancellableModel(): CompiledWorkflowDefinition {
  return compile(
    `<woml>
<workflow id="lec8-cancellable" name="LEC8 cancellable" version="1.0.0">
  <lifecycle>
    <on-cancel><script>return;</script></on-cancel>
    <on-complete><script>return;</script></on-complete>
  </lifecycle>
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="wait"><script>await new Promise(resolve => setTimeout(resolve, 10000)); return { late: true };</script></step>
  </steps>
</workflow>
</woml>`,
    'lec8-cancellable.woml'
  );
}

function summarize(samples: readonly number[]): SampleSummary {
  const sorted = [...samples].sort((left, right) => left - right);
  return {
    meanMs: samples.reduce((total, value) => total + value, 0) / samples.length,
    p95Ms: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)]!,
  };
}

async function executionProfiles(
  profiles: readonly {
    readonly name: 'legacy' | 'disabled' | 'lifecycle';
    readonly model: CompiledWorkflowDefinition;
    readonly statePath: string;
  }[]
): Promise<Record<'legacy' | 'disabled' | 'lifecycle', SampleSummary>> {
  const samples = {
    legacy: [] as number[],
    disabled: [] as number[],
    lifecycle: [] as number[],
  };
  for (let round = 0; round < warmup + iterations; round += 1) {
    const offset = round % profiles.length;
    const ordered = [...profiles.slice(offset), ...profiles.slice(0, offset)];
    for (const profile of ordered) {
      const started = performance.now();
      await executeWorkflowWithRustDurable(profile.model, profile.statePath, {
        nativeCorePath,
      });
      const elapsed = performance.now() - started;
      if (round >= warmup) samples[profile.name].push(elapsed);
    }
  }
  return {
    legacy: summarize(samples.legacy),
    disabled: summarize(samples.disabled),
    lifecycle: summarize(samples.lifecycle),
  };
}

function syncMean(iterationCount: number, operation: () => void): number {
  const started = performance.now();
  for (let index = 0; index < iterationCount; index += 1) operation();
  return (performance.now() - started) / iterationCount;
}

const directory = await mkdtemp(join(tmpdir(), 'woml-lec8-benchmark-'));
try {
  const enabled = modelWithLifecycle();
  const disabledV11 = withoutLifecycle(enabled, 11);
  const legacyV8 = withoutLifecycle(enabled, 8);
  const profiles = await executionProfiles([
    {
      name: 'legacy',
      model: legacyV8,
      statePath: join(directory, 'legacy-v8.sqlite'),
    },
    {
      name: 'disabled',
      model: disabledV11,
      statePath: join(directory, 'disabled-v11.sqlite'),
    },
    {
      name: 'lifecycle',
      model: enabled,
      statePath: join(directory, 'enabled-v11.sqlite'),
    },
  ]);
  const { legacy, disabled, lifecycle } = profiles;

  const inspectionState = join(directory, 'inspection.sqlite');
  const completed = await executeWorkflowWithRustDurable(
    disabledV11,
    inspectionState,
    { nativeCorePath }
  );
  const operatorIterations = Math.max(20, iterations * 10);
  const listMeanMs = syncMean(operatorIterations, () => {
    listRunsWithRust(
      inspectionState,
      { limit: 20 },
      { nativeCorePath }
    );
  });
  const getMeanMs = syncMean(operatorIterations, () => {
    inspectRunV2WithRust(inspectionState, completed.runId, { nativeCorePath });
  });

  const cancellationState = join(directory, 'cancellation.sqlite');
  const cancellationExecution = executeWorkflowWithRustDurable(
    cancellableModel(),
    cancellationState,
    { nativeCorePath }
  );
  const runningDeadline = Date.now() + 5_000;
  let cancellationRunId: string | undefined;
  while (cancellationRunId === undefined) {
    cancellationRunId = listRunsWithRust(
      cancellationState,
      { status: 'running', limit: 1 },
      { nativeCorePath }
    ).runs[0]?.runId;
    if (Date.now() >= runningDeadline) {
      throw new Error('The cancellation benchmark run did not start.');
    }
    await Bun.sleep(5);
  }
  const cancellationStarted = performance.now();
  const cancellation = cancelRunWithRust(
    cancellationState,
    cancellationRunId,
    'cancel_lec8_benchmark',
    { nativeCorePath }
  );
  const cancellationRequestMs = performance.now() - cancellationStarted;
  if (cancellation.status !== 'accepted') {
    throw new Error(`Cancellation benchmark was ${cancellation.status}.`);
  }
  let cancellationError: unknown;
  try {
    await cancellationExecution;
  } catch (error) {
    cancellationError = error;
  }
  if (
    !(cancellationError instanceof RustWorkflowExecutionError) ||
    cancellationError.code !== 'WOML_RUN_CANCELLED'
  ) {
    throw new Error('Cancellation benchmark did not settle as cancelled.');
  }
  const cancellationDetectionMs = performance.now() - cancellationStarted;
  const cancelled = inspectRunV2WithRust(
    cancellationState,
    cancellationRunId,
    { nativeCorePath }
  );
  if (cancelled.status !== 'cancelled') {
    throw new Error('Cancellation benchmark did not finalize durably.');
  }

  const disabledRegressionMs = disabled.meanMs - legacy.meanMs;
  const lifecycleFinalizationMs = Math.max(0, lifecycle.meanMs - disabled.meanMs);
  const budgets = {
    lifecycleDisabledRegressionMs: 75,
    lifecycleFinalizationMs: 250,
    listMeanMs: 25,
    getMeanMs: 50,
    cancellationRequestMs: 100,
    cancellationDetectionMs: 1_500,
  };
  const withinBudgets =
    disabledRegressionMs <= budgets.lifecycleDisabledRegressionMs &&
    lifecycleFinalizationMs <= budgets.lifecycleFinalizationMs &&
    listMeanMs <= budgets.listMeanMs &&
    getMeanMs <= budgets.getMeanMs &&
    cancellationRequestMs <= budgets.cancellationRequestMs &&
    cancellationDetectionMs <= budgets.cancellationDetectionMs;

  process.stdout.write(
    `${JSON.stringify({
      benchmark: 'woml-lifecycle-controls-local-v1',
      iterations,
      warmup,
      legacyV8: legacy,
      lifecycleDisabledV11: disabled,
      lifecycleEnabledV11: lifecycle,
      lifecycleDisabledRegressionMs: disabledRegressionMs,
      lifecycleFinalizationMs,
      runControl: {
        listMeanMs,
        getMeanMs,
        cancellationRequestMs,
        cancellationDetectionMs,
      },
      budgets,
      withinBudgets,
    })}\n`
  );
  if (!withinBudgets) process.exitCode = 1;
} finally {
  await rm(directory, { recursive: true, force: true });
}
