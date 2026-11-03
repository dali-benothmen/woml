import { expect, test } from 'bun:test';
import acceptance from '../../docs/validated-state-reuse-acceptance.v1.json';
import { linuxMaximumRssBytes, median, summarizePair } from '../scripts/validated-state-comparison';
import { resolve } from 'node:path';
import { parseMemusage } from '../scripts/validated-state-allocation';

function capture(cpu: number | null, warmup = 999) {
  const timing = (value: number | null) => ({ wallMs: value ?? 1, threadCpuMs: value });
  return { profile: 'woml.validated-state-baseline/v1', allocationMeasurement: null, peakLiveBytes: null,
    rows: acceptance.eventLengths.flatMap(events => Array.from({ length: 7 }, (_, sample) => ({
      events, sample, warmup: sample < 2, fixtureSha256: String(events).padStart(64, '0'),
      miss: timing(sample < 2 ? warmup : cpu), structuralFold: timing(1), hits: Array.from({ length: 5 }, () => timing(1)),
    }))) };
}
const cohort = (cpu: number | null) => Array.from({ length: 7 }, () => capture(cpu));

test('uses process medians, excludes only frozen warmups and keeps outliers', () => {
  const baseline = cohort(100), candidate = cohort(80);
  candidate[6] = capture(200);
  const result = summarizePair(baseline, candidate);
  expect(result.primary.baseline).toBe(100);
  expect(result.primary.candidate).toBe(80);
  expect(result.primary.candidateProcessMedians).toEqual([80, 80, 80, 80, 80, 80, 200]);
  expect(result.primaryAccepted).toBe(true);
  expect(result.releaseAccepted).toBe(false);
  expect(result.allocationMeasurement).toBeNull();
  expect(median([8, 2, 10, 4])).toBe(6);
});
test('does not certify a sub-target gain, unavailable CPU or zero denominator', () => {
  expect(summarizePair(cohort(100), cohort(91)).primaryAccepted).toBe(false);
  expect(summarizePair(cohort(100), cohort(90)).primaryAccepted).toBe(true);
  const unavailable = summarizePair(cohort(null), cohort(80));
  expect(unavailable.primary.improvementPercent).toBeNull();
  expect(unavailable.primaryAccepted).toBe(false);
  expect(summarizePair(cohort(0), cohort(0)).primaryAccepted).toBe(false);
});
test('rejects missing cohorts, fixture changes, reordered samples and nonfinite timing', () => {
  expect(() => summarizePair(cohort(100).slice(1), cohort(80))).toThrow();
  const substitution = cohort(80);
  for (const row of substitution[0]!.rows) if (row.events === 128) row.fixtureSha256 = 'f'.repeat(64);
  expect(() => summarizePair(cohort(100), substitution)).toThrow(/substitution/);
  const reordered = cohort(80); reordered[0]!.rows.reverse();
  expect(() => summarizePair(cohort(100), reordered)).toThrow();
  const invalid = cohort(80); invalid[0]!.rows[0]!.miss.wallMs = NaN;
  expect(() => summarizePair(cohort(100), invalid)).toThrow();
});
test('reports control regressions above the frozen investigation threshold', () => {
  const result = summarizePair(cohort(100), cohort(110));
  expect(result.investigations.some(row => row.operation === 'miss')).toBe(true);
  expect(result.primaryAccepted).toBe(false);
  expect(acceptance.investigation.controlMedianRegressionPercent).toBe(5);
});

test('cross-platform CI keeps actual feature and prepared-ownership checks serial', async () => {
  const root = resolve(import.meta.dir, '../..');
  const ci = await Bun.file(resolve(root, '.github/workflows/ci.yml')).text();
  const combined = ci.slice(ci.indexOf('\n  combined-developer-experience:'), ci.indexOf('\n  operational-output:'));
  expect(combined).toContain('os: [ubuntu-24.04, macos-15, windows-2022]');
  expect(combined).toContain('bun run test:validated-state-reuse-safety');
  expect(combined).toContain('bun run test:startup-request-separation-package');
  const commands = (await Bun.file(resolve(root, 'woml-cli/package.json')).json()).scripts;
  const safety = commands['test:validated-state-reuse-safety'];
  expect(safety).toContain('--lib durable::prepared -- --test-threads=1');
  for (const family of ['runtime_policy_store', 'durable_wait_runtime', 'workflow_timeouts', 'runtime_ownership', 'locks_and_leases', 'compensation_runtime', 'backup_restore', 'retention']) {
    expect(safety).toContain(`--test ${family}`);
  }
  expect(safety.endsWith('-- --test-threads=1')).toBe(true);
});

test('verified Linux GNU time KiB is distinct from Bun raw RSS and missing allocation data', () => {
  expect(linuxMaximumRssBytes('16384\n')).toBe(16_777_216);
  for (const invalid of ['0', '-1', 'NaN', '0.5', '123 extra', '9007199254740992']) {
    expect(() => linuxMaximumRssBytes(invalid)).toThrow();
  }
});

test('requires exactly one complete nonzero profiler summary and preserves its metric labels', () => {
  const summary = 'Memory usage summary: heap total: 45200, heap peak: 6440, stack peak: 224';
  expect(parseMemusage(`\x1b[31m${summary}\x1b[0m`)).toEqual({
    interceptedHeapTotalBytes: 45200, reportedHeapPeakBytes: 6440, reportedStackPeakBytes: 224,
  });
  for (const invalid of ['', `${summary}\n${summary}`, summary.replace('45200', '0'), summary.replace('45200', '9007199254740992')]) {
    expect(() => parseMemusage(invalid)).toThrow();
  }
});
