import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import contract from '../../docs/invocation-setup-acceptance.v1.json';
import { cohorts, fixture, sha256 } from '../scripts/invocation-setup-fixtures';
import { summarizeInvocationBaseline, summarizeInvocationTrace, validateInvocationCapture } from '../scripts/invocation-setup-measurement';
import { traceQuality } from '../scripts/invocation-setup-analysis';

function report() {
  return { profile: 'woml.invocation-setup-capture/v1', mode: 'timing', releaseAccepted: false,
    rows: cohorts.map(cohort => {
      const spec = fixture(cohort), requestRows = [{ batch: 'first', index: 0, wallMs: 1, hostDurationMs: 1 }];
      const batches = Array.from({ length: 4 }, (_, sample) => {
        for (let index = 0; index < spec.count; index++) requestRows.push({ batch: `sample${sample}`, index, wallMs: 1, hostDurationMs: 1 });
        return { sample, warmup: sample === 0, invocations: spec.count, wallMs: sample === 0 ? 9999 : 32 };
      });
      return { cohort, fixtureSha256: spec.fixtureSha256, concurrency: spec.concurrency,
        readyMs: 1, registrationMs: 1, firstInvocationMs: 1, teardownMs: 1, lifetimeMs: 100,
        batches, requestRows, completed: requestRows.length, allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null };
    }) };
}

test('ISR freezes real invocations, target, controls and fresh-worker isolation before edits', () => {
  expect(contract.baselineCommit).toBe('3064a2b5092b1b24da328f9843965139d9079407');
  expect(contract.primary).toEqual({ cohort: 'mixed', invocationsPerBatch: 32, metric: 'wallMs', minimumImprovementPercent: 10 });
  expect(contract.processesPerArtifact).toBe(7); expect(contract.isolation.reusedExecutedWorkers).toBe(false);
  expect(contract.releasePrerequisites).toContain('prepared-runtime-locking-correction');
});
test('fixtures are deterministic, source-integrity-bound and contain no resolved bindings', () => {
  for (const cohort of cohorts) {
    const spec = fixture(cohort); expect(spec.fixtureSha256).toBe(fixture(cohort).fixtureSha256);
    for (const artifact of spec.artifacts) {
      expect(artifact.bundleDigest).toBe(`sha256:${sha256(artifact.bundle)}`);
      expect(artifact.sourceMapDigest).toBe(`sha256:${sha256(artifact.sourceMap)}`);
    }
    expect(spec.request(0, 'a').bindings.secrets).toEqual({});
    expect(spec.request(0, 'a').invocationId).not.toBe(spec.request(0, 'b').invocationId);
  }
  expect(fixture('mixed').request(0, 'a').modules).toHaveLength(0);
  expect(fixture('mixed').request(1, 'a').modules).toHaveLength(1);
});
test('capture rejects missing/duplicate completions, relabelled warmups and substituted inputs', () => {
  expect(() => validateInvocationCapture(report())).not.toThrow();
  for (const mutate of [
    (r: ReturnType<typeof report>) => { r.rows.pop(); },
    (r: ReturnType<typeof report>) => { r.rows[0]!.requestRows.pop(); },
    (r: ReturnType<typeof report>) => { r.rows[0]!.requestRows[1] = r.rows[0]!.requestRows[0]!; },
    (r: ReturnType<typeof report>) => { r.rows[0]!.batches[1]!.warmup = true; },
    (r: ReturnType<typeof report>) => { r.rows[0]!.fixtureSha256 = 'a'.repeat(64); },
    (r: ReturnType<typeof report>) => { r.rows[0]!.batches[1]!.wallMs = NaN; },
  ]) { const r = report(); mutate(r); expect(() => validateInvocationCapture(r)).toThrow(); }
});
test('baseline retains seven process medians, excludes only warmups and cannot invent a speedup', () => {
  const reports = Array.from({ length: 7 }, report), result = summarizeInvocationBaseline(reports);
  expect(result.rows[0]!.processMedians).toEqual(Array(7).fill(32)); expect(result.rows[0]!.meanPerInvocationMs).toBe(1);
  expect(result.primaryAccepted).toBeNull(); expect(result.releaseAccepted).toBe(false); expect(result.cpuMs).toBeNull();
  expect(() => summarizeInvocationBaseline(reports.slice(1))).toThrow();
  reports[0]!.rows[0]!.batches[1]!.wallMs = 999;
  expect(summarizeInvocationBaseline(reports).rows[0]!.processMedians).toHaveLength(7);
});
test('traces preserve process domains and reject invalid clocks or captured sensitive fields', () => {
  const spans = [1, 2].map(durationMs => ({ profile: 'woml.performance-span/v1', process: 'worker', name: 'worker.compile_script', durationMs }));
  expect(summarizeInvocationTrace(spans)[0]!.medianMs).toBe(1.5);
  expect(() => summarizeInvocationTrace([{ ...spans[0], source: 'private' }])).toThrow();
  expect(() => summarizeInvocationTrace([{ ...spans[0], durationMs: Infinity }])).toThrow();
});
test('trace diagnosis flags repeated host spans and worker realm ID collisions without trimming records', () => {
  const spans = [
    { process: 'host', spanId: 'span_pid_1', name: 'host.invoke_worker' },
    { process: 'host', spanId: 'span_pid_1', name: 'host.invoke_worker' },
    { process: 'worker', spanId: 'span_pid_2', name: 'worker.compile_script', invocationId: 'a' },
    { process: 'worker', spanId: 'span_pid_2', name: 'worker.compile_script', invocationId: 'b' },
  ];
  const result = traceQuality(spans);
  expect(result.host.raw).toBe(2); expect(result.host.unique).toBe(1);
  expect(result.hostQuantitativeAttributionUsable).toBe(false);
  expect(result.workerInvocationScopedKeysUnique).toBe(true); expect(result.globalSpanIdsUnique).toBe(false);
  expect(spans).toHaveLength(4);
});
test('capture never implicitly builds or loads a native addon; original worker execution remains unchanged', async () => {
  const harness = await readFile(new URL('../scripts/invocation-setup-child.ts', import.meta.url), 'utf8');
  expect(harness).toContain('new FrameDecoder'); expect(harness).toContain('await child.stdin.flush()');
  expect(harness).not.toContain('WOML_RUST_CORE_PATH'); expect(harness).not.toContain('cargo');
  const worker = await readFile(new URL('../src/script-host/worker.ts', import.meta.url), 'utf8');
  expect(worker).toContain('const sourceMapUrl = Buffer.from(module.sourceMap');
  expect(worker).toContain('namespace = (await import(moduleUrl))');
  const ci = await readFile(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  expect(ci).toContain('bun run test:invocation-setup-contracts');
  expect(ci).not.toContain('bun run capture:invocation-setup-baseline');
});
