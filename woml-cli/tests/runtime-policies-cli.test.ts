import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/runtime-policies'
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
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('RP3 runtime-policy CLI execution boundary', () => {
  test('woml check accepts config and explains the enforcement boundary', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'runtime-policy.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Model v12 concurrency and durable FIFO queueing are executable');
    expect(result.stdout).toContain('remain staged for RP4 and RP5');
  });

  test('woml check --json exposes Definition Package v7 and Model v12 for modules', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'runtime-policy-module.woml'),
      '--json',
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(JSON.parse(result.stdout)).toMatchObject({
      schemaVersion: 7,
      profile: 'woml.definition-package/v7',
      runtimeReady: false,
      workflow: {
        model: {
          schemaVersion: 12,
          runtimePolicy: { profileVersion: 1, concurrency: 2 },
        },
      },
    });
  });

  test('woml run/test reject the RP4/RP5 policy fields instead of ignoring them', async () => {
    for (const command of ['run', 'test'] as const) {
      const result = await invoke([
        command,
        resolve(fixtureRoot, 'runtime-policy.woml'),
      ]);
      expect(result.exitCode, command).toBe(1);
      expect(result.stdout, command).toBe('');
      expect(result.stderr, command).toContain(
        'WOML input error [WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE]'
      );
      expect(result.stderr, command).toContain(
        'workflow timeout and rate-limit enforcement arrive in RP4 and RP5'
      );
    }
  });
});
