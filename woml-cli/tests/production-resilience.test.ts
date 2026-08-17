import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  preflightRuntimeConfiguration,
  resolveRuntimeConfiguration,
} from '../src/runtime-config';

describe('Production preflight resilience', () => {
  test('fails closed before activation when durable storage has low disk headroom', async () => {
    const root = await mkdtemp(join(tmpdir(), 'woml-pro9-low-disk-'));
    try {
      const configPath = join(root, 'woml.runtime.json');
      await writeFile(configPath, JSON.stringify({
        schemaVersion: 1,
        statePath: './data/state.sqlite',
        logging: { directory: './logs' },
      }));
      const configuration = await resolveRuntimeConfiguration(configPath);
      await expect(
        preflightRuntimeConfiguration(configuration, {
          statFilesystem: async () => ({ bavail: 1, bsize: 4096 }),
        })
      ).rejects.toThrow('at least 67108864 available bytes');
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test('fails closed before activation when required paths are read-only', async () => {
    const root = await mkdtemp(join(tmpdir(), 'woml-pro9-read-only-'));
    try {
      const configPath = join(root, 'woml.runtime.json');
      await writeFile(configPath, JSON.stringify({
        schemaVersion: 1,
        statePath: './data/state.sqlite',
      }));
      const configuration = await resolveRuntimeConfiguration(configPath);
      await expect(
        preflightRuntimeConfiguration(configuration, {
          accessPath: async () => {
            const error = new Error('read-only filesystem') as NodeJS.ErrnoException;
            error.code = 'EACCES';
            throw error;
          },
        })
      ).rejects.toThrow('path is not readable and writable');
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
