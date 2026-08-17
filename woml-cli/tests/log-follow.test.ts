import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  followWorkflowLogs,
  LogFollowError,
  parseLogFollowArguments,
} from '../src/log-follower';
import type { RuntimeDescriptorV1 } from '../src/runtime-control';
import type {
  RunPresentationListV1,
  RunPresentationV1,
} from '../src/terminal-presentation';

const fixturePath = resolve(
  import.meta.dir,
  'fixtures/terminal-presentation/success.v1.json'
);

const descriptor: RuntimeDescriptorV1 = {
  profile: 'woml.runtime-descriptor/v1',
  runtimeInstanceId: 'runtime_logs',
  deploymentId: 'deployment_logs',
  pid: process.pid,
  adminUrl: 'http://127.0.0.1:31999',
  capability: 'a'.repeat(43),
  createdAt: '2026-08-15T09:00:00.000Z',
  expiresAt: '2027-08-15T09:00:00.000Z',
};

async function presentation(): Promise<RunPresentationV1> {
  return await Bun.file(fixturePath).json();
}

function snapshot(workflows = ['order-processing']) {
  return {
    profile: 'woml.runtime-operations-snapshot/v1',
    runtimeInstanceId: descriptor.runtimeInstanceId,
    sequence: 4,
    capturedAt: '2026-08-15T09:00:00.000Z',
    lifecycle: 'ready',
    ready: true,
    uptimeMs: 1_000,
    workflows: workflows.map(workflowId => ({ workflowId })),
    runs: [],
    components: [],
    alerts: [],
  };
}

function streamEvent(sequence = 5): string {
  return `id: ${sequence}\r\nevent: operations\r\ndata: ${JSON.stringify({
    profile: 'woml.runtime-operations-stream/v1',
    runtimeInstanceId: descriptor.runtimeInstanceId,
    sequence,
    occurredAt: '2026-08-15T09:00:01.000Z',
    kind: 'trigger',
    subject: { id: `run_${sequence}`, status: 'occurrence_accepted' },
  })}\r\n\r\n`;
}

async function eventually(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 5_000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error('Timed out waiting for log output.');
    await Bun.sleep(5);
  }
}

