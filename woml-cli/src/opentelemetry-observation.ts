import {
  mapTelemetrySignalToSpan,
  type TelemetrySignalV1,
} from './opentelemetry-signal';

export type TelemetryMetricInstrument = 'counter' | 'histogram';

export interface TelemetryMetricFactV1 {
  readonly name: string;
  readonly instrument: TelemetryMetricInstrument;
  readonly value: number;
  readonly unit: '1' | 'ms';
  readonly attributes: Readonly<Record<string, string>>;
}

export interface TelemetryGaugeFactV1 {
  readonly name:
    | 'woml.runtime.active_runs'
    | 'woml.runtime.active_waits'
    | 'woml.runtime.active_approvals'
    | 'woml.runtime.active_locks';
  readonly value: number;
  readonly unit: '1';
}

export interface TelemetryLogRecordV1 {
  readonly body: string;
  readonly timestamp: string;
  readonly severityText: 'INFO' | 'WARN' | 'ERROR';
  readonly severityNumber: 9 | 13 | 17;
  readonly traceId?: string;
  readonly spanId?: string;
  readonly attributes: Readonly<Record<string, string | number | boolean>>;
}

const terminal = new Set([
  'succeeded',
  'failed',
  'cancelled',
  'timed_out',
  'resolved',
  'released',
  'degraded',
  'stopped',
]);

function selectedAttributes(
  signal: TelemetrySignalV1,
  keys: readonly string[]
): Record<string, string> {
  const result: Record<string, string> = {};
  for (const key of keys) {
    const value = signal.attributes[key];
    if (typeof value === 'string') result[key] = value;
  }
  return result;
}

export function metricFactsForTelemetrySignal(
  signal: TelemetrySignalV1
): readonly TelemetryMetricFactV1[] {
  const facts: TelemetryMetricFactV1[] = [];
  const duration = signal.durationMs;
  if (signal.kind === 'run' && terminal.has(signal.status)) {
    const attributes = {
      status: signal.status,
      ...selectedAttributes(signal, ['trigger.type']),
    };
    facts.push({
      name: 'woml.runtime.runs',
      instrument: 'counter',
      value: 1,
      unit: '1',
      attributes,
    });
    if (duration !== undefined) {
      facts.push({
        name: 'woml.runtime.run.duration',
        instrument: 'histogram',
        value: duration,
        unit: 'ms',
        attributes: { status: signal.status },
      });
    }
  }
  if (signal.kind === 'attempt' && terminal.has(signal.status)) {
    const attributes = {
      status: signal.status,
      ...selectedAttributes(signal, ['handler.category']),
    };
    facts.push({
      name: 'woml.runtime.step.attempts',
      instrument: 'counter',
      value: 1,
      unit: '1',
      attributes,
    });
    if (duration !== undefined) {
      facts.push({
        name: 'woml.runtime.step.duration',
        instrument: 'histogram',
        value: duration,
        unit: 'ms',
        attributes,
      });
    }
  }
  if (signal.kind === 'retry') {
    facts.push({
      name: 'woml.runtime.retries',
      instrument: 'counter',
      value: 1,
      unit: '1',
      attributes: { outcome: signal.status },
    });
  }
  if (signal.kind === 'capability' && terminal.has(signal.status)) {
    const attributes = {
      status: signal.status,
      ...selectedAttributes(signal, ['capability.category']),
    };
    facts.push({
      name: 'woml.runtime.capability.calls',
      instrument: 'counter',
      value: 1,
      unit: '1',
      attributes,
    });
    if (duration !== undefined) {
      facts.push({
        name: 'woml.runtime.capability.duration',
        instrument: 'histogram',
        value: duration,
        unit: 'ms',
        attributes,
      });
    }
  }
  if (signal.kind === 'remote_dispatch') {
    const phase = signal.attributes['remote.phase'];
    const outcome = typeof signal.attributes['remote.outcome'] === 'string'
      ? signal.attributes['remote.outcome']
      : signal.status;
    if (phase === 'queue' && duration !== undefined) {
      facts.push({
        name: 'woml.runtime.remote.dispatch.queue_delay',
        instrument: 'histogram', value: duration, unit: 'ms', attributes: {},
      });
    } else if (phase === 'execution') {
      facts.push({
        name: 'woml.runtime.remote.dispatches',
        instrument: 'counter', value: 1, unit: '1', attributes: { outcome },
      });
      if (duration !== undefined) {
        facts.push({
          name: 'woml.runtime.remote.execution.duration',
          instrument: 'histogram', value: duration, unit: 'ms', attributes: { outcome },
        });
      }
      if (outcome === 'ambiguous') {
        facts.push({
          name: 'woml.runtime.remote.ambiguities',
          instrument: 'counter', value: 1, unit: '1', attributes: {},
        });
        facts.push({
          name: 'woml.runtime.remote.losses',
          instrument: 'counter', value: 1, unit: '1', attributes: {},
        });
      }
    }
  }
  return facts;
}

