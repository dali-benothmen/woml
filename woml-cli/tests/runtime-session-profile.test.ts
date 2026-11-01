import { expect, test } from 'bun:test';
import { TerminalObserver, sessionFixture, verifySessionHistory, type SessionResult } from '../scripts/runtime-session-profile-child';
import { nativeWindowGroups, sessionSummary, type SessionSpan } from '../scripts/profile-runtime-sessions';
import { runtimeProfileFixtures } from '../scripts/runtime-profile-fixtures';

test('session inputs preserve original bodies with only reviewed activation-compatible IDs', () => {
  expect(sessionFixture('storage-4').expected).toEqual({ value: 4 });
  expect(sessionFixture('sequential-32').expected).toEqual({ value: 32 });
  expect(() => sessionFixture('state-8')).toThrow('repeat-safe');
  expect(() => sessionFixture('combined')).toThrow('repeat-safe');
  for (const id of ['sequential-2', 'sequential-32', 'storage-4']) {
    expect(sessionFixture(id).source.replace(`runtime-profile-${id.replace(/-(\d+)/gu, '-n$1')}`, `runtime-profile-${id}`))
      .toBe(runtimeProfileFixtures().find(fixture => fixture.id === id)!.source);
  }
});

test('terminal observation handles callbacks before and after admission without polling', async () => {
  const observer = new TerminalObserver();
  const terminal = (runId: string, status: 'succeeded' | 'failed' = 'succeeded') => ({
    contract: 'woml.trigger-progress' as const, contractVersion: 1 as const, type: 'run_terminal' as const,
    runId, workflowId: 'test', status, occurredAt: new Date().toISOString(),
  });
  observer.record(terminal('early'));
  expect((await observer.wait('early')).status).toBe('succeeded');
  const pending = observer.wait('later');
  observer.record(terminal('later', 'failed'));
  expect((await pending).status).toBe('failed');
  expect(() => observer.record(terminal('later'))).toThrow('Duplicate');
  await expect(observer.wait('never', 5)).rejects.toThrow('timed out');
});

test('durable verification rejects success-shaped results with hidden failures or unfinished runs', () => {
  const events = [
    { type: 'run_admitted', sequence: 1, data: {} },
    { type: 'run_outcome_decided', sequence: 2, data: { outcome: 'succeeded', result: { value: 4 } } },
    { type: 'run_finalized', sequence: 3, data: {} },
  ];
  expect(() => verifySessionHistory(events, { value: 4 })).not.toThrow();
  expect(() => verifySessionHistory(events, { value: 3 })).toThrow('result');
  expect(() => verifySessionHistory(events.slice(0, 2), { value: 4 })).toThrow();
  expect(() => verifySessionHistory([{ ...events[0]!, type: 'operation_failed' }, ...events.slice(1)], { value: 4 })).toThrow('hidden');
});

test('native window attribution excludes unrelated and cross-origin host spans', () => {
  const span = (name: string, startOffsetMs: number, durationMs: number, process = 'rust', runId?: string): SessionSpan => ({
    profile: 'woml.performance-span/v1', traceId: 'test', spanId: name, layer: 'runtime',
    status: 'succeeded', process, name, startOffsetMs, durationMs, runId,
  } as SessionSpan);
  const sample: SessionResult = { profile: 'woml.runtime-session-sample/v1', case: 'sequential-2', mode: 'activated',
    definitionHash: 'hash', modelVersion: 12, compileMs: 0,
    samples: [{ index: 0, runId: 'run_a', durationMs: 10, ownProcessCpuMs: 1, statePath: 'state' }] };
  const groups = nativeWindowGroups([
    span('runtime.dispatch_run', 10, 20, 'rust', 'run_a'),
    span('sqlite.open_initialize', 12, 2), span('sqlite.open_initialize', 0, 5),
    span('host.execute_invocation', 12, 4, 'host'),
  ], sample)[0]!.groups;
  expect(groups.find(group => group.name === 'sqlite.open_initialize')?.count).toBe(1);
  expect(groups.some(group => group.process === 'host')).toBe(false);
  expect(() => nativeWindowGroups([], sample)).toThrow('window');
});

test('summary treats independent processes as replicates, not each warmed run', () => {
  const result = (times: number[]): SessionResult => ({ profile: 'woml.runtime-session-sample/v1', case: 'sequential-2', mode: 'activated', definitionHash: 'hash', modelVersion: 12, compileMs: 0,
    samples: times.map((durationMs, index) => ({ index, durationMs, runId: `run_${index}`, ownProcessCpuMs: 1, statePath: 'state' })) });
  const rows = [result([50, 1, 3, 5, 7]), result([70, 10, 20, 30, 40])].map(result => ({ build: 'candidate' as const, case: 'sequential-2', mode: 'activated' as const, phase: 'baseline', batch: 0, directory: 'test', processWallMs: 0, result }));
  expect(sessionSummary(rows)).toContain('| 2 | 60.00 [50.00–70.00] | 14.50 [4.00–25.00] |');
});
