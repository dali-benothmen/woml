import type { RustRunListV1 } from '../rust-executor';
import type { RunPresentationV1 } from '../terminal-presentation/types';

export interface AnalyticsWorkflow {
  readonly workflowId: string;
  readonly name?: string;
  readonly description?: string;
  readonly file?: string;
  readonly triggerTypes: readonly string[];
}

export interface InspectorAnalyticsV1 {
  readonly profile: 'woml.inspector-analytics/v1';
  readonly workflow: AnalyticsWorkflow;
  readonly capturedAt: string;
  readonly windowStart: string;
  readonly coverage: {
    readonly runs: number;
    readonly runsLimited: boolean;
    readonly detailedRuns: number;
    readonly detailsLimited: boolean;
    readonly unavailableRuns: number;
  };
  readonly totals: {
    readonly succeeded: number; readonly failed: number; readonly cancelled: number;
    readonly active: number; readonly queued: number; readonly waiting: number;
    readonly retriedRuns: number; readonly retryAttempts: number;
    readonly averageDurationMs?: number; readonly p95DurationMs?: number;
    readonly medianDurationMs?: number; readonly longestDurationMs?: number;
  };
  readonly activity: readonly {
    readonly startAt: string; readonly endAt: string;
    readonly succeeded: number; readonly failed: number; readonly total: number;
  }[];
  readonly steps: readonly {
    readonly nodeId: string; readonly name?: string;
    readonly averageDurationMs?: number; readonly failures: number; readonly retries: number;
  }[];
  // Deliberately only codes, never script error messages, results or payloads.
  readonly failures: readonly { readonly at: string; readonly nodeId: string; readonly code: string }[];
}

export const ANALYTICS_RUN_LIMIT = 200;
export const ANALYTICS_DETAIL_LIMIT = 10;
export const ANALYTICS_CACHE_MS = 15_000;
const DAY = 86_400_000;
const FOUR_HOURS = DAY / 6;

function percentile(sorted: readonly number[], fraction: number): number | undefined {
  return sorted.length === 0 ? undefined : sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
}

/** Read-only adapter over existing native inspection APIs. No database access in the renderer. */
export function createInspectorAnalytics(options: {
  readonly workflows: readonly AnalyticsWorkflow[];
  readonly listRuns: (workflowId: string) => RustRunListV1;
  readonly inspectRun: (runId: string) => RunPresentationV1 | Promise<RunPresentationV1>;
  readonly now?: () => number;
}) {
  const now = options.now ?? Date.now;
  const workflows = new Map(options.workflows.map(workflow => [workflow.workflowId, workflow]));
  const cache = new Map<string, { at: number; value: InspectorAnalyticsV1 }>();
  // One bounded build at a time, across all inspector clients and workflows.
  let pending: { id: string; value: Promise<InspectorAnalyticsV1> } | undefined;
  async function build(workflow: AnalyticsWorkflow): Promise<InspectorAnalyticsV1> {
    const end = now();
    const start = end - DAY;
    const listed = options.listRuns(workflow.workflowId).runs;
    const own = listed.slice(0, ANALYTICS_RUN_LIMIT).filter(run => run.workflowId === workflow.workflowId);
    const runs = own.filter(run => Date.parse(run.updatedAt) >= start && Date.parse(run.updatedAt) <= end);
    const count = (status: string): number => runs.filter(run => run.status === status).length;
    const terminal = runs.filter(run => run.status === 'succeeded' || run.status === 'failed');
    const durations = terminal.flatMap(run => {
      const began = run.startedAt;
      const duration = began === undefined ? NaN : Date.parse(run.updatedAt) - Date.parse(began);
      return Number.isFinite(duration) && duration >= 0 ? [duration] : [];
    }).sort((a, b) => a - b);
    const activity = Array.from({ length: 6 }, (_, index) => ({
      startAt: new Date(start + index * FOUR_HOURS).toISOString(),
      endAt: new Date(start + (index + 1) * FOUR_HOURS).toISOString(),
      succeeded: 0, failed: 0, total: 0,
    }));
    for (const run of terminal) {
      const index = Math.min(5, Math.floor((Date.parse(run.updatedAt) - start) / FOUR_HOURS));
      const bucket = activity[index]!;
      if (run.status === 'succeeded') bucket.succeeded += 1;
      else bucket.failed += 1;
      bucket.total += 1;
    }
    const steps = new Map<string, { nodeId: string; name?: string; total: number; samples: number; failures: number; retries: number }>();
    const failures: { at: string; nodeId: string; code: string }[] = [];
    let retriedRuns = 0, retryAttempts = 0, detailedRuns = 0, unavailableRuns = 0;
    let truncatedSteps = false;
    for (const run of terminal.slice(0, ANALYTICS_DETAIL_LIMIT)) {
      await new Promise<void>(resolve => setTimeout(resolve, 0));
      // Stop additional native reads after the budget; no unbounded batch.
      if (now() - end > 250) break;
      try {
        const detail = await options.inspectRun(run.runId);
        if (detail.runId !== run.runId || detail.workflow.id !== workflow.workflowId || detail.status !== run.status) {
          unavailableRuns += 1;
          continue;
        }
        detailedRuns += 1;
        truncatedSteps ||= detail.steps.length > 200 || detail.steps.some(step => step.forEach?.iterationsTruncated);
        let retries = 0;
        for (const step of detail.steps.slice(0, 200)) {
          const value = steps.get(step.id) ?? { nodeId: step.id, name: step.name, total: 0, samples: 0, failures: 0, retries: 0 };
          if (step.durationMs !== undefined && Number.isFinite(step.durationMs) && step.durationMs >= 0) {
            value.total += step.durationMs;
            value.samples += 1;
          }
          value.failures += Number(step.status === 'failed' || step.status === 'timed_out');
          const extra = Math.max(0, step.attempts - 1);
          value.retries += extra;
          retries += extra;
          steps.set(step.id, value);
          if (step.failure !== undefined && failures.length < 10) {
            failures.push({ at: step.completedAt ?? run.updatedAt, nodeId: step.id, code: step.failure.code });
          }
        }
        retryAttempts += retries;
        retriedRuns += Number(retries > 0);
      } catch {
        // Retention can remove a run between reads. Never expose exception text.
        unavailableRuns += 1;
      }
    }
    return {
      profile: 'woml.inspector-analytics/v1', workflow,
      capturedAt: new Date(end).toISOString(), windowStart: new Date(start).toISOString(),
      coverage: { runs: runs.length, runsLimited: listed.length >= ANALYTICS_RUN_LIMIT,
        detailedRuns, detailsLimited: detailedRuns < terminal.length || steps.size > 100 || truncatedSteps, unavailableRuns },
      totals: {
        succeeded: count('succeeded'), failed: count('failed'), cancelled: count('cancelled'),
        active: own.filter(run => ['running', 'cancelling', 'finalizing'].includes(run.status)).length,
        queued: own.filter(run => run.status === 'queued' || run.status === 'not_started').length,
        waiting: own.filter(run => run.status === 'waiting').length,
        retriedRuns, retryAttempts,
        ...(durations.length === 0 ? {} : {
          averageDurationMs: durations.reduce((sum, ms) => sum + ms, 0) / durations.length,
          p95DurationMs: percentile(durations, 0.95), medianDurationMs: percentile(durations, 0.5),
          longestDurationMs: durations.at(-1),
        }),
      }, activity,
      steps: [...steps.values()].slice(0, 100).map(step => ({
        nodeId: step.nodeId, ...(step.name === undefined ? {} : { name: step.name }),
        ...(step.samples === 0 ? {} : { averageDurationMs: step.total / step.samples }),
        failures: step.failures, retries: step.retries,
      })),
      failures: failures.sort((a, b) => b.at.localeCompare(a.at)).slice(0, 3),
    };
  }
  return {
    async get(workflowId: string): Promise<InspectorAnalyticsV1> {
      const workflow = workflows.get(workflowId);
      if (workflow === undefined) throw new Error('Unknown inspector workflow.');
      const entry = cache.get(workflowId);
      if (entry !== undefined && now() - entry.at < ANALYTICS_CACHE_MS) return entry.value;
      if (pending !== undefined) {
        if (pending.id === workflowId) return pending.value;
        throw new Error('Inspector analytics is busy.');
      }
      const value = build(workflow);
      pending = { id: workflowId, value };
      try {
        const result = await value;
        if (cache.size >= 8) cache.delete(cache.keys().next().value!);
        cache.set(workflowId, { at: now(), value: result });
        return result;
      } finally { pending = undefined; }
    },
  };
}

