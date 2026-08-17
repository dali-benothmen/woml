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
  executeWorkflowWithRustDurable,
  inspectRunV2WithRust,
  listRunsWithRust,
  RustWorkflowExecutionError,
} from '../src/rust-executor';

interface Summary {
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
  if (!Number.isSafeInteger(value)) throw new Error(`${name} requires an integer.`);
  return value;
}

const iterations = integerOption('--iterations', 5);
const warmup = integerOption('--warmup', 1);
if (iterations < 1 || iterations > 25 || warmup < 0 || warmup > 5) {
  throw new Error('RP7 benchmark bounds are --iterations 1..25 and --warmup 0..5.');
}
if (!(await Bun.file(nativeCorePath).exists())) {
  throw new Error('Build the packaged CLI first with: bun run build');
}

function compile(source: string, file: string): CompiledWorkflowDefinition {
  return compileWoml(parseWoml(source, { file }));
}

function basicModel(): CompiledWorkflowDefinition {
  return compile(
    `<woml>
<workflow id="rp7-disabled" name="RP7 disabled" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="finish"><script>return { ok: true };</script></step></steps>
</workflow>
</woml>`,
    'rp7-disabled.woml'
  );
}

function policyModel(
  id: string,
  script = 'return { ok: true };',
  config = 'concurrency="2" timeout="5s" queue="rp7-benchmark"'
): CompiledWorkflowDefinition {
  return compile(
    `<woml>
<workflow id="${id}" name="RP7 policy" version="1.0.0">
  <config ${config} />
  <triggers><manual id="start" /></triggers>
  <steps><step id="finish"><script>${script}</script></step></steps>
</workflow>
</woml>`,
    `${id}.woml`
  );
}

function summarize(samples: readonly number[]): Summary {
  const sorted = [...samples].sort((left, right) => left - right);
  return {
    meanMs: samples.reduce((sum, value) => sum + value, 0) / samples.length,
    p95Ms: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)]!,
  };
}

async function profileExecutions(
  profiles: readonly {
    readonly name: 'noConfig' | 'policy';
    readonly model: CompiledWorkflowDefinition;
    readonly statePath: string;
  }[]
): Promise<Record<'noConfig' | 'policy', Summary>> {
  const samples = {
    noConfig: [] as number[],
    policy: [] as number[],
  };
  for (let round = 0; round < warmup + iterations; round += 1) {
    const offset = round % profiles.length;
    const ordered = [...profiles.slice(offset), ...profiles.slice(0, offset)];
    for (const profile of ordered) {
      const started = performance.now();
      await executeWorkflowWithRustDurable(profile.model, profile.statePath, {
        nativeCorePath,
      });
      if (round >= warmup) samples[profile.name].push(performance.now() - started);
    }
  }
  return {
    noConfig: summarize(samples.noConfig),
    policy: summarize(samples.policy),
  };
}

function syncMean(count: number, operation: () => void): number {
  const started = performance.now();
  for (let index = 0; index < count; index += 1) operation();
  return (performance.now() - started) / count;
}

