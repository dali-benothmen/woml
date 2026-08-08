import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = resolve(projectRoot, 'examples/scheduleWorkflow.woml');

describe('T8 schedule CLI boundary', () => {
  test('compiles schedules but refuses to pretend a clock is active before T9', async () => {
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

    const exitCode = await runCli(['run', workflowPath], io, {
      createSecretStore: () => {
        throw new Error('Schedule T8 must not touch the secret store.');
      },
      readSecret: async () => {
        throw new Error('Schedule T8 must not read a secret.');
      },
      waitForShutdown: async () => {
        throw new Error('Schedule T8 must not claim runtime readiness.');
      },
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe('');
    expect(stderr).toContain('WOML_TRIGGER_UNSUPPORTED');
    expect(stderr).toContain('Schedule trigger "dailyReport" is valid WOML');
    expect(stderr).toContain(workflowPath);
    expect(stderr).not.toContain('WOML automation is active');
  });
});
