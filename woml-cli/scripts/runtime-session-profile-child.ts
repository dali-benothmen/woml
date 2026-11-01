#!/usr/bin/env bun
// Diagnostic embedding only: observe the existing scheduler, never schedule DAG work here.
import assert from 'node:assert/strict';
import { Database } from 'bun:sqlite';
import { join, resolve } from 'node:path';
import { compileWoml, parseWoml } from '@woml/compiler';
import {
  activateWebhookRuntimeWithRust, compiledDefinitionHash, executeWorkflowWithRustDurable,
  startWebhookRuntimeWithRust, stopWebhookRuntimeWithRust, submitManualTriggerWithRust,
  type RuntimeCapabilitiesV1, type TriggerProgressV1,
} from '../src/rust-executor';
import { flushPerformanceProfile } from '../src/performance-profiler';
import { runtimeProfileFixtures, runtimeProfilePolicy } from './runtime-profile-fixtures';

export const sessionModes = ['fresh-store', 'reused-store', 'activated'] as const;
export type SessionMode = typeof sessionModes[number];
const supportedCases = new Set(['sequential-2', 'sequential-32', 'storage-4']);
export function sessionFixture(id: string) {
  assert.ok(supportedCases.has(id), 'Use repeat-safe sequential-2, sequential-32, or storage-4');
  const fixture = runtimeProfileFixtures().find(fixture => fixture.id === id)!;
  // Activated target IDs require each hyphen segment to begin with a letter.
  // Apply this naming-only adaptation to EVERY mode and both libraries. Keep
  // historical sources untouched and disclose the changed definition identity.
  return { ...fixture, source: fixture.source.replace(`id="runtime-profile-${id}"`,
    `id="runtime-profile-${id.replace(/-(\d+)/gu, '-n$1')}"`) };
}

// A terminal callback can arrive before the admission Promise resolves. Retain
// it rather than polling inspection (which would perturb store-opening costs).
export class TerminalObserver {
  private readonly completed = new Map<string, { status: string; at: number }>();
  private readonly waiting = new Map<string, (value: { status: string; at: number }) => void>();
  record(progress: TriggerProgressV1): void {
    if (progress.type !== 'run_terminal') return;
    assert.ok(!this.completed.has(progress.runId), 'Duplicate terminal callback');
    const value = { status: progress.status, at: performance.now() };
    this.completed.set(progress.runId, value);
    this.waiting.get(progress.runId)?.(value);
  }
  async wait(runId: string, timeoutMs = 60_000) {
    const existing = this.completed.get(runId);
    if (existing) return existing;
    assert.ok(!this.waiting.has(runId), 'Only one observer per run');
    return new Promise<{ status: string; at: number }>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.waiting.delete(runId);
        reject(new Error(`Terminal callback timed out: ${runId}`));
      }, timeoutMs);
      this.waiting.set(runId, value => {
        clearTimeout(timeout);
        this.waiting.delete(runId);
        resolve(value);
      });
    });
  }
}

export function verifySessionHistory(events: readonly { type: string; sequence: number; data: any }[], expected: unknown) {
  assert.ok(events.length > 0, 'Real durable events');
  events.forEach((event, index) => assert.equal(event.sequence, index + 1));
  assert.equal(events.filter(event => ['operation_failed', 'lifecycle_action_failed', 'correction_attempt_failed', 'run_failed'].includes(event.type)).length, 0, 'No hidden failures');
  const outcomes = events.filter(event => event.type === 'run_outcome_decided');
  assert.equal(outcomes.length, 1, 'Exactly one business outcome');
  assert.equal(outcomes[0]!.data.outcome, 'succeeded');
  assert.deepEqual(outcomes[0]!.data.result, expected, 'Durable authoritative result');
  assert.equal(events.at(-1)!.type, 'run_finalized');
}

export interface SessionSample {
  index: number; runId: string; durationMs: number; admissionMs?: number;
  ownProcessCpuMs: number; statePath: string; eventCount?: number;
}
export interface SessionResult {
  profile: 'woml.runtime-session-sample/v1'; case: string; mode: SessionMode;
  definitionHash: string; modelVersion: number; compileMs: number;
  prepareMs?: number; activateMs?: number; shutdownMs?: number;
  samples: SessionSample[];
}

