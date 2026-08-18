import { afterAll, describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { createPlatformPackage } from '../scripts/native-release';
import {
  detectLinuxLibc,
  localNativeBinaryName,
  nativePackageBinaryName,
  nativePackageName,
  nativeTargetForRuntime,
  womlNativeTargets,
  womlNativeTargetSpecs,
} from '../src/native-platform';

const temporaryDirectories: string[] = [];

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(path => rm(path, { recursive: true, force: true })),
  );
});

describe('WOML cross-platform native release', () => {
  test('maps every supported runtime to one exact package', () => {
    expect(nativeTargetForRuntime('darwin', 'x64')).toBe('darwin-x64');
    expect(nativeTargetForRuntime('darwin', 'arm64')).toBe('darwin-arm64');
    expect(nativeTargetForRuntime('win32', 'x64')).toBe('win32-x64-msvc');
    expect(nativeTargetForRuntime('win32', 'arm64')).toBe('win32-arm64-msvc');
    expect(nativeTargetForRuntime('linux', 'x64', 'glibc')).toBe(
      'linux-x64-gnu',
    );
    expect(nativeTargetForRuntime('linux', 'x64', 'musl')).toBe(
      'linux-x64-musl',
    );
    expect(nativeTargetForRuntime('linux', 'arm64', 'glibc')).toBe(
      'linux-arm64-gnu',
    );
    expect(nativeTargetForRuntime('linux', 'arm64', 'musl')).toBe(
      'linux-arm64-musl',
    );
    expect(() => nativeTargetForRuntime('freebsd', 'x64')).toThrow(
      'WOML does not support native builds for freebsd.',
    );
    expect(() => nativeTargetForRuntime('linux', 'riscv64')).toThrow(
      'WOML does not support native builds for riscv64.',
    );
    expect(['glibc', 'musl']).toContain(detectLinuxLibc());
  });

  test('freezes package, binary, Rust target, and local-development names', () => {
    expect(womlNativeTargets).toHaveLength(8);
    for (const target of womlNativeTargets) {
      expect(nativePackageName(target)).toBe(`@woml-org/cli-${target}`);
      expect(nativePackageBinaryName(target)).toBe(`woml-core.${target}.node`);
      expect(womlNativeTargetSpecs[target].target).toBe(target);
      expect(womlNativeTargetSpecs[target].rustTarget).toMatch(
        /^(?:x86_64|aarch64)-/,
      );
    }
    expect(localNativeBinaryName('linux', 'x64')).toBe(
      'woml-core.linux-x64.node',
    );
  });

  test('creates a bounded npm package with platform metadata and license', async () => {
    const root = await mkdtemp(resolve(tmpdir(), 'woml-native-package-'));
    temporaryDirectories.push(root);
    const artifact = resolve(root, 'libwoml_core.so');
    const output = resolve(root, 'package');
    await writeFile(artifact, 'test-native-bytes');
    await createPlatformPackage('linux-x64-musl', artifact, output);

    const manifest = JSON.parse(
      await readFile(resolve(output, 'package.json'), 'utf8'),
    ) as Record<string, unknown>;
    expect(manifest.name).toBe('@woml-org/cli-linux-x64-musl');
    expect(manifest.version).toBe('1.0.1');
    expect(manifest.main).toBe('./woml-core.linux-x64-musl.node');
    expect(manifest.os).toEqual(['linux']);
    expect(manifest.cpu).toEqual(['x64']);
    expect(manifest.libc).toEqual(['musl']);
    expect(await Bun.file(resolve(output, 'LICENSE')).exists()).toBe(true);
    expect(await Bun.file(resolve(output, 'NOTICE.md')).exists()).toBe(true);
    expect(
      await Bun.file(resolve(output, 'woml-core.linux-x64-musl.node')).text(),
    ).toBe('test-native-bytes');
    expect(await Bun.file(resolve(output, 'README.md')).text()).toContain(
      'installed automatically by `woml`',
    );
  });
});
