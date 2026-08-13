import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixtureRoot = resolve(import.meta.dir, '../../woml/tests/fixtures');

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

describe('FJ1 woml check conditional migration', () => {
  test('keeps canonical choice checks quiet', async () => {
    const result = capture();
    const exitCode = await runCli(
      ['check', resolve(fixtureRoot, 'branch.woml')],
      result.io
    );

    expect(exitCode).toBe(0);
    expect(result.output().stdout).toContain('WOML check passed');
    expect(result.output().stderr).not.toContain(
      'WOML_DEPRECATED_CONDITIONAL_BRANCH'
    );
  });

  test('prints a source-located legacy warning to stderr and still exits zero', async () => {
    const result = capture();
    const exitCode = await runCli(
      ['check', resolve(fixtureRoot, 'branch.legacy.woml'), '--json'],
      result.io
    );
    const output = result.output();

    expect(exitCode).toBe(0);
    expect(() => JSON.parse(output.stdout)).not.toThrow();
    expect(output.stderr).toContain(
      'Warning [WOML_DEPRECATED_CONDITIONAL_BRANCH]'
    );
    expect(output.stderr).toContain('branch.legacy.woml:16:5');
    expect(output.stderr).toContain('Rename the opening <branch> tag');
  });
});
