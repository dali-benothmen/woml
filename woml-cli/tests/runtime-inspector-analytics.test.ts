import { describe, expect, test } from 'bun:test';
import { createInspectorAnalytics, decodeInspectorAnalytics, ANALYTICS_CACHE_MS } from '../src/runtime-inspector/analytics';
import { startRuntimeControl } from '../src/runtime-control';
import type { RunPresentationV1 } from '../src/terminal-presentation/types';

const instant = Date.parse('2026-11-02T12:00:00.000Z');
const workflow = { workflowId: 'customer-onboarding', name: 'Customer onboarding', description: 'Create an account and provision its workspace.', file: 'customer-onboarding.woml', triggerTypes: ['manual'] };
const date = (ms: number): string => new Date(instant + ms).toISOString();
function run(id: string, status: 'succeeded' | 'failed' | 'cancelled' | 'running' | 'waiting' | 'queued' = 'succeeded', duration = 1000) {
  return { runId: `run_${id}`, workflowId: workflow.workflowId, status, startedAt: date(-duration), updatedAt: date(0) };
}
function detail(runId: string): RunPresentationV1 {
  return {
    profile: 'woml.run-presentation/v1', workflow: { id: workflow.workflowId, definitionHash: `sha256:${'a'.repeat(64)}`, triggers: [] },
    runId, status: runId === 'run_failed' ? 'failed' : 'succeeded', trigger: { id: 'manual', type: 'manual' },
    admittedAt: date(-3000), completedAt: date(0), durationMs: 3000,
    steps: [{ id: 'provisionWorkspace', name: 'Provision workspace', kind: 'step', status: runId === 'run_failed' ? 'failed' : 'succeeded', depth: 0, attempts: 3, durationMs: 2700, completedAt: date(0),
      failure: { code: 'WOML_SCRIPT_THROWN', message: 'PRIVATE_SECRET_ERROR' }, result: 'PRIVATE_RESULT' }],
    summary: { succeeded: 0, failed: 1, skipped: 0, cancelled: 0, total: 1 }, lifecycle: [], warnings: [], result: 'PRIVATE_PAYLOAD',
  };
}