describe('workflow log following', () => {
  test('parses direct run/workflow subjects and rejects ambiguous input', () => {
    expect(parseLogFollowArguments(['run_abc', '--logs', '--json'])).toMatchObject({
      subjectKind: 'run',
      json: true,
    });
    expect(parseLogFollowArguments(['order-processing', '--logs'])).toMatchObject({
      subjectKind: 'workflow',
    });
    expect(() => parseLogFollowArguments(['Not A Workflow', '--logs'])).toThrow(
      LogFollowError
    );
    expect(() => parseLogFollowArguments(['order-processing'])).toThrow(
      'Usage: woml <run-id|workflow-id> --logs'
    );
  });

  test('renders a terminal retained run and exits without requiring a runtime', async () => {
    const run = await presentation();
    let stdout = '';
    let stderr = '';
    const code = await followWorkflowLogs({
      args: {
        subject: run.runId,
        subjectKind: 'run',
        statePath: '/virtual/state.sqlite',
        json: true,
        color: 'always',
      },
      io: {
        stdout: text => { stdout += text; },
        stderr: text => { stderr += text; },
        isTTY: false,
      },
      dependencies: {
        readRun: () => run,
        readWorkflow: () => { throw new Error('unexpected workflow query'); },
        readDescriptor: async () => { throw Object.assign(new Error('missing'), { code: 'ENOENT' }); },
      },
    });
    expect(code).toBe(0);
    expect(JSON.parse(stdout)).toMatchObject({
      profile: 'woml.run-presentation/v1',
      runId: run.runId,
      status: 'succeeded',
    });
    expect(stdout).not.toContain('\u001b[');
    expect(stderr).toBe('');
  });

  test('renders retained workflow history oldest-to-newest and explains inactive following', async () => {
    const newest = await presentation();
    const oldest = {
      ...newest,
      runId: 'run_oldest',
      admittedAt: '2026-08-14T08:00:00.000Z',
    };
    let stdout = '';
    let stderr = '';
    const code = await followWorkflowLogs({
      args: {
        subject: 'order-processing',
        subjectKind: 'workflow',
        statePath: '/virtual/state.sqlite',
        json: true,
        color: 'never',
      },
      io: {
        stdout: text => { stdout += text; },
        stderr: text => { stderr += text; },
        isTTY: false,
      },
      dependencies: {
        readWorkflow: () => ({
          profile: 'woml.run-presentation-list/v1',
          workflowId: 'order-processing',
          runs: [newest, oldest],
        }),
        hasWorkflow: () => true,
        readRun: () => { throw new Error('unexpected run query'); },
        readDescriptor: async () => { throw Object.assign(new Error('missing'), { code: 'ENOENT' }); },
      },
    });
    expect(code).toBe(0);
    const records = stdout.trim().split('\n').map(line => JSON.parse(line));
    expect(records.map(record => record.runId)).toEqual(['run_oldest', newest.runId]);
    expect(stderr).toContain('WOML_LOG_RUNTIME_UNAVAILABLE');
  });

  test('recognizes a retained workflow definition even before its first run', async () => {
    let stderr = '';
    const code = await followWorkflowLogs({
      args: {
        subject: 'order-processing',
        subjectKind: 'workflow',
        statePath: '/virtual/state.sqlite',
        json: false,
        color: 'never',
      },
      io: { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: false },
      dependencies: {
        readWorkflow: () => ({
          profile: 'woml.run-presentation-list/v1',
          workflowId: 'order-processing',
          runs: [],
        }),
        hasWorkflow: () => true,
        readRun: () => { throw new Error('unexpected run query'); },
        readDescriptor: async () => { throw Object.assign(new Error('missing'), { code: 'ENOENT' }); },
      },
    });
    expect(code).toBe(0);
    expect(stderr).toContain('WOML_LOG_RUNTIME_UNAVAILABLE');
    expect(stderr).not.toContain('WOML_LOG_WORKFLOW_NOT_FOUND');
  });

  test('uses stream events only to refresh the matching durable workflow projection', async () => {
    const run = await presentation();
    const empty: RunPresentationListV1 = {
      profile: 'woml.run-presentation-list/v1',
      workflowId: 'order-processing',
      runs: [],
    };
    let stdout = '';
    let presentationRequests = 0;
    let resolveDetach!: () => void;
    const detach = new Promise<void>(resolve => { resolveDetach = resolve; });
    const fetcher = (async (input: string) => {
      if (input.endsWith('/v1/snapshot')) return Response.json(snapshot());
      if (input.includes('/v1/presentations/workflows/')) {
        presentationRequests += 1;
        return Response.json({
          ...empty,
          runs: presentationRequests === 1 ? [] : [run],
        });
      }
      if (input.includes('/v1/stream')) {
        return new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode(streamEvent()));
            },
          }),
          { headers: { 'content-type': 'text/event-stream' } }
        );
      }
      return Response.json({}, { status: 404 });
    }) as (input: string, init?: RequestInit) => Promise<Response>;
    const running = followWorkflowLogs({
      args: {
        subject: 'order-processing',
        subjectKind: 'workflow',
        statePath: '/virtual/state.sqlite',
        json: true,
        color: 'never',
      },
      io: { stdout: text => { stdout += text; }, stderr: () => {}, isTTY: false },
      dependencies: {
        readWorkflow: () => empty,
        hasWorkflow: () => true,
        readRun: () => { throw new Error('unexpected run query'); },
        readDescriptor: async () => descriptor,
        fetch: fetcher,
        waitForDetach: () => detach,
      },
    });
    await eventually(() => stdout.includes(run.runId));
    resolveDetach();
    expect(await running).toBe(0);
    expect(stdout.trim().split('\n')).toHaveLength(1);
    expect(JSON.parse(stdout)).toMatchObject({ runId: run.runId });
    expect(presentationRequests).toBeGreaterThanOrEqual(2);
  });

  test('reports a followed run whose retained history is pruned', async () => {
    const run = { ...(await presentation()), status: 'running' as const };
    let stderr = '';
    const fetcher = (async (input: string) => {
      if (input.endsWith('/v1/snapshot')) {
        return Response.json({
          ...snapshot(),
          runs: [{ runId: run.runId, workflowId: run.workflow.id }],
        });
      }
      if (input.includes('/v1/presentations/runs/')) {
        return Response.json(
          { error: { code: 'WOML_RUN_NOT_FOUND' } },
          { status: 404 }
        );
      }
      if (input.includes('/v1/stream')) {
        return new Response(streamEvent(), {
          headers: { 'content-type': 'text/event-stream' },
        });
      }
      return Response.json({}, { status: 404 });
    }) as (input: string, init?: RequestInit) => Promise<Response>;
    const code = await followWorkflowLogs({
      args: {
        subject: run.runId,
        subjectKind: 'run',
        statePath: '/virtual/state.sqlite',
        json: true,
        color: 'never',
      },
      io: { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: false },
      dependencies: {
        readRun: () => run,
        readWorkflow: () => { throw new Error('unexpected workflow query'); },
        readDescriptor: async () => descriptor,
        fetch: fetcher,
        reconnectWindowMs: 10,
      },
    });
    expect(code).toBe(0);
    expect(stderr).toContain('WOML_LOG_HISTORY_PRUNED');
  });

  test('reconnects through a rotated same-deployment runtime descriptor', async () => {
    const run = await presentation();
    const replacement: RuntimeDescriptorV1 = {
      ...descriptor,
      runtimeInstanceId: 'runtime_logs_restarted',
      adminUrl: 'http://127.0.0.1:32000',
      capability: 'b'.repeat(43),
    };
    const empty: RunPresentationListV1 = {
      profile: 'woml.run-presentation-list/v1',
      workflowId: 'order-processing',
      runs: [],
    };
    let descriptorReads = 0;
    let streams = 0;
    let stdout = '';
    let resolveDetach!: () => void;
    const detach = new Promise<void>(resolve => { resolveDetach = resolve; });
    const fetcher = (async (input: string) => {
      if (input.endsWith('/v1/snapshot')) {
        const restarted = input.startsWith(replacement.adminUrl);
        return Response.json({
          ...snapshot(),
          runtimeInstanceId: restarted
            ? replacement.runtimeInstanceId
            : descriptor.runtimeInstanceId,
          sequence: restarted ? 1 : 6,
        });
      }
      if (input.includes('/v1/presentations/workflows/')) {
        return Response.json(
          input.startsWith(replacement.adminUrl) ? { ...empty, runs: [run] } : empty
        );
      }
      if (input.includes('/v1/stream')) {
        streams += 1;
        if (input.startsWith(descriptor.adminUrl)) {
          return new Response('', {
            headers: { 'content-type': 'text/event-stream' },
          });
        }
        return new Response(
          new ReadableStream({ start() {} }),
          { headers: { 'content-type': 'text/event-stream' } }
        );
      }
      return Response.json({}, { status: 404 });
    }) as (input: string, init?: RequestInit) => Promise<Response>;

    const running = followWorkflowLogs({
      args: {
        subject: 'order-processing',
        subjectKind: 'workflow',
        statePath: '/virtual/state.sqlite',
        json: true,
        color: 'never',
      },
      io: { stdout: text => { stdout += text; }, stderr: () => {}, isTTY: false },
      dependencies: {
        readWorkflow: () => empty,
        hasWorkflow: () => true,
        readRun: () => { throw new Error('unexpected run query'); },
        readDescriptor: async () => {
          descriptorReads += 1;
          return descriptorReads < 3 ? descriptor : replacement;
        },
        fetch: fetcher,
        waitForDetach: () => detach,
        reconnectWindowMs: 1_000,
      },
    });
    await eventually(() => stdout.includes(run.runId));
    resolveDetach();
    expect(await running).toBe(0);
    expect(streams).toBeGreaterThanOrEqual(1);
    expect(descriptorReads).toBeGreaterThanOrEqual(3);
  });

  test('reports an actionable error when durable history permissions deny access', async () => {
    const denied = Object.assign(new Error('permission denied: secret path'), {
      code: 'EACCES',
    });
    try {
      await followWorkflowLogs({
        args: {
          subject: 'run_denied',
          subjectKind: 'run',
          statePath: '/private/state.sqlite',
          json: false,
          color: 'never',
        },
        io: { stdout: () => {}, stderr: () => {}, isTTY: false },
        dependencies: {
          readRun: () => { throw denied; },
          readWorkflow: () => { throw new Error('unexpected workflow query'); },
          hasWorkflow: () => false,
          readDescriptor: async () => { throw new Error('unexpected descriptor query'); },
        },
      });
      throw new Error('expected log following to reject denied state access');
    } catch (error) {
      expect(error).toBeInstanceOf(LogFollowError);
      expect((error as LogFollowError).code).toBe('WOML_LOG_STATE_UNAVAILABLE');
      expect((error as Error).message).not.toContain('secret path');
    }
  });

  test('fails closed when a replacement descriptor belongs to another deployment', async () => {
    const empty: RunPresentationListV1 = {
      profile: 'woml.run-presentation-list/v1',
      workflowId: 'order-processing',
      runs: [],
    };
    const foreign: RuntimeDescriptorV1 = {
      ...descriptor,
      runtimeInstanceId: 'runtime_foreign',
      deploymentId: 'deployment_foreign',
      capability: 'c'.repeat(43),
    };
    let descriptorReads = 0;
    let stderr = '';
    const fetcher = (async (input: string) => {
      if (input.endsWith('/v1/snapshot')) return Response.json(snapshot());
      if (input.includes('/v1/presentations/workflows/')) return Response.json(empty);
      return new Response('', { headers: { 'content-type': 'text/event-stream' } });
    }) as (input: string, init?: RequestInit) => Promise<Response>;

    expect(await followWorkflowLogs({
      args: {
        subject: 'order-processing',
        subjectKind: 'workflow',
        statePath: '/virtual/state.sqlite',
        json: false,
        color: 'never',
      },
      io: { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: false },
      dependencies: {
        readWorkflow: () => empty,
        hasWorkflow: () => true,
        readRun: () => { throw new Error('unexpected run query'); },
        readDescriptor: async () => {
          descriptorReads += 1;
          return descriptorReads === 1 ? descriptor : foreign;
        },
        fetch: fetcher,
        reconnectWindowMs: 0,
      },
    })).toBe(0);
    expect(stderr).toContain('WOML_LOG_RUNTIME_UNAVAILABLE');
    expect(stderr).toContain('different deployment');
  });
});
