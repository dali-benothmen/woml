import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import type { ManualLineInput } from '../src/manual-input';
import { listRunsWithRust } from '../src/rust-executor';

const projectRoot = resolve(import.meta.dir, '../..');
const manualWorkflow = join(projectRoot, 'examples', 'switchWorkflow.woml');
const nativeCorePath = join(
  projectRoot,
  'woml-cli',
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;
const directories: string[] = [];

class ControlledManualInput implements ManualLineInput {
  readonly isTTY: boolean;
  #onLine?: (line: string) => void | Promise<void>;
  #resolveReady!: () => void;
  #resolveClosed!: () => void;
  readonly #ready = new Promise<void>(resolveReady => {
    this.#resolveReady = resolveReady;
  });
  readonly #closed = new Promise<void>(resolveClosed => {
    this.#resolveClosed = resolveClosed;
  });

  constructor(isTTY = true) {
    this.isTTY = isTTY;
  }

  async run(onLine: (line: string) => void | Promise<void>): Promise<void> {
    this.#onLine = onLine;
    this.#resolveReady();
    await this.#closed;
  }

  async submit(line: string): Promise<void> {
    await this.#ready;
    await this.#onLine!(line);
  }

  close(): void {
    this.#resolveClosed();
  }
}

afterEach(async () => {
  await Promise.all(
    directories.splice(0).map(directory =>
      rm(directory, { recursive: true, force: true })
    )
  );
});