const directory = await mkdtemp(join(tmpdir(), 'woml-rp7-benchmark-'));
try {
  const base = basicModel();
  const enabled = policyModel('rp7-enabled');
  const profiles = await profileExecutions([
    { name: 'noConfig', model: base, statePath: join(directory, 'no-config.sqlite') },
    { name: 'policy', model: enabled, statePath: join(directory, 'policy.sqlite') },
  ]);

  const operatorState = join(directory, 'operator.sqlite');
  const completed = await executeWorkflowWithRustDurable(enabled, operatorState, {
    nativeCorePath,
  });
  const operatorIterations = Math.max(20, iterations * 10);
  const listMeanMs = syncMean(operatorIterations, () => {
    listRunsWithRust(operatorState, { limit: 20 }, { nativeCorePath });
  });
  const getMeanMs = syncMean(operatorIterations, () => {
    inspectRunV2WithRust(operatorState, completed.runId, { nativeCorePath });
  });

  const burstSize = Math.max(8, iterations * 2);
  const burst = policyModel(
    'rp7-burst',
    'await new Promise(resolve => setTimeout(resolve, 10)); return { ok: true };',
    'concurrency="2" timeout="5s" queue="rp7-burst"'
  );
  const burstState = join(directory, 'burst.sqlite');
  await executeWorkflowWithRustDurable(burst, burstState, { nativeCorePath });
  const burstStarted = performance.now();
  const burstResults = await Promise.all(
    Array.from({ length: burstSize }, () =>
      executeWorkflowWithRustDurable(burst, burstState, {
        nativeCorePath,
      })
    )
  );
  const burstMs = performance.now() - burstStarted;
  if (new Set(burstResults.map(result => result.runId)).size !== burstSize) {
    throw new Error('RP7 burst benchmark did not produce unique durable runs.');
  }

  const timeout = policyModel(
    'rp7-timeout-detection',
    'await new Promise(resolve => setTimeout(resolve, 5000)); return { late: true };',
    'concurrency="1" timeout="100ms" queue="rp7-timeout"'
  );
  const timeoutStarted = performance.now();
  let timeoutError: unknown;
  try {
    await executeWorkflowWithRustDurable(timeout, join(directory, 'timeout.sqlite'), {
      nativeCorePath,
    });
  } catch (error) {
    timeoutError = error;
  }
  const timeoutDetectionMs = performance.now() - timeoutStarted;
  if (
    !(timeoutError instanceof RustWorkflowExecutionError) ||
    timeoutError.code !== 'WOML_WORKFLOW_TIMED_OUT'
  ) {
    throw new Error('RP7 timeout benchmark did not settle with the durable timeout code.');
  }

  const rate = policyModel(
    'rp7-rate-eligibility',
    'return { ok: true };',
    'concurrency="2" rate-limit="1/2s" timeout="5s" queue="rp7-rate"'
  );
  const rateState = join(directory, 'rate.sqlite');
  await executeWorkflowWithRustDurable(rate, rateState, { nativeCorePath });
  const rateStarted = performance.now();
  await executeWorkflowWithRustDurable(rate, rateState, { nativeCorePath });
  const rateEligibilityMs = performance.now() - rateStarted;

  const memoryState = join(directory, 'memory.sqlite');
  const memoryBefore = process.memoryUsage().rss;
  const memoryIterations = Math.max(10, iterations * 4);
  for (let index = 0; index < memoryIterations; index += 1) {
    await executeWorkflowWithRustDurable(enabled, memoryState, { nativeCorePath });
  }
  Bun.gc(true);
  const memoryGrowthBytes = Math.max(0, process.memoryUsage().rss - memoryBefore);

  const policyAdmissionAndExecutionMs = Math.max(
    0,
    profiles.policy.meanMs - profiles.noConfig.meanMs
  );
  const burstPerRunMs = burstMs / burstSize;
  const budgets = {
    noConfigMeanMs: 2_000,
    policyAdmissionAndExecutionMs: 250,
    burstPerRunMs: 500,
    listMeanMs: 25,
    getMeanMs: 50,
    rateEligibilityMs: 3_000,
    timeoutDetectionMs: 1_500,
    memoryGrowthBytes: 128 * 1024 * 1024,
  };
  const withinBudgets =
    profiles.noConfig.meanMs <= budgets.noConfigMeanMs &&
    policyAdmissionAndExecutionMs <= budgets.policyAdmissionAndExecutionMs &&
    burstPerRunMs <= budgets.burstPerRunMs &&
    listMeanMs <= budgets.listMeanMs &&
    getMeanMs <= budgets.getMeanMs &&
    rateEligibilityMs <= budgets.rateEligibilityMs &&
    timeoutDetectionMs <= budgets.timeoutDetectionMs &&
    memoryGrowthBytes <= budgets.memoryGrowthBytes;

  process.stdout.write(
    `${JSON.stringify({
      benchmark: 'woml-runtime-policies-local-v1',
      iterations,
      warmup,
      profiles,
      policyDisabledBaselineMs: profiles.noConfig.meanMs,
      policyAdmissionAndExecutionMs,
      queueBurst: { runs: burstSize, totalMs: burstMs, perRunMs: burstPerRunMs },
      operator: { listMeanMs, getMeanMs },
      rateEligibilityMs,
      timeoutDetectionMs,
      memory: { iterations: memoryIterations, growthBytes: memoryGrowthBytes },
      budgets,
      withinBudgets,
    })}\n`
  );
  if (!withinBudgets) process.exitCode = 1;
} finally {
  await rm(directory, { recursive: true, force: true });
}
