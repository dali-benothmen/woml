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
const branchFixturePath = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'branch.woml',
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

  test('runs the selected when route through the public executable', async () => {
    const result = await runCli('run', branchFixturePath);

    expect(result).toEqual({
      stdout: '{"message":"Final status: reviewed"}\n',
      stderr: '',
      exitCode: 0,
    });
  });

  test('runs otherwise when no condition is true', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-otherwise.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'needsReview: false',
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result).toEqual({
      stdout: '{"message":"Final status: accepted-automatically"}\n',
      stderr: '',
      exitCode: 0,
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
      `<workflow version="1.0.0" id="failure">
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

  test('points a non-boolean branch test to its original test attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-non-boolean.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'needsReview: "yes"',
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_BRANCH_TEST_NOT_BOOLEAN]',
    );
    expect(result.stderr).toContain(`${workflowPath}:19:`);
    expect(result.stderr).toContain('<when test> in branch "decision"');
    expect(result.stderr).toContain('must resolve to a JSON boolean');
    expect(result.exitCode).toBe(1);
  });

  test('points a missing condition reference to its original test attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-missing-test.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      'needsReview: true',
      'otherProperty: true',
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_REFERENCE_NOT_AVAILABLE]',
    );
    expect(result.stderr).toContain(`${workflowPath}:19:`);
    expect(result.stderr).toContain('<when test> in branch "decision"');
    expect(result.stderr).toContain(
      'context.steps.checkContent.needsReview',
    );
    expect(result.exitCode).toBe(1);
  });

  test('points a missing result reference to its original result attribute', async () => {
    const workflowPath = join(temporaryDirectory, 'branch-missing-result.woml');
    const source = (await Bun.file(branchFixturePath).text()).replace(
      '{{context.steps.reviewContent}}',
      '{{context.steps.reviewContent.missing}}',
    );
    await writeFile(workflowPath, source);

    const result = await runCli('run', workflowPath);

    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML runtime error [WOML_REFERENCE_NOT_AVAILABLE]',
    );
    expect(result.stderr).toContain(`${workflowPath}:29:`);
    expect(result.stderr).toContain('<result value> in branch "decision"');
    expect(result.stderr).toContain(
      'context.steps.reviewContent.missing',
    );
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
    await Bun.write(
      join(consumerDirectory, 'branch.woml'),
      await Bun.file(branchFixturePath).text(),
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
    const helloResult = Bun.spawnSync([executable, 'run', 'hello.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const branchResult = Bun.spawnSync([executable, 'run', 'branch.woml'], {
      cwd: consumerDirectory,
      stdout: 'pipe',
      stderr: 'pipe',
    });

    expect(helloResult.stdout.toString()).toBe('{"message":"Hello World"}\n');
    expect(helloResult.stderr.toString()).toBe('');
    expect(helloResult.exitCode).toBe(0);
    expect(branchResult.stdout.toString()).toBe(
      '{"message":"Final status: reviewed"}\n',
    );
    expect(branchResult.stderr.toString()).toBe('');
    expect(branchResult.exitCode).toBe(0);
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
