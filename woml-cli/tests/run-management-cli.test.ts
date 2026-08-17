import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import packageMetadata from '../package.json' with { type: 'json' };

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const helloWorkflow = join(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'hello.woml'
);
let temporaryDirectory: string;

function invoke(...args: string[]) {
  return Bun.spawnSync([cliPath, ...args], {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
}

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the  CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-lec7-cli-'));
}, 120_000);

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Direct run management', () => {
  test('prints global help and version information', () => {
    for (const flag of ['--version', '-v']) {
      const version = invoke(flag);
      expect(version.exitCode).toBe(0);
      expect(version.stdout.toString()).toBe(
        `woml ${packageMetadata.version}\n`
      );
      expect(version.stderr.toString()).toBe('');
    }

    for (const flag of ['--help', '-h']) {
      const help = invoke(flag);
      expect(help.exitCode).toBe(0);
      expect(help.stdout.toString()).toContain('Usage: woml run');
      expect(help.stdout.toString()).toContain('Usage: woml list');
      expect(help.stdout.toString()).toContain('Usage: woml cancel');
      expect(help.stderr.toString()).toBe('');
    }
  });

  test('lists an empty store and validates filters before opening Rust', () => {
    const statePath = join(temporaryDirectory, 'empty.sqlite');
    const empty = invoke('list', '--state', statePath, '--json');
    expect(empty.exitCode).toBe(0);
    expect(JSON.parse(empty.stdout.toString())).toEqual({
      profile: 'woml.run-list/v2',
      runs: [],
    });

    const invalidStatus = invoke('list', '--status', 'done');
    expect(invalidStatus.exitCode).toBe(2);
    expect(invalidStatus.stderr.toString()).toContain(
      'WOML_RUN_STATUS_INVALID'
    );
    const invalidLimit = invoke('list', '--limit', '201');
    expect(invalidLimit.exitCode).toBe(2);
    expect(invalidLimit.stderr.toString()).toContain(
      'WOML_RUN_LIMIT_INVALID'
    );
  });

  test('lists and safely inspects a completed run', () => {
    const statePath = join(temporaryDirectory, 'completed.sqlite');
    const execution = invoke('test', helloWorkflow, '--state', statePath);
    expect(execution.exitCode).toBe(0);

    const listing = invoke(
      'list',
      '--workflow',
      'hello',
      '--status',
      'succeeded',
      '--limit',
      '1',
      '--state',
      statePath,
      '--json'
    );
    expect(listing.exitCode).toBe(0);
    const listed = JSON.parse(listing.stdout.toString());
    expect(listed.profile).toBe('woml.run-list/v2');
    expect(listed.runs).toHaveLength(1);
    const runId = listed.runs[0].runId as string;

    const inspection = invoke('get', runId, '--state', statePath, '--json');
    expect(inspection.exitCode).toBe(0);
    const inspected = JSON.parse(inspection.stdout.toString());
    expect(inspected).toMatchObject({
      profile: 'woml.run-inspection/v2',
      runId,
      workflowId: 'hello',
      status: 'succeeded',
      businessOutcome: 'succeeded',
      cancellation: { requested: false },
    });
    expect(JSON.stringify(inspected)).not.toContain('Hello World');
    expect(JSON.stringify(inspected)).not.toContain('context');

    const human = invoke('get', runId, '--state', statePath);
    expect(human.exitCode).toBe(0);
    expect(human.stdout.toString()).toContain(`Run: ${runId}`);
    expect(human.stdout.toString()).toContain('Business outcome: succeeded');

    const terminalCancellation = invoke(
      'cancel',
      runId,
      '--state',
      statePath,
      '--json'
    );
    expect(terminalCancellation.exitCode).toBe(1);
    expect(JSON.parse(terminalCancellation.stdout.toString())).toMatchObject({
      profile: 'woml.run-control.result/v1',
      runId,
      status: 'rejected',
      code: 'WOML_RUN_OUTCOME_ALREADY_DECIDED',
    });
  });

  test('cancels a live run from another process and reports the durable state', async () => {
    const statePath = join(temporaryDirectory, 'active.sqlite');
    const workflowPath = join(temporaryDirectory, 'cancellable.woml');
    await writeFile(
      workflowPath,
      `<woml>
  <workflow id="lec7-cancellable" name="cancellable" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="wait">
        <script>
          await new Promise(resolve => setTimeout(resolve, 10000));
          return { completed: true };
        </script>
      </step>
    </steps>
  </workflow>
</woml>`,
      'utf8'
    );
    const child = Bun.spawn(
      [cliPath, 'test', workflowPath, '--state', statePath],
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
    const runDeadline = Date.now() + 10_000;
    let runId: string | undefined;
    while (runId === undefined) {
      const listing = invoke(
        'list',
        '--status',
        'running',
        '--state',
        statePath,
        '--json'
      );
      if (listing.exitCode === 0) {
        runId = JSON.parse(listing.stdout.toString()).runs[0]?.runId;
      }
      if (Date.now() >= runDeadline) throw new Error(stderr);
      await Bun.sleep(10);
    }

    const first = invoke('cancel', runId, '--state', statePath, '--json');
    expect(first.exitCode).toBe(0);
    expect(JSON.parse(first.stdout.toString())).toMatchObject({
      runId,
      status: 'accepted',
    });
    const second = invoke('cancel', runId, '--state', statePath, '--json');
    expect(second.exitCode).toBe(0);
    expect(['already_requested', 'already_cancelled']).toContain(
      JSON.parse(second.stdout.toString()).status
    );

    const cancelDeadline = Date.now() + 10_000;
    while (!stderr.includes('WOML_RUN_CANCELLED')) {
      if (Date.now() >= cancelDeadline) throw new Error(stderr);
      await Bun.sleep(10);
    }
    expect(await child.exited).toBe(1);
    await stderrDone;
    expect(stderr).toContain(runId);
    const inspection = invoke('get', runId, '--state', statePath, '--json');
    expect(inspection.exitCode).toBe(0);
    expect(JSON.parse(inspection.stdout.toString())).toMatchObject({
      status: 'cancelled',
      businessOutcome: 'cancelled',
      cancellation: { requested: true },
    });
  }, 30_000);

  test('reports unknown runs and removes the old namespace', () => {
    const statePath = join(temporaryDirectory, 'unknown.sqlite');
    const missing = invoke('get', 'run_missing', '--state', statePath);
    expect(missing.exitCode).toBe(1);
    expect(missing.stderr.toString()).toContain('WOML_RUN_NOT_FOUND');

    const removed = invoke('runs', 'get', 'run_missing');
    expect(removed.exitCode).toBe(2);
    expect(removed.stderr.toString()).toContain(
      'The "woml runs" namespace was removed.'
    );
  });
});