describe('Read-only inspector analytics', () => {
  test('calculates the real window, percentiles, retry counts and four-hour buckets', async () => {
    const provider = createInspectorAnalytics({ workflows: [workflow], now: () => instant,
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: [
        run('first'), run('failed', 'failed', 3000), run('cancelled', 'cancelled'), run('active', 'running'),
        { ...run('old'), updatedAt: date(-86_400_001) }, { ...run('future'), updatedAt: date(1) },
        { ...run('other'), workflowId: 'other' },
      ] }), inspectRun: detail });
    const value = await provider.get(workflow.workflowId);
    expect(value.totals).toEqual({ succeeded: 1, failed: 1, cancelled: 1, active: 1, queued: 0, waiting: 0,
      retriedRuns: 2, retryAttempts: 4, averageDurationMs: 2000, p95DurationMs: 3000, medianDurationMs: 1000, longestDurationMs: 3000 });
    expect(value.activity).toHaveLength(6);
    expect(value.activity.reduce((sum, bucket) => sum + bucket.total, 0)).toBe(2);
    expect(value.activity.at(-1)).toMatchObject({ succeeded: 1, failed: 1, total: 2 });
    expect(value.steps).toEqual([{ nodeId: 'provisionWorkspace', name: 'Provision workspace', averageDurationMs: 2700, failures: 1, retries: 4 }]);
    expect(JSON.stringify(value)).not.toContain('PRIVATE_');
    expect(decodeInspectorAnalytics(value, workflow.workflowId)).toEqual(value);
    expect(() => decodeInspectorAnalytics(value, 'other')).toThrow('invalid');
  });

  test('excludes cancelled and queued work from finished-duration statistics; empty is unknown, not zero', async () => {
    const provider = createInspectorAnalytics({ workflows: [workflow], now: () => instant,
      listRuns: () => ({ profile: 'woml.run-list/v2', runs: [
        { runId: 'run_queued', workflowId: workflow.workflowId, status: 'queued', admittedAt: date(-1000), updatedAt: date(0) },
        { ...run('cancelled', 'cancelled'), admittedAt: date(-1000) },
      ] }), inspectRun: () => { throw new Error('must not read'); } });
    expect((await provider.get(workflow.workflowId)).totals).toMatchObject({ succeeded: 0, failed: 0, cancelled: 1, queued: 1, retriedRuns: 0 });
    expect((await provider.get(workflow.workflowId)).totals.averageDurationMs).toBeUndefined();
  });

  test('bounds native detail reads, caches results, coalesces clients, and rejects unrelated work while busy', async () => {
    let clock = instant, listings = 0, reads = 0;
    const provider = createInspectorAnalytics({ workflows: [workflow, { ...workflow, workflowId: 'other' }], now: () => clock,
      listRuns: () => { listings++; return { profile: 'woml.run-list/v1', runs: Array.from({ length: 200 }, (_, index) => run(String(index))) }; },
      inspectRun: id => { reads++; return detail(id); } });
    const first = provider.get(workflow.workflowId);
    const second = provider.get(workflow.workflowId);
    await expect(provider.get('other')).rejects.toThrow('busy');
    await expect(provider.get('unknown')).rejects.toThrow('Unknown');
    const [a, b] = await Promise.all([first, second]);
    expect(a).toBe(b); expect(reads).toBe(10); expect(listings).toBe(1);
    expect(a.coverage).toMatchObject({ runs: 200, runsLimited: true, detailedRuns: 10, detailsLimited: true });
    await provider.get(workflow.workflowId); expect(reads).toBe(10);
    clock += ANALYTICS_CACHE_MS + 1;
    await provider.get(workflow.workflowId); expect(listings).toBe(2); expect(reads).toBe(20);
  });

  test('handles retention races, changed identity and interrupted details without exposing native errors', async () => {
    const provider = createInspectorAnalytics({ workflows: [workflow], now: () => instant,
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: [run('deleted'), run('changed')] }),
      inspectRun: id => {
        if (id === 'run_deleted') throw new Error('PRIVATE_NATIVE_PATH');
        return { ...detail(id), runId: 'run_substituted' };
      } });
    const result = await provider.get(workflow.workflowId);
    expect(result.coverage).toMatchObject({ detailedRuns: 0, unavailableRuns: 2, detailsLimited: true });
    expect(result.totals.succeeded).toBe(2);
    expect(JSON.stringify(result)).not.toContain('PRIVATE');
  });

  test('stops additional detail reads when the soft time budget is consumed', async () => {
    let clock = instant, reads = 0;
    const provider = createInspectorAnalytics({ workflows: [workflow], now: () => clock,
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: Array.from({ length: 20 }, (_, index) => run(String(index))) }),
      inspectRun: id => { reads++; clock += 300; return detail(id); } });
    const result = await provider.get(workflow.workflowId);
    expect(reads).toBe(1);
    expect(result.coverage.detailsLimited).toBe(true);
  });

  test('validates bounded shapes, timestamps, codes and non-finite numbers before rendering', async () => {
    const provider = createInspectorAnalytics({ workflows: [workflow], now: () => instant,
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: [run('failed', 'failed')] }), inspectRun: detail });
    const value = await provider.get(workflow.workflowId);
    for (const invalid of [
      { ...value, profile: 'future' }, { ...value, capturedAt: 'invalid' },
      { ...value, totals: { ...value.totals, averageDurationMs: Infinity } },
      { ...value, steps: Array(101).fill(value.steps[0]) },
      { ...value, failures: [{ at: date(0), nodeId: 'step', code: 'PRIVATE_ERROR' }] },
    ]) expect(() => decodeInspectorAnalytics(invalid, workflow.workflowId)).toThrow('invalid');
  });

  test('protects the endpoint with existing authentication, rate, query and response bounds', async () => {
    let requests = 0;
    const control = startRuntimeControl({ runtimeInstanceId: 'runtime_analytics', deploymentId: 'deployment_analytics', port: 0,
      maxOperationsPerMinute: 1, inspectorAnalytics: { get: async id => { requests++; return { workflowId: id }; } } });
    try {
      const url = `${control.descriptor.adminUrl}/v1/inspector/analytics`;
      const headers = { authorization: `Bearer ${control.descriptor.capability}` };
      expect((await fetch(`${url}?workflow=orders`)).status).toBe(401);
      expect((await fetch(url, { headers })).status).toBe(400);
      expect((await fetch(`${url}?workflow=orders&workflow=other`, { headers })).status).toBe(400);
      expect((await fetch(`${url}?workflow=orders&extra=1`, { headers })).status).toBe(400);
      expect(requests).toBe(0);
      const response = await fetch(`${url}?workflow=orders`, { headers });
      expect(response.status).toBe(200); expect(response.headers.get('cache-control')).toBe('no-store');
      expect(await response.json()).toEqual({ workflowId: 'orders' });
      expect((await fetch(`${url}?workflow=orders`, { headers })).status).toBe(429);
      expect(requests).toBe(1);
    } finally { await control.close(); }
    const bounded = startRuntimeControl({ runtimeInstanceId: 'runtime_analytics_bounds', deploymentId: 'deployment_analytics_bounds', port: 0,
      maxResponseBytes: 1024, inspectorAnalytics: { get: async () => ({ value: 'x'.repeat(2000) }) } });
    try {
      const response = await fetch(`${bounded.descriptor.adminUrl}/v1/inspector/analytics?workflow=orders`, { headers: { authorization: `Bearer ${bounded.descriptor.capability}` } });
      expect(response.status).toBe(507);
    } finally { await bounded.close(); }
  });
});
