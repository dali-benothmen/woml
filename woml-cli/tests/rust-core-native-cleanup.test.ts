import { afterEach, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { cleanupNativeVerificationDirectory } from '../scripts/verify-rust-core-native';

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true });
});

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'woml-native-cleanup-test-'));
  roots.push(directory);
  await writeFile(join(directory, 'candidate.node'), 'simulated loaded addon');
  for (const file of ['durable.sqlite', 'durable.output.sqlite', 'ingress.sqlite', 'ingress.sqlite.workflow-routing-v1.key', 'outcome.sqlite']) {
    await writeFile(join(directory, file), 'test runtime data');
  }
  await mkdir(join(directory, 'objects-v1', 'nested'), { recursive: true });
  await writeFile(join(directory, 'objects-v1', 'nested', 'object'), 'test artifact');
  return directory;
}

test('a locked Windows addon cannot prevent runtime data from being cleaned first', async () => {
  const directory = await fixture();
  const removed: string[] = [];
  await cleanupNativeVerificationDirectory(directory, true, {
    platform: 'win32',
    remove: async (path, options) => {
      removed.push(path);
      if (path === directory) {
        expect(await readdir(directory)).toEqual(['candidate.node']);
        throw Object.assign(new Error('simulated loaded DLL lock'), { code: 'EPERM' });
      }
      await rm(path, options);
    },
  });
  expect(removed.at(-1)).toBe(directory);
  expect(removed).toContain(join(directory, 'objects-v1'));
  expect(removed).toContain(join(directory, 'durable.sqlite'));
  expect(await readdir(directory)).toEqual(['candidate.node']);
});

test('an unlocked addon and all runtime data are removed normally', async () => {
  for (const platform of ['linux', 'win32'] as const) {
    const directory = await fixture();
    await cleanupNativeVerificationDirectory(directory, true, { platform });
    expect(existsSync(directory)).toBe(false);
  }
});

test('Windows database or artifact cleanup failures remain fatal', async () => {
  for (const name of ['durable.sqlite', 'objects-v1']) {
    const directory = await fixture();
    const failure = Object.assign(new Error('simulated runtime data lock'), { code: 'EPERM' });
    await expect(cleanupNativeVerificationDirectory(directory, true, {
      platform: 'win32',
      remove: async (path, options) => {
        if (path === join(directory, name)) throw failure;
        if (path === directory) throw new Error('must not tolerate a runtime data failure as a DLL lock');
        await rm(path, options);
      },
    })).rejects.toBe(failure);
    expect(existsSync(join(directory, name))).toBe(true);
  }
});

test('non-Windows, unloaded-addon, and unrelated errors are not ignored', async () => {
  for (const entry of [
    { platform: 'linux', loaded: true, code: 'EPERM' },
    { platform: 'win32', loaded: false, code: 'EPERM' },
    { platform: 'win32', loaded: true, code: 'EIO' },
  ] as const) {
    const directory = await fixture();
    const failure = Object.assign(new Error('simulated removal failure'), { code: entry.code });
    await expect(cleanupNativeVerificationDirectory(directory, entry.loaded, {
      platform: entry.platform,
      remove: async (path, options) => {
        if (path === directory) throw failure;
        await rm(path, options);
      },
    })).rejects.toBe(failure);
  }
});

test('new runtime data appearing during final removal is not accepted as a DLL leftover', async () => {
  const directory = await fixture();
  await expect(cleanupNativeVerificationDirectory(directory, true, {
    platform: 'win32',
    remove: async (path, options) => {
      if (path === directory) {
        await writeFile(join(directory, 'late.sqlite'), 'simulated leaked writer');
        throw Object.assign(new Error('directory not empty'), { code: 'ENOTEMPTY' });
      }
      await rm(path, options);
    },
  })).rejects.toThrow('deep-equal');
  expect(existsSync(join(directory, 'late.sqlite'))).toBe(true);
});
