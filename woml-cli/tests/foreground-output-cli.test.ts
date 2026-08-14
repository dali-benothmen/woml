import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = join(projectRoot, 'examples', 'switchWorkflow.woml');
const nativeCorePath = join(
  projectRoot,
  'woml-cli',
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;
const directories: string[] = [];

afterEach(async () => {
  await Promise.all(
    directories.splice(0).map(directory =>
      rm(directory, { recursive: true, force: true })
    )
  );
});

function dependencies() {
  return {
    nativeCorePath,
    createSecretStore: () => ({
      provider: 'environment' as const,
      get: async () => undefined,
      has: async () => false,
      list: async () => [],
      set: async () => {},
      delete: async () => false,
    }),
    readSecret: async () => '',
    waitForShutdown: async () => {},
  };
}

describe('foreground CLI presentation modes', () => {
  nativeTest('emits only newline-delimited presentation records with --json', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-foreground-json-'));
    directories.push(directory);
    let stdout = '';
    let stderr = '';
    const io: CliIo = {
      stdout: text => { stdout += text; },
      stderr: text => { stderr += text; },
      isTTY: false,
    };

    const exitCode = await runCli(
      [
        'run',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
        '--json',
      ],
      io,
      dependencies()
    );

    expect(exitCode).toBe(0);
    const records = stdout.trim().split('\n').map(line => JSON.parse(line));
    expect(records).toHaveLength(2);
    expect(records[0]).toMatchObject({ id: 'switch-demo', version: '1.0.0' });
    expect(records[1]).toMatchObject({
      profile: 'woml.run-presentation/v1',
      workflow: { id: 'switch-demo' },
      status: 'succeeded',
      result: { delivered: true },
    });
    expect(stdout).not.toContain('\u001b[');
    expect(stderr).not.toContain('WOML WORKFLOW');
  });

  nativeTest('honors explicit color in an interactive foreground terminal', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-foreground-color-'));
    directories.push(directory);
    let stderr = '';
    const io: CliIo = {
      stdout: () => {},
      stderr: text => { stderr += text; },
      isTTY: true,
      columns: 80,
    };

    const exitCode = await runCli(
      [
        'run',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
        '--color=always',
      ],
      io,
      dependencies()
    );

    expect(exitCode).toBe(0);
    expect(stderr).toContain('\u001b[');
    expect(stderr).toContain('Switch routing demo');
    expect(stderr).toContain('RUN COMPLETED');
    expect(stderr).toContain('Final result');
  });
});
