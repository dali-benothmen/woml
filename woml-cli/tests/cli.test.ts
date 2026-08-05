import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
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
  test('runs hello.woml through the public executable', async () => {
    const expected = JSON.parse(
      await Bun.file(
        join(packageRoot, 'tests', 'fixtures', 'hello.cli.v0.1.json'),
      ).text(),
    );

    const result = await runCli('run', 'hello.woml');

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
    expect(result.stderr).toContain('step "broken"');
    expect(result.stderr).toContain('boom');
    expect(result.exitCode).toBe(1);
  });
});
