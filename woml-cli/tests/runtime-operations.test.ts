import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import {
  readRuntimeDescriptor,
  requestRuntimeStop,
  runtimeDescriptorPath,
  startRuntimeControl,
  writeRuntimeDescriptor,
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

async function temporaryDirectory(label: string): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), `woml-pro3-${label}-`));
  temporaryDirectories.push(directory);
  return directory;
}

async function availablePort(): Promise<number> {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = server.port;
  server.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a local port.');
  return port;
}

async function output(process: ReturnType<typeof Bun.spawn>): Promise<{
  readonly exitCode: number;
  readonly stdout: string;
  readonly stderr: string;
}> {
  const piped = process as unknown as {
    readonly exited: Promise<number>;
    readonly stdout: ReadableStream<Uint8Array>;
    readonly stderr: ReadableStream<Uint8Array>;
  };
  const [exitCode, stdout, stderr] = await Promise.all([
    piped.exited,
    new Response(piped.stdout).text(),
    new Response(piped.stderr).text(),
  ]);
  return { exitCode, stdout, stderr };
}

describe('Owner-only runtime control', () => {
  test('requires the capability and exact runtime identity', async () => {
    const control = await startRuntimeControl({
      runtimeInstanceId: 'runtime_control_test',
      deploymentId: 'deployment_control_test',
      port: 0,
    });
    try {
      const unauthorized = await fetch(
        `${control.descriptor.adminUrl}/v1/control`,
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            profile: 'woml.runtime-admin-http/v1',
            kind: 'request',
            requestId: 'request_unauthorized',
            operation: 'stop',
            subjectId: 'runtime_control_test',
          }),
        }
      );
      expect(unauthorized.status).toBe(401);

      const wrongSubject = await fetch(
        `${control.descriptor.adminUrl}/v1/control`,
        {
          method: 'POST',
          headers: {
            authorization: `Bearer ${control.descriptor.capability}`,
            'content-type': 'application/json',
          },
          body: JSON.stringify({
            profile: 'woml.runtime-admin-http/v1',
            kind: 'request',
            requestId: 'request_wrong_subject',
            operation: 'stop',
            subjectId: 'runtime_someone_else',
          }),
        }
      );
      expect(wrongSubject.status).toBe(400);

      expect(await requestRuntimeStop(control.descriptor)).toBe('requested');
      await expect(control.stopRequested).resolves.toBeUndefined();
    } finally {
      await control.close();
    }
  });

  test('writes descriptor mode 0600 and removes only the exact instance', async () => {
    const directory = await temporaryDirectory('descriptor');
    const path = join(directory, 'runtime.json');
    const control = await startRuntimeControl({
      runtimeInstanceId: 'runtime_descriptor_test',
      deploymentId: 'deployment_descriptor_test',
      port: 0,
    });
    try {
      await writeRuntimeDescriptor(path, control.descriptor);
      expect((await stat(path)).mode & 0o777).toBe(0o600);
      expect((await readRuntimeDescriptor(path)).runtimeInstanceId).toBe(
        'runtime_descriptor_test'
      );
    } finally {
      await control.close();
    }
  });

  test('cleans a descriptor only after its recorded process is proven absent', async () => {
    const directory = await temporaryDirectory('stale');
    const statePath = join(directory, 'state.sqlite');
    const descriptorPath = runtimeDescriptorPath(statePath);
    await writeRuntimeDescriptor(descriptorPath, {
      profile: 'woml.runtime-descriptor/v1',
      runtimeInstanceId: 'runtime_stale',
      deploymentId: 'deployment_stale',
      pid: 2_147_483_647,
      adminUrl: 'http://127.0.0.1:65534',
      capability: 'a'.repeat(43),
      createdAt: new Date().toISOString(),
      expiresAt: new Date(Date.now() + 60_000).toISOString(),
    });
    let stderr = '';
    const io: CliIo = {
      stdout: () => {},
      stderr: value => {
        stderr += value;
      },
    };
    expect(await runCli(['stop', '--state', statePath], io)).toBe(1);
    expect(stderr).toContain('WOML_RUNTIME_STALE_DESCRIPTOR');
    expect(await Bun.file(descriptorPath).exists()).toBe(false);
  });
});

