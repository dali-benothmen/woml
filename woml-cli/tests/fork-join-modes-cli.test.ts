import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { createSecretStore } from '../src/secrets';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/fork-branch'
);
const nativeCorePath = resolve(
  import.meta.dir,
  `../dist/woml-core.${process.platform}-${process.arch}.node`
);

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: value => {
      stdout += value;
    },
    stderr: value => {
      stderr += value;
    },
  };
  const exitCode = await runCli(args, io, {
    createSecretStore: () => createSecretStore(),
    readSecret: async () => '',
    nativeCorePath,
  });
  return { exitCode, stdout, stderr };
}

describe('selected and non-blocking fork joins in the CLI', () => {
  test('selected join executes only after its named branches and publishes their outputs', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-selected-join-cli-'));
    try {
      const result = await invoke([
        'test',
        join(fixtureRoot, 'join-selected.woml'),
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({
        instagram: { status: 'published' },
        facebook: { status: 'published' },
      });
      expect(result.stderr).not.toContain('UNSUPPORTED_FORK_EXECUTION');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('non-blocking join returns the main result only after its branch remains owned and settles', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-none-join-cli-'));
    try {
      const result = await invoke([
        'test',
        join(fixtureRoot, 'join-none.woml'),
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({ accepted: true });
      expect(result.stderr).not.toContain('UNSUPPORTED_FORK_EXECUTION');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
