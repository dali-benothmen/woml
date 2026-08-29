import { describe, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';
import { compileWoml, parseWoml } from '@woml/compiler';

import {
  PERFORMANCE_MEASUREMENT_PROFILE,
  PERFORMANCE_SIGNAL_PROFILE,
  decodePerformanceSignal,
  performanceMetric,
  summarizeSamples,
} from '../scripts/performance-measurement';
import { decodePerformanceRegressionBudgets } from '../scripts/performance-regression';

const projectRoot = resolve(import.meta.dir, '../..');
const fixture = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-two-step-manual.woml'
);
const stepCountFixture = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-eight-step-manual.woml'
);
const growingContextFixture = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-growing-context.woml'
);
const nativeCore = resolve(
  import.meta.dir,
  '../dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const regressionBudgets = decodePerformanceRegressionBudgets(
  JSON.parse(
    readFileSync(
      resolve(projectRoot, 'docs/performance-regression-budgets.v1.json'),
      'utf8'
    )
  )
);
const builtTest = existsSync(nativeCore) ? test : test.skip;

describe('performance measurement contract v1', () => {
  test('summarizes raw samples without hiding their distribution', () => {
    expect(summarizeSamples([4, 1, 3, 2])).toEqual({
      count: 4,
      minimumMs: 1,
      medianMs: 2.5,
      p95Ms: 4,
      maximumMs: 4,
      madMs: 1,
    });
    expect(() => summarizeSamples([])).toThrow('finite non-negative');
    expect(() => summarizeSamples([1, Number.NaN])).toThrow(
      'finite non-negative'
    );
  });

  test('decodes only explicit child protocol signals', () => {
    expect(decodePerformanceSignal('ordinary workflow output')).toBeUndefined();
    expect(
      decodePerformanceSignal(JSON.stringify({ profile: 'something-else' }))
    ).toBeUndefined();
    expect(
      decodePerformanceSignal(
        JSON.stringify({
          profile: PERFORMANCE_SIGNAL_PROFILE,
          type: 'run_terminal',
          requestId: 'request_1',
          runId: 'run_1',
          status: 'succeeded',
        })
      )
    ).toEqual({
      profile: PERFORMANCE_SIGNAL_PROFILE,
      type: 'run_terminal',
      requestId: 'request_1',
      runId: 'run_1',
      status: 'succeeded',
    });
  });

  test('keeps the canonical fixture intentionally small and valid', async () => {
    const source = await Bun.file(fixture).text();
    const workflow = compileWoml(parseWoml(source, { file: fixture }));
    expect(workflow.workflowId).toBe(
      regressionBudgets.hard.canonicalWorkflowId
    );
    expect(workflow.graph.nodes.map(node => node.id)).toEqual([
      'prepare',
      'result',
    ]);
    expect(workflow.graph.nodes).toHaveLength(
      regressionBudgets.hard.canonicalNodes
    );
    expect(workflow.graph.edges).toHaveLength(
      regressionBudgets.hard.canonicalEdges
    );
  });

  test('keeps the PERF4 scaling fixtures valid and intentionally shaped', async () => {
    const manySource = await Bun.file(stepCountFixture).text();
    const many = compileWoml(parseWoml(manySource, { file: stepCountFixture }));
    expect(many.graph.nodes).toHaveLength(8);
    expect(many.graph.edges).toHaveLength(7);

    const growingSource = await Bun.file(growingContextFixture).text();
    const growing = compileWoml(
      parseWoml(growingSource, { file: growingContextFixture })
    );
    expect(growing.graph.nodes).toHaveLength(5);
    expect(growing.graph.edges).toHaveLength(4);
  });

  test('validates a representative result against the frozen JSON Schema', async () => {
    const schema = JSON.parse(
      await Bun.file(
        resolve(
          projectRoot,
          'docs/schemas/performance-measurement.v1.schema.json'
        )
      ).text()
    );
    const ajv = new Ajv2020({ strict: false, allErrors: true });
    addFormats(ajv);
    const validate = ajv.compile(schema);
    const result = {
      profile: PERFORMANCE_MEASUREMENT_PROFILE,
      createdAt: '2026-08-28T12:00:00.000Z',
      fixture: { path: 'fixture.woml', sha256: 'a'.repeat(64) },
      environment: {
        platform: 'linux',
        architecture: 'x64',
        bunVersion: '1.3.14',
        cpuModel: 'test',
        logicalCpuCount: 8,
        memoryBytes: 1024,
        nativeBuild: 'release',
        cliArtifact: 'built',
      },
      parameters: { mode: 'manual', warmups: 1, iterations: 2 },
      metrics: [
        performanceMetric(
          'manual.visible',
          'warm',
          {
            start: 'manual trigger control is submitted',
            end: 'terminal run presentation is received',
          },
          [12, 14]
        ),
      ],
    };
    expect(validate(result), JSON.stringify(validate.errors, null, 2)).toBe(
      true
    );
  });

  test('freezes a secret-free cross-process span shape for later phases', async () => {
    const schema = JSON.parse(
      await Bun.file(
        resolve(projectRoot, 'docs/schemas/performance-span.v1.schema.json')
      ).text()
    );
    const ajv = new Ajv2020({ strict: false, allErrors: true });
    const validate = ajv.compile(schema);
    const span = {
      profile: 'woml.performance-span/v1',
      traceId: 'trace_1',
      spanId: 'span_1',
      process: 'worker',
      layer: 'worker',
      name: 'worker.execute',
      startOffsetMs: 12.5,
      durationMs: 3.25,
      status: 'succeeded',
      counts: { nodes: 2 },
      bytes: { context: 128 },
    };
    expect(validate(span), JSON.stringify(validate.errors, null, 2)).toBe(true);
    expect(validate({ ...span, secret: 'must-not-be-recorded' })).toBe(false);
  });

  builtTest(
    'measures a real manual run without mistaking startup output for completion',
    () => {
      const directory = mkdtempSync(join(tmpdir(), 'woml-performance-test-'));
      const tracePath = join(directory, 'frontend.ndjson');
      try {
        const result = Bun.spawnSync(
          [
            process.execPath,
            'scripts/measure-workflow.ts',
            fixture,
            '--mode',
            'all',
            '--warmups',
            '0',
            '--iterations',
            '1',
            '--profile-output',
            tracePath,
            '--json',
          ],
          {
            cwd: resolve(projectRoot, 'woml-cli'),
            stdout: 'pipe',
            stderr: 'pipe',
            timeout: 60_000,
          }
        );
        expect(result.exitCode, result.stderr.toString()).toBe(0);
        const measurement = JSON.parse(result.stdout.toString());
        expect(measurement.profile).toBe(PERFORMANCE_MEASUREMENT_PROFILE);
        expect(
          measurement.metrics.map((metric: { name: string }) => metric.name)
        ).toEqual([
          'harness.process',
          'activation.cold',
          'manual.visible',
          'engine.durable',
        ]);
        expect(measurement.metrics[2].summary.count).toBe(1);
        expect(measurement.metrics[2].samples[0]).toBeGreaterThan(0);

        const traceText = readFileSync(tracePath, 'utf8');
        expect(traceText).not.toContain('Hello World');
        expect(traceText).not.toContain('return {');
        const spans: Array<{
          traceId: string;
          name: string;
          process: string;
          layer: string;
          startOffsetMs: number;
          durationMs: number;
          runId?: string;
          invocationId?: string;
          counts?: Readonly<Record<string, number>>;
          bytes?: Readonly<Record<string, number>>;
        }> = traceText
          .trim()
          .split('\n')
          .map(line => JSON.parse(line));
        const schema = JSON.parse(
          readFileSync(
            resolve(
              projectRoot,
              'docs/schemas/performance-span.v1.schema.json'
            ),
            'utf8'
          )
        );
        const validate = new Ajv2020({
          strict: false,
          allErrors: true,
        }).compile(schema);
        expect(
          spans.every(span => validate(span)),
          JSON.stringify(validate.errors, null, 2)
        ).toBe(true);
        const names = spans.map((span: { name: string }) => span.name);
        expect(names).toContain('cli.process_to_profiler');
        expect(names).toContain('cli.parse_run_arguments');
        expect(names).toContain('compiler.compile_inputs');
        expect(names).toContain('compiler.parse_markup');
        expect(names).toContain('compiler.build_definition_package');
        expect(names).toContain('compiler.lower_model');
        expect(names).toContain('compiler.hash_definition');
        expect(names).toContain('compiler.verify_source_snapshot');
        expect(names).toContain('compiler.refresh_editor_types');
        expect(names).toContain('cli.run_command');
        expect(names).toContain('napi.load_native_addon');
        expect(names).toContain('napi.serialize_runtime_registration');
        expect(names).toContain('napi.decode_runtime_registration');
        expect(names).toContain('napi.start_trigger_runtime');
        expect(names).toContain('runtime.prepare_trigger_host');
        expect(names).toContain('sqlite.open_initialize');
        expect(names).toContain('sqlite.register_definition');
        expect(names).toContain('napi.activate_trigger_runtime');
        expect(names).toContain('runtime.activate_trigger_host');
        expect(names).toContain('napi.submit_manual_trigger');
        expect(names).toContain('runtime.admit_trigger');
        expect(names).toContain('sqlite.admit_trigger');
        expect(names).toContain('runtime.execute_admitted_run');
        expect(names).toContain('runtime.resume_engine');
        expect(names).toContain('sqlite.append_events');
        expect(names).toContain('runtime.fold_events');
        expect(names).toContain('napi.serialize_durable_request');
        expect(names).toContain('napi.decode_durable_request');
        expect(names).toContain('napi.execute_durable');
        expect(names).toContain('napi.encode_durable_result');
        expect(names).toContain('napi.decode_durable_result');
        expect(names).toContain('runtime.execute_durable');
        expect(names).toContain('runtime.run_engine');
        expect(names).toContain('host.spawn_process');
        expect(names).toContain('host.register_modules');
        expect(names).toContain('host.execute_invocation');
        expect(names).toContain('host.write_frame');
        expect(names).toContain('host.read_frame');
        expect(names).toContain('host.ready');
        expect(names).toContain('host.process_to_profiler');
        expect(names).toContain('host.decode_frames');
        expect(names).toContain('host.create_worker');
        expect(names).toContain('host.post_worker_request');
        expect(names).toContain('host.receive_worker_result');
        expect(names).toContain('host.terminate_worker');
        expect(names).toContain('worker.execute_invocation');
        expect(names).toContain('worker.process_to_profiler');
        expect(names).toContain('worker.prepare_context');
        expect(names).toContain('worker.load_modules');
        expect(names).toContain('worker.compile_script');
        expect(names).toContain('worker.execute_user_code');
        expect(names).toContain('worker.validate_result');
        expect(names).toContain('worker.prepare_result_transfer');
        expect(names).toContain('runtime.resolve_registrations');
        expect(names).toContain('runtime.start_core_services');
        expect(names).toContain('runtime.initialize_observability');
        expect(names).toContain('runtime.start_control');
        expect(names).toContain('runtime.start_provider_hosts');
        expect(names).toContain('runtime.final_source_revalidation');
        expect(names).toContain('runtime.open_admission');
        expect(names).toContain('runtime.schedule_retention');
        expect(names).toContain('runtime.publish_descriptor');
        expect(names).toContain('runtime.report_ready');
        expect(names).toContain('presentation.handle_startup');
        expect(names).toContain('presentation.render_startup');
        expect(names).toContain('presentation.handle_progress');
        expect(names).toContain('presentation.present_terminal_result');
        expect(names).toContain('presentation.inspect_durable_run');
        expect(names).toContain('presentation.inspect_store');
        expect(names).toContain('presentation.project_run');
        expect(names).toContain('presentation.summarize_result');
        expect(names).toContain('napi.inspect_run_presentation');
        expect(names).toContain('presentation.decode_run_presentation');
        expect(names).toContain('presentation.render_final');
        expect(names).toContain('presentation.output_write');
        expect(
          spans.some(span => span.process === 'native' && span.layer === 'napi')
        ).toBe(true);
        expect(
          spans.some(span => span.process === 'rust' && span.layer === 'sqlite')
        ).toBe(true);

        const manualCli = spans.find(
          span =>
            span.process === 'cli' && span.name === 'napi.submit_manual_trigger'
        );
        const manualNative = spans.find(
          span =>
            span.process === 'native' &&
            span.name === 'napi.submit_manual_trigger'
        );
        expect(manualCli?.invocationId).toBeDefined();
        expect(manualNative?.invocationId).toBe(manualCli?.invocationId);
        expect(manualNative?.runId).toBeDefined();

        const engineCli = spans.find(
          span => span.process === 'cli' && span.name === 'napi.execute_durable'
        );
        const engineNative = spans.find(
          span =>
            span.process === 'native' && span.name === 'napi.execute_durable'
        );
        expect(engineCli?.runId).toBeDefined();
        expect(engineNative?.runId).toBe(engineCli?.runId);
        expect(engineNative?.traceId).toBe(engineCli?.traceId);

        const rustHost = spans.find(
          span =>
            span.process === 'rust' && span.name === 'host.execute_invocation'
        );
        const bunHost = spans.find(
          span =>
            span.process === 'host' && span.name === 'host.execute_invocation'
        );
        const worker = spans.find(
          span =>
            span.process === 'worker' &&
            span.name === 'worker.execute_invocation'
        );
        expect(rustHost?.invocationId).toBeDefined();
        expect(bunHost?.invocationId).toBe(rustHost?.invocationId);
        expect(worker?.invocationId).toBe(rustHost?.invocationId);
        expect(bunHost?.runId).toBe(rustHost?.runId);
        expect(worker?.runId).toBe(rustHost?.runId);
        expect(worker?.bytes?.context).toBeGreaterThan(0);

        const compile = spans.find(
          (span: { name: string }) => span.name === 'compiler.compile_inputs'
        );
        if (compile === undefined)
          throw new Error('The compile envelope span is missing.');
        const intervals = spans
          .filter(
            span =>
              span.process === 'cli' &&
              span.name !== 'compiler.compile_inputs' &&
              span.startOffsetMs >= compile.startOffsetMs &&
              span.startOffsetMs + span.durationMs <=
                compile.startOffsetMs + compile.durationMs
          )
          .map((span: { startOffsetMs: number; durationMs: number }) => ({
            start: span.startOffsetMs,
            end: span.startOffsetMs + span.durationMs,
          }))
          .sort(
            (left: { start: number }, right: { start: number }) =>
              left.start - right.start
          );
        let covered = 0;
        let coveredUntil = compile.startOffsetMs;
        for (const interval of intervals) {
          const start = Math.max(coveredUntil, interval.start);
          if (interval.end > start) covered += interval.end - start;
          coveredUntil = Math.max(coveredUntil, interval.end);
        }
        expect(covered).toBeGreaterThan(compile.durationMs * 0.95);
      } finally {
        rmSync(directory, { recursive: true, force: true });
      }
    },
    70_000
  );

  builtTest(
    'reuses one supervised script host while preserving isolated workers',
    () => {
      const directory = mkdtempSync(
        join(tmpdir(), 'woml-performance-host-pool-')
      );
      const tracePath = join(directory, 'shared-host.ndjson');
      try {
        const result = Bun.spawnSync(
          [
            process.execPath,
            'scripts/measure-workflow.ts',
            fixture,
            '--mode',
            'manual',
            '--warmups',
            '1',
            '--iterations',
            String(regressionBudgets.hard.measuredRuns - 1),
            '--profile-output',
            tracePath,
            '--json',
          ],
          {
            cwd: resolve(projectRoot, 'woml-cli'),
            stdout: 'pipe',
            stderr: 'pipe',
            timeout: 60_000,
          }
        );
        expect(result.exitCode, result.stderr.toString()).toBe(0);

        const spans: Array<{
          readonly process: string;
          readonly name: string;
          readonly runId?: string;
        }> = readFileSync(tracePath, 'utf8')
          .trim()
          .split('\n')
          .map(line => JSON.parse(line));
        const rustHostSpawns = spans.filter(
          span => span.process === 'rust' && span.name === 'host.spawn_process'
        );
        const rustHostShutdowns = spans.filter(
          span =>
            span.process === 'rust' && span.name === 'host.shutdown_process'
        );
        const workerInvocations = spans.filter(
          span =>
            span.process === 'worker' &&
            span.name === 'worker.execute_invocation'
        );

        expect(rustHostSpawns.length).toBeGreaterThan(0);
        expect(rustHostSpawns.length).toBeLessThanOrEqual(
          regressionBudgets.hard.maxScriptHostSpawns
        );
        expect(rustHostShutdowns.length).toBeGreaterThan(0);
        expect(rustHostShutdowns.length).toBeLessThanOrEqual(
          regressionBudgets.hard.maxScriptHostShutdowns
        );
        expect(workerInvocations).toHaveLength(
          regressionBudgets.hard.measuredRuns *
            regressionBudgets.hard.canonicalNodes *
            regressionBudgets.hard.isolatedWorkersPerScript
        );
        expect(new Set(workerInvocations.map(span => span.runId)).size).toBe(
          regressionBudgets.hard.measuredRuns
        );
      } finally {
        rmSync(directory, { recursive: true, force: true });
      }
    },
    70_000
  );
});
