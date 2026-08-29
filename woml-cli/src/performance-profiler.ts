import { randomUUID } from 'node:crypto';
import { appendFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

export type PerformanceProcessV1 =
  | 'cli'
  | 'native'
  | 'rust'
  | 'host'
  | 'worker';
export type PerformanceLayerV1 =
  | 'cli'
  | 'compiler'
  | 'napi'
  | 'runtime'
  | 'sqlite'
  | 'host'
  | 'worker'
  | 'presentation';

export interface PerformanceMeasurements {
  readonly counts: Record<string, number>;
  readonly bytes: Record<string, number>;
  readonly identity: {
    runId?: string;
    invocationId?: string;
  };
}

interface PerformanceSpanV1 {
  readonly profile: 'woml.performance-span/v1';
  readonly traceId: string;
  readonly spanId: string;
  readonly parentSpanId?: string;
  readonly runId?: string;
  readonly invocationId?: string;
  readonly process: PerformanceProcessV1;
  readonly layer: PerformanceLayerV1;
  readonly name: string;
  readonly startOffsetMs: number;
  readonly durationMs: number;
  readonly status: 'succeeded' | 'failed' | 'cancelled';
  readonly counts?: Readonly<Record<string, number>>;
  readonly bytes?: Readonly<Record<string, number>>;
}

interface ProfilerState {
  readonly outputPath: string;
  readonly traceId: string;
  readonly process: PerformanceProcessV1;
  readonly spans: PerformanceSpanV1[];
  nextSpan: number;
  writtenSpans: number;
}

const profilerLoadedAt = performance.now();
const allowedProcesses = new Set<PerformanceProcessV1>([
  'cli',
  'native',
  'rust',
  'host',
  'worker',
]);

function configuredState(): ProfilerState | undefined {
  if (process.env.WOML_PROFILE !== '1') return undefined;
  const output = process.env.WOML_PROFILE_OUTPUT;
  if (output === undefined || output.length === 0) return undefined;
  const configuredProcess = process.env.WOML_PROFILE_PROCESS;
  const processName = allowedProcesses.has(configuredProcess as PerformanceProcessV1)
    ? configuredProcess as PerformanceProcessV1
    : 'cli';
  const configuredTrace = process.env.WOML_PROFILE_TRACE_ID;
  const traceId =
    configuredTrace !== undefined && /^[A-Za-z0-9_.:-]{1,128}$/u.test(configuredTrace)
      ? configuredTrace
      : `trace_${randomUUID().replaceAll('-', '')}`;
  return {
    outputPath: resolve(output),
    traceId,
    process: processName,
    spans: [],
    nextSpan: 1,
    writtenSpans: 0,
  };
}

const state = configuredState();

function validMeasurements(
  values: Readonly<Record<string, number>>
): Readonly<Record<string, number>> | undefined {
  const safe = Object.fromEntries(
    Object.entries(values).filter(
      ([name, value]) =>
        /^[a-z][a-z0-9_]*$/u.test(name) &&
        Number.isSafeInteger(value) &&
        value >= 0
    )
  );
  return Object.keys(safe).length === 0 ? undefined : safe;
}

function appendSpan(
  layer: PerformanceLayerV1,
  name: string,
  startedAt: number,
  status: PerformanceSpanV1['status'],
  measurements?: PerformanceMeasurements
): void {
  if (state === undefined) return;
  const counts = validMeasurements(measurements?.counts ?? {});
  const bytes = validMeasurements(measurements?.bytes ?? {});
  state.spans.push({
    profile: 'woml.performance-span/v1',
    traceId: state.traceId,
    spanId: `span_${process.pid}_${state.nextSpan++}`,
    process: state.process,
    layer,
    name,
    startOffsetMs: startedAt,
    durationMs: Math.max(0, performance.now() - startedAt),
    status,
    ...(measurements?.identity.runId === undefined
      ? {}
      : { runId: measurements.identity.runId }),
    ...(measurements?.identity.invocationId === undefined
      ? {}
      : { invocationId: measurements.identity.invocationId }),
    ...(counts === undefined ? {} : { counts }),
    ...(bytes === undefined ? {} : { bytes }),
  });
}

if (state !== undefined) {
  appendSpan(
    state.process === 'host'
      ? 'host'
      : state.process === 'worker'
        ? 'worker'
        : 'cli',
    `${state.process}.process_to_profiler`,
    0,
    'succeeded'
  );
  const bootstrap = state.spans[0]!;
  state.spans[0] = {
    ...bootstrap,
    durationMs: profilerLoadedAt,
  };
}

export function performanceProfileEnabled(): boolean {
  return state !== undefined;
}

export function performanceTraceId(): string | undefined {
  return state?.traceId;
}

export function profileSync<T>(
  layer: PerformanceLayerV1,
  name: string,
  operation: (measurements?: PerformanceMeasurements) => T
): T {
  if (state === undefined) return operation();
  const measurements: PerformanceMeasurements = { counts: {}, bytes: {}, identity: {} };
  const startedAt = performance.now();
  try {
    const result = operation(measurements);
    appendSpan(layer, name, startedAt, 'succeeded', measurements);
    return result;
  } catch (error) {
    appendSpan(layer, name, startedAt, 'failed', measurements);
    throw error;
  }
}

export async function profileAsync<T>(
  layer: PerformanceLayerV1,
  name: string,
  operation: (measurements?: PerformanceMeasurements) => Promise<T>
): Promise<T> {
  if (state === undefined) return await operation();
  const measurements: PerformanceMeasurements = { counts: {}, bytes: {}, identity: {} };
  const startedAt = performance.now();
  try {
    const result = await operation(measurements);
    appendSpan(layer, name, startedAt, 'succeeded', measurements);
    return result;
  } catch (error) {
    appendSpan(layer, name, startedAt, 'failed', measurements);
    throw error;
  }
}

export async function flushPerformanceProfile(): Promise<void> {
  if (state === undefined || state.writtenSpans >= state.spans.length) return;
  const unwritten = state.spans.slice(state.writtenSpans);
  await mkdir(dirname(state.outputPath), { recursive: true });
  await appendFile(
    state.outputPath,
    `${unwritten.map(span => JSON.stringify(span)).join('\n')}\n`,
    { encoding: 'utf8', mode: 0o600 }
  );
  state.writtenSpans += unwritten.length;
}
