import { expect, test } from 'bun:test';
import acceptance from '../../docs/invocation-setup-acceptance.v1.json';
import { cohorts, fixture } from '../scripts/invocation-setup-fixtures';
import { breakEven, distribution, reviewInvocationEvidence } from '../scripts/invocation-setup-review';

function report(candidate = false) {
  return { profile: 'woml.invocation-setup-capture/v1', mode: 'timing', releaseAccepted: false,
    rows: cohorts.map(cohort => {
      const spec = fixture(cohort), requestRows = [{ batch: 'first', index: 0, wallMs: 1, hostDurationMs: 1 }];
      const batches = Array.from({ length: acceptance.warmupBatches + acceptance.measuredBatches }, (_, sample) => {
        for (let index = 0; index < spec.count; index++) requestRows.push({ batch: `sample${sample}`, index, wallMs: 1, hostDurationMs: 1 });
        return { sample, warmup: sample === 0, invocations: spec.count, wallMs: candidate ? 110 : 100 };
      });
      return { cohort, fixtureSha256: spec.fixtureSha256, concurrency: spec.concurrency, readyMs: 10, registrationMs: 2,
        firstInvocationMs: 1, teardownMs: 1, lifetimeMs: candidate ? 460 : 420, batches, requestRows, completed: requestRows.length,
        allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null };
    }) };
}
test('signed paired distributions preserve slow/fast observations without trimming', () => {
  expect(distribution([-10, 0, 90]).median).toBe(0);
  expect(distribution([9, 1, 3, 5])).toEqual({ count: 4, minimum: 1, median: 4, maximum: 9, values: [9, 1, 3, 5] });
  expect(() => distribution([NaN])).toThrow(); expect(() => distribution([])).toThrow();
});
test('a slower primary has no finite break-even and positive models are not observed crossings', () => {
  expect(breakEven(10, -1)).toEqual({ observedFiniteBreakEven: false, hypotheticalBatches: null });
  expect(breakEven(10, 0).hypotheticalBatches).toBeNull();
  expect(breakEven(10, 3)).toMatchObject({ observedFiniteBreakEven: null, hypotheticalBatches: 4 });
});
test('ISR5 retains cold/lifetime, transfer accounting and every triggered control without certifying resources', () => {
  const review = reviewInvocationEvidence(Array.from({ length: 7 }, () => report()), Array.from({ length: 7 }, () => report(true)));
  expect(review.comparison.numericalPrimaryTargetMet).toBe(false);
  expect(review.comparison.controlInvestigations).toEqual(acceptance.controls);
  expect(review.primaryBreakEven.observedFiniteBreakEven).toBe(false);
  expect(review.rows[0]!.candidateSlowerPairs).toBe(7);
  expect(review.rows[0]!.metrics.find(metric => metric.name === 'lifetimeMs')!.reference.median).toBe(420);
  expect(review.rows[0]!.volumesPerHostBothVariants.requests).toBe(129);
  expect(review.rows.find(row => row.cohort === 'inline')!.volumesPerHostBothVariants.workerDeliveredBundleUtf8Bytes).toBe(0);
  expect(review.rows.find(row => row.cohort === 'module')!.volumesPerHostBothVariants.workerDeliveredBundleUtf8Bytes).toBeGreaterThan(0);
  expect(review.rssTriggerEvaluable).toBe(false); expect(review.unmeasured.actualCloneBytes).toBeNull();
  expect(review.unmeasured.repeatedRunPlateau).toBeNull(); expect(review.releaseAccepted).toBe(false); expect(review.integrationAccepted).toBe(false);
});
test('missing processes and impossible lifetime accounting cannot become an ISR5 report', () => {
  const all = Array.from({ length: 7 }, () => report());
  expect(() => reviewInvocationEvidence(all.slice(1), all)).toThrow();
  all[0]!.rows[0]!.lifetimeMs = 1;
  expect(() => reviewInvocationEvidence(all, all)).toThrow('lifetime');
});
