import { expect, test } from 'bun:test';
import { validateReadinessCapture } from '../scripts/readiness-investigation';

function fixture() {
  const time = () => ({ wallMs: 1, threadCpuMs: 0.8 });
  return { profile: 'woml.readiness-investigation/v1',
    components: [16, 64, 256, 1024].flatMap(nodes => [0, nodes / 2, nodes - 1].flatMap(completed => Array.from({ length: 5 }, (_, sample) => {
      let predecessorTests = 0;
      for (let index = Math.max(1, completed); index < nodes; index++) predecessorTests += index;
      return { nodes, completed, historyEvents: 1 + 2 * completed, sample, warmup: sample < 2, fixtureSha256: String(nodes).padStart(64, '0'),
        wholeQuery: time(), stages: { activeTraversal: time(), incomingConstruction: time(), nodeFiltering: time() },
        countedInspections: { outgoingEdgeInspections: nodes * (nodes - 1), latestAttemptInspections: (nodes - completed) * completed, predecessorNodeInspections: predecessorTests } };
    }))),
    histories: [16, 64, 256, 1024].flatMap(nodes => ['full-active', 'fixed-17-events'].map(cohort => ({ nodes, cohort,
      events: cohort === 'full-active' ? nodes * 2 : 17, prefixVisits: cohort === 'full-active' ? nodes * 2 : 17,
      queries: cohort === 'full-active' ? nodes : 8, fixtureSha256: String(nodes).padStart(64, '0'),
      wholeValidation: time(), payloadChecks: time(), structuralFold: time(), readinessAndPrefixReplay: time() }))),
  };
}

test('readiness diagnosis retains independent graph-width and history-length axes', () => {
  expect(() => validateReadinessCapture(fixture())).not.toThrow();
});
test('readiness diagnosis rejects missing samples and incorrect deterministic counts', () => {
  const missing = fixture(); missing.components.pop(); expect(() => validateReadinessCapture(missing)).toThrow();
  const counts = fixture(); counts.components[0]!.countedInspections.outgoingEdgeInspections--;
  expect(() => validateReadinessCapture(counts)).toThrow();
});
test('readiness diagnosis rejects fixture drift and incorrect prefix visits', () => {
  const changed = fixture(); changed.histories[0]!.fixtureSha256 = 'f'.repeat(64);
  expect(() => validateReadinessCapture(changed)).toThrow();
  const visits = fixture(); visits.histories[0]!.prefixVisits--;
  expect(() => validateReadinessCapture(visits)).toThrow();
});
test('readiness diagnosis rejects nonfinite clocks without fabricating unavailable CPU', () => {
  const value = fixture(); value.components[0]!.wholeQuery.threadCpuMs = null as any;
  expect(() => validateReadinessCapture(value)).not.toThrow();
  value.components[0]!.wholeQuery.wallMs = NaN;
  expect(() => validateReadinessCapture(value)).toThrow();
});
