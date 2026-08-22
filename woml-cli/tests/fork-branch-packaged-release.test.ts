import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

const projectRoot = resolve(import.meta.dir, '../..');
let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-fork-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('fork and branch clean package', () => {
  test('a clean consumer installs the CLI and runs the social distribution workflow', async () => {
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'bun-cache');
    await Promise.all([consumer, cache].map(path => mkdir(path, { recursive: true })));

    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-fork-clean-consumer', private: true })
    );
    await installLocalReleaseCandidate(consumer, { cache });

    const workflowPath = join(consumer, 'social-distribution.woml');
    await writeFile(
      workflowPath,
      await readFile(resolve(projectRoot, 'examples/forkDistributionWorkflow.woml'), 'utf8')
    );
    const executed = Bun.spawnSync(
      [
        process.execPath,
        join(consumer, 'node_modules/woml-cli/dist/cli.js'),
        'test',
        workflowPath,
        '--state',
        join(consumer, 'state.sqlite'),
      ],
      { cwd: consumer, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(executed.exitCode, executed.stderr.toString()).toBe(0);
    expect(JSON.parse(executed.stdout.toString())).toEqual({
      campaign: 'WOML launch',
      published: ['tiktok', 'instagram', 'facebook', 'pinterest'],
    });
  }, 30_000);
});