async function waitUntil(
  predicate: () => boolean,
  description: string,
  timeoutMs = 10_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for ${description}.`);
    await Bun.sleep(10);
  }
}

function secretStore() {
  return {
    provider: 'environment' as const,
    get: async () => undefined,
    has: async () => false,
    list: async () => [],
    set: async () => {},
    delete: async () => false,
  };
}

describe('interactive manual trigger runtime', () => {
  nativeTest('admits no run before Enter and creates a distinct run for every Enter', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-input-'));
    directories.push(directory);
    const statePath = join(directory, 'state.sqlite');
    const input = new ControlledManualInput();
    let stderr = '';
    let resolveShutdown!: () => void;
    const shutdown = new Promise<void>(resolve => { resolveShutdown = resolve; });
    let resolveReady!: () => void;
    const ready = new Promise<void>(resolve => { resolveReady = resolve; });
    const running = runCli(
      ['run', manualWorkflow, '--state', statePath, '--color=never'],
      {
        stdout: () => {},
        stderr: text => { stderr += text; },
        isTTY: true,
        columns: 76,
      },
      {
        nativeCorePath,
        createSecretStore: secretStore,
        readSecret: async () => '',
        createManualInput: () => input,
        waitForShutdown: () => shutdown,
        onRuntimeReady: () => resolveReady(),
      }
    );

    try {
      await ready;
      await waitUntil(() => stderr.includes('Ready · Press Enter to run'), 'the ready prompt');
      expect(listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs).toHaveLength(0);
      expect(stderr).toContain('Press Enter to start a run');

      await input.submit('not-a-command');
      expect(stderr).toContain('WOML_MANUAL_TRIGGER_SELECTION_REQUIRED');
      expect(listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs).toHaveLength(0);

      await input.submit('');
      await waitUntil(
        () => listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs.length === 1,
        'the first manual run'
      );
      await waitUntil(() => stderr.includes('RUN COMPLETED'), 'the first settled run');
      const firstRun = listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs[0]!;

      await input.submit('');
      await waitUntil(
        () => listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs.length === 2,
        'the second manual run'
      );
      const runs = listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs;
      expect(new Set(runs.map(run => run.runId)).size).toBe(2);
      expect(runs.some(run => run.runId === firstRun.runId)).toBe(true);
    } finally {
      resolveShutdown();
    }
    expect(await running).toBe(0);
    expect(stderr).toContain('WOML automation stopped.');
  }, 30_000);

  nativeTest('requires a TTY for a manual-only woml run', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-no-tty-'));
    directories.push(directory);
    let stderr = '';
    const exitCode = await runCli(
      ['run', manualWorkflow, '--state', join(directory, 'state.sqlite')],
      { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: false },
      {
        nativeCorePath,
        createSecretStore: secretStore,
        readSecret: async () => '',
        createManualInput: () => new ControlledManualInput(false),
        waitForShutdown: async () => {},
      }
    );
    expect(exitCode).toBe(1);
    expect(stderr).toContain('WOML_MANUAL_TRIGGER_TTY_REQUIRED');
    expect(stderr).toContain('woml test');
  });

  nativeTest('rejects a manual-only background runtime before activation', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-background-'));
    directories.push(directory);
    let stderr = '';
    const previousHandoff = process.env.WOML_BACKGROUND_HANDOFF;
    process.env.WOML_BACKGROUND_HANDOFF = join(directory, 'handoff.json');
    try {
      const exitCode = await runCli(
        ['run', manualWorkflow, '--state', join(directory, 'state.sqlite')],
        { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: true },
        {
          nativeCorePath,
          createSecretStore: secretStore,
          readSecret: async () => '',
          createManualInput: () => new ControlledManualInput(),
          waitForShutdown: async () => {},
        }
      );
      expect(exitCode).toBe(1);
      expect(stderr).toContain('WOML_MANUAL_TRIGGER_BACKGROUND_UNAVAILABLE');
    } finally {
      if (previousHandoff === undefined) delete process.env.WOML_BACKGROUND_HANDOFF;
      else process.env.WOML_BACKGROUND_HANDOFF = previousHandoff;
    }
  });

  nativeTest('keeps runtime concurrency policy on the manual admission path', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-policy-'));
    directories.push(directory);
    const workflowPath = join(directory, 'policy.woml');
    await writeFile(workflowPath, `<woml><workflow id="manual-policy" name="Manual policy"><config concurrency="1" /><triggers><manual id="start" /></triggers><steps><step id="slow"><script>await new Promise(resolve => setTimeout(resolve, 500)); return { done: true };</script></step></steps></workflow></woml>`);
    const statePath = join(directory, 'state.sqlite');
    const input = new ControlledManualInput();
    let stderr = '';
    let resolveShutdown!: () => void;
    const shutdown = new Promise<void>(resolve => { resolveShutdown = resolve; });
    let resolveReady!: () => void;
    const ready = new Promise<void>(resolve => { resolveReady = resolve; });
    const running = runCli(
      ['run', workflowPath, '--state', statePath, '--color=never'],
      { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: true },
      {
        nativeCorePath,
        createSecretStore: secretStore,
        readSecret: async () => '',
        createManualInput: () => input,
        waitForShutdown: () => shutdown,
        onRuntimeReady: () => resolveReady(),
      }
    );
    try {
      await ready;
      await input.submit('');
      await input.submit('');
      await waitUntil(() => stderr.includes('Waiting for concurrency'), 'the concurrency queue notice');
      await waitUntil(
        () => listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs.filter(run => run.status === 'succeeded').length === 2,
        'both policy-controlled manual runs',
        15_000
      );
    } finally {
      resolveShutdown();
    }
    expect(await running).toBe(0);
  }, 30_000);

  nativeTest('uses numbered selection when multiple workflows expose manual triggers', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-manual-targets-'));
    directories.push(directory);
    const workflows = join(directory, 'workflows');
    await mkdir(workflows, { recursive: true });
    await writeFile(join(workflows, 'alpha.woml'), `<woml><workflow id="alpha" name="Alpha"><triggers><manual id="start" /></triggers><steps><step id="result"><script>return { selected: 'alpha' };</script></step></steps></workflow></woml>`);
    await writeFile(join(workflows, 'beta.woml'), `<woml><workflow id="beta" name="Beta"><triggers><manual id="refresh" /></triggers><steps><step id="result"><script>return { selected: 'beta' };</script></step></steps></workflow></woml>`);
    const statePath = join(directory, 'state.sqlite');
    const input = new ControlledManualInput();
    let stderr = '';
    let resolveShutdown!: () => void;
    const shutdown = new Promise<void>(resolve => { resolveShutdown = resolve; });
    let resolveReady!: () => void;
    const ready = new Promise<void>(resolve => { resolveReady = resolve; });
    const running = runCli(
      ['run', workflows, '--state', statePath, '--color=never'],
      { stdout: () => {}, stderr: text => { stderr += text; }, isTTY: true },
      {
        nativeCorePath,
        createSecretStore: secretStore,
        readSecret: async () => '',
        createManualInput: () => input,
        waitForShutdown: () => shutdown,
        onRuntimeReady: () => resolveReady(),
      }
    );
    try {
      await ready;
      await waitUntil(() => stderr.includes('MANUAL TRIGGERS'), 'the numbered trigger menu');
      expect(stderr).toContain('alpha / start');
      expect(stderr).toContain('beta / refresh');

      await input.submit('');
      expect(listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs).toHaveLength(0);
      await input.submit('2');
      await waitUntil(() => stderr.includes('{ selected: "beta" }'), 'the selected beta result');
      const runs = listRunsWithRust(statePath, { limit: 20 }, { nativeCorePath }).runs;
      expect(runs).toHaveLength(1);
      expect(runs[0]!.workflowId).toBe('beta');
    } finally {
      resolveShutdown();
    }
    expect(await running).toBe(0);
  }, 30_000);
});
