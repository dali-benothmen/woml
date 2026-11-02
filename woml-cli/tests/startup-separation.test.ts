import { expect, test } from 'bun:test';
import { startupAcceptance, startupScenarios, startupSummary } from '../scripts/startup-separation-baseline';
import { sessionFixture, sessionParameters, type SessionResult } from '../scripts/runtime-session-profile-child';
import { resolve } from 'node:path';

test('startup baseline freezes independent process cohorts and diagnostic dimensions', () => {
  const scenarios = startupScenarios();
  expect(scenarios).toHaveLength(13);
  expect(new Set(scenarios.map(item => item.id)).size).toBe(13);
  expect(startupAcceptance.batches).toBe(3);
  expect(startupAcceptance.runsPerProcess).toBe(3);
  expect(scenarios.filter(item => item.retained > 0).map(item => item.retained)).toEqual([20, 100]);
  expect(scenarios.find(item => item.id === 'idle-heartbeat')!.idleMs).toBe(4500);
  for (const scenario of scenarios) expect(() => sessionParameters(scenario.mode, scenario.retained, scenario.concurrency, scenario.idleMs)).not.toThrow();
});
test('probe parameters reject unbounded or incorrectly attributed experiments', () => {
  for (const retained of [-1, 101, NaN, 1.2]) expect(() => sessionParameters('activated', retained)).toThrow();
  expect(() => sessionParameters('fresh-store', 20)).toThrow();
  expect(() => sessionParameters('reused-store', 0, 2)).toThrow();
  expect(() => sessionParameters('fresh-store', 0, 1, 1)).toThrow();
  expect(() => sessionParameters('activated', 0, 3)).toThrow();
  expect(() => sessionParameters('activated', 0, 1, 10001)).toThrow();
  expect(sessionFixture('sequential-2', 2).source.replace('concurrency="2"', 'concurrency="1"')).toBe(sessionFixture('sequential-2').source);
});
test('startup summary separates first request and uses process medians, not pooled runs', () => {
  const scenario = startupScenarios()[0]!;
  const result = (times: number[]): SessionResult => ({ profile: 'woml.runtime-session-sample/v1', case: scenario.case, mode: scenario.mode,
    definitionHash: 'hash', modelVersion: 12, compileMs: 0, samples: times.map((durationMs, index) => ({ durationMs, index,
      runId: `run-${index}`, statePath: 'state', ownProcessCpuMs: 0 })) });
  const summary = startupSummary([{ scenario, batch: 0, result: result([100, 10, 20]) }, { scenario, batch: 1, result: result([200, 30, 50]) }]);
  expect(summary[0]!.first.medianMs).toBe(150);
  expect(summary[0]!.laterProcessMedians.medianMs).toBe(27.5);
  expect(summary[0]!.processes).toBe(2);
});
test('readiness prototype stays test-only and has serial cross-platform CI coverage', async () => {
  const root = resolve(import.meta.dir, '../..');
  const facade = await Bun.file(resolve(root, 'core/woml-engine/src/durable.rs')).text();
  const prototype = await Bun.file(resolve(root, 'core/woml-engine/src/durable/prepared_contract.rs')).text();
  const ci = await Bun.file(resolve(root, '.github/workflows/ci.yml')).text();
  expect(facade).toContain('#[cfg(test)]\nmod prepared_contract;');
  expect(prototype).not.toContain('pub struct ReadinessPrototype');
  expect([...prototype.matchAll(/#\[test\]/g)].length).toBeGreaterThanOrEqual(11);
  expect(prototype).toContain('SQLITE_FCNTL_HAS_MOVED');
  expect(prototype).toContain('SQLITE_FCNTL_WIN32_GET_HANDLE');
  const combined = ci.slice(ci.indexOf('\n  combined-developer-experience:'), ci.indexOf('\n  operational-output:'));
  expect(combined).toContain('os: [ubuntu-24.04, macos-15, windows-2022]');
  expect(combined).toContain('bun run test:startup-request-separation');
});
