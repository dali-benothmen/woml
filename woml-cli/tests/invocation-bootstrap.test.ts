import { expect, test } from 'bun:test';
import contract from '../../docs/invocation-bootstrap-diagnostic.v1.json';
import { bootstrapFixture, bootstrapOrder } from '../scripts/invocation-bootstrap-child';
import { summarizeBootstrap, validateBootstrap } from '../scripts/invocation-bootstrap';
const fixtures = new Map(contract.cases.map(name => [name, bootstrapFixture(name).fixtureSha256]));

function report(processIndex = 0) {
  let id = 0;
  return { profile: 'woml.invocation-bootstrap-capture/v1', processIndex, releaseAccepted: false, cpuMs: null, allocatedBytes: null, peakLiveBytes: null,
    rows: Array.from({ length: contract.warmupSamples + contract.measuredSamples }, (_, sample) => bootstrapOrder(sample, processIndex).map(name => ({
      name, sample, warmup: sample < contract.warmupSamples, batchMs: 10,
      workers: Array.from({ length: name === 'wrapped-concurrent' ? 4 : 1 }, () => {
        const raw = name.startsWith('raw-'), unique = ++id;
        return { name, sample, warmup: sample < contract.warmupSamples, observerId: `${processIndex}-${unique}`, fixtureSha256: fixtures.get(name)!,
          realmId: raw ? null : `00000000-0000-4000-8000-${String(unique).padStart(12, '0')}`, executedProductionCode: name !== 'wrapped-empty',
          constructorMs: 1, enteredMs: raw ? null : 2, readyMs: raw ? null : 3, importMs: raw || name === 'wrapped-empty' ? null : 1,
          executionMs: raw ? null : 1, requestRoundTripMs: 2, journeyMs: 5 };
      }),
    }))).flat() };
}
test('bootstrap diagnostics rotate balanced cases without enabling the production profiler', () => {
  for (let process = 0; process < 7; process++) {
    expect([...bootstrapOrder(0, process)].sort()).toEqual([...contract.cases].sort());
    expect(() => validateBootstrap(report(process))).not.toThrow();
  }
  expect(bootstrapOrder(0, 0)).not.toEqual(bootstrapOrder(1, 0));
});
test('bootstrap validation rejects missing/reordered observations, duplicate realms and impossible parent boundaries', () => {
  for (const mutate of [
    (r: ReturnType<typeof report>) => { r.rows.pop(); },
    (r: ReturnType<typeof report>) => { r.rows.reverse(); },
    (r: ReturnType<typeof report>) => { r.rows[0]!.workers[0]!.observerId = r.rows[1]!.workers[0]!.observerId; },
    (r: ReturnType<typeof report>) => { r.rows.find(row => row.name === 'wrapped-inline')!.workers[0]!.realmId = r.rows.find(row => row.name === 'wrapped-module')!.workers[0]!.realmId; },
    (r: ReturnType<typeof report>) => { r.rows[0]!.workers[0]!.fixtureSha256 = 'b'.repeat(64); },
    (r: ReturnType<typeof report>) => { r.rows.find(row => row.name === 'wrapped-inline')!.workers[0]!.readyMs = 99; },
    (r: ReturnType<typeof report>) => { r.rows[0]!.workers[0]!.journeyMs = NaN; },
  ]) { const r = report(); mutate(r); expect(() => validateBootstrap(r)).toThrow(); }
});
test('raw-worker missing stages remain null and diagnostic summaries cannot claim a speedup', () => {
  const result = summarizeBootstrap(Array.from({ length: 7 }, (_, index) => report(index)));
  expect(result.releaseAccepted).toBe(false);
  expect(result.rows.find(row => row.name === 'raw-inline')!.metrics.find(row => row.metric === 'importMs')!.medianMs).toBeNull();
  expect(result.rows[0]!.metrics[0]!.processMedians).toHaveLength(7);
  expect(() => summarizeBootstrap([report()])).toThrow();
});
