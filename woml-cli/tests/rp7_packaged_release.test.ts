import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-rp7-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('RP7 packaged Runtime Policies release', () => {
  test('a clean consumer can check, execute, list, and inspect a policy workflow', async () => {
    const packageDirectory = join(temporaryDirectory, 'package');
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [packageDirectory, consumerDirectory, bunTemporaryDirectory, bunCacheDirectory].map(
        directory => mkdir(directory, { recursive: true })
      )
    );
    await Bun.write(
      join(consumerDirectory, 'package.json'),
      JSON.stringify({ name: 'woml-rp7-clean-consumer', private: true })
    );
    await Bun.write(
      join(consumerDirectory, 'policy.woml'),
      `<woml>
<workflow id="rp7-packaged-policy" name="Packaged policy" version="1.0.0">
  <config concurrency="2" rate-limit="20/1m" timeout="5s" queue="rp7-package" />
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="finish"><script>return { packaged: true };</script></step>
  </steps>
</workflow>
</woml>`
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
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode).toBe(0);
    const archive = (await readdir(packageDirectory))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(packageDirectory, name))[0];
    expect(archive).toBeDefined();

    const installed = Bun.spawnSync([Bun.which('bun')!, 'add', archive!, '--no-save'], {
      cwd: consumerDirectory,
      env: {
        ...process.env,
        TMPDIR: bunTemporaryDirectory,
        BUN_INSTALL_CACHE_DIR: bunCacheDirectory,
      },
      stdout: 'pipe',
      stderr: 'pipe',
    });
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install packed WOML CLI:\n${installed.stdout.toString()}${installed.stderr.toString()}`
      );
    }

    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const invoke = (...args: string[]) =>
      Bun.spawnSync([executable, ...args], {
        cwd: consumerDirectory,
        stdout: 'pipe',
        stderr: 'pipe',
      });
    const checked = invoke('check', 'policy.woml');
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr.toString()).toBe('');
    expect(checked.stdout.toString()).toContain('Model v12 concurrency');
    expect(checked.stdout.toString()).toContain('workflow timeouts are executable');

    const statePath = join(consumerDirectory, 'state.sqlite');
    const executed = invoke('test', 'policy.woml', '--state', statePath);
    expect(executed.exitCode).toBe(0);
    expect(executed.stdout.toString()).toBe('{"packaged":true}\n');
    expect(executed.stderr.toString()).toContain('started under runtime policy');

    const listed = invoke('list', '--state', statePath, '--json');
    expect(listed.exitCode).toBe(0);
    const list = JSON.parse(listed.stdout.toString());
    expect(list).toMatchObject({
      profile: 'woml.run-list/v2',
      runs: [
        {
          workflowId: 'rp7-packaged-policy',
          status: 'succeeded',
          queue: 'rp7-package',
        },
      ],
    });
    const runId = list.runs[0].runId;

    const inspected = invoke('get', runId, '--state', statePath, '--json');
    expect(inspected.exitCode).toBe(0);
    const inspectionText = inspected.stdout.toString();
    expect(JSON.parse(inspectionText)).toMatchObject({
      profile: 'woml.run-inspection/v3',
      runId,
      status: 'succeeded',
      policy: {
        queue: 'rp7-package',
        timeoutAt: expect.any(String),
      },
    });
    expect(inspectionText).not.toContain('context');
    expect(inspectionText).not.toContain('secret');
  }, 60_000);
});
