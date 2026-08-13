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

describe('FJ2 fork authoring gate', () => {
  test('reports the source-located Model v13 gate instead of compiling an older model', async () => {
    const source = resolve(
      import.meta.dir,
      '../../woml/tests/fixtures/fork-branch/join-all.woml'
    );
    const result = capture();

    const exitCode = await runCli(['check', source], result.io);
    const output = result.output();

    expect(exitCode).toBe(1);
    expect(output.stdout).toBe('');
    expect(output.stderr).toContain('WOML_MODEL_V13_REQUIRED');
    expect(output.stderr).toContain('join-all.woml:');
    expect(output.stderr).toContain('source passed FJ2 validation');
  });
});
