import { afterEach, describe, expect, test } from 'bun:test';
import Ajv2020 from 'ajv/dist/2020';

import { RuntimeObservability } from '../src/runtime-observability';
import { startRuntimeControl } from '../src/runtime-control';
import type { RustRunListV1 } from '../src/rust-executor';

const controls: { close(): Promise<void> }[] = [];

afterEach(async () => {
  await Promise.all(controls.splice(0).map(control => control.close()));
});

async function schema(name: string) {
  return JSON.parse(
    await Bun.file(new URL(`../../docs/schemas/${name}`, import.meta.url)).text()
  );
}

async function validators() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', {
    validate: (value: string) => Number.isFinite(Date.parse(value)),
  });
  return {
    snapshot: ajv.compile(await schema('runtime-operations-snapshot.v1.schema.json')),
    stream: ajv.compile(await schema('runtime-operations-stream.v1.schema.json')),
    log: ajv.compile(await schema('runtime-log-record.v1.schema.json')),
    metric: ajv.compile(await schema('runtime-metrics.v1.schema.json')),
    health: ajv.compile(await schema('runtime-health.v1.schema.json')),
  };
}

const runs: RustRunListV1 = {
  profile: 'woml.run-list/v2',
  runs: [
    {
      runId: 'run_active',
      workflowId: 'orders',
      status: 'running',
      admittedAt: '2026-08-12T09:00:00.000Z',
      startedAt: '2026-08-12T09:00:00.010Z',
      updatedAt: '2026-08-12T09:00:00.020Z',
      queue: 'default',
    },
    {
      runId: 'run_waiting',
      workflowId: 'orders',
      status: 'waiting',
      admittedAt: '2026-08-12T09:00:01.000Z',
      startedAt: '2026-08-12T09:00:01.010Z',
      updatedAt: '2026-08-12T09:00:01.020Z',
      queue: 'default',
    },
  ],
};

function fixture(options: {
  logs?: string[];
  runs?: RustRunListV1;
  now?: () => number;
} = {}) {
  return new RuntimeObservability({
    runtimeInstanceId: 'runtime_observability',
    deploymentId: 'deployment_observability',
    workflows: [
      {
        workflowId: 'orders',
        definitionHash: `sha256:${'a'.repeat(64)}`,
        triggerTypes: ['webhook', 'schedule'],
      },
    ],
    listRuns: () => options.runs ?? runs,
    storeSize: async () => 4096,
    logFormat: 'json',
    emitLog: text => options.logs?.push(text),
    now: options.now,
    components: [
      { name: 'sqlite', kind: 'store', status: 'ready' },
      { name: 'script-host', kind: 'worker', status: 'ready' },
    ],
  });
}

function authorization(capability: string): HeadersInit {
  return { authorization: `Bearer ${capability}` };
}

