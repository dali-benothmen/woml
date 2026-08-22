import type {
  RustRunListV1,
  RustRuntimeObservationV1,
} from './rust-executor';

export type RuntimeLifecycle =
  | 'starting'
  | 'recovering'
  | 'ready'
  | 'degraded'
  | 'draining'
  | 'stopped'
  | 'failed';

export interface ObservedWorkflow {
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly triggerTypes: readonly (
    | 'manual'
    | 'webhook'
    | 'slack'
    | 'telegram'
    | 'discord'
    | 'whatsapp'
    | 'schedule'
    | 'interval'
    | 'event'
  )[];
}

export interface OperationsStreamV1 {
  readonly profile: 'woml.runtime-operations-stream/v1';
  readonly runtimeInstanceId: string;
  readonly sequence: number;
  readonly occurredAt: string;
  readonly kind:
    | 'runtime'
    | 'workflow'
    | 'run'
    | 'trigger'
    | 'approval'
    | 'retry'
    | 'workflow_call'
    | 'policy'
    | 'provider'
    | 'storage'
    | 'maintenance'
    | 'alert';
  readonly subject: {
    readonly id: string;
    readonly status: string;
    readonly code?: string;
  };
}

export interface RuntimeLogRecordV1 {
  readonly profile: 'woml.runtime-log-record/v1';
  readonly timestamp: string;
  readonly level: 'error' | 'warn' | 'info' | 'debug';
  readonly code: string;
  readonly message: string;
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly workflowId?: string;
  readonly runId?: string;
  readonly triggerId?: string;
  readonly nodeId?: string;
  readonly provider?: string;
}

export interface RuntimeObservabilitySurface {
  minimalHealth(kind: 'liveness' | 'readiness'): unknown;
  detailedHealth(): unknown;
  snapshot(): Promise<unknown>;
  metrics(): Promise<readonly unknown[]>;
  prometheusMetrics(): Promise<string>;
  stream(afterSequence?: number): Response;
  closeStreams(): void;
}

type RuntimeComponentKind =
  | 'store'
  | 'trigger'
  | 'provider'
  | 'worker'
  | 'backup'
  | 'retention';

interface StreamSubscriber {
  readonly id: number;
  readonly controller: ReadableStreamDefaultController<Uint8Array>;
}

interface ObservedForEachProgress {
  readonly runId: string;
  readonly forEachId: string;
  readonly status: 'running' | 'succeeded' | 'failed' | 'cancelled';
  readonly total: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly skipped: number;
  readonly active: number;
  readonly pending: number;
  readonly concurrency: number;
}

const STREAM_BUFFER_SIZE = 1024;
const MAX_STREAM_CLIENTS = 8;
const MAX_ALERTS = 200;
const SAFE_CODE = /^WOML_[A-Z0-9_]{1,123}$/;
const encoder = new TextEncoder();

function bounded(value: unknown, fallback: string, maximum = 320): string {
  if (typeof value !== 'string' || value.length === 0) return fallback;
  return value.slice(0, maximum);
}

function safeCode(value: unknown): string | undefined {
  return typeof value === 'string' && SAFE_CODE.test(value) ? value : undefined;
}

function nonNegativeInteger(value: unknown, maximum = 10_000): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= maximum
    ? value
    : undefined;
}

function publicStatus(status: string):
  | 'queued'
  | 'running'
  | 'waiting'
  | 'retrying'
  | 'succeeded'
  | 'failed'
  | 'cancelled' {
  if (status === 'not_started' || status === 'queued') return 'queued';
  if (status === 'waiting') return 'waiting';
  if (status === 'succeeded' || status === 'failed' || status === 'cancelled')
    return status;
  return 'running';
}

function progressKind(value: Record<string, unknown>): OperationsStreamV1['kind'] {
  const contract = String(value.contract ?? value.profile ?? '');
  if (contract.includes('workflow-call')) return 'workflow_call';
  if (contract.includes('approval')) return 'approval';
  if (contract.includes('retry')) return 'retry';
  if (contract.includes('runtime-policy')) return 'policy';
  if (contract.includes('schedule') || contract.includes('interval') || contract.includes('trigger'))
    return 'trigger';
  if (contract.includes('runtime-instance')) return 'runtime';
  if (contract.includes('notification') || contract.includes('provider')) return 'provider';
  return 'run';
}

function progressStatus(value: Record<string, unknown>): string {
  return bounded(
    value.status ?? value.phase ?? value.lifecycle ?? value.type ?? value.messageType,
    'updated',
    64
  );
}

