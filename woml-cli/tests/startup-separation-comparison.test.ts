import { expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { resolve } from 'node:path';
import { compareObservations, comparisonSchedule, validateObservation, verifyProductionDiff, type Observation } from '../scripts/startup-separation-comparison';

// Match the complete command, including its line boundary, on either checkout
// style. Do not confuse the package-only command with the native safety gate.
const startupReleaseCommand = /bun run test:startup-request-separation\r?\n/u;

test('release command check accepts LF and CRLF without accepting a different gate', () => {
  for (const newline of ['\n', '\r\n']) {
    const command = `          bun run test:startup-request-separation${newline}`;
    expect(command).toMatch(startupReleaseCommand);
    expect(command.replace('separation', 'separation-package')).not.toMatch(startupReleaseCommand);
    expect(command.replace('separation', 'separation-disabled')).not.toMatch(startupReleaseCommand);
    expect(command.replace(newline, ` --skip-native${newline}`)).not.toMatch(startupReleaseCommand);
  }
});

function cohort(scale = 0.8): Observation[] {
  return comparisonSchedule().map(row => ({ ...row, result: {
    profile: 'woml.runtime-session-sample/v1', case: row.scenario.case, mode: row.scenario.mode,
    definitionHash: 'same-definition', modelVersion: 12, compileMs: 1,
    parameters: { retained: row.scenario.retained, concurrency: row.scenario.concurrency, idleMs: row.scenario.idleMs },
    ...(row.scenario.mode === 'activated' ? { prepareMs: 100, activateMs: 10, shutdownMs: 20 } : {}),
    samples: [0, 1, 2].map(index => ({ index, runId: `run-${index}`, statePath: 'unused', eventCount: 8,
      durationMs: index > 0 && row.variant === 'candidate' && row.scenario.mode === 'activated' ? 100 * scale : 100,
      ownProcessCpuMs: 10, ownProcessRssBytes: 100 * 1024 ** 2 })),
  } }));
}
test('matched schedule freezes every independent cohort and balances adjacent library order', () => {
  const schedule = comparisonSchedule(); expect(schedule).toHaveLength(78);
  const firsts = schedule.filter((_, index) => index % 2 === 0);
  expect(Math.abs(firsts.filter(row => row.variant === 'baseline').length - firsts.filter(row => row.variant === 'candidate').length)).toBe(1);
  for (let index = 0; index < schedule.length; index += 2) {
    expect(schedule[index]!.scenario).toEqual(schedule[index + 1]!.scenario);
    expect(schedule[index]!.variant).not.toBe(schedule[index + 1]!.variant);
  }
});
test('missing processes and duplicate batches cannot produce a passing result', () => {
  expect(() => compareObservations(cohort().slice(1))).toThrow('Incomplete cohort');
  const duplicate = cohort(); duplicate[0] = duplicate[1]!;
  expect(() => compareObservations(duplicate)).toThrow('Duplicate batch');
});
test('reviewed target is separate from first requests, CPU and memory', () => {
  const result = compareObservations(cohort());
  expect(result.targetStatus).toBe('met'); expect(result.improvementPercent).toBeCloseTo(20);
  expect(result.performanceGate).toBe('met'); expect(result.investigations).toEqual([]);
  const target = result.rows.find(row => row.scenario === 'retained-100')!;
  expect(target.metrics.find(m => m.name === 'firstMs')!.candidate.medianMs).toBe(100);
  expect(target.metrics.find(m => m.name === 'rssBytes')!.unit).toBe('bytes');
});
test('overlapping primary ranges are inconclusive even with a favorable median', () => {
  const rows = cohort();
  rows.find(row => row.variant === 'candidate' && row.batch === 0 && row.scenario.id === 'retained-100')!.result.samples.slice(1).forEach(s => { s.durationMs = 110; });
  const result = compareObservations(rows);
  expect(result.improvementPercent).toBeCloseTo(20);
  expect(result.targetStatus).toBe('inconclusive'); expect(result.performanceGate).toBe('requires-review');
});
test('a real target miss cannot pass, and outliers stay in process medians', () => {
  const result = compareObservations(cohort(1.1));
  expect(result.targetStatus).toBe('not-met'); expect(result.performanceGate).toBe('requires-review');
  const rows = cohort();
  rows.find(row => row.variant === 'candidate' && row.batch === 1 && row.scenario.id === 'retained-100')!.result.samples[2]!.durationMs = 1000;
  const target = compareObservations(rows).rows.find(row => row.scenario === 'retained-100')!;
  expect(target.metrics.find(m => m.name === 'laterMs')!.candidate.maximumMs).toBe(540);
});
test('slower controls and own-process memory growth require investigation, not silent success', () => {
  const rows = cohort();
  for (const row of rows.filter(row => row.variant === 'candidate' && row.scenario.id === 'sequential-2-fresh-store')) {
    row.result.samples.forEach(sample => { sample.durationMs = 106; sample.ownProcessRssBytes! += 17 * 1024 ** 2; });
  }
  const result = compareObservations(rows);
  expect(result.targetStatus).toBe('met'); expect(result.performanceGate).toBe('requires-review');
  expect(result.investigations.map(issue => issue.metric)).toEqual(['firstMs', 'laterMs', 'rssBytes']);
});
test('invalid measurements and mismatched definitions/facts fail closed', () => {
  const rows = cohort(); rows[0]!.result.samples[0]!.durationMs = NaN;
  expect(() => validateObservation(rows[0]!)).toThrow();
  const changed = cohort(); changed[0]!.result.definitionHash = 'substituted';
  expect(() => compareObservations(changed)).toThrow('Definition identity drift');
  const drift = cohort(); drift[0]!.result.samples[0]!.eventCount = 9;
  expect(() => compareObservations(drift)).toThrow('Durable fact count drift');
});

test('build provenance rejects patch drift and preserves exact bytes for new worktree receipts', () => {
  const patch = 'diff --git a/file b/file\n+source\n';
  const receipt = { productionDiffNormalization: 'raw', productionDiffSha256: createHash('sha256').update(patch).digest('hex') };
  expect(() => verifyProductionDiff(patch, receipt)).not.toThrow();
  expect(() => verifyProductionDiff(patch.trimEnd(), receipt)).toThrow('Candidate production patch drift');
  expect(() => verifyProductionDiff(patch.replace('source', 'substitution'), receipt)).toThrow();
  expect(() => verifyProductionDiff('', {})).not.toThrow();
  expect(() => verifyProductionDiff(patch, {})).toThrow();
  expect(() => verifyProductionDiff('', { productionDiffNormalization: 'trust-me' })).toThrow();
});

test('installed request coverage follows fresh platform builds without replacing existing release budgets', async () => {
  const root = resolve(import.meta.dir, '../..');
  const document = Bun.YAML.parse(await Bun.file(resolve(root, '.github/workflows/ci.yml')).text()) as {
    jobs: Record<string, { strategy: { matrix: { os: string[] } }; steps: { name?: string; run?: string }[] }>;
  };
  const job = document.jobs['combined-developer-experience']!;
  expect(job.strategy.matrix.os).toEqual(['ubuntu-24.04', 'macos-15', 'windows-2022']);
  const build = job.steps.findIndex(step => step.name === 'Build the local native and portable release candidates');
  const installed = job.steps.findIndex(step => step.run === 'bun run test:startup-request-separation-package');
  expect(build).toBeGreaterThanOrEqual(0); expect(installed).toBeGreaterThan(build);
  expect(job.steps.some(step => step.run === 'bun run test:combined-developer-experience-package')).toBe(true);
  expect(job.steps.some(step => step.run?.includes('benchmark-combined-developer-experience.ts'))).toBe(true);
  const scripts = (await Bun.file(resolve(root, 'woml-cli/package.json')).json()).scripts;
  expect(scripts['test:startup-request-separation-package']).toBe('bun test tests/startup-separation-packaged-release.test.ts --max-concurrency=1');
  expect(scripts['compare:startup-request-separation']).not.toContain('build');
  const release = await Bun.file(resolve(root, '.github/workflows/release.yml')).text();
  expect(release).toMatch(startupReleaseCommand);
  expect(release).toContain('bun run test:startup-request-separation-package');
});
