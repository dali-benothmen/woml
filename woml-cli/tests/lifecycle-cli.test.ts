import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { formatExecutionProgress, runCli, type CliIo } from '../src/cli';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/lifecycle'
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

describe('LEC4 lifecycle CLI admission', () => {
  test('LEC4 progress identifies the observed step', () => {
    expect(
      formatExecutionProgress({
        profile: 'woml.lifecycle-progress/v1',
        runId: 'run_lec4',
        workflowId: 'lifecycle-demo',
        phase: 'action_started',
        hookId: 'lifecycle:step_success',
        actionId: 'lifecycle:step_success:action:0',
        stepId: 'prepare',
      })
    ).toBe('Lifecycle lifecycle:step_success for step prepare action started.');
  });

  test('woml check accepts module-free and module-backed lifecycle source', async () => {
    for (const name of ['lifecycle.woml', 'lifecycle-module.woml']) {
      const result = await invoke(['check', resolve(fixtureRoot, name)]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toContain('WOML check passed');
      expect(result.stdout).toContain(
        'workflow and step lifecycle scripts are executable'
      );
    }
  });

  test('woml check --json exposes the reviewed module-backed v6/v11 package', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'lifecycle-module.woml'),
      '--json',
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(JSON.parse(result.stdout)).toMatchObject({
      schemaVersion: 6,
      profile: 'woml.definition-package/v6',
      runtimeReady: false,
      workflow: { model: { schemaVersion: 11 } },
    });
  });

  test('woml run keeps lifecycle notifications staged for LEC5', async () => {
    const result = await invoke([
      'run',
      resolve(fixtureRoot, 'lifecycle.woml'),
    ]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_LIFECYCLE_RUNTIME_UNAVAILABLE');
    expect(result.stderr).toContain('introduced in LEC5');
  });
});