export class TelemetryActiveGaugeState {
  readonly #activeRuns = new Set<string>();
  readonly #activeWaits = new Set<string>();
  readonly #activeApprovals = new Set<string>();
  readonly #activeLocks = new Set<string>();

  observe(signal: TelemetrySignalV1): void {
    if (signal.runId === undefined) return;
    if (signal.kind === 'run') {
      if (signal.status === 'admitted' || signal.status === 'started') {
        this.#activeRuns.add(signal.runId);
      } else if (terminal.has(signal.status)) {
        this.#activeRuns.delete(signal.runId);
      }
      return;
    }
    const target =
      signal.kind === 'wait'
        ? this.#activeWaits
        : signal.kind === 'approval'
          ? this.#activeApprovals
          : signal.kind === 'lock'
            ? this.#activeLocks
            : undefined;
    if (target === undefined) return;
    const identity = `${signal.runId}:${signal.nodeId ?? ''}:${signal.invocationId ?? signal.signalId}`;
    if (signal.status === 'waiting' || signal.status === 'started') {
      target.add(identity);
    } else if (terminal.has(signal.status)) {
      target.delete(identity);
    }
  }

  facts(): readonly TelemetryGaugeFactV1[] {
    return [
      { name: 'woml.runtime.active_runs', value: this.#activeRuns.size, unit: '1' },
      { name: 'woml.runtime.active_waits', value: this.#activeWaits.size, unit: '1' },
      { name: 'woml.runtime.active_approvals', value: this.#activeApprovals.size, unit: '1' },
      { name: 'woml.runtime.active_locks', value: this.#activeLocks.size, unit: '1' },
    ];
  }
}

export function safeLogForTelemetrySignal(
  signal: TelemetrySignalV1
): TelemetryLogRecordV1 | undefined {
  if (!terminal.has(signal.status)) return undefined;
  const span = mapTelemetrySignalToSpan(signal);
  const failed = ['failed', 'timed_out', 'degraded'].includes(signal.status);
  const cancelled = signal.status === 'cancelled';
  return {
    body: `woml.${signal.kind}.${signal.status}`,
    timestamp: signal.endedAt ?? signal.occurredAt,
    severityText: failed ? 'ERROR' : cancelled ? 'WARN' : 'INFO',
    severityNumber: failed ? 17 : cancelled ? 13 : 9,
    ...(span === undefined
      ? {}
      : { traceId: span.traceId, spanId: span.spanId }),
    attributes: {
      'woml.signal.kind': signal.kind,
      'woml.durable.status': signal.status,
      ...(signal.code === undefined ? {} : { 'error.type': signal.code }),
      ...signal.attributes,
    },
  };
}

export function prometheusIdentity(fact: TelemetryMetricFactV1): string {
  const name = fact.name.replaceAll('.', '_');
  const labels = Object.entries(fact.attributes)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key.replaceAll('.', '_')}="${value}"`)
    .join(',');
  return `${name}${labels.length === 0 ? '' : `{${labels}}`}`;
}
