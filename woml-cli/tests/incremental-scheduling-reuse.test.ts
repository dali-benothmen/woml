import { expect, test } from 'bun:test';
import { summarizeReuseProbe, validateReuseProbe } from '../scripts/incremental-scheduling-reuse';

function capture(resources = false) {
  const time = () => ({ wallMs: 1, threadCpuMs: 1 as number | null });
  const usage = () => ({ allocated: 100, freed: 100, live: 0, peak: 50 });
  return { profile: 'woml.isq-reuse-probe/v1', resources, releaseAccepted: false,
    rows: [1, 4, 16, 64, 256, 1024].flatMap(nodes => Array.from({ length: 4 }, (_, sample) => ({
      nodes, events: nodes * 2, sample, warmup: sample === 0, fixtureSha256: String(nodes).padStart(64, '0'), engineStackBytes: 64,
      startup: resources ? null : time(), firstQuery: resources ? null : time(), warmQueryBatch20: resources ? null : time(), definitionCloneBatch20: resources ? null : time(),
      history: resources ? { allocation: usage() } : time(), firstQueryAllocation: resources ? usage() : null,
    }))),
  };
}

test('reuse probe keeps uninstrumented timing separate from scoped requested heap bytes', () => {
  expect(() => validateReuseProbe(capture(), false)).not.toThrow();
  expect(() => validateReuseProbe(capture(true), true)).not.toThrow();
  expect(() => validateReuseProbe(capture(true), false)).toThrow();
  const invalid = capture(true); invalid.rows[0]!.firstQueryAllocation!.live = 1;
  expect(() => validateReuseProbe(invalid, true)).toThrow();
});
test('reuse probe rejects omissions, clock substitutions and fixture drift', () => {
  const missing = capture(); missing.rows.pop(); expect(() => validateReuseProbe(missing, false)).toThrow();
  const clock = capture(); clock.rows[0]!.history = { wallMs: Infinity, threadCpuMs: 1 };
  expect(() => validateReuseProbe(clock, false)).toThrow();
  const drift = capture(); drift.rows[1]!.fixtureSha256 = 'a'.repeat(64);
  expect(() => validateReuseProbe(drift, false)).toThrow();
});
test('reuse comparison retains all process medians and does not certify release', () => {
  const before = Array.from({ length: 7 }, () => capture()), after = Array.from({ length: 7 }, () => capture());
  const rows = summarizeReuseProbe(before, after);
  expect(rows).toHaveLength(60);
  expect(rows.find(row => row.nodes === 1024 && row.operation === 'warmQueryBatch20')!.candidate).toBe(0.05);
  expect(() => summarizeReuseProbe(before.slice(1), after)).toThrow();
  after[0]!.rows[1]!.history = { wallMs: 1, threadCpuMs: null };
  expect(summarizeReuseProbe(before, after).find(row => row.nodes === 1 && row.operation === 'history' && row.metric === 'threadCpuMs')!.candidate).toBeNull();
});
