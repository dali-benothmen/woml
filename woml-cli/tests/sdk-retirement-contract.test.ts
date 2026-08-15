import { describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import { verifySdkRetirementContract } from '../scripts/verify-sdk-retirement-contract';

describe('Cronflow SDK retirement contract', () => {
  test('matches the package, public surface, docs, and data-safety promises', async () => {
    const root = resolve(import.meta.dir, '../..');
    await expect(verifySdkRetirementContract(root)).resolves.toBeUndefined();
  });

  test('rejects a silent public SDK export change', async () => {
    const root = resolve(import.meta.dir, '../..');
    const temporaryRoot = await mkdtemp(join(tmpdir(), 'sdk-retirement-'));
    try {
      const files = [
        'package.json',
        'src/index.ts',
        'sdk/index.ts',
        'sdk/src/index.ts',
        'sdk/src/cronflow.ts',
        'README.md',
        'docs/cronflow-sdk-retirement.md',
        'docs/cronflow-sdk-data-archive.md',
        'docs/woml-sdk-migration.md',
        'docs/contracts/cronflow-sdk-retirement.v1.json',
      ];
      for (const path of files) {
        const destination = resolve(temporaryRoot, path);
        await mkdir(dirname(destination), { recursive: true });
        await Bun.write(destination, await readFile(resolve(root, path)));
      }
      const publicEntry = resolve(temporaryRoot, 'sdk/index.ts');
      await writeFile(
        publicEntry,
        `${await readFile(publicEntry, 'utf8')}\nexport const accidentalFeature = true;\n`
      );

      await expect(verifySdkRetirementContract(temporaryRoot)).rejects.toThrow(
        'changed the frozen SDK surface'
      );
    } finally {
      await rm(temporaryRoot, { recursive: true, force: true });
    }
  });
});
