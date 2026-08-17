import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
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

function availablePort(): number {
  const probe = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
  const port = probe.port!;
  probe.stop(true);
  return port;
}

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
  test('rejects a second persistence authority for the same local state', async () => {
    const state = join(temporaryDirectory, 'cross-process.sqlite');
    const target = start([
      'run',
      childWorkflow,
      '--state',
      state,
      '--port',
      String(availablePort()),
    ]);
    await waitFor(target, 'WOML automation is active. Press Ctrl+C to stop.');

    const webhookParent = join(temporaryDirectory, 'cross-process-parent.woml');
    await writeFile(
      webhookParent,
      (await readFile(parentWorkflow, 'utf8')).replace(
        '<manual id="start" />',
        '<webhook id="start" path="/cross-process-risk" method="POST" auth="none" />'
      )
    );
    const parentPort = availablePort();
    const parent = start([
      'run',
      webhookParent,
      '--state',
      state,
      '--port',
      String(parentPort),
    ]);
    expect(await parent.child.exited).toBe(1);
    await parent.stderrDone;
    expect(parent.stderr()).toContain('WOML_DEPLOYMENT_ALREADY_RUNNING');
    expect(parent.stderr()).toContain('already owned by runtime');
    await stop(target);
  }, 30_000);

  test('explicit files form the same runtime unit as a workflow directory', async () => {
    const state = join(temporaryDirectory, 'multiple-files.sqlite');
    const webhookParent = join(temporaryDirectory, 'request-risk-webhook.woml');
    await writeFile(
      webhookParent,
      (await readFile(parentWorkflow, 'utf8')).replace(
        '<manual id="start" />',
        '<webhook id="start" path="/request-risk" method="POST" auth="none" />'
      )
    );
    const port = availablePort();
    const runtime = start([
      'run',
      webhookParent,
      childWorkflow,
      '--port',
      String(port),
      '--state',
      state,
    ]);
    await waitFor(runtime, 'WOML automation is active. Press Ctrl+C to stop.');
    const response = await fetch(`http://127.0.0.1:${port}/request-risk`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{}',
    });
    expect(response.status).toBe(202);
    await waitFor(runtime, '{ score: 90 }');
    expect(runtime.stderr()).toContain('Workflow call · calculate-risk completed');
    await stop(runtime);
  }, 30_000);
});
