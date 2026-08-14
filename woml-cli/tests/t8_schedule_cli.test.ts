import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = resolve(projectRoot, 'examples/scheduleWorkflow.woml');
const nativeCorePath = resolve(
  projectRoot,
  'woml-cli/dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;

describe('T9 schedule CLI boundary', () => {
  nativeTest(
    'activates schedules, reports the next due instant, and shuts down cleanly',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-t9-cli-'));
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

      const exitCode = await runCli(
        ['run', workflowPath, '--state', resolve(directory, 'state.sqlite')],
        io,
        {
          nativeCorePath,
          createSecretStore: () => ({
            provider: 'environment',
            get: async () => undefined,
            has: async () => false,
            list: async () => [],
            set: async () => {},
            delete: async () => false,
          }),
          readSecret: async () => {
            throw new Error('Schedule T9 must not read a secret.');
          },
          waitForShutdown: async () => {},
        }
      );

      expect(exitCode).toBe(0);
      expect(stdout).toBe('');
      expect(stderr).toContain('Schedule   0 9 * * *');
      expect(stderr).toContain('Timezone   Europe/Berlin');
      expect(stderr).toContain('Next       ');
      expect(stderr).toContain('WOML automation is active.');
      expect(stderr).toContain('WOML automation stopped.');
      expect(stderr).not.toContain('WOML_RUNTIME_PROGRESS');
      expect(stderr).not.toContain('WOML_TRIGGER_UNSUPPORTED');
      await rm(directory, { recursive: true, force: true });
    }
  );
});
