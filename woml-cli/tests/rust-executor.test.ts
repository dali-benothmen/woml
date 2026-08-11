import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml, type CompiledWorkflowDefinition } from 'woml';
import {
  compiledDefinitionHash,
  executeWorkflowWithRustDurable,
  executeWorkflowWithRust,
  parseIntervalProgress,
  parseScheduleProgress,
  parseTriggerProgress,
  parseWorkflowCallProgress,
  recoverDurableRuns,
  RustWorkflowExecutionError,
} from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const stagedNativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeCorePath =
  process.env.WOML_RUST_CORE_PATH ??
  (existsSync(stagedNativeCorePath) ? stagedNativeCorePath : undefined);
const scriptHostPath = resolve(packageRoot, 'src/script-host.ts');
const crashingHostPath = resolve(
  packageRoot,
  'tests/fixtures/crashing-script-host.ts'
);
const contextLimitedHostPath = resolve(
  packageRoot,
  'tests/fixtures/context-limited-script-host.ts'
);
const resultLimitedHostPath = resolve(
  packageRoot,
  'tests/fixtures/result-limited-script-host.ts'
);
const nativeTest = nativeCorePath === undefined ? test.skip : test;

describe('Trigger Progress v1 decoding', () => {
  test('accepts every frozen T4 progress fixture', async () => {
    const directory = resolve(
      packageRoot,
      '../woml/tests/fixtures/trigger-contracts'
    );
    for (const name of [
      'progress-ready.v1.json',
      'progress-accepted.v1.json',
      'progress-run-started.v1.json',
      'progress-run-succeeded.v1.json',
      'progress-run-failed.v1.json',
      'progress-rejected.v1.json',
    ]) {
      const progress = parseTriggerProgress(await Bun.file(join(directory, name)).text());
      expect(progress.contract).toBe('woml.trigger-progress');
      expect(progress.contractVersion).toBe(1);
    }
  });

  test('rejects unknown fields instead of silently widening the contract', () => {
    expect(() =>
      parseTriggerProgress(
        JSON.stringify({
          contract: 'woml.trigger-progress',
          contractVersion: 1,
          type: 'ready',
          registrationCount: 1,
          occurredAt: '2026-08-08T12:00:00.000Z',
          secret: 'must-not-pass',
        })
      )
    ).toThrow('invalid trigger progress');
  });

  test('accepts readiness for a call-only runtime with zero triggers', () => {
    const progress = parseTriggerProgress(
      JSON.stringify({
        contract: 'woml.trigger-progress',
        contractVersion: 1,
        type: 'ready',
        registrationCount: 0,
        occurredAt: '2026-08-11T12:00:00.000Z',
      })
    );
    expect(progress.type).toBe('ready');
    if (progress.type === 'ready') expect(progress.registrationCount).toBe(0);
  });
});

describe('Schedule Progress v1 decoding', () => {
  test('accepts next-due diagnostics without widening Trigger Progress v1', () => {
    const progress = parseScheduleProgress(
      JSON.stringify({
        contract: 'woml.schedule-progress',
        contractVersion: 1,
        type: 'next_due',
        workflowId: 'daily-report',
        triggerId: 'dailyReport',
        timezone: 'Europe/Berlin',
        nextScheduledAt: '2026-08-09T07:00:00.000Z',
        reason: 'initialized',
        occurredAt: '2026-08-08T12:00:00.000Z',
      })
    );
    expect(progress.type).toBe('next_due');
    expect(() => parseTriggerProgress(JSON.stringify(progress))).toThrow(
      'invalid trigger progress'
    );
  });

  test('rejects unknown schedule progress fields', () => {
    expect(() =>
      parseScheduleProgress(
        JSON.stringify({
          contract: 'woml.schedule-progress',
          contractVersion: 1,
          type: 'scheduler_error',
          workflowId: 'daily-report',
          triggerId: 'dailyReport',
          code: 'WOML_SCHEDULE_RUNTIME_FAILED',
          message: 'safe summary',
          occurredAt: '2026-08-08T12:00:00.000Z',
          payload: { secret: true },
        })
      )
    ).toThrow('invalid schedule progress');
  });
});

