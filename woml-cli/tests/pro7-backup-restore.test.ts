import { afterEach, describe, expect, test } from 'bun:test';
import {
  mkdtemp,
  mkdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  readRuntimeDescriptor,
  requestRuntimeStop,
  runtimeDescriptorPath,
} from '../src/runtime-control';

const packageRoot = resolve(import.meta.dir, '..');
const executable = join(packageRoot, 'dist/cli.js');
const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map(path => rm(path, { recursive: true, force: true }))
  );
});

async function directory(label: string): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), `woml-pro7-${label}-`));
  temporaryDirectories.push(path);
  return path;
}

async function invoke(cwd: string, ...args: string[]) {
  const child = Bun.spawn([process.execPath, executable, ...args], {
    cwd,
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

const writer = `<woml>
<workflow id="pro7-state" name="PRO7 state writer" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="increment"><script>
    return await services.state.increment('visits', 1, { name: 'visit' });
  </script></step></steps>
</workflow>
</woml>`;

const reader = `<woml>
<workflow id="pro7-state" name="PRO7 state reader" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="read"><script>
    return await services.state.get('visits');
  </script></step></steps>
</workflow>
</woml>`;

async function seed(root: string) {
  const writerPath = join(root, 'writer.woml');
  const readerPath = join(root, 'reader.woml');
  const statePath = join(root, 'source/state.sqlite');
  await Promise.all([
    writeFile(writerPath, writer),
    writeFile(readerPath, reader),
  ]);
  const result = await invoke(root, 'test', writerPath, '--state', statePath);
  expect(result.exitCode, result.stderr).toBe(0);
  return { writerPath, readerPath, statePath };
}

async function freePort(): Promise<number> {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response('reserved'),
  });
  const port = server.port!;
  await server.stop(true);
  return port;
}

describe('PRO7 packaged backup and restore', () => {
  test('creates a frozen verified manifest and restores exact durable user state', async () => {
    const root = await directory('journey');
    const { writerPath, readerPath, statePath } = await seed(root);
    const backupPath = join(root, 'backups/first');
    const backup = await invoke(
      root,
      'backup',
      backupPath,
      '--state',
      statePath,
      '--json'
    );
    expect(backup.exitCode, backup.stderr).toBe(0);
    const manifest = JSON.parse(backup.stdout);
    expect(manifest).toMatchObject({
      profile: 'woml.backup-manifest/v1',
      storeVersion: 14,
      database: {
        file: 'state.sqlite',
        sizeBytes: expect.any(Number),
        digest: expect.stringMatching(/^sha256:[0-9a-f]{64}$/),
      },
      verified: true,
    });
    expect(JSON.stringify(manifest)).not.toContain('visits');

    const changed = await invoke(root, 'test', writerPath, '--state', statePath);
    expect(changed.exitCode, changed.stderr).toBe(0);
    const sourceRead = await invoke(root, 'test', readerPath, '--state', statePath);
    expect(JSON.parse(sourceRead.stdout)).toMatchObject({ found: true, value: 2 });

    const restoredState = join(root, 'restored/state.sqlite');
    const restore = await invoke(
      root,
      'restore',
      backupPath,
      '--state',
      restoredState,
      '--json'
    );
    expect(restore.exitCode, restore.stderr).toBe(0);
    expect(JSON.parse(restore.stdout)).toMatchObject({
      profile: 'woml.restore-result/v1',
      backupId: manifest.backupId,
      statePath: restoredState,
      storeVersion: 14,
    });
    const restoredRead = await invoke(root, 'test', readerPath, '--state', restoredState);
    expect(restoredRead.exitCode, restoredRead.stderr).toBe(0);
    expect(JSON.parse(restoredRead.stdout)).toMatchObject({ found: true, value: 1 });
  }, 30_000);

  test('requires explicit replacement and retains a recoverable previous database', async () => {
    const root = await directory('replace');
    const { writerPath, statePath } = await seed(root);
    const backupPath = join(root, 'backup');
    expect(
      (await invoke(root, 'backup', backupPath, '--state', statePath)).exitCode
    ).toBe(0);
    expect(
      (await invoke(root, 'test', writerPath, '--state', statePath)).exitCode
    ).toBe(0);
    const rejected = await invoke(root, 'restore', backupPath, '--state', statePath);
    expect(rejected.exitCode).toBe(1);
    expect(rejected.stderr).toContain('WOML_RESTORE_CONFIRMATION_REQUIRED');
    const restored = await invoke(
      root,
      'restore',
      backupPath,
      '--state',
      statePath,
      '--replace',
      '--json'
    );
    expect(restored.exitCode, restored.stderr).toBe(0);
    const result = JSON.parse(restored.stdout);
    expect(result.rollbackPath).toBeString();
    expect(await Bun.file(result.rollbackPath).exists()).toBe(true);
  }, 30_000);

  test('rejects checksum corruption, partial backups, and symlinked input', async () => {
    const root = await directory('hostile');
    const { statePath } = await seed(root);
    const backupPath = join(root, 'backup');
    expect(
      (await invoke(root, 'backup', backupPath, '--state', statePath)).exitCode
    ).toBe(0);
    const unexpectedPath = join(backupPath, 'unexpected.txt');
    await writeFile(unexpectedPath, 'not part of Backup Manifest v1');
    const unexpected = await invoke(
      root,
      'restore',
      backupPath,
      '--state',
      join(root, 'unexpected/state.sqlite')
    );
    expect(unexpected.exitCode).toBe(1);
    expect(unexpected.stderr).toContain('WOML_BACKUP_INCOMPLETE');
    await rm(unexpectedPath);
    const databasePath = join(backupPath, 'state.sqlite');
    const bytes = await readFile(databasePath);
    bytes[Math.floor(bytes.length / 2)]! ^= 0xff;
    await writeFile(databasePath, bytes);
    const corrupted = await invoke(
      root,
      'restore',
      backupPath,
      '--state',
      join(root, 'corrupt/state.sqlite')
    );
    expect(corrupted.exitCode).toBe(1);
    expect(corrupted.stderr).toContain('WOML_BACKUP_CHECKSUM_MISMATCH');

    const partial = join(root, 'partial');
    await mkdir(partial);
    await writeFile(join(partial, 'manifest.json'), '{}');
    const incomplete = await invoke(
      root,
      'restore',
      partial,
      '--state',
      join(root, 'partial-target/state.sqlite')
    );
    expect(incomplete.stderr).toContain('WOML_BACKUP_INCOMPLETE');

    if (process.platform !== 'win32') {
      const linked = join(root, 'linked-backup');
      const link = Bun.spawnSync(['ln', '-s', backupPath, linked]);
      expect(link.exitCode).toBe(0);
      const unsafe = await invoke(
        root,
        'restore',
        linked,
        '--state',
        join(root, 'linked-target/state.sqlite')
      );
      expect(unsafe.stderr).toContain('WOML_BACKUP_PATH_UNSAFE');
    }
  }, 30_000);

  test('refuses to replace a live runtime target', async () => {
    const root = await directory('active');
    const source = await seed(root);
    const backupPath = join(root, 'backup');
    expect(
      (await invoke(root, 'backup', backupPath, '--state', source.statePath)).exitCode
    ).toBe(0);
    const workflowPath = join(root, 'active.woml');
    const targetState = join(root, 'active/state.sqlite');
    const configPath = join(root, 'runtime.json');
    await writeFile(
      workflowPath,
      `<woml><workflow id="pro7-active"><triggers><interval id="tick" every="1h" on-missed="skip" /></triggers><steps><step id="done"><script>return { ok: true };</script></step></steps></workflow></woml>`
    );
    await writeFile(
      configPath,
      JSON.stringify({
        schemaVersion: 1,
        statePath: targetState,
        admin: { host: '127.0.0.1', port: await freePort() },
      })
    );
    const runtime = Bun.spawn(
      [process.execPath, executable, 'run', workflowPath, '--config', configPath],
      { cwd: root, stdout: 'pipe', stderr: 'pipe' }
    );
    const descriptorPath = runtimeDescriptorPath(targetState);
    try {
      const deadline = Date.now() + 15_000;
      while (Date.now() < deadline && !(await Bun.file(descriptorPath).exists())) {
        if (runtime.exitCode !== null) break;
        await Bun.sleep(25);
      }
      expect(await Bun.file(descriptorPath).exists()).toBe(true);
      const rejected = await invoke(
        root,
        'restore',
        backupPath,
        '--state',
        targetState,
        '--replace'
      );
      expect(rejected.exitCode).toBe(1);
      expect(rejected.stderr).toContain('WOML_RESTORE_TARGET_ACTIVE');
      await requestRuntimeStop(await readRuntimeDescriptor(descriptorPath));
      expect(await runtime.exited).toBe(0);
    } finally {
      if (runtime.exitCode === null) runtime.kill('SIGKILL');
      await runtime.exited;
    }
  }, 30_000);
});
