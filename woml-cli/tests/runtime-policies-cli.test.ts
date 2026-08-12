import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { createSecretStore } from '../src/secrets';

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
  const exitCode = await runCli(args, io, {
    createSecretStore: () => createSecretStore(),
    readSecret: async () => '',
    nativeCorePath: resolve(
      import.meta.dir,
      `../dist/woml-core.${process.platform}-${process.arch}.node`
    ),
  });
  return { exitCode, stdout, stderr };
}

describe('RP5 executable runtime-policy CLI boundary', () => {
  test('woml check accepts config and reports the executable policy set', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'runtime-policy.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Model v12 concurrency');
    expect(result.stdout).toContain('rolling-window rate limits');
    expect(result.stdout).toContain('workflow timeouts are executable');
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

  test('woml test executes rate-limit and timeout policy fields', async () => {
    const result = await invoke([
      'test',
      resolve(fixtureRoot, 'runtime-policy.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).not.toContain(
      'WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE'
    );
    expect(JSON.parse(result.stdout)).toEqual({ ok: true });
  });
});