describe('Interval Progress v1 decoding', () => {
  test('accepts an anchored next-due diagnostic', () => {
    const progress = parseIntervalProgress(
      JSON.stringify({
        contract: 'woml.interval-progress',
        contractVersion: 1,
        type: 'next_due',
        workflowId: 'heartbeat',
        triggerId: 'tick',
        everyMs: 5000,
        anchorAt: '2026-08-08T12:00:00.000Z',
        nextSequence: 4,
        nextScheduledAt: '2026-08-08T12:00:20.000Z',
        reason: 'advanced',
        occurredAt: '2026-08-08T12:00:15.000Z',
      })
    );
    expect(progress.type).toBe('next_due');
    expect(() => parseTriggerProgress(JSON.stringify(progress))).toThrow(
      'invalid trigger progress'
    );
  });

  test('rejects unknown interval progress fields', () => {
    expect(() =>
      parseIntervalProgress(
        JSON.stringify({
          contract: 'woml.interval-progress',
          contractVersion: 1,
          type: 'scheduler_error',
          workflowId: 'heartbeat',
          triggerId: 'tick',
          code: 'WOML_INTERVAL_RUNTIME_FAILED',
          message: 'safe summary',
          occurredAt: '2026-08-08T12:00:00.000Z',
          context: { secret: true },
        })
      )
    ).toThrow('invalid interval progress');
  });
});

describe('Workflow Call Progress v1 decoding', () => {
  test('accepts safe admission and terminal messages', () => {
    const admitted = parseWorkflowCallProgress(
      JSON.stringify({
        contract: 'woml.workflow-call-progress',
        contractVersion: 1,
        type: 'call_admitted',
        parentRunId: 'run_parent',
        parentNodeId: 'calculateRisk',
        targetWorkflowId: 'calculate-risk',
        childRunId: 'run_child',
        duplicate: false,
        occurredAt: '2026-08-11T12:00:00.000Z',
      })
    );
    expect(admitted.type).toBe('call_admitted');
    const terminal = parseWorkflowCallProgress(
      JSON.stringify({
        contract: 'woml.workflow-call-progress',
        contractVersion: 1,
        type: 'child_terminal',
        parentRunId: 'run_parent',
        targetWorkflowId: 'calculate-risk',
        childRunId: 'run_child',
        status: 'succeeded',
        occurredAt: '2026-08-11T12:00:01.000Z',
      })
    );
    expect(terminal.type).toBe('child_terminal');
    const rejected = parseWorkflowCallProgress(
      JSON.stringify({
        contract: 'woml.workflow-call-progress',
        contractVersion: 1,
        type: 'call_rejected',
        parentRunId: 'run_parent',
        parentNodeId: 'calculateRisk',
        targetWorkflowId: 'approval-workflow',
        code: 'WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED',
        message: 'Run it independently.',
        occurredAt: '2026-08-11T12:00:01.000Z',
      })
    );
    expect(rejected.type).toBe('call_rejected');
  });

  test('rejects payloads and secrets on the progress surface', () => {
    expect(() =>
      parseWorkflowCallProgress(
        JSON.stringify({
          contract: 'woml.workflow-call-progress',
          contractVersion: 1,
          type: 'call_admitted',
          parentRunId: 'run_parent',
          parentNodeId: 'calculateRisk',
          targetWorkflowId: 'calculate-risk',
          childRunId: 'run_child',
          duplicate: false,
          occurredAt: '2026-08-11T12:00:00.000Z',
          payload: { token: 'must-not-pass' },
        })
      )
    ).toThrow('invalid workflow call progress');
  });
});

async function helloWorkflow(): Promise<CompiledWorkflowDefinition> {
  const path = resolve(packageRoot, '../woml/tests/fixtures/hello.woml');
  return compileWoml(parseWoml(await Bun.file(path).text(), { file: path }));
}

async function branchWorkflow(): Promise<CompiledWorkflowDefinition> {
  const path = resolve(packageRoot, '../woml/tests/fixtures/branch.woml');
  return compileWoml(parseWoml(await Bun.file(path).text(), { file: path }));
}

async function parallelWorkflow(): Promise<CompiledWorkflowDefinition> {
  const path = resolve(packageRoot, '../woml/tests/fixtures/parallel.woml');
  return compileWoml(parseWoml(await Bun.file(path).text(), { file: path }));
}

async function parallelSource(): Promise<string> {
  const path = resolve(packageRoot, '../woml/tests/fixtures/parallel.woml');
  return await Bun.file(path).text();
}

function compileSource(source: string, file = 'branch-hardening.woml') {
  return compileWoml(parseWoml(source, { file }));
}