function progressId(value: Record<string, unknown>, fallback: string): string {
  return bounded(
    value.runId ??
      value.triggerId ??
      value.workflowId ??
      value.approvalId ??
      value.requestId ??
      value.runtimeInstanceId,
    fallback
  );
}

export class RuntimeObservability implements RuntimeObservabilitySurface {
  static readonly MAX_OBSERVED_FOR_EACH = 10_000;
  readonly #runtimeInstanceId: string;
  readonly #deploymentId: string;
  readonly #startedAt: number;
  readonly #workflows: readonly ObservedWorkflow[];
  readonly #listRuns: () => RustRunListV1;
  readonly #observeDurable?: () => RustRuntimeObservationV1;
  readonly #storeSize: () => Promise<number>;
  readonly #emitLog: (text: string) => void;
  readonly #logFormat: 'text' | 'json';
  readonly #now: () => number;
  readonly #streamBuffer: OperationsStreamV1[] = [];
  readonly #subscribers = new Map<number, StreamSubscriber>();
  readonly #alerts: {
    at: string;
    level: 'warn' | 'error';
    code: string;
    message: string;
  }[] = [];
  readonly #counters = new Map<string, number>();
  readonly #currentNodes = new Map<string, string>();
  readonly #parentRuns = new Map<string, string>();
  readonly #forEachProgress = new Map<string, ObservedForEachProgress>();
  #nextSubscriberId = 1;
  #sequence = 0;
  #lifecycle: RuntimeLifecycle = 'starting';
  #components: { name: string; kind: RuntimeComponentKind; status: 'ready' | 'degraded' | 'unready' | 'stopped'; code?: string }[];

  constructor(options: {
    readonly runtimeInstanceId: string;
    readonly deploymentId: string;
    readonly workflows: readonly ObservedWorkflow[];
    readonly listRuns: () => RustRunListV1;
    readonly observeDurable?: () => RustRuntimeObservationV1;
    readonly storeSize: () => Promise<number>;
    readonly logFormat: 'text' | 'json';
    readonly emitLog: (text: string) => void;
    readonly components?: readonly { name: string; kind: RuntimeComponentKind; status: 'ready' | 'degraded' | 'unready' | 'stopped'; code?: string }[];
    readonly now?: () => number;
  }) {
    this.#runtimeInstanceId = options.runtimeInstanceId;
    this.#deploymentId = options.deploymentId;
    this.#now = options.now ?? Date.now;
    this.#startedAt = this.#now();
    this.#workflows = [...options.workflows].sort((a, b) =>
      a.workflowId.localeCompare(b.workflowId)
    );
    this.#listRuns = options.listRuns;
    this.#observeDurable = options.observeDurable;
    this.#storeSize = options.storeSize;
    this.#logFormat = options.logFormat;
    this.#emitLog = options.emitLog;
    this.#components = [...(options.components ?? [])];
  }

  setLifecycle(lifecycle: RuntimeLifecycle): void {
    this.#lifecycle = lifecycle;
    this.record('runtime', this.#runtimeInstanceId, lifecycle);
  }

  setComponent(
    name: string,
    kind: RuntimeComponentKind,
    status: 'ready' | 'degraded' | 'unready' | 'stopped',
    code?: string
  ): void {
    const item = {
      name: bounded(name, 'component'),
      kind,
      status,
      ...(safeCode(code) === undefined ? {} : { code: safeCode(code)! }),
    };
    this.#components = [
      ...this.#components.filter(existing => existing.name !== item.name),
      item,
    ].slice(0, 1000);
    this.record(
      kind === 'trigger'
        ? 'trigger'
        : kind === 'store'
          ? 'storage'
          : kind === 'backup' || kind === 'retention'
            ? 'maintenance'
            : 'provider',
      item.name,
      status,
      item.code
    );
  }

  recordMaintenance(
    operation: 'backup' | 'retention',
    status: 'completed' | 'failed',
    code?: string
  ): void {
    this.record('maintenance', operation, status, code);
    this.#counters.set(
      `maintenance:${operation}:${status}`,
      (this.#counters.get(`maintenance:${operation}:${status}`) ?? 0) + 1
    );
  }

  recordProgress(progress: unknown): void {
    if (progress === null || typeof progress !== 'object' || Array.isArray(progress)) return;
    const value = progress as Record<string, unknown>;
    const runId = typeof value.runId === 'string'
      ? value.runId
      : typeof value.parentRunId === 'string'
        ? value.parentRunId
        : undefined;
    const nodeId = typeof value.nodeId === 'string'
      ? value.nodeId
      : typeof value.stepId === 'string'
        ? value.stepId
        : typeof value.parentNodeId === 'string'
          ? value.parentNodeId
          : typeof value.forEachId === 'string'
            ? value.forEachId
            : undefined;
    if (runId !== undefined && nodeId !== undefined) {
      this.#remember(this.#currentNodes, bounded(runId, 'run_unknown'), bounded(nodeId, 'node_unknown'));
    }
    if (typeof value.childRunId === 'string' && typeof value.parentRunId === 'string') {
      this.#remember(
        this.#parentRuns,
        bounded(value.childRunId, 'run_unknown'),
        bounded(value.parentRunId, 'run_unknown')
      );
    }
    if (
      value.type === 'for_each_progress' &&
      typeof value.runId === 'string' &&
      typeof value.forEachId === 'string' &&
      ['running', 'succeeded', 'failed', 'cancelled'].includes(String(value.status)) &&
      nonNegativeInteger(value.total) !== undefined &&
      nonNegativeInteger(value.succeeded) !== undefined &&
      nonNegativeInteger(value.failed) !== undefined &&
      nonNegativeInteger(value.skipped) !== undefined &&
      nonNegativeInteger(value.active) !== undefined &&
      nonNegativeInteger(value.pending) !== undefined &&
      nonNegativeInteger(value.concurrency, 64) !== undefined &&
      Number(value.concurrency) >= 1 &&
      Number(value.succeeded) + Number(value.failed) + Number(value.skipped) +
        Number(value.active) + Number(value.pending) === Number(value.total)
    ) {
      const progress: ObservedForEachProgress = {
        runId: bounded(value.runId, 'run_unknown'),
        forEachId: bounded(value.forEachId, 'for_each_unknown'),
        status: String(value.status) as ObservedForEachProgress['status'],
        total: Number(value.total),
        succeeded: Number(value.succeeded),
        failed: Number(value.failed),
        skipped: Number(value.skipped),
        active: Number(value.active),
        pending: Number(value.pending),
        concurrency: Number(value.concurrency),
      };
      const progressKey = `${progress.runId}:${progress.forEachId}`;
      const previous = this.#forEachProgress.get(progressKey);
      const completed = progress.succeeded + progress.failed + progress.skipped;
      const previousCompleted = previous === undefined
        ? 0
        : previous.succeeded + previous.failed + previous.skipped;
      this.#counters.set(
        'for_each:completed',
        (this.#counters.get('for_each:completed') ?? 0) +
          Math.max(0, completed - previousCompleted)
      );
      this.#forEachProgress.set(progressKey, progress);
      while (this.#forEachProgress.size > RuntimeObservability.MAX_OBSERVED_FOR_EACH) {
        const oldest = this.#forEachProgress.keys().next().value;
        if (oldest === undefined) break;
        this.#forEachProgress.delete(oldest);
      }
    }
    const kind = progressKind(value);
    const status = progressStatus(value);
    if (
      kind === 'runtime' &&
      [
        'starting',
        'recovering',
        'ready',
        'degraded',
        'draining',
        'stopped',
        'failed',
      ].includes(status)
    ) {
      this.#lifecycle = status as RuntimeLifecycle;
    }
    const id = progressId(value, `${kind}_unknown`);
    const code = safeCode(value.code);
    this.record(kind, id, status, code);
    this.#counters.set(`${kind}:${status}`, (this.#counters.get(`${kind}:${status}`) ?? 0) + 1);
    const fields =
      kind === 'trigger'
        ? { triggerId: id }
        : kind === 'provider'
          ? { provider: id }
          : { runId: id };
    this.log(
      code === undefined ? 'debug' : 'warn',
      code ?? 'WOML_RUNTIME_PROGRESS',
      `${kind.replaceAll('_', ' ')} ${status}.`,
      fields
    );
  }

  alert(code: string, message: string, level: 'warn' | 'error' = 'error'): void {
    const normalizedCode = safeCode(code) ?? 'WOML_OBSERVABILITY_ALERT';
    const safeMessage = bounded(message, 'WOML runtime alert.', 1024);
    this.#alerts.push({
      at: new Date(this.#now()).toISOString(),
      level,
      code: normalizedCode,
      message: safeMessage,
    });
    if (this.#alerts.length > MAX_ALERTS) this.#alerts.shift();
    this.record('alert', normalizedCode, level, normalizedCode);
    this.log(level, normalizedCode, safeMessage);
  }

  log(
    level: RuntimeLogRecordV1['level'],
    code: string,
    message: string,
    fields: Pick<RuntimeLogRecordV1, 'workflowId' | 'runId' | 'triggerId' | 'nodeId' | 'provider'> = {}
  ): void {
    const record: RuntimeLogRecordV1 = {
      profile: 'woml.runtime-log-record/v1',
      timestamp: new Date(this.#now()).toISOString(),
      level,
      code: safeCode(code) ?? 'WOML_RUNTIME_LOG',
      message: bounded(message, 'WOML runtime update.', 2048),
      runtimeInstanceId: this.#runtimeInstanceId,
      deploymentId: this.#deploymentId,
      ...Object.fromEntries(
        Object.entries(fields)
          .filter((entry): entry is [string, string] => typeof entry[1] === 'string')
          .map(([key, value]) => [key, bounded(value, 'unknown')])
      ),
    };
    if (this.#logFormat === 'json') {
      this.#emitLog(`${JSON.stringify(record)}\n`);
      return;
    }
    const correlations = [
      record.workflowId === undefined ? undefined : `workflow=${record.workflowId}`,
      record.runId === undefined ? undefined : `run=${record.runId}`,
      record.triggerId === undefined ? undefined : `trigger=${record.triggerId}`,
      record.nodeId === undefined ? undefined : `node=${record.nodeId}`,
      record.provider === undefined ? undefined : `provider=${record.provider}`,
    ].filter(Boolean);
    this.#emitLog(
      `${record.timestamp} ${record.level.toUpperCase()} ${record.code} ${record.message}${correlations.length === 0 ? '' : ` ${correlations.join(' ')}`}\n`
    );
  }

  minimalHealth(kind: 'liveness' | 'readiness'): unknown {
    const live = !['stopped', 'failed'].includes(this.#lifecycle);
    const ready = this.#lifecycle === 'ready';
    return {
      profile: 'woml.runtime-health/v1',
      kind,
      status: (kind === 'liveness' ? live : ready) ? 'ok' : 'unready',
    };
  }

  detailedHealth(): unknown {
    const status =
      this.#lifecycle === 'ready'
        ? this.#components.some(component => component.status === 'degraded')
          ? 'degraded'
          : 'ready'
        : this.#lifecycle === 'degraded'
          ? 'degraded'
          : 'unready';
    return {
      profile: 'woml.runtime-health/v1',
      kind: 'detail',
      status,
      runtimeInstanceId: this.#runtimeInstanceId,
      components: this.#components.map(({ name, status, code }) => ({
        name,
        status,
        ...(code === undefined ? {} : { code }),
      })),
    };
  }

  async snapshot(): Promise<unknown> {
    const capturedAt = this.#now();
    const listed = this.#listRuns();
    const durable = this.#observeDurable?.();
    const retrying = new Set(durable?.retryingRunIds ?? []);
    const runs = listed.runs.slice(0, 1000).map(run => {
      const start = 'admittedAt' in run ? run.admittedAt : run.startedAt;
      const end = ['succeeded', 'failed', 'cancelled'].includes(run.status)
        ? run.updatedAt
        : new Date(capturedAt).toISOString();
      return {
        runId: run.runId,
        workflowId: run.workflowId,
        status: retrying.has(run.runId) ? ('retrying' as const) : publicStatus(run.status),
        durationMs: Math.max(0, Date.parse(end) - Date.parse(start)),
        ...(this.#currentNodes.get(run.runId) === undefined
          ? {}
          : { currentNodeId: this.#currentNodes.get(run.runId)! }),
        ...(this.#parentRuns.get(run.runId) === undefined
          ? {}
          : { parentRunId: this.#parentRuns.get(run.runId)! }),
        ...(() => {
          const loops = [...this.#forEachProgress.values()]
            .filter(progress => progress.runId === run.runId)
            .sort((left, right) => left.forEachId.localeCompare(right.forEachId))
            .slice(0, 100);
          return loops.length === 0 ? {} : { forEach: loops };
        })(),
      };
    });
    const workflows = this.#workflows.map(workflow => {
      const own = runs.filter(run => run.workflowId === workflow.workflowId);
      return {
        workflowId: workflow.workflowId,
        definitionHash: workflow.definitionHash,
        active: own.filter(run => run.status === 'running' || run.status === 'retrying').length,
        queued: own.filter(run => run.status === 'queued').length,
        waiting: own.filter(run => run.status === 'waiting').length,
        failed: own.filter(run => run.status === 'failed').length,
        triggerTypes: [...new Set(workflow.triggerTypes)].sort(),
      };
    });
    return {
      profile: 'woml.runtime-operations-snapshot/v1',
      runtimeInstanceId: this.#runtimeInstanceId,
      sequence: this.#sequence,
      capturedAt: new Date(capturedAt).toISOString(),
      lifecycle: this.#lifecycle,
      ready: this.#lifecycle === 'ready',
      uptimeMs: Math.max(0, capturedAt - this.#startedAt),
      workflows,
      runs,
      components: this.#components,
      alerts: this.#alerts,
    };
  }

  async metrics(): Promise<readonly unknown[]> {
    const snapshot = (await this.snapshot()) as {
      ready: boolean;
      uptimeMs: number;
      workflows: readonly unknown[];
      runs: readonly { status: string; durationMs: number }[];
    };
    const metric = (name: string, type: 'gauge' | 'counter' | 'histogram', value: number, labels: Record<string, string> = {}) => ({
      profile: 'woml.runtime-metrics/v1',
      name,
      type,
      value,
      labels,
    });
    const durable = this.#observeDurable?.();
    const statusTotal = (status: string): number =>
      durable?.statusTotals[status] ??
      snapshot.runs.filter(run => run.status === status).length;
    const retrying = durable?.retryingRunIds.length ??
      snapshot.runs.filter(run => run.status === 'retrying').length;
    const active = Math.max(
      0,
      statusTotal('running') + statusTotal('cancelling') + statusTotal('finalizing')
    );
    const results: unknown[] = [
      metric('woml_runtime_ready', 'gauge', snapshot.ready ? 1 : 0),
      metric('woml_runtime_uptime_seconds', 'gauge', snapshot.uptimeMs / 1000),
      metric('woml_workflows_loaded', 'gauge', snapshot.workflows.length),
      metric('woml_runs_active', 'gauge', active),
      metric('woml_runs_queued', 'gauge', statusTotal('queued') + statusTotal('not_started')),
      metric('woml_runs_waiting', 'gauge', statusTotal('waiting')),
      metric('woml_run_duration_seconds', 'histogram', snapshot.runs.length === 0 ? 0 : snapshot.runs.reduce((sum, run) => sum + run.durationMs, 0) / snapshot.runs.length / 1000),
      metric('woml_store_size_bytes', 'gauge', await this.#storeSize(), { provider: 'sqlite' }),
    ];
    for (const status of ['queued', 'running', 'waiting', 'retrying', 'succeeded', 'failed', 'cancelled']) {
      const value =
        status === 'queued'
          ? statusTotal('queued') + statusTotal('not_started')
          : status === 'running'
            ? active
            : status === 'retrying'
              ? retrying
              : statusTotal(status);
      results.push(metric('woml_runs_total', 'counter', value, { status }));
    }
    results.push(
      metric('woml_triggers_total', 'counter', durable?.triggersTotal ?? this.#counterPrefix('trigger:')),
      metric('woml_retries_total', 'counter', durable?.retriesTotal ?? this.#counterPrefix('retry:')),
      metric('woml_approvals_waiting', 'gauge', durable?.approvalWaitingRunIds.length ?? this.#counters.get('approval:waiting') ?? 0),
      metric('woml_workflow_calls_active', 'gauge', durable?.workflowCallsActive ?? this.#counterPrefix('workflow_call:')),
      metric('woml_worker_restarts_total', 'counter', this.#counters.get('provider:restarted') ?? 0, { provider: 'script_host' })
    );
    const loops = [...this.#forEachProgress.values()];
    results.push(
      metric('woml_for_each_iterations_active', 'gauge', loops.reduce((sum, loop) => sum + loop.active, 0)),
      metric('woml_for_each_iterations_pending', 'gauge', loops.reduce((sum, loop) => sum + loop.pending, 0)),
      metric('woml_for_each_iterations_completed_total', 'counter', this.#counters.get('for_each:completed') ?? 0)
    );
    for (const [operation, status] of [
      ['backup', 'completed'],
      ['retention', 'completed'],
      ['retention', 'failed'],
    ] as const) {
      const value = this.#counters.get(`maintenance:${operation}:${status}`);
      if (value !== undefined) {
        results.push(
          metric(`woml_${operation}_total`, 'counter', value, { status })
        );
      }
    }
    return results;
  }

  async prometheusMetrics(): Promise<string> {
    const lines: string[] = [];
    for (const candidate of await this.metrics()) {
      const metric = candidate as { name: string; type: string; value: number; labels: Record<string, string> };
      const labels = Object.entries(metric.labels);
      lines.push(`# TYPE ${metric.name} ${metric.type === 'histogram' ? 'gauge' : metric.type}`);
      lines.push(
        `${metric.name}${labels.length === 0 ? '' : `{${labels.map(([key, value]) => `${key}="${value}"`).join(',')}}`} ${Number.isFinite(metric.value) ? metric.value : 0}`
      );
    }
    return `${lines.join('\n')}\n`;
  }

  stream(afterSequence = this.#sequence): Response {
    if (this.#subscribers.size >= MAX_STREAM_CLIENTS) {
      return Response.json(
        { error: { code: 'WOML_OBSERVABILITY_CLIENT_LIMIT' } },
        { status: 503 }
      );
    }
    let subscriberId = 0;
    const stream = new ReadableStream<Uint8Array>({
      start: controller => {
        subscriberId = this.#nextSubscriberId++;
        const oldest = this.#streamBuffer[0]?.sequence ?? this.#sequence + 1;
        if (afterSequence < oldest - 1) {
          controller.enqueue(this.#sse(this.#gapEvent()));
          controller.close();
          return;
        }
        for (const event of this.#streamBuffer) {
          if (event.sequence > afterSequence) controller.enqueue(this.#sse(event));
        }
        this.#subscribers.set(subscriberId, { id: subscriberId, controller });
      },
      cancel: () => {
        this.#subscribers.delete(subscriberId);
      },
    });
    return new Response(stream, {
      headers: {
        'content-type': 'text/event-stream; charset=utf-8',
        'cache-control': 'no-store',
        connection: 'keep-alive',
      },
    });
  }

  closeStreams(): void {
    for (const subscriber of this.#subscribers.values()) {
      try {
        subscriber.controller.close();
      } catch {
        // A disconnected telemetry client never affects workflow execution.
      }
    }
    this.#subscribers.clear();
  }

  #recordEvent(event: OperationsStreamV1): void {
    this.#streamBuffer.push(event);
    if (this.#streamBuffer.length > STREAM_BUFFER_SIZE) this.#streamBuffer.shift();
    const encoded = this.#sse(event);
    for (const subscriber of [...this.#subscribers.values()]) {
      try {
        if ((subscriber.controller.desiredSize ?? 1) <= 0) {
          subscriber.controller.enqueue(this.#sse(this.#gapEvent()));
          subscriber.controller.close();
          this.#subscribers.delete(subscriber.id);
          continue;
        }
        subscriber.controller.enqueue(encoded);
      } catch {
        this.#subscribers.delete(subscriber.id);
      }
    }
  }

  record(kind: OperationsStreamV1['kind'], id: string, status: string, code?: string): void {
    this.#sequence += 1;
    this.#recordEvent({
      profile: 'woml.runtime-operations-stream/v1',
      runtimeInstanceId: this.#runtimeInstanceId,
      sequence: this.#sequence,
      occurredAt: new Date(this.#now()).toISOString(),
      kind,
      subject: {
        id: bounded(id, `${kind}_unknown`),
        status: bounded(status, 'updated', 64),
        ...(safeCode(code) === undefined ? {} : { code: safeCode(code)! }),
      },
    });
  }

  #gapEvent(): OperationsStreamV1 {
    return {
      profile: 'woml.runtime-operations-stream/v1',
      runtimeInstanceId: this.#runtimeInstanceId,
      sequence: Math.max(1, this.#sequence),
      occurredAt: new Date(this.#now()).toISOString(),
      kind: 'alert',
      subject: {
        id: 'stream',
        status: 'resync_required',
        code: 'WOML_OBSERVABILITY_STREAM_GAP',
      },
    };
  }

  #sse(event: OperationsStreamV1): Uint8Array {
    return encoder.encode(`id: ${event.sequence}\nevent: operations\ndata: ${JSON.stringify(event)}\n\n`);
  }

  #counterPrefix(prefix: string): number {
    let total = 0;
    for (const [key, value] of this.#counters) {
      if (key.startsWith(prefix)) total += value;
    }
    return total;
  }

  #remember(map: Map<string, string>, key: string, value: string): void {
    map.delete(key);
    map.set(key, value);
    if (map.size > 1000) map.delete(map.keys().next().value!);
  }
}
