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
      expect(result.stdout).toMatch(
        /executable through the durable Rust runtime|Telegram triggers, notifications, and messaging are runnable together/
      );
    }
  });

  test('publishes Model v15 as runtime-ready without contacting Telegram during check', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'telegram-acp2.woml'),
      '--json',
    ]);
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toMatchObject({
      schemaVersion: 1,
      profile: 'woml.definition-package/v1',
      workflow: { id: 'telegram-agent' },
    });
  });

  test('keeps a local <telegram> provider while the trigger resolves to the built-in', async () => {
    const path = resolve(fixtureRoot, 'telegram-contextual-alias-acp2.woml');
    const checked = await invoke(['check', path]);
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr).toContain('WOML_BUILTIN_PROVIDER_SHADOWED');
    expect(checked.stdout).toContain('Compiled Model v15 package');
    expect(checked.stdout).toContain(
      'custom definitions and Telegram triggers, notifications, and messaging are runnable together'
    );
  });
});
