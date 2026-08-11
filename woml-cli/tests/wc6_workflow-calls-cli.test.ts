import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { formatWorkflowCallProgress } from '../src/cli';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = join(packageRoot, 'dist', 'cli.js');
const moduleExample = join(projectRoot, 'examples', 'workflowCallsModule');
let temporaryDirectory: string;

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

function start(args: readonly string[]): CapturedProcess {
  const child = Bun.spawn([executable, ...args], {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  let stderr = '';
  const stderrDone = (async () => {
    const reader = child.stderr.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      stderr += decoder.decode(chunk.value, { stream: true });
    }
    stderr += decoder.decode();
  })();
  return { child, stderr: () => stderr, stderrDone };
}

async function waitFor(process: CapturedProcess, text: string): Promise<void> {
  const deadline = Date.now() + 20_000;
  while (!process.stderr().includes(text)) {
    if (process.child.exitCode !== null) {
      await process.stderrDone;
      throw new Error(process.stderr());
    }
    if (Date.now() >= deadline) throw new Error(process.stderr());
    await Bun.sleep(10);
  }
}

async function stop(process: CapturedProcess): Promise<void> {
  if (process.child.exitCode === null) process.child.kill('SIGINT');
  expect(await process.child.exited).toBe(0);
  await process.stderrDone;
}

function availablePort(): number {
  const probe = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = probe.port;
  probe.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a test port.');
  return port;
}

beforeAll(async () => {
  await chmod(executable, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-wc6-cli-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('WC6 Workflow Call product journey', () => {
  test('formats only safe workflow call progress', () => {
    const message = formatWorkflowCallProgress({
      contract: 'woml.workflow-call-progress',
      contractVersion: 1,
      type: 'call_admitted',
      parentRunId: 'run_parent',
      parentNodeId: 'requestRisk',
      targetWorkflowId: 'calculate-risk',
      childRunId: 'run_child',
      duplicate: false,
      occurredAt: '2026-08-11T12:00:00.000Z',
    });
    expect(message).toContain('run_parent/requestRisk');
    expect(message).toContain('run_child');
    expect(message).not.toContain('payload');
    expect(message).not.toContain('secret');
  });

  test('calls a module-backed child and inspects both durable runs', async () => {
    const state = join(temporaryDirectory, 'module-call.sqlite');
    const runtime = start(['run', moduleExample, '--state', state]);
    await waitFor(runtime, 'result: {"score":90}');
    const progress = runtime.stderr();
    const ids = progress.match(
      /Workflow call (run_[^/]+)\/requestRisk started child (run_call_[^ ]+) for/
    );
    expect(ids).not.toBeNull();
    const [, parentRunId, childRunId] = ids!;

    for (const [runId, relation] of [
      [parentRunId!, 'childCalls'],
      [childRunId!, 'parentCall'],
    ] as const) {
      const inspection = Bun.spawnSync([
        executable,
        'runs',
        'get',
        runId,
        '--state',
        state,
      ], { cwd: projectRoot });
      expect(inspection.exitCode).toBe(0);
      const decoded = JSON.parse(inspection.stdout.toString());
      expect(decoded.workflowCalls[relation]).toBeDefined();
      expect(JSON.stringify(decoded.workflowCalls)).not.toContain('customer-42');
      expect(JSON.stringify(decoded.workflowCalls)).not.toContain('payloadDigest');
    }
    await stop(runtime);
  }, 30_000);

  test('explains the Human Approval limitation before admitting a child', async () => {
    const directory = join(temporaryDirectory, 'approval-target');
    await mkdir(directory, { recursive: true });
    await writeFile(
      join(directory, 'parent.woml'),
      `<woml><workflow id="approval-caller"><triggers><manual id="start" /></triggers>
        <steps><step id="callApproval"><script>
          return services.workflows.call('approval-target', {});
        </script></step></steps></workflow></woml>`
    );
    await writeFile(
      join(directory, 'target.woml'),
      `<woml><workflow id="approval-target"><triggers>
        <webhook id="incoming" path="/wc6-approval" method="POST" auth="none">
          <schema>{"type":"object"}</schema>
        </webhook>
      </triggers><steps>
        <approval id="review" timeout="24h" on-timeout="reject">
          <when-approved><step id="approved"><script>return { approved: true };</script></step></when-approved>
          <when-rejected><step id="rejected"><script>return { approved: false };</script></step></when-rejected>
        </approval>
      </steps></workflow></woml>`
    );
    const state = join(temporaryDirectory, 'approval-call.sqlite');
    const runtime = start([
      'run',
      directory,
      '--state',
      state,
      '--port',
      String(availablePort()),
    ]);
    await waitFor(runtime, 'WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED');
    expect(runtime.stderr()).toContain('contains Human Approval');
    expect(runtime.stderr()).toContain('Run it independently');
    expect(runtime.stderr()).not.toContain('started child');
    await stop(runtime);
  }, 30_000);
});
