import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-event.woml'
);

describe('T12 named-event CLI safety boundary', () => {
  test('automatically resolves the secret referenced by <event> before binding', async () => {
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
        throw new Error('run must resolve secrets through the store.');
      },
      waitForShutdown: async () => {
        throw new Error('an unauthenticated event runtime must not start.');
      },
    });

    expect(exitCode).toBe(1);
    expect(stdout).toBe('');
    expect(stderr).toContain('WOML_SECRET_NOT_FOUND');
    expect(stderr).toContain('EVENT_CONTROL_TOKEN');
  });

  test('rejects the removed --control-secret runtime convention', async () => {
    let stderr = '';
    const exitCode = await runCli(
      ['run', workflowPath, '--control-secret', 'CUSTOM_EVENT_TOKEN'],
      {
        stdout: () => {},
        stderr: text => {
          stderr += text;
        },
      },
      {
        createSecretStore: () => ({
          provider: 'environment',
          get: async () => undefined,
          has: async () => false,
          list: async () => [],
          set: async () => {},
          delete: async () => false,
        }),
        readSecret: async () => '',
        waitForShutdown: async () => {},
      }
    );

    expect(exitCode).toBe(2);
    expect(stderr).toContain('Usage: woml run');
    expect(stderr).not.toContain('--control-secret');
  });
});
