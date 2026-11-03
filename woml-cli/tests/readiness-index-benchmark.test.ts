import { expect, test } from 'bun:test';
import { validateIndexCapture } from '../scripts/readiness-index-benchmark';

function fixture() {
  const time = () => ({ wallMs: 1, threadCpuMs: 0.8 as number | null });
  return { profile: 'woml.readiness-indexes/v1', allocationMeasurement: null, peakLiveBytes: null,
    rows: [1, 4, 16, 64, 256, 1024].flatMap(nodes => Array.from({ length: 4 }, (_, sample) => ({
      nodes, events: nodes * 2, sample, warmup: sample === 0, fixtureSha256: String(nodes).padStart(64, '0'),
      scanValidation: time(), indexedValidation: time(), scanQuery: time(), indexedQueryIncludingPreparation: time(), graphPreparation: time(),
    }))),
  };
}

test('paired capture keeps small graphs, warmup and all preparation costs explicit', () => {
  expect(() => validateIndexCapture(fixture())).not.toThrow();
});
test('paired capture rejects omitted samples, warmup relabeling and fixture drift', () => {
  const missing = fixture(); missing.rows.pop(); expect(() => validateIndexCapture(missing)).toThrow();
  const warmup = fixture(); warmup.rows[1]!.warmup = true; expect(() => validateIndexCapture(warmup)).toThrow();
  const drift = fixture(); drift.rows[1]!.fixtureSha256 = 'f'.repeat(64); expect(() => validateIndexCapture(drift)).toThrow();
});
test('paired capture does not turn unavailable CPU into zero or allow nonfinite samples', () => {
  const value = fixture(); value.rows[0]!.scanValidation.threadCpuMs = null;
  expect(() => validateIndexCapture(value)).not.toThrow();
  value.rows[0]!.indexedQueryIncludingPreparation.wallMs = Infinity;
  expect(() => validateIndexCapture(value)).toThrow();
});
