import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import type { ManualLineInput } from '../src/manual-input';
import { listRunsWithRust } from '../src/rust-executor';

const projectRoot = resolve(import.meta.dir, '../..');
const workflowPath = join(projectRoot, 'examples', 'switchWorkflow.woml');
const forkWorkflowPath = join(projectRoot, 'examples', 'manualForkWorkflow.woml');
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

class OneRunManualInput implements ManualLineInput {
  readonly isTTY = true;
  #closed = false;

  constructor(private readonly statePath: string) {}

  async run(onLine: (line: string) => void | Promise<void>): Promise<void> {
    await onLine('');
    while (!this.#closed) {
      const run = listRunsWithRust(
        this.statePath,
        { limit: 1 },
        { nativeCorePath }
      ).runs[0];
      if (run?.status === 'succeeded' || run?.status === 'failed' || run?.status === 'cancelled') {
        await Bun.sleep(100);
        return;
      }
      await Bun.sleep(10);
    }
  }

  close(): void {
    this.#closed = true;
  }
}

function dependencies(statePath: string) {
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
    createManualInput: () => new OneRunManualInput(statePath),
    waitForShutdown: () => new Promise<void>(() => {}),
  };
}

describe('foreground CLI presentation modes', () => {
  nativeTest('emits only newline-delimited presentation records with --json', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-foreground-json-'));
    directories.push(directory);
    let stdout = '';
    let stderr = '';
    const statePath = join(directory, 'state.sqlite');
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
        statePath,
        '--json',
      ],
      io,
      dependencies(statePath)
    );

    expect(exitCode).toBe(0);
    const records = stdout.trim().split('\n').map(line => JSON.parse(line));
    expect(records.length).toBeGreaterThanOrEqual(3);
    expect(records[0]).toMatchObject({ id: 'switch-demo', version: '1.0.0' });
    expect(records.at(-1)).toMatchObject({
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
    const statePath = join(directory, 'state.sqlite');
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
        statePath,
        '--color=always',
      ],
      io,
      dependencies(statePath)
    );

    expect(exitCode).toBe(0);
    expect(stderr).toContain('\u001b[');
    expect(stderr).toContain('Switch routing demo');
    expect(stderr).toContain('RUN COMPLETED');
    expect(stderr).toContain('Final result');
  });

  nativeTest('renders nested fork and conditional execution as one atomic run block', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-foreground-fork-'));
    directories.push(directory);
    let stdout = '';
    let stderr = '';
    const statePath = join(directory, 'state.sqlite');
    const exitCode = await runCli(
      ['run', forkWorkflowPath, '--state', statePath, '--json'],
      {
        stdout: text => { stdout += text; },
        stderr: text => { stderr += text; },
        isTTY: false,
      },
      dependencies(statePath)
    );

    expect(exitCode, stderr).toBe(0);
    const records = stdout.trim().split('\n').map(line => JSON.parse(line));
    const run = records.at(-1);
    expect(run, stderr).toMatchObject({
      profile: 'woml.run-presentation/v1',
      workflow: { id: 'manual-fork-demo' },
      status: 'succeeded',
    });
    const fork = run.steps.find((step: { kind: string }) => step.kind === 'fork');
    expect(fork).toMatchObject({ status: 'succeeded' });
    expect(fork.detail).toContain('join inventory, payment');
    expect(fork.detail).toContain('continuation released');
    expect(run.steps.filter((step: { kind: string }) => step.kind === 'branch')).toHaveLength(3);
    expect(run.steps.find((step: { id: string }) => step.id === 'inventoryOutcome')).toMatchObject({
      kind: 'choose',
      detail: 'Selected condition 1.',
      depth: 2,
    });
    expect(run.steps.find((step: { id: string }) => step.id === 'recordAnalytics').depth).toBe(2);
    expect(JSON.stringify(run)).not.toContain('__woml_fork__');
    expect(run.summary).toMatchObject({ total: 14, succeeded: 12, skipped: 2 });
  });
});
