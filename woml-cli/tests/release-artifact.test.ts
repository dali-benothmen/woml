import { afterAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import {
  packReleaseArtifact,
  releaseArtifactManifestName,
  sealReleaseArtifact,
  verifyReleaseArtifact,
} from '../scripts/release-artifact';

const temporaryDirectories: string[] = [];

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(path => rm(path, { recursive: true, force: true })),
  );
});

async function fixture(): Promise<string> {
  const root = await mkdtemp(resolve(tmpdir(), 'woml-artifact-test-'));
  temporaryDirectories.push(root);
  await mkdir(resolve(root, 'dist'), { recursive: true });
  await writeFile(
    resolve(root, 'package.json'),
    `${JSON.stringify({
      name: 'woml-artifact-fixture',
      version: '1.0.0',
      files: ['dist/cli.js'],
      bin: { woml: './dist/cli.js' },
    })}\n`,
  );
  await writeFile(resolve(root, 'dist/cli.js'), '#!/usr/bin/env bun\n');
  return root;
}

describe('sealed release artifacts', () => {
  test('hashes the complete package and its npm archive deterministically', async () => {
    const root = await fixture();
    const archive = await packReleaseArtifact(root);
    expect(archive).toEndWith('.tgz');

    const sealed = await sealReleaseArtifact(root, 'main');
    expect(sealed.kind).toBe('main');
    expect(sealed.files.some(file => file.path === archive)).toBe(true);
    await expect(verifyReleaseArtifact(root)).resolves.toEqual(sealed);
    expect(await Bun.file(resolve(root, releaseArtifactManifestName)).exists()).toBe(true);
  });

  test('fails closed when one byte changes after sealing', async () => {
    const root = await fixture();
    await packReleaseArtifact(root);
    await sealReleaseArtifact(root, 'main');
    await writeFile(resolve(root, 'dist/cli.js'), 'tampered\n');

    await expect(verifyReleaseArtifact(root)).rejects.toThrow(
      'checksum mismatch',
    );
  });
});
