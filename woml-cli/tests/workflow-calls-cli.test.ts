import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixtureDirectory = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/workflow-calls'
);

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: text => {
      stdout += text;
    },
    stderr: text => {
      stderr += text;
    },
  };
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('WC1 Workflow Calls CLI boundary', () => {
  test('checks a call-only workflow without manufacturing a trigger', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureDirectory, 'calculate-risk.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Workflow Calls frontend is valid');
    expect(result.stdout).toContain('execution begins in WC2');
  });

  test('stops run at the honest frontend boundary before invoking Rust', async () => {
    const result = await invoke([
      'run',
      resolve(fixtureDirectory, 'calculate-risk.woml'),
    ]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain(
      'WOML_WORKFLOW_CALL_RUNTIME_UNAVAILABLE'
    );
    expect(result.stderr).toContain('durable Rust execution begins in WC2');
  });

  test('stops a triggered parent using workflows.call at the same boundary', async () => {
    const result = await invoke([
      'run',
      resolve(fixtureDirectory, 'request-risk.woml'),
    ]);
    expect(result.exitCode).toBe(1);
    expect(result.stderr).toContain(
      'WOML_WORKFLOW_CALL_RUNTIME_UNAVAILABLE'
    );
  });
});
