#!/usr/bin/env bun

import { RuntimeObservability } from '../src/runtime-observability';

const disabledIterations = 1_000_000;
let disabledCalls = 0;
const disabledStarted = performance.now();
for (let index = 0; index < disabledIterations; index += 1) {
  const observability: RuntimeObservability | undefined = undefined;
  observability?.record('run', 'never', 'never');
  if (observability !== undefined) disabledCalls += 1;
}
const disabledElapsedMs = performance.now() - disabledStarted;
if (disabledCalls !== 0 || disabledElapsedMs > 250) {
  throw new Error(
    `Disabled observability exceeded its no-allocation branch budget: ${disabledElapsedMs.toFixed(2)}ms.`
  );
}

const observed = new RuntimeObservability({
  runtimeInstanceId: 'runtime_benchmark',
  deploymentId: 'deployment_benchmark',
  workflows: [],
  listRuns: () => ({ profile: 'woml.run-list/v2', runs: [] }),
  storeSize: async () => 0,
  logFormat: 'json',
  emitLog: () => {},
});
const eventCount = 20_000;
const enabledStarted = performance.now();
for (let index = 0; index < eventCount; index += 1) {
  observed.record('run', `run_${index}`, 'running');
}
const enabledElapsedMs = performance.now() - enabledStarted;
if (enabledElapsedMs > 2_000) {
  throw new Error(
    `Observability stream normalization exceeded its budget: ${enabledElapsedMs.toFixed(2)}ms.`
  );
}
const snapshotStarted = performance.now();
await observed.snapshot();
const snapshotElapsedMs = performance.now() - snapshotStarted;
if (snapshotElapsedMs > 100) {
  throw new Error(
    `Empty observability snapshot exceeded its budget: ${snapshotElapsedMs.toFixed(2)}ms.`
  );
}

console.log(
  `[PRO5 benchmark] disabled=${disabledElapsedMs.toFixed(2)}ms/${disabledIterations}, stream=${enabledElapsedMs.toFixed(2)}ms/${eventCount}, snapshot=${snapshotElapsedMs.toFixed(2)}ms`
);
