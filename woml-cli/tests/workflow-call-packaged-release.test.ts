import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const exampleDirectory = join(projectRoot, 'examples', 'workflowCalls');
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

async function waitFor(process: CapturedProcess, text: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (!process.stderr().includes(text)) {
    if (process.child.exitCode !== null) {
      await process.stderrDone;
      throw new Error(
        `Packed WOML process exited before ${JSON.stringify(text)}:\n${process.stderr()}`
      );
    }
    if (Date.now() >= deadline) throw new Error(process.stderr());
    await Bun.sleep(10);
  }
}

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-wc7-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Packaged Workflow Calls release journey', () => {
  test('a clean consumer runs a parent and child with the packaged native engine', async () => {
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
      JSON.stringify({ name: 'woml-wc7-clean-consumer', private: true })
    );
    await Bun.write(
      join(consumerDirectory, 'parent.woml'),
      await Bun.file(join(exampleDirectory, 'request-risk.woml')).text()
    );
    await Bun.write(
      join(consumerDirectory, 'child.woml'),
      await Bun.file(join(exampleDirectory, 'calculate-risk.woml')).text()
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
    const runtime = start(
      executable,
      [
        'run',
        'parent.woml',
        'child.woml',
        '--state',
        join(temporaryDirectory, 'packaged-state.sqlite'),
      ],
      consumerDirectory
    );
    await waitFor(runtime, ' result: {"message":"Customer risk score: 90","score":90}');
    const runtimeLog = runtime.stderr();
    runtime.child.kill('SIGINT');
    expect(await runtime.child.exited).toBe(0);
    await runtime.stderrDone;
    expect(runtimeLog).toContain('WOML runtime is ready with 1 registered trigger.');
    expect(runtimeLog).toContain('Workflow call ');
    expect(runtimeLog).toContain(' started child run_call_');
    expect(runtimeLog).not.toContain('customer-42');

    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'dist',
          `woml-core.${process.platform}-${process.arch}.node`
        )
      ).exists()
    ).toBe(true);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'dist',
          'script-host.js'
        )
      ).exists()
    ).toBe(true);
  }, 45_000);
});
