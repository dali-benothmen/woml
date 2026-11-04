import { describe, expect, test } from 'bun:test';
import acceptance from '../../docs/validated-state-reuse-acceptance.v1.json';
import { resourceRecord, validateCapture } from '../scripts/validated-state-baseline';

function report() {
  const timing = () => ({ wallMs: 1, threadCpuMs: 0.5 });
  return { profile: 'woml.validated-state-baseline/v1', allocationMeasurement: null, peakLiveBytes: null,
    rows: acceptance.eventLengths.flatMap(events => Array.from({ length: acceptance.warmupConnections + acceptance.measuredConnections }, (_, sample) => ({ events, sample, warmup: sample < acceptance.warmupConnections,
      fixtureSha256: String(events).padStart(64, '0'), miss: timing(), structuralFold: timing(), hits: Array.from({ length: acceptance.hitsPerConnection }, timing) }))) };
}

describe('Validated state reuse baseline contract', () => {
  test('freezes the cohort and target before production changes', () => {
    expect(acceptance.eventLengths).toEqual([128, 512, 2048]);
    expect(acceptance.processesPerArtifact).toBe(7);
    expect(acceptance.primary).toEqual({ operation: 'cache-miss', events: 2048, metric: 'threadCpuMs', minimumImprovementPercent: 10 });
    expect(acceptance.requiredBeforeVsr2).toContain('allocation-and-peak-live-capture');
  });
  test('accepts complete ordered raw samples', () => { expect(() => validateCapture(report())).not.toThrow(); });
  test('allows unavailable CPU but never substitutes nonfinite or negative measurements', () => {
    const value = report(); value.rows[0]!.miss.threadCpuMs = null as any;
    expect(() => validateCapture(value)).not.toThrow();
    for (const invalid of [NaN, Infinity, -1]) {
      value.rows[0]!.miss.wallMs = invalid;
      expect(() => validateCapture(value)).toThrow();
    }
  });
  test('rejects missing/reordered samples or a wrong hit count', () => {
    const missing = report(); missing.rows.pop(); expect(() => validateCapture(missing)).toThrow();
    const reordered = report(); reordered.rows.reverse(); expect(() => validateCapture(reordered)).toThrow();
    const hits = report(); hits.rows[0]!.hits.pop(); expect(() => validateCapture(hits)).toThrow();
  });
  test('rejects changed fixture identity and false allocator zeros', () => {
    const value = report(); value.rows[1]!.fixtureSha256 = 'f'.repeat(64); expect(() => validateCapture(value)).toThrow();
    expect(() => validateCapture({ ...report(), allocationMeasurement: 0 })).toThrow();
    expect(() => validateCapture({ ...report(), peakLiveBytes: 0 })).toThrow();
  });
  test('materializes non-enumerable Bun resource getters before serialization', () => {
    const usage = Object.defineProperties({}, {
      maxRSS: { get: () => 123456 },
      cpuTime: { get: () => ({ user: 10, system: 2, total: 12 }) },
      contextSwitches: { get: () => ({ voluntary: 3, involuntary: 4 }) },
    }) as Bun.ResourceUsage;
    expect(JSON.stringify(usage)).toBe('{}');
    expect(JSON.parse(JSON.stringify(resourceRecord(usage)))).toEqual({ maxRssRaw: 123456, maxRssUnit: 'unverified-runtime-unit', bun: Bun.version,
      cpuTimeMicroseconds: { user: 10, system: 2, total: 12 }, contextSwitches: { voluntary: 3, involuntary: 4 } });
    expect(resourceRecord(undefined)).toBeNull();
  });
  test('retains actual BigInt CPU fields without inventing an RSS byte unit', () => {
    const usage = { maxRSS: 32768, cpuTime: { user: 15n, system: 3n, total: 18n }, contextSwitches: { voluntary: 1, involuntary: 0 } };
    const value = JSON.parse(JSON.stringify(resourceRecord(usage as any)));
    expect(value.cpuTimeMicroseconds).toEqual({ user: '15', system: '3', total: '18' });
    expect(value.maxRssRaw).toBe(32768);
    expect(value.maxRssUnit).toBe('unverified-runtime-unit');
    expect(value.maxRssBytes).toBeUndefined();
  });
});