const branchAtBeginningSource = `<woml>
<workflow version="0.1" id="branch-first">
  <triggers><manual id="start" /></triggers>
  <steps>
    <branch id="route">
      <when test="{{context.trigger.primary}}">
        <step id="primary"><script>return { selected: "primary" };</script></step>
        <result value="{{context.steps.primary}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return { selected: "fallback" };</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>
    <step id="finish"><script>return { selected: context.steps.route.selected };</script></step>
  </steps>
</workflow>
</woml>`;

const branchAtEndSource = `<woml>
<workflow version="0.1" id="branch-last">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="choice"><script>return { primary: true };</script></step>
    <branch id="route">
      <when test="{{context.steps.choice.primary}}">
        <step id="primary"><script>return { selected: "primary" };</script></step>
        <result value="{{context.steps.primary}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return { selected: "fallback" };</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>
  </steps>
</workflow>
</woml>`;

const nestedBranchSource = `<woml>
<workflow version="0.1" id="nested-branches">
  <triggers><manual id="start" /></triggers>
  <steps>
    <branch id="outer">
      <when test="{{context.trigger.outer}}">
        <branch id="inner">
          <when test="{{context.trigger.inner}}">
            <step id="nested"><script>return { selected: "nested" };</script></step>
            <result value="{{context.steps.nested}}" />
          </when>
          <otherwise>
            <step id="innerFallback"><script>return { selected: "inner-fallback" };</script></step>
            <result value="{{context.steps.innerFallback}}" />
          </otherwise>
        </branch>
        <result value="{{context.steps.inner}}" />
      </when>
      <otherwise>
        <step id="outerFallback"><script>return { selected: "outer-fallback" };</script></step>
        <result value="{{context.steps.outerFallback}}" />
      </otherwise>
    </branch>
    <step id="finish"><script>return { selected: context.steps.outer.selected };</script></step>
  </steps>
</workflow>
</woml>`;

const multipleTrueSource = `<woml>
<workflow version="0.1" id="first-match">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="flags"><script>return { first: true, second: true };</script></step>
    <branch id="route">
      <when test="{{context.steps.flags.first}}">
        <step id="firstRoute"><script>return { selected: "first" };</script></step>
        <result value="{{context.steps.firstRoute}}" />
      </when>
      <when test="{{context.steps.flags.second}}">
        <step id="secondRoute"><script>return { selected: "second" };</script></step>
        <result value="{{context.steps.secondRoute}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return { selected: "fallback" };</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>
  </steps>
</workflow>
</woml>`;

const largeBranchResultSource = `<woml>
<workflow version="0.1" id="large-branch-result">
  <triggers><manual id="start" /></triggers>
  <steps>
    <branch id="route">
      <when test="{{context.trigger.primary}}">
        <step id="largeResult"><script>return { payload: "x".repeat(131072) };</script></step>
        <result value="{{context.steps.largeResult}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return { payload: "fallback" };</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>
    <step id="finish"><script>return { length: context.steps.route.payload.length };</script></step>
  </steps>
</workflow>
</woml>`;

const parallelAtBeginningSource = `<woml>
<workflow version="0.1" id="parallel-first">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="load" concurrency="2" on-error="wait-all">
      <step id="left"><script>return { value: 20 };</script></step>
      <step id="right"><script>return { value: 22 };</script></step>
    </parallel>
    <step id="finish"><script>return { value: context.steps.left.value + context.steps.right.value };</script></step>
  </steps>
</workflow>
</woml>`;

const parallelBranchCompositionSource = `<woml>
<workflow version="0.1" id="parallel-branch-composition">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="flags"><script>return { selected: true };</script></step>
    <branch id="route">
      <when test="{{context.steps.flags.selected}}">
        <parallel id="selectedChecks" concurrency="2" on-error="wait-all">
          <step id="selectedLeft"><script>return { value: "selected-left" };</script></step>
          <step id="selectedRight"><script>return { value: "selected-right" };</script></step>
        </parallel>
        <result value="{{context.steps.selectedLeft}}" />
      </when>
      <otherwise>
        <parallel id="unselectedChecks" concurrency="2" on-error="wait-all">
          <step id="unselectedLeft"><script>return { value: "unselected-left" };</script></step>
          <step id="unselectedRight"><script>return { value: "unselected-right" };</script></step>
        </parallel>
        <result value="{{context.steps.unselectedLeft}}" />
      </otherwise>
    </branch>
    <step id="finish"><script>return { value: context.steps.route.value };</script></step>
  </steps>
</workflow>
</woml>`;

