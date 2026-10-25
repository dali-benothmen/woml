import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, realpath, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await realpath(
    await mkdtemp(join(tmpdir(), 'woml-artifact-package-'))
  );
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('Packaged Durable Artifacts release', () => {
  test('a clean consumer can store, inspect, and explicitly export verified bytes', async () => {
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'cache');
    const temporary = join(temporaryDirectory, 'tmp');
    await Promise.all([consumer, cache, temporary].map(path => mkdir(path, { recursive: true })));
    await Bun.write(join(consumer, 'package.json'), JSON.stringify({ name: 'artifact-consumer', private: true }));
    await Bun.write(join(consumer, 'artifact.woml'), `<woml>
  <workflow id="packaged-artifact" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="create"><script>
        await Bun.write('input/report.txt', 'packaged durable bytes');
        return { ready: true };
      </script></step>
      <step id="preserve"><script>
        const artifact = await services.artifacts.put('input/report.txt', {
          name: 'packaged-report', contentType: 'text/plain', retention: '24h'
        });
        return { artifact };
      </script></step>
    </steps>
  </workflow>
</woml>`);
    const policy = {
      contract: 'woml.artifact-policy', contractVersion: 1, enabled: true,
      modes: { foreground: true, background: false }, projectRoot: consumer,
      readRoots: ['input'],
      materialization: { directory: join(consumer, 'materialized'), cleanupOnStart: true, maxBytesPerAttempt: 1_048_576 },
      limits: { maxArtifactBytes: 1_048_576, maxRunBytes: 2_097_152, maxStoreBytes: 4_194_304, maxConcurrentTransfers: 2, transferTimeoutMs: 30_000 },
      retention: { defaultMs: 86_400_000, minimumMs: 3_600_000, maximumMs: 31_536_000_000 },
      secretScanning: { knownSecretAction: 'reject', minimumSecretBytes: 8 },
      backend: { contract: 'woml.artifact-backend', contractVersion: 1, kind: 'local', directory: join(consumer, 'artifacts') },
    };
    await Bun.write(join(consumer, 'artifact-policy.json'), JSON.stringify(policy));
    await mkdir(join(consumer, 'input'));
    const candidate = await installLocalReleaseCandidate(consumer, { cache, temporary });
    const binaryName = `woml-core.${candidate.target}.node`;
    const installedBinary = join(
      consumer,
      'node_modules',
      ...candidate.nativePackage.split('/'),
      binaryName
    );
    const digest = async (path: string): Promise<string> =>
      new Bun.CryptoHasher('sha256')
        .update(await Bun.file(path).arrayBuffer())
        .digest('hex');
    expect(await digest(installedBinary)).toBe(
      await digest(join(candidate.nativeRoot, binaryName))
    );
    const resolvedBinary = createRequire(
      join(consumer, 'node_modules', 'woml-cli', 'dist', 'cli.js')
    ).resolve(candidate.nativePackage);
    expect(await digest(resolvedBinary)).toBe(
      await digest(join(candidate.nativeRoot, binaryName))
    );
    const executable = join(consumer, 'node_modules', '.bin', 'woml');
    const state = join(consumer, 'state.sqlite');
    const invoke = (...args: string[]) => Bun.spawnSync([executable, ...args], {
      cwd: consumer, stdout: 'pipe', stderr: 'pipe', env: { ...process.env, NO_COLOR: '1' },
    });

    const execution = invoke('test', 'artifact.woml', '--artifact-policy', 'artifact-policy.json', '--state', state);
    expect(execution.exitCode, execution.stderr.toString()).toBe(0);
    const artifact = JSON.parse(execution.stdout.toString()).artifact;
    const inspected = invoke('artifacts', 'inspect', artifact.id, '--state', state, '--json');
    expect(inspected.exitCode, inspected.stderr.toString()).toBe(0);
    expect(JSON.parse(inspected.stdout.toString())).toMatchObject({
      profile: 'woml.artifact-inspection/v1',
      artifact: { id: artifact.id },
    });
    const exported = invoke(
      'artifacts', 'export', artifact.id, '--output', 'exported.txt',
      '--artifact-policy', 'artifact-policy.json', '--state', state, '--json'
    );
    expect(exported.exitCode, exported.stderr.toString()).toBe(0);
    expect(JSON.parse(exported.stdout.toString())).toMatchObject({ verified: true, artifactId: artifact.id });
    expect(await Bun.file(join(consumer, 'exported.txt')).text()).toBe('packaged durable bytes');
  }, 90_000);
});
