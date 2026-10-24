import { resourceFromAttributes } from '@opentelemetry/resources';
import {
  ROOT_CONTEXT,
  SpanKind,
  SpanStatusCode,
  TraceFlags,
  trace,
  type SpanAttributes,
} from '@opentelemetry/api';
import {
  AlwaysOffSampler,
  AlwaysOnSampler,
  BatchSpanProcessor,
  ParentBasedSampler,
  TraceIdRatioBasedSampler,
  TracerProvider,
  RandomIdGenerator,
  type IdGenerator,
  type Sampler,
} from '@opentelemetry/sdk-trace';
import {
  MeterProvider,
  PeriodicExportingMetricReader,
} from '@opentelemetry/sdk-metrics';
import {
  BatchLogRecordProcessor,
  LoggerProvider,
} from '@opentelemetry/sdk-logs';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-proto';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-proto';
import { OTLPLogExporter } from '@opentelemetry/exporter-logs-otlp-proto';

import type {
  OpenTelemetryExporterEvent,
  OpenTelemetrySdkHandle,
  OpenTelemetrySdkStartOptions,
} from './opentelemetry-runtime';
import {
  mapTelemetrySignalToSpan,
  type TelemetryAdmissionLinkV1,
  type TelemetrySignalV1,
  type TelemetrySpanV1,
} from './opentelemetry-signal';
import {
  metricFactsForTelemetrySignal,
  safeLogForTelemetrySignal,
  TelemetryActiveGaugeState,
} from './opentelemetry-observation';

class DeterministicIdGenerator implements IdGenerator {
  readonly #fallback = new RandomIdGenerator();
  #next?: { readonly traceId: string; readonly spanId: string };

  withIds<T>(traceId: string, spanId: string, action: () => T): T {
    this.#next = { traceId, spanId };
    try {
      return action();
    } finally {
      this.#next = undefined;
    }
  }

  generateTraceId(): string {
    return this.#next?.traceId ?? this.#fallback.generateTraceId();
  }

  generateSpanId(): string {
    return this.#next?.spanId ?? this.#fallback.generateSpanId();
  }
}

function mergedHeaders(
  common: Readonly<Record<string, string>>,
  signal: Readonly<Record<string, string>>
): Record<string, string> {
  return { ...common, ...signal };
}

function sampler(
  configuration: OpenTelemetrySdkStartOptions['configuration']
): Sampler {
  if (configuration.sampler.name === 'always_on') return new AlwaysOnSampler();
  if (configuration.sampler.name === 'always_off') return new AlwaysOffSampler();
  if (configuration.sampler.name !== 'parentbased_traceidratio') {
    return new AlwaysOffSampler();
  }
  return new ParentBasedSampler({
    root: new TraceIdRatioBasedSampler(configuration.sampler.ratio),
  });
}

interface ExportResultLike {
  readonly code: number;
}

interface ExporterLike<T> {
  export(value: T, callback: (result: ExportResultLike) => void): void;
  forceFlush?(): Promise<void>;
  shutdown(): Promise<void>;
}

const EXPORT_SUCCESS = 0;
const EXPORT_FAILURE = 1;
const EXPORT_RETRY_DELAYS_MS = [100, 250] as const;
const EXPORT_CIRCUIT_FAILURE_THRESHOLD = 3;
const EXPORT_CIRCUIT_OPEN_MS = 5_000;

