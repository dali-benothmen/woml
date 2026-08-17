import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = join(packageRoot, 'dist', 'cli.js');
const fixtures = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'workflow-calls'
);
const parentWorkflow = join(fixtures, 'request-risk.woml');
const childWorkflow = join(fixtures, 'calculate-risk.woml');
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

async function waitFor(
  process: CapturedProcess,
  text: string,
  timeoutMs = 15_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!process.stderr().includes(text)) {
    if (process.child.exitCode !== null) {
      await process.stderrDone;
      throw new Error(
        `WOML process exited before ${JSON.stringify(text)}:\n${process.stderr()}`
      );
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

beforeAll(async () => {
  await chmod(executable, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-wc5-cli-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Local Workflow Call routing', () => {
  test('two woml run processes communicate through shared local state', async () => {
    const state = join(temporaryDirectory, 'cross-process.sqlite');
    const target = start(['run', childWorkflow, '--state', state]);
    await waitFor(target, 'WOML runtime is ready with 0 registered triggers.');

    const parent = start(['run', parentWorkflow, '--state', state]);
    await waitFor(parent, 'result: {"score":90}');
    expect(parent.stderr()).toContain('Run ');
    expect(parent.stderr()).toContain(' succeeded.');

    await stop(parent);
    await stop(target);
  }, 30_000);

  test('explicit files form the same runtime unit as a workflow directory', async () => {
    const state = join(temporaryDirectory, 'multiple-files.sqlite');
    const runtime = start([
      'run',
      parentWorkflow,
      childWorkflow,
      '--state',
      state,
    ]);
    await waitFor(runtime, 'result: {"score":90}');
    expect(runtime.stderr()).toContain(
      'WOML runtime is ready with 1 registered trigger.'
    );
    await stop(runtime);
  }, 30_000);
});
