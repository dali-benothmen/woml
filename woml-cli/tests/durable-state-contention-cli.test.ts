import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const executable = join(packageRoot, 'dist', 'cli.js');
let directory: string;

function writer(): string {
  return `<woml>
<workflow id="ds4-packaged-contention" name="packaged contention" version="1.0.0">
  <config concurrency="4" />
  <triggers><manual id="start" /></triggers>
  <steps><step id="increment"><script>
    let finalSeen = 0;
    for (const index of Array.from({ length: 16 }, (_, value) => value)) {
      const result = await services.state.increment('shared-counter', 1, {
        name: 'increment-' + index
      });
      finalSeen = result.value;
    }
    return { finalSeen };
  </script></step></steps>
</workflow>
</woml>`;
}

const reader = `<woml>
<workflow id="ds4-packaged-contention" name="packaged reader" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="read"><script>
    return await services.state.get('shared-counter');
  </script></step></steps>
</workflow>
</woml>`;

async function run(path: string, state: string) {
  const child = Bun.spawn([executable, 'test', path, '--state', state], {
    cwd: directory,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const [exitCode, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  return { exitCode, stdout, stderr };
}

beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), 'woml-ds4-packaged-'));
  await chmod(executable, 0o755);
}, 10_000);

afterAll(async () => {
  if (process.env.WOML_DS4_KEEP_FIXTURE !== '1') {
    await rm(directory, { recursive: true, force: true });
  }
});

describe('Packaged CLI contention', () => {
  test('two packaged processes preserve all independently named increments', async () => {
    const workflow = join(directory, 'writer.woml');
    const read = join(directory, 'read.woml');
    const state = join(directory, 'state.sqlite');
    await Promise.all([
      Bun.write(workflow, writer()),
      Bun.write(read, reader),
    ]);

    // Initialize and migrate the shared database before the two independent
    // packaged processes race only on normal run/state transactions.
    const initialized = await run(read, state);
    expect(initialized.exitCode, initialized.stderr).toBe(0);

    const [firstResult, secondResult] = await Promise.all([
      run(workflow, state),
      run(workflow, state),
    ]);
    expect(firstResult.exitCode, firstResult.stderr).toBe(0);
    expect(secondResult.exitCode, secondResult.stderr).toBe(0);
    expect(firstResult.stderr).not.toContain('shared-counter');
    expect(secondResult.stderr).not.toContain('shared-counter');

    const readResult = await run(read, state);
    expect(readResult.exitCode, readResult.stderr).toBe(0);
    expect(JSON.parse(readResult.stdout)).toEqual({
      found: true,
      value: 32,
      version: 32,
      updatedAt: expect.any(String),
    });
  }, 30_000);
});
