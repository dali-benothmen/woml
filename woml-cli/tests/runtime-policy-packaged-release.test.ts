import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-rp7-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Packaged Runtime Policies release', () => {
  test('a clean consumer can check, execute, list, and inspect a policy workflow', async () => {
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [consumerDirectory, bunTemporaryDirectory, bunCacheDirectory].map(
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

    await installLocalReleaseCandidate(consumerDirectory, {
      cache: bunCacheDirectory,
      temporary: bunTemporaryDirectory,
    });

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
