import { expect, test } from 'bun:test';
import { startupAcceptance, startupScenarios, startupSummary } from '../scripts/startup-separation-baseline';
import { sessionFixture, sessionParameters, type SessionResult } from '../scripts/runtime-session-profile-child';
import { resolve } from 'node:path';

// Git can check source out as CRLF on Windows. Permit that line ending only;
// the test-only attribute and private module declaration must remain adjacent.
const testOnlyPreparedContract = /#\[cfg\(test\)\]\r?\nmod prepared_contract;/u;

test('prototype declaration check accepts LF and CRLF but rejects changed visibility or gating', () => {
  for (const newline of ['\n', '\r\n']) {
    const declaration = `#[cfg(test)]${newline}mod prepared_contract;`;
    expect(declaration).toMatch(testOnlyPreparedContract);
    expect(declaration.replace('mod prepared_contract;', 'pub mod prepared_contract;')).not.toMatch(testOnlyPreparedContract);
    expect(declaration.replace('#[cfg(test)]', '#[cfg(any())]')).not.toMatch(testOnlyPreparedContract);
    expect(declaration.replace(newline, `${newline}mod unrelated;${newline}`)).not.toMatch(testOnlyPreparedContract);
  }
});

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
  expect(facade).toMatch(testOnlyPreparedContract);
  expect(prototype).not.toContain('pub struct ReadinessPrototype');
  expect([...prototype.matchAll(/#\[test\]/g)].length).toBeGreaterThanOrEqual(11);
  expect(prototype).toContain('SQLITE_FCNTL_HAS_MOVED');
  expect(prototype).toContain('SQLITE_FCNTL_WIN32_GET_HANDLE');
  const combined = ci.slice(ci.indexOf('\n  combined-developer-experience:'), ci.indexOf('\n  operational-output:'));
  expect(combined).toContain('os: [ubuntu-24.04, macos-15, windows-2022]');
  expect(combined).toContain('bun run test:startup-request-separation');
});

test('production readiness remains private, non-creating, and transaction guarded', async () => {
  const root = resolve(import.meta.dir, '../..');
  const source = await Bun.file(resolve(root, 'core/woml-engine/src/durable/prepared.rs')).text();
  const options = await Bun.file(resolve(root, 'core/woml-engine/src/runtime/options.rs')).text();
  const composition = await Bun.file(resolve(root, 'core/woml-engine/src/runtime/composition.rs')).text();
  const scripts = await Bun.file(resolve(root, 'woml-cli/package.json')).json();
  expect(source).toContain('SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX');
  expect(source).not.toContain('SQLITE_OPEN_CREATE');
  expect(source).toContain('context.check_transaction(&self.tx');
  expect(source).toContain('full startup receipt missing');
  expect(options).toContain('pub(crate) prepared_runtime:');
  expect(composition.match(/self\.prepared_runtime = None;/gu)).toHaveLength(2);
  expect(scripts.scripts['test:startup-request-separation']).toContain('--lib durable::prepared ');
  expect(scripts.scripts['test:startup-request-separation']).toContain('test:startup-request-separation-native');
});
