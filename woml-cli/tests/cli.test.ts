import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const helloFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'hello.woml',
);
let temporaryDirectory: string;

interface CommandResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number;
}

async function runCli(...args: string[]): Promise<CommandResult> {
  const process = Bun.spawn([cliPath, ...args], {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });

  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(process.stdout).text(),
    new Response(process.stderr).text(),
    process.exited,
  ]);

  return { stdout, stderr, exitCode };
}

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the CLI:\n${build.stdout.toString()}${build.stderr.toString()}`,
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-cli-phase4-'));
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('woml run', () => {
  test('has no production dependency on the TypeScript workflow executor', async () => {
    const source = await Bun.file(join(packageRoot, 'src', 'cli.ts')).text();
    expect(source).toContain('executeWorkflowWithRust');
    expect(source).not.toMatch(/\bexecuteWorkflow\s*\(/);
  });

  test('runs hello.woml through the public executable', async () => {
    const expected = JSON.parse(
      await Bun.file(
        join(packageRoot, 'tests', 'fixtures', 'hello.cli.v0.1.json'),
      ).text(),
    );

    const result = await runCli('run', helloFixturePath);

    expect(result).toEqual({
      stdout: expected.stdout,
      stderr: expected.stderr,
      exitCode: expected.exitCode,
    });
  });

  test('rejects invalid command arguments with usage and exit code 2', async () => {
    const result = await runCli('hello.woml');

    expect(result.stdout).toBe('');
    expect(result.stderr).toBe('Usage: woml run <workflow.woml>\n');
    expect(result.exitCode).toBe(2);
  });

  test('reports a missing workflow file without printing success output', async () => {
    const result = await runCli('run', 'missing.woml');

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML input error [WOML_FILE_NOT_FOUND]');
    expect(result.stderr).toContain('missing.woml');
    expect(result.exitCode).toBe(1);
  });

  test('reports source diagnostics with phase, file, line, and column', async () => {
    const workflowPath = join(temporaryDirectory, 'invalid.woml');
    await writeFile(workflowPath, '<workflow>');

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML parse error');
    expect(result.stderr).toContain(`${workflowPath}:1:`);
    expect(result.exitCode).toBe(1);
  });

  test('reports script failures with the runtime phase and step ID', async () => {
    const workflowPath = join(temporaryDirectory, 'failure.woml');
    await writeFile(
      workflowPath,
      `<workflow woml-version="0.1" id="failure">
  <triggers><manual id="start" /></triggers>
  <steps><step id="broken"><script>throw new Error("boom");</script></step></steps>
</workflow>`,
    );

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML runtime error [WOML_SCRIPT_FAILED]');
    expect(result.stderr).toContain(`${workflowPath}:3:`);
    expect(result.stderr).toContain('step "broken"');
    expect(result.stderr).toContain('boom');
    expect(result.exitCode).toBe(1);
  });

  test('runs from a clean package installation with its native Rust engine', async () => {
    const packageDirectory = join(temporaryDirectory, 'package');
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await mkdir(packageDirectory, { recursive: true });
    await mkdir(consumerDirectory, { recursive: true });
    await mkdir(bunTemporaryDirectory, { recursive: true });
    await mkdir(bunCacheDirectory, { recursive: true });
    await Bun.write(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-clean-install-test', private: true }),
    );
    await Bun.write(
      join(consumerDirectory, 'hello.woml'),
      await Bun.file(helloFixturePath).text(),
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
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' },
    );
    expect(packed.exitCode).toBe(0);
    const archive = (await readdir(packageDirectory))
      .filter((name) => name.endsWith('.tgz'))
      .map((name) => join(packageDirectory, name))[0];
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
      },
    );
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install packed WOML CLI:\n${installed.stdout.toString()}${installed.stderr.toString()}`,
      );
    }
    const entriesBeforeRun = (await readdir(consumerDirectory)).sort();
    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const result = Bun.spawnSync([executable, 'run', 'hello.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });

    expect(result.stdout.toString()).toBe('{"message":"Hello World"}\n');
    expect(result.stderr.toString()).toBe('');
    expect(result.exitCode).toBe(0);
    expect((await readdir(consumerDirectory)).sort()).toEqual(entriesBeforeRun);
    expect(
      await Bun.file(
        join(
          consumerDirectory,
          'node_modules',
          'woml-cli',
          'dist',
          `woml-core.${process.platform}-${process.arch}.node`,
        ),
      ).exists(),
    ).toBe(true);
  });
});
