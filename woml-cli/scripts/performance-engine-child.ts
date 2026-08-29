#!/usr/bin/env bun

import { compileWoml, parseWoml } from '@woml/compiler';

import { executeWorkflowWithRustDurable } from '../src/rust-executor';
import { flushPerformanceProfile } from '../src/performance-profiler';
import {
  PERFORMANCE_SIGNAL_PROFILE,
  type PerformanceSignalV1,
} from './performance-measurement';

function emit(signal: PerformanceSignalV1): void {
  process.stdout.write(`${JSON.stringify(signal)}\n`);
}

function argument(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index === -1 ? undefined : process.argv[index + 1];
  if (value === undefined || value.startsWith('--')) {
    throw new Error(`The engine performance child requires ${name}.`);
  }
  return value;
}

function count(name: string, allowZero = false): number {
  const value = Number(argument(name));
  if (!Number.isSafeInteger(value) || value < (allowZero ? 0 : 1)) {
    throw new Error(`${name} must be ${allowZero ? 'a non-negative' : 'a positive'} integer.`);
  }
  return value;
}

try {
  const workflowPath = argument('--workflow');
  const statePath = argument('--state');
  const nativeCorePath = argument('--native-core');
  const scriptHostPath = argument('--script-host');
  const warmups = count('--warmups', true);
  const iterations = count('--iterations');
  const source = await Bun.file(workflowPath).text();
  const workflow = compileWoml(parseWoml(source, { file: workflowPath }));

  emit({ profile: PERFORMANCE_SIGNAL_PROFILE, type: 'child_ready' });
  const samples: number[] = [];
  for (let index = 0; index < warmups + iterations; index += 1) {
    const started = performance.now();
    const execution = await executeWorkflowWithRustDurable(workflow, statePath, {
      nativeCorePath,
      scriptHostPath,
      trigger: {},
    });
    const elapsed = performance.now() - started;
    if (execution.result === undefined) {
      throw new Error(`Engine benchmark run ${execution.runId} returned no result.`);
    }
    if (index >= warmups) samples.push(elapsed);
  }
  await flushPerformanceProfile();
  emit({ profile: PERFORMANCE_SIGNAL_PROFILE, type: 'engine_samples', samples });
} catch (error) {
  emit({
    profile: PERFORMANCE_SIGNAL_PROFILE,
    type: 'error',
    message: error instanceof Error ? error.message : String(error),
  });
  process.exitCode = 1;
}
