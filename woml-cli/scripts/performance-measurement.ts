export const PERFORMANCE_MEASUREMENT_PROFILE =
  'woml.performance-measurement/v1' as const;
export const PERFORMANCE_CONTROL_PROFILE =
  'woml.performance-control/v1' as const;
export const PERFORMANCE_SIGNAL_PROFILE =
  'woml.performance-signal/v1' as const;

export type PerformanceMode = 'manual' | 'engine' | 'all';
export type PerformanceMetricName =
  | 'harness.process'
  | 'activation.cold'
  | 'manual.visible'
  | 'engine.durable';

export interface PerformanceSummaryV1 {
  readonly count: number;
  readonly minimumMs: number;
  readonly medianMs: number;
  readonly p95Ms: number;
  readonly maximumMs: number;
  readonly madMs: number;
}

export interface PerformanceMetricV1 {
  readonly name: PerformanceMetricName;
  readonly temperature: 'cold' | 'warm' | 'calibration';
  readonly unit: 'ms';
  readonly boundary: {
    readonly start: string;
    readonly end: string;
  };
  readonly samples: readonly number[];
  readonly summary: PerformanceSummaryV1;
}

export interface PerformanceMeasurementV1 {
  readonly profile: typeof PERFORMANCE_MEASUREMENT_PROFILE;
  readonly createdAt: string;
  readonly fixture: {
    readonly path: string;
    readonly sha256: string;
  };
  readonly environment: {
    readonly platform: string;
    readonly architecture: string;
    readonly bunVersion: string;
    readonly cpuModel: string;
    readonly logicalCpuCount: number;
    readonly memoryBytes: number;
    readonly nativeBuild: 'release' | 'debug' | 'unknown';
    readonly cliArtifact: 'built' | 'source';
  };
  readonly parameters: {
    readonly mode: PerformanceMode;
    readonly warmups: number;
    readonly iterations: number;
  };
  readonly metrics: readonly PerformanceMetricV1[];
}

export type PerformanceControlV1 =
  | {
      readonly profile: typeof PERFORMANCE_CONTROL_PROFILE;
      readonly type: 'trigger';
      readonly requestId: string;
    }
  | {
      readonly profile: typeof PERFORMANCE_CONTROL_PROFILE;
      readonly type: 'stop';
    };

export type PerformanceSignalV1 =
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'child_ready';
    }
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'runtime_ready';
      readonly runtimeInstanceId: string;
      readonly workflowCount: number;
    }
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'run_terminal';
      readonly requestId: string;
      readonly runId: string;
      readonly status: string;
    }
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'engine_samples';
      readonly samples: readonly number[];
    }
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'stopped';
      readonly exitCode: number;
    }
  | {
      readonly profile: typeof PERFORMANCE_SIGNAL_PROFILE;
      readonly type: 'error';
      readonly message: string;
    };

function median(sorted: readonly number[]): number {
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1]! + sorted[middle]!) / 2
    : sorted[middle]!;
}

export function summarizeSamples(
  samples: readonly number[]
): PerformanceSummaryV1 {
  if (
    samples.length === 0 ||
    samples.some(sample => !Number.isFinite(sample) || sample < 0)
  ) {
    throw new Error('Performance samples must contain finite non-negative values.');
  }
  const sorted = [...samples].sort((left, right) => left - right);
  const medianMs = median(sorted);
  const deviations = sorted
    .map(sample => Math.abs(sample - medianMs))
    .sort((left, right) => left - right);
  const p95Index = Math.max(0, Math.ceil(sorted.length * 0.95) - 1);
  return {
    count: sorted.length,
    minimumMs: sorted[0]!,
    medianMs,
    p95Ms: sorted[p95Index]!,
    maximumMs: sorted[sorted.length - 1]!,
    madMs: median(deviations),
  };
}

export function performanceMetric(
  name: PerformanceMetricName,
  temperature: PerformanceMetricV1['temperature'],
  boundary: PerformanceMetricV1['boundary'],
  samples: readonly number[]
): PerformanceMetricV1 {
  return {
    name,
    temperature,
    unit: 'ms',
    boundary,
    samples,
    summary: summarizeSamples(samples),
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function decodePerformanceSignal(line: string): PerformanceSignalV1 | undefined {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (!record(value) || value.profile !== PERFORMANCE_SIGNAL_PROFILE) {
    return undefined;
  }
  if (value.type === 'child_ready') {
    return { profile: PERFORMANCE_SIGNAL_PROFILE, type: 'child_ready' };
  }
  if (
    value.type === 'runtime_ready' &&
    typeof value.runtimeInstanceId === 'string' &&
    Number.isSafeInteger(value.workflowCount)
  ) {
    return value as PerformanceSignalV1;
  }
  if (
    value.type === 'run_terminal' &&
    typeof value.requestId === 'string' &&
    typeof value.runId === 'string' &&
    typeof value.status === 'string'
  ) {
    return value as PerformanceSignalV1;
  }
  if (
    value.type === 'engine_samples' &&
    Array.isArray(value.samples) &&
    value.samples.length > 0 &&
    value.samples.every(sample =>
      typeof sample === 'number' && Number.isFinite(sample) && sample >= 0
    )
  ) {
    return value as PerformanceSignalV1;
  }
  if (value.type === 'stopped' && Number.isSafeInteger(value.exitCode)) {
    return value as PerformanceSignalV1;
  }
  if (value.type === 'error' && typeof value.message === 'string') {
    return value as PerformanceSignalV1;
  }
  throw new Error('The performance child emitted an invalid protocol signal.');
}

export function formatPerformanceMeasurement(
  measurement: PerformanceMeasurementV1
): string {
  const lines = [
    'WOML PERFORMANCE BASELINE v1',
    '',
    `Fixture     ${measurement.fixture.path}`,
    `Mode        ${measurement.parameters.mode}`,
    `Samples     ${measurement.parameters.iterations} measured · ${measurement.parameters.warmups} warm-up`,
    `Runtime     Bun ${measurement.environment.bunVersion} · ${measurement.environment.platform}/${measurement.environment.architecture}`,
    '',
  ];
  const labels: Record<PerformanceMetricName, string> = {
    'harness.process': 'Harness process',
    'activation.cold': 'Cold activation',
    'manual.visible': 'Enter to result',
    'engine.durable': 'Durable engine',
  };
  for (const metric of measurement.metrics) {
    const summary = metric.summary;
    lines.push(
      `${labels[metric.name].padEnd(18)} median ${summary.medianMs.toFixed(2).padStart(8)} ms · p95 ${summary.p95Ms.toFixed(2).padStart(8)} ms · MAD ${summary.madMs.toFixed(2).padStart(7)} ms`
    );
  }
  lines.push('');
  if (measurement.metrics.some(metric => metric.name === 'harness.process')) {
    lines.push(
      'Calibration is reported separately and is never subtracted from WOML timings.'
    );
  }
  lines.push('Use --json to save the versioned artifact with raw samples.');
  return `${lines.join('\n')}\n`;
}
