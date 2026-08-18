import { createHash } from 'node:crypto';
import { afterAll, describe, expect, test } from 'bun:test';
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';

import {
  createPlatformPackage,
  verifyCollectedRelease,
} from '../scripts/native-release';
import {
  nativeLoadReceiptName,
  packReleaseArtifact,
  sealReleaseArtifact,
} from '../scripts/release-artifact';
import { publicPackageFiles } from '../scripts/release-contract';
import { expectedNativeExports } from '../scripts/verify-native-exports';
import {
  nativePackageBinaryName,
  nativePackageName,
  womlNativeTargets,
} from '../src/native-platform';

const temporaryDirectories: string[] = [];

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(path => rm(path, { recursive: true, force: true })),
  );
});

async function write(path: string, value: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, value);
}

async function createMain(root: string): Promise<void> {
  for (const path of publicPackageFiles) {
    await write(resolve(root, path), `${path}\n`);
  }
  await write(
    resolve(root, 'package.json'),
    `${JSON.stringify({
      name: 'woml-cli',
      version: '1.0.1',
      files: publicPackageFiles,
      optionalDependencies: Object.fromEntries(
        womlNativeTargets.map(target => [nativePackageName(target), '1.0.1']),
      ),
    })}\n`,
  );
  await packReleaseArtifact(root);
  await sealReleaseArtifact(root, 'main');
}

async function createNativeFamily(root: string): Promise<void> {
  for (const target of womlNativeTargets) {
    const fixture = resolve(root, `${target}.fixture`);
    const output = resolve(root, `native-${target}`);
    await write(fixture, `native fixture for ${target}\n`);
    await createPlatformPackage(target, fixture, output);
    const binary = nativePackageBinaryName(target);
    const binaryBytes = await readFile(resolve(output, binary));
    await write(
      resolve(output, nativeLoadReceiptName),
      `${JSON.stringify({
        schemaVersion: 1,
        target,
        binary,
        binarySha256: createHash('sha256').update(binaryBytes).digest('hex'),
        exports: [...expectedNativeExports].sort(),
      })}\n`,
    );
    await packReleaseArtifact(output);
    await sealReleaseArtifact(output, 'native', target);
    await rm(fixture);
  }
}

describe('collected WOML release family', () => {
  test('accepts one exact sealed package per frozen target and rejects omissions', async () => {
    const root = await mkdtemp(resolve(tmpdir(), 'woml-release-family-'));
    temporaryDirectories.push(root);
    const main = resolve(root, 'main');
    const platforms = resolve(root, 'platforms');
    await Promise.all([mkdir(main), mkdir(platforms)]);
    await createMain(main);
    await createNativeFamily(platforms);

    await expect(
      verifyCollectedRelease(main, platforms, 'v1.0.1'),
    ).resolves.toBeUndefined();

    await rm(resolve(platforms, `native-${womlNativeTargets[0]}`), {
      recursive: true,
    });
    await expect(
      verifyCollectedRelease(main, platforms, 'v1.0.1'),
    ).rejects.toThrow(`Collected release is missing: ${womlNativeTargets[0]}`);
  }, 30_000);
});
