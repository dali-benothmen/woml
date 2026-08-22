import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { installLocalReleaseCandidate } from './helpers/release-candidate';

const projectRoot = resolve(import.meta.dir, '../..');
let temporaryDirectory: string;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-for-each-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('for-each clean package', () => {
  test('a clean consumer validates and executes the documented example', async () => {
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'bun-cache');
    await Promise.all([consumer, cache].map(path => mkdir(path, { recursive: true })));
    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-for-each-clean-consumer', private: true }),
    );
    await installLocalReleaseCandidate(consumer, { cache });

    const workflowPath = join(consumer, 'for-each.woml');
    await writeFile(
      workflowPath,
      await readFile(resolve(projectRoot, 'examples/forEachWorkflow.woml'), 'utf8'),
    );
    const cli = join(consumer, 'node_modules/woml-cli/dist/cli.js');
    const check = Bun.spawnSync([process.execPath, cli, 'check', workflowPath], {
      cwd: consumer,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(check.exitCode, check.stderr.toString()).toBe(0);
    expect(check.stdout.toString()).toContain('WOML check passed');

    const executed = Bun.spawnSync(
      [
        process.execPath,
        cli,
        'test',
        workflowPath,
        '--state',
        join(consumer, 'workflow-history.sqlite'),
      ],
      { cwd: consumer, stdout: 'pipe', stderr: 'pipe' },
    );
    expect(executed.exitCode, executed.stderr.toString()).toBe(0);
    expect(JSON.parse(executed.stdout.toString())).toEqual({
      processed: 3,
      greetings: [
        { position: 1, message: 'Hello Grace! Your plan is pro.', featured: true },
        { position: 2, message: 'Hello Ada! Your plan is starter.', featured: false },
        { position: 3, message: 'Hello Linus! Your plan is pro.', featured: true },
      ],
    });
  }, 30_000);
});
