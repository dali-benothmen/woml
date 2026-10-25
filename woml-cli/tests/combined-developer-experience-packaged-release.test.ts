import { afterAll, describe, expect, test } from 'bun:test';
import { createRequire } from 'node:module';
import { mkdir, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

const roots: string[] = [];
const repositoryRoot = resolve(import.meta.dir, '../..');

afterAll(async () => {
  await Promise.all(roots.map(root => rm(root, { recursive: true, force: true })));
});

function invoke(executable: string, cwd: string, ...args: string[]) {
  return Bun.spawnSync([executable, ...args], {
    cwd,
    env: { ...process.env, NO_COLOR: '1' },
    stdout: 'pipe',
    stderr: 'pipe',
  });
}

function lastJson(source: Uint8Array): unknown {
  const lines = new TextDecoder().decode(source).trim().split(/\r?\n/u);
  for (const line of lines.reverse()) {
    try {
      return JSON.parse(line);
    } catch {
      // Human project-selection output may precede the machine result.
    }
  }
  throw new Error(`Command produced no JSON value:\n${new TextDecoder().decode(source)}`);
}

describe('Combined Developer Experience clean release', () => {
  test('initializes, checks, runs, inspects, logs, backs up, and restores a clean project', async () => {
    const root = await realpath(
      await mkdtemp(join(tmpdir(), 'woml-cdx-release-'))
    );
    roots.push(root);
    const consumer = join(root, 'consumer');
    const cache = join(root, 'cache');
    const temporary = join(root, 'tmp');
    await Promise.all(
      [consumer, cache, temporary].map(path => mkdir(path, { recursive: true }))
    );
    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-cdx-clean-consumer', private: true })
    );
    const candidate = await installLocalReleaseCandidate(consumer, {
      cache,
      temporary,
    });
    const executable = join(consumer, 'node_modules', '.bin', 'woml');
    const installed = join(consumer, 'node_modules', 'woml-cli');

    for (const path of [
      'schemas/project-check.v1.schema.json',
      'schemas/woml-project.v1.schema.json',
      'templates/workflow.json',
      'templates/AGENTS.md',
      'templates/workflows/hello.woml',
      'dist/opentelemetry-runtime.js',
      'dist/opentelemetry-sdk.js',
    ]) {
      expect(await Bun.file(join(installed, path)).exists(), path).toBe(true);
    }

    const addon = createRequire(join(installed, 'dist', 'cli.js'))(
      candidate.nativePackage
    ) as Record<string, unknown>;
    for (const name of [
      'startWomlRemoteLoopbackCoordinator',
      'enqueueWomlRemoteDispatch',
      'prepareWomlRemoteAttempt',
      'executeWomlRemoteAttempt',
    ]) {
      expect(typeof addon[name], name).toBe('function');
    }

    const project = join(consumer, 'demo');
    const initialized = invoke(executable, consumer, 'init', project, '--json');
    expect(initialized.exitCode, initialized.stderr.toString()).toBe(0);
    expect(JSON.parse(initialized.stdout.toString())).toMatchObject({
      profile: 'woml.project-initialization/v1',
      status: 'created',
    });
    const manifest = join(project, 'workflow.json');
    const checked = invoke(executable, project, 'check', '--project', manifest, '--json');
    expect(checked.exitCode, checked.stderr.toString()).toBe(0);
    expect(JSON.parse(checked.stdout.toString())).toMatchObject({
      profile: 'woml.project-check/v1',
      status: 'ready',
    });

    const state = join(project, '.woml', 'state.sqlite');
    const execution = invoke(
      executable,
      project,
      'test',
      '--project',
      manifest,
      '--state',
      state
    );
    expect(execution.exitCode, execution.stderr.toString()).toBe(0);
    expect(lastJson(execution.stdout)).toEqual({
      message: 'Hello from WOML',
    });

    const listed = invoke(executable, project, 'list', '--state', state, '--json');
    expect(listed.exitCode, listed.stderr.toString()).toBe(0);
    const runs = JSON.parse(listed.stdout.toString()).runs as { runId: string }[];
    expect(runs).toHaveLength(1);
    const runId = runs[0]!.runId;
    const inspected = invoke(executable, project, 'get', runId, '--state', state, '--json');
    expect(inspected.exitCode, inspected.stderr.toString()).toBe(0);
    expect(JSON.parse(inspected.stdout.toString())).toMatchObject({
      runId,
      status: 'succeeded',
    });
    const logs = invoke(
      executable,
      project,
      'logs',
      runId,
      '--no-follow',
      '--state',
      state,
      '--json'
    );
    expect(logs.exitCode, logs.stderr.toString()).toBe(0);

    const backupDirectory = join(project, 'backup');
    const backup = invoke(
      executable,
      project,
      'backup',
      backupDirectory,
      '--state',
      state,
      '--json'
    );
    expect(backup.exitCode, backup.stderr.toString()).toBe(0);
    expect(JSON.parse(backup.stdout.toString())).toMatchObject({ verified: true });
    const restoredState = join(project, 'restored', 'state.sqlite');
    const restore = invoke(
      executable,
      project,
      'restore',
      backupDirectory,
      '--state',
      restoredState,
      '--json'
    );
    expect(restore.exitCode, restore.stderr.toString()).toBe(0);
    const restored = invoke(
      executable,
      project,
      'get',
      runId,
      '--state',
      restoredState,
      '--json'
    );
    expect(restored.exitCode, restored.stderr.toString()).toBe(0);
    expect(JSON.parse(restored.stdout.toString()).runId).toBe(runId);

    const examples = invoke(
      executable,
      consumer,
      'check',
      join(repositoryRoot, 'examples'),
      '--json'
    );
    expect(examples.exitCode, examples.stderr.toString()).toBe(0);
  }, 120_000);
});
