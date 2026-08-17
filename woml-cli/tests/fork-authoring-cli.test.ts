import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

function capture() {
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
  return {
    io,
    output: () => ({ stdout, stderr }),
  };
}

describe('Fork compilation', () => {
  test('woml check accepts a fork after Model v13 lowering is installed', async () => {
    const source = resolve(
      import.meta.dir,
      '../../woml/tests/fixtures/fork-branch/join-all.woml'
    );
    const result = capture();

    const exitCode = await runCli(['check', source], result.io);
    const output = result.output();

    expect(exitCode).toBe(0);
    expect(output.stdout).toContain('WOML check passed');
    expect(output.stderr).toBe('');
  });
});
