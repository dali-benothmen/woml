import { afterAll, describe, expect, test } from 'bun:test';
import { copyFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  packagedAddonName,
  rustLibraryName,
} from '../scripts/stage-native';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const packagedCli = resolve(packageRoot, 'dist/cli.js');
const packagedNative = resolve(
  packageRoot,
  'dist',
  packagedAddonName(process.platform, process.arch),
);
const temporaryDirectories: string[] = [];

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(directory =>
      rm(directory, { recursive: true, force: true })
    )
  );
});

describe('WOML native release boundary', () => {
  test('keeps stable package names for every supported host', () => {
    expect(rustLibraryName('linux')).toBe('libwoml_core.so');
    expect(rustLibraryName('darwin')).toBe('libwoml_core.dylib');
    expect(rustLibraryName('win32')).toBe('woml_core.dll');

    for (const platform of ['linux', 'darwin', 'win32']) {
      for (const architecture of ['x64', 'arm64']) {
        expect(packagedAddonName(platform, architecture)).toBe(
          `woml-core.${platform}-${architecture}.node`
        );
      }
    }

    expect(() => rustLibraryName('freebsd')).toThrow(
      'WOML does not support native builds for freebsd.'
    );
    expect(() => packagedAddonName('linux', 'riscv64')).toThrow(
      'WOML does not support native builds for riscv64.'
    );
  });

  test('honors a development override without the combined legacy addon', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-native-override-'));
    temporaryDirectories.push(directory);
    const override = join(directory, 'development-override.node');
    const state = join(directory, 'state.sqlite');
    await copyFile(packagedNative, override);

    const executed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        packagedCli,
        'test',
        resolve(projectRoot, 'examples/terminalExperience/sequential.woml'),
        '--state',
        state,
      ],
      {
        cwd: projectRoot,
        env: { ...process.env, WOML_RUST_CORE_PATH: override },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );

    expect(executed.exitCode, executed.stderr.toString()).toBe(0);
    expect(executed.stdout.toString()).toBe('{"message":"Hello Dali"}\n');

    const missing = Bun.spawnSync(
      [Bun.which('bun')!, packagedCli, 'list', '--json'],
      {
        cwd: directory,
        env: {
          ...process.env,
          WOML_RUST_CORE_PATH: join(directory, 'missing.node'),
        },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );
    expect(missing.exitCode).not.toBe(0);
    expect(missing.stderr.toString()).toContain('missing.node');
  });
});