describe('Frozen observability artifacts', () => {
  test('matches the reviewed golden snapshot, log, and Prometheus fixtures', async () => {
    const logs: string[] = [];
    const instant = Date.parse('2026-08-12T09:00:02.000Z');
    const observed = fixture({
      logs,
      runs: { profile: 'woml.run-list/v2', runs: [] },
      now: () => instant,
    });
    observed.setLifecycle('ready');
    observed.log('info', 'WOML_RUNTIME_READY', 'Runtime is ready.');

    expect(await observed.snapshot()).toEqual(
      await Bun.file(
        new URL('./fixtures/production-runtime/runtime-snapshot.v1.json', import.meta.url)
      ).json()
    );
    expect(JSON.parse(logs.at(-1)!)).toEqual(
      await Bun.file(
        new URL('./fixtures/production-runtime/runtime-log.v1.json', import.meta.url)
      ).json()
    );
    expect(await observed.prometheusMetrics()).toBe(
      await Bun.file(
        new URL('./fixtures/production-runtime/runtime-metrics.prom', import.meta.url)
      ).text()
    );
  });

  test('produces bounded redacted snapshot, health, logs, and metrics', async () => {
    const validate = await validators();
    const logs: string[] = [];
    const observed = fixture({ logs });
    observed.setLifecycle('ready');
    observed.recordProgress({
      contract: 'woml.trigger-progress',
      runId: 'run_active',
      status: 'accepted',
      payload: { token: 'secret-must-not-appear' },
      message: 'secret-must-not-appear',
    });
    observed.recordProgress({
      contract: 'woml.execution-progress',
      version: 1,
      type: 'for_each_progress',
      runId: 'run_active',
      forEachId: 'organize',
      status: 'running',
      total: 42,
      succeeded: 18,
      failed: 0,
      skipped: 0,
      active: 4,
      pending: 20,
      concurrency: 4,
      item: { token: 'secret-must-not-appear' },
    });
    observed.log('info', 'WOML_RUNTIME_READY', 'Runtime is ready.', {
      workflowId: 'orders',
    });

    const snapshot = await observed.snapshot();
    expect(validate.snapshot(snapshot), validate.snapshot.errors?.join('\n')).toBe(true);
    expect(JSON.stringify(snapshot)).not.toContain('secret-must-not-appear');
    expect(snapshot).toMatchObject({
      lifecycle: 'ready',
      ready: true,
      workflows: [{ workflowId: 'orders', active: 1, waiting: 1 }],
    });
    expect((snapshot as { runs: { runId: string; forEach?: unknown }[] }).runs
      .find(run => run.runId === 'run_active')).toMatchObject({
        forEach: [{
          forEachId: 'organize', total: 42, succeeded: 18, active: 4, pending: 20,
        }],
      });

    expect(validate.health(observed.minimalHealth('liveness'))).toBe(true);
    expect(validate.health(observed.minimalHealth('readiness'))).toBe(true);
    expect(validate.health(observed.detailedHealth())).toBe(true);

    const record = JSON.parse(logs.at(-1)!);
    expect(validate.log(record), validate.log.errors?.join('\n')).toBe(true);
    expect(record).not.toHaveProperty('payload');
    expect(record).not.toHaveProperty('context');

    const metrics = await observed.metrics();
    expect(metrics.length).toBeLessThan(100);
    for (const metric of metrics) {
      expect(validate.metric(metric), validate.metric.errors?.join('\n')).toBe(true);
      for (const forbidden of ['run_id', 'node_id', 'url', 'state_key']) {
        expect(JSON.stringify(metric)).not.toContain(forbidden);
      }
    }
    const prometheus = await observed.prometheusMetrics();
    expect(prometheus).toContain('# TYPE woml_runtime_ready gauge');
    expect(prometheus).toContain('woml_for_each_iterations_active 4');
    expect(prometheus).toContain('woml_for_each_iterations_pending 20');
    expect(prometheus).toContain('woml_for_each_iterations_completed_total 18');
    expect(prometheus).not.toContain('run_active');
    observed.recordProgress({
      contract: 'woml.execution-progress',
      version: 1,
      type: 'for_each_progress',
      runId: 'run_active',
      forEachId: 'organize',
      status: 'running',
      total: 42,
      succeeded: 19,
      failed: 0,
      skipped: 0,
      active: 4,
      pending: 19,
      concurrency: 4,
    });
    observed.recordProgress({
      contract: 'woml.execution-progress',
      version: 1,
      type: 'for_each_progress',
      runId: 'run_active',
      forEachId: 'organize',
      status: 'running',
      total: 42,
      succeeded: 19,
      failed: 0,
      skipped: 0,
      active: 4,
      pending: 19,
      concurrency: 4,
    });
    expect(await observed.prometheusMetrics()).toContain(
      'woml_for_each_iterations_completed_total 19'
    );
  });

  test('assigns monotonic sequence numbers and requires snapshot resync after a gap', async () => {
    const validate = await validators();
    const observed = fixture();
    for (let index = 0; index < 1030; index += 1) {
      observed.record('run', `run_${index}`, 'running');
    }
    const response = observed.stream(0);
    const body = await response.text();
    const encoded = body.match(/^data: (.+)$/m)?.[1];
    expect(encoded).toBeDefined();
    const gap = JSON.parse(encoded!);
    expect(validate.stream(gap), validate.stream.errors?.join('\n')).toBe(true);
    expect(gap.subject).toMatchObject({
      status: 'resync_required',
      code: 'WOML_OBSERVABILITY_STREAM_GAP',
    });

    const snapshot = (await observed.snapshot()) as { sequence: number };
    expect(snapshot.sequence).toBe(1030);
    const live = observed.stream(snapshot.sequence);
    const reader = live.body!.getReader();
    observed.record('run', 'run_next', 'succeeded');
    const chunk = await reader.read();
    const text = new TextDecoder().decode(chunk.value);
    const event = JSON.parse(text.match(/^data: (.+)$/m)![1]!);
    expect(validate.stream(event)).toBe(true);
    expect(event.sequence).toBe(1031);
    await reader.cancel();
  });

  test('disconnects slow telemetry clients without affecting event production', () => {
    const observed = fixture();
    const slow = observed.stream(0);
    expect(slow.status).toBe(200);
    expect(() => {
      for (let index = 0; index < 200; index += 1)
        observed.record('run', `run_slow_${index}`, 'running');
    }).not.toThrow();
    observed.record('run', 'business_work_continues', 'succeeded');
    observed.closeStreams();
  });
});