export function governOpenTelemetryExporter<T extends object, TValue>(
  delegate: T & ExporterLike<TValue>,
  options: {
    readonly signal: OpenTelemetryExporterEvent['signal'];
    readonly timeoutMs: number;
    readonly onEvent?: (event: OpenTelemetryExporterEvent) => void;
    readonly now?: () => number;
    readonly schedule?: (
      action: () => void,
      delayMs: number
    ) => ReturnType<typeof setTimeout>;
  }
): T & ExporterLike<TValue> {
  const now = options.now ?? Date.now;
  const schedule = options.schedule ?? setTimeout;
  let consecutiveFailures = 0;
  let circuitOpenUntil = 0;
  const publish = (
    kind: OpenTelemetryExporterEvent['kind'],
    fields: Pick<OpenTelemetryExporterEvent, 'code' | 'retryAt'> = {}
  ): void => {
    options.onEvent?.({
      kind,
      signal: options.signal,
      occurredAt: new Date(now()).toISOString(),
      ...fields,
    });
  };

  const governedExport = (
    value: TValue,
    callback: (result: ExportResultLike) => void
  ): void => {
    const startedAt = now();
    const deadline = startedAt + options.timeoutMs;
    if (circuitOpenUntil > startedAt) {
      publish('export_dropped', { code: 'WOML_OTEL_CIRCUIT_OPEN' });
      callback({ code: EXPORT_FAILURE });
      return;
    }
    if (circuitOpenUntil > 0) {
      publish('circuit_half_open');
      circuitOpenUntil = 0;
    }

    let settled = false;
    const finish = (
      result: ExportResultLike,
      kind: 'export_succeeded' | 'export_failed' | 'export_timeout'
    ): void => {
      if (settled) return;
      settled = true;
      if (kind === 'export_succeeded') {
        const recovered = consecutiveFailures > 0;
        consecutiveFailures = 0;
        publish(kind);
        if (recovered) publish('circuit_closed');
      } else {
        consecutiveFailures += 1;
        publish(kind, {
          code:
            kind === 'export_timeout'
              ? 'WOML_OTEL_EXPORT_TIMEOUT'
              : 'WOML_OTEL_EXPORT_FAILED',
        });
        if (consecutiveFailures >= EXPORT_CIRCUIT_FAILURE_THRESHOLD) {
          circuitOpenUntil = now() + EXPORT_CIRCUIT_OPEN_MS;
          publish('circuit_opened', {
            code: 'WOML_OTEL_CIRCUIT_OPEN',
            retryAt: new Date(circuitOpenUntil).toISOString(),
          });
        }
      }
      callback(result);
    };

    const attempt = (attemptIndex: number): void => {
      if (settled) return;
      const remaining = deadline - now();
      if (remaining <= 0) {
        finish({ code: EXPORT_FAILURE }, 'export_timeout');
        return;
      }
      let attemptSettled = false;
      const timer = schedule(() => {
        if (attemptSettled || settled) return;
        attemptSettled = true;
        finish({ code: EXPORT_FAILURE }, 'export_timeout');
      }, remaining);
      // Timeout and retry timers are the only authorities able to settle an
      // exporter that never invokes its callback. They must remain referenced
      // while the export is active, especially under Bun on Windows.
      try {
        delegate.export(value, result => {
          if (attemptSettled || settled) return;
          attemptSettled = true;
          clearTimeout(timer);
          if (result.code === EXPORT_SUCCESS) {
            finish(result, 'export_succeeded');
            return;
          }
          const delay = EXPORT_RETRY_DELAYS_MS[attemptIndex];
          if (delay !== undefined && now() + delay < deadline) {
            const retryAt = now() + delay;
            publish('retry_scheduled', {
              code: 'WOML_OTEL_EXPORT_RETRY',
              retryAt: new Date(retryAt).toISOString(),
            });
            schedule(() => attempt(attemptIndex + 1), delay);
            return;
          }
          finish(result, 'export_failed');
        });
      } catch {
        clearTimeout(timer);
        attemptSettled = true;
        finish({ code: EXPORT_FAILURE }, 'export_failed');
      }
    };
    attempt(0);
  };

  return new Proxy(delegate, {
    get(target, property, receiver) {
      if (property === 'export') return governedExport;
      const value = Reflect.get(target, property, receiver);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
}

async function settleAll(
  operations: readonly (() => Promise<void>)[]
): Promise<void> {
  const results = await Promise.allSettled(operations.map(operation => operation()));
  const failures = results.filter(
    (result): result is PromiseRejectedResult => result.status === 'rejected'
  );
  if (failures.length > 0) {
    throw new AggregateError(
      failures.map(failure => failure.reason),
      'One or more OpenTelemetry providers failed to settle.'
    );
  }
}

export async function startOpenTelemetrySdk(
  options: OpenTelemetrySdkStartOptions
): Promise<OpenTelemetrySdkHandle> {
  const { configuration, secrets } = options;
  const resource = resourceFromAttributes({
    'service.name': configuration.serviceName,
    'service.version': options.resource.runtimeVersion,
    'service.instance.id': options.resource.runtimeInstanceId,
    'woml.deployment.id': options.resource.deploymentId,
    'woml.runtime.version': options.resource.runtimeVersion,
    ...configuration.resourceAttributes,
  });
  const providers: {
    forceFlush(): Promise<void>;
    shutdown(): Promise<void>;
  }[] = [];
  let emitTrace: ((signal: TelemetrySignalV1) => boolean) | undefined;
  let emitMetric: ((signal: TelemetrySignalV1) => boolean) | undefined;
  let emitLog: ((signal: TelemetrySignalV1) => boolean) | undefined;
  let recordPropagationContext: ((status: 'linked' | 'invalid') => void) | undefined;
  const admissionLinks = new Map<string, TelemetryAdmissionLinkV1>();
  const seenSignals = new Set<string>();

  if (configuration.signals.traces) {
    const exporter = governOpenTelemetryExporter(
      new OTLPTraceExporter({
        url: configuration.endpoints.traces,
        headers: mergedHeaders(secrets.headers.common, secrets.headers.traces),
        timeoutMillis: configuration.exportTimeoutMs,
        concurrencyLimit: 1,
      }),
      {
        signal: 'traces',
        timeoutMs: configuration.exportTimeoutMs,
        onEvent: options.onExportEvent,
      }
    );
    const idGenerator = new DeterministicIdGenerator();
    const provider = new TracerProvider({
        resource,
        sampler: sampler(configuration),
        idGenerator,
        spanLimits: {
          attributeCountLimit: 32,
          attributeValueLengthLimit: 256,
          eventCountLimit: 16,
          linkCountLimit: 16,
          attributePerEventCountLimit: 16,
          attributePerLinkCountLimit: 8,
        },
        spanProcessors: [
          new BatchSpanProcessor({
            exporter,
            maxQueueSize: configuration.maxQueueSize,
            maxExportBatchSize: configuration.maxExportBatchSize,
            scheduledDelayMillis: configuration.exportIntervalMs,
            exportTimeoutMillis: configuration.exportTimeoutMs,
          }),
        ],
      });
    providers.push(provider);
    const tracer = provider.getTracer('woml.runtime', options.resource.runtimeVersion);
    emitTrace = signal => {
      const mapped = mapTelemetrySignalToSpan(signal);
      if (mapped === undefined) return true;
      return idGenerator.withIds(mapped.traceId, mapped.spanId, () => {
        const parent = spanParentContext(mapped, configuration);
        const span = tracer.startSpan(
          mapped.name,
          {
            kind: SpanKind.INTERNAL,
            startTime: new Date(mapped.startTime),
            attributes: mapped.attributes as SpanAttributes,
            ...(mapped.name !== 'woml.run'
              ? {}
              : {
                  links: admissionLinkContexts(
                    admissionLinks.get(signal.runId!)
                  ),
                }),
          },
          parent
        );
        span.setStatus({
          code:
            mapped.status === 'error'
              ? SpanStatusCode.ERROR
              : mapped.status === 'ok'
                ? SpanStatusCode.OK
                : SpanStatusCode.UNSET,
        });
        span.end(new Date(mapped.endTime));
        if (mapped.name === 'woml.run') admissionLinks.delete(signal.runId!);
        return true;
      });
    };
  }

  if (configuration.signals.metrics) {
    const exporter = governOpenTelemetryExporter(
      new OTLPMetricExporter({
        url: configuration.endpoints.metrics,
        headers: mergedHeaders(secrets.headers.common, secrets.headers.metrics),
        timeoutMillis: configuration.exportTimeoutMs,
        concurrencyLimit: 1,
      }),
      {
        signal: 'metrics',
        timeoutMs: configuration.exportTimeoutMs,
        onEvent: options.onExportEvent,
      }
    );
    const provider = new MeterProvider({
        resource,
        readers: [
          new PeriodicExportingMetricReader({
            exporter,
            exportIntervalMillis: configuration.exportIntervalMs,
            exportTimeoutMillis: configuration.exportTimeoutMs,
          }),
        ],
      });
    providers.push(provider);
    const meter = provider.getMeter('woml.runtime', options.resource.runtimeVersion);
    const instruments = {
      runs: meter.createCounter('woml.runtime.runs', { unit: '1' }),
      runDuration: meter.createHistogram('woml.runtime.run.duration', { unit: 'ms' }),
      stepAttempts: meter.createCounter('woml.runtime.step.attempts', { unit: '1' }),
      stepDuration: meter.createHistogram('woml.runtime.step.duration', { unit: 'ms' }),
      retries: meter.createCounter('woml.runtime.retries', { unit: '1' }),
      capabilityCalls: meter.createCounter('woml.runtime.capability.calls', { unit: '1' }),
      capabilityDuration: meter.createHistogram('woml.runtime.capability.duration', { unit: 'ms' }),
      propagationContexts: meter.createCounter('woml.runtime.propagation.contexts', { unit: '1' }),
      remoteDispatches: meter.createCounter('woml.runtime.remote.dispatches', { unit: '1' }),
      remoteQueueDelay: meter.createHistogram('woml.runtime.remote.dispatch.queue_delay', { unit: 'ms' }),
      remoteExecutionDuration: meter.createHistogram('woml.runtime.remote.execution.duration', { unit: 'ms' }),
      remoteLosses: meter.createCounter('woml.runtime.remote.losses', { unit: '1' }),
      remoteAmbiguities: meter.createCounter('woml.runtime.remote.ambiguities', { unit: '1' }),
    };
    const active = new TelemetryActiveGaugeState();
    for (const [name, gauge] of [
      ['woml.runtime.active_runs', meter.createObservableGauge('woml.runtime.active_runs', { unit: '1' })],
      ['woml.runtime.active_waits', meter.createObservableGauge('woml.runtime.active_waits', { unit: '1' })],
      ['woml.runtime.active_approvals', meter.createObservableGauge('woml.runtime.active_approvals', { unit: '1' })],
      ['woml.runtime.active_locks', meter.createObservableGauge('woml.runtime.active_locks', { unit: '1' })],
    ] as const) {
      gauge.addCallback(result => {
        const fact = active.facts().find(candidate => candidate.name === name);
        result.observe(fact?.value ?? 0);
      });
    }
    emitMetric = signal => {
      active.observe(signal);
      for (const fact of metricFactsForTelemetrySignal(signal)) {
        const attributes = fact.attributes as SpanAttributes;
        if (fact.name === 'woml.runtime.runs') instruments.runs.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.run.duration') instruments.runDuration.record(fact.value, attributes);
        else if (fact.name === 'woml.runtime.step.attempts') instruments.stepAttempts.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.step.duration') instruments.stepDuration.record(fact.value, attributes);
        else if (fact.name === 'woml.runtime.retries') instruments.retries.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.capability.calls') instruments.capabilityCalls.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.capability.duration') instruments.capabilityDuration.record(fact.value, attributes);
        else if (fact.name === 'woml.runtime.remote.dispatches') instruments.remoteDispatches.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.remote.dispatch.queue_delay') instruments.remoteQueueDelay.record(fact.value, attributes);
        else if (fact.name === 'woml.runtime.remote.execution.duration') instruments.remoteExecutionDuration.record(fact.value, attributes);
        else if (fact.name === 'woml.runtime.remote.losses') instruments.remoteLosses.add(fact.value, attributes);
        else if (fact.name === 'woml.runtime.remote.ambiguities') instruments.remoteAmbiguities.add(fact.value, attributes);
      }
      return true;
    };
    recordPropagationContext = status => {
      instruments.propagationContexts.add(1, { status });
    };
  }

  if (configuration.signals.logs) {
    const exporter = governOpenTelemetryExporter(
      new OTLPLogExporter({
        url: configuration.endpoints.logs,
        headers: mergedHeaders(secrets.headers.common, secrets.headers.logs),
        timeoutMillis: configuration.exportTimeoutMs,
        concurrencyLimit: 1,
      }),
      {
        signal: 'logs',
        timeoutMs: configuration.exportTimeoutMs,
        onEvent: options.onExportEvent,
      }
    );
    const provider = new LoggerProvider({
        resource,
        logRecordLimits: {
          attributeCountLimit: 32,
          attributeValueLengthLimit: 256,
        },
        processors: [
          new BatchLogRecordProcessor({
            exporter,
            maxQueueSize: configuration.maxQueueSize,
            maxExportBatchSize: configuration.maxExportBatchSize,
            scheduledDelayMillis: configuration.exportIntervalMs,
            exportTimeoutMillis: configuration.exportTimeoutMs,
          }),
        ],
      });
    providers.push(provider);
    const logger = provider.getLogger('woml.runtime', options.resource.runtimeVersion);
    emitLog = signal => {
      const record = safeLogForTelemetrySignal(signal);
      if (record === undefined) return true;
      const correlation =
        record.traceId === undefined || record.spanId === undefined
          ? ROOT_CONTEXT
          : trace.setSpanContext(ROOT_CONTEXT, {
              traceId: record.traceId,
              spanId: record.spanId,
              traceFlags: sampled(record.traceId, configuration)
                ? TraceFlags.SAMPLED
                : TraceFlags.NONE,
              isRemote: false,
            });
      logger.emit({
        body: record.body,
        timestamp: new Date(record.timestamp),
        severityText: record.severityText,
        severityNumber: record.severityNumber,
        attributes: record.attributes,
        context: correlation,
      });
      return true;
    };
  }

  return {
    emit: signal => {
      if (seenSignals.has(signal.signalId)) return true;
      while (seenSignals.size >= configuration.maxQueueSize) {
        const oldest = seenSignals.values().next().value;
        if (oldest === undefined) break;
        seenSignals.delete(oldest);
      }
      seenSignals.add(signal.signalId);
      const traceAccepted = emitTrace?.(signal) ?? true;
      const metricAccepted = emitMetric?.(signal) ?? true;
      const logAccepted = emitLog?.(signal) ?? true;
      return traceAccepted && metricAccepted && logAccepted;
    },
    link: link => {
      recordPropagationContext?.(link.status);
      if (link.status === 'invalid') return true;
      if (admissionLinks.size >= configuration.maxQueueSize) return false;
      if (!admissionLinks.has(link.runId)) admissionLinks.set(link.runId, link);
      return true;
    },
    forceFlush: async () => {
      await settleAll(providers.map(provider => () => provider.forceFlush()));
    },
    shutdown: async () => {
      await settleAll(providers.map(provider => () => provider.shutdown()));
    },
  };
}

function admissionLinkContexts(
  link: TelemetryAdmissionLinkV1 | undefined
) {
  if (
    link?.status !== 'linked' ||
    link.traceId === undefined ||
    link.spanId === undefined
  ) {
    return [];
  }
  return [
    {
      context: {
        traceId: link.traceId,
        spanId: link.spanId,
        traceFlags: link.sampled ? TraceFlags.SAMPLED : TraceFlags.NONE,
        isRemote: true,
      },
    },
  ];
}

function spanParentContext(
  span: TelemetrySpanV1,
  configuration: OpenTelemetrySdkStartOptions['configuration']
) {
  if (span.parentSpanId === undefined) return ROOT_CONTEXT;
  return trace.setSpanContext(ROOT_CONTEXT, {
    traceId: span.traceId,
    spanId: span.parentSpanId,
    traceFlags: sampled(span.traceId, configuration)
      ? TraceFlags.SAMPLED
      : TraceFlags.NONE,
    isRemote: false,
  });
}

function sampled(
  traceId: string,
  configuration: OpenTelemetrySdkStartOptions['configuration']
): boolean {
  if (configuration.sampler.name === 'always_on') return true;
  if (configuration.sampler.name === 'always_off') return false;
  if (configuration.sampler.name !== 'parentbased_traceidratio') return false;
  const prefix = Number.parseInt(traceId.slice(0, 13), 16);
  return prefix / 0x1_0000_0000_0000 < configuration.sampler.ratio;
}
