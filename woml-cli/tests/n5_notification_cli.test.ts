import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import type { SecretMetadata, SecretStore } from '../src/secrets';

const packageRoot = resolve(import.meta.dir, '..');
const nativePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativePath) ? test : test.skip;
const workflowPath = resolve(
  packageRoot,
  '../woml/tests/fixtures/approval-slack.woml'
);
const fakeHostPath = resolve(
  packageRoot,
  'tests/fixtures/fake-notification-provider-host.ts'
);

class ConfiguredSecrets implements SecretStore {
  readonly provider = 'environment' as const;

  async get(name: string): Promise<string | undefined> {
    return name === 'SLACK_BOT_TOKEN' || name === 'SLACK_APP_TOKEN'
      ? 'configured'
      : undefined;
  }

  async has(name: string): Promise<boolean> {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    return [];
  }

  async set(): Promise<void> {
    throw new Error('read only');
  }

  async delete(): Promise<boolean> {
    throw new Error('read only');
  }
}

describe('N5 notification CLI product path', () => {
  nativeTest('runs Model v5 through notification delivery and the selected route', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n5-cli-'));
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
    try {
      const exitCode = await runCli(
        [
          'run',
          workflowPath,
          '--state',
          join(directory, 'state.sqlite'),
        ],
        io,
        {
          createSecretStore: () => new ConfiguredSecrets(),
          readSecret: async () => 'unused',
          notificationHostPath: fakeHostPath,
          nativeCorePath: nativePath,
        }
      );
      expect(exitCode).toBe(0);
      expect(JSON.parse(stdout)).toEqual({ decision: 'approved' });
      expect(stderr).toContain('waiting for approval in Slack');
      expect(stderr).toContain('approve or reject from any configured channel');
      expect(stderr).not.toContain('Approval URL:');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
