import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = resolve(projectRoot, 'examples/intervalWorkflow.woml');
const nativeCorePath = resolve(
  projectRoot,
  'woml-cli/dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;

describe('Interval CLI boundary', () => {
  nativeTest(
    'activates an interval without opening an HTTP listener',
    async () => {
      const directory = await mkdtemp(resolve(tmpdir(), 'woml-t10-cli-'));
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
            throw new Error('Interval  must not read a secret.');
          },
          waitForShutdown: async () => {},
        }
      );

      expect(exitCode).toBe(0);
      expect(stdout).toBe('');
      expect(stderr).toContain('Every      5s');
      expect(stderr).toContain('Next       ');
      expect(stderr).toContain('WOML automation is active.');
      expect(stderr).toContain('WOML automation stopped.');
      expect(stderr).not.toContain('WOML_RUNTIME_PROGRESS');
      expect(stderr).not.toContain('WOML workflow active at http://');
      await rm(directory, { recursive: true, force: true });
    }
  );
});