describe('Packaged background journey', () => {
  test('detaches after authenticated readiness, rejects a second owner, and stops exactly', async () => {
    if (!existsSync(packagedCli) || !existsSync(nativeCore)) return;
    const directory = await temporaryDirectory('background');
    const workflowPath = join(directory, 'automation.woml');
    const statePath = join(directory, '.woml/state.sqlite');
    await writeFile(
      workflowPath,
      `<woml><workflow id="pro3-background"><triggers><interval id="heartbeat" every="1h" on-missed="skip" /></triggers><steps><step id="done"><script>return { ready: true };</script></step></steps></workflow></woml>`
    );

    const started = await output(
      Bun.spawn(
        [
          process.execPath,
          packagedCli,
          'run',
          workflowPath,
          '-d',
          '--state',
          statePath,
        ],
        { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
      )
    );
    expect(started.exitCode).toBe(0);
    expect(started.stdout).toContain('WOML runtime started in the background.');
    const descriptorPath = runtimeDescriptorPath(statePath);
    const descriptor = await readRuntimeDescriptor(descriptorPath);
    expect(descriptor.pid).not.toBe(process.pid);
    expect((await stat(descriptorPath)).mode & 0o777).toBe(0o600);

    const duplicate = await output(
      Bun.spawn(
        [
          process.execPath,
          packagedCli,
          'run',
          workflowPath,
          '--state',
          statePath,
        ],
        { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
      )
    );
    expect(duplicate.exitCode).toBe(1);
    expect(duplicate.stderr).toContain('WOML_DEPLOYMENT_ALREADY_RUNNING');

    const stopped = await output(
      Bun.spawn(
        [process.execPath, packagedCli, 'stop', '--state', statePath, '--json'],
        { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
      )
    );
    expect(stopped.exitCode).toBe(0);
    expect(JSON.parse(stopped.stdout)).toMatchObject({
      profile: 'woml.background-runtime-control/v1',
      kind: 'stop',
      runtimeInstanceId: descriptor.runtimeInstanceId,
      status: 'stopped',
    });
    expect(await Bun.file(descriptorPath).exists()).toBe(false);
    expect(
      await Bun.file(join(directory, '.woml/logs/runtime.log')).exists()
    ).toBe(true);
  }, 45_000);

  test('returns a bounded startup failure instead of claiming readiness', async () => {
    if (!existsSync(packagedCli) || !existsSync(nativeCore)) return;
    const directory = await temporaryDirectory('failure');
    const workflowPath = join(directory, 'broken.woml');
    const statePath = join(directory, '.woml/state.sqlite');
    await writeFile(workflowPath, '<woml><workflow');
    const failed = await output(
      Bun.spawn(
        [
          process.execPath,
          packagedCli,
          'run',
          workflowPath,
          '--background',
          '--state',
          statePath,
        ],
        { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
      )
    );
    expect(failed.exitCode).toBe(1);
    expect(failed.stderr).toContain('WOML_MALFORMED_MARKUP');
    expect(await Bun.file(runtimeDescriptorPath(statePath)).exists()).toBe(
      false
    );
    expect(await Bun.file(join(directory, '.woml/logs/runtime.log')).exists()).toBe(false);
  }, 15_000);

  test('first SIGTERM drains slow work and a second SIGTERM forces exit', async () => {
    if (!existsSync(packagedCli) || !existsSync(nativeCore)) return;
    const directory = await temporaryDirectory('signals');
    const workflowPath = join(directory, 'slow.woml');
    const statePath = join(directory, '.woml/state.sqlite');
    const publicPort = await availablePort();
    await writeFile(
      workflowPath,
      `<woml><workflow id="pro3-slow"><triggers><webhook id="start" path="/slow" method="POST" auth="none"><schema>{"type":"object","additionalProperties":false}</schema></webhook></triggers><steps><step id="slow"><script>await new Promise(resolve => setTimeout(resolve, 10_000)); return { done: true };</script></step></steps></workflow></woml>`
    );
    const child = Bun.spawn(
      [
        process.execPath,
        packagedCli,
        'run',
        workflowPath,
        '--port',
        String(publicPort),
        '--state',
        statePath,
      ],
      { cwd: directory, stdout: 'ignore', stderr: 'pipe' }
    );
    const childStderr = new Response(
      (child as unknown as { stderr: ReadableStream<Uint8Array> }).stderr
    ).text();
    try {
      const descriptorPath = runtimeDescriptorPath(statePath);
      const readyDeadline = Date.now() + 10_000;
      while (
        Date.now() < readyDeadline &&
        !(await Bun.file(descriptorPath).exists())
      ) {
        await Bun.sleep(25);
      }
      if (!(await Bun.file(descriptorPath).exists())) {
        if (child.exitCode === null) child.kill('SIGKILL');
        throw new Error(`Slow runtime did not become ready: ${await childStderr}`);
      }
      const admitted = await fetch(`http://127.0.0.1:${publicPort}/slow`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{}',
      });
      expect(admitted.status).toBe(202);
      await Bun.sleep(100);
      child.kill('SIGTERM');
      await Bun.sleep(200);
      expect(child.exitCode).toBeNull();
      const forcedAt = performance.now();
      child.kill('SIGTERM');
      expect(await child.exited).not.toBe(0);
      expect(performance.now() - forcedAt).toBeLessThan(2_000);
    } finally {
      if (child.exitCode === null) child.kill('SIGKILL');
    }
  }, 15_000);
});