async function main() {
  const args = new Map<string, string>();
  const argv = process.argv.slice(2);
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index]!, value = argv[index + 1];
    assert.ok(['--case', '--mode', '--directory', '--native-core', '--runs'].includes(key) && value && !args.has(key), 'Invalid child argument');
    args.set(key, value);
  }
  const fixture = sessionFixture(args.get('--case')!);
  const mode = args.get('--mode') as SessionMode;
  assert.ok(sessionModes.includes(mode), 'Unknown session mode');
  const runs = Number(args.get('--runs') ?? 5);
  assert.ok(Number.isSafeInteger(runs) && runs >= 2 && runs <= 5);
  assert.ok(args.get('--directory') && args.get('--native-core'));
  const root = resolve(args.get('--directory')!);
  const nativeCorePath = resolve(args.get('--native-core')!);
  const compilationStarted = performance.now();
  const workflow = compileWoml(parseWoml(fixture.source, { file: `${fixture.id}.woml` }));
  const compileMs = performance.now() - compilationStarted;
  const runtimeCapabilities: RuntimeCapabilitiesV1 | undefined = fixture.governed
    ? { infrastructurePolicy: { policy: { ...runtimeProfilePolicy,
      rules: runtimeProfilePolicy.rules.map(rule => ({ ...rule, match: { ...rule.match, workflows: [workflow.workflowId] } })) }, trust: {}, required: true } } : undefined;
  const options = { nativeCorePath, scriptHostPath: resolve(import.meta.dir, '../src/script-host.ts'),
    scriptTimeoutMs: 30_000, runtimeCapabilities };
  const statePath = join(root, 'state.sqlite');
  const observer = new TerminalObserver();
  let runtimeId: string | undefined;
  const result: SessionResult = { profile: 'woml.runtime-session-sample/v1', case: fixture.id, mode,
    definitionHash: compiledDefinitionHash(workflow), modelVersion: workflow.schemaVersion, compileMs, samples: [] };
  try {
    if (mode === 'activated') {
      const started = performance.now();
      const runtime = await startWebhookRuntimeWithRust([{ workflow, definitionHash: result.definitionHash, resolvedSecrets: {}, runtimeModules: [] }], statePath,
        { ...options, host: '127.0.0.1', port: 0, startSuspended: true, shutdownTimeoutMs: 10_000,
          deploymentId: 'runtime-session-profile', telemetryEnabled: false,
          onTriggerProgress: progress => observer.record(progress) });
      runtimeId = runtime.runtimeId;
      result.prepareMs = performance.now() - started;
      const activationStarted = performance.now();
      await activateWebhookRuntimeWithRust(runtimeId, options);
      result.activateMs = performance.now() - activationStarted;
    }
    for (let index = 0; index < runs; index++) {
      const path = mode === 'fresh-store' ? join(root, `state-${index}.sqlite`) : statePath;
      const cpuBefore = process.cpuUsage();
      const started = performance.now();
      let runId: string, durationMs: number, admissionMs: number | undefined;
      if (mode === 'activated') {
        const admission = await submitManualTriggerWithRust(runtimeId!, {
          profile: 'woml.manual-trigger-admission/v1', type: 'request', requestId: `session-${index}`,
          workflowId: workflow.workflowId, triggerId: 'start', payload: {}, requestedAt: new Date().toISOString(),
        }, options);
        admissionMs = performance.now() - started;
        assert.equal(admission.type, 'accepted', JSON.stringify(admission));
        assert.ok(admission.type === 'accepted');
        runId = admission.runId;
        const terminal = await observer.wait(runId);
        durationMs = terminal.at - started;
        assert.equal(terminal.status, 'succeeded');
      } else {
        const execution = await executeWorkflowWithRustDurable(workflow, path, { ...options, trigger: {} });
        durationMs = performance.now() - started;
        assert.deepEqual(execution.result, fixture.expected);
        runId = execution.runId;
      }
      const cpu = process.cpuUsage(cpuBefore);
      result.samples.push({ index, runId, statePath: path, durationMs, admissionMs,
        ownProcessCpuMs: (cpu.user + cpu.system) / 1000 });
    }
  } finally {
    if (runtimeId) {
      const started = performance.now();
      await stopWebhookRuntimeWithRust(runtimeId, options);
      result.shutdownMs = performance.now() - started;
    }
  }
  // Verify only after all timing and runtime shutdown. Read-only SQLite avoids
  // adding native open/initialization spans or inspector polling to the run.
  for (const sample of result.samples) {
    const database = new Database(sample.statePath, { readonly: true });
    try {
      const rows = database.query('SELECT event_json FROM woml_run_events WHERE run_id = ?1 ORDER BY sequence').all(sample.runId) as { event_json: string }[];
      const events = rows.map(row => JSON.parse(row.event_json));
      verifySessionHistory(events, fixture.expected);
      sample.eventCount = events.length;
    } finally { database.close(); }
  }
  assert.equal(new Set(result.samples.map(sample => sample.runId)).size, runs);
  await flushPerformanceProfile();
  console.log(JSON.stringify(result));
}

if (import.meta.main) await main();
