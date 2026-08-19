import { describe, expect, test } from 'bun:test';
import { readFile, readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import {
  verifyReleaseTag,
  verifySourceReleaseIdentity,
} from '../scripts/native-release';

const repositoryRoot = resolve(import.meta.dir, '../..');

async function json(path: string): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(resolve(repositoryRoot, path), 'utf8')) as Record<
    string,
    unknown
  >;
}

async function sourceFilesBelow(path: string): Promise<string[]> {
  const entries = await readdir(path, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    if (entry.name === 'dist' || entry.name === 'node_modules') continue;
    const absolute = join(path, entry.name);
    if (entry.isDirectory()) files.push(...(await sourceFilesBelow(absolute)));
    else if (/\.(?:ts|tsx|js|mjs)$/u.test(entry.name)) files.push(absolute);
  }
  return files;
}

describe('WOML v1 release identity', () => {
  test('pins one public package and one private compiler at 1.0.3', async () => {
    await expect(verifySourceReleaseIdentity()).resolves.toBeUndefined();

    const root = await json('package.json');
    const compiler = await json('woml/package.json');
    const runtime = await json('woml-cli/package.json');
    const extension = await json('woml-vscode/package.json');

    expect(root).toMatchObject({
      name: 'woml-repository',
      version: '1.0.3',
      private: true,
    });
    expect(compiler).toMatchObject({
      name: '@woml/compiler',
      version: '1.0.3',
      private: true,
    });
    expect(runtime).toMatchObject({
      name: 'woml-cli',
      version: '1.0.3',
      private: false,
      bin: { woml: './dist/cli.js' },
      devDependencies: { '@woml/compiler': 'file:../woml' },
    });
    expect(extension).toMatchObject({
      name: 'woml-language',
      version: '1.0.3',
      publisher: 'woml',
    });
  });

  test('accepts only the exact release tag', () => {
    expect(() => verifyReleaseTag('1.0.3', 'v1.0.3')).not.toThrow();
    expect(() => verifyReleaseTag('1.0.3', 'v1.0.1')).toThrow(
      'must exactly match woml version v1.0.3',
    );
    expect(() => verifyReleaseTag('1.0.3', '1.0.3')).toThrow(
      'must exactly match woml version v1.0.3',
    );
  });

  test('uses the private compiler identity throughout live TypeScript source', async () => {
    const files = await sourceFilesBelow(resolve(repositoryRoot, 'woml-cli'));
    const stale: string[] = [];
    for (const file of files) {
      const source = await readFile(file, 'utf8');
      if (/from\s+['"]woml['"]/u.test(source)) {
        stale.push(file.slice(repositoryRoot.length + 1));
      }
    }
    expect(stale).toEqual([]);
  });

  test('keeps public installation and repository metadata on WOML', async () => {
    for (const path of ['README.md', 'docs/getting-started.md']) {
      const source = await readFile(resolve(repositoryRoot, path), 'utf8');
      expect(source).toMatch(
        /(?:bun\s+add|npm\s+(?:install|i)).*\bwoml-cli\b/u,
      );
    }

    const releaseWorkflow = await readFile(
      resolve(repositoryRoot, '.github/workflows/release.yml'),
      'utf8',
    );
    expect(releaseWorkflow).toContain('npm view "woml-cli@${version}"');

    for (const path of [
      'package.json',
      'woml-cli/package.json',
      'woml-vscode/package.json',
    ]) {
      expect(JSON.stringify(await json(path))).toContain(
        'github.com/dali-benothmen/woml',
      );
    }
  });
});