const parallelBeforeBranchSource = `<woml>
<workflow version="0.1" id="parallel-before-branch">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="checks" concurrency="2" on-error="wait-all">
      <step id="decision"><script>return { approved: true };</script></step>
      <step id="audit"><script>return { recorded: true };</script></step>
    </parallel>
    <branch id="route">
      <when test="{{context.steps.decision.approved}}">
        <step id="approved"><script>return { status: "approved" };</script></step>
        <result value="{{context.steps.approved}}" />
      </when>
      <otherwise>
        <step id="rejected"><script>return { status: "rejected" };</script></step>
        <result value="{{context.steps.rejected}}" />
      </otherwise>
    </branch>
  </steps>
</workflow>
</woml>`;

const manyParallelChildrenSource = `<woml>
<workflow version="0.1" id="many-parallel-children">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="jobs" concurrency="2" on-error="wait-all">
      <step id="slow"><script>await new Promise(resolve => setTimeout(resolve, 180)); return { order: "slow" };</script></step>
      <step id="fast"><script>await new Promise(resolve => setTimeout(resolve, 10)); return { order: "fast" };</script></step>
      <step id="middle"><script>await new Promise(resolve => setTimeout(resolve, 70)); return { order: "middle" };</script></step>
      <step id="later"><script>await new Promise(resolve => setTimeout(resolve, 30)); return { order: "later" };</script></step>
    </parallel>
    <step id="finish"><script>return { count: 4 };</script></step>
  </steps>
</workflow>
</woml>`;

const largeParallelContextSource = `<woml>
<workflow version="0.1" id="large-parallel-context">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="seed"><script>return { payload: "x".repeat(8192) };</script></step>
    <parallel id="readers" concurrency="2" on-error="wait-all">
      <step id="readLength"><script>return { length: context.steps.seed.payload.length };</script></step>
      <step id="readAgain"><script>return { length: context.steps.seed.payload.length };</script></step>
    </parallel>
    <step id="finish"><script>return { length: context.steps.readLength.length };</script></step>
  </steps>
</workflow>
</woml>`;

const largeParallelResultSource = `<woml>
<workflow version="0.1" id="large-parallel-result">
  <triggers><manual id="start" /></triggers>
  <steps>
    <parallel id="builders" concurrency="2" on-error="wait-all">
      <step id="largeResult"><script>return { payload: "x".repeat(8192) };</script></step>
      <step id="smallResult"><script>return { ok: true };</script></step>
    </parallel>
    <step id="finish"><script>return { length: context.steps.largeResult.payload.length };</script></step>
  </steps>
</workflow>
</woml>`;

function replaceFirstScript(
  workflow: CompiledWorkflowDefinition,
  source: string
): CompiledWorkflowDefinition {
  const mutable = structuredClone(workflow) as unknown as {
    graph: {
      nodes: Array<{
        inputs: {
          fields: Record<string, { value: unknown }>;
        };
      }>;
    };
  };
  mutable.graph.nodes[0].inputs.fields.source.value = source;
  return mutable as unknown as CompiledWorkflowDefinition;
}

async function rustError(
  workflow: CompiledWorkflowDefinition,
  options: {
    readonly scriptTimeoutMs?: number;
    readonly scriptHostPath?: string;
  } = {}
): Promise<string> {
  try {
    await executeWorkflowWithRust(workflow, {
      nativeCorePath,
      scriptHostPath: options.scriptHostPath ?? scriptHostPath,
      scriptTimeoutMs: options.scriptTimeoutMs,
    });
    throw new Error('Expected Rust execution to fail.');
  } catch (error) {
    if (error instanceof RustWorkflowExecutionError) {
      return `${error.code}: ${error.message}`;
    }
    return error instanceof Error ? error.message : String(error);
  }
}

async function capturedRustExecutionError(
  workflow: CompiledWorkflowDefinition,
  options: { readonly trigger?: Record<string, boolean> } = {},
  scriptHostOverride = scriptHostPath
): Promise<RustWorkflowExecutionError> {
  try {
    await executeWorkflowWithRust(workflow, {
      nativeCorePath,
      scriptHostPath: scriptHostOverride,
      trigger: options.trigger,
    });
  } catch (error) {
    if (error instanceof RustWorkflowExecutionError) return error;
    throw error;
  }
  throw new Error('Expected Rust execution to fail.');
}

