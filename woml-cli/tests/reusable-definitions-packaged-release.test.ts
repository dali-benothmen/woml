import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

const projectRoot = resolve(import.meta.dir, '../..');
let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-reusable-package-'));
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('reusable definitions clean package', () => {
  test('an installed package compiles providers and executes composed custom steps', async () => {
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'bun-cache');
    const workflows = join(consumer, 'workflows');
    await Promise.all(
      [consumer, cache, workflows].map(path =>
        mkdir(path, { recursive: true })
      )
    );

    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-reusable-clean-consumer', private: true })
    );
    await installLocalReleaseCandidate(consumer, { cache });

    const reusableFixtures = resolve(
      projectRoot,
      'woml/tests/fixtures/reusable-production'
    );
    for (const name of [
      'composition.woml',
      'scale-value.woml',
      'read-secret.woml',
      'math.ts',
    ]) {
      await cp(resolve(reusableFixtures, name), resolve(workflows, name));
    }
    const providerFixtures = resolve(
      projectRoot,
      'woml/tests/fixtures/reusable-definitions'
    );
    for (const name of ['custom-provider-workflow.woml', 'telegram.woml']) {
      await cp(resolve(providerFixtures, name), resolve(workflows, name));
    }

    const executable = join(consumer, 'node_modules/.bin/woml');
    const checked = Bun.spawnSync(
      [executable, 'check', resolve(workflows, 'custom-provider-workflow.woml'), '--json'],
      { cwd: consumer, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(checked.exitCode, checked.stderr.toString()).toBe(0);
    const definitionPackage = JSON.parse(checked.stdout.toString());
    expect(definitionPackage).toMatchObject({
      profile: 'woml.definition-package/v9',
      runtimeReady: true,
      workflow: { model: { schemaVersion: 14 } },
    });
    expect(
      definitionPackage.artifacts.some((artifact: { path: string }) =>
        artifact.path.includes('.on-complete.')
      )
    ).toBe(true);

    const executed = Bun.spawnSync(
      [
        executable,
        'test',
        resolve(workflows, 'composition.woml'),
        '--state',
        resolve(consumer, 'state.sqlite'),
      ],
      {
        cwd: consumer,
        env: {
          ...process.env,
          WOML_SECRETS_PROVIDER: 'env',
          WOML_SECRET_REUSABLE_TEST_TOKEN: 'clean-package-secret',
        },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );
    expect(executed.exitCode, executed.stderr.toString()).toBe(0);
    expect(JSON.parse(executed.stdout.toString())).toEqual({
      switchValue: 6,
      chooseValue: 12,
      parallelValue: 60,
      forkValue: 120,
      secretConfigured: true,
    });
    expect(
      `${executed.stdout.toString()}${executed.stderr.toString()}`
    ).not.toContain('clean-package-secret');
  }, 40_000);
});
