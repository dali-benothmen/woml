import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-event.woml'
);

describe('T11 named-event CLI boundary', () => {
  test('compiles the event but rejects runtime activation until T12', async () => {
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
      createSecretStore: () => ({
        provider: 'environment',
        get: async () => undefined,
        has: async () => false,
        list: async () => [],
        set: async () => {},
        delete: async () => false,
      }),
      readSecret: async () => {
        throw new Error('T11 must not resolve a control secret yet.');
      },
      waitForShutdown: async () => {
        throw new Error('T11 must not start a long-lived runtime yet.');
      },
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe('');
    expect(stderr).toContain('WOML_TRIGGER_UNSUPPORTED');
    expect(stderr).toContain('compiled in T11');
    expect(stderr).toContain('executable in T12');
  });
});
