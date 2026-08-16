import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/communication-providers'
);

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: value => { stdout += value; },
    stderr: value => { stderr += value; },
  };
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('Telegram frontend CLI boundary', () => {
  test('checks tag and imported-module workflows without network access', async () => {
    for (const name of ['telegram-acp2.woml', 'telegram-module-acp2.woml']) {
      const result = await invoke(['check', resolve(fixtureRoot, name)]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toContain('WOML check passed');
      expect(result.stdout).toContain('compile to Model v15');
      expect(result.stdout).toContain('network execution begins in ACP3');
    }
  });

  test('rejects activation with an explicit staged-runtime diagnostic', async () => {
    const result = await invoke([
      'run',
      resolve(fixtureRoot, 'telegram-acp2.woml'),
    ]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_TELEGRAM_RUNTIME_UNAVAILABLE');
    expect(result.stderr).toContain('Telegram execution begins in ACP3');
    expect(result.stderr).toContain('Use `woml check`');
  });

  test('keeps a local <telegram> provider while the trigger resolves to the built-in', async () => {
    const path = resolve(fixtureRoot, 'telegram-contextual-alias-acp2.woml');
    const checked = await invoke(['check', path]);
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr).toContain('WOML_BUILTIN_PROVIDER_SHADOWED');
    expect(checked.stdout).toContain('Compiled Model v15 package');
    expect(checked.stdout).toContain('custom definitions and Telegram authoring compose');

    const run = await invoke(['run', path]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toContain('WOML_TELEGRAM_RUNTIME_UNAVAILABLE');
  });
});
