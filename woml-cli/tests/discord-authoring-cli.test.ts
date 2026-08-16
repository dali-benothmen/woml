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

describe('ACP4 Discord CLI boundary', () => {
  test('checks a complete Discord workflow without contacting Discord', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Discord authoring and lowering are valid');
    expect(result.stdout).toContain('execution begin in ACP5');
  });

  test('checks imported Discord messaging as staged Model v15', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-module-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('Definition package:');
    expect(result.stdout).toContain('Discord authoring and lowering are valid');
  });

  test('rejects woml run before resolving secrets or opening a network connection', async () => {
    const result = await invoke([
      'run',
      resolve(fixtureRoot, 'discord-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_DISCORD_RUNTIME_UNAVAILABLE');
    expect(result.stderr).toContain('Discord Gateway and REST execution begin in ACP5');
    expect(result.stderr).not.toContain('WOML_SECRET_NOT_FOUND');
  });

  test('keeps a local <discord> provider while the trigger stays built-in', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-contextual-alias-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toContain('WOML_BUILTIN_PROVIDER_SHADOWED');
    expect(result.stdout).toContain('Compiled Model v15 package');
    expect(result.stdout).toContain('Discord authoring and lowering are valid');
  });
});
