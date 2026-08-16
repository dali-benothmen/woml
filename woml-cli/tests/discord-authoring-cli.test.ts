import { afterAll, describe, expect, test } from 'bun:test';
import { rmSync } from 'node:fs';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/communication-providers'
);

afterAll(() => {
  rmSync(resolve(fixtureRoot, 'woml-custom-data.json'), { force: true });
});

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

describe('Discord CLI boundary', () => {
  test('checks a complete Discord workflow without contacting Discord', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Discord triggers, notifications, approval buttons');
    expect(result.stdout).toContain('services.discord.send()');
  });

  test('checks imported Discord messaging as executable Model v15', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-module-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('Definition package:');
    expect(result.stdout).toContain('services.discord.send()');
  });

  test('publishes an executable Discord definition package', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-module-acp4.woml'),
      '--json',
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(JSON.parse(result.stdout).runtimeReady).toBe(true);
  });

  test('keeps a local <discord> provider while the trigger stays built-in', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'discord-contextual-alias-acp4.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toContain('WOML_BUILTIN_PROVIDER_SHADOWED');
    expect(result.stdout).toContain('Compiled Model v15 package');
    expect(result.stdout).toContain('Discord triggers, notifications, approval buttons');
  });
});
