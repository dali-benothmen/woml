import { afterAll, describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import {
  packedPackageFiles,
  publicPackageFiles,
} from '../scripts/release-contract';
import { prepareMainPackage } from '../scripts/native-release';
import { verifyMainPackage } from '../scripts/release-package';

const cliRoot = resolve(import.meta.dir, '..');
const temporaryDirectories: string[] = [];

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(path => rm(path, { recursive: true, force: true })),
  );
});

describe('WOML release package', () => {
  test('makes direct source publication fail closed and allowlists every file', async () => {
    const manifest = await Bun.file(resolve(cliRoot, 'package.json')).json() as {
      readonly files?: readonly string[];
      readonly scripts?: Readonly<Record<string, string>>;
    };

    expect(manifest.files).toEqual(publicPackageFiles);
    expect(manifest.scripts?.prepublishOnly).toBe(
      'bun scripts/guard-source-publish.ts',
    );
    expect(packedPackageFiles).not.toContain('dist/woml-core.linux-x64.node');

    const guarded = Bun.spawnSync({
      cmd: [process.execPath, 'run', 'prepublishOnly'],
      cwd: cliRoot,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(guarded.exitCode).not.toBe(0);
    expect(guarded.stderr.toString()).toContain(
      'Publishing from woml-cli/ is disabled',
    );
  });

  test('stages the exact portable package and validates its runtime paths', async () => {
    const directory = await mkdtemp(resolve(tmpdir(), 'woml-package-test-'));
    temporaryDirectories.push(directory);

    await prepareMainPackage(directory);
    await expect(verifyMainPackage(directory)).resolves.toBeUndefined();

    const cli = await readFile(resolve(directory, 'dist/cli.js'), 'utf8');
    expect(cli.startsWith('#!/usr/bin/env bun')).toBe(true);
  });
});
