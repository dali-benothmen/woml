import { afterAll, describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { listRunsWithRust } from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const nativeCorePath = join(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const workflowPath = join(projectRoot, 'examples', 'switchWorkflow.woml');
const helloWorkflow = join(projectRoot, 'woml', 'tests', 'fixtures', 'hello.woml');
const scriptPath = Bun.which('script');
const ptyTest = process.platform === 'win32' || scriptPath === null ? test.skip : test;
const directories: string[] = [];

afterAll(async () => {
  await Promise.all(directories.map(path => rm(path, { recursive: true, force: true })));
});

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

async function waitUntil(
  predicate: () => boolean,
  description: string,
  output: () => string,
  timeoutMs = 15_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${description}:\n${output()}`);
    }
    await Bun.sleep(10);
  }
}

describe('packaged manual trigger terminal experience', () => {
  ptyTest('waits for Enter, admits repeated runs, and drains on Ctrl+C', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-pty-'));
    directories.push(directory);
    const statePath = join(directory, 'state.sqlite');
    const command = [
      shellQuote(cliPath),
      'run',
      shellQuote(workflowPath),
      '--state',
      shellQuote(statePath),
      '--color=never',
    ].join(' ');
    const child = Bun.spawn(
      [scriptPath!, '-qefc', command, '/dev/null'],
      { cwd: projectRoot, stdin: 'pipe', stdout: 'pipe', stderr: 'pipe' }
    );
    let output = '';
    const read = async (stream: ReadableStream<Uint8Array>): Promise<void> => {
      const reader = stream.getReader();
      const decoder = new TextDecoder();
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        output += decoder.decode(chunk.value, { stream: true });
      }
      output += decoder.decode();
    };
    const stdoutDone = read(child.stdout);
    const stderrDone = read(child.stderr);

    try {
      await waitUntil(
        () => output.includes('Ready · Press Enter to run'),
        'the manual ready prompt',
        () => output
      );
      expect(listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs).toHaveLength(0);

      child.stdin.write('\n');
      await child.stdin.flush();
      await waitUntil(
        () => (output.match(/RUN COMPLETED/g) ?? []).length >= 1,
        'the first completed run',
        () => output
      );

      child.stdin.write('\n');
      await child.stdin.flush();
      await waitUntil(
        () => (output.match(/RUN COMPLETED/g) ?? []).length >= 2,
        'the second completed run',
        () => output
      );
      const runs = listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs;
      expect(runs).toHaveLength(2);
      expect(new Set(runs.map(run => run.runId)).size).toBe(2);

      child.stdin.write('\x03');
      await child.stdin.flush();
      const exitCode = await Promise.race([
        child.exited,
        Bun.sleep(10_000).then(() => undefined),
      ]);
      if (exitCode === undefined) throw new Error(`Ctrl+C did not drain the runtime:\n${output}`);
      expect(exitCode).toBe(0);
      await Promise.all([stdoutDone, stderrDone]);
      expect(output).toContain('WOML runtime is draining');
      expect(output).toContain('WOML automation stopped.');
    } finally {
      if (child.exitCode === null) child.kill('SIGTERM');
    }
  }, 45_000);

  test('keeps woml test as the non-interactive one-shot command', () => {
    const result = Bun.spawnSync(
      [cliPath, 'test', helloWorkflow],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(result.exitCode).toBe(0);
    expect(result.stdout.toString()).toContain('{"message":"Hello World"}');
    expect(result.stderr.toString()).not.toContain('WOML_MANUAL_TRIGGER_TTY_REQUIRED');
  });

  test('explains why a manual-only workflow cannot run in the background', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-background-packaged-'));
    directories.push(directory);
    const result = Bun.spawnSync(
      [
        cliPath,
        'run',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
        '--background',
      ],
      { cwd: projectRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(result.exitCode).toBe(2);
    expect(result.stderr.toString()).toContain(
      'WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE'
    );
    expect(result.stderr.toString()).toContain('nobody can press Enter');
  });
});