describe('Observability HTTP surface', () => {
  test('separates public probes from authenticated detail and tracks readiness', async () => {
    const validate = await validators();
    const observed = fixture();
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_observability',
      deploymentId: 'deployment_observability',
      port: 0,
      observability: observed,
    });
    controls.push(control);

    let response = await fetch(`${control.descriptor.adminUrl}/livez`);
    expect(response.status).toBe(200);
    expect(validate.health(await response.json())).toBe(true);

    response = await fetch(`${control.descriptor.adminUrl}/readyz`);
    expect(response.status).toBe(503);
    observed.setLifecycle('ready');
    response = await fetch(`${control.descriptor.adminUrl}/readyz`);
    expect(response.status).toBe(200);

    const unauthorized = await fetch(`${control.descriptor.adminUrl}/v1/snapshot`);
    expect(unauthorized.status).toBe(401);
    const snapshot = await fetch(`${control.descriptor.adminUrl}/v1/snapshot`, {
      headers: authorization(control.descriptor.capability),
    });
    expect(snapshot.status).toBe(200);
    expect(validate.snapshot(await snapshot.json())).toBe(true);

    const health = await fetch(`${control.descriptor.adminUrl}/v1/health`, {
      headers: authorization(control.descriptor.capability),
    });
    expect(validate.health(await health.json())).toBe(true);
    const metrics = await fetch(`${control.descriptor.adminUrl}/metrics`, {
      headers: authorization(control.descriptor.capability),
    });
    expect(metrics.headers.get('content-type')).toContain('text/plain');
    expect(await metrics.text()).toContain('woml_store_size_bytes');
  });

  test('bounds telemetry responses and rate limits authenticated scraping', async () => {
    const manyRuns: RustRunListV1 = {
      profile: 'woml.run-list/v2',
      runs: Array.from({ length: 20 }, (_, index) => ({
        runId: `run_response_${index}`,
        workflowId: 'orders',
        status: 'running' as const,
        admittedAt: '2026-08-12T09:00:00.000Z',
        startedAt: '2026-08-12T09:00:00.010Z',
        updatedAt: '2026-08-12T09:00:00.020Z',
      })),
    };
    const observed = fixture({ runs: manyRuns });
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_observability_limits',
      deploymentId: 'deployment_observability_limits',
      port: 0,
      observability: observed,
      maxResponseBytes: 1024,
      maxOperationsPerMinute: 2,
    });
    controls.push(control);
    const headers = authorization(control.descriptor.capability);
    const oversized = await fetch(`${control.descriptor.adminUrl}/v1/snapshot`, {
      headers,
    });
    expect(oversized.status).toBe(507);
    expect(
      await fetch(`${control.descriptor.adminUrl}/v1/health`, { headers }).then(
        response => response.status
      )
    ).toBe(200);
    expect(
      await fetch(`${control.descriptor.adminUrl}/v1/health`, { headers }).then(
        response => response.status
      )
    ).toBe(429);
  });

  test('honors disabled health and metrics configuration without opening endpoints', async () => {
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_observability_disabled',
      deploymentId: 'deployment_observability_disabled',
      port: 0,
      observability: fixture(),
      healthEnabled: false,
      metricsEnabled: false,
    });
    controls.push(control);
    expect(
      await fetch(`${control.descriptor.adminUrl}/livez`).then(
        response => response.status
      )
    ).toBe(404);
    expect(
      await fetch(`${control.descriptor.adminUrl}/metrics`, {
        headers: authorization(control.descriptor.capability),
      }).then(response => response.status)
    ).toBe(404);
  });

  test('contains broken telemetry reads without changing runtime liveness', async () => {
    const observed = new RuntimeObservability({
      runtimeInstanceId: 'runtime_broken_telemetry',
      deploymentId: 'deployment_broken_telemetry',
      workflows: [],
      listRuns: () => {
        throw new Error('database detail must not escape');
      },
      storeSize: async () => {
        throw new Error('filesystem detail must not escape');
      },
      logFormat: 'json',
      emitLog: () => {},
    });
    observed.setLifecycle('ready');
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_broken_telemetry',
      deploymentId: 'deployment_broken_telemetry',
      port: 0,
      observability: observed,
    });
    controls.push(control);
    const headers = authorization(control.descriptor.capability);
    const failed = await fetch(`${control.descriptor.adminUrl}/v1/snapshot`, {
      headers,
    });
    expect(failed.status).toBe(503);
    expect(await failed.text()).not.toContain('database detail');
    expect(
      await fetch(`${control.descriptor.adminUrl}/livez`).then(
        response => response.status
      )
    ).toBe(200);
  });
});
