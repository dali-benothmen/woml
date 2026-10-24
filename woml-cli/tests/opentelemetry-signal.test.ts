import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  mapTelemetrySignalToSpan,
  parseTelemetryAdmissionLink,
  parseTelemetrySignal,
  womlSpanIdV1,
  womlTraceIdV1,
} from '../src/opentelemetry-signal';
import { OpenTelemetryRuntime } from '../src/opentelemetry-runtime';
import {
  metricFactsForTelemetrySignal,
  prometheusIdentity,
  safeLogForTelemetrySignal,
  TelemetryActiveGaugeState,
} from '../src/opentelemetry-observation';

const fixture = resolve(
  import.meta.dir,
  'fixtures/opentelemetry/contracts.v1.json'
);

describe('OpenTelemetry Signal v1', () => {
  test('strictly decodes the frozen cross-language fixture', async () => {
    const contracts = await Bun.file(fixture).json();
    const signal = parseTelemetrySignal(JSON.stringify(contracts.signal));
    expect(signal.kind).toBe('attempt');
    expect(signal.durationMs).toBe(125);
    expect(womlTraceIdV1(signal.runId!)).toBe(
      contracts.traceIdentities.traceId
    );
    expect(
      womlSpanIdV1(
        signal.runId!,
        'attempt',
        `${signal.nodeId}:${signal.attempt}:${signal.invocationId}`
      )
    ).toBe(contracts.traceIdentities.attemptSpanId);
  });

  test('maps a completed durable interval to one deterministic span', async () => {
    const contracts = await Bun.file(fixture).json();
    const signal = parseTelemetrySignal(JSON.stringify(contracts.signal));
    expect(mapTelemetrySignalToSpan(signal)).toEqual({
      name: 'woml.step',
      traceId: '71bbd5822afe17d9d00bf8eaa62854b3',
      spanId: '8a857c39a28e38ba',
      parentSpanId: '0868430ecec5cefe',
      startTime: '2026-10-23T12:00:00.000Z',
      endTime: '2026-10-23T12:00:00.125Z',
      status: 'ok',
      attributes: {
        'woml.signal.id': 'telemetry_run_01HXTRACE_settled',
        'woml.signal.kind': 'attempt',
        'woml.durable.status': 'succeeded',
        'woml.deployment.id': 'deployment_01HX',
        'woml.runtime.instance.id': 'runtime_01HX',
        'woml.workflow.id': 'checkout',
        'woml.run.id': 'run_01HXTRACE',
        'woml.node.id': 'prepare',
        'woml.attempt': 1,
        'woml.invocation.id': 'inv_01',
        'handler.category': 'runtime.script',
        'node.kind': 'step',
        'retry.scheduled': false,
      },
    });
    const { endedAt: _endedAt, ...withoutEnd } = signal;
    expect(
      mapTelemetrySignalToSpan({ ...withoutEnd, status: 'waiting' })
    ).toBeUndefined();
  });

  test('rejects future, oversized, non-finite, unknown, and value-bearing records', async () => {
    const contracts = await Bun.file(fixture).json();
    const base = contracts.signal;
    const invalid = [
      { ...base, profile: 'woml.telemetry-signal/v2' },
      { ...base, durationMs: 1e400 },
      { ...base, payload: { token: 'forbidden' } },
      { ...base, attributes: { ...base.attributes, custom: 'forbidden' } },
      { ...base, signalId: 'x'.repeat(321) },
    ];
    for (const value of invalid) {
      expect(() => parseTelemetrySignal(JSON.stringify(value))).toThrow();
    }
    expect(() => parseTelemetrySignal(' '.repeat(16 * 1024 + 1))).toThrow(
      'oversized telemetry'
    );
  });

  test('hands accepted records to the lazy SDK without making export authoritative', async () => {
    const contracts = await Bun.file(fixture).json();
    const signal = parseTelemetrySignal(JSON.stringify(contracts.signal));
    const accepted: string[] = [];
    const runtime = await OpenTelemetryRuntime.start(
      contracts.resolvedConfiguration,
      {
        runtimeInstanceId: 'runtime_01HX',
        deploymentId: 'deployment_01HX',
        runtimeVersion: '1.0.10',
        environment: {
          OTEL_EXPORTER_OTLP_HEADERS: 'authorization=Bearer%20test',
        },
        loadSdk: async () => ({
          emit: value => {
            accepted.push(value.signalId);
            return true;
          },
          forceFlush: async () => {},
          shutdown: async () => {},
        }),
      }
    );
    expect(runtime.record(signal)).toBe(true);
    await Promise.resolve();
    expect(accepted).toEqual(['telemetry_run_01HXTRACE_settled']);
    await runtime.shutdown();
  });

  test('strictly accepts safe admission links and rejects malformed context', () => {
    const link = parseTelemetryAdmissionLink(
      JSON.stringify({
        profile: 'woml.telemetry-admission-link/v1',
        occurredAt: '2026-10-23T12:00:00.000Z',
        runtimeInstanceId: 'runtime_01',
        deploymentId: 'deployment_01',
        runId: 'run_01',
        status: 'linked',
        traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
        spanId: '00f067aa0ba902b7',
        sampled: true,
      })
    );
    expect(link.status).toBe('linked');
    expect(() =>
      parseTelemetryAdmissionLink(
        JSON.stringify({ ...link, traceId: '0'.repeat(32) })
      )
    ).toThrow('invalid telemetry context');
    expect(() =>
      parseTelemetryAdmissionLink(
        JSON.stringify({ ...link, baggage: 'secret=value' })
      )
    ).toThrow('invalid telemetry context');
  });

  test('uses one low-cardinality fact model for OTel and Prometheus identities', async () => {
    const contracts = await Bun.file(fixture).json();
    const signal = parseTelemetrySignal(JSON.stringify(contracts.signal));
    const facts = metricFactsForTelemetrySignal(signal);
    expect(facts.map(fact => fact.name)).toEqual([
      'woml.runtime.step.attempts',
      'woml.runtime.step.duration',
    ]);
    expect(prometheusIdentity(facts[0]!)).toBe(
      'woml_runtime_step_attempts{handler_category="runtime.script",status="succeeded"}'
    );
    const encoded = JSON.stringify(facts);
    expect(encoded).not.toContain(signal.runId);
    expect(encoded).not.toContain(signal.nodeId);
    expect(encoded).not.toContain(signal.invocationId);
  });

  test('tracks active gauges without exporting identity and emits only safe correlated logs', async () => {
    const contracts = await Bun.file(fixture).json();
    const settled = parseTelemetrySignal(JSON.stringify(contracts.signal));
    const active = new TelemetryActiveGaugeState();
    active.observe({
      ...settled,
      kind: 'run',
      status: 'started',
      endedAt: undefined,
      durationMs: undefined,
    });
    expect(active.facts()[0]).toMatchObject({ value: 1 });
    active.observe({ ...settled, kind: 'run' });
    expect(active.facts()[0]).toMatchObject({ value: 0 });
    const log = safeLogForTelemetrySignal(settled)!;
    expect(log.traceId).toBe('71bbd5822afe17d9d00bf8eaa62854b3');
    expect(log.spanId).toBe('8a857c39a28e38ba');
    expect(JSON.stringify(log)).not.toContain(settled.runId);
    expect(JSON.stringify(log)).not.toContain(settled.nodeId);
    expect(JSON.stringify(log)).not.toContain(settled.invocationId);
  });

  test('exports bounded remote queue, duration, loss, and ambiguity facts without worker dimensions', async () => {
    const contracts = await Bun.file(fixture).json();
    const base = contracts.signal;
    const queue = parseTelemetrySignal(JSON.stringify({
      ...base,
      signalId: 'telemetry_remote_queue',
      kind: 'remote_dispatch',
      status: 'started',
      durationMs: 42,
      attributes: { 'remote.phase': 'queue' },
    }));
    expect(metricFactsForTelemetrySignal(queue)).toEqual([expect.objectContaining({
      name: 'woml.runtime.remote.dispatch.queue_delay', value: 42, attributes: {},
    })]);
    const ambiguous = parseTelemetrySignal(JSON.stringify({
      ...base,
      signalId: 'telemetry_remote_ambiguous',
      kind: 'remote_dispatch',
      status: 'degraded',
      code: 'WOML_REMOTE_DISPATCH_AMBIGUOUS',
      durationMs: 125,
      attributes: { 'remote.phase': 'execution', 'remote.outcome': 'ambiguous' },
    }));
    const facts = metricFactsForTelemetrySignal(ambiguous);
    expect(facts.map(fact => fact.name)).toEqual([
      'woml.runtime.remote.dispatches',
      'woml.runtime.remote.execution.duration',
      'woml.runtime.remote.ambiguities',
      'woml.runtime.remote.losses',
    ]);
    expect(JSON.stringify(facts)).not.toContain('production-runner');
    expect(JSON.stringify(facts)).not.toContain('production-private');
  });
});
