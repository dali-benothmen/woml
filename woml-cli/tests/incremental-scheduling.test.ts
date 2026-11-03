import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import contract from '../../docs/incremental-scheduling-acceptance.v1.json';
import { summarizeIsq, validateIsqCapture, withoutIsqTestInstrumentation } from '../scripts/incremental-scheduling-measurement';

function capture() {
  const time = () => ({ wallMs: 1, threadCpuMs: 1 as number | null });
  return { profile: 'woml.incremental-scheduling-capture/v1', allocationMeasurement: null, peakLiveBytes: null,
    rows: contract.sizes.flatMap(nodes => Array.from({ length: contract.warmupSamples + contract.measuredSamples }, (_, sample) => ({
      nodes, events: nodes * 2, sample, warmup: sample < contract.warmupSamples, fixtureSha256: String(nodes).padStart(64, '0'),
      historyValidation: time(), membership: time(), fullQueryIncludingPreparation: time(), topologyPreparation: time(),
    }))),
  };
}

test('ISQ freezes an indexed baseline, separate phase target, controls and release prerequisites', () => {
  expect(contract.baselineCommit).toBe('70a5d4cc484b6dca4e03aaf3f8aea1bd7127784b');
  expect(contract.sizes).toEqual([1, 4, 16, 64, 256, 1024]);
  expect(contract.processesPerArtifact).toBe(7);
  expect(contract.primary).toEqual({ operation: 'historyValidation', nodes: 1024, metric: 'threadCpuMs', minimumImprovementPercent: 10 });
  expect(contract.investigation).toEqual({ controlMedianRegressionPercent: 5, allocatedBytesIncreasePercent: 5, rssOrScopedPeakLiveIncreaseBytes: 16777216 });
  expect(contract.isq1Acceptance.retainedDynamicStateBytes).toBe(0);
  expect(contract.releasePrerequisites).toContain('prepared-runtime-locking-correction');
});

test('baseline qualification removes only explicit test probes, never production statements', () => {
  const original = 'fn query() {\n  calculate();\n}\n';
  const probes = '#[cfg(test)]\nmod incremental;\nfn query() {\n  #[cfg(test)]\n  incremental::note(|work| work.candidate_checks += 1);\n  calculate();\n}\n';
  expect(withoutIsqTestInstrumentation(probes)).toBe(original);
  expect(withoutIsqTestInstrumentation(probes.replace('calculate();', 'skip_validation();'))).not.toBe(original);
});

test('ISQ capture retains complete ordered samples and unavailable measurements', () => {
  const value = capture();
  value.rows[0]!.membership.threadCpuMs = null;
  expect(() => validateIsqCapture(value)).not.toThrow();
  expect(() => validateIsqCapture({ ...capture(), allocationMeasurement: 0 })).toThrow();
  expect(() => validateIsqCapture({ ...capture(), peakLiveBytes: 0 })).toThrow();
});

test('ISQ capture rejects missing, reordered and mislabeled samples and fixture substitution', () => {
  const missing = capture(); missing.rows.pop(); expect(() => validateIsqCapture(missing)).toThrow();
  const reversed = capture(); reversed.rows.reverse(); expect(() => validateIsqCapture(reversed)).toThrow();
  const warmup = capture(); warmup.rows[1]!.warmup = true; expect(() => validateIsqCapture(warmup)).toThrow();
  const drift = capture(); drift.rows[1]!.fixtureSha256 = 'f'.repeat(64); expect(() => validateIsqCapture(drift)).toThrow();
  for (const invalid of [NaN, Infinity, -1]) {
    const value = capture(); value.rows[0]!.membership.wallMs = invalid;
    expect(() => validateIsqCapture(value)).toThrow();
  }
});

test('ISQ comparison uses medians of process medians and never claims release acceptance', () => {
  const baseline = Array.from({ length: contract.processesPerArtifact }, capture);
  const candidate = Array.from({ length: contract.processesPerArtifact }, capture);
  for (const report of candidate) for (const row of report.rows) {
    row.historyValidation.threadCpuMs = row.warmup ? 100 : 0.8;
  }
  const result = summarizeIsq(baseline, candidate);
  expect(result.primaryAccepted).toBe(true);
  expect(result.primary.candidate).toBe(0.8);
  expect(result.releaseAccepted).toBe(false);
  expect(result.allocationMeasurement).toBeNull();
  expect(result.peakLiveBytes).toBeNull();
  expect(result.investigations).toEqual([]);
  candidate[0]!.rows[1]!.fixtureSha256 = 'e'.repeat(64);
  expect(() => summarizeIsq(baseline, candidate)).toThrow();
});

test('ISQ comparison cannot pass unavailable primary CPU or discard a slow control', () => {
  const baseline = Array.from({ length: contract.processesPerArtifact }, capture);
  const candidate = Array.from({ length: contract.processesPerArtifact }, capture);
  for (const report of candidate) for (const row of report.rows) {
    row.historyValidation.threadCpuMs = null;
    row.fullQueryIncludingPreparation.wallMs = 2;
  }
  const result = summarizeIsq(baseline, candidate);
  expect(result.primaryAccepted).toBe(false);
  expect(result.primary.improvementPercent).toBeNull();
  expect(result.investigations).toHaveLength(contract.sizes.length);
  expect(() => summarizeIsq(baseline.slice(1), candidate)).toThrow();
});

test('ISQ safety selection is explicit and timing capture is never an implicit CI build', async () => {
  const packageJson = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  expect(packageJson.scripts['test:incremental-scheduling']).toContain('--test-threads=1');
  expect(packageJson.scripts['test:incremental-scheduling']).not.toContain('--release');
  const ci = await readFile(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  expect(ci).toContain('bun run test:incremental-scheduling');
  expect(ci).not.toContain('capture:incremental-scheduling');
  expect(ci).not.toContain('investigate-incremental-scheduling-controls');
  const diagnosis = await readFile(new URL('../scripts/investigate-incremental-scheduling-controls.ts', import.meta.url), 'utf8');
  expect(diagnosis).toContain('before-history');
  expect(diagnosis).toContain('after-history');
  expect(diagnosis).toContain('releaseAccepted: false');
});
