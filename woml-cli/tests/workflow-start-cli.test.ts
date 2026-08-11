import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = join(packageRoot, 'dist', 'cli.js');
const exampleDirectory = join(projectRoot, 'examples', 'workflowStartManual');
let temporaryDirectory: string;

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

function startRuntime(state: string): CapturedProcess {
  const child = Bun.spawn(
    [
      executable,
      'run',
      join(exampleDirectory, 'workflow1.woml'),
      join(exampleDirectory, 'workflow2.woml'),
      '--state',
      state,
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
  const result = Bun.spawnSync([
    executable,
    'runs',
    'get',
    runId,
    '--state',
    state,
  ], { cwd: projectRoot });
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return JSON.parse(result.stdout.toString()) as Record<string, unknown>;
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
    const runtime = startRuntime(state);
    await waitFor(runtime, '"message":"The child is running and the parent continued"');
    const resultLine = runtime
      .stderr()
      .split('\n')
      .find(line => line.includes(' result: {"message"'));
    expect(resultLine).toBeDefined();
    const parentResult = JSON.parse(resultLine!.split(' result: ', 2)[1]!) as {
      childRunId: string;
    };

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
