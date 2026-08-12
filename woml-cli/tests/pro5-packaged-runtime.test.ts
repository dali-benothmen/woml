import { afterEach, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  readRuntimeDescriptor,
  requestRuntimeStop,
  runtimeDescriptorPath,
} from '../src/runtime-control';

const packageRoot = resolve(import.meta.dir, '..');
const packagedCli = resolve(packageRoot, 'dist/cli.js');
const nativeCore = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map(path => rm(path, { recursive: true, force: true }))
  );
});

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

test('PRO5 packaged runtime exposes truthful authenticated observability', async () => {
  if (!existsSync(packagedCli) || !existsSync(nativeCore)) return;
  const directory = await mkdtemp(join(tmpdir(), 'woml-pro5-packaged-'));
  temporaryDirectories.push(directory);
  const workflowPath = join(directory, 'automation.woml');
  const statePath = join(directory, '.woml/state.sqlite');
  const configPath = join(directory, 'woml.runtime.json');
  const adminPort = await freePort();
  await writeFile(
    workflowPath,
    `<woml><workflow id="pro5-runtime"><triggers><interval id="heartbeat" every="1h" on-missed="skip" /></triggers><steps><step id="done"><script>return { ready: true };</script></step></steps></workflow></woml>`
  );
  await writeFile(
    configPath,
    JSON.stringify({
      schemaVersion: 1,
      statePath,
      admin: { host: '127.0.0.1', port: adminPort },
      logging: { format: 'json', directory: join(directory, 'logs') },
      observability: { health: true, metrics: true },
    })
  );

  const child = Bun.spawn(
    [process.execPath, packagedCli, 'run', workflowPath, '--config', configPath],
    { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
  );
  const descriptorPath = runtimeDescriptorPath(statePath);
  try {
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline && !(await Bun.file(descriptorPath).exists())) {
      if (child.exitCode !== null) break;
      await Bun.sleep(25);
    }
    expect(await Bun.file(descriptorPath).exists()).toBe(true);
    const descriptor = await readRuntimeDescriptor(descriptorPath);
    const headers = {
      authorization: `Bearer ${descriptor.capability}`,
    };

    expect(
      await fetch(`${descriptor.adminUrl}/readyz`).then(response => response.status)
    ).toBe(200);
    const snapshot = await fetch(`${descriptor.adminUrl}/v1/snapshot`, {
      headers,
    }).then(response => response.json());
    expect(snapshot).toMatchObject({
      profile: 'woml.runtime-operations-snapshot/v1',
      runtimeInstanceId: descriptor.runtimeInstanceId,
      lifecycle: 'ready',
      ready: true,
      workflows: [{ workflowId: 'pro5-runtime' }],
    });
    expect(JSON.stringify(snapshot)).not.toContain('ready: true');

    const metrics = await fetch(`${descriptor.adminUrl}/metrics`, {
      headers,
    }).then(response => response.text());
    expect(metrics).toContain('woml_runtime_ready 1');
    expect(metrics).toContain('woml_workflows_loaded 1');

    await requestRuntimeStop(descriptor);
    expect(await child.exited).toBe(0);
    const stderr = await new Response(child.stderr).text();
    const records = stderr
      .split('\n')
      .filter(line => line.startsWith('{'))
      .map(line => JSON.parse(line));
    expect(records).toContainEqual(
      expect.objectContaining({
        profile: 'woml.runtime-log-record/v1',
        code: 'WOML_RUNTIME_READY',
        runtimeInstanceId: descriptor.runtimeInstanceId,
      })
    );
  } finally {
    if (child.exitCode === null) child.kill('SIGKILL');
    await child.exited;
  }
}, 30_000);
