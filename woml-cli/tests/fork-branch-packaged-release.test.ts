import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
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
    const archives = join(temporaryDirectory, 'archives');
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [archives, consumer, cache].map(path => mkdir(path, { recursive: true }))
    );

    const packed = Bun.spawnSync(
      [Bun.which('bun')!, 'pm', 'pack', '--ignore-scripts', '--destination', archives],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode, packed.stderr.toString()).toBe(0);
    const archive = (await readdir(archives))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(archives, name))[0];
    expect(archive).toBeDefined();

    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-fork-clean-consumer', private: true })
    );
    const installed = Bun.spawnSync([Bun.which('bun')!, 'add', archive!, '--no-save'], {
      cwd: consumer,
      env: { ...process.env, BUN_INSTALL_CACHE_DIR: cache },
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(
      installed.exitCode,
      `${installed.stdout.toString()}${installed.stderr.toString()}`
    ).toBe(0);

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
