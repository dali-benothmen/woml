import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

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
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('DS1 Durable User State CLI authoring journey', () => {
  test('normal woml check generates StateService types without requiring a module', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-ds1-cli-'));
    try {
      const workflowPath = join(directory, 'memory.woml');
      await Bun.write(
        workflowPath,
        `<woml><workflow id="state-memory" name="State memory" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="read"><script>return services.state.get('last-value');</script></step></steps>
</workflow></woml>`
      );
      const result = await invoke(['check', workflowPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toContain('WOML check passed');
      const declarations = await Bun.file(join(directory, 'woml-env.d.ts')).text();
      expect(declarations).toContain('interface WomlStateService');
      expect(declarations).toContain('readonly state: WomlStateService;');
      expect(declarations).not.toContain('declare const context');
      expect(declarations).not.toContain('declare const secrets');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('check reports Durable User State usage found in a local module', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-ds1-module-cli-'));
    try {
      const workflowPath = join(directory, 'memory.woml');
      await Bun.write(
        workflowPath,
        `<woml><imports><module name="memory" from="./memory.ts" /></imports>
<workflow id="module-memory" name="Module memory" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="read"><script>return services.memory.read();</script></step></steps>
</workflow></woml>`
      );
      await Bun.write(
        join(directory, 'memory.ts'),
        `export async function read() { return services.state.get('last-value'); }`
      );
      const result = await invoke(['check', workflowPath]);
      expect(result.exitCode).toBe(0);
      expect(result.stdout).toContain('Durable state usage: 1 local module source(s).');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
