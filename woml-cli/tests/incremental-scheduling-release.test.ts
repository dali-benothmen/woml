import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import contract from '../../docs/incremental-scheduling-acceptance.v1.json';
import { assertOriginalIsqIdentity, captureBody, releaseMicrobenchmarkEvidence } from '../scripts/incremental-scheduling-release';

const receipt = () => ({ profile: 'woml.incremental-scheduling-build/v1', label: 'baseline',
  measurementHarnessSha256: '7caabde8507ed203053e4aaa4ec88d74a70d2294ab4008b3f69f1869cc0b5bcb',
  engine: { profile: 'woml.validated-state-build/v1', buildMode: 'release-unit-test',
    source: { commit: contract.baselineCommit, sha256: '3cb433f0f405625da1125664752b46ea075ccdba7bf7cbbd308c300d75a5a0f7' },
    binarySha256: 'dcd04b1db65e2153ae441531273f09aad627d67eaed4d334a43d80dca02b9b15' } });

function captures(cpu: number | null = 1) {
  const time = () => ({ wallMs: 1, threadCpuMs: cpu });
  return Array.from({ length: 7 }, () => ({ profile: 'woml.incremental-scheduling-capture/v1', allocationMeasurement: null, peakLiveBytes: null,
    rows: contract.sizes.flatMap(nodes => Array.from({ length: 4 }, (_, sample) => ({
      nodes, events: nodes * 2, sample, warmup: sample === 0, fixtureSha256: String(nodes).padStart(64, '0'),
      historyValidation: time(), membership: time(), fullQueryIncludingPreparation: time(), topologyPreparation: time(),
    }))),
  }));
}

test('historical continuation pins the actual originally qualified source, artifact and adapter', () => {
  expect(() => assertOriginalIsqIdentity(receipt())).not.toThrow();
  for (const mutate of [
    (r: ReturnType<typeof receipt>) => { r.label = 'candidate'; },
    (r: ReturnType<typeof receipt>) => { r.engine.source.commit = 'a'.repeat(40); },
    (r: ReturnType<typeof receipt>) => { r.engine.source.sha256 = 'b'.repeat(64); },
    (r: ReturnType<typeof receipt>) => { r.engine.binarySha256 = 'c'.repeat(64); },
    (r: ReturnType<typeof receipt>) => { r.measurementHarnessSha256 = 'd'.repeat(64); },
    (r: ReturnType<typeof receipt>) => { r.engine.buildMode = 'debug'; },
  ]) {
    const value = receipt(); mutate(value); expect(() => assertOriginalIsqIdentity(value)).toThrow();
  }
});

test('frozen probe extraction rejects missing/duplicate probes and preserves workload edits', () => {
  const source = 'mod tests;\nfn capture() {\n  actual_work();\n}\n';
  expect(captureBody(source)).toBe('\nfn capture() {\n  actual_work();\n}\n');
  expect(() => captureBody('fn other() {}')).toThrow();
  expect(() => captureBody(source + '\nfn capture() {}')).toThrow();
  expect(captureBody(source.replace('actual_work', 'skip_work'))).not.toBe(captureBody(source));
});

test('numeric wins never certify release, allocation, platforms or installed execution', () => {
  const result = releaseMicrobenchmarkEvidence(captures(), captures(0.5), null);
  expect(result.primaryAccepted).toBe(true);
  expect(result.releaseAccepted).toBe(false);
  expect(result.allocationMeasurement).toBeNull(); expect(result.peakLiveBytes).toBeNull();
  expect(result.wholeProcessMaximumRss).toBeNull(); expect(result.rssInvestigationRequired).toBeNull();
  expect(result.releasePrerequisitesNotCertifiedByThisCapture).toEqual(contract.releasePrerequisites);
  expect(releaseMicrobenchmarkEvidence(captures(), captures(null), null).primaryAccepted).toBe(false);
});

test('release evidence preserves slow controls, every process and RSS units/unknowns', () => {
  const before = captures(), after = captures();
  for (const report of after) for (const row of report.rows) row.topologyPreparation.wallMs = 2;
  const rss = [Array<number>(7).fill(1024), Array<number>(7).fill(1024 + 2 ** 25)];
  const result = releaseMicrobenchmarkEvidence(before, after, rss);
  expect(result.investigations).toHaveLength(6);
  expect(result.rssInvestigationRequired).toBe(true);
  expect(result.rssIncreaseBytes).toBe(2 ** 25);
  expect(result.wholeProcessMaximumRss![1]!.processBytes).toHaveLength(7);
  expect(() => releaseMicrobenchmarkEvidence(before.slice(1), after, rss)).toThrow();
  expect(() => releaseMicrobenchmarkEvidence(before, after, [[1], [1]])).toThrow();
  expect(() => releaseMicrobenchmarkEvidence(before, after, [Array(7).fill(NaN), rss[1]!])).toThrow();
});

test('release capture remains explicit and the original frozen runner is not relaxed', async () => {
  const frozen = await readFile(new URL('../scripts/incremental-scheduling-measurement.ts', import.meta.url), 'utf8');
  expect(frozen).toContain('assert.equal(receipt.engine.source.commit, contract.baselineCommit)');
  const harness = await readFile(new URL('../scripts/incremental-scheduling-release.ts', import.meta.url), 'utf8');
  expect(harness).toContain('await snapshot(), candidate.source');
  expect(harness).toContain('captureBody(await readFile');
  expect(harness).toContain('receipt.binarySha256');
  expect(harness).not.toContain("['cargo', 'test'");
  expect(harness).not.toContain('WOML_RUST_CORE_PATH:');
});