/** Validate before the response can influence layout or iteration counts. */
export function decodeInspectorAnalytics(value: unknown, workflowId: string): InspectorAnalyticsV1 {
  const fail = (): never => { throw new Error('The runtime returned invalid inspector analytics.'); };
  const object = (input: unknown): Record<string, unknown> =>
    input !== null && typeof input === 'object' && !Array.isArray(input) ? input as Record<string, unknown> : fail();
  const text = (input: unknown, maximum = 320): string =>
    typeof input === 'string' && input.length <= maximum ? input : fail();
  const number = (input: unknown): number =>
    typeof input === 'number' && Number.isFinite(input) && input >= 0 ? input : fail();
  const date = (input: unknown): string => {
    const result = text(input, 64);
    return Number.isFinite(Date.parse(result)) ? result : fail();
  };
  const list = (input: unknown, maximum: number): unknown[] =>
    Array.isArray(input) && input.length <= maximum ? input : fail();
  const root = object(value), workflow = object(root.workflow), totals = object(root.totals), coverage = object(root.coverage);
  if (root.profile !== 'woml.inspector-analytics/v1' || workflow.workflowId !== workflowId) fail();
  text(workflow.workflowId); date(root.capturedAt); date(root.windowStart);
  for (const key of ['name', 'description', 'file']) if (workflow[key] !== undefined) text(workflow[key], 2048);
  list(workflow.triggerTypes, 16).forEach(item => text(item, 32));
  for (const key of ['succeeded', 'failed', 'cancelled', 'active', 'queued', 'waiting', 'retriedRuns', 'retryAttempts']) number(totals[key]);
  for (const key of ['averageDurationMs', 'p95DurationMs', 'medianDurationMs', 'longestDurationMs']) if (totals[key] !== undefined) number(totals[key]);
  for (const key of ['runs', 'detailedRuns', 'unavailableRuns']) number(coverage[key]);
  if (typeof coverage.runsLimited !== 'boolean' || typeof coverage.detailsLimited !== 'boolean') fail();
  list(root.activity, 6).forEach(input => {
    const item = object(input); date(item.startAt); date(item.endAt);
    for (const key of ['succeeded', 'failed', 'total']) number(item[key]);
  });
  list(root.steps, 100).forEach(input => {
    const item = object(input); text(item.nodeId); if (item.name !== undefined) text(item.name, 2048);
    number(item.failures); number(item.retries); if (item.averageDurationMs !== undefined) number(item.averageDurationMs);
  });
  list(root.failures, 3).forEach(input => {
    const item = object(input); date(item.at); text(item.nodeId);
    if (!/^WOML_[A-Z0-9_]{1,123}$/.test(text(item.code, 124))) fail();
  });
  return value as InspectorAnalyticsV1;
}
