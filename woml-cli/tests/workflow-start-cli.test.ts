import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { inspectRunWithRust } from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = join(packageRoot, 'dist', 'cli.js');
const nativeCorePath = join(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const exampleDirectory = join(projectRoot, 'examples', 'workflowStartManual');
let temporaryDirectory: string;

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

function availablePort(): number {
  const probe = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
  const port = probe.port!;
  probe.stop(true);
  return port;
}

function startRuntime(parent: string, state: string, port: number): CapturedProcess {
  const child = Bun.spawn(
    [
      executable,
      'run',
      parent,
      join(exampleDirectory, 'workflow2.woml'),
      '--state',
      state,
      '--port',
      String(port),
    ],
    { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
  );
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

function inspect(runId: string, state: string): Record<string, unknown> {
  return inspectRunWithRust(state, runId, { nativeCorePath }) as unknown as Record<string, unknown>;
}

beforeAll(async () => {
  await chmod(executable, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-start-cli-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('services.workflows.start()', () => {
  test('returns after durable admission while the child continues independently', async () => {
    const state = join(temporaryDirectory, 'state.sqlite');
    const parentPath = join(temporaryDirectory, 'workflow1-webhook.woml');
    await writeFile(
      parentPath,
      (await readFile(join(exampleDirectory, 'workflow1.woml'), 'utf8')).replace(
        '<manual id="start" />',
        '<webhook id="start" path="/start-background" method="POST" auth="none" />'
      )
    );
    const port = availablePort();
    const runtime = startRuntime(parentPath, state, port);
    await waitFor(runtime, 'WOML automation is active. Press Ctrl+C to stop.');
    const response = await fetch(`http://127.0.0.1:${port}/start-background`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{}',
    });
    expect(response.status).toBe(202);
    const admission = (await response.json()) as { runId: string };
    await waitFor(runtime, 'The child is running and the parent continued');
    const parentResult = inspect(admission.runId, state).result as {
      message: string;
      childRunId: string;
    };
    expect(parentResult).toMatchObject({
      message: 'The child is running and the parent continued',
    });

    const whileParentIsDone = inspect(parentResult.childRunId, state);
    expect(whileParentIsDone.status).toBe('running');

    const deadline = Date.now() + 10_000;
    let child = whileParentIsDone;
    while (child.status !== 'succeeded' && Date.now() < deadline) {
      await Bun.sleep(25);
      child = inspect(parentResult.childRunId, state);
    }
    expect(child.status).toBe('succeeded');
    expect(child.result).toEqual({ result: 42 });
    expect(runtime.stderr()).not.toContain('firstNumber');

    runtime.child.kill('SIGINT');
    expect(await runtime.child.exited).toBe(0);
    await runtime.stderrDone;
  }, 30_000);
});
