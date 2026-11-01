import { describe, expect, test } from 'bun:test';
import { compileWoml, parseWoml } from '@woml/compiler';
import { runtimeProfileFixtures, runtimeProfilePolicy } from '../scripts/runtime-profile-fixtures';
import { groupSpans, selectFixtures, nativeBuildOrder, type Span } from '../scripts/profile-runtime-investigation';
import { prepareInfrastructurePolicy } from '../src/infrastructure-policy';

describe('bounded runtime profiling investigation', () => {
  test('labels release comparisons honestly and alternates order', () => {
    expect(nativeBuildOrder(true, 0, 0)).toEqual(['baseline', 'candidate']);
    expect(nativeBuildOrder(true, 1, 0)).toEqual(['candidate', 'baseline']);
    expect(nativeBuildOrder(true, 0, 1)).toEqual(['candidate', 'baseline']);
    expect(nativeBuildOrder(false, 0, 0)).toEqual(['debug', 'release']);
  });
  test('compiles every deterministic fixture without running native builds', () => {
    const fixtures = runtimeProfileFixtures();
    expect(fixtures.length).toBe(10);
    expect(new Set(fixtures.map(item => item.id)).size).toBe(fixtures.length);
    expect(runtimeProfileFixtures()).toEqual(fixtures);
    for (const fixture of fixtures) {
      const workflow = compileWoml(parseWoml(fixture.source, { file: `${fixture.id}.woml` }));
      expect(workflow.graph.nodes.length).toBeGreaterThan(0);
      expect(fixture.source).not.toMatch(/services\.(?:http|process|workflow)|fetch\(|<webhook|<schedule/u);
    }
  });

  test('requires explicit known unique cases', () => {
    expect(selectFixtures('all').length).toBe(10);
    expect(selectFixtures('combined,sequential-2').map(item => item.id)).toEqual(['combined', 'sequential-2']);
    expect(() => selectFixtures('combined,combined')).toThrow('Duplicate');
    expect(() => selectFixtures('missing')).toThrow('Unknown');
    expect(() => selectFixtures('')).toThrow('Unknown');
  });

  test('keeps local service fixtures under an explicit deny-by-default policy', () => {
    expect(() => prepareInfrastructurePolicy(runtimeProfilePolicy)).not.toThrow();
    expect(runtimeProfilePolicy.defaults).toEqual({ read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' });
    expect(runtimeProfilePolicy.rules[0].match.capabilities).toEqual(['state', 'storage', 'cache', 'locks']);
  });

  test('groups inclusive spans by process and name without inventing exclusive times', () => {
    const span = (process: string, name: string, durationMs: number): Span => ({
      profile: 'woml.performance-span/v1', traceId: 'test', spanId: 'span',
      layer: 'runtime', startOffsetMs: 0, status: 'succeeded',
      process, name, durationMs, counts: { events: 2 },
    } as Span);
    const grouped = groupSpans([
      span('rust', 'runtime.outer', 10), span('rust', 'runtime.inner', 9),
      span('rust', 'runtime.inner', 1), span('worker', 'runtime.inner', 3),
    ]);
    expect(grouped.length).toBe(3);
    const inner = grouped.find(item => item.process === 'rust' && item.name === 'runtime.inner')!;
    expect(inner.count).toBe(2);
    expect(inner.inclusiveTotalMs).toBe(10);
    expect(inner.counters.events).toBe(4);
    expect(inner.summary.medianMs).toBe(5);
    expect(grouped.reduce((sum, item) => sum + item.inclusiveTotalMs, 0)).toBe(23);
    expect(() => groupSpans([span('rust', 'runtime.test', -1)])).toThrow();
    expect(() => groupSpans([span('rust', 'runtime.test', Number.NaN)])).toThrow();
    expect(() => groupSpans([span('unknown', 'runtime.test', 1)])).toThrow();
  });
});
