import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixture = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/remote-execution-targets/model.reviewed.woml'
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

describe('Remote Execution Targets CLI boundary', () => {
  test('shows every explicit placement without activating transport', async () => {
    const result = await invoke(['check', fixture]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('Compiled Model v21');
    expect(result.stdout).toContain('Target: applyRelease -> production-private');
    expect(result.stdout).toContain(
      'Target: deploy:compensate:applyRelease -> production-private'
    );
    expect(result.stdout).toContain(
      'Target: deploy:cleanup:removeTemporaryFiles -> maintenance-runner'
    );
    expect(result.stdout).toContain(
      'authenticated sessions, durable dispatch authority, and the worker attempt runtime are available; live dispatch transport completes in RET7'
    );
  });

  test('provides a stable machine-readable RET1 inspection', async () => {
    const result = await invoke(['check', fixture, '--json']);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    const value = JSON.parse(result.stdout);
    expect(value).toEqual(
      expect.objectContaining({
        profile: 'woml.model-validation/v1',
        valid: true,
        workflowId: 'remote-deployment',
        feature: 'remote-execution-targets',
        executable: false,
        modelVersion: 21,
        runtimePhase: 'RET1',
      })
    );
    expect(value.placements).toHaveLength(3);
  });
});
