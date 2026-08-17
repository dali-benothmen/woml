import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
let temporaryDirectory: string;

interface CapturedProcess {
  readonly child: ReturnType<typeof Bun.spawn>;
  readonly stderr: () => string;
  readonly stderrDone: Promise<void>;
}

function start(executable: string, args: readonly string[], cwd: string): CapturedProcess {
  const child = Bun.spawn([executable, ...args], {
    cwd,
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
        `Packaged WOML exited before ${JSON.stringify(text)}:\n${process.stderr()}`
      );
    }
    if (Date.now() >= deadline) throw new Error(process.stderr());
    await Bun.sleep(10);
  }
}

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-lec8-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Packaged Lifecycle and Engine Controls release', () => {
  test('a clean consumer can execute lifecycle, inspect, cancel, and shut down', async () => {
    const packageDirectory = join(temporaryDirectory, 'package');
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [packageDirectory, consumerDirectory, bunTemporaryDirectory, bunCacheDirectory].map(
        directory => mkdir(directory, { recursive: true })
      )
    );
    await Bun.write(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-lec8-clean-consumer', private: true })
    );
    await Bun.write(
      join(consumerDirectory, 'lifecycle.woml'),
      `<woml>
<workflow id="lec8-packaged-lifecycle" name="Packaged lifecycle" version="1.0.0">
  <lifecycle>
    <on-start><script>return;</script></on-start>
    <on-success><script>return;</script></on-success>
    <on-complete><script>return;</script></on-complete>
  </lifecycle>
  <triggers><manual id="start" /></triggers>
  <steps><step id="finish"><script>return { packaged: true };</script></step></steps>
</workflow>
</woml>`
    );
    await Bun.write(
      join(consumerDirectory, 'cancellable.woml'),
      `<woml>
<workflow id="lec8-packaged-cancellable" name="Packaged cancellation" version="1.0.0">
  <lifecycle>
    <on-cancel><script>return;</script></on-cancel>
    <on-complete><script>return;</script></on-complete>
  </lifecycle>
  <triggers><manual id="start" /></triggers>
  <steps><step id="wait"><script>await new Promise(resolve => setTimeout(resolve, 10000)); return { late: true };</script></step></steps>
</workflow>
</woml>`
    );

    const packed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        'pm',
        'pack',
        '--ignore-scripts',
        '--destination',
        packageDirectory,
      ],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode).toBe(0);
    const archive = (await readdir(packageDirectory))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(packageDirectory, name))[0];
    expect(archive).toBeDefined();

    const installed = Bun.spawnSync(
      [Bun.which('bun')!, 'add', archive!, '--no-save'],
      {
        cwd: consumerDirectory,
        env: {
          ...process.env,
          TMPDIR: bunTemporaryDirectory,
          BUN_INSTALL_CACHE_DIR: bunCacheDirectory,
        },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install packed WOML CLI:\n${installed.stdout.toString()}${installed.stderr.toString()}`
      );
    }
    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const invoke = (...args: string[]) =>
      Bun.spawnSync([executable, ...args], {
        cwd: consumerDirectory,
        stdout: 'pipe',
        stderr: 'pipe',
      });
    expect(invoke('--version').stdout.toString()).toMatch(/^woml \d+\.\d+\.\d+\n$/);
    expect(invoke('--help').stdout.toString()).toContain('Usage: woml cancel');

    const completedState = join(consumerDirectory, 'completed.sqlite');
    const completed = invoke(
      'test',
      'lifecycle.woml',
      '--state',
      completedState
    );
    expect(completed.exitCode).toBe(0);
    expect(completed.stdout.toString()).toBe('{"packaged":true}\n');
    expect(completed.stderr.toString()).toContain('run finalized');
    const listing = invoke('list', '--state', completedState, '--json');
    expect(listing.exitCode).toBe(0);
    const completedRunId = JSON.parse(listing.stdout.toString()).runs[0].runId;
    const inspection = invoke(
      'get',
      completedRunId,
      '--state',
      completedState,
      '--json'
    );
    expect(JSON.parse(inspection.stdout.toString())).toMatchObject({
      profile: 'woml.run-inspection/v2',
      status: 'succeeded',
      businessOutcome: 'succeeded',
      lifecycleStatus: 'completed',
    });

    const cancellationState = join(consumerDirectory, 'cancellation.sqlite');
    const runtime = start(
      executable,
      ['run', 'cancellable.woml', '--state', cancellationState],
      consumerDirectory
    );
    let runId: string | undefined;
    const runDeadline = Date.now() + 10_000;
    while (runId === undefined) {
      const active = invoke(
        'list',
        '--status',
        'running',
        '--state',
        cancellationState,
        '--json'
      );
      if (active.exitCode === 0) {
        runId = JSON.parse(active.stdout.toString()).runs[0]?.runId;
      }
      if (Date.now() >= runDeadline) throw new Error(runtime.stderr());
      await Bun.sleep(10);
    }
    const cancelled = invoke(
      'cancel',
      runId,
      '--state',
      cancellationState,
      '--json'
    );
    expect(cancelled.exitCode).toBe(0);
    expect(JSON.parse(cancelled.stdout.toString()).status).toBe('accepted');
    await waitFor(runtime, `Run ${runId} cancelled.`);
    await waitFor(runtime, 'WOML automation is active.');
    const finalInspection = invoke(
      'get',
      runId,
      '--state',
      cancellationState,
      '--json'
    );
    const finalJson = finalInspection.stdout.toString();
    expect(JSON.parse(finalJson)).toMatchObject({
      status: 'cancelled',
      businessOutcome: 'cancelled',
      cancellation: { requested: true },
    });
    expect(finalJson).not.toContain('late');
    expect(finalJson).not.toContain('context');

    runtime.child.kill('SIGINT');
    expect(await runtime.child.exited).toBe(0);
    await runtime.stderrDone;
  }, 60_000);
});
