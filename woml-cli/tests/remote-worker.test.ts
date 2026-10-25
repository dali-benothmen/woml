import { afterEach, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { checkWorkerConfiguration, RemoteWorkerError } from '../src/remote-worker';

const temporary: string[] = [];

afterEach(async () => {
  await Promise.all(temporary.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function fixture(): Promise<{ root: string; configuration: string }> {
  const root = await mkdtemp(join(tmpdir(), 'woml-worker-'));
  temporary.push(root);
  for (const file of ['worker.crt', 'worker.key', 'ca.crt', 'process-policy.json']) {
    await writeFile(
      join(root, file),
      file === 'process-policy.json'
        ? JSON.stringify({
            contract: 'woml.process-policy', contractVersion: 1, enabled: true,
            modes: { foreground: false, background: true }, projectRoot: root,
            executables: [{ alias: 'runtime', path: process.execPath }],
            workingDirectories: ['.'], environment: { inherit: [], allow: [] },
            limits: { maxTimeoutMs: 60_000, maxOutputBytes: 65_536, maxConcurrent: 2 },
          })
        : 'operator-owned'
    );
    if (process.platform !== 'win32') await chmod(join(root, file), 0o600);
  }
  const configuration = join(root, 'worker.json');
  await writeFile(configuration, JSON.stringify({
    profile: 'woml.worker/v1', version: 1, workerId: 'production-runner-01',
    coordinatorUrl: 'https://coordinator.internal:7443', targetIds: ['production-private'], capacity: 4,
    tls: { certificateRef: 'file:worker.crt', privateKeyRef: 'file:worker.key', caRef: 'file:ca.crt' },
    processPolicy: 'process-policy.json', secretProvider: 'environment',
  }));
  return { root, configuration };
}

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: text => { stdout += text; },
    stderr: text => { stderr += text; },
  };
  return { code: await runCli(args, io), stdout, stderr };
}

describe('Remote Worker v1', () => {
  test('checks an mTLS worker configuration without loading material into output', async () => {
    const { configuration } = await fixture();
    const checked = await checkWorkerConfiguration(configuration);
    expect(checked).toMatchObject({
      profile: 'woml.worker-check/v1', valid: true,
      workerId: 'production-runner-01', transport: 'mtls', dispatchEnabled: true,
    });
    const result = await invoke(['worker', 'check', '--config', configuration, '--json']);
    expect(result.code).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual(checked);
    expect(result.stdout).not.toContain('operator-owned');
  });

  test('fails closed for unknown fields, missing files, and symlinked key material', async () => {
    const { root, configuration } = await fixture();
    const source = JSON.parse(await Bun.file(configuration).text());
    source.unreviewed = true;
    await writeFile(configuration, JSON.stringify(source));
    await expect(checkWorkerConfiguration(configuration)).rejects.toBeInstanceOf(RemoteWorkerError);

    delete source.unreviewed;
    source.coordinatorUrl = 'https://coordinator.internal:999999';
    await writeFile(configuration, JSON.stringify(source));
    await expect(checkWorkerConfiguration(configuration)).rejects.toMatchObject({ code: 'WOML_REMOTE_PROTOCOL_INVALID' });

    source.coordinatorUrl = 'https://coordinator.internal:7443';
    source.tls.certificateRef = 'file:missing.crt';
    await writeFile(configuration, JSON.stringify(source));
    await expect(checkWorkerConfiguration(configuration)).rejects.toMatchObject({ code: 'WOML_TARGET_UNAVAILABLE' });

    if (process.platform !== 'win32') {
      await symlink(join(root, 'worker.key'), join(root, 'linked.key'));
      source.tls.certificateRef = 'file:worker.crt';
      source.tls.privateKeyRef = 'file:linked.key';
      await writeFile(configuration, JSON.stringify(source));
      await expect(checkWorkerConfiguration(configuration)).rejects.toMatchObject({ code: 'WOML_REMOTE_PROTOCOL_INVALID' });
    }
  });

  test('keeps worker command arguments explicit', async () => {
    expect((await invoke(['worker', 'check'])).code).toBe(2);
    expect((await invoke(['worker', 'drain', '--config', 'worker.json'])).stderr).toContain('woml worker start');
  });
});
