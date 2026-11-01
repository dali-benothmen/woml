#!/usr/bin/env bun
// One fresh benchmark process. Each one-shot invocation owns a new script host.
import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';
import { compileWoml, parseWoml } from '@woml/compiler';
import { executeApprovalWorkflowWithRust, executeWorkflowWithRustDurable, compiledDefinitionHash, type RuntimeCapabilitiesV1 } from '../src/rust-executor';
import { flushPerformanceProfile } from '../src/performance-profiler';
import { runtimeProfileFixtures, runtimeProfilePolicy } from './runtime-profile-fixtures';

function arg(name: string) {
  const index = process.argv.indexOf(name);
  const value = index < 0 ? undefined : process.argv[index + 1];
  if (!value || value.startsWith('--')) throw new Error(`Missing ${name}`);
  return value;
}

const fixture = runtimeProfileFixtures().find(item => item.id === arg('--case'));
assert.ok(fixture, 'Known profiling fixture');
const root = resolve(arg('--directory'));
const nativeCorePath = resolve(arg('--native-core'));
const host = resolve(import.meta.dir, '../src/script-host.ts');
const compilationStarted = performance.now();
const workflow = compileWoml(parseWoml(fixture.source, { file: `${fixture.id}.woml` }));
const compileMs = performance.now() - compilationStarted;
const runtimeCapabilities: RuntimeCapabilitiesV1 | undefined = fixture.governed ? {
  infrastructurePolicy: { policy: runtimeProfilePolicy, trust: {}, required: true },
} : undefined;
const samples = [];
for (let index = 0; index < 2; index += 1) {
  // Both stores and hosts are fresh. Invocation two has a warm native addon
  // and benchmark process; it does not reuse a host or a growing run history.
  const cpuBefore = process.cpuUsage();
  const started = performance.now();
  const options = {
    nativeCorePath, scriptHostPath: host, scriptTimeoutMs: 30_000,
    trigger: {}, runtimeCapabilities,
  };
  const statePath = join(root, `state-${index}.sqlite`);
  // The existing native API requires the outcome entrypoint for lock workflows
  // and rejects that entrypoint for ordinary workflows. Never settle a wait in
  // this measurement harness: the combined fixture must finish uncontended.
  const result = await (async () => {
    if (fixture.id !== 'combined') return executeWorkflowWithRustDurable(workflow, statePath, options);
    const outcome = await executeApprovalWorkflowWithRust(workflow, statePath, options);
    assert.equal(outcome.status, 'succeeded', 'Combined fixture must complete without waiting');
    assert.ok(outcome.status === 'succeeded');
    return outcome.execution;
  })();
  const durationMs = performance.now() - started;
  const cpu = process.cpuUsage(cpuBefore);
  assert.deepEqual(result.result, fixture.expected, 'Authoritative final result');
  assert.ok(result.events.length > 0, 'Real durable events');
  const hooks = result.events.filter(event => event.type === 'lifecycle_hook_requested');
  const failures = result.events.filter(event => ['lifecycle_action_failed', 'correction_attempt_failed', 'operation_failed'].includes(event.type));
  assert.equal(failures.length, 0, 'No hidden hook/corrective/service failures');
  if (fixture.id === 'combined') {
    assert.equal(hooks.filter(event => (event.data as { event?: string }).event === 'run_start').length, 1);
    assert.equal(result.events.filter(event => event.type === 'correction_attempt_succeeded').length, 1);
    assert.ok(result.events.some(event => event.type === 'infrastructure_policy_decision_recorded'));
  }
  samples.push({ temperature: index === 0 ? 'cold-process-fresh-store' : 'warm-process-fresh-store',
    durationMs, runId: result.runId, eventCount: result.events.length,
    ownProcessCpuMs: { user: cpu.user / 1000, system: cpu.system / 1000 },
  });
}
await flushPerformanceProfile();
console.log(JSON.stringify({ profile: 'woml.runtime-profile-sample/v1', case: fixture.id,
  compileMs, modelVersion: workflow.schemaVersion, definitionHash: compiledDefinitionHash(workflow),
  graphNodes: workflow.graph.nodes.length, governed: fixture.governed, scriptTimeoutMs: 30_000,
  samples, cpuScope: 'benchmark-process-including-native-threads; excludes-Bun-host-and-workers',
}));