describe('Rust to Bun workflow execution', () => {
  test('pins the production definition hash for hello.woml', async () => {
    expect(compiledDefinitionHash(await helloWorkflow())).toBe(
      'sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a'
    );
  });

  test('pins the generated model-v2 definition hash for branch.woml', async () => {
    const workflow = await branchWorkflow();

    expect(workflow.schemaVersion).toBe(2);
    expect(compiledDefinitionHash(workflow)).toBe(
      'sha256:6a9b3aa53e81ae0e95414f80df0192de5ff11489e9b65b1254b69b71a496155a'
    );
  });

  test('pins the generated model-v3 definition hash for parallel.woml', async () => {
    const workflow = await parallelWorkflow();

    expect(workflow.schemaVersion).toBe(3);
    expect(compiledDefinitionHash(workflow)).toBe(
      'sha256:d58dfcefdcd6c40db659042c41e17ca6c8d652033f90f120734d5cd95819b45c'
    );
  });

  nativeTest(
    'matches the reviewed result, context, and order fixtures',
    async () => {
      const workflow = await helloWorkflow();
      const expectedContexts = await Bun.file(
        resolve(packageRoot, '../woml/tests/fixtures/hello.context.v0.1.json')
      ).json();
      const rust = await executeWorkflowWithRust(workflow, {
        nativeCorePath,
        scriptHostPath,
      });

      expect(rust.result).toEqual({ message: 'Hello World' });
      expect(rust.context).toEqual(expectedContexts.afterB);
      expect(rust.executionOrder).toEqual(['a', 'b']);
      expect(rust.events.map(event => event.type)).toEqual([
        'run_started',
        'step_attempt_started',
        'step_attempt_succeeded',
        'step_attempt_started',
        'step_attempt_succeeded',
        'run_succeeded',
      ]);
    }
  );

  nativeTest(
    'executes only the selected branch route and publishes its merged result',
    async () => {
      const workflow = await branchWorkflow();
      const rust = await executeWorkflowWithRust(workflow, {
        nativeCorePath,
        scriptHostPath,
      });

      expect(rust.result).toEqual({ message: 'Final status: reviewed' });
      expect(rust.executionOrder).toEqual([
        'checkContent',
        'reviewContent',
        'decision',
        'publishDecision',
      ]);
      expect(rust.context.steps.reviewContent).toEqual({
        status: 'reviewed',
        accepted: true,
      });
      expect(rust.context.steps.acceptContent).toBeUndefined();
      expect(rust.context.steps.decision).toEqual({
        status: 'reviewed',
        accepted: true,
      });
      expect(
        rust.events.find(event => event.type === 'branch_selected')?.data
      ).toEqual({ branchId: 'decision', armId: 'decision:when:0' });
      expect(rust.events.every(event => event.eventSchemaVersion === 2)).toBe(
        true
      );
    }
  );

  nativeTest(
    'carries model v3 and event v3 through the production native boundary',
    async () => {
      const execution = await executeWorkflowWithRust(
        await parallelWorkflow(),
        { nativeCorePath, scriptHostPath }
      );

      expect(execution.result).toEqual({
        summary: 'Weather 22°C, soil 41%',
      });
      expect(execution.context.steps.loadWeather).toEqual({
        fieldId: 'field-42',
        temperature: 22,
      });
      expect(execution.context.steps.loadSoil).toEqual({
        fieldId: 'field-42',
        moisture: 41,
      });
      expect(execution.context.steps.fieldData).toBeUndefined();
      expect(
        execution.events.every(event => event.eventSchemaVersion === 3)
      ).toBe(true);
      const weatherStart = execution.events.findIndex(
        event =>
          event.type === 'step_attempt_started' &&
          (event.data as { nodeId?: string }).nodeId === 'loadWeather'
      );
      const soilStart = execution.events.findIndex(
        event =>
          event.type === 'step_attempt_started' &&
          (event.data as { nodeId?: string }).nodeId === 'loadSoil'
      );
      const firstChildTerminal = execution.events.findIndex(
        event =>
          (event.type === 'step_attempt_succeeded' ||
            event.type === 'step_attempt_failed') &&
          ['loadWeather', 'loadSoil'].includes(
            (event.data as { nodeId?: string }).nodeId ?? ''
          )
      );
      expect(weatherStart).toBeGreaterThan(-1);
      expect(soilStart).toBeGreaterThan(-1);
      expect(weatherStart).toBeLessThan(firstChildTerminal);
      expect(soilStart).toBeLessThan(firstChildTerminal);
    }
  );

  nativeTest(
    'preserves the frozen structured details for both parallel error policies',
    async () => {
      const source = await parallelSource();
      const failingWeather = source.replace(
        'await new Promise(resolve => setTimeout(resolve, 80));\n          return {\n            fieldId: context.steps.loadField.fieldId,\n            temperature: 22\n          };',
        'throw new Error("weather unavailable");'
      );
      const waitAllError = await capturedRustExecutionError(
        compileSource(failingWeather, 'parallel-wait-all.woml')
      );
      expect(waitAllError.code).toBe('WOML_PARALLEL_CHILD_FAILED');
      expect(waitAllError.nodeId).toBe('loadWeather');
      expect(waitAllError.parallelId).toBe('fieldData');
      expect(waitAllError.parallelPolicy).toBe('wait-all');
      expect(waitAllError.primaryNodeId).toBe('loadWeather');
      expect(waitAllError.failedNodeIds).toEqual(['loadWeather']);
      expect(waitAllError.cancelledNodeIds).toEqual([]);

      const failFastSource = failingWeather
        .replace('on-error="wait-all"', 'on-error="fail-fast"')
        .replace(
          'await new Promise(resolve => setTimeout(resolve, 80));\n          return {\n            fieldId: context.steps.loadField.fieldId,\n            moisture: 41\n          };',
          'await new Promise(resolve => setTimeout(resolve, 1500)); return { moisture: 41 };'
        );
      const failFastError = await capturedRustExecutionError(
        compileSource(failFastSource, 'parallel-fail-fast.woml')
      );
      expect(failFastError.code).toBe('WOML_PARALLEL_CHILD_FAILED');
      expect(failFastError.parallelId).toBe('fieldData');
      expect(failFastError.parallelPolicy).toBe('fail-fast');
      expect(failFastError.primaryNodeId).toBe('loadWeather');
      expect(failFastError.failedNodeIds).toEqual(['loadWeather']);
      expect(failFastError.cancelledNodeIds).toEqual(['loadSoil']);
    }
  );

  nativeTest(
    'composes parallel at the beginning, inside branch routes, and before a branch',
    async () => {
      const beginning = await executeWorkflowWithRust(
        compileSource(parallelAtBeginningSource, 'parallel-first.woml'),
        { nativeCorePath, scriptHostPath }
      );
      expect(beginning.result).toEqual({ value: 42 });
      expect(beginning.context.steps.load).toBeUndefined();

      const insideBranch = await executeWorkflowWithRust(
        compileSource(
          parallelBranchCompositionSource,
          'parallel-inside-branch.woml'
        ),
        { nativeCorePath, scriptHostPath }
      );
      expect(insideBranch.result).toEqual({ value: 'selected-left' });
      expect(insideBranch.context.steps.selectedLeft).toEqual({
        value: 'selected-left',
      });
      expect(insideBranch.context.steps.selectedRight).toEqual({
        value: 'selected-right',
      });
      expect(insideBranch.context.steps.unselectedLeft).toBeUndefined();
      expect(insideBranch.context.steps.unselectedRight).toBeUndefined();
      expect(
        insideBranch.events.some(
          event =>
            event.type === 'parallel_group_started' &&
            (event.data as { parallelId?: string }).parallelId ===
              'unselectedChecks'
        )
      ).toBe(false);

      const beforeBranch = await executeWorkflowWithRust(
        compileSource(
          parallelBeforeBranchSource,
          'parallel-before-branch.woml'
        ),
        { nativeCorePath, scriptHostPath }
      );
      expect(beforeBranch.result).toEqual({ status: 'approved' });
      expect(beforeBranch.context.steps.rejected).toBeUndefined();
      expect(
        beforeBranch.events.find(event => event.type === 'branch_selected')
          ?.data
      ).toEqual({ branchId: 'route', armId: 'route:when:0' });
    }
  );

  nativeTest(
    'bounds many out-of-order children by the authored concurrency',
    async () => {
      const execution = await executeWorkflowWithRust(
        compileSource(manyParallelChildrenSource, 'many-parallel.woml'),
        { nativeCorePath, scriptHostPath }
      );
      expect(execution.result).toEqual({ count: 4 });
      expect(
        execution.executionOrder.filter(nodeId =>
          ['slow', 'fast', 'middle', 'later'].includes(nodeId)
        )
      ).toEqual(['fast', 'middle', 'later', 'slow']);

      let active = 0;
      let maximumActive = 0;
      for (const event of execution.events) {
        const nodeId = (event.data as { nodeId?: string }).nodeId;
        if (!['slow', 'fast', 'middle', 'later'].includes(nodeId ?? '')) {
          continue;
        }
        if (event.type === 'step_attempt_started') {
          active += 1;
          maximumActive = Math.max(maximumActive, active);
        }
        if (
          event.type === 'step_attempt_succeeded' ||
          event.type === 'step_attempt_failed'
        ) {
          active -= 1;
        }
      }
      expect(maximumActive).toBe(2);
      expect(active).toBe(0);
    }
  );

  nativeTest(
    'carries large parallel context and results while preserving byte limits',
    async () => {
      const contextWorkflow = compileSource(
        largeParallelContextSource,
        'large-parallel-context.woml'
      );
      const contextExecution = await executeWorkflowWithRust(contextWorkflow, {
        nativeCorePath,
        scriptHostPath,
      });
      expect(contextExecution.result).toEqual({ length: 8192 });
      const contextError = await capturedRustExecutionError(
        contextWorkflow,
        {},
        contextLimitedHostPath
      );
      expect(contextError.code).toBe('WOML_PARALLEL_CHILD_FAILED');
      expect(contextError.failedNodeIds).toEqual(['readLength', 'readAgain']);

      const resultWorkflow = compileSource(
        largeParallelResultSource,
        'large-parallel-result.woml'
      );
      const resultExecution = await executeWorkflowWithRust(resultWorkflow, {
        nativeCorePath,
        scriptHostPath,
      });
      expect(resultExecution.result).toEqual({ length: 8192 });
      const resultError = await capturedRustExecutionError(
        resultWorkflow,
        {},
        resultLimitedHostPath
      );
      expect(resultError.code).toBe('WOML_PARALLEL_CHILD_FAILED');
      expect(resultError.failedNodeIds).toEqual(['largeResult']);
    }
  );

  nativeTest(
    'executes branches at the beginning, middle, and end of a workflow',
    async () => {
      const first = await executeWorkflowWithRust(
        compileSource(branchAtBeginningSource),
        { nativeCorePath, scriptHostPath, trigger: { primary: true } }
      );
      expect(first.result).toEqual({ selected: 'primary' });
      expect(first.executionOrder).toEqual(['primary', 'route', 'finish']);
      expect(first.context.steps.fallback).toBeUndefined();

      const middle = await executeWorkflowWithRust(await branchWorkflow(), {
        nativeCorePath,
        scriptHostPath,
      });
      expect(middle.executionOrder).toEqual([
        'checkContent',
        'reviewContent',
        'decision',
        'publishDecision',
      ]);

      const last = await executeWorkflowWithRust(
        compileSource(branchAtEndSource),
        {
          nativeCorePath,
          scriptHostPath,
        }
      );
      expect(last.result).toEqual({ selected: 'primary' });
      expect(last.terminalNodeId).toBe('route');
      expect(last.executionOrder).toEqual(['choice', 'primary', 'route']);
    }
  );

  nativeTest(
    'executes nested branches through the complete frontend and Rust path',
    async () => {
      const execution = await executeWorkflowWithRust(
        compileSource(nestedBranchSource),
        {
          nativeCorePath,
          scriptHostPath,
          trigger: { outer: true, inner: true },
        }
      );

      expect(execution.result).toEqual({ selected: 'nested' });
      expect(execution.executionOrder).toEqual([
        'nested',
        'inner',
        'outer',
        'finish',
      ]);
      expect(execution.context.steps.innerFallback).toBeUndefined();
      expect(execution.context.steps.outerFallback).toBeUndefined();
      expect(
        execution.events
          .filter(event => event.type === 'branch_selected')
          .map(event => event.data)
      ).toEqual([
        { branchId: 'outer', armId: 'outer:when:0' },
        { branchId: 'inner', armId: 'inner:when:0' },
      ]);
    }
  );

  nativeTest(
    'selects only the first true when through the production boundary',
    async () => {
      const execution = await executeWorkflowWithRust(
        compileSource(multipleTrueSource),
        { nativeCorePath, scriptHostPath }
      );

      expect(execution.result).toEqual({ selected: 'first' });
      expect(execution.executionOrder).toEqual([
        'flags',
        'firstRoute',
        'route',
      ]);
      expect(execution.context.steps.secondRoute).toBeUndefined();
      expect(execution.context.steps.fallback).toBeUndefined();
    }
  );

  nativeTest(
    'carries large branch results and preserves script-host size limits',
    async () => {
      const workflow = compileSource(largeBranchResultSource);
      const execution = await executeWorkflowWithRust(workflow, {
        nativeCorePath,
        scriptHostPath,
        trigger: { primary: true },
      });
      expect(execution.result).toEqual({ length: 131072 });
      expect(
        (execution.context.steps.route as { payload: string }).payload.length
      ).toBe(131072);

      const contextError = await capturedRustExecutionError(
        workflow,
        {
          trigger: { primary: true },
        },
        contextLimitedHostPath
      );
      expect(contextError.code).toBe('WOML_SCRIPT_CONTEXT_TOO_LARGE');
      expect(contextError.nodeId).toBe('finish');

      const resultError = await capturedRustExecutionError(
        workflow,
        {
          trigger: { primary: true },
        },
        resultLimitedHostPath
      );
      expect(resultError.code).toBe('WOML_SCRIPT_RESULT_TOO_LARGE');
      expect(resultError.nodeId).toBe('largeResult');
    }
  );

  nativeTest(
    'preserves structured branch identity, site, and reference details',
    async () => {
      const fixturePath = resolve(
        packageRoot,
        '../woml/tests/fixtures/branch.woml'
      );
      const source = await Bun.file(fixturePath).text();
      const conditionWorkflow = compileWoml(
        parseWoml(source.replace('needsReview: true', 'otherProperty: true'), {
          file: fixturePath,
        })
      );
      const conditionError =
        await capturedRustExecutionError(conditionWorkflow);
      expect(conditionError.code).toBe('WOML_REFERENCE_NOT_AVAILABLE');
      expect(conditionError.branchId).toBe('decision');
      expect(conditionError.armId).toBe('decision:when:0');
      expect(conditionError.referencePath).toEqual([
        'steps',
        'checkContent',
        'needsReview',
      ]);
      expect(conditionError.branchSite).toBe('test');
      expect(conditionError.nodeId).toBeUndefined();

      const resultWorkflow = compileWoml(
        parseWoml(
          source.replace(
            '{{context.steps.reviewContent}}',
            '{{context.steps.reviewContent.missing}}'
          ),
          { file: fixturePath }
        )
      );
      const resultError = await capturedRustExecutionError(resultWorkflow);
      expect(resultError.code).toBe('WOML_REFERENCE_NOT_AVAILABLE');
      expect(resultError.branchId).toBe('decision');
      expect(resultError.armId).toBe('decision:when:0');
      expect(resultError.referencePath).toEqual([
        'steps',
        'reviewContent',
        'missing',
      ]);
      expect(resultError.branchSite).toBe('result');
    }
  );

  nativeTest(
    'preserves script throw, timeout, invalid result, and host crash',
    async () => {
      const workflow = await helloWorkflow();

      expect(
        await rustError(
          replaceFirstScript(workflow, 'throw new Error("boom");')
        )
      ).toContain('WOML_SCRIPT_THROWN');
      expect(
        await rustError(replaceFirstScript(workflow, 'while (true) {}'), {
          scriptTimeoutMs: 40,
        })
      ).toContain('WOML_SCRIPT_TIMEOUT');
      expect(
        await rustError(replaceFirstScript(workflow, 'return undefined;'))
      ).toContain('WOML_SCRIPT_NON_JSON_RESULT');
      expect(
        await rustError(workflow, { scriptHostPath: crashingHostPath })
      ).toContain('WOML_SCRIPT_HOST_CRASHED');
    }
  );

  nativeTest(
    'persists a completed run and reconstructs it through the native recovery API',
    async () => {
      const directory = await mkdtemp(join(tmpdir(), 'woml-r4-native-'));
      const database = join(directory, 'runs.sqlite');
      try {
        const workflow = await helloWorkflow();
        const execution = await executeWorkflowWithRustDurable(
          workflow,
          database,
          {
            nativeCorePath,
            scriptHostPath,
          }
        );
        expect(execution.result).toEqual({ message: 'Hello World' });
        expect(execution.events).toHaveLength(6);

        const recovery = recoverDurableRuns(database, { nativeCorePath });
        expect(recovery).toEqual({
          inspectedRuns: 1,
          recoveredRuns: 0,
          interruptedAttempts: 0,
          resumableRuns: 0,
        });
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    }
  );
});
