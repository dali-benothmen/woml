#!/usr/bin/env bun

import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

import { followWorkflowLogs } from '../src/log-follower';
import {
  executeWorkflowWithRustDurable,
  inspectRunPresentationWithRust,
} from '../src/rust-executor';
import {
  decodeRunPresentationV1,
  renderRunPresentation,
  type RunPresentationV1,
} from '../src/terminal-presentation';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const nativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const sourcePath = resolve(
  projectRoot,
  'examples/terminalExperience/sequential.woml'
);
const budgets = await Bun.file(resolve(
  projectRoot,
  'examples/terminalExperience/performance-budgets.v1.json'
)).json() as Record<string, number | string>;
const root = await mkdtemp(join(tmpdir(), 'woml-terminal-benchmark-'));

function average(iterations: number, operation: (index: number) => void): number {
  for (let index = 0; index < Math.min(iterations, 20); index += 1) operation(index);
  const started = performance.now();
  for (let index = 0; index < iterations; index += 1) operation(index);
  return (performance.now() - started) / iterations;
}

async function asyncAverage(
  iterations: number,
  operation: (index: number) => Promise<void>
): Promise<number> {
  for (let index = 0; index < Math.min(iterations, 5); index += 1) await operation(index);
  const started = performance.now();
  for (let index = 0; index < iterations; index += 1) await operation(index);
  return (performance.now() - started) / iterations;
}

function maximum(name: string, value: number): void {
  const budget = Number(budgets[`${name}Max`]);
  if (!Number.isFinite(value) || !Number.isFinite(budget) || value > budget) {
    throw new Error(`${name} exceeded: ${value.toFixed(3)} > ${budget}.`);
  }
}

function minimum(name: string, value: number): void {
  const budget = Number(budgets[`${name}Min`]);
  if (!Number.isFinite(value) || !Number.isFinite(budget) || value < budget) {
    throw new Error(`${name} fell below: ${value.toFixed(2)} < ${budget}.`);
  }
}

try {
  const source = await Bun.file(sourcePath).text();
  const workflow = compileWoml(parseWoml(source, { file: sourcePath }));
  const statePath = join(root, 'state.sqlite');
  const execution = await executeWorkflowWithRustDurable(workflow, statePath, {
    nativeCorePath,
    scriptHostPath: resolve(packageRoot, 'dist/script-host.js'),
  });
  const presentation = inspectRunPresentationWithRust(
    statePath,
    execution.runId,
    { nativeCorePath }
  );
  const encoded = JSON.stringify(presentation);

  const rendererAverageMs = average(2_000, () => {
    renderRunPresentation(presentation, {
      format: 'plain', width: 80, unicode: true, timeZone: 'UTC',
    });
  });
  const projectionAverageMs = average(500, () => {
    inspectRunPresentationWithRust(statePath, execution.runId, { nativeCorePath });
  });
  const retainedAttachAverageMs = await asyncAverage(100, async () => {
    await followWorkflowLogs({
      args: {
        subject: execution.runId,
        subjectKind: 'run',
        statePath,
        json: false,
        color: 'never',
      },
      io: { stdout: () => {}, stderr: () => {}, isTTY: false, columns: 80 },
      dependencies: {
        readRun: () => presentation,
        readWorkflow: () => { throw new Error('unexpected workflow query'); },
        hasWorkflow: () => false,
        readDescriptor: async () => { throw new Error('terminal history must not attach live'); },
      },
    });
  });
  const liveRefreshAverageMs = average(1_000, () => {
    renderRunPresentation(decodeRunPresentationV1(encoded), {
      format: 'plain', width: 80, unicode: true, timeZone: 'UTC',
    });
  });

  Bun.gc(true);
  const memoryBefore = process.memoryUsage().rss;
  const active = Array.from({ length: 2_000 }, (_, index) => ({
    ...structuredClone(presentation),
    runId: `run_memory_${index}`,
    status: index % 2 === 0 ? 'running' as const : 'waiting' as const,
  }));
  const memoryAfter = process.memoryUsage().rss;
  const measuredMemoryPerRun = Math.max(0, memoryAfter - memoryBefore) / active.length;
  const serializedMemoryFloor = new TextEncoder().encode(encoded).byteLength * 2;
  const memoryPerActiveRunBytes = Math.max(
    measuredMemoryPerRun,
    serializedMemoryFloor
  );
  if (active.length !== 2_000) throw new Error('active-run memory fixture was optimized away');

  const large: RunPresentationV1 = {
    ...structuredClone(presentation),
    result: {
      rows: Array.from({ length: 20 }, (_, index) => ({
        index,
        label: `row-${index}-${'x'.repeat(450)}`,
      })),
      text: '界'.repeat(500),
    },
  };
  const largeResultSummaryAverageMs = average(200, () => {
    renderRunPresentation(large, { format: 'plain', width: 80, unicode: true });
  });

  const streamCount = 2_000;
  const streamStarted = performance.now();
  for (let index = 0; index < streamCount; index += 1) {
    renderRunPresentation({
      ...presentation,
      runId: `run_stream_${index}`,
    }, { format: 'json' });
  }
  const streamElapsedMs = performance.now() - streamStarted;
  const highRateRunPresentationsPerSecond = streamCount / (streamElapsedMs / 1_000);

  const results = {
    profile: 'woml.terminal-performance-results/v1',
    rendererAverageMs,
    projectionAverageMs,
    retainedAttachAverageMs,
    liveRefreshAverageMs,
    memoryPerActiveRunBytes,
    largeResultSummaryAverageMs,
    highRateRunPresentationsPerSecond,
  };
  maximum('rendererAverageMs', rendererAverageMs);
  maximum('projectionAverageMs', projectionAverageMs);
  maximum('retainedAttachAverageMs', retainedAttachAverageMs);
  maximum('liveRefreshAverageMs', liveRefreshAverageMs);
  maximum('memoryPerActiveRunBytes', memoryPerActiveRunBytes);
  maximum('largeResultSummaryAverageMs', largeResultSummaryAverageMs);
  minimum('highRateRunPresentationsPerSecond', highRateRunPresentationsPerSecond);
  console.log(JSON.stringify(results));
} finally {
  await rm(root, { recursive: true, force: true });
}
